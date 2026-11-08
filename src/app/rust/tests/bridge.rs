//! The app's Rust half against the simulated feeder, which runs the real
//! feeder's pairing, panel and feeding owner:
//! - the BLE setup driver proves its code and enrolls, carried over the
//!   recovery window's TCP framing (the same frames as the characteristic);
//! - LAN recovery reports the code window, then the feeder's identity;
//! - a session shows the card, runs a feed to its `Fed` end, reports a
//!   refused feed, and reports a feed whose answer never came as
//!   unanswered, without sending it again.

use std::future::Future;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Notify, mpsc};
use tokio::task::LocalSet;
use wellfed_app_bridge::api::error::ErrorKind;
use wellfed_app_bridge::api::feeder::{
    FeederRefusal, FeederReply, FeederRequest, NotAsked, PlanEdit, PlanView, PortionChange,
    SettingsChanges,
};
use wellfed_app_bridge::api::join::{JoinAnswer, invite_feeder, join_feeder};
use wellfed_app_bridge::api::keys::ClientKey;
use wellfed_app_bridge::api::model::{CallClip, ClientRole, FeedOutcome, StatsSection};
use wellfed_app_bridge::api::session::{Card, FedBy, FeedRefusal, HistoryBody, SessionUpdate};
use wellfed_app_bridge::api::setup::{
    SetupDriver, SetupStep, recover, setup_control, setup_service,
};
use wellfed_app_bridge::api::store::{
    AppStore, KnownFeeder, OpenedStore, StoreKind, feeder_address, load_store, open_store,
    save_store, store_notice_read,
};
use wellfed_app_bridge::requests;
use wellfed_app_bridge::session::{self, Command};
use wellfed_client::setup::{Ask, Session, panel_position};
use wellfed_connect::Connection;
use wellfed_feeder_sim::flash::Flash;
use wellfed_feeder_sim::harness::{Relay, Running};
use wellfed_feeder_sim::serve::Control;
use wellfed_session::noise::Keypair;
use wellfed_session::setup_code::Code;

const OWNER: [u8; 32] = [0x21; 32];
const WAIT: Duration = Duration::from_secs(20);

fn local(test: impl Future<Output = ()>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(LocalSet::new().run_until(test));
}

/// Unlock the panel and open the recovery window, as the owner holding
/// Lock, then Lock and Dispense.
fn open_window(sim: &Running) {
    sim.type_line("hold lock").unwrap();
    sim.type_line("hold lock+dispense").unwrap();
}

fn press(sim: &Running, positions: &[u8]) {
    let presses: Vec<String> = positions.iter().map(u8::to_string).collect();
    sim.type_line(&format!("press {}", presses.join(" ")))
        .unwrap();
}

#[test]
fn the_setup_driver_proves_its_code_and_enrolls_as_owner() {
    local(async {
        let sim = Running::start(&Flash::memory(), None).await;
        open_window(&sim);
        let key = ClientKey::restore(vec![0x31; 32]).unwrap();
        let mut driver = SetupDriver::new(&key, "phone".into(), None).unwrap();
        let code = driver.code();
        assert_eq!(code.len(), 6);
        assert!(code.iter().all(|position| (1..=4).contains(position)));

        // The recovery session opens with a clear byte naming it; after
        // that the frames are the characteristic's.
        let mut stream = tokio::net::TcpStream::connect(sim.address).await.unwrap();
        let mut hello = vec![1, 0, 2];
        hello.extend(driver.start().unwrap().concat());
        stream.write_all(&hello).await.unwrap();

        let mut buffer = [0u8; 1024];
        let done = 'exchange: loop {
            let n = tokio::time::timeout(WAIT, stream.read(&mut buffer))
                .await
                .unwrap()
                .unwrap();
            assert_ne!(n, 0, "the feeder closed the session");
            // Split each read, as notifications split frames.
            for piece in buffer[..n].chunks(7) {
                for step in driver.notified(piece.to_vec()).unwrap() {
                    match step {
                        SetupStep::Write { pieces } => {
                            assert!(pieces.iter().all(|piece| piece.len() <= 20));
                            stream.write_all(&pieces.concat()).await.unwrap();
                        }
                        SetupStep::EnterCode { seconds } => {
                            assert_eq!(seconds, 60);
                            press(&sim, &code);
                        }
                        done @ SetupStep::Done { .. } => break 'exchange done,
                        SetupStep::Refused { refusal } => panic!("refused: {refusal:?}"),
                        SetupStep::Failed { error } => panic!("failed: {error}"),
                    }
                }
            }
        };
        let SetupStep::Done { feeder, .. } = done else {
            unreachable!()
        };
        assert_eq!(feeder, sim.identity.to_vec());

        let keys = Keypair::from_secret([0x31; 32]);
        let owner = Connection::open(&keys, sim.identity, sim.address, [0xa1; 32])
            .await
            .unwrap();
        assert_eq!(owner.role(), wellfed_model::ClientRole::Owner);
        sim.stop().await;
    });
}

#[test]
fn recovery_reports_the_code_window_then_the_feeders_identity() {
    local(async {
        let sim = Running::start(&Flash::memory(), None).await;
        open_window(&sim);
        let code = Code::from_bits(0x0bad);
        let ask = Ask {
            label: "phone".into(),
            wifi: None,
        };
        let keys = Keypair::from_secret([0x32; 32]);
        let session = Session::new(&keys, [0x42; 32], ask, code, [0x43; 32]).unwrap();
        let positions: Vec<u8> = code.presses().iter().map(|&b| panel_position(b)).collect();

        let mut steps = Vec::new();
        recover(session, &sim.address.to_string(), &Notify::new(), |step| {
            if let SetupStep::EnterCode { .. } = step {
                press(&sim, &positions);
            }
            steps.push(step);
            true
        })
        .await
        .unwrap();

        assert!(matches!(steps[0], SetupStep::EnterCode { seconds: 60 }));
        let Some(SetupStep::Done { feeder, .. }) = steps.last() else {
            panic!("no Done: {} steps", steps.len());
        };
        assert_eq!(feeder, &sim.identity.to_vec());
        sim.stop().await;
    });
}

#[test]
fn closing_a_recovery_hangs_up_while_the_code_window_waits() {
    local(async {
        let sim = Running::start(&Flash::memory(), None).await;
        open_window(&sim);
        let ask = Ask {
            label: "phone".into(),
            wifi: None,
        };
        let keys = Keypair::from_secret([0x33; 32]);
        let session =
            Session::new(&keys, [0x44; 32], ask, Code::from_bits(0x0123), [0x45; 32]).unwrap();
        let closed = Notify::new();

        // Nobody keys the code in: only the close can end this in time.
        let mut steps = 0;
        tokio::time::timeout(
            Duration::from_secs(5),
            recover(session, &sim.address.to_string(), &closed, |step| {
                if let SetupStep::EnterCode { .. } = step {
                    closed.notify_one();
                }
                steps += 1;
                true
            }),
        )
        .await
        .expect("the close ended the exchange")
        .unwrap();
        assert_eq!(steps, 1);
        sim.stop().await;
    });
}

/// A session against `address`, run on the local set; its updates and
/// the sender of its commands.
fn start_session(
    feeder: [u8; 32],
    address: std::net::SocketAddr,
    sync_clock: bool,
) -> (
    mpsc::UnboundedSender<Command>,
    mpsc::UnboundedReceiver<SessionUpdate>,
) {
    let (commands, receiver) = mpsc::unbounded_channel();
    let (updates, heard) = mpsc::unbounded_channel();
    tokio::task::spawn_local(session::run(
        Keypair::from_secret(OWNER),
        feeder,
        address,
        sync_clock,
        receiver,
        move |update| updates.send(update).is_ok(),
    ));
    (commands, heard)
}

/// The next update `pick` accepts, skipping status refreshes.
async fn next<T>(
    heard: &mut mpsc::UnboundedReceiver<SessionUpdate>,
    mut pick: impl FnMut(SessionUpdate) -> Option<T>,
) -> T {
    tokio::time::timeout(WAIT, async {
        loop {
            let update = heard.recv().await.expect("the session ended");
            if let Some(found) = pick(update) {
                return found;
            }
        }
    })
    .await
    .expect("no such update in time")
}

/// Updates through `pick` until the caller's timeout gives up.
async fn next_forever<T>(
    heard: &mut mpsc::UnboundedReceiver<SessionUpdate>,
    mut pick: impl FnMut(SessionUpdate) -> Option<T>,
) -> T {
    loop {
        let update = heard.recv().await.expect("the session ended");
        if let Some(found) = pick(update) {
            return found;
        }
    }
}

#[test]
fn a_session_shows_the_card_and_runs_a_feed_to_its_end() {
    local(async {
        let owner = *Keypair::from_secret(OWNER).public();
        let sim = Running::start(&Flash::memory(), Some(owner)).await;
        let (commands, mut heard) = start_session(sim.identity, sim.address, false);

        let role = next(&mut heard, |update| match update {
            SessionUpdate::Opened { role, .. } => Some(role),
            _ => None,
        })
        .await;
        assert_eq!(role, ClientRole::Owner);
        let card = next(&mut heard, |update| match update {
            SessionUpdate::Status { card } => Some(card),
            _ => None,
        })
        .await;
        assert!(!card.build.is_empty());
        // The panel boots locked.
        assert_eq!(card.locked, Some(true));

        commands
            .send(Command::Feed {
                portions: 2,
                reply: None,
            })
            .unwrap();
        let (feed, portions) = next(&mut heard, |update| match update {
            SessionUpdate::FeedStarted { feed, portions, .. } => Some((feed, portions)),
            _ => None,
        })
        .await;
        assert_eq!(portions, 2);
        let result = next(&mut heard, |update| match update {
            SessionUpdate::FeedEnded { result } => Some(result),
            _ => None,
        })
        .await;
        assert_eq!(result.feed, feed);
        assert_eq!(result.outcome, FeedOutcome::Done);
        assert_eq!((result.confirmed, result.at_most), (2, 2));
        assert!(!result.uncertain);
        assert_eq!(result.words, "2 of 2 portions, done");

        commands
            .send(Command::Feed {
                portions: 0,
                reply: None,
            })
            .unwrap();
        let refusal = next(&mut heard, |update| match update {
            SessionUpdate::FeedRefused { refusal } => Some(refusal),
            _ => None,
        })
        .await;
        assert_eq!(refusal, FeedRefusal::Portions);

        commands.send(Command::Close).unwrap();
        let error = next(&mut heard, |update| match update {
            SessionUpdate::Closed { error } => Some(error),
            _ => None,
        })
        .await;
        assert_eq!(error, None);
        sim.stop().await;
    });
}

/// A stalled feed's session on a runtime whose clock moves only when idle.
fn paused(test: impl Future<Output = ()>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap();
    runtime.block_on(LocalSet::new().run_until(test));
}

/// A session through `relay`, with a feed of 2 that stays in flight.
async fn feed_in_flight(
    sim: &Running,
    relay: &Relay,
) -> (
    mpsc::UnboundedSender<Command>,
    mpsc::UnboundedReceiver<SessionUpdate>,
) {
    let (commands, mut heard) = start_session(sim.identity, relay.address, false);
    next(&mut heard, |update| match update {
        SessionUpdate::Status { .. } => Some(()),
        _ => None,
    })
    .await;
    // The backlog's quiet moment passes first: a real session is past it.
    tokio::time::sleep(Duration::from_secs(3)).await;

    sim.control(Control::StallFeeds(true));
    commands
        .send(Command::Feed {
            portions: 2,
            reply: None,
        })
        .unwrap();
    next(&mut heard, |update| match update {
        SessionUpdate::FeedStarted { .. } => Some(()),
        _ => None,
    })
    .await;
    (commands, heard)
}

/// A feeder that loses power mid-feed sends no reset, and the screen hears of
/// it from the feed's own silence limit, well before the usual one.
#[test]
fn a_silent_feeder_ends_the_session_of_a_running_feed_within_20_s() {
    paused(async {
        let owner = *Keypair::from_secret(OWNER).public();
        let sim = Running::start(&Flash::memory(), Some(owner)).await;
        let relay = Relay::start(sim.address).await;
        let (_commands, mut heard) = feed_in_flight(&sim, &relay).await;

        relay.pause();
        let error = tokio::time::timeout(
            Duration::from_secs(21),
            next_forever(&mut heard, |update| match update {
                SessionUpdate::FeedUnanswered | SessionUpdate::FeedStarted { .. } => {
                    panic!("the feed was already answered, once");
                }
                SessionUpdate::Closed { error } => Some(error),
                _ => None,
            }),
        )
        .await
        .expect("closed by the feed's silence limit");

        assert!(error.is_some());
        sim.stop().await;
    });
}

/// The reads that keep the link heard from also keep a slow feed's session
/// open, for as long as the feeder keeps answering.
#[test]
fn a_feeder_answering_keeps_a_stalled_feeds_session_open() {
    paused(async {
        let owner = *Keypair::from_secret(OWNER).public();
        let sim = Running::start(&Flash::memory(), Some(owner)).await;
        let relay = Relay::start(sim.address).await;
        let (commands, mut heard) = feed_in_flight(&sim, &relay).await;

        let waited = tokio::time::timeout(
            Duration::from_secs(60),
            next_forever(&mut heard, |update| match update {
                SessionUpdate::Closed { error } => panic!("it closed: {error:?}"),
                _ => None::<()>,
            }),
        )
        .await;
        assert!(waited.is_err());

        // The usual limit is back once the feed has ended: the session
        // survives a 30 s silence that a running feed's would not.
        sim.control(Control::StallFeeds(false));
        next(&mut heard, |update| match update {
            SessionUpdate::FeedEnded { .. } => Some(()),
            _ => None,
        })
        .await;
        relay.pause();
        let silent = tokio::time::timeout(
            Duration::from_secs(25),
            next_forever(&mut heard, |update| match update {
                SessionUpdate::Closed { error } => panic!("it closed: {error:?}"),
                _ => None::<()>,
            }),
        )
        .await;
        assert!(silent.is_err());

        commands.send(Command::Close).unwrap();
        sim.stop().await;
    });
}

/// A feed whose `Fed` never comes does not keep the short limit for good.
#[test]
fn a_stalled_feed_past_its_due_time_goes_back_to_the_usual_limit() {
    paused(async {
        let owner = *Keypair::from_secret(OWNER).public();
        let sim = Running::start(&Flash::memory(), Some(owner)).await;
        let relay = Relay::start(sim.address).await;
        let (commands, mut heard) = feed_in_flight(&sim, &relay).await;

        // Two portions are due after 49 s, and the next turn sees it.
        tokio::time::sleep(Duration::from_secs(56)).await;
        relay.pause();
        let silent = tokio::time::timeout(
            Duration::from_secs(20),
            next_forever(&mut heard, |update| match update {
                SessionUpdate::Closed { error } => panic!("it closed: {error:?}"),
                _ => None::<()>,
            }),
        )
        .await;
        assert!(silent.is_err());

        commands.send(Command::Close).unwrap();
        sim.stop().await;
    });
}

/// A session's first status, past the backlog's quiet moment.
async fn quiet_session(
    sim: &Running,
    relay: &Relay,
) -> (
    mpsc::UnboundedSender<Command>,
    mpsc::UnboundedReceiver<SessionUpdate>,
) {
    let (commands, mut heard) = start_session(sim.identity, relay.address, false);
    next(&mut heard, |update| match update {
        SessionUpdate::Status { .. } => Some(()),
        _ => None,
    })
    .await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    (commands, heard)
}

/// Whether the session closes within `wait`.
async fn closes_within(heard: &mut mpsc::UnboundedReceiver<SessionUpdate>, wait: Duration) -> bool {
    tokio::time::timeout(
        wait,
        next_forever(heard, |update| match update {
            SessionUpdate::Closed { .. } => Some(()),
            _ => None,
        }),
    )
    .await
    .is_ok()
}

/// A healthy quiet session outlasts the limit on its reads alone, and a feeder
/// that goes silent is given up on within it: a power cut closes nothing.
#[test]
fn a_quiet_session_gives_up_on_a_silent_feeder_within_thirty_seconds() {
    paused(async {
        let owner = *Keypair::from_secret(OWNER).public();
        let sim = Running::start(&Flash::memory(), Some(owner)).await;
        let relay = Relay::start(sim.address).await;
        let (commands, mut heard) = quiet_session(&sim, &relay).await;

        assert!(!closes_within(&mut heard, Duration::from_secs(60)).await);

        relay.pause();
        // The last read answered at most one turn before the pause.
        assert!(!closes_within(&mut heard, Duration::from_secs(19)).await);
        assert!(closes_within(&mut heard, Duration::from_secs(12)).await);

        drop(commands);
        sim.stop().await;
    });
}

/// A slot reads nothing while its write waits, and a write waits behind a
/// running feed: a request keeps the usual limit, so saving during another
/// client's long feed doesn't read as a lost feeder. It still ends, at the
/// usual limit.
#[test]
fn a_request_keeps_the_usual_limit_while_it_may_be_waiting() {
    paused(async {
        let owner = *Keypair::from_secret(OWNER).public();
        let sim = Running::start(&Flash::memory(), Some(owner)).await;
        let relay = Relay::start(sim.address).await;
        let (commands, mut heard) = quiet_session(&sim, &relay).await;

        relay.pause();
        let (reply, _answer) = tokio::sync::oneshot::channel();
        commands
            .send(Command::Ask {
                op: wellfed_model::Op::GetSettings,
                reply,
            })
            .unwrap();

        assert!(!closes_within(&mut heard, Duration::from_secs(60)).await);
        assert!(closes_within(&mut heard, Duration::from_secs(35)).await);

        drop(commands);
        sim.stop().await;
    });
}

/// A session that lost its feed never sends it again: only the new session's
/// history reports it, once.
#[test]
fn a_feed_lost_with_its_session_is_read_back_and_not_resent() {
    paused(async {
        let owner = *Keypair::from_secret(OWNER).public();
        let sim = Running::start(&Flash::memory(), Some(owner)).await;
        let relay = Relay::start(sim.address).await;
        let (_commands, mut heard) = feed_in_flight(&sim, &relay).await;

        relay.pause();
        tokio::time::timeout(
            Duration::from_secs(21),
            next_forever(&mut heard, |update| match update {
                SessionUpdate::Closed { .. } => Some(()),
                _ => None,
            }),
        )
        .await
        .expect("closed by the feed's silence limit");

        sim.control(Control::StallFeeds(false));
        tokio::time::sleep(Duration::from_secs(60)).await;

        let (again, mut heard) = start_session(sim.identity, sim.address, false);
        let entries = next(&mut heard, |update| match update {
            SessionUpdate::History { entries, .. } => Some(entries),
            _ => None,
        })
        .await;
        let fed = entries
            .iter()
            .filter(|entry| matches!(entry.body, HistoryBody::Fed { .. }))
            .count();
        assert_eq!(fed, 1);

        again.send(Command::Close).unwrap();
        sim.stop().await;
    });
}

#[test]
fn a_feed_whose_answer_never_comes_is_reported_once_and_not_resent() {
    local(async {
        let owner = *Keypair::from_secret(OWNER).public();
        let flash = Flash::memory();
        let sim = Running::start(&flash, Some(owner)).await;
        let relay = Relay::start(sim.address).await;
        let (commands, mut heard) = start_session(sim.identity, relay.address, false);
        next(&mut heard, |update| match update {
            SessionUpdate::Status { .. } => Some(()),
            _ => None,
        })
        .await;

        // The request reaches the feeder; its answer is lost on the way
        // back, then the connection with it.
        relay.hold_answers();
        commands
            .send(Command::Feed {
                portions: 1,
                reply: None,
            })
            .unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        relay.cut();

        let mut unanswered = 0;
        let error = next(&mut heard, |update| match update {
            SessionUpdate::FeedUnanswered => {
                unanswered += 1;
                None
            }
            SessionUpdate::FeedStarted { .. } => panic!("an answer got through"),
            SessionUpdate::Closed { error } => Some(error),
            _ => None,
        })
        .await;
        assert_eq!(unanswered, 1);
        assert!(error.is_some());

        // The feeder ran it exactly once: one Fed event in its history.
        let keys = Keypair::from_secret(OWNER);
        let mut reader = Connection::open(&keys, sim.identity, sim.address, [0xa2; 32])
            .await
            .unwrap();
        reader.subscribe(None, false).await.unwrap();
        let mut fed = 0;
        while let Ok(pushed) = reader.next(Duration::from_secs(3)).await {
            if let wellfed_model::Pushed::Event(event) = pushed
                && matches!(
                    event.body,
                    wellfed_model::EventBody::Feeder(wellfed_model::FeederEvent::Fed { .. })
                )
            {
                fed += 1;
            }
        }
        assert_eq!(fed, 1);
        sim.stop().await;
    });
}

/// After pairing the session gives the feeder this phone's clock, as
/// `wellfed time` does; the feeder's own event records it, and the next
/// card shows the clock set.
#[test]
fn a_session_after_pairing_syncs_the_feeders_clock() {
    local(async {
        let owner = *Keypair::from_secret(OWNER).public();
        let sim = Running::start(&Flash::memory(), Some(owner)).await;
        let (commands, mut heard) = start_session(sim.identity, sim.address, true);

        next(&mut heard, |update| match update {
            SessionUpdate::ClockSynced => Some(()),
            SessionUpdate::ClockNotSynced => panic!("the feeder refused the clock"),
            _ => None,
        })
        .await;
        let card = next(&mut heard, |update| match update {
            SessionUpdate::Status { card } if card.clock_set => Some(card),
            _ => None,
        })
        .await;
        assert!(card.clock_set);

        commands.send(Command::Close).unwrap();
        sim.stop().await;
    });
}

/// A feeder whose window is shut closes the connection, which is the first
/// thing most owners meet: the failure stays typed, so the screen can say
/// to open the window. On the bridge it travels as a stream value.
#[test]
fn a_recovery_with_the_window_shut_fails_with_its_kind() {
    local(async {
        let sim = Running::start(&Flash::memory(), None).await;
        let ask = Ask {
            label: "phone".into(),
            wifi: None,
        };
        let keys = Keypair::from_secret([0x34; 32]);
        let session =
            Session::new(&keys, [0x46; 32], ask, Code::from_bits(0x0123), [0x47; 32]).unwrap();

        let error = tokio::time::timeout(
            WAIT,
            recover(session, &sim.address.to_string(), &Notify::new(), |_| true),
        )
        .await
        .expect("the feeder answered")
        .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Closed, "{error}");
        sim.stop().await;
    });
}

#[test]
fn the_setup_uuids_are_the_feeders() {
    assert_eq!(setup_service(), "504c5345-5455-5000-8000-000000000000");
    assert_eq!(setup_control(), "504c5345-5455-5000-8000-000000000001");
}

#[test]
fn the_store_round_trips_and_starts_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.json");
    let path = path.to_str().unwrap();

    assert_eq!(load_store(path).unwrap(), AppStore::default());
    let store = AppStore {
        appearance: wellfed_app_bridge::api::store::Appearance::Dark,
        label: "Pixel 8".into(),
        feeders: vec![KnownFeeder {
            identity: "ab".repeat(32),
            name: "Kitchen".into(),
            address: Some("192.168.1.20:4747".into()),
            manual_address: Some("100.101.102.103:4747".into()),
        }],
        server: None,
    };
    save_store(path, &store).unwrap();
    assert_eq!(load_store(path).unwrap(), store);
    assert_eq!(
        open_store(path).unwrap(),
        OpenedStore {
            store,
            set_aside: None
        }
    );
}

/// A store this build cannot read (damaged, or an unversioned file) is kept
/// under another name, never deleted, and the app starts on an empty one.
/// Only the start sets a file aside: a change later reads it strictly.
#[test]
fn a_store_that_does_not_read_is_set_aside_at_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wellfed.json");
    let unversioned = br#"{"label": "", "feeders": [], "appearance": "system", "server": null}"#;
    std::fs::write(&path, unversioned).unwrap();
    let path = path.to_str().unwrap();

    assert_eq!(load_store(path).unwrap_err().kind, ErrorKind::Store);
    let opened = open_store(path).unwrap();
    assert_eq!(opened.store, AppStore::default());
    let set_aside = opened.set_aside.expect("set aside");
    assert_eq!(set_aside.kind, StoreKind::App);
    assert_eq!(std::fs::read(&set_aside.moved_to).unwrap(), unversioned);
    assert_eq!(load_store(path).unwrap(), AppStore::default(), "replaced");

    // An app that dies before the owner reads the notice says it again,
    // and a change meanwhile keeps it.
    let named = AppStore {
        label: "Pixel 8".into(),
        ..AppStore::default()
    };
    save_store(path, &named).unwrap();
    let again = open_store(path).unwrap();
    assert_eq!(again.store, named);
    assert_eq!(again.set_aside, Some(set_aside));

    store_notice_read(path).unwrap();
    assert_eq!(
        open_store(path).unwrap(),
        OpenedStore {
            store: named,
            set_aside: None
        }
    );
}

/// The owner's typed address becomes what a session dials: an IP alone takes
/// the device API's port, and a host name is refused, since nothing resolves
/// it before the session dials.
#[test]
fn a_typed_feeder_address_is_an_ip_and_port() {
    assert_eq!(
        feeder_address(" 192.168.1.20 ").unwrap(),
        "192.168.1.20:4747"
    );
    assert_eq!(
        feeder_address("192.168.1.20:5000").unwrap(),
        "192.168.1.20:5000"
    );
    assert_eq!(
        feeder_address("[fd7a:115c:a1e0::1]:4747").unwrap(),
        "[fd7a:115c:a1e0::1]:4747"
    );
    assert_eq!(
        feeder_address("fd7a:115c:a1e0::1").unwrap(),
        "[fd7a:115c:a1e0::1]:4747"
    );
    for refused in ["", "feeder.local", "192.168.1.20:", "192.168.1.20:99999"] {
        assert_eq!(
            feeder_address(refused).unwrap_err().kind,
            ErrorKind::Address,
            "{refused}"
        );
    }
}

/// A session on the local set, opened and caught up with the history.
async fn opened(
    feeder: [u8; 32],
    address: std::net::SocketAddr,
) -> (
    mpsc::UnboundedSender<Command>,
    mpsc::UnboundedReceiver<SessionUpdate>,
) {
    let (commands, mut heard) = start_session(feeder, address, false);
    next(&mut heard, |update| match update {
        SessionUpdate::History { caught_up, .. } => caught_up.then_some(()),
        _ => None,
    })
    .await;
    (commands, heard)
}

/// A time setting moves no event, so only a screen's read can tell the card of
/// it: through the session, the card must take what that read answered.
#[test]
fn a_screens_read_of_the_time_reaches_the_card() {
    local(async {
        let owner = *Keypair::from_secret(OWNER).public();
        let sim = Running::start(&Flash::memory(), Some(owner)).await;
        let (commands, mut heard) = opened(sim.identity, sim.address).await;

        let FeederReply::Time { time } =
            requests::run(&commands, sim.identity, FeederRequest::GetTime).await
        else {
            panic!("no time");
        };
        let reply = requests::run(
            &commands,
            sim.identity,
            FeederRequest::PutTime {
                base: time.generation,
                zone: None,
                ntp: Some("off".into()),
            },
        )
        .await;
        let FeederReply::Done {
            generation: Some(moved),
        } = reply
        else {
            panic!("the change did not land: {reply:?}");
        };
        assert_ne!(moved, time.generation);

        let FeederReply::Time { time: read } =
            requests::run(&commands, sim.identity, FeederRequest::GetTime).await
        else {
            panic!("no time");
        };
        assert_eq!(read.generation, moved);

        next(&mut heard, |update| match update {
            SessionUpdate::Status { card } => card
                .time
                .filter(|time| time.generation == moved)
                .map(|_| ()),
            _ => None,
        })
        .await;

        commands.send(Command::Close).unwrap();
        sim.stop().await;
    });
}

#[test]
fn requests_read_and_change_the_feeder_through_the_session() {
    local(async {
        let owner = *Keypair::from_secret(OWNER).public();
        let sim = Running::start(&Flash::memory(), Some(owner)).await;
        let (commands, mut heard) = opened(sim.identity, sim.address).await;
        let ask = |request| requests::run(&commands, sim.identity, request);

        let FeederReply::Settings { settings } = ask(FeederRequest::GetSettings).await else {
            panic!("no settings");
        };
        let changes = SettingsChanges {
            volume: Some(30),
            ..SettingsChanges::default()
        };
        let reply = ask(FeederRequest::PutSettings {
            base: settings.generation,
            changes,
        })
        .await;
        assert!(matches!(reply, FeederReply::Done { .. }), "{reply:?}");
        let reply = ask(FeederRequest::PutSettings {
            base: settings.generation,
            changes,
        })
        .await;
        assert!(
            matches!(
                reply,
                FeederReply::Refused {
                    refusal: FeederRefusal::Stale { .. }
                }
            ),
            "{reply:?}"
        );

        // What the feeder would refuse anyway goes no further than Rust.
        let reply = ask(FeederRequest::PutWifi {
            base: 0,
            ssid: "Home".into(),
            passphrase: "short".into(),
        })
        .await;
        assert_eq!(
            reply,
            FeederReply::NotAsked {
                reason: NotAsked::Credentials
            }
        );

        let FeederReply::Invite { invite } = ask(FeederRequest::CreateInvite {
            role: ClientRole::Member,
        })
        .await
        else {
            panic!("no invite");
        };
        assert_eq!(invite.role, ClientRole::Member);
        assert_eq!(
            wellfed_client::api::Invite::parse(&invite.text)
                .unwrap()
                .feeder,
            sim.identity
        );

        // The invite's reader names the feeder, and the invite works once.
        assert_eq!(invite_feeder(&invite.text), Some(sim.identity.to_vec()));
        assert_eq!(invite_feeder("not an invite"), None);
        let joining = ClientKey::generate().unwrap();
        let answer = join_feeder(
            &joining,
            invite.text.clone(),
            "Ana".into(),
            sim.address.to_string(),
        )
        .await
        .unwrap();
        assert_eq!(
            answer,
            JoinAnswer::Joined {
                role: ClientRole::Member
            }
        );
        let again = join_feeder(&joining, invite.text, "Ana".into(), sim.address.to_string()).await;
        assert!(again.is_err(), "{again:?}");

        let FeederReply::Clients { clients } = ask(FeederRequest::ListClients).await else {
            panic!("no clients");
        };
        assert!(clients.iter().any(|client| client.you));

        let FeederReply::Time { time } = ask(FeederRequest::GetTime).await else {
            panic!("no time");
        };
        let reply = ask(FeederRequest::PutTime {
            base: time.generation,
            zone: None,
            ntp: Some("pool.ntp.org".into()),
        })
        .await;
        assert_eq!(
            reply,
            FeederReply::NotAsked {
                reason: NotAsked::BadNtp
            }
        );

        let FeederReply::Calibration { calibration } = ask(FeederRequest::GetCalibration).await
        else {
            panic!("no calibration");
        };
        let reply = ask(FeederRequest::PutCalibration {
            base: calibration.generation,
            portion: PortionChange::Weighed {
                grams: "98.4".into(),
                portions: 10,
            },
            bowl: calibration.bowl,
        })
        .await;
        assert!(matches!(reply, FeederReply::Done { .. }), "{reply:?}");
        let FeederReply::Calibration { calibration } = ask(FeederRequest::GetCalibration).await
        else {
            panic!("no calibration");
        };
        assert_eq!(calibration.portion_mg, Some(9_840));

        let reply = ask(FeederRequest::Stats {
            section: StatsSection::Counters,
        })
        .await;
        assert!(matches!(reply, FeederReply::Stats { .. }), "{reply:?}");

        let reply = ask(FeederRequest::TailLog { after: None }).await;
        assert!(matches!(reply, FeederReply::Done { .. }), "{reply:?}");
        next(&mut heard, |update| match update {
            SessionUpdate::Log { lines, .. } => (!lines.is_empty()).then_some(()),
            _ => None,
        })
        .await;

        let reply = ask(FeederRequest::SetLock { locked: false }).await;
        assert!(matches!(reply, FeederReply::Done { .. }), "{reply:?}");
        let card = next(&mut heard, |update| match update {
            SessionUpdate::Status { card } if card.locked == Some(false) => Some(card),
            _ => None,
        })
        .await;
        assert!(!card.build.is_empty());

        commands.send(Command::Close).unwrap();
        sim.stop().await;
    });
}

#[test]
fn the_plans_change_through_their_edit_and_show_on_the_card() {
    local(async {
        let owner = *Keypair::from_secret(OWNER).public();
        let sim = Running::start(&Flash::memory(), Some(owner)).await;
        let (commands, mut heard) = opened(sim.identity, sim.address).await;
        let ask = |request| requests::run(&commands, sim.identity, request);
        // The card says no meal without a clock.
        let reply = ask(FeederRequest::SyncClock).await;
        assert!(matches!(reply, FeederReply::Done { .. }), "{reply:?}");

        let FeederReply::Plans { plans } = ask(FeederRequest::GetPlans).await else {
            panic!("no plans");
        };
        let plan = PlanView {
            minute: 7 * 60 + 30,
            portions: 2,
            days: vec![wellfed_model::Weekday::Mon, wellfed_model::Weekday::Thu],
            enabled: true,
            call: 0,
        };
        let reply = ask(FeederRequest::EditPlans {
            plans: plans.clone(),
            edit: PlanEdit::Add { plan: plan.clone() },
        })
        .await;
        assert!(matches!(reply, FeederReply::Done { .. }), "{reply:?}");

        let FeederReply::Plans { plans: stored } = ask(FeederRequest::GetPlans).await else {
            panic!("no plans");
        };
        assert_eq!(stored.plans[0], Some(plan));

        // Its `Plans` event moves the card without being asked.
        let next_meal = next(&mut heard, |update| match update {
            SessionUpdate::Status { card } => card.next_meal,
            _ => None,
        })
        .await;
        assert_eq!(next_meal.minute, 7 * 60 + 30);
        assert_eq!(next_meal.portions, 2);

        // An edit of the table as it was is refused rather than merged.
        let reply = ask(FeederRequest::EditPlans {
            plans,
            edit: PlanEdit::Add {
                plan: PlanView {
                    minute: 9 * 60,
                    portions: 1,
                    days: vec![],
                    enabled: true,
                    call: 0,
                },
            },
        })
        .await;
        assert!(
            matches!(
                reply,
                FeederReply::Refused {
                    refusal: FeederRefusal::Stale { .. }
                }
            ),
            "{reply:?}"
        );

        let reply = ask(FeederRequest::EditPlans {
            plans: stored,
            edit: PlanEdit::Delete { slot: 5 },
        })
        .await;
        assert_eq!(
            reply,
            FeederReply::NotAsked {
                reason: NotAsked::EmptySlot
            }
        );

        commands.send(Command::Close).unwrap();
        sim.stop().await;
    });
}

#[test]
fn the_card_and_the_history_show_a_feed_read_again_by_a_new_session() {
    local(async {
        let owner = *Keypair::from_secret(OWNER).public();
        let sim = Running::start(&Flash::memory(), Some(owner)).await;
        let (commands, mut heard) = opened(sim.identity, sim.address).await;
        commands
            .send(Command::Feed {
                portions: 1,
                reply: None,
            })
            .unwrap();
        let ended = next(&mut heard, |update| match update {
            SessionUpdate::FeedEnded { result } => Some(result),
            _ => None,
        })
        .await;
        assert_eq!(
            ended.by,
            FedBy::Client {
                fingerprint: ended_by(owner)
            }
        );
        commands.send(Command::Close).unwrap();

        // The next session has it from the history, as no news: no FeedEnded.
        let (commands, mut heard) = start_session(sim.identity, sim.address, false);
        let mut ends = 0;
        let entries = next(&mut heard, |update| match update {
            SessionUpdate::History { entries, caught_up } => caught_up.then_some(entries),
            SessionUpdate::FeedEnded { .. } => {
                ends += 1;
                None
            }
            _ => None,
        })
        .await;
        assert_eq!(ends, 0);
        assert!(entries.iter().any(
            |entry| matches!(&entry.body, HistoryBody::Fed { result } if result.feed == ended.feed)
        ));
        let card = next(&mut heard, |update| match update {
            SessionUpdate::Status { card } => Some(card),
            _ => None,
        })
        .await;
        assert_eq!(card.last_fed.map(|fed| fed.feed), Some(ended.feed));

        commands.send(Command::Close).unwrap();
        sim.stop().await;
    });
}

/// A client's name in events: its key's first 4 bytes.
fn ended_by(key: [u8; 32]) -> String {
    wellfed_session::hex::Lower(&key[..4]).to_string()
}

#[test]
fn a_write_whose_answer_never_comes_says_so_and_a_request_after_the_end_went_nowhere() {
    local(async {
        let owner = *Keypair::from_secret(OWNER).public();
        let sim = Running::start(&Flash::memory(), Some(owner)).await;
        let relay = Relay::start(sim.address).await;
        let (commands, mut heard) = opened(sim.identity, relay.address).await;

        relay.hold_answers();
        let lock = requests::run(
            &commands,
            sim.identity,
            FeederRequest::SetLock { locked: false },
        );
        let cut = async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            relay.cut();
        };
        let (reply, ()) = tokio::join!(lock, cut);
        assert_eq!(reply, FeederReply::Unanswered);

        next(&mut heard, |update| match update {
            SessionUpdate::Closed { error } => error,
            _ => None,
        })
        .await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        let reply = requests::run(&commands, sim.identity, FeederRequest::Reboot).await;
        assert_eq!(reply, FeederReply::NotSent);
        sim.stop().await;
    });
}

/// 8 kHz, 16-bit mono PCM of `samples` samples.
fn wav(samples: usize) -> Vec<u8> {
    let data = samples * 2;
    let mut file = Vec::new();
    file.extend(b"RIFF");
    file.extend(u32::try_from(36 + data).unwrap().to_le_bytes());
    file.extend(b"WAVEfmt ");
    file.extend(16u32.to_le_bytes());
    file.extend(1u16.to_le_bytes());
    file.extend(1u16.to_le_bytes());
    file.extend(8_000u32.to_le_bytes());
    file.extend(16_000u32.to_le_bytes());
    file.extend(2u16.to_le_bytes());
    file.extend(16u16.to_le_bytes());
    file.extend(b"data");
    file.extend(u32::try_from(data).unwrap().to_le_bytes());
    for sample in 0..samples {
        let tone = if sample % 16 < 8 { 8_000i16 } else { -8_000 };
        file.extend(tone.to_le_bytes());
    }
    file
}

#[test]
fn a_recording_uploads_as_the_meal_call_and_a_trace_refused_says_so() {
    local(async {
        let owner = *Keypair::from_secret(OWNER).public();
        let sim = Running::start(&Flash::memory(), Some(owner)).await;
        let (commands, mut heard) = opened(sim.identity, sim.address).await;
        let ask = |request| requests::run(&commands, sim.identity, request);

        let reply = ask(FeederRequest::UploadCall { wav: wav(4_000) }).await;
        let FeederReply::Call { call } = reply else {
            panic!("no call: {reply:?}");
        };
        assert!(matches!(call.clip, CallClip::Stored { .. }), "{call:?}");
        assert!(call.ms.is_some());

        let reply = ask(FeederRequest::UploadCall {
            wav: b"not a recording".to_vec(),
        })
        .await;
        assert_eq!(
            reply,
            FeederReply::NotAsked {
                reason: NotAsked::BadRecording
            }
        );

        let reply = ask(FeederRequest::DeleteCall).await;
        assert!(matches!(reply, FeederReply::Done { .. }), "{reply:?}");

        commands
            .send(Command::Feed {
                portions: 1,
                reply: None,
            })
            .unwrap();
        let feed = next(&mut heard, |update| match update {
            SessionUpdate::FeedEnded { result } => Some(result.feed),
            _ => None,
        })
        .await;
        // The simulated feeder keeps no traces, so this is a refusal, by name.
        let reply = ask(FeederRequest::ReadTrace { feed }).await;
        assert_eq!(
            reply,
            FeederReply::Refused {
                refusal: FeederRefusal::NotFound
            }
        );

        commands.send(Command::Close).unwrap();
        sim.stop().await;
    });
}

/// The newest card already sent, without waiting for another.
fn card_so_far(heard: &mut mpsc::UnboundedReceiver<SessionUpdate>) -> Option<Card> {
    let mut card = None;
    while let Ok(update) = heard.try_recv() {
        if let SessionUpdate::Status { card: sent } = update {
            card = Some(sent);
        }
    }
    card
}

/// A settings, lock or plans write moves the card on its `Done`, by the time
/// its caller hears it: no event or read again comes between.
#[test]
fn a_done_moves_the_card_by_the_time_its_caller_hears_it() {
    local(async {
        let owner = *Keypair::from_secret(OWNER).public();
        let sim = Running::start(&Flash::memory(), Some(owner)).await;
        let (commands, mut heard) = opened(sim.identity, sim.address).await;
        let ask = |request| requests::run(&commands, sim.identity, request);
        let FeederReply::Settings { settings } = ask(FeederRequest::GetSettings).await else {
            panic!("no settings");
        };
        let FeederReply::Plans { plans } = ask(FeederRequest::GetPlans).await else {
            panic!("no plans");
        };

        let reply = ask(FeederRequest::EditPlans {
            plans,
            edit: PlanEdit::Add {
                plan: PlanView {
                    minute: 8 * 60,
                    portions: 1,
                    days: vec![],
                    enabled: true,
                    call: 0,
                },
            },
        })
        .await;
        assert!(matches!(reply, FeederReply::Done { .. }), "{reply:?}");
        let held = card_so_far(&mut heard)
            .and_then(|card| card.plans)
            .expect("the added meal is on the card");
        assert_eq!(held.plans[0].as_ref().map(|plan| plan.minute), Some(8 * 60));

        let volume = (settings.volume + 17) % 100;
        let reply = ask(FeederRequest::PutSettings {
            base: settings.generation,
            changes: SettingsChanges {
                volume: Some(volume),
                ..SettingsChanges::default()
            },
        })
        .await;
        let FeederReply::Done {
            generation: Some(moved),
        } = reply
        else {
            panic!("the change did not land: {reply:?}");
        };
        let shown = card_so_far(&mut heard)
            .and_then(|card| card.settings)
            .expect("the change is on the card");
        assert_eq!((shown.volume, shown.generation), (volume, moved));

        let reply = ask(FeederRequest::SetLock { locked: true }).await;
        assert!(matches!(reply, FeederReply::Done { .. }), "{reply:?}");
        let card = card_so_far(&mut heard).expect("the lock is on the card");
        assert_eq!(card.locked, Some(true));

        let reply = ask(FeederRequest::EditPlans {
            plans: held,
            edit: PlanEdit::Enabled {
                slot: 0,
                enabled: false,
            },
        })
        .await;
        assert!(matches!(reply, FeederReply::Done { .. }), "{reply:?}");
        let off = card_so_far(&mut heard)
            .and_then(|card| card.plans)
            .expect("the meal's switch is on the card");
        assert_eq!(off.plans[0].as_ref().map(|plan| plan.enabled), Some(false));

        commands.send(Command::Close).unwrap();
        sim.stop().await;
    });
}

/// Two quick changes on the generation the screen showed: the second waits
/// for the first's answer and goes on its generation, so neither is refused.
#[test]
fn a_quick_second_settings_change_goes_on_the_first_s_generation() {
    local(async {
        let owner = *Keypair::from_secret(OWNER).public();
        let sim = Running::start(&Flash::memory(), Some(owner)).await;
        let (commands, mut heard) = opened(sim.identity, sim.address).await;
        let ask = |request| requests::run(&commands, sim.identity, request);
        let FeederReply::Settings { settings } = ask(FeederRequest::GetSettings).await else {
            panic!("no settings");
        };
        let put = |changes| FeederRequest::PutSettings {
            base: settings.generation,
            changes,
        };

        let (first, second) = tokio::join!(
            ask(put(SettingsChanges {
                volume: Some(20),
                ..SettingsChanges::default()
            })),
            ask(put(SettingsChanges {
                lights: Some(!settings.lights),
                ..SettingsChanges::default()
            })),
        );
        let (
            FeederReply::Done {
                generation: Some(first),
            },
            FeederReply::Done {
                generation: Some(second),
            },
        ) = (&first, &second)
        else {
            panic!("both land: {first:?} {second:?}");
        };
        assert_ne!(first, second);

        let FeederReply::Settings { settings: read } = ask(FeederRequest::GetSettings).await else {
            panic!("no settings");
        };
        assert_eq!(read.generation, *second);
        assert_eq!((read.volume, read.lights), (20, !settings.lights));
        let shown = card_so_far(&mut heard)
            .and_then(|card| card.settings)
            .expect("the card shows both");
        assert_eq!((shown.volume, shown.lights), (20, !settings.lights));

        commands.send(Command::Close).unwrap();
        sim.stop().await;
    });
}

/// A change waiting behind one the feeder refused never goes: the generation
/// it would go on is not known.
#[test]
fn a_settings_change_waiting_behind_a_refused_one_is_not_sent() {
    local(async {
        let owner = *Keypair::from_secret(OWNER).public();
        let sim = Running::start(&Flash::memory(), Some(owner)).await;
        let (commands, _heard) = opened(sim.identity, sim.address).await;
        let ask = |request| requests::run(&commands, sim.identity, request);
        let FeederReply::Settings { settings } = ask(FeederRequest::GetSettings).await else {
            panic!("no settings");
        };
        let volume = |volume| SettingsChanges {
            volume: Some(volume),
            ..SettingsChanges::default()
        };

        let (refused, held) = tokio::join!(
            ask(FeederRequest::PutSettings {
                base: settings.generation.wrapping_add(100),
                changes: volume(20),
            }),
            ask(FeederRequest::PutSettings {
                base: settings.generation,
                changes: volume(40),
            }),
        );
        assert!(
            matches!(
                refused,
                FeederReply::Refused {
                    refusal: FeederRefusal::Stale { .. }
                }
            ),
            "{refused:?}"
        );
        assert_eq!(
            held,
            FeederReply::NotAsked {
                reason: NotAsked::Held
            }
        );

        let FeederReply::Settings { settings: read } = ask(FeederRequest::GetSettings).await else {
            panic!("no settings");
        };
        assert_eq!(read, settings, "nothing was written");

        commands.send(Command::Close).unwrap();
        sim.stop().await;
    });
}

/// The listener goes away at the very card the change moves: the change was
/// answered, so its caller hears `Done`, never "not sent", which a screen may
/// send again.
#[test]
fn a_write_answered_as_the_listener_leaves_is_heard_as_answered() {
    const QUIET_VOLUME: u8 = 33;
    local(async {
        let owner = *Keypair::from_secret(OWNER).public();
        let sim = Running::start(&Flash::memory(), Some(owner)).await;
        let (commands, receiver) = mpsc::unbounded_channel();
        let (updates, mut heard) = mpsc::unbounded_channel();
        tokio::task::spawn_local(session::run(
            Keypair::from_secret(OWNER),
            sim.identity,
            sim.address,
            false,
            receiver,
            move |update| {
                let leaving = matches!(
                    &update,
                    SessionUpdate::Status { card }
                        if card.settings.as_ref().is_some_and(|settings| settings.volume == QUIET_VOLUME)
                );
                updates.send(update).is_ok() && !leaving
            },
        ));
        next(&mut heard, |update| match update {
            SessionUpdate::History { caught_up, .. } => caught_up.then_some(()),
            _ => None,
        })
        .await;
        let ask = |request| requests::run(&commands, sim.identity, request);
        let FeederReply::Settings { settings } = ask(FeederRequest::GetSettings).await else {
            panic!("no settings");
        };
        assert_ne!(settings.volume, QUIET_VOLUME);

        let reply = ask(FeederRequest::PutSettings {
            base: settings.generation,
            changes: SettingsChanges {
                volume: Some(QUIET_VOLUME),
                ..SettingsChanges::default()
            },
        })
        .await;
        assert!(matches!(reply, FeederReply::Done { .. }), "{reply:?}");

        next(&mut heard, |update| match update {
            SessionUpdate::Closed { .. } => Some(()),
            _ => None,
        })
        .await;
        sim.stop().await;
    });
}

/// A desiccant reset moves the settings generation, so a change asked while
/// one is out waits for it and goes on the generation it made.
#[test]
fn a_settings_change_behind_a_desiccant_reset_goes_on_the_reset_s_generation() {
    local(async {
        let owner = *Keypair::from_secret(OWNER).public();
        let sim = Running::start(&Flash::memory(), Some(owner)).await;
        let (commands, _heard) = opened(sim.identity, sim.address).await;
        let ask = |request| requests::run(&commands, sim.identity, request);
        // A reset is dated from the feeder's clock.
        let reply = ask(FeederRequest::SyncClock).await;
        assert!(matches!(reply, FeederReply::Done { .. }), "{reply:?}");
        let FeederReply::Settings { settings } = ask(FeederRequest::GetSettings).await else {
            panic!("no settings");
        };

        let (reset, change) = tokio::join!(
            ask(FeederRequest::ResetDesiccant),
            ask(FeederRequest::PutSettings {
                base: settings.generation,
                changes: SettingsChanges {
                    desiccant_days: Some(60),
                    ..SettingsChanges::default()
                },
            }),
        );
        assert!(matches!(reset, FeederReply::Done { .. }), "{reset:?}");
        assert!(matches!(change, FeederReply::Done { .. }), "{change:?}");

        let FeederReply::Settings { settings: read } = ask(FeederRequest::GetSettings).await else {
            panic!("no settings");
        };
        assert_eq!(read.desiccant_days, 60);
        assert!(read.desiccant_replaced.is_some());
        assert_eq!(read.generation, settings.generation + 2);

        commands.send(Command::Close).unwrap();
        sim.stop().await;
    });
}

/// A reset's `Done` moves the card's settings generation, so a change tapped
/// before its event's read comes goes on the generation the reset made.
#[test]
fn a_change_tapped_after_a_reset_goes_on_its_generation() {
    local(async {
        let owner = *Keypair::from_secret(OWNER).public();
        let sim = Running::start(&Flash::memory(), Some(owner)).await;
        let (commands, mut heard) = opened(sim.identity, sim.address).await;
        let ask = |request| requests::run(&commands, sim.identity, request);
        let reply = ask(FeederRequest::SyncClock).await;
        assert!(matches!(reply, FeederReply::Done { .. }), "{reply:?}");
        let FeederReply::Settings { settings } = ask(FeederRequest::GetSettings).await else {
            panic!("no settings");
        };

        let reply = ask(FeederRequest::ResetDesiccant).await;
        assert!(matches!(reply, FeederReply::Done { .. }), "{reply:?}");
        let shown = card_so_far(&mut heard)
            .and_then(|card| card.settings)
            .expect("the reset's generation is on the card");
        assert_eq!(shown.generation, settings.generation + 1);

        let reply = ask(FeederRequest::PutSettings {
            base: shown.generation,
            changes: SettingsChanges {
                volume: Some(12),
                ..SettingsChanges::default()
            },
        })
        .await;
        assert!(matches!(reply, FeederReply::Done { .. }), "{reply:?}");

        commands.send(Command::Close).unwrap();
        sim.stop().await;
    });
}

/// Two meals switched within one round trip, each from the table the screen
/// showed: the second waits and goes as its switch on the table the first
/// made, so both land.
#[test]
fn two_meals_switched_at_once_both_land() {
    local(async {
        let owner = *Keypair::from_secret(OWNER).public();
        let sim = Running::start(&Flash::memory(), Some(owner)).await;
        let (commands, _heard) = opened(sim.identity, sim.address).await;
        let ask = |request| requests::run(&commands, sim.identity, request);
        let meal = |minute| PlanView {
            minute,
            portions: 1,
            days: vec![],
            enabled: true,
            call: 0,
        };
        for minute in [7 * 60, 18 * 60] {
            let FeederReply::Plans { plans } = ask(FeederRequest::GetPlans).await else {
                panic!("no plans");
            };
            let reply = ask(FeederRequest::EditPlans {
                plans,
                edit: PlanEdit::Add { plan: meal(minute) },
            })
            .await;
            assert!(matches!(reply, FeederReply::Done { .. }), "{reply:?}");
        }
        let FeederReply::Plans { plans } = ask(FeederRequest::GetPlans).await else {
            panic!("no plans");
        };
        let off = |slot| FeederRequest::EditPlans {
            plans: plans.clone(),
            edit: PlanEdit::Enabled {
                slot,
                enabled: false,
            },
        };

        let (first, second) = tokio::join!(ask(off(0)), ask(off(1)));
        assert!(matches!(first, FeederReply::Done { .. }), "{first:?}");
        assert!(matches!(second, FeederReply::Done { .. }), "{second:?}");

        let FeederReply::Plans { plans: read } = ask(FeederRequest::GetPlans).await else {
            panic!("no plans");
        };
        assert_eq!(read.generation, plans.generation + 2);
        for slot in [0, 1] {
            assert_eq!(
                read.plans[slot].as_ref().map(|plan| plan.enabled),
                Some(false)
            );
        }

        commands.send(Command::Close).unwrap();
        sim.stop().await;
    });
}
