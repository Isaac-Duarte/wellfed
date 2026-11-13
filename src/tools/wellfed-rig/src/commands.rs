//! Bench sequences. Each one either completes, or stops without retry and
//! says what state it left the feeder in.
//!
//! `wellfed-bench`'s command line dispatches each subcommand to a [`Bench`]
//! method. Bench only switches relays and orders steps; serial reads,
//! inspections, plans and writes go through [`Tools`] as separate
//! `wellfed-flash` runs (and the over-the-air loop's build and review as
//! `wellfed-build` runs; it signs through `wellfed-signing` and offers
//! over the device API). Evidence directories are never reused, and an
//! execute refuses once its output exists, so each prepared plan runs at
//! most once.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};
use wellfed_bench_formats::transients::{self, TransientCycle, TransientsManifest};
use wellfed_core::buttons::Buttons;
use wellfed_formats::canonical::sha256;
use wellfed_formats::flash_plan::{DevelopmentPlan, GiveWayPlan, StockRecoveryReviewed};
use wellfed_formats::manifest::BuildManifest;

use crate::tools::{Analyzer, Children, Observation, Observer, Tools};
use crate::{Cancel, Error, Result};
use wellfed_bench_formats::banks::{Bank, Content, Source, Written};
use wellfed_bench_formats::progress::{BenchCommand, Status};
use wellfed_bench_formats::{CAPTURES, Config, Paths, plain_name, relative};
use wellfed_formats::require;
use wellfed_formats::review::{ImageKind, ImageReview};
use wellfed_proto::device::Model;
use wellfed_relay::{POWER, Relay, STRAP, Switch, UART};

mod bank0;
pub mod ota;
pub use ota::{Firmware, Image, OtaArgs, Variant};

/// How long the strap holds feeder TX low across a power-on.
const STRAP_HOLD: Duration = Duration::from_secs(2);
/// The capture that confirms a loader entry.
const LOADER_CAPTURE: Duration = Duration::from_secs(5);
/// A ROM loader answers the strap with a run of NAKs at least this long.
const LOADER_NAK_RUN: u64 = 8;

/// Observed per staged frame, framing included; for progress only.
const FRAME_TX_BYTES: u64 = 1056;
/// Logic analyzer (sigrok fx2lafw, 8 channels). Capture starts this long
/// before power-on so the unpowered state is recorded first; the first open
/// also uploads the analyzer's firmware, which takes about a second.
const ANALYZER_ARMING: Duration = Duration::from_secs(3);
/// Launches of the analyzer one cycle may take. The fx2lafw clone fails
/// its USB open about one launch in five (October 27), always while
/// arming: another launch repeats nothing the cycle has done.
const ANALYZER_STARTS: u32 = 3;
const ANALYZER_DRIVER_CHANNELS: [(&str, u8); 4] =
    [("ina", 0), ("inb", 1), ("outa", 2), ("outb", 3)];
const ANALYZER_OUTPUT_CHANNELS: [(&str, u8); 2] = [("outa", 2), ("outb", 3)];

/// Whether a press is its own job, with a step row, or one step inside
/// another job's sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PressStep {
    Own,
    Within,
}

/// The relay board's power-cut count when the feeder entered the loader,
/// `None` from the SH-UR04A, which cannot cut power by itself; the execute
/// in that helper session requires the same answer.
#[derive(Serialize, Deserialize)]
struct PowerAtLoader {
    power_cuts: Option<u32>,
}

const STOPPED_POWERED: &str =
    "Stopped without retry; the feeder stays powered. Inspect the evidence.";

/// Why a power, flash or capture sequence refused or stopped. Each refusal
/// comes before the step it guards, so none leaves a write half done.
#[derive(Debug, thiserror::Error)]
pub enum CommandsError {
    #[error("Session name {name:?} must be a single plain name")]
    BadName { name: String },
    /// A press needs a running feeder to read it.
    #[error("The feeder is off: power it and boot a virtual-panel build first")]
    PressUnpowered,
    /// Evidence directories are never reused; `path` root-relative.
    #[error("Refusing to reuse {path}")]
    NameTaken { path: String },
    #[error("Boot reached the ROM loader, not the application")]
    ReachedLoader,
    /// The strapped boot did not show the loader's banner and NAK run.
    #[error("Loader not confirmed; see {capture}")]
    LoaderNotConfirmed { capture: String },
    #[error("Need cycles >= 1 and a capture longer than arming")]
    TransientBounds,
    /// A length on the command line that no wait can take.
    #[error("{flag} {value} must be a finite, non-negative number, at most a week")]
    BadLength { flag: &'static str, value: f64 },
    #[error("{tool} not found; install it before a transient session")]
    ToolMissing { tool: &'static str },
    #[error("Analyzer exited before power-on: {stderr:?}")]
    AnalyzerExited { stderr: String },
    #[error("Analyzer did not finish its bounded capture")]
    AnalyzerTimeout,
    #[error("Analyzer failed: {stderr:?}")]
    AnalyzerFailed { stderr: String },
    #[error("--source is required except when overwriting bank 0 or the PLWF106's bank 1")]
    SourceRequired,
    /// A bank-0 write finishes by restoring bank 1's marker against its
    /// review, so it needs that review before anything is written.
    #[error("--bank 0 needs --bank1-source, the frozen review of what bank 1 holds")]
    Bank1SourceRequired,
    #[error("--bank1-source applies only to --bank 0")]
    Bank1SourceUnused,
    #[error("Cannot use the frozen review {review}: {error}")]
    Review {
        review: String,
        error: wellfed_image::Error,
    },
    /// Bank 1 does not hold the image `--bank1-source` names (its first
    /// marker byte aside, which a give-way clears), so restoring its marker
    /// against that review would fail after bank 0 was written.
    #[error(
        "Bank 1's first sector is not {review}'s, its first byte aside; name the review bank 1 holds"
    )]
    Bank1SourceDiffers { review: String },
    /// The bench ledger names what bank 1 holds, and it is not the review.
    #[error(
        "The bank ledger has bank 1 holding {ledger:?}, not {review}; name the review it holds"
    )]
    Bank1LedgerDiffers { review: String, ledger: Content },
    #[error("The prepare's inspection holds no bank-1 sector at {sector}")]
    Bank1SectorMissing { sector: String },
    /// The bank-1 review an execute names must be the one its prepare
    /// checked against bank 1, unchanged.
    #[error("{binding} does not bind --bank1-source {review} as prepared; prepare again")]
    Bank1SourceUnbound { binding: String, review: String },
    /// A bank-0 write exited 0, which only a `wellfed-flash` older than this
    /// bench does: the current one exits 3 for a verified bank-0 write.
    #[error(
        "wellfed-flash exited 0 for a bank-0 write: an older wellfed-flash than this bench \
         (version skew). Bank 0 is likely written and verified; rebuild wellfed-flash"
    )]
    FlashSkew,
    /// A target whose review or manifest cannot be read is not taken as
    /// motionless.
    #[error("Cannot read {file} to check the target's motion policy: {error}")]
    MotionPolicyUnreadable {
        file: String,
        error: wellfed_formats::Error,
    },
    #[error(
        "Target drives the motor ({policy}); rerun with --attended-motion only with the owner watching"
    )]
    MotionNotAttended { policy: String },
    /// An execute needs its prepared plan, and runs it at most once.
    #[error("Prepared plan required; execution is single-use")]
    PlanRequired,
    #[error("Feeder power was cycled since prepare; start over")]
    PowerCycled,
    #[error(
        "Bank 1 holds a valid image, which stock recovery refuses; give way first: \
         wellfed-bench stock-prepare {name} --fallback <bank-0 review>"
    )]
    Bank1Valid { name: String },
    #[error("The boot capture shows no stock version banner")]
    NoStockBanner,
    #[error("Bank 1 offset out of range")]
    Bank1Offset,
    #[error("{} is not a full read", read.display())]
    NotFullRead { read: PathBuf },
}

#[derive(Debug, Clone, Args)]
pub struct UpdateArgs {
    /// Session name for private evidence paths.
    pub name: String,
    /// Frozen review of the installed image (bank 0, and the PLWF106's bank 1, may omit it to overwrite the baseline's bytes).
    #[arg(long)]
    pub source: Option<PathBuf>,
    /// Bank to update; 0 is the factory loader's fallback.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(0..=1))]
    pub bank: u8,
    /// Bank 0 without --source: a full flash read pinning bank 0's current bytes.
    #[arg(long)]
    pub original_read: Option<PathBuf>,
    /// Frozen review to install.
    #[arg(long)]
    pub target: PathBuf,

    /// Finish a stopped update from a fresh session.
    #[arg(long)]
    pub interrupted: bool,
    /// Restore bank 1 after a give-way (the source with its first marker byte cleared).
    #[arg(long)]
    pub restore_marker: bool,
    /// Allow a target whose manifest names a motion policy (owner present).
    #[arg(long)]
    pub attended_motion: bool,
    /// Bank 0 only: the frozen review of what bank 1 holds. After the write
    /// the job gives way, boots bank 0 so it records itself, restores bank
    /// 1's marker against this review and boots bank 1, which must report
    /// `PLOTA1 bank0 check=match`.
    #[arg(long)]
    pub bank1_source: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
pub struct ExecuteArgs {
    #[command(flatten)]
    pub update: UpdateArgs,
    #[arg(long)]
    pub expected_plan_sha256: String,
    #[arg(long, default_value_t = 30.0)]
    pub boot_seconds: f64,
}

#[derive(Debug, Clone, Args)]
pub struct GiveWayArgs {
    /// Session name for private evidence paths.
    pub name: String,
    /// Frozen review of bank 0's image.
    #[arg(long)]
    pub fallback: PathBuf,
}

#[derive(Debug, Clone, Args)]
pub struct GiveWayExecuteArgs {
    #[command(flatten)]
    pub give_way: GiveWayArgs,
    #[arg(long)]
    pub expected_plan_sha256: String,
    #[arg(long, default_value_t = 30.0)]
    pub boot_seconds: f64,
}

/// The original stock application back into bank 1 over UART.
#[derive(Debug, Clone, Args)]
pub struct StockArgs {
    /// Session name for private evidence paths.
    pub name: String,
    /// Frozen review of bank 0's image. While bank 1 holds a valid image,
    /// stock recovery refuses, so the session gives way to bank 0 first.
    #[arg(long)]
    pub fallback: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
pub struct StockExecuteArgs {
    #[command(flatten)]
    pub stock: StockArgs,
    #[arg(long)]
    pub expected_plan_sha256: String,
    /// Stock's banner arrives within seconds; its log is long.
    #[arg(long, default_value_t = 60.0)]
    pub boot_seconds: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TransientMode {
    Boot,
    Loader,
}

/// Which driver lines the analyzer is clipped to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Probes {
    /// INA, INB, OUTA and OUTB on D0-D3, at the driver's pins.
    Driver,
    /// OUTA and OUTB only, on D2 and D3: the motor's own terminals take
    /// ordinary hooks where the driver's 1.27 mm pins do not.
    Outputs,
}

#[derive(Debug, Clone, Args)]
pub struct TransientArgs {
    /// Session name under private/captures/.
    pub name: String,
    #[arg(long, default_value_t = 10)]
    pub cycles: u32,
    #[arg(long, value_enum, default_value = "boot")]
    pub mode: TransientMode,
    #[arg(long, value_enum, default_value = "driver")]
    pub probes: Probes,
    /// Capture length per cycle, arming included.
    #[arg(long, default_value_t = 8.0)]
    pub seconds: f64,
    #[arg(long, default_value_t = 2_000_000)]
    pub samplerate: u64,
}

/// `rom-inspect`: boot stock and sample its ROM through its console.
#[derive(Debug, Clone, Args)]
pub struct RomInspectArgs {
    /// Session name under private/captures/.
    pub name: String,
    /// Pinned SDK ROM image to compare the sampled words with (repeatable),
    /// root-relative.
    #[arg(long = "candidate-rom", required = true)]
    pub candidate_rom: Vec<PathBuf>,
    /// The boot capture's length: long enough for stock to reach its
    /// console before the inspection opens the port.
    #[arg(long, default_value_t = 20.0)]
    pub seconds: f64,
}

/// A session name is one plain path component.
pub fn check_name(name: &str) -> Result<()> {
    require(plain_name(name), || {
        CommandsError::BadName { name: name.into() }.into()
    })
}

/// One command line: the subcommand words, then each group of flags.
fn argv(head: &[&str], parts: &[&[String]]) -> Vec<String> {
    head.iter()
        .map(|word| (*word).to_string())
        .chain(parts.iter().flat_map(|part| part.iter().cloned()))
        .collect()
}

/// What the UART lines do while the feeder is off.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum UartWhenOff {
    /// Relays 3 and 4 open, so the adapter cannot back-power the feeder.
    #[default]
    Isolated,
    /// Left connected, as the bench was before they were wired: to compare.
    Connected,
}

pub struct Bench {
    pub root: PathBuf,
    /// The model of the device on the bench, which every `wellfed-flash`
    /// writer and `wellfed-build` build is told (`--device`).
    pub device: Model,
    pub config: Config,
    pub relay: Relay,
    pub status: Status,
    pub tools: Box<dyn Tools>,
    /// The registry the tools' children run in; the suite registers the
    /// servers it starts there too, so a cancel reaches them.
    pub children: Children,
    /// The operator's interrupt, which every wait of a sequence ends on.
    pub cancel: Cancel,
    /// Interruptible in production ([`crate::sleep`]); instant in tests.
    pub sleep: fn(&Cancel, Duration) -> Result<()>,
    /// How long the feeder stays off before a cold boot.
    pub off_time: Duration,
    pub uart_when_off: UartWhenOff,
    /// What the sequence left in each bank, as it believes: the bench
    /// server records these in its bank ledger once the job ends.
    pub written: Vec<Written>,
}

/// Whether the owner is watching, which a build that drives the motor
/// after boot needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Attendance {
    Attended,
    Unattended,
}

impl Attendance {
    /// `--attended-motion`, as the command line gives it.
    pub(crate) fn from_flag(attended_motion: bool) -> Self {
        if attended_motion {
            Self::Attended
        } else {
            Self::Unattended
        }
    }
}

/// The longest wait the command line may ask for. Far past any bench run,
/// and short enough that adding it to `Instant::now()` cannot overflow.
const LONGEST: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// A length the command line gives in seconds, refused unless it is one a
/// wait can take: finite, not negative and at most [`LONGEST`].
pub fn seconds_arg(flag: &'static str, value: f64) -> Result<Duration> {
    length(flag, value, value)
}

/// As [`seconds_arg`], for a length given in minutes.
pub(crate) fn minutes_arg(flag: &'static str, value: f64) -> Result<Duration> {
    length(flag, value, value * 60.0)
}

fn length(flag: &'static str, value: f64, seconds: f64) -> Result<Duration> {
    Duration::try_from_secs_f64(seconds)
        .ok()
        .filter(|span| *span <= LONGEST)
        .ok_or_else(|| CommandsError::BadLength { flag, value }.into())
}

impl Bench {
    pub fn rel(&self, path: &Path) -> String {
        relative(&self.root, path)
    }

    /// Wait `span` unless the operator interrupts first.
    pub fn pause(&self, span: Duration) -> Result<()> {
        (self.sleep)(&self.cancel, span)
    }

    pub fn set(&mut self, channel: u8, switch: Switch) -> Result<()> {
        self.relay.set(channel, switch)?;
        // The state records what the relay acknowledged.
        self.status.relay(channel, switch.is_on());
        Ok(())
    }

    pub fn get(&mut self, channel: u8) -> Result<bool> {
        let on = self.relay.get(channel)?;
        self.status.relay(channel, on);
        Ok(on)
    }

    /// Cut power, then release the strap and isolate the UART, with no
    /// wait: safe in cleanup paths, and the kill switch of unattended
    /// motion. Power goes first and every channel is tried even when one
    /// fails, so a strap or UART relay fault never leaves the feeder
    /// powered; the first failure is returned once all were tried.
    pub fn cut_power(&mut self) -> Result<()> {
        let mut channels = vec![POWER, STRAP];
        if self.uart_when_off == UartWhenOff::Isolated {
            channels.extend(UART);
        }

        let mut first = None;
        for channel in channels {
            if let Err(error) = self.set(channel, Switch::Off) {
                first.get_or_insert(error);
            }
        }
        first.map_or(Ok(()), Err)
    }

    /// Connect the UART, then power the feeder. The lines go live a relay's
    /// switching time before the supply, so the ROM's first bytes are kept.
    pub fn power_on(&mut self) -> Result<()> {
        self.connect_uart()?;
        self.set(POWER, Switch::On)
    }

    fn connect_uart(&mut self) -> Result<()> {
        for channel in UART {
            self.set(channel, Switch::On)?;
        }
        Ok(())
    }

    fn power_off(&mut self, wait: Duration) -> Result<()> {
        self.cut_power()?;
        self.pause(wait)
    }

    /// Captures never reuse a directory; refuse before any relay change.
    pub fn fresh(&self, output: &Path) -> Result<()> {
        require(!output.exists(), || {
            CommandsError::NameTaken {
                path: self.rel(output),
            }
            .into()
        })
    }

    fn start_observer(&mut self, output: &Path, length: Duration) -> Result<Box<dyn Observer>> {
        let observer = self.tools.observe(&self.config.uart_port, output, length)?;
        self.status.capture(Some(output), length.as_secs_f64());
        Ok(observer)
    }

    fn finish_observer(&mut self, observer: &mut dyn Observer) -> Result<Observation> {
        let result = observer.finish();
        self.status.capture(None, 0.0);
        result
    }

    /// Before a write: a cancel already taken stops the job here, with
    /// nothing written, so its bank keeps what the ledger says.
    fn not_cancelled(&self) -> Result<()> {
        if self.cancel.is_cancelled() {
            return Err(Error::Interrupted);
        }
        Ok(())
    }

    /// Run one `wellfed-flash` command; its exit status. A cancel before it
    /// keeps it from starting, and one during it stops the sequence even if
    /// the child finished, so no step follows.
    fn run(&mut self, args: &[String]) -> Result<i32> {
        if self.cancel.is_cancelled() {
            return Err(Error::Interrupted);
        }
        say!("+ wellfed-flash {}", args.join(" "));
        self.status
            .step(&format!("wellfed-flash {} {}", args[0], args[1]));
        let code = self.tools.flash(args, &mut self.status)?;

        if self.cancel.is_cancelled() {
            return Err(Error::Interrupted);
        }
        Ok(code)
    }

    fn run_all(&mut self, steps: &[Vec<String>]) -> Result<bool> {
        for step in steps {
            if self.run(step)? != 0 {
                say!("{STOPPED_POWERED}");
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn on(&mut self) -> Result<u8> {
        self.set(STRAP, Switch::Off)?;
        self.power_on()?;
        Ok(0)
    }

    pub fn off(&mut self) -> Result<u8> {
        self.cut_power()?;
        Ok(0)
    }

    /// Record the running feeder without touching power. A powered feeder's
    /// UART is connected first; an unpowered one's stays isolated.
    pub fn capture(&mut self, name: &str, length: Duration) -> Result<u8> {
        let output = self.root.join(CAPTURES).join(name);
        self.fresh(&output)?;
        if self.get(POWER)? {
            self.connect_uart()?;
        }
        let mut observer = self.start_observer(&output, length)?;
        let result = self.finish_observer(observer.as_mut())?;
        say!("Captured {} bytes in {}", result.bytes, self.rel(&output));
        Ok(0)
    }

    /// Type one press to the running `virtual-panel` build: the chord held
    /// for `hold_ms`, through the same gestures as a finger. Never touches
    /// power; the press is sent once, and a missing echo stops the job.
    pub fn press(&mut self, name: &str, chord: Buttons, hold_ms: u32) -> Result<u8> {
        if self.typed_press(name, chord, hold_ms, PressStep::Own)? {
            return Ok(0);
        }
        say!("{STOPPED_POWERED}");
        Ok(1)
    }

    /// One press inside another job's sequence (the install's answers to
    /// its panel prompts): whether it echoed and released. It adds no step
    /// row of its own and says nothing about stopping, since the job that
    /// asked decides what a failed press means.
    pub fn press_within(&mut self, name: &str, chord: Buttons, hold_ms: u32) -> Result<bool> {
        self.typed_press(name, chord, hold_ms, PressStep::Within)
    }

    fn typed_press(
        &mut self,
        name: &str,
        chord: Buttons,
        hold_ms: u32,
        step: PressStep,
    ) -> Result<bool> {
        if !self.get(POWER)? {
            return Err(CommandsError::PressUnpowered.into());
        }
        let output = self.root.join(CAPTURES).join(name);
        self.fresh(&output)?;
        self.connect_uart()?;

        let args = argv(
            &["read", "press"],
            &[
                &["--port".into(), self.config.uart_port.clone()],
                &self.out(&output),
                &[
                    "--chord".into(),
                    chord.to_string(),
                    "--hold-ms".into(),
                    hold_ms.to_string(),
                ],
            ],
        );
        let code = match step {
            PressStep::Own => self.run(&args)?,
            PressStep::Within => {
                say!("+ wellfed-flash {}", args.join(" "));
                let code = self.tools.flash(&args, &mut self.status)?;
                if self.cancel.is_cancelled() {
                    return Err(Error::Interrupted);
                }
                code
            }
        };
        if code != 0 {
            return Ok(false);
        }
        say!("Pressed {chord} for {hold_ms} ms; the gesture is in the capture");
        Ok(true)
    }

    /// Cold boot into the installed application while recording it from the start.
    pub fn boot(&mut self, output: &Path, length: Duration) -> Result<Observation> {
        self.status.step("boot capture");
        self.fresh(output)?;
        self.power_off(self.off_time)?;
        let mut observer = self.start_observer(output, length)?;
        if let Err(error) = self.power_on() {
            observer.kill();
            return Err(error);
        }

        let result = self.finish_observer(observer.as_mut())?;
        require(!result.download_banner_seen, || {
            CommandsError::ReachedLoader
        })?;

        say!(
            "Boot captured: {} bytes in {}; fault marker {}",
            result.bytes,
            self.rel(output),
            if result.fault_marker_seen {
                "seen"
            } else {
                "not seen"
            }
        );
        Ok(result)
    }

    /// Cold boot with the strap held; leave the feeder powered in the ROM loader.
    pub fn loader(&mut self, output: &Path) -> Result<Observation> {
        self.status.step("loader entry");
        self.fresh(output)?;
        self.power_off(self.off_time)?;
        let mut observer = self.start_observer(output, LOADER_CAPTURE)?;

        // A failed entry kills the observer and tries to cut power rather
        // than leave the feeder strapped or half in the loader.
        let entered = (|| {
            self.set(STRAP, Switch::On)?;
            self.power_on()?;
            self.pause(STRAP_HOLD)?;
            self.set(STRAP, Switch::Off)?;
            let result = self.finish_observer(observer.as_mut())?;
            require(
                result.download_banner_seen
                    && result.longest_nak_run >= LOADER_NAK_RUN
                    && !result.fault_marker_seen
                    && result.stock_startup_markers == 0,
                || CommandsError::LoaderNotConfirmed {
                    capture: self.rel(output),
                },
            )?;
            Ok(result)
        })();

        match entered {
            Ok(result) => {
                say!(
                    "Loader confirmed: banner and {} NAKs in {}",
                    result.nak_bytes,
                    self.rel(output)
                );
                Ok(result)
            }

            Err(error) => {
                observer.kill();
                self.status.capture(None, 0.0);
                match self.cut_power() {
                    Ok(()) => eprintln!("Feeder powered off"),
                    Err(cut) => eprintln!("Could not power the feeder off: {cut}"),
                }
                Err(error)
            }
        }
    }

    /// Repeated captured power-ons (or loader entries) for the driver-input check.
    ///
    /// Each cycle: power off, start the analyzer, wait for it to arm, power on
    /// (holding the strap for a loader entry), let the capture finish, power off.
    /// Never opens the UART; never writes flash.
    pub fn transients(&mut self, args: &TransientArgs) -> Result<u8> {
        let out = self.root.join(CAPTURES).join(&args.name);
        self.fresh(&out)?;
        require(
            args.cycles >= 1 && args.seconds > ANALYZER_ARMING.as_secs_f64(),
            || CommandsError::TransientBounds,
        )?;
        let length = seconds_arg("--seconds", args.seconds)?;
        require(self.tools.sigrok_available(), || {
            CommandsError::ToolMissing { tool: "sigrok-cli" }
        })?;
        fs::create_dir_all(&out)?;

        let channels: BTreeMap<String, u8> =
            self.config.analyzer_channels.clone().unwrap_or_else(|| {
                let defaults: &[(&str, u8)] = match args.probes {
                    Probes::Driver => &ANALYZER_DRIVER_CHANNELS,
                    Probes::Outputs => &ANALYZER_OUTPUT_CHANNELS,
                };
                defaults
                    .iter()
                    .map(|(name, channel)| ((*name).to_string(), *channel))
                    .collect()
            });
        let mut manifest = TransientsManifest {
            schema_version: 1,
            mode: match args.mode {
                TransientMode::Boot => transients::TransientMode::Boot,
                TransientMode::Loader => transients::TransientMode::Loader,
            },
            samplerate: args.samplerate,
            seconds: args.seconds,
            arming_seconds: ANALYZER_ARMING.as_secs_f64(),
            channels: channels.into_iter().map(|(k, v)| (k, v.into())).collect(),
            cycles: Vec::new(),
        };

        // Power off whether or not the cycles succeeded, then report the
        // cycles' failure ahead of the power-off's.
        let session = self.transient_cycles(args, length, &out, &mut manifest);
        let cut = self.cut_power();
        session?;
        cut?;

        say!(
            "Transient captures in {}; audit them with wellfed-audit transients",
            self.rel(&out)
        );
        Ok(0)
    }

    fn transient_cycles(
        &mut self,
        args: &TransientArgs,
        length: Duration,
        out: &Path,
        manifest: &mut TransientsManifest,
    ) -> Result<()> {
        for n in 1..=args.cycles {
            self.status.step(&format!("cycle {n}/{}", args.cycles));
            self.power_off(self.off_time)?;
            let name = format!("cycle-{n:03}.bin");

            let (mut analyzer, started) =
                self.armed_analyzer(&out.join(&name), length, args.samplerate)?;
            let cycle = self.transient_cycle(args, length, analyzer.as_mut());
            if cycle.is_err() {
                analyzer.kill();
            }
            let powered = cycle?;

            // Rewritten after every cycle, so a stopped session keeps the
            // manifest of the cycles it finished.
            let bytes = fs::metadata(out.join(&name)).map_or(0, |m| m.len());
            manifest.cycles.push(TransientCycle {
                file: name,
                bytes,
                power_on_after_launch_s: (powered.duration_since(started).as_secs_f64() * 1000.0)
                    .round()
                    / 1000.0,
            });
            fs::write(
                out.join("manifest.json"),
                serde_json::to_string_pretty(manifest)?,
            )?;
            say!("Cycle {n}/{}: {bytes} bytes", args.cycles);
        }
        Ok(())
    }

    /// The analyzer, still running after its arming pause, and when the
    /// one that runs was launched. A start that exits during arming is
    /// launched again, up to [`ANALYZER_STARTS`]: nothing is powered yet.
    fn armed_analyzer(
        &mut self,
        output: &Path,
        length: Duration,
        samplerate: u64,
    ) -> Result<(Box<dyn Analyzer>, Instant)> {
        let mut start = 1;
        loop {
            let launched = Instant::now();
            let mut analyzer = self.tools.analyzer(output, length, samplerate)?;
            if let Err(error) = self.pause(ANALYZER_ARMING) {
                analyzer.kill();
                return Err(error);
            }
            if analyzer.running() {
                return Ok((analyzer, launched));
            }

            let stderr = analyzer.stderr();
            if start == ANALYZER_STARTS {
                return Err(CommandsError::AnalyzerExited { stderr }.into());
            }
            say!(
                "Analyzer exited before power-on (start {start}/{ANALYZER_STARTS}): {}; starting it again",
                stderr.trim()
            );
            start += 1;
        }
    }

    /// One analyzer capture across a power-on; when power came on.
    fn transient_cycle(
        &mut self,
        args: &TransientArgs,
        length: Duration,
        analyzer: &mut dyn Analyzer,
    ) -> Result<Instant> {
        let powered = Instant::now();
        if args.mode == TransientMode::Loader {
            self.set(STRAP, Switch::On)?;
            self.power_on()?;
            self.pause(STRAP_HOLD)?;
            self.set(STRAP, Switch::Off)?;
        } else {
            self.power_on()?;
        }

        let code = analyzer.wait(length + Duration::from_secs(30))?;
        let Some(code) = code else {
            return Err(CommandsError::AnalyzerTimeout.into());
        };
        self.power_off(Duration::ZERO)?;
        require(code == 0, || CommandsError::AnalyzerFailed {
            stderr: analyzer.stderr(),
        })?;
        Ok(powered)
    }

    /// Loader, bounded read, then a full 8 MiB read in the same helper session.
    pub fn dump(&mut self, name: &str) -> Result<u8> {
        let p = Paths::new(&self.root, name);
        for path in [&p.bounded, &p.full] {
            self.fresh(path)?;
        }
        self.loader(&p.loader)?;

        let read = self.read_args();
        let steps = [
            argv(&["read", "small"], &[&read, &self.out(&p.bounded)]),
            argv(
                &["read", "full"],
                &[
                    &read,
                    &self.out(&p.full),
                    &["--small-report".into(), self.report(&p.bounded)],
                ],
            ),
        ];
        if !self.run_all(&steps)? {
            return Ok(1);
        }

        say!(
            "Full read in {}; compare it before relying on it",
            self.rel(&p.full)
        );
        Ok(0)
    }

    /// Cold boot into stock, capturing until its console is up, then sample
    /// the ROM through that console (`read inspect-rom`) into
    /// `private/captures/NAME/rom`: the report that names the model whose
    /// stock console answered, which every later read of that model needs.
    /// The capture ends before the inspection opens the port, so the UART
    /// has one owner at a time. Nothing is retried, and stock stays
    /// running afterwards.
    pub fn rom_inspect(&mut self, args: &RomInspectArgs) -> Result<u8> {
        let out = self.root.join(CAPTURES).join(&args.name);
        self.fresh(&out)?;
        let length = seconds_arg("--seconds", args.seconds)?;
        let (boot, rom) = (out.join("boot"), out.join("rom"));
        self.boot(&boot, length)?;

        let mut inspect = vec![
            "--port".into(),
            self.config.uart_port.clone(),
            "--output".into(),
            self.path_arg(&rom),
        ];
        for candidate in &args.candidate_rom {
            inspect.extend([
                "--candidate-rom".into(),
                self.path_arg(&self.root.join(candidate)),
            ]);
        }
        if !self.run_all(&[argv(&["read", "inspect-rom"], &[&inspect])])? {
            return Ok(1);
        }

        say!(
            "ROM report in {}; check the model it names before a read relies on it",
            self.rel(&rom.join("report.json"))
        );
        Ok(0)
    }

    fn path_arg(&self, path: &Path) -> String {
        path.display().to_string()
    }

    fn out(&self, path: &Path) -> Vec<String> {
        vec!["--output".into(), self.path_arg(path)]
    }

    fn report(&self, directory: &Path) -> String {
        self.path_arg(&directory.join("report.json"))
    }

    fn read_args(&self) -> Vec<String> {
        vec![
            "--device".into(),
            self.device.name().into(),
            "--port".into(),
            self.config.uart_port.clone(),
            "--rom-report".into(),
            self.path_arg(&self.root.join(&self.config.rom_report)),
        ]
    }

    /// `--device <model>`, which the flash writers and the builds take.
    pub(crate) fn device_args(&self) -> [String; 2] {
        ["--device".into(), self.device.name().into()]
    }

    fn evidence_args(&self, p: &Paths) -> Vec<String> {
        vec![
            "--device".into(),
            self.device.name().into(),
            "--baseline".into(),
            self.path_arg(&self.root.join(&self.config.baseline)),
            "--rom-report".into(),
            self.path_arg(&self.root.join(&self.config.rom_report)),
            "--small-report".into(),
            self.report(&p.bounded),
            "--port".into(),
            self.config.uart_port.clone(),
        ]
    }

    fn update_args(&self, args: &UpdateArgs, p: &Paths) -> Result<Vec<String>> {
        // The PLWF106's bank-1 update takes bank 1's original bytes from the
        // pinned baseline, and wellfed-flash refuses it when bank 1 no longer
        // equals them; the PLAF103's bank 1 always names what it holds.
        let original_bank = args.bank == 0 || self.device == Model::Plwf106;
        require(args.source.is_some() || original_bank, || {
            CommandsError::SourceRequired
        })?;
        require(args.bank != 0 || args.bank1_source.is_some(), || {
            CommandsError::Bank1SourceRequired
        })?;
        require(args.bank == 0 || args.bank1_source.is_none(), || {
            CommandsError::Bank1SourceUnused
        })?;

        let mut out = self.evidence_args(p);
        out.extend([
            "--target".into(),
            self.path_arg(&args.target),
            "--bank".into(),
            args.bank.to_string(),
        ]);
        if let Some(source) = &args.source {
            out.extend(["--source".into(), self.path_arg(source)]);
        } else if let Some(read) = &args.original_read {
            out.extend(["--original-read".into(), self.path_arg(read)]);
        }
        if args.restore_marker {
            out.push("--restore-marker".into());
        }
        if args.interrupted {
            out.push("--interrupted".into());
        }
        Ok(out)
    }

    fn give_way_args(&self, args: &GiveWayArgs, p: &Paths) -> Vec<String> {
        let mut out = self.evidence_args(p);
        out.extend(["--fallback".into(), self.path_arg(&args.fallback)]);
        out
    }

    /// A target whose manifest names a motion policy drives the motor after
    /// boot. The target is root-relative, as `wellfed-flash` resolves it,
    /// whatever directory the bench runs in. Only a stock review has no
    /// manifest; a review or manifest that cannot be read is refused rather
    /// than assumed motionless, and so is a target with no review at all.
    fn require_attended_motion(&self, args: &UpdateArgs) -> Result<()> {
        #[derive(Deserialize)]
        struct Reviewed {
            image_kind: ImageKind,
        }

        let target = self.root.join(&args.target);
        let unreadable = |path: &Path| {
            let file = self.rel(path);
            move |error| CommandsError::MotionPolicyUnreadable { file, error }
        };
        let review = target.join("review.json");
        let reviewed: Reviewed = wellfed_formats::load(&review).map_err(unreadable(&review))?;
        if reviewed.image_kind == ImageKind::Stock {
            return Ok(());
        }

        let path = target.join("build-manifest.json");
        let manifest: BuildManifest = wellfed_formats::load(&path).map_err(unreadable(&path))?;
        self.attended_motion(&manifest, Attendance::from_flag(args.attended_motion))
    }

    /// Refuse a build manifest naming a motion policy unless attended.
    fn attended_motion(&self, manifest: &BuildManifest, attendance: Attendance) -> Result<()> {
        let Some(policy) = manifest
            .startup_motion_policy
            .as_deref()
            .filter(|p| !p.is_empty())
        else {
            return Ok(());
        };

        require(attendance == Attendance::Attended, || {
            CommandsError::MotionNotAttended {
                policy: policy.into(),
            }
        })?;
        say!("Attended motion image ({policy}): the motor will move after boot");
        Ok(())
    }

    fn prepared(&self, p: &Paths) -> Result<(Vec<u8>, String)> {
        let raw = fs::read(&p.plan)?;
        let sha = sha256(&raw);
        Ok((raw, sha))
    }

    /// Loader, helper bounded read, inspection and plan; stop for plan review.
    pub fn flash_prepare(&mut self, args: &UpdateArgs) -> Result<u8> {
        let Some((sha, plan)) = self.update_plan(args)? else {
            return Ok(1);
        };
        say!(
            "Plan {}: bank {}, {} sectors staged, {} erases, target {}",
            self.rel(&Paths::new(&self.root, &args.name).plan),
            plan.bank,
            plan.ordered_sectors.len(),
            plan.erase_sectors,
            plan.target_ota_sha256,
        );

        let next = execute_line(args, &sha);
        say!("Review it, then: {next}");
        self.status.next(next);
        Ok(0)
    }

    /// The update plan's SHA-256 and operation once prepared; `None` if a
    /// step failed.
    fn update_plan(&mut self, args: &UpdateArgs) -> Result<Option<(String, DevelopmentPlan)>> {
        let p = Paths::new(&self.root, &args.name);
        self.require_attended_motion(args)?;
        let update = self.update_args(args, &p)?;
        for path in [&p.loader, &p.bounded, &p.inspect, &p.plan] {
            self.fresh(path)?;
        }
        if args.bank1_source.is_some() {
            self.fresh(&bank0::binding(&self.root, &args.name))?;
        }
        self.helper_session(&p)?;

        let looks = [
            argv(
                &["read", "small"],
                &[&self.read_args(), &self.out(&p.bounded)],
            ),
            argv(&["update", "inspect"], &[&update, &self.out(&p.inspect)]),
        ];
        if !self.run_all(&looks)? {
            return Ok(None);
        }
        // Bank 1's review is checked against what the inspection read, and
        // bound, before a plan exists to execute.
        if let Some(bank1) = &args.bank1_source {
            self.bind_bank1(&args.name, bank1)?;
        }
        let prepare = argv(
            &["update", "prepare"],
            &[
                &update,
                &["--inspection".into(), self.report(&p.inspect)],
                &self.out(&p.plan),
            ],
        );
        if !self.run_all(&[prepare])? {
            return Ok(None);
        }

        let (raw, sha) = self.prepared(&p)?;
        let Prepared {
            operation_plan: plan,
        } = serde_json::from_slice::<Prepared<DevelopmentPlan>>(&raw)?;
        Ok(Some((sha, plan)))
    }

    /// Execute a reviewed plan in the still-powered helper session, then boot.
    pub fn flash_execute(&mut self, args: &ExecuteArgs) -> Result<u8> {
        let boot = seconds_arg("--boot-seconds", args.boot_seconds)?;
        let p = Paths::new(&self.root, &args.update.name);
        self.require_attended_motion(&args.update)?;
        let update = self.update_args(&args.update, &p)?;
        require(p.plan.is_file() && !p.execute.exists(), || {
            CommandsError::PlanRequired
        })?;
        if let Some(bank1) = &args.update.bank1_source {
            self.require_bank1_bound(&args.update.name, bank1)?;
        }
        self.require_unbroken_power(&p)?;

        // Progress estimate only; the updater reads and checks the plan itself.
        if let Some(plan) = fs::read(&p.plan)
            .ok()
            .and_then(|raw| serde_json::from_slice::<Prepared<DevelopmentPlan>>(&raw).ok())
        {
            let (tx, rx) = update_traffic(&plan.operation_plan);
            self.status.transfer(Some(&p.execute), tx, rx);
        }

        let command = argv(
            &["update", "execute"],
            &[
                &update,
                &[
                    "--inspection".into(),
                    self.report(&p.inspect),
                    "--plan".into(),
                    self.path_arg(&p.plan),
                    "--expected-plan-sha256".into(),
                    args.expected_plan_sha256.clone(),
                ],
                &self.out(&p.execute),
            ],
        );
        // A verified bank-0 write exits 3: written, but not yet recorded.
        let written = if args.update.bank == 0 { 3 } else { 0 };
        self.not_cancelled()?;
        match self.run(&command) {
            Ok(code) if code == written => {}
            Ok(0) => {
                // Only an older wellfed-flash says 0 for a bank-0 write.
                if let (Some(bank), Some(content)) = (
                    Bank::of(args.update.bank),
                    installed(&self.root, &args.update.target),
                ) {
                    self.wrote(bank, content, Source::FlashExecute);
                }
                say!("{}", CommandsError::FlashSkew);
                return Ok(self.unfinished(
                    "bank 0 is likely written and verified, unrecorded; bank 1 is unchanged \
                     and refuses every update",
                    &bank0::skew_steps(&args.update),
                ));
            }
            Ok(_) => {
                say!("Update stopped; the feeder stays powered and is not booted. No retry.");
                self.stopped_write(args.update.bank, Source::FlashExecute);
                return Ok(1);
            }
            Err(Error::Interrupted) => {
                self.stopped_write(args.update.bank, Source::FlashExecute);
                return Err(Error::Interrupted);
            }
            Err(error) => return Err(error),
        }

        if let (Some(bank), Some(content)) = (
            Bank::of(args.update.bank),
            installed(&self.root, &args.update.target),
        ) {
            self.wrote(bank, content, Source::FlashExecute);
        }
        self.status.transfer(None, 0, 0);
        if args.update.bank == 0 {
            return self.finish_bank0(args);
        }
        self.boot(&p.boot, boot)?;
        Ok(0)
    }

    /// The loader, for a helper session a later execute writes in, with
    /// the board's power-cut count recorded for that execute to check.
    fn helper_session(&mut self, p: &Paths) -> Result<()> {
        self.fresh(&p.power)?;
        // A board that cannot be checked is refused before a relay moves.
        self.relay.power_cuts()?;
        self.loader(&p.loader)?;

        let power_cuts = self.relay.power_cuts()?;
        if let Some(parent) = p.power.parent() {
            fs::create_dir_all(parent)?;
        }
        let record = serde_json::to_vec(&PowerAtLoader { power_cuts })?;
        wellfed_formats::private::write_new_whole(&p.power, &record)?;
        Ok(())
    }

    /// The relays show the supply on and the strap released, and the
    /// Wellfed Bench board has made no power cut since loader entry: the
    /// Wellfed Bench board's RECOVER button cycles the power by itself and
    /// leaves the relays as they were. It cannot rule out a brownout.
    fn require_unbroken_power(&mut self, p: &Paths) -> Result<()> {
        let powered = self.get(POWER)?;
        let strap = self.get(STRAP)?;
        require(powered && !strap, || CommandsError::PowerCycled)?;

        // No record is a session this build did not enter: start over.
        let at_loader = fs::read(&p.power).map_err(|_| CommandsError::PowerCycled)?;
        let PowerAtLoader { power_cuts } = serde_json::from_slice(&at_loader)?;
        require(self.relay.power_cuts()? == power_cuts, || {
            CommandsError::PowerCycled.into()
        })
    }

    /// Loader, bounded read, give-way inspection and plan; stop for review.
    pub fn give_way_prepare(&mut self, args: &GiveWayArgs) -> Result<u8> {
        let Some(sha) = self.give_way_plan(args)? else {
            return Ok(1);
        };

        let next = format!(
            "wellfed-bench give-way-execute {} --fallback {} --expected-plan-sha256 {sha}",
            args.name,
            args.fallback.display()
        );
        say!("Review it, then: {next}");
        self.status.next(next);
        Ok(0)
    }

    /// The give-way plan's SHA-256 once prepared; `None` if a step failed.
    fn give_way_plan(&mut self, args: &GiveWayArgs) -> Result<Option<String>> {
        let p = Paths::new(&self.root, &args.name);
        for path in [&p.loader, &p.bounded, &p.inspect, &p.plan] {
            self.fresh(path)?;
        }
        self.helper_session(&p)?;

        let give_way = self.give_way_args(args, &p);
        let steps = [
            argv(
                &["read", "small"],
                &[&self.read_args(), &self.out(&p.bounded)],
            ),
            argv(
                &["give-way", "inspect"],
                &[&give_way, &self.out(&p.inspect)],
            ),
            argv(
                &["give-way", "prepare"],
                &[
                    &give_way,
                    &["--inspection".into(), self.report(&p.inspect)],
                    &self.out(&p.plan),
                ],
            ),
        ];
        if !self.run_all(&steps)? {
            return Ok(None);
        }

        let (raw, sha) = self.prepared(&p)?;
        let Prepared {
            operation_plan: plan,
        } = serde_json::from_slice::<Prepared<GiveWayPlan>>(&raw)?;
        say!(
            "Give-way plan {}: clears bank 1's first marker byte; bank 0 is {}",
            self.rel(&p.plan),
            // The fountain's fallback is its stock bank 0, not an image.
            plan.fallback_ota_sha256.unwrap_or(plan.bank0_region_sha256),
        );
        Ok(Some(sha))
    }

    /// Execute a reviewed give-way in the still-powered helper session, then boot.
    pub fn give_way_execute(&mut self, args: &GiveWayExecuteArgs) -> Result<u8> {
        let boot = seconds_arg("--boot-seconds", args.boot_seconds)?;
        let p = Paths::new(&self.root, &args.give_way.name);
        require(p.plan.is_file() && !p.execute.exists(), || {
            CommandsError::PlanRequired
        })?;
        self.require_unbroken_power(&p)?;

        let command = argv(
            &["give-way", "execute"],
            &[
                &self.give_way_args(&args.give_way, &p),
                &[
                    "--inspection".into(),
                    self.report(&p.inspect),
                    "--plan".into(),
                    self.path_arg(&p.plan),
                    "--expected-plan-sha256".into(),
                    args.expected_plan_sha256.clone(),
                ],
                &self.out(&p.execute),
            ],
        );
        self.not_cancelled()?;
        match self.run(&command) {
            Ok(0) => {}
            Ok(_) => {
                say!("Give-way stopped; the feeder stays powered and is not booted. No retry.");
                self.stopped_write(1, Source::GiveWay);
                return Ok(1);
            }
            // The flash client ran and stopped on the cancel.
            Err(Error::Interrupted) => {
                self.stopped_write(1, Source::GiveWay);
                return Err(Error::Interrupted);
            }
            Err(error) => return Err(error),
        }

        self.wrote(Bank::One, Content::Invalid, Source::GiveWay);
        self.boot(&p.boot, boot)?;
        Ok(0)
    }

    /// The pinned original stock back into bank 1 over UART (the restore
    /// procedure), stopping for review at each plan. With `--fallback`, bank
    /// 1 still holds a valid image, so this prepares the give-way to that
    /// bank-0 review first; stock recovery refuses a valid target marker.
    /// Otherwise: loader, bounded and full reads, then the recovery plan.
    pub fn stock_prepare(&mut self, args: &StockArgs) -> Result<u8> {
        if let Some(fallback) = &args.fallback {
            let give_way = GiveWayArgs {
                name: give_way_name(&args.name),
                fallback: fallback.clone(),
            };
            let Some(sha) = self.give_way_plan(&give_way)? else {
                return Ok(1);
            };
            let next = format!(
                "wellfed-bench stock-execute {} --fallback {} --expected-plan-sha256 {sha}",
                args.name,
                fallback.display()
            );
            say!("Review it, then: {next}");
            self.status.next(next);
            return Ok(0);
        }

        let p = Paths::new(&self.root, &args.name);
        for path in [&p.loader, &p.bounded, &p.full, &p.plan] {
            self.fresh(path)?;
        }
        self.helper_session(&p)?;

        let read = self.read_args();
        let reads = [
            argv(&["read", "small"], &[&read, &self.out(&p.bounded)]),
            argv(
                &["read", "full"],
                &[
                    &read,
                    &self.out(&p.full),
                    &["--small-report".into(), self.report(&p.bounded)],
                ],
            ),
        ];
        if !self.run_all(&reads)? {
            return Ok(1);
        }

        let current = p.full.join("flash-read.bin");
        require(!bank1_marker_valid(&current)?, || {
            CommandsError::Bank1Valid {
                name: args.name.clone(),
            }
        })?;

        let recover = argv(
            &["stock-recovery", "prepare"],
            &[
                &self.evidence_args(&p),
                &["--current".into(), self.path_arg(&current)],
                &self.out(&p.plan),
            ],
        );
        if self.run(&recover)? != 0 {
            return Ok(1);
        }

        let (raw, sha) = self.prepared(&p)?;
        let reviewed: StockRecoveryReviewed = serde_json::from_slice(&raw)?;
        let plan = reviewed.recovery_plan;
        say!(
            "Stock plan {}: the pinned original into bank {}, {} sectors changed",
            self.rel(&p.plan),
            plan.target_bank,
            plan.changed_sector_count,
        );

        let next = format!(
            "wellfed-bench stock-execute {} --expected-plan-sha256 {sha}",
            args.name
        );
        say!("Review it, then: {next} (about an hour: three 8 MiB reads)");
        self.status.next(next);
        Ok(0)
    }

    /// Execute what `stock-prepare` planned. With `--fallback`, the give-way,
    /// then boot bank 0 and point at the recovery plan. Otherwise the stock
    /// recovery, single-use in the still-powered helper session, then a boot
    /// that must show stock's banner. The capture holds the unit's secrets.
    pub fn stock_execute(&mut self, args: &StockExecuteArgs) -> Result<u8> {
        let boot = seconds_arg("--boot-seconds", args.boot_seconds)?;
        let name = &args.stock.name;
        if let Some(fallback) = &args.stock.fallback {
            let give_way = GiveWayExecuteArgs {
                give_way: GiveWayArgs {
                    name: give_way_name(name),
                    fallback: fallback.clone(),
                },
                expected_plan_sha256: args.expected_plan_sha256.clone(),
                boot_seconds: args.boot_seconds,
            };
            let code = self.give_way_execute(&give_way)?;
            if code == 0 {
                let next = format!("wellfed-bench stock-prepare {name}");
                say!("Bank 0 runs; bank 1 can take stock now: {next}");
                self.status.next(next);
            }
            return Ok(code);
        }

        let p = Paths::new(&self.root, name);
        require(p.plan.is_file() && !p.execute.exists(), || {
            CommandsError::PlanRequired
        })?;
        self.require_unbroken_power(&p)?;

        let command = argv(
            &["stock-recovery", "execute"],
            &[
                &self.evidence_args(&p),
                &[
                    "--current".into(),
                    self.path_arg(&p.full.join("flash-read.bin")),
                    "--plan".into(),
                    self.path_arg(&p.plan),
                    "--expected-plan-sha256".into(),
                    args.expected_plan_sha256.clone(),
                ],
                &self.out(&p.execute),
            ],
        );
        self.not_cancelled()?;
        match self.run(&command) {
            Ok(0) => {}
            Ok(_) => {
                say!(
                    "Stock recovery stopped; the feeder stays powered and is not booted. No retry."
                );
                self.stopped_write(1, Source::StockExecute);
                return Ok(1);
            }
            // The flash client ran and stopped on the cancel.
            Err(Error::Interrupted) => {
                self.stopped_write(1, Source::StockExecute);
                return Err(Error::Interrupted);
            }
            Err(error) => return Err(error),
        }

        self.wrote(Bank::One, Content::Stock, Source::StockExecute);
        let seen = self.boot(&p.boot, boot)?;
        require(seen.stock_startup_markers > 0, || {
            CommandsError::NoStockBanner
        })?;
        say!(
            "Stock boots from bank 1. {} holds the unit's serial and cloud keys: keep it private.",
            self.rel(&p.boot)
        );
        Ok(0)
    }
}

impl Bench {
    /// A write that exited short of complete: the bank may hold part of it.
    ///
    /// Every non-zero exit counts, 2 as well: `wellfed-flash` exits 2 for
    /// any error its command returns, and nothing pins that only errors
    /// before serial access come back that way. A refusal that touched no
    /// flash is then recorded unknown too, which only asks for a read.
    fn stopped_write(&mut self, bank: u8, source: Source) {
        if let Some(bank) = Bank::of(bank) {
            self.wrote(bank, Content::Unknown, source);
        }
    }

    /// Note what this sequence left in `bank`, for the bank ledger.
    pub fn wrote(&mut self, bank: Bank, content: Content, source: Source) {
        self.written.push(Written {
            bank,
            content,
            source,
        });
    }
}

/// What the frozen review at `review` installs, as the bank ledger names
/// it: a build by its startup identity, the stock image, or else a build
/// by its OTA image's hash. `None` when neither file reads.
fn installed(root: &Path, review: &Path) -> Option<Content> {
    let review = root.join(review);
    let manifest: Option<BuildManifest> =
        wellfed_formats::load(&review.join("build-manifest.json")).ok();
    if let Some(id) = manifest.and_then(|manifest| ota::build_id(&manifest).ok()) {
        return Some(Content::Build { id });
    }
    let frozen: ImageReview = wellfed_formats::load(&review.join("review.json")).ok()?;
    Some(match frozen.image_kind {
        ImageKind::Stock => Content::Stock,
        ImageKind::Build => Content::Image {
            sha256: frozen.ota_sha256.to_ascii_lowercase(),
        },
    })
}

/// The execute command matching a prepare: same evidence flags, plus the hash.
fn execute_line(args: &UpdateArgs, sha: &str) -> String {
    update_line("flash-execute", args) + &format!(" --expected-plan-sha256 {sha}")
}

/// `wellfed-bench COMMAND` with `args`' update flags.
fn update_line(command: &str, args: &UpdateArgs) -> String {
    let mut line = format!("wellfed-bench {command} {} --bank {}", args.name, args.bank);
    if let Some(source) = &args.source {
        line += &format!(" --source {}", source.display());
    } else if let Some(read) = &args.original_read {
        line += &format!(" --original-read {}", read.display());
    }

    line += &format!(" --target {}", args.target.display());
    for (set, flag) in [
        (args.restore_marker, " --restore-marker"),
        (args.interrupted, " --interrupted"),
        (args.attended_motion, " --attended-motion"),
    ] {
        if set {
            line += flag;
        }
    }
    if let Some(bank1) = &args.bank1_source {
        line += &format!(" --bank1-source {}", bank1.display());
    }
    line
}

/// The give-way session a stock return runs first, beside its own evidence.
fn give_way_name(name: &str) -> String {
    format!("{name}-giveway")
}

/// Whether a full read's bank 1 starts with the loader marker. Stock
/// recovery refuses a valid one, and only a full read shows it.
fn bank1_marker_valid(read: &Path) -> Result<bool> {
    let bytes = fs::read(read)?;
    let start =
        usize::try_from(wellfed_signed::BANK1_OFFSET).map_err(|_| CommandsError::Bank1Offset)?;
    let marker = bytes
        .get(start..start + wellfed_signed::MARKER.len())
        .ok_or_else(|| CommandsError::NotFullRead { read: read.into() })?;
    Ok(marker == wellfed_signed::MARKER)
}

/// Expected serial bytes (out, back) of a development update, for the
/// estimated bar. It mirrors the read sequence in wellfed-flash's
/// `development::run`, so a change there must change this too.
///
/// Each acquisition reads the factory region, the other bank's guard sector
/// and the application region. A plan with sectors to stage (or an interrupted
/// one) does three acquisitions, reads every staged sector three times
/// (preimage, erased, programmed) and the marker sector twice; otherwise only
/// the preimage acquisition runs.
fn update_traffic(plan: &DevelopmentPlan) -> (u64, u64) {
    const SECTOR: u64 = 0x1000;
    const ACQUIRED_GUARDS: u64 = 0x6000 + SECTOR;

    let packets = plan.staging_packets + plan.commit_packets;
    let sectors = plan.ordered_sectors.len() as u64;
    let acquisition = ACQUIRED_GUARDS + plan.region.bytes;

    let read = if sectors > 0 || plan.interrupted {
        3 * acquisition + (3 * sectors + 2) * SECTOR
    } else {
        acquisition
    };

    // Read packets carry 1024 payload bytes in 1028 on the wire.
    (packets * FRAME_TX_BYTES, read / 1024 * 1028)
}

/// The operation a prepared plan describes; the updater checks the rest.
#[derive(Deserialize)]
struct Prepared<P> {
    operation_plan: P,
}

const READ_SMALL: &str = "wellfed-flash read small";

/// A bank-0 `flash-execute`'s steps: the write, then the give-way that boots
/// bank 0, then bank 1's marker restore that boots bank 1 (`bank0`).
pub const BANK0_EXECUTE_STEPS: &[&str] = &[
    "wellfed-flash update execute",
    "loader entry",
    READ_SMALL,
    "wellfed-flash give-way inspect",
    "wellfed-flash give-way prepare",
    "wellfed-flash give-way execute",
    "boot capture",
    "loader entry",
    READ_SMALL,
    "wellfed-flash update inspect",
    "wellfed-flash update prepare",
    "wellfed-flash update execute",
    "boot capture",
];

/// Labels for the overall-progress steps of each command.
pub fn steps(command: BenchCommand) -> &'static [&'static str] {
    match command {
        BenchCommand::Boot => &["boot capture"],
        BenchCommand::Loader => &["loader entry"],
        BenchCommand::RomInspect => &["boot capture", "wellfed-flash read inspect-rom"],
        BenchCommand::Dump => &["loader entry", READ_SMALL, "wellfed-flash read full"],
        BenchCommand::FlashPrepare => &[
            "loader entry",
            READ_SMALL,
            "wellfed-flash update inspect",
            "wellfed-flash update prepare",
        ],
        BenchCommand::FlashExecute => &["wellfed-flash update execute", "boot capture"],
        BenchCommand::GiveWayPrepare => &[
            "loader entry",
            READ_SMALL,
            "wellfed-flash give-way inspect",
            "wellfed-flash give-way prepare",
        ],
        BenchCommand::GiveWayExecute => &["wellfed-flash give-way execute", "boot capture"],
        // With --fallback, the stock commands run the give-way steps instead.
        BenchCommand::StockPrepare => &[
            "loader entry",
            READ_SMALL,
            "wellfed-flash read full",
            "wellfed-flash stock-recovery prepare",
        ],
        BenchCommand::StockExecute => &["wellfed-flash stock-recovery execute", "boot capture"],
        BenchCommand::Ota => &[
            ota::SIGN_STEP,
            ota::OFFER_STEP,
            ota::STAGES[0],
            ota::STAGES[1],
            ota::STAGES[2],
            ota::STAGES[3],
            ota::HEALTH_STEP,
        ],
        // Steps of their own, set where they run, or none to show.
        BenchCommand::Status
        | BenchCommand::Off
        | BenchCommand::On
        | BenchCommand::Capture
        | BenchCommand::Press
        | BenchCommand::Transients
        | BenchCommand::Itest
        | BenchCommand::RestoreStock => &[],
    }
}

#[cfg(test)]
mod tests;
