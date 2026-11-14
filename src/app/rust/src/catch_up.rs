//! One catch-up with one feeder: a short read-only session that reads the
//! status, reads the alert table if the status shows it moved, and applies it
//! to the feeder's [`Watch`] in `alerts.json`. Dart only starts one, from the
//! periodic job, the "instant while home" service or a screen, and posts what
//! it returns.
//!
//! A catch-up reads and nothing else. It reaches the session only through
//! [`Read`], whose ops are the status and the alert table, so no path from
//! here can send a feed, a write, a retry or the clock sync an ordinary
//! session offers.
//!
//! A catch-up is short and never left open: [`SITTING`] bounds it, and a
//! catch-up that runs out of time counts as one that heard nothing.
//!
//! Only the last try of a round judges the link. The first tries where the
//! feeder last answered; if nothing answers there, the caller looks for it
//! with NSD and tries again, since a lease may have moved it, and only that
//! try's failure counts towards "unreachable", and only on the network
//! where the feeder last answered.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use jiff::Timestamp;
use tokio::time::timeout;
use wellfed_client::alerts::{Change, Context, Cursor, FeederClock, Watch};
use wellfed_connect::{Connection, Error};
use wellfed_model::{Answer, Hex, Op};
use wellfed_session::noise::Keypair;

use crate::alerts::{self, Sitting};
use crate::api::alerts::{Attempt, CatchUpEnd};
use crate::api::error::BridgeError;
use crate::api::keys::random;

/// The longest a catch-up stays on the feeder once it dials: the handshake
/// and two reads, each answered at once by a feeder that is there.
const SITTING: Duration = Duration::from_secs(20);

/// What a catch-up may ask: reads only.
#[derive(Clone, Copy, Debug)]
enum Read {
    Status,
    Alerts,
}

impl Read {
    const fn op(self) -> Op {
        match self {
            Self::Status => Op::Status,
            Self::Alerts => Op::GetAlerts,
        }
    }
}

/// The feeder a catch-up is with, and where it might be.
pub(crate) struct Target<'a> {
    pub(crate) keys: &'a Keypair,
    pub(crate) identity: [u8; 32],
    /// Where the owner said to dial it, which is tried first and last.
    pub(crate) manual: Option<&'a str>,
    /// Where the app last reached it.
    pub(crate) known: Option<&'a str>,
    pub(crate) attempt: &'a Attempt,
    /// A fingerprint of the network the phone or computer is on, or `None`
    /// off Wi-Fi or on a network it can't name.
    pub(crate) network: Option<String>,
    /// Whether it is on a network at all. On one it can't name it dials,
    /// but with no fingerprint learns no home and counts no failure.
    pub(crate) online: bool,
}

/// What the session found.
enum Visit {
    /// The table and the cursor it was read under, and the feeder's clock
    /// as its status showed it.
    Table(Cursor, Box<wellfed_model::AlertsView>, FeederClock),
    Unchanged(FeederClock),
    Answered,
    NoAnswer,
}

/// One catch-up with `target`'s feeder: its end, and every change the phone
/// still has to make for this feeder, older ones included.
pub(crate) async fn catch_up(
    dir: &Path,
    target: &Target<'_>,
    now: impl Fn() -> Timestamp,
) -> Result<(CatchUpEnd, Vec<Change>), BridgeError> {
    let identity = wellfed_session::hex::Lower(&target.identity).to_string();
    if alerts::held() {
        return Ok((CatchUpEnd::Skipped, pending(dir, &identity).await?));
    }
    let Some(_sitting) = Sitting::take(target.identity) else {
        return Ok((CatchUpEnd::Skipped, pending(dir, &identity).await?));
    };

    let state = alerts::read(dir).await?;
    let watched = state.watched(&identity);
    let watch = watched
        .map(|watched| watched.watch.clone())
        .unwrap_or_default();
    let tried = match target.attempt {
        Attempt::First => target
            .manual
            .or_else(|| watched.and_then(|watched| watched.address.as_deref()))
            .or(target.known),
        Attempt::Last { located } => target.manual.or(located.as_deref()).or(target.known),
    }
    .map(str::to_owned);
    // Off Wi-Fi the feeder is out of reach, unless the owner gave an address
    // for a route to it, such as a VPN's.
    let address = tried.filter(|_| target.online || target.manual.is_some());

    let visit = match &address {
        Some(address) => visit(address, target, &watch).await,
        None => Visit::NoAnswer,
    };
    let end = match (&visit, &address) {
        (Visit::Table(..), _) => CatchUpEnd::Read,
        (Visit::Unchanged(_), _) => CatchUpEnd::Unchanged,
        (Visit::Answered, _) => CatchUpEnd::Answered,
        (Visit::NoAnswer, None) if !target.online => CatchUpEnd::Away,
        (Visit::NoAnswer, _) if *target.attempt == Attempt::First => CatchUpEnd::Unanswered,
        (Visit::NoAnswer, _) => CatchUpEnd::Failed,
    };

    let [a, b, c, d, ..] = *target.keys.public();
    let own = Hex([a, b, c, d]);
    let changes = alerts::change(dir, |state| {
        state.catch_up(dir, &identity, |watched, mutes| {
            let clock = match visit {
                Visit::Table(.., clock) | Visit::Unchanged(clock) => clock,
                Visit::Answered | Visit::NoAnswer => FeederClock::Unknown,
            };
            let context = Context {
                now: now(),
                mutes,
                own,
                clock,
            };
            match visit {
                Visit::Table(cursor, view, _) => {
                    watched.watch.apply(cursor, &view, &context);
                    watched.watch.reached(target.network.clone(), context.now);
                    watched.address = address;
                }
                Visit::Unchanged(_) => {
                    watched.watch.lapse(&context);
                    watched.watch.reached(target.network.clone(), context.now);
                    watched.address = address;
                }
                Visit::Answered => watched.watch.reached(target.network.clone(), context.now),
                Visit::NoAnswer if end == CatchUpEnd::Failed => {
                    watched
                        .watch
                        .unanswered(target.network.as_deref(), &context);
                }
                Visit::NoAnswer => {}
            }
        })
    })
    .await??;
    Ok((end, changes))
}

/// The changes still to make for the feeder `identity`, with no catch-up.
async fn pending(dir: &Path, identity: &str) -> Result<Vec<Change>, BridgeError> {
    let state = alerts::read(dir).await?;
    Ok(state
        .watched(identity)
        .map(|watched| watched.watch.changes())
        .unwrap_or_default())
}

/// The session itself, bounded by [`SITTING`].
async fn visit(address: &str, target: &Target<'_>, watch: &Watch) -> Visit {
    let Ok(address) = wellfed_connect::parse_address(address) else {
        return Visit::NoAnswer;
    };
    let Ok(ephemeral) = random() else {
        return Visit::NoAnswer;
    };
    match timeout(SITTING, read(address, target, watch, ephemeral)).await {
        Ok(Ok(visit)) => visit,
        // The feeder's own refusal: it is there.
        Ok(Err(Error::UnknownKey | Error::Version(_) | Error::Full | Error::Legacy)) => {
            Visit::Answered
        }
        Ok(Err(_)) | Err(_) => Visit::NoAnswer,
    }
}

async fn read(
    address: SocketAddr,
    target: &Target<'_>,
    watch: &Watch,
    ephemeral: [u8; 32],
) -> Result<Visit, Error> {
    let mut connection = Connection::open(target.keys, target.identity, address, ephemeral).await?;
    let epoch = connection.epoch();

    let Answer::Status(status) = ask(&mut connection, Read::Status).await? else {
        return Ok(Visit::Answered);
    };
    let cursor = Cursor {
        epoch,
        boot: status.boot_count,
        generation: status.alerts_generation,
    };
    let clock = match status.clock {
        Some(_) => FeederClock::Known,
        None => FeederClock::Unknown,
    };
    if !watch.needs_read(cursor) {
        return Ok(Visit::Unchanged(clock));
    }

    let Answer::Alerts(view) = ask(&mut connection, Read::Alerts).await? else {
        return Ok(Visit::Answered);
    };
    // The table may have moved since the status: its own generation is the
    // one it was read under.
    let cursor = Cursor {
        epoch: view.epoch,
        generation: view.generation,
        ..cursor
    };
    Ok(Visit::Table(cursor, Box::new(view), clock))
}

async fn ask(connection: &mut Connection, read: Read) -> Result<Answer, Error> {
    let id = random().map_err(|_| Error::Timeout)?;
    connection
        .ask(id, &read.op())
        .await
        .map_err(wellfed_connect::AskError::into_error)
}
