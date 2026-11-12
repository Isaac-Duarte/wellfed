//! `wellfed-flash`: the UART flash client.
//!
//! Progress goes to stderr; stdout carries exactly one JSON summary, so a
//! script or agent can act on the result. Exit status: 0 complete; 1 stopped
//! once the serial session began, whether or not the port opened (inspect
//! the evidence; nothing is retried); 2 refused before the session began; 3
//! a bank-0 write that verified but is a step, not the write's end (its
//! summary names the steps that finish it); 4 an inspection that read the
//! bank and found it in another state than the command takes on.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::Ordering;

use clap::{Args, Parser, Subcommand};
use serde::Serialize;
use serde_json::Value;
use wellfed_core::buttons::Buttons;
use wellfed_core::virtual_panel;

use wellfed_bench_formats::lease::Lease;
use wellfed_flash::error::ErrorKind;
use wellfed_flash::evidence::report::SummaryStatus;
use wellfed_flash::evidence::session::{Context, EMULATOR, Outcome};
use wellfed_flash::link::protocol::{Core, INTERRUPTED};
use wellfed_flash::writers::{read, stock, update};
use wellfed_formats::cli::Device;
use wellfed_proto::device::Model;

#[derive(Parser)]
#[command(
    version,
    about = "UART flash reads, development updates and stock recovery for the PLAF103 and PLWF106 research units",
    long_about = "UART flash reads, development updates and stock recovery for the PLAF103 and PLWF106 research units.\n\n\
        Every device command continues the supervised bench procedure in docs/development/uart-development.md: \
        nothing here enters the loader, controls power, retries or boots. Progress goes to stderr and one JSON \
        summary to stdout. Exit status: 0 complete; 1 stopped once the serial session began, whether or not the port opened; \
        2 refused before the session began; 3 a bank-0 write that verified but is a step, not the write's end \
        (the summary names the steps that finish it); 4 an inspection that read the bank and found it in \
        another state than the command takes on."
)]
struct Cli {
    /// Repository checkout whose ignored private/ holds all evidence.
    #[arg(long, global = true, default_value = ".")]
    root: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Restricted reads that never program or erase flash.
    Read(ReadArgs),
    /// Region-scoped development update of bank 1 (default) or bank 0.
    Update(UpdateArgs),
    /// Clear bank 1's first marker byte so the loader falls back to bank 0.
    GiveWay(GiveWayArgs),
    /// Exact original bank-1 stock recovery from a full read.
    StockRecovery(StockArgs),

    /// Offline: plan stock recovery sectors from a baseline and current read.
    PlanStockRecovery {
        #[arg(long)]
        baseline: PathBuf,
        #[arg(long)]
        current: PathBuf,
        /// New private directory for plan.json and its sector payloads.
        #[arg(long)]
        output: PathBuf,
    },

    /// Offline: decode GD25Q64E status registers and assess a write range.
    DecodeStatus {
        #[arg(long, value_parser = number::<u8>)]
        sr1: u8,
        #[arg(long, value_parser = number::<u8>)]
        sr2: u8,
        #[arg(long, value_parser = number::<u8>)]
        sr3: u8,
        #[arg(long, value_parser = number::<usize>, default_value = "0x186000")]
        start: usize,
        #[arg(long, value_parser = number::<usize>, default_value = "0x2d6000")]
        end: usize,
    },

    /// Offline: check a transaction journal's hash chain; never resumes it.
    InspectJournal { journal: PathBuf },
}

/// Whether a command opens the feeder's serial port.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Port {
    /// A device session: it holds the bench lease while it runs.
    Opens,
    /// Works on files only; the bench may be anyone's.
    Offline,
}

impl Command {
    fn port(&self) -> Port {
        match self {
            Self::Read(_) | Self::Update(_) | Self::GiveWay(_) | Self::StockRecovery(_) => {
                Port::Opens
            }
            Self::PlanStockRecovery { .. }
            | Self::DecodeStatus { .. }
            | Self::InspectJournal { .. } => Port::Offline,
        }
    }
}

/// The bench lease for a device command: taken here when run by hand,
/// joined when `wellfed-bench` (an ancestor) holds it. An emulated helper
/// is not the bench, so the offline tests that serve one take none.
fn lease(root: &Path, port: Port) -> wellfed_flash::Result<Option<Lease>> {
    if port == Port::Offline || std::env::var_os(EMULATOR).is_some() {
        return Ok(None);
    }
    Ok(Some(Lease::acquire(root)?))
}

/// Why a decimal or `0x` hex argument did not parse; clap shows it.
#[derive(Debug, thiserror::Error)]
enum NumberError {
    #[error(transparent)]
    Parse(#[from] std::num::ParseIntError),
    /// It parsed, but does not fit the argument's type.
    #[error("{text} is out of range")]
    Range { text: String },
}

fn number<T: TryFrom<u64>>(text: &str) -> Result<T, NumberError> {
    let value = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => text.parse(),
    }?;
    T::try_from(value).map_err(|_| NumberError::Range { text: text.into() })
}

#[derive(Args)]
struct ReadArgs {
    mode: read::Mode,
    /// The model whose stock console --rom-report showed. Required by the
    /// reads that continue a ROM inspection (small, full, status); a report
    /// of another model's console is refused.
    #[arg(long, value_enum)]
    device: Option<Device>,
    #[arg(long)]
    port: String,
    /// New private output directory.
    #[arg(long)]
    output: PathBuf,
    /// Pinned vendor helper (default: the SDK's imgtool_flashloader_amebad.bin under private/).
    #[arg(long)]
    helper: Option<PathBuf>,
    /// Pinned SDK ROM image to compare the sampled words with (repeatable).
    #[arg(long)]
    candidate_rom: Vec<PathBuf>,
    #[arg(long)]
    rom_report: Option<PathBuf>,
    #[arg(long)]
    small_report: Option<PathBuf>,
    /// Receive-only observation duration (1-180 seconds).
    #[arg(long, default_value_t = 60.0)]
    seconds: f64,
    /// Known console owner for ROM inspection; never inferred on a mismatch.
    #[arg(long, value_enum, default_value = "km4")]
    start_core: Core,
    /// The chord a press holds: reset, mic, dispense or lock, or several
    /// joined by + (lock+dispense).
    #[arg(long, value_parser = virtual_panel::chord)]
    chord: Option<Buttons>,
    /// How long a press holds its chord.
    #[arg(long, default_value_t = virtual_panel::DEFAULT_HOLD_MS,
          value_parser = clap::value_parser!(u32).range(1..=i64::from(virtual_panel::MAX_HOLD_MS)))]
    hold_ms: u32,
}

#[derive(Args)]
struct EvidenceArgs {
    /// The model of the device written. Required: each model has its own
    /// pins (backup, banks, preserved ranges, plan kinds), and nothing of
    /// one binds a session on the other.
    #[arg(long, value_enum)]
    device: Device,
    #[arg(long)]
    port: String,
    /// The pinned original full-flash backup.
    #[arg(long)]
    baseline: PathBuf,
    #[arg(long)]
    rom_report: PathBuf,
    /// The bounded read whose helper session this continues.
    #[arg(long)]
    small_report: PathBuf,
    #[arg(long)]
    helper: Option<PathBuf>,
    /// New private output: a directory, or the plan file when preparing.
    #[arg(long)]
    output: PathBuf,
    /// Completed inspection report (prepare and execute).
    #[arg(long)]
    inspection: Option<PathBuf>,
    /// Reviewed plan (execute).
    #[arg(long)]
    plan: Option<PathBuf>,
    #[arg(long)]
    expected_plan_sha256: Option<String>,
}

impl EvidenceArgs {
    fn evidence(self) -> update::Evidence {
        update::Evidence {
            port: self.port,
            baseline: self.baseline,
            rom_report: self.rom_report,
            small_report: self.small_report,
            helper: self.helper,
            output: self.output,
            inspection: self.inspection,
            plan: self.plan,
            expected_plan_sha256: self.expected_plan_sha256,
        }
    }
}

#[derive(Args)]
struct UpdateArgs {
    phase: update::Mode,
    #[command(flatten)]
    evidence: EvidenceArgs,
    /// Frozen review directory of the image to install.
    #[arg(long)]
    target: PathBuf,
    /// Frozen review of the installed image; bank 0 may omit it with --original-read.
    #[arg(long)]
    source: Option<PathBuf>,
    /// Bank 0 without a source: a private full read pinning bank 0's current
    /// bytes (plaf103). A plwf106 bank 1 without a source is its pinned
    /// backup's.
    #[arg(long)]
    original_read: Option<PathBuf>,
    /// Bank to update; 0 is the factory loader's fallback (bank 1 must stay
    /// valid). The plwf106 writes bank 1 alone.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(0..=1))]
    bank: u8,
    /// Bank 1 after a give-way: the source with only its first marker byte cleared.
    #[arg(long)]
    restore_marker: bool,
    /// Finish a stopped update: staged marker, every sector source, staged target or erased.
    #[arg(long)]
    interrupted: bool,
    /// The stopped update's output directory (plwf106 with --interrupted):
    /// its reviewed plan and journal must be this update's, it must not
    /// have stopped on the bytes beside bank 1, and those bytes must still
    /// be as its plan read them.
    #[arg(long)]
    stopped_session: Option<PathBuf>,
}

#[derive(Args)]
struct GiveWayArgs {
    phase: update::Mode,
    #[command(flatten)]
    evidence: EvidenceArgs,
    /// Frozen review of the image bank 0 must hold exactly (plaf103). The
    /// plwf106 takes none: its bank 0 must be its pinned backup's.
    #[arg(long)]
    fallback: Option<PathBuf>,
}

#[derive(Args)]
struct StockArgs {
    phase: stock::Phase,
    #[command(flatten)]
    evidence: EvidenceArgs,
    /// The complete full read (flash-read.bin) of this helper session.
    #[arg(long)]
    current: PathBuf,
}

fn device(
    root: &Path,
    run: impl FnOnce(&Context) -> wellfed_flash::Result<Outcome>,
) -> (u8, Value) {
    let context = match Context::new(root) {
        Ok(context) => context,
        Err(error) => return refused(&error),
    };
    match run(&context) {
        Ok(outcome) => (outcome.completion.code(), outcome.summary),
        Err(error) => refused(&error),
    }
}

/// A writer's session, under the pins of the model it names alone.
fn writer(
    root: &Path,
    device: Device,
    run: impl FnOnce(&Context, Model) -> wellfed_flash::Result<Outcome>,
) -> (u8, Value) {
    let model = Model::from(device);
    self::device(root, |context| run(context, model))
}

#[derive(Serialize)]
struct Refusal {
    status: SummaryStatus,
    error_type: ErrorKind,
    error: String,
}

fn refused(error: &wellfed_flash::Error) -> (u8, Value) {
    eprintln!("Refused before serial access: {error}");
    let refusal = Refusal {
        status: SummaryStatus::Refused,
        error_type: error.kind(),
        error: error.to_string(),
    };

    // Strings only, so this always encodes.
    (2, serde_json::to_value(refusal).unwrap_or_default())
}

/// An offline command's result, refused if it fails or cannot be encoded.
fn offline<T: Serialize>(
    result: wellfed_flash::Result<T>,
    code: impl FnOnce(&T) -> u8,
) -> (u8, Value) {
    let encoded = result.and_then(|value| {
        let summary = serde_json::to_value(&value)?;
        Ok((code(&value), summary))
    });
    encoded.unwrap_or_else(|error| refused(&error))
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // An operator interrupt stops the session terminally and records it.
    let _ = ctrlc::set_handler(|| INTERRUPTED.store(true, Ordering::SeqCst));
    let root = cli.root.as_path();

    // Held until the summary is printed; a refusal here is before any
    // serial access, like every other refusal.
    let _lease = match lease(root, cli.command.port()) {
        Ok(lease) => lease,
        Err(error) => {
            let (code, summary) = refused(&error);
            println!("{summary}");
            return ExitCode::from(code);
        }
    };

    let (code, summary) = match cli.command {
        Command::Read(args) => device(root, |context| {
            read::run(
                context,
                &read::Args {
                    mode: args.mode,
                    port: args.port,
                    output: args.output,
                    helper: args.helper,
                    candidate_rom: args.candidate_rom,
                    rom_report: args.rom_report,
                    small_report: args.small_report,
                    seconds: args.seconds,
                    start_core: args.start_core,
                    device: args.device.map(Model::from),
                    press: args.chord.map(|chord| virtual_panel::Press {
                        chord,
                        hold_ms: args.hold_ms,
                    }),
                },
            )
        }),

        Command::Update(args) => writer(root, args.evidence.device, |context, model| {
            update::run_for(
                context,
                model,
                &update::Args {
                    mode: args.phase,
                    evidence: args.evidence.evidence(),
                    target: args.target,
                    source: args.source,
                    original_read: args.original_read,
                    bank: args.bank,
                    restore_marker: args.restore_marker,
                    interrupted: args.interrupted,
                    stopped_session: args.stopped_session,
                },
            )
        }),

        Command::GiveWay(args) => writer(root, args.evidence.device, |context, model| {
            update::give_way_for(
                context,
                model,
                &update::GiveWayArgs {
                    mode: args.phase,
                    evidence: args.evidence.evidence(),
                    fallback: args.fallback,
                },
            )
        }),

        Command::StockRecovery(args) => writer(root, args.evidence.device, |context, model| {
            stock::run_for(
                context,
                model,
                &stock::Args {
                    phase: args.phase,
                    evidence: args.evidence.evidence(),
                    current: args.current,
                },
            )
        }),

        Command::PlanStockRecovery {
            baseline,
            current,
            output,
        } => device(root, |context| {
            stock::plan_offline(context, &baseline, &current, &output)
        }),

        Command::DecodeStatus {
            sr1,
            sr2,
            sr3,
            start,
            end,
        } => offline(
            wellfed_flash::link::status::assess_range([sr1, sr2, sr3], start, end),
            |_| 0,
        ),

        Command::InspectJournal { journal } => offline(
            std::fs::read(&journal)
                .map_err(wellfed_flash::Error::from)
                .and_then(|data| wellfed_flash::evidence::journal::inspect(&data)),
            |receipt| u8::from(!receipt.verified_terminal),
        ),
    };

    println!("{summary}");
    ExitCode::from(code)
}

#[cfg(test)]
mod tests {
    use clap::ValueEnum;
    use wellfed_bench_formats::flash::FlashUse;

    use super::*;

    /// What each command does with the UART. Every match is exhaustive, so
    /// a new command or mode is decided here before it builds.
    fn flash_use(command: &Command) -> FlashUse {
        let update = |phase: update::Mode, counted| match phase {
            update::Mode::Inspect | update::Mode::Prepare => FlashUse::Session,
            update::Mode::Execute => FlashUse::Write { counted },
        };
        match command {
            Command::Read(args) => match args.mode {
                read::Mode::Observe => FlashUse::Observe,
                read::Mode::InspectRom
                | read::Mode::Small
                | read::Mode::Full
                | read::Mode::Status
                | read::Mode::Press => FlashUse::Session,
            },
            Command::Update(args) => update(args.phase, true),
            Command::GiveWay(args) => update(args.phase, false),
            Command::StockRecovery(args) => match args.phase {
                stock::Phase::Prepare => FlashUse::Session,
                stock::Phase::Execute => FlashUse::Write { counted: false },
            },
            Command::PlanStockRecovery { .. }
            | Command::DecodeStatus { .. }
            | Command::InspectJournal { .. } => FlashUse::Session,
        }
    }

    fn name(value: &impl ValueEnum) -> String {
        value
            .to_possible_value()
            .map(|value| value.get_name().to_owned())
            .unwrap_or_default()
    }

    /// The bench reads a child's use from its command line alone; it must
    /// agree with what every command and mode here does, writes above all.
    #[test]
    fn the_bench_reads_every_command_s_use_of_the_uart_as_it_is() {
        let evidence =
            "--device plaf103 --port p --baseline b --rom-report r --small-report s --output o";
        let mut lines: Vec<String> = Vec::new();
        for mode in read::Mode::value_variants() {
            lines.push(format!("read {} --port p --output o", name(mode)));
        }
        for phase in update::Mode::value_variants() {
            lines.push(format!("update {} {evidence} --target t", name(phase)));
            lines.push(format!("give-way {} {evidence}", name(phase)));
        }
        for phase in stock::Phase::value_variants() {
            lines.push(format!(
                "stock-recovery {} {evidence} --current c",
                name(phase)
            ));
        }
        lines.push("plan-stock-recovery --baseline b --current c --output o".into());
        lines.push("decode-status --sr1 0 --sr2 0 --sr3 0".into());
        lines.push("inspect-journal j".into());

        let mut writes = 0;
        for line in &lines {
            let words: Vec<String> = line.split(' ').map(String::from).collect();
            let cli = Cli::try_parse_from(
                std::iter::once("wellfed-flash".to_owned()).chain(words.clone()),
            )
            .unwrap_or_else(|error| panic!("{line}: {error}"));
            let expected = flash_use(&cli.command);

            assert_eq!(FlashUse::of(&words), expected, "{line}");
            writes += usize::from(matches!(expected, FlashUse::Write { .. }));
        }
        assert_eq!(writes, 3);
    }
}
