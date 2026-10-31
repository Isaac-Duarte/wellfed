//! `wellfed install`: one run from a feeder on stock 1.1.15 to the
//! replacement confirmed on bank 1 ([consumer installation]).
//!
//! The run is `wellfed_install`'s, the same the Android app runs: stock's
//! server in this process, the request, pairing, the backup and the promotion.
//! This module is its shell for a computer: it reads
//! and checks the review, the keys, the manifest and the networks before
//! the feeder is touched, holds the bench lease beside the stock config for
//! the whole run, and gives the run this computer's platform:
//! - the person at the feeder is a [`Guide`] at a terminal, which also holds
//!   back the setup frame until this computer is on the feeder's network
//!   ([`network`]). With `--prompts lines` it is an [`Asker`] speaking JSON
//!   lines to the bench server that runs it as a job. The setup code and the
//!   enrollment's Dispense press are only announced: the feeder times their
//!   windows and judges the presses. A job needs nothing read at a terminal,
//!   so its home network comes from a file (`--home-network`), and the owner
//!   may answer the feeder check ahead (`--expect-device`);
//! - Bluetooth is BlueZ ([`crate::ble`]), discovery is mDNS, and the client
//!   key is the key file;
//! - progress goes to standard error, so standard output carries prompts
//!   alone. A job prints every event with the stock server's own log, which
//!   are its evidence ([`crate::report`]). A person reads numbered steps and
//!   milestones ([`Screen`]), and the log only with `--verbose` or `RUST_LOG`.
//!
//! SIGTERM (a job's cancel) and SIGINT end the run through the same stop as
//! its end. The report names the step it stopped in and any write whose
//! outcome had not settled; that write is uncertain, and never sent again.
//!
//! [consumer installation]: ../../../../docs/firmware/consumer-installation.md

mod guide;
mod network;
mod screen;

use std::ffi::OsString;
use std::io::{BufRead, IsTerminal};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Args;
use guide::Guide;
use network::{FrameRefused, Host, Link, Nmcli};
use screen::{Detail, Screen, Style};
use tokio::signal::unix::{SignalKind, signal};
use wellfed_client::install::{InstallError, Provision, SetupFrame, StockBackup, manifest_version};
use wellfed_client::setup::Credentials;
use wellfed_formats::bench::lease::Lease;
use wellfed_formats::credentials::{KeyScope, WifiCredential};
use wellfed_formats::manifest::Provisioning;
use wellfed_formats::stock::ServerConfig;
use wellfed_install::StockPlans;
use wellfed_install::{
    Api, Mdns, Plan, Platform, Review, Signer, Start, StockOptions, Stop, build_info,
};
use wellfed_signing::{Frozen, Key, signed_so_far};

use crate::ask::{Asker, Prompts};
use crate::ble::Bluez;
use crate::keys::{OwnerKey, fingerprint};
use crate::report::{Printer, short};

/// The installer's config when `--stock-config` names none.
const STOCK_CONFIG: &str = "private/installer.json";
/// The update key a consumer run at a terminal makes and signs with, when it
/// is given neither a key nor a manifest.
const UPDATE_KEY: &str = "private/update-key.json";
/// Stock reads four port digits; this is the port it knows brokers on.
const MQTT_PORT: u16 = 1883;

#[derive(Args)]
pub(crate) struct InstallArgs {
    /// The replacement's frozen review (its `review.json`, `image.bin` and
    /// `build-manifest.json`): what stock installs into bank 0.
    #[arg(long)]
    review: PathBuf,
    /// A signed update manifest for the same build, which bank 0 installs
    /// into bank 1 at promotion; its version must be above the feeder's
    /// floor. Without it the run signs the build itself with --update-key,
    /// at a version above the floor and above any it signed before, and
    /// keeps the manifest under --backups.
    #[arg(long)]
    manifest: Option<PathBuf>,
    /// The owner's update-signing key file, created here if missing (as
    /// `wellfed update-key new` does). When the image does not build it in,
    /// the run enrolls it after pairing, confirmed by a Dispense press.
    /// Without it, a bench review signs with the bench key
    /// (private/ota-key.json), and a consumer review needs --manifest.
    /// Refused with --manifest, which is promoted as signed. At a terminal, a
    /// consumer review with neither uses private/update-key.json.
    #[arg(long)]
    update_key: Option<PathBuf>,
    /// The network a consumer image is given over Bluetooth setup
    /// (passphrase from standard input); with --ssid, that network.
    #[arg(long, conflicts_with = "ssid")]
    home_ssid: Option<String>,
    /// The network a consumer image is given, from a private JSON file of
    /// its `ssid` and `password` (as `private/hotspot.json` holds them),
    /// so nothing is read at a terminal and the run works with --prompts
    /// lines. The passphrase is never printed.
    #[arg(long, conflicts_with_all = ["home_ssid", "ssid"])]
    home_network: Option<PathBuf>,
    /// Stock's MQTT client ID as the owner already knows it (the feeder's
    /// label, the vendor app): when the broker enrolls exactly this
    /// device, the feeder check passes without asking, and when it enrolls
    /// another, the run stops before anything is sent to stock. The
    /// owner's answer to that check, given ahead from evidence they hold.
    #[arg(long)]
    expect_device: Option<String>,
    /// The feeder's Bluetooth address for setup; otherwise the first in
    /// setup mode.
    #[arg(long)]
    ble_address: Option<bluer::Address>,
    /// A copy of stock 1.1.15's OTA image that matches the pinned digest.
    /// Without one, the run reads bank 1 from the replacement on bank 0
    /// and keeps it under --backups; promotion overwrites the feeder's
    /// only copy of stock, so the run stops before it unless one copy or
    /// the other checks out, with the replacement on bank 0 and stock
    /// intact in bank 1.
    #[arg(long)]
    stock_backup: Option<PathBuf>,
    /// The owner's data directory for stock backups, one per feeder.
    #[arg(long, default_value = crate::backup::BACKUPS)]
    backups: PathBuf,
    /// The owner holds Reset, and the setup frame points stock at this
    /// network (passphrase from standard input) and the installer's broker.
    /// Without it, stock already connects to the installer's broker.
    #[arg(long)]
    ssid: Option<String>,
    /// The numeric account ID the frame carries; stock requires nonzero
    /// digits and keeps it beside the broker.
    #[arg(long, default_value = "1")]
    account: String,
    /// The broker the frame names; the stock config's MQTT address otherwise.
    #[arg(long, requires = "ssid")]
    broker: Option<SocketAddr>,
    /// The replacement is already on bank 0 from an earlier run: start at
    /// pairing, and ask stock nothing.
    #[arg(long, conflicts_with = "ssid")]
    from_bank0: bool,
    /// `wellfed-stock`'s config: an installer's (`init-installer`). One
    /// with a plan snapshot, as the bench's captured config has, is
    /// refused. The run serves stock from this process with it. At a terminal,
    /// with --ssid, it is created if missing, naming this computer's address
    /// on that network (or --broker).
    #[arg(long, default_value = STOCK_CONFIG)]
    stock_config: PathBuf,
    /// How long the broker trusts the first stock feeder that connects.
    #[arg(long, default_value_t = 15)]
    enroll_minutes: u64,
    /// This computer's client key, created on first use.
    #[arg(long, default_value = "private/wellfed-owner.json")]
    keys: PathBuf,
    /// The feeder's identity, as a hex prefix, when the key file knows more
    /// than one.
    #[arg(long)]
    feeder: Option<String>,
    /// The replacement's address; where stock connected from otherwise.
    #[arg(long)]
    address: Option<String>,
    /// How the run asks the person at the feeder: at this terminal, or as
    /// JSON lines on standard output and input for the bench server, which
    /// runs it as a job. The frame route (--ssid) asks at a terminal only,
    /// since it reads the passphrase there.
    #[arg(long, value_enum, default_value_t)]
    prompts: Prompts,
    /// At a terminal, show every event and the stock server's log (`RUST_LOG`
    /// shows that log too). A run with --prompts lines always does.
    #[arg(long, short)]
    verbose: bool,
}

/// A run that cannot ask the way it was told to.
#[derive(Debug, thiserror::Error)]
enum PromptsRefused {
    #[error(
        "--ssid reads the network's passphrase at a terminal, so it does not run with \
         --prompts lines; nothing was started. Leave it out on the bench, whose stock already reaches the broker"
    )]
    Frame,
    #[error(
        "--home-ssid reads the network's passphrase at a terminal, so it does not run with \
         --prompts lines; nothing was started. Pass --home-network <file> instead"
    )]
    HomeNetwork,
}

/// A stock config the install will not run on.
#[derive(Debug, thiserror::Error)]
enum ConfigRefused {
    /// The server would answer stock's plan request with this snapshot,
    /// and stock replaces its stored table with whatever a reply carries
    /// (an empty list erases it), before the run can import the owner's
    /// plans.
    #[error(
        "{path} holds a plan snapshot, which the stock server would send stock in place of its own plans; nothing was started. Use an installer's config (wellfed-stock init-installer), or remove the `plans` key"
    )]
    PlanSnapshot { path: PathBuf },
}

/// What a run refuses before anything starts, beyond a file it cannot read,
/// and the run's own refusals in this command's words.
#[derive(Debug, thiserror::Error)]
enum Refused {
    #[error(
        "a consumer image trusts the release key, which this tool never holds: pass --update-key (created if missing) for the run to sign with and enroll, or --manifest"
    )]
    NoSigner,
    #[error(
        "a consumer image joins no network until Bluetooth setup gives it one: pass --home-network <file> or --home-ssid <network> (or --ssid)"
    )]
    NoHomeNetwork,
    #[error(
        "a bench image joins its build network; --home-ssid and --home-network are for a consumer image"
    )]
    HomeForBench,
    /// The broker enrolled another device than the owner pinned: whatever
    /// reached it first is not the feeder the owner means to install.
    #[error(
        "the broker enrolled device {enrolled}, connecting from {address}, but --expect-device is {expected}; nothing was sent to stock. Check which device reached the broker before running again"
    )]
    OtherDevice {
        expected: String,
        enrolled: String,
        address: String,
    },
    #[error(
        "the build moves the motor at startup ({policy}): sign it attended (wellfed-build sign --attended-motion) and pass --manifest"
    )]
    Motion { policy: String },
    #[error("the review's image is build {signed}, not {served}")]
    OtherBuild { signed: String, served: String },
    #[error("{} is not under a checkout's private/ directory, where reviews are signed", review.display())]
    NotPrivate { review: PathBuf },
    /// `--manifest` names its own signer, which the run cannot see: an
    /// update key beside it would be enrolled for nothing, or the manifest
    /// refused for want of its own.
    #[error(
        "--manifest and --update-key are exclusive: a given manifest is promoted as signed, and the run enrolls no key for it; pass one or the other"
    )]
    KeyWithManifest,
    #[error(
        "feeder {feeder}, which this install paired with, did not answer where it last did, where stock connected from, or over mDNS: pass --address <ip> to resume, or remove installing.json under --backups if its store was reset"
    )]
    NotAnswering { feeder: String },
    #[error("pass --address: no image lease names where the feeder connected from")]
    NoAddress,
    /// Only the frame's network says where the broker must listen.
    #[error(
        "{} does not exist, and without --ssid nothing says where its broker listens: write it with wellfed-stock init-installer --mqtt-listen <this computer's address>:1883 --output {}",
        path.display(),
        path.display()
    )]
    NoConfig { path: PathBuf },
    #[error(
        "this computer has no address on {ssid:?}, so it cannot say where the feeder finds its broker: join that network first, or pass --broker <this computer's address>:1883"
    )]
    NoBrokerAddress { ssid: String },
    /// The frame would point stock at this address after the Reset hold has
    /// erased everything, so a stale one strands it. Checked before that hold.
    #[error(
        "the setup frame would tell the feeder its broker is {broker}, which is not an address of this computer, so {} looks stale. Nothing was asked of the feeder. Remove that file, and this run writes a new one for {ssid:?}",
        path.display()
    )]
    BrokerNotHere {
        path: PathBuf,
        broker: SocketAddr,
        ssid: String,
    },
    /// An address of this computer, but not one a feeder on `ssid` can reach:
    /// the hotspot's, say, on a run that gives it the home network.
    #[error(
        "the setup frame would tell the feeder its broker is {broker}, on this computer's Wi-Fi network {on:?}, but it joins {ssid:?}, so {} looks stale. Nothing was asked of the feeder. Remove that file, and this run writes a new one for {ssid:?}",
        path.display()
    )]
    BrokerOnOtherNetwork {
        path: PathBuf,
        broker: SocketAddr,
        on: String,
        ssid: String,
    },
    /// The server listens where the config says (`Bind::Configured`), so a
    /// frame naming anywhere else points stock at nothing.
    #[error(
        "--broker {broker} is not where {} has the broker listen, {listen}. Nothing was asked of the feeder. Remove that file, and this run writes a new one with --broker",
        path.display()
    )]
    BrokerNotListening {
        path: PathBuf,
        broker: SocketAddr,
        listen: SocketAddr,
    },
    #[error(
        "you did not confirm that the feeder on {ssid:?} can reach this computer at {broker}. Nothing was asked of the feeder"
    )]
    BrokerNotConfirmed { broker: SocketAddr, ssid: String },
}

/// The broker a frame would name must be where this run's server listens, and
/// on the network the frame gives the feeder: after the Reset hold wipes it,
/// the feeder goes there and nowhere else. Where this computer cannot read that
/// network's name, as on a wired interface, `confirm` asks once.
fn require_broker_here(
    args: &InstallArgs,
    host: &impl Host,
    config: &ServerConfig,
    path: &Path,
    confirm: impl FnOnce(&str) -> Result<bool>,
) -> Result<()> {
    let Some(ssid) = args.ssid.clone() else {
        return Ok(());
    };
    let path = path.to_path_buf();
    let listen = config.mqtt_listen;
    let broker = args.broker.unwrap_or(listen);
    if broker != listen {
        return Err(Refused::BrokerNotListening {
            path,
            broker,
            listen,
        }
        .into());
    }
    let Some(interface) = host.interface_of(broker.ip()) else {
        return Err(Refused::BrokerNotHere { path, broker, ssid }.into());
    };
    match host.link(&interface) {
        Link::Wifi(on) if on == ssid => Ok(()),
        Link::Wifi(on) => Err(Refused::BrokerOnOtherNetwork {
            path,
            broker,
            on,
            ssid,
        }
        .into()),
        Link::NotWifi | Link::Unknown => {
            let question = format!(
                "This computer cannot read which network {} on {interface} is part of. Can a \
                 device on {ssid:?} reach this computer at {}? The feeder, once wiped, looks for \
                 it there and nowhere else. [y/N]",
                broker.ip(),
                broker.ip()
            );
            wellfed_formats::require(confirm(&question)?, || Refused::BrokerNotConfirmed {
                broker,
                ssid,
            })?;
            Ok(())
        }
    }
}

/// One yes or no at this terminal, before anything else reads standard input
/// for the run. Only `y` or `yes` is yes; a closed input is no.
fn confirm_at_terminal(question: &str) -> Result<bool> {
    eprintln!("{question}");
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .context("read the answer")?;
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// How a run's progress reaches the person.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Presentation {
    /// The stock server's log goes to standard error.
    stock_log: bool,
    style: Style,
    detail: Detail,
}

impl Presentation {
    /// A job always gets the log, its evidence. Anyone else asks for it, and
    /// then nothing is drawn in place, since the log writes over it.
    fn of(prompts: Prompts, verbose: bool, rust_log: bool, terminal: bool) -> Self {
        let stock_log = prompts == Prompts::Lines || verbose || rust_log;
        Self {
            stock_log,
            style: if terminal && !stock_log {
                Style::Live
            } else {
                Style::Plain
            },
            detail: if verbose {
                Detail::Everything
            } else {
                Detail::Milestones
            },
        }
    }
}

/// Standard error has a person's terminal behind it, able to draw in place.
fn drawable_terminal() -> bool {
    std::io::stderr().is_terminal() && std::env::var_os("TERM").is_some_and(|term| term != "dumb")
}

/// What a run says before it starts about going without `--stock-backup`.
const fn backup_notice(prompts: Prompts) -> &'static str {
    match prompts {
        // A job's evidence keeps its words.
        Prompts::Lines => {
            "No --stock-backup: bank 1 is read from the replacement before promotion, which \
             would overwrite the feeder's only copy of stock."
        }
        Prompts::Terminal => {
            "Your stock is safe: before anything overwrites the feeder's copy, this run reads it \
             and keeps it on this computer, and it stops there if that copy does not check out."
        }
    }
}

/// The installer's config for a run at a terminal, if it has none yet. The
/// frame's network says where the broker must listen. A job's config stays its
/// owner's to write, so a run there goes on to refuse the missing file.
fn ensure_config(args: &InstallArgs, host: &impl Host, path: &Path) -> Result<()> {
    if args.prompts == Prompts::Lines || path.exists() {
        return Ok(());
    }
    let Some(ssid) = &args.ssid else {
        return Err(Refused::NoConfig {
            path: path.to_path_buf(),
        }
        .into());
    };
    let listen = match args.broker {
        Some(broker) => broker,
        None => {
            let address = host
                .address_on(ssid)
                .ok_or_else(|| Refused::NoBrokerAddress { ssid: ssid.clone() })?;
            SocketAddr::from((address, MQTT_PORT))
        }
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let zone = jiff::tz::TimeZone::system();
    wellfed_install::stock::installer_config(path, listen, zone.iana_name())
        .with_context(|| format!("write {}", path.display()))?;
    eprintln!(
        "Wrote the installer's settings to {}: the feeder finds this computer at {listen}.",
        path.display()
    );
    Ok(())
}

/// The command that picks a stopped run up where it stopped: this one's
/// arguments, from bank 0. The frame's flags go, since it asks stock nothing,
/// and the frame's network stays as the one a consumer image is given.
fn resume_command(arguments: &[OsString], provisioning: Provisioning) -> String {
    let mut words = Vec::new();
    let mut arguments = arguments.iter().map(|word| word.to_string_lossy());
    while let Some(word) = arguments.next() {
        let (flag, attached) = match word.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_string())),
            _ => (word.as_ref(), None),
        };
        let mut value = || {
            attached
                .clone()
                .or_else(|| arguments.next().map(|next| next.into_owned()))
        };
        match (flag, provisioning) {
            ("--broker" | "--account", _) | ("--ssid", Provisioning::Bench) => {
                value();
            }
            ("--ssid", Provisioning::Consumer) => {
                words.push("--home-ssid".to_string());
                words.extend(value());
            }
            ("--from-bank0", _) => {}
            _ => words.push(word.to_string()),
        }
    }
    words.push("--from-bank0".to_string());
    words
        .iter()
        .map(|word| network::quoted(word))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A run at a terminal that stopped with the replacement on bank 0: its error,
/// headed by what was kept and the one command that resumes it. A run stopped
/// anywhere else keeps its own words.
fn with_resume(error: wellfed_install::Error, resume: &str) -> anyhow::Error {
    use wellfed_install::Error;
    use wellfed_model::setup::Refusal;

    let lapsed = match &error {
        Error::SetupRefused(refusal) => *refusal == Refusal::Unconfirmed,
        Error::Setup(inner) => matches!(**inner, Error::SetupRefused(Refusal::Unconfirmed)),
        _ => false,
    };
    let resumable = lapsed
        || matches!(
            error,
            Error::Setup(_)
                | Error::SetupRefused(_)
                | Error::RecoveryClosed
                | Error::RecoveryTimeout
                | Error::Backup(_)
                | Error::KeepGiven(_)
                | Error::Steps(
                    InstallError::UpdateKey(_)
                        | InstallError::UpdateKeyUncertain
                        | InstallError::NotAboveFloor { .. }
                        | InstallError::NoVersionAbove { .. }
                )
        );
    let explained = explain(error);
    if !resumable {
        return explained;
    }
    let opening = if lapsed {
        "Time ran out for the code, and nothing was lost."
    } else {
        "The install stopped, and nothing was lost."
    };
    explained.context(format!(
        "{opening} Stock is still safe in bank 1, with Wellfed on bank 0. To pick up where this \
         stopped, once anything below is fixed, run:\n\n    {resume}\n"
    ))
}

pub(crate) async fn run(args: InstallArgs) -> Result<()> {
    if args.prompts == Prompts::Lines && args.ssid.is_some() {
        return Err(PromptsRefused::Frame.into());
    }
    if args.prompts == Prompts::Lines && args.home_ssid.is_some() {
        return Err(PromptsRefused::HomeNetwork.into());
    }
    let config_path = std::path::absolute(&args.stock_config).context("--stock-config")?;
    ensure_config(&args, &Nmcli, &config_path)?;
    let config: ServerConfig = wellfed_formats::load(&config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    if config.has_plan_snapshot() {
        return Err(ConfigRefused::PlanSnapshot { path: config_path }.into());
    }
    require_broker_here(&args, &Nmcli, &config, &config_path, confirm_at_terminal)?;
    let directory = config_path.parent().context("config directory missing")?;
    let _lease = Lease::acquire_in(directory).context("bench lease")?;

    // Everything the run needs is read and checked before the feeder is
    // touched: a bad backup, a missing manifest or key, or a consumer run
    // with no network to give stops it here.
    let review = Review::open(&args.review)?;
    if args.prompts == Prompts::Terminal {
        eprintln!("{INTRODUCTION}");
    }
    let backup = match &args.stock_backup {
        Some(path) => {
            let image = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
            let backup = StockBackup::open(image).context("--stock-backup")?;
            backup
                .verified()
                .context("--stock-backup is not stock's pinned image")?;
            Some(backup)
        }
        None => {
            eprintln!("{}", backup_notice(args.prompts));
            None
        }
    };

    let provisioning = review.provisioning();
    let route = match provisioning {
        Provisioning::Consumer => Route::Setup,
        Provisioning::Bench => Route::Recovery,
    };
    let signer = signer(&args, &review, update_key(&args, provisioning)?)?;
    let passphrase = match (&args.ssid, &args.home_ssid) {
        (None, None) => None,
        _ => Some(crate::passphrase()?),
    };
    let home = home_wifi(&args, route, passphrase.as_deref())?;
    let start = if args.from_bank0 {
        Start::Bank0
    } else {
        Start::Stock(provision(&args, &config, passphrase)?)
    };
    let address = args
        .address
        .as_deref()
        .map(wellfed_connect::parse_address)
        .transpose()
        .context("--address")?;
    // The stock server refuses the trial on such a build too; checked here
    // as well, so a refused run stops before the feeder is touched.
    if !args.from_bank0 {
        build_info::BUILD.require_reviewed()?;
    }

    let bluetooth = Bluez {
        address: args.ble_address,
    };
    let keys = OwnerKey::open(&args.keys, None, OwnerKey::system())?;
    // After the Reset hold no plan is left to come due in the jump.
    let plan = Plan {
        review,
        start,
        signer,
        home,
        backup,
        expect_device: args.expect_device.clone(),
        feeder: args.feeder.clone(),
        address,
        unjoined: wellfed_install::Unjoined::Wait,
        backups: args.backups.clone(),
        stock: StockOptions {
            config: config_path,
            enroll: Duration::from_secs(args.enroll_minutes * 60),
            // The bench's page reads the stock server's status.
            api: Api::Loopback,
            bind: wellfed_install::Bind::Configured,
            stock_plans: stock_plans(&args),
        },
    };

    let presentation = Presentation::of(
        args.prompts,
        args.verbose,
        std::env::var_os("RUST_LOG").is_some(),
        drawable_terminal(),
    );
    // The stock server's log, as `wellfed-stock serve` writes it: what it
    // heard from stock, and why it held stock's clock.
    if presentation.stock_log {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "wellfed_stock=info,mqtt5=warn".into()),
            )
            .with_writer(std::io::stderr)
            .try_init();
    }

    // From here on a signal ends the run through its stop, so the stock
    // server's broker is never left on the MQTT port.
    match args.prompts {
        Prompts::Lines => {
            let platform = Platform {
                prompter: Asker::standard(Prompts::Lines),
                bluetooth,
                discovery: Mdns,
                keys,
                report: Printer,
            };
            wellfed_install::run(plan, platform, signalled()?)
                .await
                .map_err(explain)
        }
        Prompts::Terminal => {
            let screen = Screen::new(
                Box::new(std::io::stderr()),
                presentation.style,
                presentation.detail,
            );
            let platform = Platform {
                prompter: Guide::new(Asker::standard(Prompts::Terminal), screen.clone(), Nmcli),
                bluetooth,
                discovery: Mdns,
                keys,
                report: screen.clone(),
            };
            let arguments: Vec<OsString> = std::env::args_os().collect();
            let ended = wellfed_install::run(plan, platform, signalled()?).await;
            // A countdown may still be drawn, and the error prints next.
            screen.pause();
            ended.map_err(|error| with_resume(error, &resume_command(&arguments, provisioning)))
        }
    }
}

/// What a person reads as the run starts.
const INTRODUCTION: &str = "\nInstalling Wellfed, in five steps. Stay near the feeder: steps 1, 3 \
                            and 4 need you there, and this run says when. If it stops, it says \
                            what was kept and how to go on. Ctrl-C stops it safely.";

/// Resolves on SIGTERM, SIGINT or SIGHUP, so a terminal that closes still ends
/// the run through its stop, which undoes a hop to the feeder's network. Once
/// seen, the handlers stay installed but nothing listens, so a second Ctrl-C
/// does not cut the stop short; the stop is itself bounded. A SIGHUP this
/// process was started ignoring (`nohup`) stays ignored: whoever started it
/// asked for the run to outlive the terminal.
pub(crate) fn signalled() -> Result<impl Future<Output = Stop>> {
    let mut terminate = signal(SignalKind::terminate()).context("handle SIGTERM")?;
    let mut interrupt = signal(SignalKind::interrupt()).context("handle SIGINT")?;
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let mut hangup = if hangup_ignored(&status) {
        None
    } else {
        Some(signal(SignalKind::hangup()).context("handle SIGHUP")?)
    };
    Ok(async move {
        let hung_up = async {
            match &mut hangup {
                Some(hangup) => hangup.recv().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = terminate.recv() => Stop::Terminate,
            _ = interrupt.recv() => Stop::Interrupt,
            _ = hung_up => Stop::Hangup,
        }
    })
}

/// Whether `/proc/self/status` lists SIGHUP, signal 1 and so the mask's lowest
/// bit, among the ignored signals.
fn hangup_ignored(status: &str) -> bool {
    status
        .lines()
        .find_map(|line| line.strip_prefix("SigIgn:"))
        .and_then(|mask| u64::from_str_radix(mask.trim(), 16).ok())
        .is_some_and(|mask| mask & 1 == 1)
}

/// How the replacement is paired, by the review's provisioning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    /// A consumer image starts Bluetooth setup by itself.
    Setup,
    /// A bench image joins its build network and pairs through the
    /// recovery window.
    Recovery,
}

/// The run's error in this command's words: the flags that resume it, or
/// that answer a refusal, where the run names none.
/// Only the setup frame follows the Reset hold, which erases stock's plans.
/// Without it stock reached the broker some other way and may still hold them,
/// so its clock stays where it reads.
fn stock_plans(args: &InstallArgs) -> StockPlans {
    if args.ssid.is_some() {
        StockPlans::Erased
    } else {
        StockPlans::Unknown
    }
}

fn explain(error: wellfed_install::Error) -> anyhow::Error {
    use wellfed_install::Error;

    match error {
        Error::Steps(
            InstallError::UpdateKey(_)
            | InstallError::UpdateKeyUncertain
            | InstallError::NotAboveFloor { .. }
            | InstallError::NoVersionAbove { .. },
        ) => anyhow::Error::from(error).context(
            "Promotion refused: the feeder runs the replacement from bank 0 and still holds \
             stock in bank 1. Run again with --from-bank0 once the cause is fixed",
        ),
        Error::Backup(_) => anyhow::Error::from(error).context(
            "Promotion refused: no verified stock backup. The feeder runs the replacement from \
             bank 0 and still holds stock in bank 1; run again with --from-bank0 once a backup \
             can be read, or with --stock-backup <stock 1.1.15 OTA image>",
        ),
        Error::KeepGiven(_) => anyhow::Error::from(error).context(
            "Promotion refused: the --stock-backup copy could not be kept; the feeder runs the \
             replacement from bank 0 and still holds stock in bank 1",
        ),
        // Only a refusal from the check before the frame says nothing went out:
        // a handover can fail on the way back too, with the frame already sent.
        Error::Handover(ref source) if source.is::<FrameRefused>() => anyhow::Error::from(error)
            .context(
                "The setup frame was refused before it went: your Wi-Fi password was not sent. \
                 The feeder stays in setup mode, so run this again from your own network, \
                 with no second Reset hold",
            ),
        Error::Setup(_) => anyhow::Error::from(error).context(
            "Bluetooth setup failed: the feeder runs the replacement from bank 0 and still holds \
             stock in bank 1. Run again with --from-bank0; with no Wi-Fi stored, it stays in \
             Bluetooth setup for another try, and goes back to setup by itself if it restarts",
        ),
        Error::OtherDevice {
            expected,
            enrolled,
            address,
        } => Refused::OtherDevice {
            expected,
            enrolled,
            address,
        }
        .into(),
        Error::NotAnswering { feeder } => Refused::NotAnswering {
            feeder: fingerprint(&feeder),
        }
        .into(),
        Error::NoHomeNetwork => Refused::NoHomeNetwork.into(),
        Error::NoAddress => Refused::NoAddress.into(),
        error => error.into(),
    }
}

/// The update key file a consumer run with neither `--update-key` nor
/// `--manifest` uses: at a terminal, one kept for it. A job names its own.
fn default_update_key(prompts: Prompts) -> Option<&'static Path> {
    (prompts == Prompts::Terminal).then(|| Path::new(UPDATE_KEY))
}

/// The update key the run signs with: `--update-key` (created if
/// missing), or the bench key, never created here, for a bench review.
/// `None` with `--manifest`, which is promoted as signed; the two together
/// are refused.
fn update_key(args: &InstallArgs, provisioning: Provisioning) -> Result<Option<Key>> {
    let default = match (&args.update_key, &args.manifest, provisioning) {
        (None, None, Provisioning::Consumer) => default_update_key(args.prompts),
        _ => None,
    };
    if let Some(path) = default {
        return open_update_key(path).map(Some);
    }
    match (&args.update_key, &args.manifest, provisioning) {
        (Some(_), Some(_), _) => Err(Refused::KeyWithManifest.into()),
        (Some(path), None, _) => open_update_key(path).map(Some),
        (None, Some(_), _) => Ok(None),
        (None, None, Provisioning::Bench) => {
            let path = Path::new(wellfed_signing::KEY_PATH);
            let key = Key::load(path).with_context(|| format!("load {}", path.display()))?;
            Ok(Some(key))
        }
        (None, None, Provisioning::Consumer) => Err(Refused::NoSigner.into()),
    }
}

/// What the promotion is signed with: `--manifest` as given, or the
/// review signed here with `key` above every version signed so far.
fn signer(args: &InstallArgs, review: &Review, key: Option<Key>) -> Result<Signer> {
    match (&args.manifest, key) {
        (Some(path), _) => {
            let manifest = std::fs::read(path).context("read --manifest")?;
            let version =
                manifest_version(&manifest).context("--manifest is not a signed manifest")?;
            let (frozen, _) = open_review(&args.review)?;
            frozen
                .require_same_image(&manifest)
                .context("--manifest does not match --review")?;
            Ok(Signer::Manifest { manifest, version })
        }
        (None, Some(key)) => {
            let (frozen, root) = frozen_review(&args.review, review.build())?;
            let backups = std::path::absolute(&args.backups).context("--backups")?;
            let above = signed_so_far(&root, &backups).map_or(0, |(version, _)| version);
            if args.prompts == Prompts::Lines || args.verbose {
                eprintln!(
                    "The run signs build {} with update key {} once it has read the feeder's \
                     floor, above version {above}.",
                    short(review.build()),
                    wellfed_formats::hex::encode(&key.id())
                );
            }
            Ok(Signer::Key {
                key: Box::new(key),
                frozen: Box::new(frozen),
                above,
            })
        }
        (None, None) => Err(Refused::NoSigner.into()),
    }
}

/// Load the owner's update key at `path`, creating one there first if
/// there is none, as `wellfed update-key new` would.
fn open_update_key(path: &Path) -> Result<Key> {
    if !path.exists() {
        let public = wellfed_signing::keygen(path, KeyScope::Owner)
            .with_context(|| format!("write {}", path.display()))?;
        eprintln!(
            "Created update key {} (public key {public}); it holds the private key: keep it \
             private.",
            path.display()
        );
    }
    Key::load(path).with_context(|| format!("load {}", path.display()))
}

/// The review, verified as the signer verifies it; refused when it is not
/// the served build, or when it moves the motor at startup (signed only
/// attended). With it, the checkout root the review sits under.
fn frozen_review(review: &Path, build: &str) -> Result<(Frozen, PathBuf)> {
    let (frozen, root) = open_review(review)?;
    if frozen.build_id() != build {
        return Err(Refused::OtherBuild {
            signed: frozen.build_id(),
            served: build.to_string(),
        }
        .into());
    }
    if let Some(policy) = frozen.motion_policy() {
        return Err(Refused::Motion {
            policy: policy.to_string(),
        }
        .into());
    }
    Ok((frozen, root))
}

/// The review, verified as the signer verifies it, and the checkout root it
/// sits under.
fn open_review(review: &Path) -> Result<(Frozen, PathBuf)> {
    let absolute = std::path::absolute(review).context("--review")?;
    let root = absolute
        .ancestors()
        .find(|dir| dir.file_name().is_some_and(|name| name == "private"))
        .and_then(Path::parent)
        .ok_or_else(|| Refused::NotPrivate {
            review: review.to_path_buf(),
        })?;
    let frozen = Frozen::open(root, &absolute).context("verify the review to sign it")?;
    Ok((frozen, root.to_path_buf()))
}

/// The Wi-Fi Bluetooth setup gives a consumer image: `--home-network`'s
/// file, or `--home-ssid` (or the frame's `--ssid`) with the passphrase
/// read once.
fn home_wifi(
    args: &InstallArgs,
    route: Route,
    passphrase: Option<&str>,
) -> Result<Option<Credentials>> {
    if route == Route::Recovery {
        return match (&args.home_ssid, &args.home_network) {
            (None, None) => Ok(None),
            _ => Err(Refused::HomeForBench.into()),
        };
    }
    let network = match (&args.home_network, &args.home_ssid, &args.ssid, passphrase) {
        (Some(path), _, _, _) => Some(home_network(path)?),
        (None, Some(ssid), _, Some(passphrase)) | (None, None, Some(ssid), Some(passphrase)) => {
            Some((ssid.clone(), passphrase.to_string()))
        }
        _ => None,
    };
    match network {
        Some((ssid, password)) => Ok(Some(
            Credentials::new(ssid.into_bytes(), password).context("the home network")?,
        )),
        // A resumed run may find the feeder on the network already.
        None if args.from_bank0 => Ok(None),
        None => Err(Refused::NoHomeNetwork.into()),
    }
}

/// The SSID and passphrase in `path`, a file shaped as the bench's
/// `private/hotspot.json` is. Neither the error nor anything else here
/// prints the passphrase.
fn home_network(path: &Path) -> Result<(String, String)> {
    let WifiCredential { ssid, password } = wellfed_formats::load(path)
        .with_context(|| format!("read the home network from {}", path.display()))?;
    Ok((ssid, password))
}

/// How stock reaches the installer's broker: the setup frame after the Reset
/// hold (`--ssid`: the owner's network and the installer's broker), or already.
fn provision(
    args: &InstallArgs,
    config: &ServerConfig,
    passphrase: Option<String>,
) -> Result<Provision> {
    let Some(ssid) = &args.ssid else {
        return Ok(Provision::Done);
    };
    let broker = args.broker.unwrap_or(config.mqtt_listen);
    let Some(password) = passphrase else {
        bail!("the frame's passphrase was not read");
    };
    Ok(Provision::Frame(SetupFrame {
        ssid: ssid.clone(),
        password,
        host: broker.ip().to_string(),
        port: broker.port(),
        id: args.account.clone(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_the_setup_frame_says_stock_s_plans_are_erased() {
        let frame = args(&["--ssid", "Maple St"]);
        assert_eq!(stock_plans(&frame), StockPlans::Erased);
        assert_eq!(stock_plans(&args(&[])), StockPlans::Unknown);
    }

    #[derive(clap::Parser)]
    struct Line {
        #[command(flatten)]
        args: InstallArgs,
    }

    /// A config with a plan snapshot, even an empty one, is refused before
    /// the lease, the server or the feeder: the server would send stock
    /// that snapshot in place of its own plans.
    #[tokio::test]
    async fn a_config_with_a_plan_snapshot_is_refused_before_anything_starts() {
        use clap::Parser;

        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("server.json");
        std::fs::write(
            &config,
            json!({"api_token": "t", "http_listen": "127.0.0.1:8765",
                   "mqtt_listen": "10.42.103.1:1883", "plans": []})
            .to_string(),
        )
        .unwrap();
        let line = Line::parse_from([
            "install",
            "--review",
            "missing-review",
            "--manifest",
            "missing.bin",
            "--stock-config",
            config.to_str().unwrap(),
        ]);

        let error = run(line.args).await.unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ConfigRefused>(),
            Some(ConfigRefused::PlanSnapshot { .. })
        ));
        // Nothing beside the config: no lease (nor its lock), no enrollment.
        let beside: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(beside, ["server.json"]);
    }

    /// The frame route reads its passphrase at a terminal, so a run asked
    /// to prompt in lines is refused before it reads anything.
    #[tokio::test]
    async fn the_frame_route_is_refused_with_prompts_in_lines() {
        use clap::Parser;

        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("server.json");
        let line = Line::parse_from([
            "install",
            "--review",
            "missing-review",
            "--manifest",
            "missing.bin",
            "--stock-config",
            config.to_str().unwrap(),
            "--ssid",
            "Home",
            "--prompts",
            "lines",
        ]);

        let error = run(line.args).await.unwrap_err();
        assert!(matches!(
            error.downcast_ref::<PromptsRefused>(),
            Some(PromptsRefused::Frame)
        ));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);

        // A consumer image's home network reads its passphrase the same way.
        let home = Line::parse_from([
            "install",
            "--review",
            "missing-review",
            "--stock-config",
            config.to_str().unwrap(),
            "--home-ssid",
            "Home",
            "--prompts",
            "lines",
        ]);
        let error = run(home.args).await.unwrap_err();
        assert!(matches!(
            error.downcast_ref::<PromptsRefused>(),
            Some(PromptsRefused::HomeNetwork)
        ));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    fn args(line: &[&str]) -> InstallArgs {
        use clap::Parser;

        let base = [
            "install",
            "--review",
            "private/review",
            "--stock-config",
            "private/installer.json",
        ];
        Line::parse_from([&base[..], line].concat()).args
    }

    /// A consumer image trusts only the release key and has no network:
    /// the run needs a key of its own (or a manifest) and the network to
    /// give it, before anything starts.
    #[test]
    fn a_consumer_run_without_a_signer_or_a_network_is_refused() {
        fn refused<T>(result: Result<T>) -> Option<Refused> {
            result
                .err()
                .and_then(|error| error.downcast::<Refused>().ok())
        }

        let bare = args(&[]);
        // A job names what it signs with, and is refused otherwise. A person
        // at a terminal gets the kept key (not made here: this reads the path).
        let job = args(&["--prompts", "lines"]);
        assert!(matches!(
            refused(update_key(&job, Provisioning::Consumer)),
            Some(Refused::NoSigner)
        ));
        assert_eq!(default_update_key(job.prompts), None);
        assert_eq!(
            default_update_key(bare.prompts),
            Some(Path::new("private/update-key.json"))
        );
        let given = args(&["--manifest", "private/v9.manifest"]);
        assert!(
            update_key(&given, Provisioning::Consumer)
                .unwrap()
                .is_none()
        );

        assert!(matches!(
            refused(home_wifi(&bare, Route::Setup, None)),
            Some(Refused::NoHomeNetwork)
        ));
        let resumed = args(&["--from-bank0"]);
        assert_eq!(home_wifi(&resumed, Route::Setup, None).unwrap(), None);
        let home = args(&["--home-ssid", "Home"]);
        assert!(
            home_wifi(&home, Route::Setup, Some("pass word!"))
                .unwrap()
                .is_some()
        );
        // The frame's network is the home network too.
        let frame = args(&["--ssid", "Home"]);
        assert!(
            home_wifi(&frame, Route::Setup, Some("pass word!"))
                .unwrap()
                .is_some()
        );
        assert!(matches!(
            refused(home_wifi(&home, Route::Recovery, Some("pass word!"))),
            Some(Refused::HomeForBench)
        ));
        assert_eq!(home_wifi(&frame, Route::Recovery, Some("x")).unwrap(), None);
    }

    /// `--home-network` gives a consumer image the network from a file, so
    /// it reads nothing at a terminal: it runs with lines prompts, and it
    /// takes the place of `--home-ssid` and `--ssid`, never beside them.
    #[tokio::test]
    async fn a_home_network_file_runs_with_lines_prompts_and_alone() {
        use clap::Parser;

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hotspot.json");
        std::fs::write(
            &file,
            json!({"connection": "bench", "ssid": "Bench", "password": "pass word!"}).to_string(),
        )
        .unwrap();
        let path = file.to_str().unwrap();

        let given = args(&["--home-network", path]);
        let wifi = home_wifi(&given, Route::Setup, None).unwrap();
        assert!(wifi.is_some());
        assert!(matches!(
            home_wifi(&given, Route::Recovery, None)
                .unwrap_err()
                .downcast::<Refused>(),
            Ok(Refused::HomeForBench)
        ));
        let missing = args(&["--home-network", "private/none.json"]);
        let error = home_wifi(&missing, Route::Setup, None).unwrap_err();
        assert!(
            format!("{error:#}").contains("private/none.json"),
            "{error:#}"
        );

        // Refused before it reads anything only for a missing review: the
        // prompts mode lets it through.
        let config = dir.path().join("server.json");
        let line = Line::parse_from([
            "install",
            "--review",
            "missing-review",
            "--stock-config",
            config.to_str().unwrap(),
            "--home-network",
            path,
            "--prompts",
            "lines",
        ]);
        let error = run(line.args).await.unwrap_err();
        assert!(
            error.downcast_ref::<PromptsRefused>().is_none(),
            "{error:#}"
        );
        assert!(!format!("{error:#}").contains("pass word!"), "{error:#}");

        for other in [["--home-ssid", "Home"], ["--ssid", "Home"]] {
            let line = Line::try_parse_from(
                [
                    &["install", "--review", "r", "--stock-config", "c"][..],
                    &["--home-network", path],
                    &other,
                ]
                .concat(),
            );
            assert!(line.is_err(), "{other:?}");
        }
    }

    /// The log is quiet unless asked for, and a job always gets it: its output
    /// is its evidence. Anything sharing the terminal with it stops drawing in
    /// place, as does anything that is not a terminal.
    #[test]
    fn the_stock_log_is_a_job_s_or_asked_for() {
        let of = Presentation::of;

        let quiet = of(Prompts::Terminal, false, false, true);
        assert!(!quiet.stock_log);
        assert_eq!(quiet.style, Style::Live);
        assert_eq!(quiet.detail, Detail::Milestones);

        let piped = of(Prompts::Terminal, false, false, false);
        assert!(!piped.stock_log);
        assert_eq!(piped.style, Style::Plain);

        let verbose = of(Prompts::Terminal, true, false, true);
        assert!(verbose.stock_log);
        assert_eq!(verbose.style, Style::Plain);
        assert_eq!(verbose.detail, Detail::Everything);

        let rust_log = of(Prompts::Terminal, false, true, true);
        assert!(rust_log.stock_log);
        assert_eq!(rust_log.style, Style::Plain);
        assert_eq!(rust_log.detail, Detail::Milestones);

        for terminal in [false, true] {
            assert!(of(Prompts::Lines, false, false, terminal).stock_log);
        }
    }

    /// A bench job's standard error is its evidence, so these words and the
    /// JSON lines beside them stay as they are. A person's words may change.
    #[test]
    fn a_job_s_output_is_unchanged() {
        use crate::report::line;
        use wellfed_install::event::Wait;

        assert_eq!(
            backup_notice(Prompts::Lines),
            "No --stock-backup: bank 1 is read from the replacement before promotion, which \
             would overwrite the feeder's only copy of stock."
        );
        for (event, words) in [
            (
                wellfed_install::Event::StockServerStarted,
                "Started the stock server (its broker trusts the first feeder that connects).",
            ),
            (
                wellfed_install::Event::Waiting(Wait::NoFeeder),
                "Waiting: no stock feeder has connected to the broker yet",
            ),
            (
                wellfed_install::Event::StockProgress(42),
                "Stock: 42% downloaded",
            ),
            (
                wellfed_install::Event::Written {
                    written: 512,
                    length: 1024,
                },
                "  512 of 1024 bytes written",
            ),
            (
                wellfed_install::Event::Promoting,
                "Promoting: bank 0 writes the replacement into bank 1.",
            ),
            (
                wellfed_install::Event::Floor(7),
                "The feeder's floor is version 7.",
            ),
        ] {
            assert_eq!(line(&event), words);
        }
    }

    /// Stock's silence after the frame is normal, and the words say so: they
    /// never suggest sending it again.
    #[test]
    fn an_unanswered_frame_reads_as_normal() {
        let words = crate::report::line(&wellfed_install::Event::FrameSent { answer: None });
        assert!(words.contains("which is normal"), "{words}");
        assert!(words.contains("never sent twice"), "{words}");
        assert!(words.contains("within about 2 minutes"), "{words}");
    }

    #[test]
    fn a_person_is_told_stock_is_kept_before_anything_overwrites_it() {
        let words = backup_notice(Prompts::Terminal);
        assert!(words.contains("before anything overwrites"), "{words}");
        assert!(words.contains("keeps it on this computer"), "{words}");
    }

    /// The config is made only for a person, only where the frame's network
    /// says where its broker listens, and is never replaced.
    #[test]
    fn a_missing_config_is_made_from_the_frame_s_network() {
        use network::tests::FakeHost;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deep/installer.json");
        let on_network = FakeHost {
            address: Some("10.42.103.1".parse().unwrap()),
            ..FakeHost::default()
        };

        let no_frame = args(&[]);
        let error = ensure_config(&no_frame, &on_network, &path).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<Refused>(),
            Some(Refused::NoConfig { .. })
        ));
        assert!(!path.exists());

        let frame = args(&["--ssid", "Lab"]);
        let error = ensure_config(&frame, &FakeHost::default(), &path).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<Refused>(),
            Some(Refused::NoBrokerAddress { ssid }) if ssid == "Lab"
        ));
        assert!(!path.exists());

        // A job's config stays its owner's: nothing is made for it.
        let job = args(&["--prompts", "lines"]);
        ensure_config(&job, &on_network, &path).unwrap();
        assert!(!path.exists());

        ensure_config(&frame, &on_network, &path).unwrap();
        let config: ServerConfig = wellfed_formats::load(&path).unwrap();
        assert_eq!(config.mqtt_listen, "10.42.103.1:1883".parse().unwrap());
        assert!(!config.has_plan_snapshot());

        // Kept, whatever the network says now.
        let before = std::fs::read(&path).unwrap();
        let elsewhere = FakeHost {
            address: Some("10.42.103.9".parse().unwrap()),
            ..FakeHost::default()
        };
        ensure_config(&frame, &elsewhere, &path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);

        // --broker names it outright.
        let named = dir.path().join("named.json");
        let broker = args(&["--ssid", "Lab", "--broker", "10.0.0.5:1883"]);
        ensure_config(&broker, &FakeHost::default(), &named).unwrap();
        let config: ServerConfig = wellfed_formats::load(&named).unwrap();
        assert_eq!(config.mqtt_listen, "10.0.0.5:1883".parse().unwrap());
    }

    fn command(line: &[&str]) -> Vec<OsString> {
        line.iter().map(OsString::from).collect()
    }

    /// The owner's real command becomes the one that resumes it: nothing asks
    /// stock again, and a consumer image still gets its network.
    #[test]
    fn the_resume_command_keeps_this_run_s_arguments() {
        let owner = command(&[
            "target/release/wellfed",
            "install",
            "--stock-config",
            "private/installer.json",
            "--review",
            "private/verification/review",
            "--manifest",
            "private/v384.manifest",
            "--ssid",
            "Maple St",
            "--account",
            "7",
            "--broker=10.0.0.5:1883",
        ]);
        assert_eq!(
            resume_command(&owner, Provisioning::Consumer),
            "target/release/wellfed install --stock-config private/installer.json --review \
             private/verification/review --manifest private/v384.manifest --home-ssid 'Maple St' \
             --from-bank0"
        );
        // A bench image joins its build network and takes no home network.
        assert_eq!(
            resume_command(&owner, Provisioning::Bench),
            "target/release/wellfed install --stock-config private/installer.json --review \
             private/verification/review --manifest private/v384.manifest --from-bank0"
        );
        // A resumed run resumes the same way, once.
        let resumed = command(&["wellfed", "install", "--from-bank0", "--ssid=Lab"]);
        assert_eq!(
            resume_command(&resumed, Provisioning::Consumer),
            "wellfed install --home-ssid Lab --from-bank0"
        );
    }

    /// The command that comes out parses as what it says.
    #[test]
    fn the_resume_command_is_a_valid_run() {
        use clap::Parser;

        let line = resume_command(
            &command(&["install", "--review", "r", "--ssid", "Lab"]),
            Provisioning::Consumer,
        );
        let parsed = Line::try_parse_from(line.split(' ')).unwrap();
        assert!(parsed.args.from_bank0);
        assert_eq!(parsed.args.home_ssid.as_deref(), Some("Lab"));
        assert_eq!(parsed.args.ssid, None);
    }

    /// A run that ran out of time on the code ends with what is safe, and the
    /// one command; so do the promotion's other stops. Others keep their words.
    #[test]
    fn a_stopped_run_ends_with_the_command_that_resumes_it() {
        use wellfed_install::Error;
        use wellfed_model::setup::Refusal;

        let lapsed = Error::Setup(Box::new(Error::SetupRefused(Refusal::Unconfirmed)));
        let text = format!("{:#}", with_resume(lapsed, "wellfed install --from-bank0"));
        assert!(text.starts_with("Time ran out for the code"), "{text}");
        assert!(text.contains("nothing was lost"), "{text}");
        assert!(text.contains("safe in bank 1"), "{text}");
        assert!(text.contains("    wellfed install --from-bank0"), "{text}");

        let wrong = Error::Setup(Box::new(Error::SetupRefused(Refusal::WrongCode)));
        let text = format!("{:#}", with_resume(wrong, "resume"));
        assert!(text.starts_with("The install stopped"), "{text}");
        assert!(text.contains("resume"), "{text}");

        // Bank 0 is not known to hold anything here: no such words.
        let other = Error::OtherDevice {
            expected: "AF_A".into(),
            enrolled: "AF_B".into(),
            address: "10.0.0.2".into(),
        };
        let text = format!("{:#}", with_resume(other, "resume"));
        assert!(!text.contains("resume"), "{text}");
        assert!(!text.contains("safe in bank 1"), "{text}");
    }

    /// A broker that is not on this computer is refused before the Reset hold:
    /// the frame would strand a wiped feeder there. Without a frame, nothing is
    /// checked, since nothing is named.
    #[test]
    fn a_stale_broker_is_refused_before_the_reset_hold() {
        use network::tests::FakeHost;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("installer.json");
        let on_network = |ssid: &str, address: &str| FakeHost {
            address: Some(address.parse().unwrap()),
            held: vec![address.parse().unwrap()],
            link: Some(Link::Wifi(ssid.into())),
            ..FakeHost::default()
        };
        let never = |question: &str| -> Result<bool> { panic!("asked: {question}") };
        let refused = |error: anyhow::Error| error.downcast::<Refused>().unwrap();

        let here = on_network("Lab", "10.42.103.1");
        let frame = args(&["--ssid", "Lab"]);
        ensure_config(&frame, &here, &path).unwrap();
        let config: ServerConfig = wellfed_formats::load(&path).unwrap();
        require_broker_here(&frame, &here, &config, &path, never).unwrap();

        // The same config on a computer that has moved on.
        let moved = on_network("Lab", "10.42.103.9");
        let error = require_broker_here(&frame, &moved, &config, &path, never).unwrap_err();
        assert!(matches!(
            refused(error),
            Refused::BrokerNotHere { broker, .. } if broker.ip().to_string() == "10.42.103.1"
        ));

        // The hotspot's address is this computer's, but a feeder given the
        // home network cannot reach it.
        let home = args(&["--ssid", "Home"]);
        let error = require_broker_here(&home, &here, &config, &path, never).unwrap_err();
        let error = refused(error);
        assert!(error.to_string().contains("Remove that file"), "{error}");
        assert!(matches!(
            error,
            Refused::BrokerOnOtherNetwork { on, ssid, .. } if on == "Lab" && ssid == "Home"
        ));

        // --broker names what the frame says, but the server listens where the
        // config has it, so it must be the same address.
        let named = args(&["--ssid", "Lab", "--broker", "10.42.103.9:1883"]);
        let error = require_broker_here(&named, &moved, &config, &path, never).unwrap_err();
        assert!(matches!(refused(error), Refused::BrokerNotListening { .. }));

        // No frame names nothing.
        require_broker_here(&args(&[]), &moved, &config, &path, never).unwrap();

        // A network it cannot read, or a wired interface: asked once, naming
        // the address and the network, and only a yes passes.
        for link in [Link::Unknown, Link::NotWifi] {
            let unread = FakeHost {
                link: Some(link),
                ..on_network("", "10.42.103.1")
            };
            let mut asked = String::new();
            require_broker_here(&frame, &unread, &config, &path, |question| {
                asked = question.to_string();
                Ok(true)
            })
            .unwrap();
            assert!(asked.contains("10.42.103.1"), "{asked}");
            assert!(asked.contains("\"Lab\""), "{asked}");

            let error =
                require_broker_here(&frame, &unread, &config, &path, |_| Ok(false)).unwrap_err();
            assert!(matches!(refused(error), Refused::BrokerNotConfirmed { .. }));
        }

        // The refusal's advice works: with the file gone, the run writes one
        // for the network it was given, which then passes.
        std::fs::remove_file(&path).unwrap();
        let home_here = on_network("Home", "192.168.4.20");
        ensure_config(&home, &home_here, &path).unwrap();
        let config: ServerConfig = wellfed_formats::load(&path).unwrap();
        require_broker_here(&home, &home_here, &config, &path, never).unwrap();

        // So does the same for a stale --broker.
        std::fs::remove_file(&path).unwrap();
        let broker = args(&["--ssid", "Home", "--broker", "192.168.4.20:1883"]);
        ensure_config(&broker, &FakeHost::default(), &path).unwrap();
        let config: ServerConfig = wellfed_formats::load(&path).unwrap();
        require_broker_here(&broker, &home_here, &config, &path, never).unwrap();
    }

    #[test]
    fn an_ignored_hangup_is_read_from_the_process_status() {
        let status = |mask: &str| format!("Name:\twellfed\nSigIgn:\t{mask}\nSigCgt:\t0\n");
        assert!(hangup_ignored(&status("0000000000000001")));
        assert!(hangup_ignored(&status("0000000000001003")));
        assert!(!hangup_ignored(&status("0000000000001000")));
        assert!(!hangup_ignored(""));
    }

    /// A terminal that closes ends the run through its stop, so a hop is undone.
    #[tokio::test]
    async fn a_closed_terminal_stops_the_run() {
        // Under nohup the signal stays ignored, by design, and nothing would end.
        if hangup_ignored(&std::fs::read_to_string("/proc/self/status").unwrap_or_default()) {
            return;
        }
        let stop = signalled().unwrap();
        let status = std::process::Command::new("kill")
            .args(["-HUP", &std::process::id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(stop.await, Stop::Hangup);
    }

    /// The frame's refusal reaches the owner saying the password did not go.
    #[test]
    fn a_refused_frame_says_the_password_was_not_sent() {
        use wellfed_install::Error;

        let refused = FrameRefused::OtherNetwork {
            interface: "wlan0".into(),
            ssid: "Maple St".into(),
        };
        let error = explain(Error::Handover(Box::new(refused)));
        let text = format!("{error:#}");
        assert!(text.contains("password was not sent"), "{text}");
        assert!(text.contains("Maple St"), "{text}");

        // Coming back from the feeder's network happens after the frame, so
        // nothing there claims it did not go.
        let error = explain(Error::Handover("no way back".into()));
        assert!(!format!("{error:#}").contains("not sent"));
    }

    /// `--update-key` names a file the run creates once and reuses.
    #[test]
    fn the_update_key_is_created_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update-key.json");
        let created = open_update_key(&path).unwrap();
        assert_eq!(created.scope(), KeyScope::Owner);
        assert_eq!(open_update_key(&path).unwrap().public(), created.public());
    }

    /// A given manifest is promoted as signed: an update key beside it is
    /// refused rather than enrolled for nothing.
    #[test]
    fn an_update_key_beside_a_manifest_is_refused() {
        let both = args(&[
            "--manifest",
            "private/v9.manifest",
            "--update-key",
            "private/update-key.json",
        ]);
        let Err(refused) = update_key(&both, Provisioning::Bench) else {
            panic!("a key beside a manifest is refused");
        };
        assert!(matches!(
            refused.downcast_ref::<Refused>(),
            Some(Refused::KeyWithManifest)
        ));
    }

    /// The run's refusals that name a flag are worded with it, and a
    /// stopped run's report is the run's own.
    #[test]
    fn the_run_s_refusals_name_this_command_s_flags() {
        use wellfed_install::Error;

        let error = explain(Error::OtherDevice {
            expected: "AF_OTHER".into(),
            enrolled: "AF_FEEDER".into(),
            address: "10.42.103.172".into(),
        });
        match error.downcast_ref::<Refused>() {
            Some(Refused::OtherDevice {
                expected,
                enrolled,
                address,
            }) => assert_eq!(
                (expected.as_str(), enrolled.as_str(), address.as_str()),
                ("AF_OTHER", "AF_FEEDER", "10.42.103.172")
            ),
            other => panic!("{other:?}"),
        }
        assert!(error.to_string().contains("--expect-device is AF_OTHER"));

        let error = explain(Error::NotAnswering { feeder: [0xab; 32] });
        assert!(error.to_string().contains("feeder abababababababab"));
        assert!(error.to_string().contains("pass --address <ip> to resume"));
    }
}
