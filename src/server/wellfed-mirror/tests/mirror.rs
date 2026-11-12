//! The mirror on its own, against the simulated feeder: updates follow the
//! store, a command that waits past its turn is never sent, a late answer
//! still reaches its caller, and a stopped feeder is let go (offline last)
//! once its task has ended, two starts of one feeder take turns, and a
//! zone change during a clock sync is synced too.

mod support;

use std::time::Duration;

use jiff::Timestamp;
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use wellfed_api::{Connection as Link, EventBody, EventKind, FeederEvent, HistoryEvent, Source};
use wellfed_mirror::{AskError, Feeder, MirrorEvent, Zone};
use wellfed_model::{Answer, Op, Skew};
use wellfed_server_testkit::feeder::{self, Relay};
use wellfed_server_testkit::fixed::FixedClock;
use wellfed_server_testkit::{TempStore, WAIT, local, until};
use wellfed_store::Store;

use support::{ID, SECRET, mirrored, mirrored_timed, next};

/// The next new event the mirror broadcasts.
async fn next_event(updates: &mut Receiver<MirrorEvent>) -> HistoryEvent {
    loop {
        if let MirrorEvent::Event(event) = next(updates).await {
            return event;
        }
    }
}

async fn locks(store: &Store, epoch: u32) -> Vec<bool> {
    let rows = store
        .events(ID, epoch, None, 100, Some(EventKind::Lock))
        .await
        .unwrap();
    rows.iter()
        .filter_map(|row| match wellfed_mirror::stored_event(epoch, row)?.body {
            EventBody::Feeder(FeederEvent::Lock { locked, .. }) => Some(locked),
            _ => None,
        })
        .collect()
}

#[test]
fn an_answered_command_and_its_event_are_stored_before_they_are_broadcast() {
    local(async {
        let temp = TempStore::new();
        let store = temp.open().await;
        let sim = feeder::start().await;
        let address = sim.address.to_string();
        let (mirror, mut updates) = mirrored(&store, &sim, &address, WAIT).await;

        let lock = Op::SetLock { locked: false };
        let answer = mirror.ask(ID, [1; 16], lock.clone()).await.unwrap();
        assert_eq!(answer, Answer::Done { generation: None });
        let event = loop {
            let event = next_event(&mut updates).await;
            if matches!(event.body, EventBody::Feeder(FeederEvent::Lock { .. })) {
                break event;
            }
        };
        assert_eq!(locks(&store, event.epoch).await, [false], "stored first");
        let projected = || store.lock_changes(ID, Timestamp::MIN, Timestamp::MAX);
        let changes = projected().await.unwrap();
        assert_eq!(changes.len(), 1, "projected with it");
        assert_eq!((changes[0].epoch, changes[0].seq), (event.epoch, event.seq));
        assert!(!changes[0].value.locked);
        assert!(matches!(changes[0].value.by, Source::Client { .. }));

        // The same request id is answered from the feeder's memory.
        let again = mirror.ask(ID, [1; 16], lock).await.unwrap();
        assert_eq!(again, answer);
        let status = mirror.ask(ID, [2; 16], Op::Status).await.unwrap();
        assert!(matches!(status, Answer::Status(_)));
        assert_eq!(locks(&store, event.epoch).await, [false], "once");
        assert_eq!(projected().await.unwrap(), changes, "projected once");

        mirror.shutdown().await;
        store.close().await;
        sim.stop().await;
    });
}

/// The mirror's clocks for a stalled feeder: a command waits `queue` for
/// its turn, and once sent the session waits `answer` for the feeder.
fn stalling(queue: Duration, answer: Duration) -> wellfed_mirror::Timing {
    wellfed_mirror::Timing {
        session: wellfed_connect::Timing {
            answer,
            ..wellfed_connect::Timing::DEFAULT
        },
        ..support::timing(queue)
    }
}

/// The feeder stalls with one write on the wire and a second queued
/// behind it. The first caller hears `Unknown` once the session's window
/// passes; the second waited past its turn by then, so it is turned away
/// `Expired`, a write the mirror knows it never sent, and never applied.
#[test]
fn a_command_that_waits_past_its_turn_is_never_sent() {
    local(async {
        let temp = TempStore::new();
        let store = temp.open().await;
        let sim = feeder::start().await;
        let relay = Relay::start(sim.address).await;
        let address = relay.address.to_string();
        let timing = stalling(Duration::from_millis(300), Duration::from_millis(600));
        let (mirror, mut updates) = mirrored_timed(&store, &sim, &address, timing).await;

        relay.pause();
        let unlock = mirror.ask(ID, [1; 16], Op::SetLock { locked: false });
        let lock = mirror.ask(ID, [2; 16], Op::SetLock { locked: true });
        let (unlocked, locked) = tokio::join!(unlock, lock);
        assert_eq!(unlocked, Err(AskError::Unknown));
        assert_eq!(locked, Err(AskError::Expired));
        relay.resume();

        // The task runs its queue in order: once a later command is
        // answered, the turned-away write has had its turn.
        let event = loop {
            let event = next_event(&mut updates).await;
            if matches!(event.body, EventBody::Feeder(FeederEvent::Lock { .. })) {
                break event;
            }
        };
        until("the session serves again", || async {
            mirror.ask(ID, [3; 16], Op::Status).await.is_ok()
        })
        .await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            locks(&store, event.epoch).await,
            [false],
            "the first write landed once; the turned-away one never did"
        );

        mirror.shutdown().await;
        store.close().await;
        sim.stop().await;
    });
}

/// A write goes out late, after queueing behind a stalled one, and its
/// answer comes after a caller's turn would have run out but inside the
/// session's window. The caller hears it: an answer the server took in is
/// never thrown away as unknown.
#[test]
fn a_late_answer_inside_the_sessions_window_reaches_its_caller() {
    local(async {
        let temp = TempStore::new();
        let store = temp.open().await;
        let sim = feeder::start().await;
        let relay = Relay::start(sim.address).await;
        let address = relay.address.to_string();
        let timing = stalling(Duration::from_secs(1), Duration::from_secs(4));
        let (mirror, _updates) = mirrored_timed(&store, &sim, &address, timing).await;

        relay.hold_answers();
        let unlock = mirror.ask(ID, [1; 16], Op::SetLock { locked: false });
        let lock = mirror.ask(ID, [2; 16], Op::SetLock { locked: true });
        let stall = async {
            tokio::time::sleep(Duration::from_millis(600)).await;
            // The first answer goes back, the second write goes out, and
            // its answer is held past the second's turn.
            relay.hold_answers_after(1);
            relay.resume();
            tokio::time::sleep(Duration::from_millis(1_400)).await;
            assert!(relay.holding_answers(), "the second answer is held");
            relay.resume();
        };
        let (unlocked, locked, ()) = tokio::join!(unlock, lock, stall);
        assert_eq!(unlocked, Ok(Answer::Done { generation: None }));
        assert_eq!(locked, Ok(Answer::Done { generation: None }));

        mirror.shutdown().await;
        store.close().await;
        sim.stop().await;
    });
}

#[test]
fn a_stopped_feeder_is_let_go() {
    local(async {
        let temp = TempStore::new();
        let store = temp.open().await;
        let sim = feeder::start().await;
        let address = sim.address.to_string();
        let (mirror, mut updates) = mirrored(&store, &sim, &address, WAIT).await;
        assert_eq!(mirror.link(ID), Some(Link::Online));
        assert_eq!(mirror.skew(ID), Some(Skew::Same));

        assert!(mirror.stop(ID).await);
        assert!(!mirror.stop(ID).await, "already stopped");
        assert_eq!(mirror.link(ID), None);
        assert_eq!(mirror.skew(ID), None);
        assert!(mirror.subscribe(ID).is_none());
        let asked = mirror.ask(ID, [1; 16], Op::Status).await;
        assert_eq!(asked, Err(AskError::NotRunning));

        // `stop` returned after the task ended: its updates are all sent,
        // the last link is offline, and the broadcast is closed.
        let mut last = None;
        loop {
            match updates.try_recv() {
                Ok(MirrorEvent::Link(link)) => last = Some(link),
                Ok(_) => {}
                Err(TryRecvError::Closed) => break,
                Err(other) => panic!("the broadcast is closed and whole: {other:?}"),
            }
        }
        assert_eq!(last, Some(Link::Offline));

        mirror.shutdown().await;
        store.close().await;
        sim.stop().await;
    });
}

/// Two starts of a running feeder race. They take turns: the later one
/// stops the earlier one's task, whose broadcast has closed before the
/// later task starts, so the feeder serves the mirror one session.
#[test]
fn two_starts_of_one_feeder_take_turns() {
    local(async {
        let temp = TempStore::new();
        let store = temp.open().await;
        let sim = feeder::start().await;
        let address = sim.address.to_string();
        let (mirror, _updates) = mirrored(&store, &sim, &address, WAIT).await;
        let feeder = || Feeder {
            id: ID,
            identity: sim.identity,
            secret: SECRET,
            address: Some(address.clone()),
        };

        let (mut earlier, mut later) = tokio::join!(mirror.start(feeder()), mirror.start(feeder()));
        let closed = tokio::time::timeout(WAIT, async {
            loop {
                match earlier.recv().await {
                    Err(RecvError::Closed) => break,
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                }
            }
        });
        assert!(closed.await.is_ok(), "the earlier start's task ended");
        while !matches!(next(&mut later).await, MirrorEvent::Link(Link::Online)) {}
        let status = mirror.ask(ID, [5; 16], Op::Status).await;
        let Ok(Answer::Status(status)) = status else {
            panic!("a status read: {status:?}");
        };
        assert_eq!(status.sessions, 1);

        mirror.shutdown().await;
        store.close().await;
        sim.stop().await;
    });
}

/// The household's zone changes while the feeder's answer to the clock
/// sync for the zone before is held back. The server's key may not set the
/// feeder's zone, so only clock syncs carry it: the change is synced on its
/// own at once, not left to the next periodic sync.
#[test]
fn a_zone_change_during_a_clock_sync_gets_a_sync_of_its_own() {
    local(async {
        let temp = TempStore::new();
        let store = temp.open().await;
        let sim = feeder::start().await;
        let relay = Relay::start(sim.address).await;
        let address = relay.address.to_string();
        let (mirror, _updates) = mirrored(&store, &sim, &address, WAIT).await;
        let tokyo = Zone::new("Asia/Tokyo", FixedClock.now()).unwrap();
        let kolkata = Zone::new("Asia/Kolkata", FixedClock.now()).unwrap();

        relay.hold_answers();
        let before = relay.passed();
        mirror.set_zone(tokyo);
        until("the clock sync reached the feeder", || async {
            relay.passed() > before
        })
        .await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        mirror.set_zone(kolkata);
        tokio::time::sleep(Duration::from_millis(100)).await;
        relay.resume();

        let reads = std::cell::Cell::new(0x6000_u128);
        until("the feeder keeps Kolkata's offset", || async {
            reads.set(reads.get() + 1);
            let time = mirror.ask(ID, reads.get().to_be_bytes(), Op::GetTime).await;
            matches!(time, Ok(Answer::Time(view)) if view.offset_s == Some(5 * 3600 + 1800))
        })
        .await;
        mirror.shutdown().await;
        store.close().await;
        sim.stop().await;
    });
}
