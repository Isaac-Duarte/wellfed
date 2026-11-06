//! The writer can finish a plan's claim between the loop's schedule step
//! and a device request. That request must not replace the claimed meal.

use wellfed_core::clock::wall::{Origin, Sync};
use wellfed_core::schedule::{PLANS, Plan, Weekdays};
use wellfed_device_api::FeederDevice;
use wellfed_device_api::mailbox::{MealSkip, NtpServer, PlansChange, PlansResult, TimeSettings};
use wellfed_feeder::feeder::{Feeder, Feeds, Platform};
use wellfed_feeder_sim::flash::Flash;
use wellfed_feeder_sim::platform::SimPlatform;
use wellfed_proto::api::{Blocked, Refusal, Source};

#[derive(Default)]
struct RecordedFeeds(Vec<(u8, Source)>);

impl Feeds for RecordedFeeds {
    fn feeding(&self) -> bool {
        false
    }

    fn busy(&self) -> bool {
        false
    }

    fn blocked(&self) -> Option<Blocked> {
        None
    }

    fn submit(&mut self, portions: u8, by: Source) -> Result<u32, Refusal> {
        self.0.push((portions, by));
        Ok(1)
    }

    fn reserved(&self, _: u32) -> Option<Result<(), Refusal>> {
        Some(Ok(()))
    }

    fn cancel(&self, _: u32) -> bool {
        false
    }

    fn high_water(&self) -> Option<u32> {
        Some(0)
    }
}

#[test]
fn plan_changes_wait_for_a_claimed_meal_to_be_queued() {
    // 2026-01-01 08:00 UTC, with a one-portion meal due this minute.
    let breakfast = 1_767_254_400_000;
    let flash = Flash::memory();
    let mut platform =
        SimPlatform::open(&flash, Box::new(|| Some([23; 32])), 1, 1, [2; 32], None).unwrap();
    platform
        .time_sync(
            0,
            Sync {
                unix_ms: breakfast,
                offset_s: 0,
                next_change_s: 0,
                next_offset_s: 0,
            },
            Origin::Owner,
        )
        .unwrap();
    platform.time_settings_change(TimeSettings {
        zone: None,
        ntp: NtpServer::Off,
    });
    let mut table = [None; PLANS];
    table[0] = Some(Plan {
        minute: 8 * 60,
        portions: 1,
        days: Weekdays::EVERY_DAY,
        enabled: true,
        call: 0,
    });
    table[1] = Some(Plan {
        minute: 9 * 60,
        ..table[0].unwrap()
    });
    platform.plans_change(PlansChange::Replace {
        table,
        at_s: Some(u32::try_from(breakfast / 1000 - 60).unwrap()),
    });
    platform.write();
    let mut feeder = Feeder::new(platform);
    let mut feeds = RecordedFeeds::default();
    feeder.pass(0, &mut feeds, |_| {});
    feeder.pass(1, &mut feeds, |_| {});
    assert!(matches!(
        feeder.platform_mut().plans(),
        PlansResult::Current { pending: true, .. }
    ));
    feeder.pass(2, &mut feeds, |_| {});

    // The writer publishes the claim after local_stage saw it pending,
    // before serve handles a PutPlans with the still-current revision.
    feeder.platform_mut().write();
    let mut device = feeder.device(2, &mut feeds);
    let PlansResult::Current { view, pending } = device.plans(None) else {
        panic!("the plans are readable");
    };
    assert!(!pending);
    assert_eq!(
        view.revision, 1,
        "a recurring claim leaves the revision alone"
    );

    // The other slot's next meal is still to come today, so the claim's
    // hold is what refuses its skip, not the due-meal validation.
    let later = u32::try_from((breakfast + 3_600_000) / 1000).unwrap();
    assert_eq!(
        device.skip_meal(1, later, MealSkip::Skip, 2),
        Err(Refusal::Busy)
    );

    let replacement = table[0].as_mut().unwrap();
    replacement.minute = 9 * 60;
    replacement.portions = 7;
    assert_eq!(device.plans(Some(table)), PlansResult::Busy);

    feeder.platform_mut().write();
    feeder.pass(3, &mut feeds, |_| {});
    assert_eq!(feeds.0, [(1, Source::Schedule(0))]);

    assert_eq!(
        feeder
            .device(3, &mut feeds)
            .skip_meal(1, later, MealSkip::Skip, 3),
        Ok(()),
        "the same future meal can be skipped once the claim is queued"
    );
    feeder.platform_mut().write();

    // Once the meal has been queued, the owner guards any actual feed.
    // This recording owner is idle, so the table can now be changed.
    assert!(matches!(
        feeder.device(3, &mut feeds).plans(Some(table)),
        PlansResult::Current { pending: true, .. }
    ));
}
