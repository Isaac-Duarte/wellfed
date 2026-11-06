//! The feeding plans through the service: replaced as one table against
//! their generation, a retry under the same key answered once, and the
//! next meal reckoned from the copy the server holds, which follows a
//! change another client makes; and a skip of one of today's meals, which
//! moves that next meal past it until its undo.

use jiff::Timestamp;
use wellfed_api::{
    Connection as Link, FeederRefusal, Generation, MealRequest, NextMeal, PLANS, Plan, Plans,
    PlansChange, Weekdays,
};
use wellfed_model::{Answer, Op, TimeSync};
use wellfed_server_testkit::feeder::{self, open_owner};
use wellfed_server_testkit::{TestService, ada, ada_setup, local, until};
use wellfed_service::{Error, Invalid};

/// 18:00 every day, two portions.
const SUPPER: Plan = Plan {
    minute: 18 * 60,
    portions: 2,
    days: Weekdays::DAILY,
    enabled: true,
    call: 0,
};

/// [`SUPPER`]'s next meal from the fixed clock (2026-09-21T14:13:20Z),
/// in Berlin's summer time: 16:00Z that day.
const SUPPER_AT: i64 = 1_790_006_400;

/// The plans with `plan` in slot `slot` and every other slot empty.
fn only(slot: usize, plan: Plan) -> PlansChange {
    let mut plans = [None; PLANS];
    plans[slot] = Some(plan);
    PlansChange { plans }
}

#[test]
fn plans_change_against_the_current_generation_only() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        let service = server.service();
        service.setup(ada_setup()).await.unwrap();
        let id = server.adopt(&sim).await;
        server.link(id, Link::Online).await;

        let read = service.plans(id).await.unwrap();
        assert_eq!(read.plans, [None; PLANS]);
        let change = only(0, SUPPER);
        let changed = service
            .put_plans(&ada().into(), id, read.generation, change, Some([1; 16]))
            .await
            .unwrap();
        assert_eq!(changed.plans, change.plans);
        assert_eq!(changed.generation, Generation(read.generation.0 + 1));

        // A retry under the same key is answered from the journal, not
        // applied again.
        let retried = service
            .put_plans(&ada().into(), id, read.generation, change, Some([1; 16]))
            .await
            .unwrap();
        assert_eq!(retried, changed);
        assert_eq!(
            service.plans(id).await.unwrap().generation,
            changed.generation,
            "the retry did not write again"
        );

        // A stale base gets the current plans.
        let stale = service
            .put_plans(&ada().into(), id, read.generation, change, Some([2; 16]))
            .await;
        let Err(Error::StalePlans(current)) = stale else {
            panic!("{stale:?}");
        };
        assert_eq!(*current, changed);

        // A plan out of range is refused by its slot before anything is
        // sent.
        let empty = Plan {
            portions: 0,
            ..SUPPER
        };
        let refused = service
            .put_plans(
                &ada().into(),
                id,
                changed.generation,
                only(3, empty),
                Some([3; 16]),
            )
            .await;
        assert!(matches!(
            refused,
            Err(Error::Invalid(Invalid::Plan { slot: 3 }))
        ));
        assert_eq!(service.plans(id).await.unwrap(), changed);

        // The mirror reads the plans again after the change's `plans` event.
        let want = NextMeal {
            at: Timestamp::from_second(SUPPER_AT).unwrap(),
            slot: 0,
            portions: 2,
        };
        until("the next meal follows the change", || async {
            service.feeder(id).await.unwrap().next_meal == Some(want)
        })
        .await;
        server.shutdown().await;
        sim.stop().await;
    });
}

/// The owner changes the plans on the feeder, not through the server: the
/// `plans` event has the mirror read them again, so the next meal follows.
#[test]
fn a_change_another_client_makes_moves_the_next_meal() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        let service = server.service();
        service.setup(ada_setup()).await.unwrap();
        let id = server.adopt(&sim).await;
        server.link(id, Link::Online).await;
        assert_eq!(service.feeder(id).await.unwrap().next_meal, None);

        let mut owner = open_owner(&sim).await;
        let Answer::Plans(Plans { generation, .. }) =
            owner.ask([0x71; 16], &Op::GetPlans).await.unwrap()
        else {
            panic!("no plans");
        };
        let breakfast = Plan {
            minute: 7 * 60,
            portions: 1,
            ..SUPPER
        };
        let put = Op::PutPlans {
            base: generation,
            plans: only(4, breakfast).plans,
        };
        let answer = owner.ask([0x72; 16], &put).await.unwrap();
        assert!(matches!(answer, Answer::Done { .. }), "{answer:?}");
        drop(owner);

        // 07:00 in Berlin the next day is 05:00Z.
        let want = NextMeal {
            at: Timestamp::from_second(1_790_053_200).unwrap(),
            slot: 4,
            portions: 1,
        };
        until("the next meal follows the owner's change", || async {
            service.feeder(id).await.unwrap().next_meal == Some(want)
        })
        .await;
        server.shutdown().await;
        sim.stop().await;
    });
}

/// A plan that runs once disables itself on the feeder with no `plans`
/// event: its scheduled feed has the mirror read the plans again, so the
/// meal it ran is not shown as the next one.
#[test]
fn a_plan_that_runs_once_leaves_no_next_meal() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        let service = server.service();
        service.setup(ada_setup()).await.unwrap();
        let id = server.adopt(&sim).await;
        server.link(id, Link::Online).await;

        let once = Plan {
            days: Weekdays::ONCE,
            ..SUPPER
        };
        let read = service.plans(id).await.unwrap();
        service
            .put_plans(&ada().into(), id, read.generation, only(0, once), None)
            .await
            .unwrap();
        until("the once plan is the next meal", || async {
            service.feeder(id).await.unwrap().next_meal.is_some()
        })
        .await;

        // The owner moves the feeder's clock to two seconds before the
        // meal, at Berlin's summer offset (the server's key keeps no zone
        // rule on this feeder).
        let mut owner = open_owner(&sim).await;
        let sync = TimeSync {
            unix_ms: u64::try_from(SUPPER_AT * 1000 - 2000).unwrap(),
            offset_s: 2 * 3600,
            next_change_s: 0,
            next_offset_s: 0,
        };
        let answer = owner.ask([0x73; 16], &Op::TimeSync(sync)).await.unwrap();
        assert!(matches!(answer, Answer::Done { .. }), "{answer:?}");
        drop(owner);

        until("the meal ran and the plan disabled itself", || async {
            service.feeder(id).await.unwrap().next_meal.is_none()
        })
        .await;
        let plans = service.plans(id).await.unwrap();
        assert!(plans.plans[0].is_some_and(|plan| !plan.enabled));
        server.shutdown().await;
        sim.stop().await;
    });
}

/// A skip through the server: the feeder's plans name it, the next meal
/// moves to the day after, a retry under its key is answered once, and a
/// second skip is refused. The owner's undo on the feeder itself brings the
/// meal back as the next one, read again after its `unskipped` event.
#[test]
fn a_skip_moves_the_next_meal_until_another_client_undoes_it() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        let service = server.service();
        service.setup(ada_setup()).await.unwrap();
        let id = server.adopt(&sim).await;
        server.link(id, Link::Online).await;
        let read = service.plans(id).await.unwrap();
        service
            .put_plans(&ada().into(), id, read.generation, only(0, SUPPER), None)
            .await
            .unwrap();
        let supper = Timestamp::from_second(SUPPER_AT).unwrap();
        let next = |at: i64| NextMeal {
            at: Timestamp::from_second(at).unwrap(),
            slot: 0,
            portions: 2,
        };
        until("supper is the next meal", || async {
            service.feeder(id).await.unwrap().next_meal == Some(next(SUPPER_AT))
        })
        .await;

        let meal = MealRequest {
            slot: 0,
            due: supper,
        };
        let caller = ada().into();
        service
            .skip_meal(&caller, id, meal, Some([4; 16]))
            .await
            .unwrap();
        assert_eq!(
            service.plans(id).await.unwrap().skipped.get(0),
            Some(supper)
        );
        until("the next meal is tomorrow's supper", || async {
            service.feeder(id).await.unwrap().next_meal == Some(next(SUPPER_AT + 86_400))
        })
        .await;

        service
            .skip_meal(&caller, id, meal, Some([4; 16]))
            .await
            .expect("a retry under the same key is answered from the journal");
        let again = service.skip_meal(&caller, id, meal, Some([5; 16])).await;
        assert!(
            matches!(again, Err(Error::Refused(FeederRefusal::Invalid))),
            "{again:?}"
        );
        let out_of_range = MealRequest { slot: 10, ..meal };
        let refused = service.skip_meal(&caller, id, out_of_range, None).await;
        assert!(
            matches!(refused, Err(Error::Invalid(Invalid::Meal))),
            "{refused:?}"
        );

        let mut owner = open_owner(&sim).await;
        let undo = Op::UnskipMeal {
            slot: 0,
            due: supper,
        };
        let answer = owner.ask([0x74; 16], &undo).await.unwrap();
        assert!(matches!(answer, Answer::Done { .. }), "{answer:?}");
        drop(owner);
        until("supper is the next meal again", || async {
            service.feeder(id).await.unwrap().next_meal == Some(next(SUPPER_AT))
        })
        .await;

        let late = service.unskip_meal(&caller, id, meal, None).await;
        assert!(
            matches!(late, Err(Error::Refused(FeederRefusal::Invalid))),
            "no skip left to undo: {late:?}"
        );
        server.shutdown().await;
        sim.stop().await;
    });
}
