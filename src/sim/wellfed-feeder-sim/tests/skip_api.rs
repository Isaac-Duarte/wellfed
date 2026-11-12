//! Skipping a meal through the device API, end to end: a client over TCP,
//! the simulated feeder's loop, its plans store and its event ring. The
//! answer waits for the store, the plans name the skip, the history names
//! who asked, and an undo whose write lands only once the meal is due is
//! still answered `Done`, and the meal feeds once. Simulation, on tokio's
//! paused clock.

mod common;

use std::time::Duration;
use wellfed_connect::Samples;

use common::paused;
use jiff::Timestamp;
use wellfed_connect::{Connection, Received};
use wellfed_feeder_sim::flash::Flash;
use wellfed_feeder_sim::harness::Running;
use wellfed_feeder_sim::serve::Control;
use wellfed_model::{
    Answer, EventBody, FeederEvent, Generation, Op, PLANS, Plan, Plans, Pushed, Refusal, Source,
    TimeSync, Weekdays,
};
use wellfed_proto::api::RequestId;
use wellfed_session::noise::Keypair;

const OWNER: [u8; 32] = [0x21; 32];
const WAIT: Duration = Duration::from_secs(5);
/// 2026-09-21T14:13:20Z, inside the clock's accepted range.
const NOW_MS: u64 = 1_790_000_000_000;
/// The plan's minute: 15:00, at UTC.
const MINUTE: u16 = 15 * 60;
const DONE: Answer = Answer::Done { generation: None };

fn keys() -> Keypair {
    Keypair::from_secret(OWNER)
}

/// Today's 15:00 at UTC, in Unix seconds.
fn due_s() -> i64 {
    let now_s = i64::try_from(NOW_MS / 1000).unwrap();
    now_s - now_s.rem_euclid(86_400) + i64::from(MINUTE) * 60
}

fn due() -> Timestamp {
    Timestamp::from_second(due_s()).unwrap()
}

/// The owner's session, and the next request id.
struct Owner {
    connection: Connection,
    ids: u8,
}

impl Owner {
    async fn open(sim: &Running) -> Self {
        let connection = Connection::open(&keys(), sim.identity, sim.address, [0xa1; 32])
            .await
            .unwrap();
        Self { connection, ids: 0 }
    }

    fn id(&mut self) -> RequestId {
        self.ids += 1;
        [self.ids; 16]
    }

    async fn ask(&mut self, op: &Op) -> Answer {
        let id = self.id();
        self.connection.ask(id, op).await.unwrap()
    }

    /// The feeder's wall clock, in Unix milliseconds.
    async fn clock(&mut self) -> u64 {
        let answer = self.ask(&Op::Status).await;
        let Answer::Status(status) = answer else {
            panic!("{answer:?}");
        };
        let clock = status.clock.expect("a synced clock");
        u64::try_from(clock.as_millisecond()).unwrap()
    }

    async fn plans(&mut self) -> Plans {
        let answer = self.ask(&Op::GetPlans).await;
        let Answer::Plans(plans) = answer else {
            panic!("{answer:?}");
        };
        plans
    }

    /// The skip and undo events, and the plan's feeds, in the order the
    /// feeder recorded them, read from a fresh subscription.
    async fn history(sim: &Running) -> Vec<EventBody> {
        let mut reader = Self::open(sim).await;
        reader
            .connection
            .subscribe(None, Samples::Off)
            .await
            .unwrap();
        let mut events = Vec::new();
        while let Ok(message) = reader.connection.next(Duration::from_millis(500)).await {
            if let Pushed::Event(event) = message
                && matches!(
                    event.body,
                    EventBody::Feeder(
                        FeederEvent::Skipped { .. }
                            | FeederEvent::Unskipped { .. }
                            | FeederEvent::Fed { .. }
                    )
                )
            {
                events.push(event.body);
            }
        }
        events
    }
}

/// A feeder synced to `NOW_MS` at UTC, with a daily plan at 15:00 in slot 0.
async fn breakfast(sim: &Running) -> Owner {
    let mut owner = Owner::open(sim).await;
    let sync = TimeSync {
        unix_ms: NOW_MS,
        offset_s: 0,
        next_change_s: 0,
        next_offset_s: 0,
    };
    assert_eq!(owner.ask(&Op::TimeSync(sync)).await, DONE);
    let mut table = [None; PLANS];
    table[0] = Some(Plan {
        minute: MINUTE,
        portions: 1,
        days: Weekdays::DAILY,
        enabled: true,
        call: 0,
    });
    let put = Op::PutPlans {
        base: Generation(0),
        plans: table,
    };
    let saved = owner.ask(&put).await;
    assert_eq!(
        saved,
        Answer::Done {
            generation: Some(Generation(1))
        }
    );
    owner
}

#[test]
fn a_skip_and_its_undo_are_answered_once_stored_and_named_in_the_plans_and_history() {
    paused(async {
        let sim = Running::start(&Flash::memory(), Some(*keys().public())).await;
        let mut owner = breakfast(&sim).await;
        let skip = Op::SkipMeal {
            slot: 0,
            due: due(),
        };
        let unskip = Op::UnskipMeal {
            slot: 0,
            due: due(),
        };

        assert_eq!(owner.ask(&skip).await, DONE);
        assert_eq!(owner.plans().await.skipped.get(0), Some(due()));
        assert_eq!(
            owner.ask(&skip).await,
            Answer::Refused(Refusal::Invalid),
            "skipped once"
        );
        assert_eq!(owner.ask(&unskip).await, DONE);
        assert_eq!(owner.plans().await.skipped.get(0), None);

        let history = Owner::history(&sim).await;
        let [
            EventBody::Feeder(FeederEvent::Skipped {
                plan: 0,
                due: skipped,
                by: Source::Client { .. },
            }),
            EventBody::Feeder(FeederEvent::Unskipped {
                plan: 0,
                due: undone,
                by: Source::Client { .. },
            }),
        ] = history[..]
        else {
            panic!("{history:?}");
        };
        assert_eq!((skipped, undone), (due(), due()));
        sim.stop().await;
    });
}

/// An undo sent seconds before the meal, whose write the writer only lands
/// once the meal is due: the schedule claims the meal as soon as the undo
/// lands, which marks it as the skip did, yet the undo is answered `Done`
/// and recorded, and the meal feeds once.
#[test]
fn an_undo_that_lands_as_its_meal_comes_due_is_done_and_the_meal_feeds_once() {
    paused(async {
        let sim = Running::start(&Flash::memory(), Some(*keys().public())).await;
        let mut owner = breakfast(&sim).await;
        let skip = Op::SkipMeal {
            slot: 0,
            due: due(),
        };
        assert_eq!(owner.ask(&skip).await, DONE);

        // Most of the way to the meal; a session idle that long has ended,
        // so open another, with ids the feeder has not answered yet (it
        // remembers a write's id).
        let due_ms = u64::try_from(due_s()).unwrap() * 1000;
        tokio::time::sleep(Duration::from_millis(due_ms - NOW_MS - 60_000)).await;
        let ids = owner.ids;
        let mut owner = Owner::open(&sim).await;
        owner.ids = ids;
        // Then to a second before it, by the feeder's own clock. The write
        // is held until a second after, inside the client's 10 s wait for
        // its answer.
        let clock = owner.clock().await;
        tokio::time::sleep(Duration::from_millis(due_ms - clock - 1_000)).await;
        sim.control(Control::HoldPlans(true));
        let id = owner.id();
        let unskip = Op::UnskipMeal {
            slot: 0,
            due: due(),
        };
        owner.connection.request(id, &unskip).unwrap();
        // A receive writes the queued request; nothing answers it yet.
        let waiting = owner.connection.receive(Duration::from_millis(100)).await;
        assert!(waiting.is_err(), "{waiting:?}");
        tokio::time::sleep(Duration::from_secs(2)).await;
        sim.control(Control::HoldPlans(false));

        let answer = loop {
            match owner.connection.receive(WAIT).await.unwrap() {
                Received::Answer { id: to, answer } if to == id => break answer,
                Received::Answer { .. } | Received::Pushed(_) => {}
            }
        };
        assert_eq!(answer, Some(DONE));
        tokio::time::sleep(Duration::from_secs(30)).await;

        let history = Owner::history(&sim).await;
        let kinds: Vec<_> = history
            .iter()
            .map(|body| match body {
                EventBody::Feeder(FeederEvent::Skipped { .. }) => "skipped",
                EventBody::Feeder(FeederEvent::Unskipped { .. }) => "unskipped",
                EventBody::Feeder(FeederEvent::Fed {
                    by: Source::Schedule { plan: 0 },
                    ..
                }) => "fed",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["skipped", "unskipped", "fed"]);
        sim.stop().await;
    });
}
