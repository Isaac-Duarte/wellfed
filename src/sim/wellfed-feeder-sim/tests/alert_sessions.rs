//! The alert rules with a client on the socket: a session that connects across
//! a restart, while the boot read runs, changes nothing about a meal.

use std::cell::RefCell;
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::rc::Rc;
use std::time::Duration;
use wellfed_connect::Samples;

use tokio::task::LocalSet;
use wellfed_connect::Connection;
use wellfed_feeder_sim::banks::Banks;
use wellfed_feeder_sim::flash::Flash;
use wellfed_feeder_sim::harness::Running;
use wellfed_model::{
    Answer, CoreEvent, EventBody, FeedOutcome, FeederEvent, Generation, Op, PLANS, Plan, Pushed,
    Source, TimeSync, Weekdays,
};
use wellfed_session::noise::Keypair;

const OWNER: [u8; 32] = [0x21; 32];
/// 2026-10-28T00:00:00Z.
const WEDNESDAY_MS: u64 = 1_793_145_600_000;

fn keys() -> Keypair {
    Keypair::from_secret(OWNER)
}

fn paused(test: impl Future<Output = ()>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap();
    runtime.block_on(LocalSet::new().run_until(test));
}

/// A meal as its `Fed` event tells it, leaving out when.
type Meal = (u8, u8, FeedOutcome, Source);

/// A session as soon as the feeder takes one, from the moment it restarts.
async fn connect(sim: &Running) -> Connection {
    let ephemeral = [OWNER[0] ^ 0x80; 32];
    for _ in 0..200 {
        if let Ok(connection) =
            Connection::open(&keys(), sim.identity, sim.address, ephemeral).await
        {
            return connection;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the feeder took no session");
}

async fn sync_clock(connection: &mut Connection, id: &mut u8, unix_ms: u64) {
    *id += 1;
    let sync = TimeSync {
        unix_ms,
        offset_s: 0,
        next_change_s: 0,
        next_offset_s: 0,
    };
    let answer = connection
        .ask([*id; 16], &Op::TimeSync(sync))
        .await
        .unwrap();
    assert_eq!(answer, Answer::Done { generation: None });
}

async fn history(connection: &mut Connection) -> Vec<EventBody> {
    connection.subscribe(None, Samples::Off).await.unwrap();
    let mut events = Vec::new();
    while let Ok(message) = connection.next(Duration::from_millis(500)).await {
        if let Pushed::Event(event) = message {
            events.push(event.body);
        }
    }
    events
}

/// Two meals either side of a client's reboot, with the client back on the
/// socket at once and following the history through the boot read.
async fn run(rules: bool) -> (Vec<Meal>, Vec<EventBody>) {
    let flash = Flash::memory();
    let banks: Rc<RefCell<Banks>> = Banks::new();
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
    let owner = Some(*keys().public());
    let sim = Running::start_ruled(&flash, &banks, None, owner, address, rules).await;
    let started = tokio::time::Instant::now();
    let wall = |started: tokio::time::Instant| {
        WEDNESDAY_MS + u64::try_from(started.elapsed().as_millis()).unwrap()
    };

    let mut id = 0;
    let mut connection = connect(&sim).await;
    sync_clock(&mut connection, &mut id, wall(started)).await;
    let mut plans: [Option<Plan>; PLANS] = [None; PLANS];
    for (slot, minute) in [(0, 2), (1, 6)] {
        plans[slot] = Some(Plan {
            minute,
            portions: 1,
            days: Weekdays::DAILY,
            enabled: true,
            call: 0,
        });
    }
    let put = Op::PutPlans {
        base: Generation(0),
        plans,
    };
    id += 1;
    let answer = connection.ask([id; 16], &put).await.unwrap();
    assert!(matches!(answer, Answer::Done { .. }), "{answer:?}");

    // The first meal, then a reboot well before the second.
    drop(connection);
    tokio::time::sleep(Duration::from_secs(4 * 60)).await;
    let mut connection = connect(&sim).await;
    id += 1;
    let answer = connection.ask([id; 16], &Op::Reboot).await.unwrap();
    assert_eq!(answer, Answer::Done { generation: None });
    assert!(connection.next(Duration::from_secs(5)).await.is_err());

    // Back at once, following the history while the feeder reads it too.
    let mut connection = connect(&sim).await;
    connection.subscribe(None, Samples::Off).await.unwrap();
    sync_clock(&mut connection, &mut id, wall(started)).await;
    tokio::time::sleep(Duration::from_secs(4 * 60)).await;
    drop(connection);

    let mut connection = connect(&sim).await;
    let events = history(&mut connection).await;
    sim.stop().await;

    let meals = events
        .iter()
        .filter_map(|body| match *body {
            EventBody::Feeder(FeederEvent::Fed {
                requested,
                completed,
                outcome,
                by,
                ..
            }) => Some((requested, completed, outcome, by)),
            _ => None,
        })
        .collect();
    (meals, events)
}

#[test]
fn a_client_across_a_restart_changes_no_meal() {
    paused(async {
        let (with_rules, events) = run(true).await;
        let (without, _) = run(false).await;

        assert_eq!(with_rules, without);
        assert_eq!(
            with_rules,
            [
                (1, 1, FeedOutcome::Done, Source::Schedule { plan: 0 }),
                (1, 1, FeedOutcome::Done, Source::Schedule { plan: 1 }),
            ],
            "{events:?}"
        );

        // The reboot was on the record, ahead of the boot it caused.
        let boots: Vec<usize> = events
            .iter()
            .enumerate()
            .filter(|(_, body)| matches!(body, EventBody::Core(CoreEvent::Boot { .. })))
            .map(|(at, _)| at)
            .collect();
        assert_eq!(boots.len(), 2, "{events:?}");
        assert!(matches!(
            events[boots[1] - 1],
            EventBody::Core(CoreEvent::Restarting)
        ));
    });
}
