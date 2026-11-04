//! The derived views from stored rows: a feeder's food view counts
//! portions from the last recorded refill and weighs them only once the
//! server has read the feeder's calibration, a beam-seen refill only asks
//! "refilled?", and a pet's intake splits a shared calibrated feeder, uses
//! each feed's own food, and reports an uncalibrated feeder in portions.
//!
//! The history is written straight into the store, with no feeder behind
//! it, so each feed lands at the time the test gives it.

use std::sync::Arc;

use jiff::Timestamp;
use uuid::Uuid;
use wellfed_api::{
    ClientRole, EventBody, FeedAnomaly, FeedOutcome, FeedReason, FeederEvent, FeederId,
    FeederPortions, FoodLevel, FoodStats, IntakeQuery, MissedReason, NewFood, NewPet, NewRefill,
    PetId, PortionWeight, Source, Species, Stats,
};
use wellfed_server_testkit::fixed::{ManualClock, NOW_MS};
use wellfed_server_testkit::{TestService, ada, ada_setup, stored_body};
use wellfed_service::{Error, Invalid, Resource, Service};
use wellfed_store::{NewEvent, NewFeeder, SealedKey, Store};

const KITCHEN: FeederId = FeederId(Uuid::from_u128(0x0192c5d0_1111_7000_8000_000000000001));

/// `minutes` after the test clock's start.
fn at(minutes: i64) -> Timestamp {
    Timestamp::from_millisecond(NOW_MS + minutes * 60_000).unwrap()
}

/// The service on a clock the test moves, with its household set up and
/// a feeder row no task mirrors, and a store handle to write history.
async fn start() -> (TestService, ManualClock, Store) {
    let clock = ManualClock::new();
    let test = TestService::start_with_clock(Arc::new(clock.clone())).await;
    test.service().setup(ada_setup()).await.unwrap();
    let store = test.store().open().await;
    store
        .insert_feeder(NewFeeder {
            id: KITCHEN,
            identity: [4; 32],
            name: "kitchen",
            role: ClientRole::Server,
            address: None,
            key: SealedKey {
                nonce: [7; 12],
                sealed: vec![1, 2, 3],
            },
            created: at(0),
        })
        .await
        .unwrap();
    (test, clock, store)
}

fn food(name: &str, kcal_per_kg: u32) -> NewFood {
    NewFood {
        brand: "Acme".into(),
        name: name.into(),
        kcal_per_kg,
        bag_grams: 2000,
    }
}

/// A feed's `Fed` event, received at `received`, at the feeder's own time
/// `at` (`None` while its clock was invalid).
async fn fed(
    store: &Store,
    seq: u64,
    at: Option<Timestamp>,
    received: Timestamp,
    portions: (u8, u8),
) {
    let (requested, completed) = portions;
    let outcome = if completed == requested {
        FeedOutcome::Done
    } else {
        FeedOutcome::Uncertain
    };
    let body = EventBody::Feeder(FeederEvent::Fed {
        feed: u32::try_from(seq).unwrap(),
        requested,
        completed,
        outcome,
        reason: FeedReason::Done,
        hopper: FoodLevel::Present,
        by: Source::Feeder,
    });
    record(store, seq, at, received, body).await;
}

async fn record(
    store: &Store,
    seq: u64,
    at: Option<Timestamp>,
    received: Timestamp,
    body: EventBody,
) {
    let raw = stored_body(body);
    let event = NewEvent {
        epoch: 1,
        seq,
        at,
        received,
        body,
        raw: &raw,
    };
    assert!(store.record_event(KITCHEN, event).await.unwrap());
}

async fn beam(store: &Store, received: Timestamp, present: bool) {
    let food = FoodStats {
        present: Some(present),
        ran_low: 0,
        refilled: 0,
        feeds_since_refill: None,
    };
    store
        .record_stats(KITCHEN, received, Some(1), &Stats::Food(food))
        .await
        .unwrap();
}

async fn calibrate(store: &Store, portion_mg: u32) {
    let portions = Stats::Portions(wellfed_model::Portions {
        portion_mg: Some(portion_mg),
        ..Default::default()
    });
    store
        .record_stats(KITCHEN, at(0), Some(1), &portions)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_recorded_refill_resets_the_count_and_the_beam_only_asks() {
    let (test, clock, store) = start().await;
    let service = test.service();
    let dry = service.create_food(food("Dry", 3600)).await.unwrap();
    service.set_feeder_food(KITCHEN, dry.id).await.unwrap();
    service
        .add_refill(&ada(), KITCHEN, NewRefill::Grams { grams: 1000 })
        .await
        .unwrap();

    // Uncalibrated: three portions, and no weight to set against the
    // refill's grams.
    fed(&store, 1, Some(at(30)), at(31), (3, 3)).await;
    beam(&store, at(40), false).await;
    beam(&store, at(50), true).await;
    clock.advance_to(at(60));
    let view = service.feeder_food(KITCHEN).await.unwrap();
    assert_eq!(view.weight, PortionWeight::Uncalibrated);
    let since = view.since_refill.unwrap();
    assert_eq!((since.portions, since.left), (3, None));
    assert_eq!(view.refilled_unrecorded, Some(at(50)), "refilled?");

    // The feeder's own calibration weighs every feed once the server has
    // read it.
    calibrate(&store, 9_000).await;
    let view = service.feeder_food(KITCHEN).await.unwrap();
    let calibrated = PortionWeight::Calibration { portion_mg: 9_000 };
    assert_eq!(view.weight, calibrated);
    let left = view.since_refill.unwrap().left.unwrap();
    assert_eq!((left.grams_used, left.grams_left), (27, 973));

    // A recorded refill starts the count again, and the beam's refill is
    // behind it.
    service
        .add_refill(&ada(), KITCHEN, NewRefill::Grams { grams: 500 })
        .await
        .unwrap();
    fed(&store, 2, Some(at(70)), at(70), (4, 1)).await;
    let missed = EventBody::Feeder(FeederEvent::Missed {
        plan: 2,
        due: at(80),
        reason: MissedReason::Late,
    });
    record(&store, 3, Some(at(95)), at(95), missed).await;
    // A meal due before the week the view looks back over, reported now.
    let stale = EventBody::Feeder(FeederEvent::Missed {
        plan: 3,
        due: at(-8 * 24 * 60),
        reason: MissedReason::Late,
    });
    record(&store, 4, Some(at(100)), at(100), stale).await;
    clock.advance_to(at(120));
    let view = service.feeder_food(KITCHEN).await.unwrap();
    let since = view.since_refill.unwrap();
    assert_eq!((since.portions, since.uncertain_portions), (1, 3));
    let left = since.left.unwrap();
    assert_eq!((left.grams_left, left.grams_left_least), (491, 464));
    assert_eq!(view.refilled_unrecorded, None);
    let uncertain = FeedAnomaly::Uncertain {
        feeder: KITCHEN,
        feed: 2,
        at: at(70),
        requested: 4,
        completed: 1,
    };
    let missed = FeedAnomaly::Missed {
        feeder: KITCHEN,
        plan: 2,
        due: at(80),
        reason: MissedReason::Late,
    };
    assert_eq!(view.anomalies, [uncertain, missed]);
    test.shutdown().await;
}

fn pet(name: &str, daily_kcal: Option<u32>) -> NewPet {
    NewPet {
        name: name.into(),
        species: Species::Cat,
        birth_date: None,
        daily_kcal,
    }
}

async fn intake(service: &Service, pet: PetId, days: u32) -> wellfed_api::PetIntake {
    let query = IntakeQuery { days: Some(days) };
    service.pet_intake(pet, query).await.unwrap()
}

#[tokio::test]
async fn intake_splits_a_shared_feeder_and_counts_each_feeds_food() {
    let (test, clock, store) = start().await;
    let service = test.service();
    let dry = service.create_food(food("Dry", 3600)).await.unwrap();
    let wet = service.create_food(food("Wet", 1000)).await.unwrap();
    let tom = service.create_pet(pet("Tom", Some(250))).await.unwrap();
    let jerry = service.create_pet(pet("Jerry", None)).await.unwrap();
    service.set_feeder_food(KITCHEN, dry.id).await.unwrap();
    service.assign_feeder(tom.id, KITCHEN).await.unwrap();
    calibrate(&store, 10_000).await;

    // Tom's alone: 20 g of dry.
    fed(&store, 1, Some(at(60)), at(60), (2, 2)).await;
    clock.advance_to(at(120));
    service.assign_feeder(jerry.id, KITCHEN).await.unwrap();
    // Shared, from a feeder whose clock was invalid: dated when received.
    fed(&store, 2, None, at(180), (2, 2)).await;
    clock.advance_to(at(240));
    service.set_feeder_food(KITCHEN, wet.id).await.unwrap();
    // Shared: 20 g of wet.
    fed(&store, 3, Some(at(300)), at(300), (2, 2)).await;
    clock.advance_to(at(330));

    let toms = intake(service, tom.id, 2).await;
    let jerrys = intake(service, jerry.id, 2).await;

    assert_eq!(toms.zone, wellfed_server_testkit::ZONE);
    assert_eq!(toms.days.len(), 2);
    assert_eq!(toms.days[0].weighed.unwrap().grams, 0, "yesterday");
    let today = &toms.days[1];
    let weighed = today.weighed.unwrap();
    // 20 + 10 + 10 g; 72 + 36 + 10 kcal of a 250 kcal target.
    assert_eq!((weighed.grams, weighed.kcal), (40, 118));
    assert_eq!(today.target_percent, Some(47));
    assert!(today.estimate);
    assert!(toms.feeders[0].shared);
    let today = &jerrys.days[1];
    let weighed = today.weighed.unwrap();
    assert_eq!((weighed.grams, weighed.kcal), (20, 46));
    assert_eq!(today.target_percent, None);
    test.shutdown().await;
}

#[tokio::test]
async fn an_uncalibrated_feeder_counts_portions_only() {
    let (test, clock, store) = start().await;
    let service = test.service();
    let tom = service.create_pet(pet("Tom", Some(250))).await.unwrap();
    let jerry = service.create_pet(pet("Jerry", None)).await.unwrap();
    service.assign_feeder(tom.id, KITCHEN).await.unwrap();
    service.assign_feeder(jerry.id, KITCHEN).await.unwrap();
    fed(&store, 1, Some(at(60)), at(60), (3, 2)).await;
    clock.advance_to(at(120));

    let toms = intake(service, tom.id, 1).await;

    assert_eq!(toms.feeders[0].weight, PortionWeight::Uncalibrated);
    let today = &toms.days[0];
    assert_eq!((today.weighed, today.target_percent), (None, None));
    // Whole, not split between Tom and Jerry.
    let portions = FeederPortions {
        feeder: KITCHEN,
        portions: 2,
        uncertain_portions: 1,
    };
    assert_eq!(today.unweighed, [portions]);
    let view = service.feeder_food(KITCHEN).await.unwrap();
    assert_eq!(view.since_refill, None, "no refill recorded");
    test.shutdown().await;
}

#[tokio::test]
async fn an_intake_covers_one_to_31_days_of_a_known_pet() {
    let (test, _clock, _store) = start().await;
    let service = test.service();
    let tom = service.create_pet(pet("Tom", None)).await.unwrap();

    for days in [0, 32] {
        let query = IntakeQuery { days: Some(days) };
        match service.pet_intake(tom.id, query).await {
            Err(Error::Invalid(Invalid::IntakeDays)) => {}
            other => panic!("{days} days: {other:?}"),
        }
    }
    let week = service.pet_intake(tom.id, IntakeQuery::default()).await;
    assert_eq!(week.unwrap().days.len(), 7);
    let nobody = PetId(Uuid::from_u128(1));
    match service.pet_intake(nobody, IntakeQuery::default()).await {
        Err(Error::NotFound(Resource::Pet)) => {}
        other => panic!("{other:?}"),
    }
    test.shutdown().await;
}
