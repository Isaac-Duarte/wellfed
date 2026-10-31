//! The zone through the HAL's clock: the loop keeps the year's transitions
//! from `keep`, and every reading agrees with the rule evaluated whole,
//! before and after the year is kept, across a transition, New Year and a
//! settings write that changes the zone.

use wellfed_core::clock::wall::{Origin, Sync};
use wellfed_core::clock::zone::{Rule, Zone};
use wellfed_hal::clock;
use wellfed_hal::sdk::fake;

// 2026-11-01T08:00:00Z, Denver leaves daylight time.
const FALL_BACK_S: u64 = 1_793_520_000;
// 2027-01-01T07:00:00Z, New Year in Denver's standard time.
const NEW_YEAR_S: u64 = 1_798_786_800;

fn rule(text: &str) -> Rule {
    Zone::parse(text.as_bytes()).unwrap().rule()
}

/// The clock's count at `ms`.
fn at(ms: u32) {
    fake::with(|h| {
        h.uptime_ms = ms;
        h.raw_ticks = u32::try_from(u64::from(ms) * 32_768 / 1000).unwrap();
    });
}

fn sync_to(unix_s: u64) {
    let sync = Sync {
        unix_ms: unix_s * 1000,
        offset_s: 0,
        next_change_s: 0,
        next_offset_s: 0,
    };
    clock::sync(sync, Origin::Owner).unwrap();
}

/// What the clock reads now, and what the rule says for that instant.
fn reading_agrees(zone: Rule) -> u64 {
    let reading = clock::state().reading.expect("the clock is set");
    let unix_s = i64::try_from(reading.unix_ms / 1000).unwrap();
    assert_eq!(
        reading.offset_s,
        Some(zone.offset_at(unix_s)),
        "at {unix_s}"
    );
    assert_eq!(clock::unix_ms(), Some(reading.unix_ms));
    assert_eq!(clock::offset_at(reading.unix_ms), reading.offset_s);
    reading.unix_ms
}

/// One process, one clock: the scenario runs in order.
#[test]
fn every_reading_agrees_with_the_rule_as_the_loop_keeps_the_year() {
    let denver = rule("MST7MDT,M3.2.0,M11.1.0");
    let sydney = rule("AEST-10AEDT,M10.1.0,M4.1.0/3");
    clock::reset();

    // A first fix 30 s before the transition, and the rule set before any
    // year is kept.
    at(1_000);
    clock::keep();
    sync_to(FALL_BACK_S - 30);
    clock::set_rule(Some(denver));
    let unix_ms = reading_agrees(denver);
    assert!(!clock::year_kept_for(unix_ms), "only the loop keeps it");

    // The loop's store comes due and keeps the year: the same answers, on
    // both sides of the transition.
    let mut now = 1_000;
    for _ in 0..7 {
        now += 10_000;
        at(now);
        clock::keep();
        let unix_ms = reading_agrees(denver);
        assert!(clock::year_kept_for(unix_ms));
        assert_eq!(clock::offset_at(FALL_BACK_S * 1000 - 1), Some(-6 * 3600));
        assert_eq!(clock::offset_at(FALL_BACK_S * 1000), Some(-7 * 3600));
        assert_eq!(unix_ms, (FALL_BACK_S - 30) * 1000 + u64::from(now - 1_000));
    }

    // A settings write: the new zone holds from that moment, with no pass of
    // the loop in between, and after its year is kept too.
    clock::set_rule(Some(sydney));
    let unix_ms = reading_agrees(sydney);
    assert!(
        !clock::year_kept_for(unix_ms),
        "the old year went with the rule"
    );
    now += 10_000;
    at(now);
    clock::keep();
    let unix_ms = reading_agrees(sydney);
    assert!(clock::year_kept_for(unix_ms));
    clock::set_rule(Some(denver));
    reading_agrees(denver);

    // The last store of the year keeps that year, up to New Year and no further.
    sync_to(NEW_YEAR_S - 15);
    now += 10_000;
    at(now);
    clock::keep();
    let unix_ms = reading_agrees(denver);
    assert_eq!(unix_ms, (NEW_YEAR_S - 5) * 1000);
    assert!(clock::year_kept_for(NEW_YEAR_S * 1000 - 1));
    assert!(!clock::year_kept_for(NEW_YEAR_S * 1000));

    // Past New Year, before the next store: the kept year does not cover the
    // reading, which takes the rule whole.
    now += 6_000;
    at(now);
    clock::keep();
    let unix_ms = reading_agrees(denver);
    assert_eq!(unix_ms, (NEW_YEAR_S + 1) * 1000);
    assert!(!clock::year_kept_for(unix_ms));

    // The next store replaces it.
    now += 4_000;
    at(now);
    clock::keep();
    let unix_ms = reading_agrees(denver);
    assert_eq!(unix_ms, (NEW_YEAR_S + 5) * 1000);
    assert!(clock::year_kept_for(unix_ms));
}
