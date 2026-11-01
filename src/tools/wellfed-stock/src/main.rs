//! `wellfed-stock` command line: the bench's tools for a stock 1.1.15 feeder.
//!
//! `serve` runs the server ([`wellfed_stock::server`]): the MQTT broker stock
//! connects to and the loopback HTTP API, in this one process, and never asks
//! stock to install: `wellfed install` and the app run that in theirs. The
//! `status` and `ota-trial-*` subcommands are clients of that API and read the
//! bearer token from the private configuration, so the token never appears in
//! shell arguments.

#![forbid(unsafe_code)]
#![warn(clippy::print_stdout)]

use std::{path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use reqwest::Method;
use serde::Serialize;
use tracing::info;
use wellfed_client::install::{SETUP_ADDRESS, SETUP_SSID_PREFIX, SetupFrame};
use wellfed_formats::bench::lease::Lease;
use wellfed_stock::{
    clock,
    config::{self, State},
    ota,
    server::{self, OtaTrial, Server},
};

#[derive(Parser)]
#[command(
    version,
    about = "The bench's server for stock PLAF103 1.1.15: its broker and the install ledger"
)]
struct Cli {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Write an installer's config: fresh secrets and the broker address,
    /// and no device. The first stock feeder that connects while `serve
    /// --enroll-minutes` runs becomes its device. Never replaces a file.
    InitInstaller {
        /// Specific local IPv4 address and port the feeder reaches: the
        /// broker the setup frame names.
        #[arg(long)]
        mqtt_listen: std::net::SocketAddr,
        /// IANA timezone used for device clock replies.
        #[arg(long, default_value = "UTC")]
        timezone: chrono_tz::Tz,
        #[arg(long, default_value = "private/server.json")]
        output: PathBuf,
    },

    /// Start the Rust broker, device protocol service and loopback HTTP API.
    Serve {
        #[arg(long, default_value = "private/server.json")]
        config: PathBuf,
        /// With an installer's config and no feeder enrolled yet: trust the
        /// first stock feeder that connects within this many minutes.
        #[arg(long)]
        enroll_minutes: Option<u64>,
    },

    /// Inspect authenticated local status without printing credentials.
    Status {
        #[arg(long, default_value = "private/server.json")]
        config: PathBuf,
    },

    /// Close the latest uncertain/failed trial from fresh version evidence after
    /// the documented waiting periods; never sends a device command.
    OtaTrialResolve {
        #[arg(long, default_value = "private/server.json")]
        config: PathBuf,
        #[arg(long)]
        request_id: uuid::Uuid,
        #[arg(long)]
        note: String,
    },

    /// Close the latest submitted or uncertain trial on the replacement's
    /// answer: the build identity and bank its device API reported, and a
    /// note of what answered, and when. Refused while stock still reports,
    /// or for another build or bank. Never sends a device command.
    OtaTrialAnswered {
        #[arg(long, default_value = "private/server.json")]
        config: PathBuf,
        #[arg(long)]
        request_id: uuid::Uuid,
        /// The build identity the device API reported (64 hex).
        #[arg(long)]
        build: String,
        /// The bank the device API reported; the trial closes only on 0.
        #[arg(long)]
        bank: u8,
        #[arg(long)]
        note: String,
    },

    /// Point a stock feeder in setup mode at a network and this
    /// server with stock's own type-3 setup frame, sent once to its setup
    /// access point (join that network first). The passphrase is read from
    /// stdin, never the command line.
    SetupSend {
        #[arg(long, default_value = "private/server.json")]
        config: PathBuf,
        /// The network stock should join; its passphrase comes from stdin.
        #[arg(long)]
        ssid: String,
        /// The account ID stock keeps beside the broker (ASCII digits).
        #[arg(long)]
        id: String,
        /// The broker; this server's MQTT address otherwise.
        #[arg(long)]
        broker: Option<std::net::SocketAddr>,
        #[arg(long, default_value_t = std::net::SocketAddr::V4(SETUP_ADDRESS))]
        to: std::net::SocketAddr,
        /// The feeder's setup network this computer is on now, as you read it.
        /// This tool does not check: the frame carries the password in the
        /// clear, and a home router may hold the same address. `wellfed install`
        /// checks for itself.
        #[arg(long)]
        on_feeder_network: String,
    },
}

#[tokio::main]
#[expect(
    clippy::print_stdout,
    reason = "the CLI's results and reports are its stdout"
)]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "wellfed_stock=info,mqtt5=warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    match Cli::parse().command {
        Action::InitInstaller {
            mqtt_listen,
            timezone,
            output,
        } => {
            config::init_installer(&output, mqtt_listen, timezone)?;
            println!(
                "Wrote an installer's configuration to {}; no feeder is enrolled yet.",
                output.display()
            );
        }
        Action::Serve {
            config,
            enroll_minutes,
        } => serve(config, enroll_minutes).await?,
        Action::Status { config } => {
            api_request(config, Method::GET, "/api/status", None::<&()>).await?
        }
        Action::SetupSend {
            config,
            ssid,
            id,
            broker,
            to,
            on_feeder_network,
        } => {
            use wellfed_stock::setup::send;
            anyhow::ensure!(
                on_feeder_network.starts_with(SETUP_SSID_PREFIX),
                "{on_feeder_network:?} is not a feeder's setup network, whose name starts \
                 {SETUP_SSID_PREFIX}; nothing was sent"
            );
            let _lease = bench_lease(&config)?;
            let broker = match broker {
                Some(broker) => broker,
                None => match config::state(&config)? {
                    State::Device(config) => config.mqtt_listen,
                    State::Unenrolled(config) => config.mqtt_listen,
                },
            };
            println!("Passphrase for {ssid} (read from stdin):");
            let mut password = String::new();
            std::io::stdin().read_line(&mut password)?;
            let setup = SetupFrame {
                ssid,
                password: password.trim_end_matches(['\r', '\n']).to_owned(),
                host: broker.ip().to_string(),
                port: broker.port(),
                id,
            };
            match send(&setup, to, std::time::Duration::from_secs(5))? {
                Some(answer) => println!(
                    "Sent once to {to}; the feeder answered {} bytes: {}",
                    answer.len(),
                    wellfed_formats::hex::encode(&answer)
                ),
                None => println!("Sent once to {to}; no answer within 5 s (not resent)"),
            }
        }
        Action::OtaTrialResolve {
            config,
            request_id,
            note,
        } => {
            api_request(
                config,
                Method::POST,
                "/api/ota/trial/resolve",
                Some(&ota::trial::ResolveInput { request_id, note }),
            )
            .await?;
        }

        Action::OtaTrialAnswered {
            config,
            request_id,
            build,
            bank,
            note,
        } => {
            api_request(
                config,
                Method::POST,
                "/api/ota/trial/answered",
                Some(&ota::trial::AnsweredInput {
                    request_id,
                    build: build.to_ascii_lowercase(),
                    bank,
                    note,
                }),
            )
            .await?;
        }
    }

    Ok(())
}

/// Resolves on SIGTERM; never elsewhere.
async fn terminated() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?
            .recv()
            .await;
        Ok(())
    }
    #[cfg(not(unix))]
    std::future::pending().await
}

/// The bench lease beside `config` (in a checkout, `private/`), for what
/// drives the feeder: `setup-send`, for its one frame. `serve` takes none: it
/// only answers, beside the bench command that drives the feeder.
fn bench_lease(config: &std::path::Path) -> Result<Lease> {
    // The directory the config is named in, not where a symlinked config
    // points: a worktree whose `private/server.json` links to the main
    // checkout's keeps its lease in its own `private/`, beside the bench
    // that runs from it.
    let absolute = std::path::absolute(config).with_context(|| format!("{}", config.display()))?;
    let named = absolute.parent().context("config directory missing")?;
    let directory = named
        .canonicalize()
        .with_context(|| format!("{}", named.display()))?;
    Lease::acquire_in(&directory).context("bench lease")
}

async fn serve(path: PathBuf, enroll_minutes: Option<u64>) -> Result<()> {
    let path = path.canonicalize()?;
    let mut server = Server::start(
        &path,
        server::Options {
            trial: OtaTrial::Disabled,
            enroll: enroll_minutes.map(|minutes| Duration::from_secs(minutes * 60)),
            api: server::Api::Loopback,
            bind: server::Bind::Configured,
            stock_plans: clock::StockPlans::Unknown,
        },
    )?;

    // A SIGTERM (kill, timeout) must stop the broker too, through the same
    // stop as Ctrl-C.
    let outcome: Result<()> = tokio::select! {
        result = server.finished() => result,
        result = tokio::signal::ctrl_c() => result.context("shutdown signal failed"),
        result = terminated() => result.context("shutdown signal failed"),
    };
    server.stop().await;

    info!("service stopped");
    outcome
}

#[expect(
    clippy::print_stdout,
    reason = "the API's answer is the command's stdout"
)]
async fn api_request(
    path: PathBuf,
    method: Method,
    route: &str,
    body: Option<&impl Serialize>,
) -> Result<()> {
    let config = config::load(&path)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .no_proxy()
        .build()?;

    let mut request = client
        .request(method, format!("http://{}{route}", config.http_listen))
        .bearer_auth(&config.api_token);
    if let Some(body) = body {
        request = request.json(body);
    }

    let response = request.send().await?;
    let status = response.status();
    let body: serde_json::Value = response.json().await?;
    println!("{}", serde_json::to_string_pretty(&body)?);
    if !status.is_success() {
        bail!("local API returned {status}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
    }
}
