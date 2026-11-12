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
//! try's failure counts towards "unreachable", only on the network where
//! the feeder last answered, and only when no owner's server answers for
//! the feeder.
//!
//! Away from home (off the network the feeder last answered on), a feeder
//! the owner's server holds is read through the server instead
//! ([`ServerRoute`]). So is one that never answered this phone and doesn't
//! now, and one silent on the last try at home, since a network is told by
//! its gateway and subnet alone and a café's can match. The server's read
//! is `GET /feeders/{id}/alerts`, the table the server last read under the
//! same cursor a session reads, applied to the same watch, so an alert seen
//! at home and again through the server posts once. The server's judgement
//! that it can't reach the feeder stands in for the phone's. That read is a
//! `GET` and nothing else: the route is the only one a catch-up has.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use jiff::Timestamp;
use tokio::time::timeout;
use wellfed_api::{FeederAlerts, FeederId};
use wellfed_client::alerts::{Change, Context, Cursor, FeederClock, Relayed, RelayedTable, Watch};
use wellfed_connect::{Connection, Error};
use wellfed_model::{AlertsView, Answer, Hex, Op};
use wellfed_server_client::Client;
use wellfed_session::noise::Keypair;

use crate::alerts::{self, Sitting};
use crate::api::alerts::{Attempt, CatchUpEnd};
use crate::api::error::BridgeError;
use crate::api::keys::random;
use crate::api::server_error::ServerErrorKind;

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
    /// A fingerprint of the Wi-Fi network the phone is on, or `None` off
    /// Wi-Fi.
    pub(crate) network: Option<String>,
    /// The owner's server, when it holds this feeder.
    pub(crate) server: Option<ServerRoute>,
}

/// A feeder as the owner's server holds it.
pub(crate) struct ServerRoute {
    pub(crate) client: Client,
    pub(crate) feeder: FeederId,
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
    // Off the feeder's home network the server answers for it. On Wi-Fi
    // before this phone has ever reached it, or with a home learned under
    // an older fingerprint, home is unknown: the feeder is tried first, and
    // the server only if nothing answers.
    let home = watch
        .reach
        .home
        .as_deref()
        .filter(|home| crate::api::alerts::current_form(home));
    let unknown_home = home.is_none() && target.network.is_some();
    let away = !unknown_home && (target.network.is_none() || target.network.as_deref() != home);
    if away && let Some(server) = &target.server {
        return through_server(dir, target, server, &identity, &now).await;
    }
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
    let address = tried.filter(|_| target.network.is_some() || target.manual.is_some());

    let visit = match &address {
        Some(address) => visit(address, target, &watch).await,
        None => Visit::NoAnswer,
    };
    // A network only looks like home by its gateway and subnet, which a
    // café's can share. So silence on the last try at home goes to the
    // owner's server too: it reads the feeder, or judges it unreachable
    // itself. Only its answer stands in for the phone's: a server that is
    // down or turns this phone away leaves the silence at home to count as
    // the phone's own, so a hung feeder still raises its alert, while a
    // lookalike's few failures still wait out the alert's thresholds.
    let last = *target.attempt != Attempt::First;
    if (unknown_home || last)
        && matches!(visit, Visit::NoAnswer)
        && let Some(server) = &target.server
    {
        let (end, changes) = through_server(dir, target, server, &identity, &now).await?;
        let unheard = matches!(
            end,
            CatchUpEnd::ServerUnanswered | CatchUpEnd::ServerRefused
        );
        if unknown_home || !unheard {
            return Ok((end, changes));
        }
    }
    let end = match (&visit, &address) {
        (Visit::Table(..), _) => CatchUpEnd::Read,
        (Visit::Unchanged(_), _) => CatchUpEnd::Unchanged,
        (Visit::Answered, _) => CatchUpEnd::Answered,
        (Visit::NoAnswer, None) if target.network.is_none() => CatchUpEnd::Away,
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

/// A catch-up away from home, through the owner's server.
async fn through_server(
    dir: &Path,
    target: &Target<'_>,
    server: &ServerRoute,
    identity: &str,
    now: &impl Fn() -> Timestamp,
) -> Result<(CatchUpEnd, Vec<Change>), BridgeError> {
    let read = timeout(SITTING, server.client.feeder_alerts(server.feeder)).await;
    let alerts = match read {
        Ok(Ok(alerts)) => alerts,
        // Turned away: every round goes the same way until the owner acts.
        Ok(Err(error)) if refused(&crate::server::kind(&error)) => {
            return Ok((CatchUpEnd::ServerRefused, pending(dir, identity).await?));
        }
        // The server's silence says nothing of the feeder.
        Ok(Err(_)) | Err(_) => {
            return Ok((CatchUpEnd::ServerUnanswered, pending(dir, identity).await?));
        }
    };

    let [a, b, c, d, ..] = *target.keys.public();
    let own = Hex([a, b, c, d]);
    let changes = alerts::change(dir, |state| {
        state.catch_up(dir, identity, |watched, mutes| {
            let clock = match alerts.read {
                Some(read) if read.clock_valid => FeederClock::Known,
                _ => FeederClock::Unknown,
            };
            let context = Context {
                now: now(),
                mutes,
                own,
                clock,
            };
            relayed(&mut watched.watch, &alerts, &context);
        })
    })
    .await??;
    Ok((CatchUpEnd::Server, changes))
}

/// Whether the server turned this phone away from the feeder, rather than
/// failed to answer: its token no longer holds, its account or role may not
/// read the feeder, or the server no longer has it.
const fn refused(kind: &ServerErrorKind) -> bool {
    matches!(
        kind,
        ServerErrorKind::TokenRevoked
            | ServerErrorKind::AccountDisabled
            | ServerErrorKind::Forbidden
            | ServerErrorKind::NotFound
    )
}

/// Apply what the server last read of the feeder, as a session's read
/// would be applied, unless this phone knows better from its own
/// ([`Watch::apply_relayed`]).
fn relayed(watch: &mut Watch, alerts: &FeederAlerts, context: &Context<'_>) {
    let view = alerts.read.map(|read| AlertsView {
        epoch: read.epoch,
        generation: read.generation,
        more: read.more,
        settling: read.settling,
        alerts: alerts.alerts.clone(),
        unread: read.unread,
    });
    let table = alerts
        .read
        .zip(view.as_ref())
        .map(|(read, view)| RelayedTable {
            cursor: Cursor {
                epoch: read.epoch,
                boot: read.boot,
                generation: read.generation,
            },
            view,
            checked_at: read.checked,
        });
    let relayed = Relayed {
        table,
        unreachable: alerts.unreachable,
    };
    watch.apply_relayed(relayed, context);
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

#[cfg(test)]
mod tests {
    use wellfed_api::AlertsRead;
    use wellfed_client::alerts::Mutes;
    use wellfed_model::{Alert, Generation, OpenAlert};

    use super::*;

    fn at(second: i64) -> Timestamp {
        Timestamp::from_second(1_800_000_000 + second).expect("a time")
    }

    fn food_low() -> OpenAlert {
        OpenAlert {
            at: Some(at(0)),
            alert: Alert::FoodLow,
        }
    }

    fn view(generation: u32, alerts: &[OpenAlert]) -> AlertsView {
        AlertsView {
            epoch: 1,
            generation: Generation(generation),
            more: false,
            settling: false,
            alerts: alerts.to_vec(),
            unread: 0,
        }
    }

    fn cursor(generation: u32) -> Cursor {
        Cursor {
            epoch: 1,
            boot: 4,
            generation: Generation(generation),
        }
    }

    /// The server's copy of the table at `generation`, read at `read_at`.
    fn served(generation: u32, alerts: &[OpenAlert], read_at: Timestamp) -> FeederAlerts {
        FeederAlerts {
            read: Some(AlertsRead {
                epoch: 1,
                boot: 4,
                generation: Generation(generation),
                more: false,
                settling: false,
                unread: 0,
                clock_valid: true,
                at: read_at,
                checked: read_at,
            }),
            alerts: alerts.to_vec(),
            unreachable: None,
        }
    }

    /// A home read of `table` at `generation`, its changes confirmed as
    /// Dart would.
    fn read_at_home(
        watch: &mut Watch,
        generation: u32,
        table: &[OpenAlert],
        context: &Context<'_>,
    ) {
        watch.apply(cursor(generation), &view(generation, table), context);
        watch.reached(Some("home".into()), context.now);
        for change in watch.changes() {
            watch.confirm(&change);
        }
    }

    fn context(mutes: &Mutes, now: Timestamp) -> Context<'_> {
        Context {
            now,
            mutes,
            own: Hex([0; 4]),
            clock: FeederClock::Known,
        }
    }

    #[test]
    fn a_server_one_generation_behind_a_home_read_ends_and_reposts_nothing() {
        let mutes = Mutes::default();
        let mut watch = Watch::default();
        read_at_home(&mut watch, 5, &[food_low()], &context(&mutes, at(0)));

        // Read by the server just before the phone's own read.
        relayed(
            &mut watch,
            &served(4, &[], at(-30)),
            &context(&mutes, at(600)),
        );

        assert!(watch.changes().is_empty());
        assert_eq!(watch.listed().count(), 1);
        assert_eq!(watch.cursor, Some(cursor(5)));
    }

    #[test]
    fn a_server_copy_read_well_after_the_phone_s_last_read_counts() {
        let mutes = Mutes::default();
        let mut watch = Watch::default();
        read_at_home(&mut watch, 5, &[], &context(&mutes, at(0)));

        let later = served(6, &[food_low()], at(900));
        relayed(&mut watch, &later, &context(&mutes, at(960)));

        assert_eq!(watch.changes().len(), 1);
    }

    #[test]
    fn an_alert_that_ended_at_home_is_not_revived_by_a_lagging_server() {
        let mutes = Mutes::default();
        let mut watch = Watch::default();
        read_at_home(&mut watch, 4, &[food_low()], &context(&mutes, at(0)));
        read_at_home(&mut watch, 5, &[], &context(&mutes, at(60)));

        relayed(
            &mut watch,
            &served(4, &[food_low()], at(30)),
            &context(&mutes, at(600)),
        );

        assert!(watch.changes().is_empty());
        assert_eq!(watch.listed().count(), 0);
    }

    /// At a known home where the feeder is silent, a server that is down or
    /// turns this phone away answers nothing for it: the phone's own
    /// silence counts, and past the thresholds the feeder is unreachable.
    #[test]
    fn a_silent_feeder_at_home_counts_when_the_server_cannot_answer_for_it() {
        use std::cell::Cell;

        use wellfed_client::alerts::{Notice, UNREACHABLE_AFTER, UNREACHABLE_FOR};
        use wellfed_server_testkit::{TestServer, local};

        use crate::api::server_views::ServerLink;
        use crate::api::store::{self, AppStore, KnownFeeder, swap_store};

        const HOME: &str = "gateway-subnet:home";
        let identity = [0xab; 32];
        let hex = "ab".repeat(32);

        local(async {
            let turning_away = TestServer::start().await;
            let servers = [
                "http://127.0.0.1:9".to_owned(),
                format!("http://{}", turning_away.address()),
            ];
            for url in servers {
                let dir = tempfile::tempdir().unwrap();
                let dir = dir.path();
                let feeder = KnownFeeder {
                    identity: hex.clone(),
                    name: "Kitchen".into(),
                    address: None,
                    manual_address: None,
                };
                let store = AppStore {
                    feeders: vec![feeder],
                    ..AppStore::default()
                };
                let path = dir.join(store::FILE_NAME);
                let path = path.to_string_lossy();
                assert!(swap_store(&path, &AppStore::default(), &store).unwrap());
                alerts::change(dir, |state| {
                    state.catch_up(dir, &hex, |watched, _| {
                        watched.watch.reached(Some(HOME.into()), at(0));
                    })
                })
                .await
                .unwrap()
                .unwrap();

                let link = ServerLink {
                    url: url.clone(),
                    user: "ada".into(),
                    token_id: "00000000-0000-0000-0000-000000000001".into(),
                };
                let keys = Keypair::from_secret([0x31; 32]);
                let attempt = Attempt::Last {
                    located: Some("127.0.0.1:9".into()),
                };
                let clock = Cell::new(at(60));
                let mut unreachable = false;
                for round in 0..UNREACHABLE_AFTER {
                    let client = crate::api::server::alert_client(
                        &link,
                        "wf_0000000000000000000000000000000000000000000",
                    )
                    .unwrap();
                    let target = Target {
                        keys: &keys,
                        identity,
                        manual: None,
                        known: None,
                        attempt: &attempt,
                        network: Some(HOME.into()),
                        server: Some(ServerRoute {
                            client,
                            feeder: crate::server::ids::feeder(
                                "0192c5d0-3333-7000-8000-000000000001",
                            )
                            .unwrap(),
                        }),
                    };
                    let (end, changes) = catch_up(dir, &target, || clock.get()).await.unwrap();

                    assert_eq!(end, CatchUpEnd::Failed, "{url}, round {round}");
                    unreachable |= changes.iter().any(|change| {
                        matches!(change, Change::Post(tracked) if matches!(tracked.notice, Notice::Unreachable { .. }))
                    });
                    clock.set(clock.get() + UNREACHABLE_FOR);
                }
                assert!(unreachable, "{url}: the feeder never read as unreachable");
            }
            turning_away.stop().await;
        });
    }
}
