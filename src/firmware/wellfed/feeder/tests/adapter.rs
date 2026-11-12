//! The feeding adapter against a scripted owner: the answers, `Fed`
//! events and readiness the firmware and the simulator both get from it.
//!
//! - A fault before the reservation ends the feed refused `Fault`, with no
//!   `Fed` event: nothing moved.
//! - A fault after the reservation, its result unwritten, is recorded as
//!   uncertain, and nothing is acknowledged.
//! - A feed a reset interrupted holds readiness at `Resolving` until the
//!   owner holds its resolution, then is recorded once and acknowledged
//!   once.
//! - The owner may still refuse a feed once its claim is taken, at once or
//!   after the claim waited: the refusal reaches the asker, and the number
//!   goes to the next feed.
//! - Readiness puts the runtime's holds and the owner's fault ahead of the
//!   interrupted feed.

use std::cell::{Cell, RefCell};

use wellfed_core_feeder::feed::food_read::FoodReads;
use wellfed_core_feeder::feed::observe::Observation;
use wellfed_core_feeder::feed::storage::OwnerResult;
use wellfed_feeder::feed::{self, Durability};
use wellfed_feeder::feeder::{Feeds, RefusedClaim};
use wellfed_feeder::trace::DriveFigures;
use wellfed_fw_feeder::feeding::adapter::{FeedOwner, Feeding, Opening, Runtime};
use wellfed_fw_feeder::feeding::{PowerCycle, Record};
use wellfed_hal_feeder::owner::{OwnerFault, OwnerState, OwnerStatus, SubmitResult};
use wellfed_proto::api::{Blocked, EventBody, Refusal, Source, TraceSummary};
use wellfed_sdk::hal::interlock::{Interlock, WriteStep, WriteTurn};
use wellfed_sdk::hal::supervision::supervise;

/// An owner whose status the test sets, and whose answer to the next
/// submission it chooses.
#[derive(Default)]
struct Scripted {
    status: RefCell<OwnerStatus>,
    /// What `submit` answers; `Enqueued` queues the feed in the status.
    answer: Cell<Option<SubmitResult>>,
    acknowledged: Cell<u32>,
}

impl Scripted {
    fn set(&self, change: impl FnOnce(&mut OwnerStatus)) {
        change(&mut self.status.borrow_mut());
    }
}

impl FeedOwner for Scripted {
    fn step(&self) -> Option<u32> {
        Some(20)
    }

    fn status(&self) -> OwnerStatus {
        *self.status.borrow()
    }

    fn submit(&self, id: u64, portions: u16) -> SubmitResult {
        let answer = self.answer.get().unwrap_or(SubmitResult::Enqueued);
        if answer == SubmitResult::Enqueued {
            self.set(|status| {
                status.state = OwnerState::Queued;
                status.result = OwnerResult {
                    id,
                    requested: portions,
                    ..OwnerResult::default()
                };
            });
        }
        answer
    }

    fn cancel(&self, _: u64) -> bool {
        false
    }

    fn acknowledge(&self, id: u64) -> bool {
        let mut status = self.status.borrow_mut();
        let ours =
            status.state == OwnerState::Terminal && status.result.id == id && status.result_durable;
        if ours {
            status.state = OwnerState::Idle;
            self.acknowledged.set(self.acknowledged.get() + 1);
        }
        ours
    }

    fn high_water_id(&self) -> u64 {
        self.status.borrow().high_water_id
    }

    fn faulted(&self) -> bool {
        self.status.borrow().state == OwnerState::Fault
    }

    fn take_observation(&self) -> Option<Observation> {
        None
    }

    fn yield_food_read(&self, _: &Interlock) {}

    fn food_read_step(&self, _: &mut FoodReads, _: &Interlock, _: u32, _: bool) {}
}

#[derive(Default)]
struct Rig {
    owner: Scripted,
    opened: Cell<bool>,
    /// The feed the journal holds as interrupted, given at the opening.
    interrupted: Cell<Option<u64>>,
    held: Cell<bool>,
    confirming: Cell<bool>,
    interlock: Interlock,
    now_ms: Cell<u32>,
    records: RefCell<Vec<Record>>,
}

impl Runtime for Rig {
    type Owner = Scripted;
    type Entered<'a> = &'a Scripted;

    fn owner(&self) -> Option<&Scripted> {
        self.opened.get().then_some(&self.owner)
    }

    fn open(&self, _: &mut bool) -> Option<Opening> {
        self.opened.set(true);
        let high_water = self.owner.high_water_id();
        Some(Opening {
            record: Record::Opened {
                created: false,
                high_water,
            },
            interrupted: self.interrupted.get(),
        })
    }

    fn interlock(&self) -> &Interlock {
        &self.interlock
    }

    fn now_ms(&self) -> u32 {
        self.now_ms.get()
    }

    fn held(&self) -> bool {
        self.held.get()
    }

    fn confirming(&self) -> bool {
        self.confirming.get()
    }

    fn storage_armed(&self) -> bool {
        true
    }

    fn installing(&self) -> bool {
        false
    }

    fn writes_queued(&self) -> bool {
        false
    }

    fn power_cycle(&self) -> PowerCycle {
        PowerCycle::Restarted
    }

    fn report_feeding(&self, _: supervise::Feeding) {}

    fn request_reset(&self) {}

    fn record(&self, record: Record) {
        self.records.borrow_mut().push(record);
    }

    fn trace_begin(&self, _: u32) {}

    fn trace_observe(&self, _: &Observation) {}

    fn trace_seal(&self, _: &EventBody, _: &DriveFigures) -> Option<TraceSummary> {
        None
    }
}

/// An adapter on an owner at rest, opened by its first step.
fn opened(rig: Rig) -> Feeding<Rig> {
    rig.owner.set(|status| {
        status.state = OwnerState::Idle;
        status.stopped = true;
        status.optics_off = true;
    });
    let mut feeding = Feeding::new(rig);
    assert_eq!(step(&mut feeding), Vec::new());
    feeding
}

/// One step, a cadence's wait later: the events it recorded.
fn step(feeding: &mut Feeding<Rig>) -> Vec<EventBody> {
    let now = &feeding.runtime().now_ms;
    now.set(now.get() + 20);
    let mut events = Vec::new();
    feeding.step(false, |body, _| events.push(body));
    events
}

fn owner(feeding: &Feeding<Rig>) -> &Scripted {
    &feeding.runtime().owner
}

const BY: Source = Source::Panel;

#[test]
fn a_fault_before_the_reservation_ends_the_feed_refused() {
    let mut feeding = opened(Rig::default());
    assert_eq!(feeding.blocked(), None);
    assert_eq!(feeding.submit(2, BY), Ok(1));
    assert_eq!(feeding.reserved(1), None, "queued, not reserved");

    owner(&feeding).set(|status| status.state = OwnerState::Fault);
    assert_eq!(step(&mut feeding), Vec::new(), "nothing moved");

    assert_eq!(feeding.reserved(1), Some(Err(Refusal::Fault)));
    assert_eq!(feeding.blocked(), Some(Blocked::Fault));
    assert_eq!(owner(&feeding).acknowledged.get(), 0);
}

#[test]
fn a_fault_after_the_reservation_with_its_result_unwritten_is_uncertain() {
    let mut feeding = opened(Rig::default());
    assert_eq!(feeding.submit(3, BY), Ok(1));
    owner(&feeding).set(|status| {
        status.state = OwnerState::Feeding;
        status.intent_reserved = true;
    });
    assert_eq!(step(&mut feeding), Vec::new());
    assert_eq!(feeding.reserved(1), Some(Ok(())));

    owner(&feeding).set(|status| {
        status.state = OwnerState::Fault;
        status.fault = OwnerFault::Storage;
        status.result.completed = 1;
        status.result_durable = false;
    });
    let result = owner(&feeding).status().result;

    assert_eq!(
        step(&mut feeding),
        vec![feed::fed(1, &result, None, Durability::Lost, BY)]
    );
    assert_eq!(feeding.reserved(1), Some(Ok(())), "it ran");
    assert_eq!(owner(&feeding).acknowledged.get(), 0, "a fault takes none");
    assert_eq!(step(&mut feeding), Vec::new(), "recorded once");
}

#[test]
fn an_interrupted_feed_is_recorded_once_and_acknowledged_once() {
    let rig = Rig::default();
    rig.interrupted.set(Some(7));
    rig.owner.set(|status| status.high_water_id = 7);
    let mut feeding = opened(rig);
    assert_eq!(feeding.blocked(), Some(Blocked::Resolving));
    assert_eq!(step(&mut feeding), Vec::new(), "not resolved yet");
    assert_eq!(feeding.blocked(), Some(Blocked::Resolving));

    let resolved = OwnerResult {
        id: 7,
        requested: 2,
        ..OwnerResult::default()
    };
    owner(&feeding).set(|status| {
        status.state = OwnerState::Terminal;
        status.result = resolved;
        status.result_durable = true;
    });
    assert_eq!(
        step(&mut feeding),
        vec![feed::interrupted(7, &resolved, Source::Feeder)]
    );
    assert_eq!(owner(&feeding).acknowledged.get(), 1);
    assert_eq!(feeding.blocked(), None);

    assert_eq!(step(&mut feeding), Vec::new());
    assert_eq!(owner(&feeding).acknowledged.get(), 1, "once");
}

#[test]
fn a_refusal_after_the_claim_reaches_the_asker_and_frees_the_number() {
    let mut feeding = opened(Rig::default());

    // At once: the claim is taken in the submission itself.
    owner(&feeding).answer.set(Some(SubmitResult::RejectBusy));
    assert_eq!(feeding.submit(2, BY), Err(Refusal::Busy));
    owner(&feeding)
        .answer
        .set(Some(SubmitResult::RejectRequest));
    assert_eq!(feeding.submit(9, BY), Err(Refusal::Portions { asked: 9 }));
    let submitted = |records: &[Record]| {
        records
            .iter()
            .filter(|record| matches!(record, Record::Submitted { feed: 1, .. }))
            .count()
    };
    assert_eq!(submitted(&feeding.runtime().records.borrow()), 2);
    assert_eq!(step(&mut feeding), Vec::new());
    assert!(
        !feeding.runtime().interlock.outputs_claimed(),
        "handed back"
    );

    // After a wait: a write holds the turn, so the claim waits a pass.
    let rig = feeding.runtime();
    let begun = WriteTurn::new().begin(&rig.interlock, rig.now_ms.get(), || true);
    assert!(matches!(begun, WriteStep::Go(_)));
    owner(&feeding).answer.set(Some(SubmitResult::RejectBusy));
    assert_eq!(feeding.submit(1, BY), Ok(1), "promised, waiting");
    assert_eq!(feeding.claiming(), Some(BY));
    assert_eq!(feeding.reserved(1), None);
    assert_eq!(feeding.blocked(), Some(Blocked::Feeding));

    feeding.runtime().interlock.end_write();
    assert_eq!(
        feeding.claim_step(None),
        Some(RefusedClaim {
            feed: 1,
            by: BY,
            refusal: Refusal::Busy,
        })
    );
    assert_eq!(feeding.reserved(1), Some(Err(Refusal::Busy)));
    assert_eq!(feeding.claiming(), None);

    owner(&feeding).answer.set(None);
    assert_eq!(
        feeding.submit(1, BY),
        Ok(1),
        "the next feed takes the number"
    );
}

#[test]
fn a_faulted_owner_refuses_after_the_claim_as_a_fault() {
    let mut feeding = opened(Rig::default());
    owner(&feeding)
        .answer
        .set(Some(SubmitResult::RejectStopped));
    assert_eq!(feeding.submit(1, BY), Err(Refusal::Busy), "still opening");
    owner(&feeding).set(|status| status.state = OwnerState::Fault);
    assert_eq!(feeding.submit(1, BY), Err(Refusal::Fault));
}

#[test]
fn readiness_puts_holds_and_faults_ahead_of_the_interrupted_feed() {
    let rig = Rig::default();
    rig.interrupted.set(Some(4));
    rig.owner.set(|status| status.high_water_id = 4);
    let feeding = opened(rig);
    let rig = feeding.runtime();
    assert_eq!(feeding.blocked(), Some(Blocked::Resolving));

    rig.owner.set(|status| status.state = OwnerState::Fault);
    assert_eq!(feeding.blocked(), Some(Blocked::Fault));
    rig.owner.set(|status| status.state = OwnerState::Stopped);
    assert_eq!(feeding.blocked(), Some(Blocked::Fault), "a stop asked for");
    rig.owner.set(|status| status.state = OwnerState::Starting);
    assert_eq!(feeding.blocked(), Some(Blocked::Starting));

    rig.owner.set(|status| status.state = OwnerState::Idle);
    rig.confirming.set(true);
    assert_eq!(
        feeding.blocked(),
        Some(Blocked::Starting),
        "a trial confirms first"
    );
    rig.held.set(true);
    assert_eq!(
        feeding.blocked(),
        Some(Blocked::Fault),
        "a reset storm holds it"
    );
}
