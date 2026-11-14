use wellfed_core::clock::wall::{
    CARRY_MAX_S, CARRY_WORDS, DISJOINT_LIMIT, Fix, Jump, LEARNED_FLOOR_PPM, MAX_ERROR_MS,
    MIN_UNIX_MS, MISSED_GRACE_MS, NoResume, Occurrence, Origin, Reading, Restart, Resumed, STEP_MS,
    SYNC_ERROR_MS, Sync, SyncError, SyncKey, UNLEARNED_PPM, WallClock, occurrence,
};
use wellfed_core::clock::zone::Zone;

// 2026-10-22T13:00:00Z; Mountain Daylight Time until 2026-11-01T08:00:00Z.
const NOON: u64 = 1_792_674_000_000;
const MDT: i32 = -6 * 3600;
const MST: i32 = -7 * 3600;
const FALL_BACK_S: u64 = 1_793_520_000;
const HOUR_MS: u32 = 3_600_000;
/// The owner's key a test's syncs come from.
const OWNER_KEY: SyncKey = [0x21; 4];

fn sync(unix_ms: u64) -> Sync {
    Sync {
        unix_ms,
        offset_s: MDT,
        next_change_s: FALL_BACK_S,
        next_offset_s: MST,
    }
}

fn fix(unix_ms: u64, error_ms: u32, origin: Origin) -> Fix {
    Fix {
        unix_ms,
        error_ms,
        origin,
    }
}

fn ntp(unix_ms: u64, error_ms: u32) -> Fix {
    fix(unix_ms, error_ms, Origin::Ntp)
}

/// A valid reading never claims to be closer to `truth` than it is.
fn honest(reading: Reading, truth: u64, when: &str) {
    let off = reading.unix_ms.abs_diff(truth);
    assert!(
        off <= reading.error_ms,
        "{when}: {off} ms off, claims {} ms",
        reading.error_ms
    );
}

#[test]
fn invalid_until_the_first_sync_then_carried_by_uptime() {
    let mut clock = WallClock::default();
    assert_eq!(clock.read(5_000), None);

    assert_eq!(
        clock.sync(5_000, sync(NOON), Origin::Owner, OWNER_KEY),
        Ok(Jump::First)
    );
    let reading = clock.read(65_000).unwrap();
    assert_eq!(reading.unix_ms, NOON + 60_000);
    assert_eq!(reading.offset_s, Some(MDT));
    assert_eq!(
        reading.local_ms(),
        Some((NOON + 60_000) as i64 - 6 * 3_600_000)
    );
    // The sync's own second plus 5000 ppm of a minute.
    assert_eq!(reading.error_ms, 1_300);
    assert_eq!(clock.syncs(), 1);
}

#[test]
fn an_unlearned_clock_expires_when_its_bound_passes_two_minutes() {
    let mut clock = WallClock::default();
    let start = u32::MAX - 1_000;
    clock
        .sync(start, sync(NOON), Origin::Owner, OWNER_KEY)
        .unwrap();
    assert_eq!(clock.read(9_000).unwrap().unix_ms, NOON + 10_001);

    // 1 s plus 5000 ppm reaches 120 s after 23 800 s, about 6.6 h.
    let limit =
        u32::try_from((MAX_ERROR_MS - 1000) * 1_000_000 / u64::from(UNLEARNED_PPM)).unwrap();
    assert!(clock.read(start.wrapping_add(limit)).is_some());
    assert_eq!(clock.read(start.wrapping_add(limit + 1000)), None);
}

#[test]
fn ntp_alone_sets_an_invalid_clock_without_an_offset() {
    let mut clock = WallClock::default();
    assert_eq!(clock.fix(0, ntp(NOON, 40)), Ok(Jump::First));
    let reading = clock.read(0).unwrap();
    assert_eq!((reading.unix_ms, reading.error_ms), (NOON, 40));
    assert_eq!(reading.offset_s, None);
    assert_eq!(reading.local_ms(), None);
}

#[test]
fn the_offset_follows_the_announced_change_or_the_zone_rule() {
    let mut clock = WallClock::default();
    let before = FALL_BACK_S * 1000 - 1000;
    clock
        .sync(0, sync(before), Origin::Owner, OWNER_KEY)
        .unwrap();
    assert_eq!(clock.read(999).unwrap().offset_s, Some(MDT));
    assert_eq!(clock.read(1000).unwrap().offset_s, Some(MST));

    // A rule wins over the sync's hint: New York left daylight time two
    // hours before Denver.
    let rule = Zone::parse(b"EST5EDT,M3.2.0,M11.1.0").unwrap().rule();
    clock.set_rule(Some(rule));
    assert_eq!(clock.read(999).unwrap().offset_s, Some(-5 * 3600));
    clock.set_rule(None);
    assert_eq!(clock.read(999).unwrap().offset_s, Some(MDT));
}

#[test]
fn owner_syncs_step_freely_and_are_classified() {
    let mut clock = WallClock::default();
    clock.sync(0, sync(NOON), Origin::Owner, OWNER_KEY).unwrap();
    assert_eq!(
        clock.sync(
            10_000,
            sync(NOON + 10_000 + STEP_MS as u64),
            Origin::Owner,
            OWNER_KEY
        ),
        Ok(Jump::Step(STEP_MS))
    );
    assert_eq!(
        clock.sync(
            20_000,
            sync(NOON + 20_000 + 3_600_000),
            Origin::Owner,
            OWNER_KEY
        ),
        Ok(Jump::Jumped(3_600_000 - STEP_MS))
    );
    assert_eq!(
        clock.sync(30_000, sync(NOON), Origin::Owner, OWNER_KEY),
        Ok(Jump::Jumped(-3_630_000))
    );
    assert_eq!(clock.read(30_000).unwrap().unix_ms, NOON);
}

#[test]
fn other_sources_move_the_reading_only_within_the_trusted_interval() {
    let mut clock = WallClock::default();
    clock.sync(0, sync(NOON), Origin::Owner, OWNER_KEY).unwrap();

    // Ten minutes on, the trusted interval is NOON+600 000 ± 4 000 ms. NTP
    // 1.5 s ahead with 100 ms of error lies inside: the reading moves to
    // it, but still claims the trusted interval's error and its own move.
    let now = 600_000;
    let jump = clock.fix(now, ntp(NOON + 601_500, 100)).unwrap();
    let reading = clock.read(now).unwrap();
    assert_eq!(reading.unix_ms, NOON + 601_500);
    assert_eq!(reading.error_ms, 1_500 + 4_000);
    assert_eq!(jump, Jump::Step(1_500));

    // An hour off is refused, and the clock is unchanged.
    assert_eq!(
        clock.fix(now, ntp(NOON + 600_000 + 3_600_000, 100)),
        Err(SyncError::Bounded)
    );
    assert_eq!(
        clock.sync(
            now,
            sync(NOON + 600_000 + 3_600_000),
            Origin::Client,
            OWNER_KEY
        ),
        Err(SyncError::Bounded)
    );
    assert_eq!(clock.read(now).unwrap().unix_ms, NOON + 601_500);
    assert_eq!(clock.syncs(), 2);
}

#[test]
fn a_member_cannot_walk_the_clock_with_many_syncs() {
    let mut clock = WallClock::default();
    clock.sync(0, sync(NOON), Origin::Owner, OWNER_KEY).unwrap();

    // Each sync claims a time just inside the clock's interval, ahead.
    let mut now = 0;
    for _ in 0..1000 {
        now += 100;
        let reading = clock.read(now).unwrap();
        let claimed = reading.unix_ms + reading.error_ms + u64::from(SYNC_ERROR_MS);
        let _ = clock.sync(now, sync(claimed), Origin::Client, OWNER_KEY);
    }
    // 100 s passed; the clock may sit at most the trusted interval's error
    // (the owner's second plus drift) ahead of true time, and says so.
    let truth = NOON + u64::from(now);
    let reading = clock.read(now).unwrap();
    assert!(reading.unix_ms - truth <= 1_000 + 500 + 1);
    honest(reading, truth, "after 1000 syncs");
}

/// The reviewer's walk: an attacker answers the feeder's NTP every hour at
/// the edge of what the clock allows, with a small claimed error, and an
/// honest answer follows. With an owner's sync every twelve hours the
/// clock never leaves the trusted interval and never claims to know more
/// than it does, for a week.
#[test]
fn an_ntp_attacker_cannot_walk_a_valid_clock_past_its_bound() {
    let mut clock = WallClock::default();
    clock.fix(0, fix(NOON, 50, Origin::Owner)).unwrap();
    for hour in 1..=24 * 7u32 {
        let now = hour * HOUR_MS;
        let truth = NOON + u64::from(now);
        if hour % 12 == 0 {
            clock.fix(now, fix(truth, 50, Origin::Owner)).unwrap();
        }
        let reading = clock.read(now).unwrap();
        let edge = reading.unix_ms + reading.error_ms - 5;
        let _ = clock.fix(now, ntp(edge, 5));
        let _ = clock.fix(now + 1000, ntp(truth + 1000, 20));

        let reading = clock.read(now + 1000).expect("valid with owner syncs");
        honest(reading, truth + 1000, &format!("hour {hour}"));
        assert!(reading.error_ms <= MAX_ERROR_MS);
    }
}

/// Without an owner's sync, the same attack runs into the clock's expiry:
/// it reads invalid once what it claims passes two minutes, instead of
/// staying valid with a small error while walked away. (Pushed to the edge
/// of the trusted interval, it claims the push as well, so it expires
/// sooner than the trusted interval alone would.)
#[test]
fn without_an_owner_the_attacked_clock_expires_rather_than_lie() {
    let mut clock = WallClock::default();
    clock.fix(0, fix(NOON, 50, Origin::Owner)).unwrap();
    let mut expired = None;
    for hour in 1..=24u32 {
        let now = hour * HOUR_MS;
        let truth = NOON + u64::from(now);
        let reading = clock.read(now).expect("valid until pushed past its bound");
        honest(reading, truth, &format!("hour {hour}"));
        let _ = clock.fix(now, ntp(reading.unix_ms + reading.error_ms - 5, 5));
        match clock.read(now) {
            Some(reading) => honest(reading, truth, &format!("hour {hour}, pushed")),
            None => {
                expired = Some(hour);
                break;
            }
        }
        let _ = clock.fix(now + 1000, ntp(truth + 1000, 20));
    }
    assert!(expired.is_some_and(|hour| hour <= 7), "{expired:?}");
}

/// The reviewer's poisoning: one spoofed answer an hour after the first
/// NTP fix must not teach the clock a rate. Every reading afterwards stays
/// honest, and nothing is learned from untrusted fixes.
#[test]
fn a_spoofed_answer_does_not_teach_the_clock_a_rate() {
    let mut clock = WallClock::default();
    clock.fix(0, ntp(NOON, 20)).unwrap();
    let span = HOUR_MS;
    let reading = clock.read(span).unwrap();
    clock
        .fix(span, ntp(reading.unix_ms + reading.error_ms - 25, 20))
        .unwrap();
    assert!(!clock.state(span).learned);
    assert_eq!(clock.state(span).rate_ppb, 0);

    for hours in [1u32, 6, 24, 72, 240] {
        let now = span + hours * HOUR_MS;
        let truth = NOON + u64::from(now);
        if let Some(reading) = clock.read(now) {
            honest(reading, truth, &format!("+{hours} h"));
        }
        let _ = clock.fix(now, ntp(truth, 20));
        let reading = clock.read(now).expect("an honest answer sets it");
        honest(reading, truth, &format!("+{hours} h, answered"));
    }
}

/// An uptime clock that runs `ppm` slow.
fn slow_by(ppm: u64, true_elapsed_ms: u64) -> u32 {
    u32::try_from(true_elapsed_ms - true_elapsed_ms * ppm / 1_000_000).unwrap()
}

/// An uptime clock that runs 735 ppm slow, as the research unit's did on
/// October 22.
fn slow(true_elapsed_ms: u64) -> u32 {
    slow_by(735, true_elapsed_ms)
}

/// The bench's refusal (October 26): with the uptime clock 2,060 ppm slow,
/// the poll 1,026 s after the first NTP fix fell outside a 2,000 ppm
/// interval. It falls well inside the unlearned bound now, and the clock
/// stays honest across a day of polls.
#[test]
fn ntp_polls_stay_inside_the_bound_at_the_measured_worst_drift() {
    let mut clock = WallClock::default();
    clock.fix(0, ntp(NOON, 25)).unwrap();

    let step = 1_026_000u64;
    for i in 1..=84u64 {
        let truth = i * step;
        let now = slow_by(2060, truth);
        if let Some(reading) = clock.read(now) {
            honest(reading, NOON + truth, &format!("poll {i}"));
            assert!(clock.fix(now, ntp(NOON + truth, 25)).is_ok(), "poll {i}");
        } else {
            assert_eq!(clock.fix(now, ntp(NOON + truth, 25)), Ok(Jump::First));
        }
    }
    assert!(!clock.state(slow_by(2060, 84 * step)).learned);
}

#[test]
fn the_rate_is_learned_from_owner_syncs_and_the_clock_holds_for_six_days() {
    let mut clock = WallClock::default();
    clock.fix(0, fix(NOON, 20, Origin::Owner)).unwrap();
    assert!(!clock.state(0).learned);

    // Owner fixes every 17 minutes for two hours: the baseline passes the
    // 50 ppm measurement after about 27 minutes.
    let step = 1_024_000;
    for i in 1..=7u64 {
        let truth = i * step;
        clock
            .fix(slow(truth), fix(NOON + truth, 20, Origin::Owner))
            .unwrap();
    }
    let state = clock.state(slow(7 * step));
    assert!(state.learned);
    assert!(
        (state.rate_ppb - 735_000).abs() < 1_000,
        "{}",
        state.rate_ppb
    );
    assert_eq!(state.rate_bound_ppb, LEARNED_FLOOR_PPM * 1000);

    // Six days with no source: the corrected clock is within a second of
    // the truth, and still valid with a bound under two minutes.
    let truth = 7 * step + 144 * u64::from(HOUR_MS);
    let reading = clock.read(slow(truth)).unwrap();
    let off = reading.unix_ms.abs_diff(NOON + truth);
    assert!(off < 1_000, "off by {off} ms");
    assert!(reading.error_ms <= MAX_ERROR_MS);

    // Seven days is past the learned floor's 6.9.
    let truth = 7 * step + 168 * u64::from(HOUR_MS);
    assert_eq!(clock.read(slow(truth)), None);
}

/// Two owners' phones 1.5 s apart, each well inside the other's interval:
/// measured across them, 12 hours would teach 35 ppm of the phones'
/// disagreement as the feeder's rate. The rate is measured between one
/// key's syncs only: the second key's sync re-anchors the clock and teaches
/// nothing, and the first key's next sync measures from its own baseline.
#[test]
fn the_rate_is_learned_from_one_owner_key_s_syncs_alone() {
    const OTHER: SyncKey = [0x42; 4];
    let mut clock = WallClock::default();
    clock.sync(0, sync(NOON), Origin::Owner, OWNER_KEY).unwrap();

    // The other phone, 1.5 s ahead, twelve hours on (the unlearned clock
    // expired at 6.6): the clock takes it, but learns nothing from it.
    let twelve = 12 * HOUR_MS;
    let ahead = NOON + u64::from(twelve) + 1_500;
    assert_eq!(
        clock.sync(twelve, sync(ahead), Origin::Owner, OTHER),
        Ok(Jump::First)
    );
    let state = clock.state(twelve);
    assert!(!state.learned, "nothing measured across two keys");
    assert_eq!(clock.read(twelve).unwrap().unix_ms, ahead, "re-anchored");

    // The first phone thirteen hours on, on time: measured from its own
    // baseline, the rate is the clock's true one, zero.
    let thirteen = 13 * HOUR_MS;
    clock
        .sync(
            thirteen,
            sync(NOON + u64::from(thirteen)),
            Origin::Owner,
            OWNER_KEY,
        )
        .unwrap();
    let state = clock.state(thirteen);
    assert!(state.learned);
    assert!(state.rate_ppb.abs() < 50_000, "{}", state.rate_ppb);
}

/// A key that never synced before teaches nothing until the rate is
/// forgotten; then the next owner's sync, whoever's, is a new baseline.
#[test]
fn another_key_starts_a_baseline_only_once_the_rate_is_forgotten() {
    const OTHER: SyncKey = [0x42; 4];
    let mut clock = WallClock::default();
    clock.sync(0, sync(NOON), Origin::Owner, OWNER_KEY).unwrap();
    for hours in [12u32, 24] {
        let now = hours * HOUR_MS;
        clock
            .sync(now, sync(NOON + u64::from(now)), Origin::Owner, OTHER)
            .unwrap();
        assert!(!clock.state(now).learned, "+{hours} h");
    }

    // An owner's sync far outside the interval: the rate and the baseline
    // go, and the other key's later syncs build their own.
    let off = 25 * HOUR_MS;
    clock
        .sync(
            off,
            sync(NOON + u64::from(off) + 3_600_000),
            Origin::Owner,
            OTHER,
        )
        .unwrap();
    for hours in [26u32, 38] {
        let now = hours * HOUR_MS;
        clock
            .sync(
                now,
                sync(NOON + u64::from(now) + 3_600_000),
                Origin::Owner,
                OTHER,
            )
            .unwrap();
    }
    assert!(clock.state(38 * HOUR_MS).learned);
}

/// The final review's scenario: after a cold boot an attacker's NTP answer
/// sets the clock 100 s behind, and an owner's sync six hours later falls
/// inside its interval. Measured from the attacker's fix that sync would
/// teach +4,630 ppm with a 200 ppm bound, and 20 h on the clock would be
/// 333 s off while claiming about 15 s. A free NTP set is no baseline: the
/// owner's sync becomes it, and the clock stays honest or expires.
#[test]
fn an_attackers_boot_fix_is_no_baseline_for_an_owners_rate() {
    let mut clock = WallClock::default();
    clock.fix(0, ntp(NOON - 100_000, 20)).unwrap();

    let six = 6 * HOUR_MS;
    clock
        .sync(six, sync(NOON + u64::from(six)), Origin::Owner, OWNER_KEY)
        .unwrap();
    assert!(!clock.state(six).learned);

    for hours in [6u32, 12, 20] {
        let now = six + hours * HOUR_MS;
        if let Some(reading) = clock.read(now) {
            honest(reading, NOON + u64::from(now), &format!("+{hours} h"));
        }
    }
}

#[test]
fn ntp_alone_never_learns_a_rate_and_an_owner_sync_does() {
    let mut clock = WallClock::default();
    clock.fix(0, ntp(NOON, 20)).unwrap();

    // Answers every 17 minutes keep the reading on time but teach nothing.
    // The reading follows NTP while the trusted estimate runs 735 ppm slow,
    // and the gap counts in the error, so it expires after about 5.8 h; the
    // next answer re-sets it inside the old interval, again teaching
    // nothing, since it only had to fall inside that interval.
    let step = 1_024_000u64;
    let mut resets = 0;
    for i in 1..=80u64 {
        let truth = i * step;
        let now = slow(truth);
        if let Some(reading) = clock.read(now) {
            honest(reading, NOON + truth, &format!("poll {i}"));
        }
        if clock.fix(now, ntp(NOON + truth, 20)) == Ok(Jump::First) {
            resets += 1;
        }
        assert!(!clock.state(now).learned, "poll {i}");
    }
    assert!((3..=4).contains(&resets), "{resets} re-sets");

    // An owner's sync is the baseline, and the next one measures the rate.
    let first = 80 * step + 60_000;
    clock
        .fix(slow(first), fix(NOON + first, 20, Origin::Owner))
        .unwrap();
    assert!(!clock.state(slow(first)).learned);
    let truth = first + 2 * u64::from(HOUR_MS);
    clock
        .fix(slow(truth), fix(NOON + truth, 20, Origin::Owner))
        .unwrap();
    let state = clock.state(slow(truth));
    assert!(state.learned);
    assert!(
        (state.rate_ppb - 735_000).abs() < 1_000,
        "{}",
        state.rate_ppb
    );
}

#[test]
fn an_owner_sync_that_disagrees_forgets_the_rate() {
    let mut clock = WallClock::default();
    clock.fix(0, fix(NOON, 20, Origin::Owner)).unwrap();
    clock
        .fix(slow(7_200_000), fix(NOON + 7_200_000, 20, Origin::Owner))
        .unwrap();
    assert!(clock.state(slow(7_200_000)).learned);

    let now = slow(7_300_000);
    clock
        .fix(now, fix(NOON + 7_300_000 + 3_600_000, 20, Origin::Owner))
        .unwrap();
    let state = clock.state(now);
    assert!(!state.learned);
    assert_eq!(state.rate_bound_ppb, UNLEARNED_PPM * 1000);
}

#[test]
fn ntp_answers_that_keep_missing_the_clock_make_it_need_the_time() {
    let mut clock = WallClock::default();
    clock.fix(0, fix(NOON, 20, Origin::Owner)).unwrap();
    let off = NOON + 3_600_000;
    for n in 1..DISJOINT_LIMIT {
        assert_eq!(
            clock.fix(u32::from(n) * 1000, ntp(off, 20)),
            Err(SyncError::Bounded)
        );
        assert!(clock.read(u32::from(n) * 1000).is_some());
    }
    // A client's disagreement does not count towards it.
    assert_eq!(
        clock.fix(5_000, fix(off, 20, Origin::Client)),
        Err(SyncError::Bounded)
    );
    assert!(clock.read(5_000).is_some());
    assert_eq!(clock.fix(6_000, ntp(off, 20)), Err(SyncError::Bounded));
    assert_eq!(clock.read(6_000), None, "the clock needs the time");

    // The anchor still bounds NTP: the same answer again is refused, and
    // one inside the interval it missed sets the clock.
    assert_eq!(clock.fix(7_000, ntp(off, 20)), Err(SyncError::Bounded));
    assert_eq!(clock.read(7_000), None);
    assert_eq!(clock.fix(8_000, ntp(NOON + 8_000, 20)), Ok(Jump::First));
    assert_eq!(clock.read(8_000).unwrap().unix_ms, NOON + 8_000);
}

/// An attacker on the path answering a day ahead never sets the clock,
/// however many answers it sends: after `DISJOINT_LIMIT` of them the clock
/// needs the time, and only an owner's or a client's sync gives it.
#[test]
fn ntp_answers_a_day_ahead_never_set_an_anchored_clock() {
    const DAY_MS: u64 = 86_400_000;
    for origin in [Origin::Owner, Origin::Client] {
        let mut clock = WallClock::default();
        clock.fix(0, fix(NOON, 20, Origin::Owner)).unwrap();
        let mut now = 0;
        for n in 0..=4 * u32::from(DISJOINT_LIMIT) {
            now = (n + 1) * 1_000;
            let ahead = NOON + u64::from(now) + DAY_MS;
            assert_eq!(clock.fix(now, ntp(ahead, 20)), Err(SyncError::Bounded));
            let valid = n + 1 < u32::from(DISJOINT_LIMIT);
            assert_eq!(clock.read(now).is_some(), valid, "answer {n}");
        }

        now += 1_000;
        let truth = NOON + u64::from(now);
        assert_eq!(clock.fix(now, fix(truth, 20, origin)), Ok(Jump::First));
        assert_eq!(clock.read(now).unwrap().unix_ms, truth);
    }
}

/// After expiry NTP must still fall inside the old anchor's interval, grown
/// at the bound without the two-minute cap: each re-set moves the clock by
/// at most that interval, not to whatever answers, however many miss.
#[test]
fn an_expired_clock_is_re_set_by_ntp_only_inside_its_old_interval() {
    let mut clock = WallClock::default();
    clock.fix(0, fix(NOON, 20, Origin::Owner)).unwrap();

    // Seven hours on: invalid, the old interval ±126 s.
    let now = 7 * HOUR_MS;
    let truth = NOON + u64::from(now);
    assert_eq!(clock.read(now), None);
    for n in 1..DISJOINT_LIMIT {
        assert_eq!(
            clock.fix(now + u32::from(n), ntp(truth + 3_600_000, 20)),
            Err(SyncError::Bounded)
        );
    }
    assert_eq!(
        clock.fix(now + 10, ntp(truth + 100_000, 20)),
        Ok(Jump::First),
        "inside the grown interval"
    );
    assert_eq!(clock.read(now + 10).unwrap().unix_ms, truth + 100_000);

    // An attacker re-setting it at the edge every expiry walks it at most
    // one grown interval each time: about 2 minutes per 6.6 hours, not a
    // jump to anything.
    let mut clock = WallClock::default();
    clock.fix(0, fix(NOON, 20, Origin::Owner)).unwrap();
    let mut now = 0u32;
    for _ in 0..4 {
        while clock.read(now).is_some() {
            now += HOUR_MS;
        }
        assert_eq!(
            clock.fix(now, ntp(NOON + u64::from(now) + 3_600_000, 20)),
            Err(SyncError::Bounded)
        );
        // The furthest ahead it accepts, found on copies so the probes
        // count no misses.
        let mut edge = NOON + u64::from(now) + 3_600_000;
        while {
            let mut probe = clock;
            probe.fix(now, ntp(edge, 20)).is_err()
        } {
            edge -= 1_000;
        }
        assert_eq!(clock.fix(now, ntp(edge, 20)), Ok(Jump::First));
    }
    // Each re-set moves it by at most the interval grown since the anchor
    // (a little over 2 minutes, expiry checked hourly), plus what the
    // taught rate carries: minutes over four expiries, never the hour.
    let walked = clock.read(now).unwrap().unix_ms - (NOON + u64::from(now));
    assert!(walked <= 4 * 130_000, "walked {walked} ms");

    // Misses in a row never free it: the old interval still bounds NTP.
    let mut clock = WallClock::default();
    clock.fix(0, fix(NOON, 20, Origin::Owner)).unwrap();
    let now = 7 * HOUR_MS;
    let far = NOON + u64::from(now) + 3_600_000;
    for _ in 0..=DISJOINT_LIMIT {
        assert_eq!(clock.fix(now, ntp(far, 20)), Err(SyncError::Bounded));
    }
    assert_eq!(
        clock.fix(now, ntp(NOON + u64::from(now), 20)),
        Ok(Jump::First)
    );
}

#[test]
fn bad_syncs_are_refused_and_leave_the_clock() {
    let mut clock = WallClock::default();
    clock.sync(0, sync(NOON), Origin::Owner, OWNER_KEY).unwrap();
    let cases = [
        (
            Sync {
                unix_ms: MIN_UNIX_MS - 1,
                ..sync(NOON)
            },
            SyncError::Time,
        ),
        (
            Sync {
                offset_s: 15 * 3600,
                ..sync(NOON)
            },
            SyncError::Offset,
        ),
        (
            Sync {
                next_offset_s: -13 * 3600,
                ..sync(NOON)
            },
            SyncError::Offset,
        ),
        (
            Sync {
                next_change_s: NOON / 1000,
                ..sync(NOON)
            },
            SyncError::NextChange,
        ),
    ];
    for (bad, error) in cases {
        assert_eq!(clock.sync(1_000, bad, Origin::Owner, OWNER_KEY), Err(error));
    }
    assert_eq!(
        clock.fix(1_000, ntp(MIN_UNIX_MS - 1, 1)),
        Err(SyncError::Time)
    );
    assert_eq!(clock.read(1_000).unwrap().unix_ms, NOON + 1_000);
    assert_eq!(clock.syncs(), 1);

    // No announced change is allowed.
    assert!(
        clock
            .sync(
                2_000,
                Sync {
                    next_change_s: 0,
                    ..sync(NOON + 2_000)
                },
                Origin::Owner,
                OWNER_KEY,
            )
            .is_ok()
    );
}

/// An owner's sync at uptime 0, saved a minute later at RTC second 1000.
fn saved() -> [u32; CARRY_WORDS] {
    let mut clock = WallClock::default();
    clock.sync(0, sync(NOON), Origin::Owner, OWNER_KEY).unwrap();
    clock.carry(60_000, 1000).expect("valid a minute on")
}

#[test]
fn a_warm_reset_resumes_the_clock_with_the_rtc_gap_in_its_error() {
    let mut clock = WallClock::default();

    // 12 RTC seconds later, 3 s into the new boot. The saved error is the
    // sync's second plus 5000 ppm of a minute; the gap adds the RTC's
    // second of resolution and 1 % of 13 s.
    let resumed = clock.resume(3_000, &saved(), Some(1012), Restart::Warm);
    assert_eq!(
        resumed,
        Ok(Resumed {
            gap_s: 12,
            error_ms: 1_300 + 1_000 + 130,
            contested: false,
        })
    );

    let reading = clock.read(3_000).unwrap();
    assert_eq!(reading.unix_ms, NOON + 72_000);
    assert_eq!(reading.offset_s, Some(MDT));
    for truth in [NOON + 71_000, NOON + 73_000] {
        honest(reading, truth, "either end of the RTC's second");
    }

    // Carried on by uptime from there, unlearned, with no sync counted. The
    // owner's sync is still the last fix, 132 s old a minute later.
    let state = clock.state(63_000);
    assert_eq!(state.reading.unwrap().error_ms, 2_430 + 300);
    assert!(!state.learned);
    assert_eq!(state.syncs, 0);
    assert_eq!(state.last, Some((Origin::Owner, 132_000)));
}

#[test]
fn a_resumed_clock_keeps_the_trusted_interval_and_learns_no_rate_from_it() {
    let mut clock = WallClock::default();
    clock
        .resume(3_000, &saved(), Some(1012), Restart::Warm)
        .unwrap();

    // Still trusted: NTP an hour off is refused, inside it only narrows.
    assert_eq!(
        clock.fix(4_000, ntp(NOON + 73_000 + 3_600_000, 20)),
        Err(SyncError::Bounded)
    );
    assert_eq!(
        clock.fix(4_000, ntp(NOON + 73_500, 20)),
        Ok(Jump::Step(500))
    );
    assert!(clock.read(4_000).unwrap().error_ms >= 2_430);

    // The resumed anchor is no baseline: one owner sync after it teaches
    // nothing, however far apart.
    clock
        .fix(4_000 + 7_200_000, fix(NOON + 7_273_500, 20, Origin::Owner))
        .unwrap();
    assert!(!clock.state(4_000 + 7_200_000).learned);
}

#[test]
fn nothing_resumes_after_a_power_cut() {
    let mut clock = WallClock::default();
    let record = saved();

    assert_eq!(
        clock.resume(3_000, &record, Some(1012), Restart::Cold),
        Err(NoResume::Cold)
    );
    // A power cut the reset cause missed: the RTC restarted at 0.
    assert_eq!(
        clock.resume(3_000, &record, Some(4), Restart::Warm),
        Err(NoResume::Backwards)
    );
    // Memory a power-on left.
    assert_eq!(
        clock.resume(3_000, &[0; CARRY_WORDS], Some(1012), Restart::Warm),
        Err(NoResume::Empty)
    );
    assert_eq!(
        clock.resume(
            3_000,
            &[0xa5a5_a5a5; CARRY_WORDS],
            Some(1012),
            Restart::Warm
        ),
        Err(NoResume::Empty)
    );
    assert_eq!(clock.read(3_000), None);
}

#[test]
fn a_stale_or_corrupt_record_is_not_resumed() {
    let mut clock = WallClock::default();
    let record = saved();

    let gap = 1000 + CARRY_MAX_S + 1;
    assert_eq!(
        clock.resume(3_000, &record, Some(gap), Restart::Warm),
        Err(NoResume::Stale)
    );
    assert_eq!(
        clock.resume(3_000, &record, None, Restart::Warm),
        Err(NoResume::Rtc)
    );

    // Any flipped bit fails the check, the check word's included.
    for word in 1..CARRY_WORDS {
        let mut bad = record;
        bad[word] ^= 1 << (word % 32);
        assert_eq!(
            clock.resume(3_000, &bad, Some(1012), Restart::Warm),
            Err(NoResume::Corrupt),
            "word {word}"
        );
    }
    assert_eq!(clock.read(3_000), None);
}

/// A version-1 record came from a build that ran the clock on the
/// scheduler's tick, which flash erases stop: after a download it read 12 s
/// behind while claiming 5.5 s, so a well-formed one is still refused.
#[test]
fn a_record_from_a_tick_counted_build_is_not_resumed() {
    let mut clock = WallClock::default();
    let mut old = saved();
    assert_eq!(old[1] & 0xffff, 2, "the record's version");
    old[1] = (old[1] & !0xffff) | 1;
    let mut bytes = [0; (CARRY_WORDS - 1) * 4];
    for (chunk, word) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(old) {
        *chunk = word.to_le_bytes();
    }
    old[CARRY_WORDS - 1] = wellfed_core::store::slot::crc32(&bytes);

    assert_eq!(
        clock.resume(3_000, &old, Some(1012), Restart::Warm),
        Err(NoResume::Corrupt)
    );
    assert_eq!(clock.read(3_000), None);
}

/// Forgetting drops the time and the rate but keeps the zone's offsets.
#[test]
fn a_forgotten_clock_needs_the_time_and_keeps_its_offsets() {
    let mut clock = WallClock::default();
    clock.sync(0, sync(NOON), Origin::Owner, OWNER_KEY).unwrap();

    clock.forget();
    assert_eq!(clock.read(1_000), None);
    assert_eq!(clock.carry(1_000, 1000), None);

    // The next fix of any origin sets it, as a first fix.
    assert_eq!(clock.fix(2_000, ntp(NOON + 2_000, 20)), Ok(Jump::First));
    assert_eq!(clock.read(2_000).unwrap().offset_s, Some(MDT));
}

#[test]
fn a_record_too_close_to_expiry_is_not_resumed() {
    let mut clock = WallClock::default();
    clock.sync(0, sync(NOON), Origin::Owner, OWNER_KEY).unwrap();
    // 118.5 s of error at 23 500 s; the gap's second and more pass 120 s.
    let record = clock.carry(23_500_000, 50_000).unwrap();
    assert!(clock.carry(23_900_000, 50_000).is_none(), "invalid by then");

    let mut fresh = WallClock::default();
    assert_eq!(
        fresh.resume(1_000, &record, Some(50_100), Restart::Warm),
        Err(NoResume::Expired)
    );
    assert_eq!(fresh.read(1_000), None);
}

#[test]
fn an_ntp_last_fix_survives_the_reset_as_ntp() {
    let mut clock = WallClock::default();
    clock.fix(0, ntp(NOON, 20)).unwrap();
    let record = clock.carry(10_000, 500).unwrap();

    let mut resumed = WallClock::default();
    resumed
        .resume(2_000, &record, Some(505), Restart::Warm)
        .unwrap();
    assert_eq!(resumed.state(2_000).last, Some((Origin::Ntp, 15_000)));
}

#[test]
fn a_clock_with_the_time_is_not_overwritten_by_a_record() {
    let mut clock = WallClock::default();
    clock.fix(0, ntp(NOON + 3_600_000, 20)).unwrap();
    assert_eq!(
        clock.resume(0, &saved(), Some(1012), Restart::Warm),
        Err(NoResume::Set)
    );
    assert_eq!(clock.read(0).unwrap().unix_ms, NOON + 3_600_000);
}

#[test]
fn occurrences_run_once_and_late_ones_are_missed() {
    let due = NOON;
    assert_eq!(occurrence(due, None, due - 1), Occurrence::Later);
    assert_eq!(occurrence(due, None, due), Occurrence::Run);
    assert_eq!(
        occurrence(due, Some(due - 1), due + MISSED_GRACE_MS),
        Occurrence::Run
    );
    // A forward jump or a restart past the grace window.
    assert_eq!(
        occurrence(due, Some(due - 1), due + MISSED_GRACE_MS + 1),
        Occurrence::Missed
    );
    // A backward jump re-enters evaluated time: never twice.
    assert_eq!(
        occurrence(due, Some(due + 5_000), due - 3_600_000),
        Occurrence::Evaluated
    );
    assert_eq!(occurrence(due, Some(due), due + 1), Occurrence::Evaluated);
}

/// A contested clock is carried across a warm reset with its anchor, so
/// the reset hands the next NTP answer nothing: one a day ahead is still
/// refused, the clock still needs the time, and an honest answer or an
/// owner's sync sets it.
#[test]
fn a_contested_clock_keeps_its_bound_across_a_warm_reset() {
    const DAY_MS: u64 = 86_400_000;
    let contested = || {
        let mut clock = WallClock::default();
        clock.fix(0, fix(NOON, 20, Origin::Owner)).unwrap();
        for n in 1..=u32::from(DISJOINT_LIMIT) {
            let now = n * 1_000;
            let ahead = NOON + u64::from(now) + DAY_MS;
            assert_eq!(clock.fix(now, ntp(ahead, 20)), Err(SyncError::Bounded));
        }
        assert_eq!(clock.read(4_000), None);
        clock
            .carry(10_000, 1_000)
            .expect("a contested clock is carried")
    };

    // Ten RTC seconds on, two seconds into the new boot.
    let mut clock = WallClock::default();
    let resumed = clock.resume(2_000, &contested(), Some(1_010), Restart::Warm);
    assert!(resumed.is_ok_and(|resumed| resumed.contested));
    assert_eq!(clock.read(2_000), None, "still needs the time");
    let truth = NOON + 20_000;
    assert_eq!(
        clock.fix(2_000, ntp(truth + DAY_MS, 20)),
        Err(SyncError::Bounded)
    );
    // Carried again, it stays contested.
    let again = clock.carry(3_000, 1_011).expect("still carried");
    let mut next = WallClock::default();
    assert!(
        next.resume(1_000, &again, Some(1_012), Restart::Warm)
            .is_ok_and(|resumed| resumed.contested)
    );

    assert_eq!(clock.fix(2_000, ntp(truth, 20)), Ok(Jump::First));
    assert!(clock.read(2_000).is_some());
    let mut clock = WallClock::default();
    clock
        .resume(2_000, &contested(), Some(1_010), Restart::Warm)
        .unwrap();
    assert_eq!(
        clock.fix(2_000, fix(truth + DAY_MS, 20, Origin::Owner)),
        Ok(Jump::First)
    );
}

/// A learned rate gone wrong (the uptime clock's drift moved past the
/// learned bound) makes honest NTP answers miss. Once three have, the rate
/// is forgotten and the bound grows at the unlearned rate, so the next
/// honest answer sets the clock without a client's sync.
#[test]
fn a_wrong_learned_rate_is_forgotten_and_honest_ntp_sets_the_clock() {
    let mut clock = WallClock::default();
    clock.fix(0, fix(NOON, 20, Origin::Owner)).unwrap();
    let step = 1_024_000;
    for i in 1..=7u64 {
        let truth = i * step;
        clock
            .fix(slow(truth), fix(NOON + truth, 20, Origin::Owner))
            .unwrap();
    }
    assert!(clock.state(slow(7 * step)).learned);

    // The drift moves from 735 to 3,735 ppm slow: three hours on, the
    // learned clock is about 32 s fast, outside its 200 ppm bound.
    let base = 7 * step;
    let uptime = |truth: u64| slow(base) + slow_by(3_735, truth - base);
    let mut truth = base + 3 * u64::from(HOUR_MS);
    for _ in 0..DISJOINT_LIMIT {
        assert_eq!(
            clock.fix(uptime(truth), ntp(NOON + truth, 20)),
            Err(SyncError::Bounded)
        );
        truth += 60_000;
    }
    let state = clock.state(uptime(truth));
    assert!(state.reading.is_none());
    assert!(!state.learned, "the rate is forgotten");

    assert_eq!(
        clock.fix(uptime(truth), ntp(NOON + truth, 20)),
        Ok(Jump::First)
    );
    honest(clock.read(uptime(truth)).unwrap(), NOON + truth, "re-set");
}

/// A contested clock's error grows past the two minutes a valid one may
/// claim, and its record carries it: 24 days on, about 2.9 hours, still
/// refusing an answer a day ahead after the reset. Past half the uptime
/// clock's range no projection holds, so nothing is carried, and a record
/// claiming more than the carry could grow is refused as corrupt.
#[test]
fn a_contested_record_carries_a_grown_error_up_to_its_limit() {
    const DAY_MS: u64 = 86_400_000;
    let mut clock = WallClock::default();
    clock.fix(0, fix(NOON, 20, Origin::Owner)).unwrap();
    for n in 1..=u32::from(DISJOINT_LIMIT) {
        let now = n * 1_000;
        let ahead = NOON + u64::from(now) + DAY_MS;
        assert_eq!(clock.fix(now, ntp(ahead, 20)), Err(SyncError::Bounded));
    }

    let later = 24 * 24 * HOUR_MS;
    let record = clock.carry(later, 5_000).expect("carried at 24 days");
    let mut resumed = WallClock::default();
    let result = resumed.resume(1_000, &record, Some(5_010), Restart::Warm);
    let resumed_error = result.map(|resumed| (resumed.contested, resumed.error_ms));
    assert!(
        resumed_error.is_ok_and(|(contested, error_ms)| contested && error_ms > 10_000_000),
        "{resumed_error:?}"
    );
    let truth = NOON + u64::from(later) + 11_000;
    assert_eq!(
        resumed.fix(1_000, ntp(truth + DAY_MS, 20)),
        Err(SyncError::Bounded)
    );
    assert_eq!(
        clock.carry(25 * 24 * HOUR_MS, 5_000),
        None,
        "past half the range"
    );

    // The same record claiming another error, with a valid check: within
    // the most a carry grows it resumes, past it it is refused.
    let claiming = |error_us: u64| {
        let mut forged = record;
        forged[5] = u32::try_from(error_us & 0xffff_ffff).unwrap();
        forged[6] = u32::try_from(error_us >> 32).unwrap();
        let bytes: Vec<u8> = forged[..CARRY_WORDS - 1]
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect();
        forged[CARRY_WORDS - 1] = wellfed_core::store::slot::crc32(&bytes);
        WallClock::default().resume(1_000, &forged, Some(5_010), Restart::Warm)
    };
    assert!(claiming(10_000_000_000).is_ok());
    assert_eq!(claiming(20_000_000_000), Err(NoResume::Corrupt));
}

/// A clock with no fix for longer than the uptime count's range stays
/// invalid as the count comes round, learned or not, with its maintenance
/// run hourly as the loop runs it each pass; NTP keeps its bound while the
/// anchor can still be placed, and a fix sets the clock again.
#[test]
fn an_expired_clock_stays_invalid_across_uptime_wraps() {
    for learned in [false, true] {
        let mut clock = WallClock::default();
        clock.fix(0, fix(NOON, 20, Origin::Owner)).unwrap();
        if learned {
            for i in 1..=7u32 {
                let now = i * 1_024_000;
                let truth = NOON + u64::from(now);
                clock.fix(now, fix(truth, 20, Origin::Owner)).unwrap();
            }
            assert!(clock.state(7_168_000).learned);
        }

        let last_fix = if learned { 7_168_000 } else { 0 };
        // Three wraps of the 32-bit millisecond count, an hour at a time.
        let hours = 3 * (1u64 << 32) / u64::from(HOUR_MS) + 2;
        for hour in 8..=hours {
            let truth = hour * u64::from(HOUR_MS);
            let now = u32::try_from(truth % (1 << 32)).unwrap();
            clock.retire(now);
            if hour > 24 * 8 {
                assert_eq!(clock.read(now), None, "hour {hour}, learned {learned}");
            }
            // The last fix's age is its true age, or none at all.
            if let Some((_, age)) = clock.state(now).last {
                assert_eq!(u64::from(age), truth - last_fix, "hour {hour}");
            }
        }

        // Its anchor retired, NTP sets it as a first fix again.
        let truth = hours * u64::from(HOUR_MS) + 1_000;
        let now = u32::try_from(truth % (1 << 32)).unwrap();
        assert_eq!(clock.fix(now, ntp(NOON + truth, 25)), Ok(Jump::First));
        honest(clock.read(now).unwrap(), NOON + truth, "after the fix");
    }
}

/// While the anchor can still be placed it stays NTP's bound: an expired
/// clock is not re-set to any time an answer names.
#[test]
fn retiring_keeps_an_expired_anchor_bounding_ntp_while_it_can_be_placed() {
    let mut clock = WallClock::default();
    clock.fix(0, fix(NOON, 20, Origin::Owner)).unwrap();
    let now = 10 * 24 * HOUR_MS;
    clock.retire(now);
    assert_eq!(clock.read(now), None);
    let far = NOON + u64::from(now) + 30 * 24 * u64::from(HOUR_MS);
    assert_eq!(clock.fix(now, ntp(far, 25)), Err(SyncError::Bounded));
}

/// An owner's sync read late disagrees with the clock: it moves the clock,
/// but it is no baseline, so the rate later syncs measure carries none of
/// its error.
#[test]
fn a_late_owner_sync_teaches_no_rate() {
    let mut clock = WallClock::default();
    clock.fix(0, fix(NOON, 20, Origin::Owner)).unwrap();

    // Stamped 30 s before the feeder read it.
    let late = HOUR_MS;
    let stale = NOON + u64::from(late) - 30_000;
    assert!(matches!(
        clock.sync(late, sync(stale), Origin::Owner, OWNER_KEY),
        Ok(Jump::Jumped(_))
    ));

    // A true sync after the clock expired: no rate from the late one.
    let later = 13 * HOUR_MS;
    let owner = |now: u32| fix(NOON + u64::from(now), 20, Origin::Owner);
    clock.fix(later, owner(later)).unwrap();
    assert!(!clock.state(later).learned);

    // True syncs from there teach the true rate, and the clock stays honest.
    for i in 1..=7u32 {
        let now = later + i * 1_024_000;
        clock.fix(now, owner(now)).unwrap();
    }
    let state = clock.state(later + 7 * 1_024_000);
    assert!(
        state.learned && state.rate_ppb.unsigned_abs() < 1_000,
        "{state:?}"
    );
    let day = later + 7 * 1_024_000 + 24 * HOUR_MS;
    honest(clock.read(day).unwrap(), NOON + u64::from(day), "a day on");
}
