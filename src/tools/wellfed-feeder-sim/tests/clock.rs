//! The platform maintains an expired clock even when no time fixes arrive.

use wellfed_core::clock::wall::{Fix, Origin};
use wellfed_feeder::feeder::Platform;
use wellfed_feeder_sim::flash::Flash;
use wellfed_feeder_sim::platform::SimPlatform;

#[test]
fn an_offline_platform_clock_stays_expired_across_uptime_wraps() {
    let flash = Flash::memory();
    let mut platform =
        SimPlatform::open(&flash, Box::new(|| Some([23; 32])), 1, 1, [2; 32], None).unwrap();
    let start = 1_767_254_400_000;
    platform
        .time_fix(
            0,
            Fix {
                unix_ms: start,
                error_ms: 1000,
                origin: Origin::Owner,
            },
        )
        .unwrap();

    for hour in 1..=24 * 100_u64 {
        let elapsed = hour * 3_600_000;
        let now = u32::try_from(elapsed % (u64::from(u32::MAX) + 1)).unwrap();
        // The loop gets its count before asking for wall time, even when
        // no schedule is configured and the clock has long been invalid.
        assert_eq!(platform.clock_ms(now), now);
        let wall = platform.wall_ms(now);
        if hour < 7 {
            assert_eq!(wall, Some(start + elapsed));
        } else {
            assert_eq!(wall, None, "clock revived after {hour} hours offline");
            assert_eq!(platform.clock_state(now).reading, None);
        }
    }
}
