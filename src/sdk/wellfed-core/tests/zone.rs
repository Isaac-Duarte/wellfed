use proptest::prelude::*;
use wellfed_core::clock::zone::{Rule, TEXT_MAX, Year, Zone};

const H: i32 = 3600;

fn rule(text: &str) -> Rule {
    Zone::parse(text.as_bytes()).unwrap().rule()
}

#[test]
fn fixed_zones_have_one_offset() {
    assert_eq!(rule("UTC0").offset_at(0), 0);
    assert_eq!(rule("<+0530>-5:30").offset_at(1_800_000_000), 5 * H + 1800);
    assert_eq!(rule("<-03>3").offset_at(1_800_000_000), -3 * H);
    assert_eq!(rule("MST7").next_change(1_800_000_000), None);
}

#[test]
fn denver_changes_on_the_second_sunday_of_march_and_first_of_november() {
    let denver = rule("MST7MDT,M3.2.0,M11.1.0");
    // 2026-03-08T09:00:00Z is 02:00 MST: daylight time from then.
    assert_eq!(denver.offset_at(1_772_960_399), -7 * H);
    assert_eq!(denver.offset_at(1_772_960_400), -6 * H);
    // 2026-11-01T08:00:00Z is 02:00 MDT: standard time from then.
    assert_eq!(denver.offset_at(1_793_519_999), -6 * H);
    assert_eq!(denver.offset_at(1_793_520_000), -7 * H);
    assert_eq!(
        denver.next_change(1_792_674_000),
        Some((1_793_520_000, -7 * H))
    );
    assert_eq!(
        denver.next_change(1_793_520_000),
        Some((1_805_014_800, -6 * H))
    );
}

#[test]
fn the_southern_hemisphere_spans_new_year() {
    // Sydney: daylight from the first Sunday of October to the first of April.
    let sydney = rule("AEST-10AEDT,M10.1.0,M4.1.0/3");
    // 2026-01-15: summer.
    assert_eq!(sydney.offset_at(1_768_435_200), 11 * H);
    // 2026-07-01: winter.
    assert_eq!(sydney.offset_at(1_782_864_000), 10 * H);
}

#[test]
fn last_weekday_and_julian_dates() {
    // Europe/Berlin: last Sunday of March 01:00 UTC.
    let berlin = rule("CET-1CEST,M3.5.0,M10.5.0/3");
    // 2026-03-29T01:00:00Z.
    assert_eq!(berlin.offset_at(1_774_745_999), H);
    assert_eq!(berlin.offset_at(1_774_746_000), 2 * H);

    // Permanent daylight time written as all-year rules.
    let permanent = rule("EST5EDT,0/0,J365/25");
    assert_eq!(permanent.offset_at(1_782_864_000), -4 * H);
}

#[test]
fn bad_rules_are_refused() {
    for text in [
        "",
        "U0",
        "UTC",
        "UTC25",
        "<-13>13",
        "<+15>-15",
        "<+14>-14<+15>,M3.2.0,M11.1.0",
        "MST7MDT",
        "MST7MDT,M3.2.0",
        "MST7MDT,M13.2.0,M11.1.0",
        "MST7MDT,M3.6.0,M11.1.0",
        "MST7MDT,M3.2.7,M11.1.0",
        "MST7MDT,J0,J365",
        "MST7MDT,M3.2.0,M11.1.0/168",
        "<+05>-5x",
        "<+5>-5",
    ] {
        assert!(Zone::parse(text.as_bytes()).is_err(), "{text:?}");
    }
    assert!(Zone::parse(&[b'A'; TEXT_MAX + 1]).is_err());
}

#[test]
fn the_text_is_kept_as_sent() {
    let zone = Zone::parse(b"MST7MDT,M3.2.0,M11.1.0").unwrap();
    assert_eq!(zone.text(), b"MST7MDT,M3.2.0,M11.1.0");
}

/// Every form of date, both hemispheres, a transition past the day's end
/// and a zone with no daylight time.
const RULES: [&str; 7] = [
    "MST7MDT,M3.2.0,M11.1.0",
    "AEST-10AEDT,M10.1.0,M4.1.0/3",
    "CET-1CEST,M3.5.0,M10.5.0/3",
    "<+0530>-5:30",
    "AAA3BBB,J60,J300",
    "AAA-14BBB-13,59/25,300/-1",
    "AAA-1BBB,M1.1.0/167,M12.5.0/-167",
];

const YEAR_S: i64 = 366 * 86_400;

/// The instants a rule's offset could get wrong: each second around every
/// transition and New Year, from `from_s` on for two years.
fn edges(rule: Rule, from_s: i64) -> Vec<i64> {
    let mut at = Vec::new();
    let mut seen = from_s;
    while let Some((change, _)) = rule.next_change(seen) {
        if change > from_s + 2 * YEAR_S {
            break;
        }
        at.extend([change - 1, change, change + 1]);
        seen = change;
    }
    for year in 0..3 {
        // Around New Year at each offset a zone can have: UTC-12 to UTC+14.
        let new_year = 1_767_225_600 + year * 365 * 86_400;
        at.extend((-14..=12).flat_map(|hour| {
            let utc = new_year + hour * 3600;
            [utc - 1, utc, utc + 1]
        }));
    }
    at
}

#[test]
fn a_kept_year_answers_as_the_rule_does_across_each_transition_and_new_year() {
    for text in RULES {
        let rule = rule(text);
        for at in edges(rule, 1_767_225_600) {
            let expected = rule.offset_at(at);
            assert_eq!(rule.offset_in(&Year::NONE, at), expected, "{text} at {at}");

            // Kept for that instant's year, and for the years either side:
            // one that does not cover the instant leaves it to the rule.
            for kept_at in [at, at - YEAR_S, at + YEAR_S] {
                if let Some(year) = rule.year_at(kept_at) {
                    assert_eq!(rule.offset_in(&year, at), expected, "{text} at {at}");
                }
            }
        }
    }
}

/// The kept year's bounds are the rule's: each instant covered belongs to it,
/// by the year `offset_at` reckons, and the instants either side do not.
#[test]
fn a_kept_year_covers_exactly_its_own_standard_local_year() {
    let denver = rule("MST7MDT,M3.2.0,M11.1.0");
    // 2027-01-01T07:00:00Z, New Year in standard time.
    let new_year = 1_798_786_800;

    let year = denver.year_at(new_year).unwrap();
    assert!(!year.covers(new_year - 1));
    assert!(year.covers(new_year));
    assert!(year.covers(new_year + 365 * 86_400 - 1));
    assert!(!year.covers(new_year + 365 * 86_400));
    assert!(!Year::NONE.covers(0));

    // A leap year's is a day longer: 2028-01-01T07:00:00Z.
    let leap = new_year + 365 * 86_400;
    let year = denver.year_at(leap).unwrap();
    assert!(year.covers(leap + 366 * 86_400 - 1));
    assert!(!year.covers(leap + 366 * 86_400));
}

proptest! {
    /// Whatever year a rule keeps, and whatever instant is asked about.
    #[test]
    fn a_kept_year_never_changes_the_answer(
        text in proptest::sample::select(RULES.to_vec()),
        kept_at in 1_767_225_600i64..4_000_000_000,
        at in 1_767_225_600i64..4_000_000_000,
        near in -40_000_000i64..40_000_000,
    ) {
        let rule = rule(text);
        if let Some(year) = rule.year_at(kept_at) {
            prop_assert_eq!(rule.offset_in(&year, at), rule.offset_at(at));
            prop_assert_eq!(rule.offset_in(&year, kept_at + near), rule.offset_at(kept_at + near));
        }
    }

    #[test]
    fn no_text_panics(bytes in proptest::collection::vec(any::<u8>(), 0..64), at in any::<i64>()) {
        if let Ok(zone) = Zone::parse(&bytes) {
            let at = at.clamp(-10_000_000_000, 10_000_000_000);
            let _ = zone.rule().offset_at(at);
            let _ = zone.rule().next_change(at);
        }
    }
}
