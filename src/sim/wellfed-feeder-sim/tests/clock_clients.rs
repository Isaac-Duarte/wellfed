//! The clients' rule for syncing the clock ([`wellfed_client::time`])
//! against the simulated feeder's own clock, in virtual time. A phone some
//! seconds off the truth must never fight a time server, and must never
//! re-anchor a clock it disagrees with.

use std::time::Duration;

use jiff::Timestamp;
use wellfed_client::time::{AutoSync, ClockCheck};
use wellfed_core::clock::wall::{Fix, Origin, State, Sync};
use wellfed_feeder::feeder::Platform;
use wellfed_feeder_sim::banks::Banks;
use wellfed_feeder_sim::flash::Flash;
use wellfed_feeder_sim::sim::Sim;
use wellfed_model::{ClientRole, Generation, NtpServer, TimeSource, TimeView};

const MINUTE_MS: u32 = 60_000;
/// 2026-10-28T00:00:00Z.
const START: u64 = 1_793_145_600_000;
/// An owner's key.
const OWNER: [u8; 4] = [0x21; 4];

fn open() -> Sim {
    let flash = Flash::memory();
    let banks = Banks::new();
    let random = Box::new(|| Some([0x5a; 32]));
    Sim::open(&flash, &banks, None, random, 0, 1, None).unwrap()
}

/// The time view a client reads, from the feeder's clock state.
fn view(state: State, ntp: NtpServer) -> TimeView {
    TimeView {
        generation: Generation(1),
        writable: true,
        zone: None,
        ntp,
        clock_ms: state.reading.map(|reading| reading.unix_ms),
        error_ms: state
            .reading
            .map(|reading| u32::try_from(reading.error_ms).unwrap_or(u32::MAX)),
        offset_s: None,
        rate_ppb: state.rate_ppb,
        rate_bound_ppb: state.rate_bound_ppb,
        learned: state.learned,
        syncs: state.syncs,
        last: state.last.map(|(origin, _)| match origin {
            Origin::Owner => TimeSource::Owner,
            Origin::Client => TimeSource::Client,
            Origin::Ntp => TimeSource::Ntp,
        }),
        last_age_s: None,
        ntp_last: None,
    }
}

fn sync(unix_ms: u64) -> Sync {
    Sync {
        unix_ms,
        offset_s: 0,
        next_change_s: 0,
        next_offset_s: 0,
    }
}

fn at(ms: u64) -> Timestamp {
    Timestamp::from_millisecond(i64::try_from(ms).unwrap()).unwrap()
}

/// What a day of one client against the feeder came to.
#[derive(Debug, Default)]
struct Day {
    /// Syncs the client sent.
    syncs: usize,
    /// Reads where the client said it disagrees with the feeder.
    disagreed: usize,
    /// NTP answers refused as outside the trusted interval.
    misses: usize,
    /// Minutes the clock read invalid.
    invalid_minutes: u32,
}

/// `hours` of the feeder in virtual time, its clock set at the start. With
/// `ntp`, a time server answers on the truth every 17 minutes (each minute
/// while the clock needs the time, as the exchange asks within 64 s). An
/// owner's phone `phone_off_ms` off the truth reads the time view each 15
/// minutes and syncs as the rule says.
fn day(ntp: NtpServer, phone_off_ms: i64, hours: u32) -> Day {
    let mut sim = open();
    let mut memo = AutoSync::default();
    let mut day = Day::default();
    let truth = |now: u32| START + u64::from(now);
    let phone = |now: u32| truth(now).saturating_add_signed(phone_off_ms);
    let asks = ntp != NtpServer::Off;

    sim.platform()
        .time_sync(0, sync(START), Origin::Owner, OWNER)
        .unwrap();
    let mut last_poll = 0;
    for minute in 1..=hours * 60 {
        let now = minute * MINUTE_MS;
        let valid = sim.platform().clock_state(now).reading.is_some();
        if !valid {
            day.invalid_minutes += 1;
        }
        if asks && (!valid || now - last_poll >= 17 * MINUTE_MS) {
            last_poll = now;
            let fix = Fix {
                unix_ms: truth(now),
                error_ms: 25,
                origin: Origin::Ntp,
            };
            if sim.platform().time_fix(now, fix).is_err() {
                day.misses += 1;
            }
        }
        if minute % 15 == 0 {
            let state = sim.platform().clock_state(now);
            let read = view(state, ntp);
            let spacing = Duration::from_secs(30 * 60);
            match memo.read(&read, ClientRole::Owner, at(phone(now)), spacing) {
                ClockCheck::Sync => {
                    memo.sent(at(phone(now)));
                    day.syncs += 1;
                    let _ = sim
                        .platform()
                        .time_sync(now, sync(phone(now)), Origin::Owner, OWNER);
                }
                ClockCheck::Disagrees { .. } => day.disagreed += 1,
                ClockCheck::Leave => {}
            }
        }
    }
    day
}

/// A phone 30 s off, against a feeder its time server keeps: the phone
/// leaves the clock to NTP, so no answer is ever pushed outside the
/// interval and the clock is never contested. It reads invalid only in the
/// minute each expiry takes NTP to re-set it.
#[test]
fn a_phone_off_by_30_s_never_fights_the_time_server() {
    let day = day(NtpServer::Gateway { port: 123 }, 30_000, 30);
    assert_eq!(day.syncs, 0, "{day:?}");
    assert_eq!(day.misses, 0, "{day:?}");
    assert!(day.invalid_minutes <= 6, "{day:?}");
}

/// The same phone with no time server: the feeder's error starts at a
/// second, so the phone is off by more than it allows and syncs nothing by
/// itself; once the error has grown past its 30 s it agrees, and syncs at
/// the 60 s point, re-anchoring there.
#[test]
fn with_no_time_server_a_phone_syncs_only_once_it_agrees() {
    let day = day(NtpServer::Off, 30_000, 30);
    assert!(day.disagreed > 0, "{day:?}");
    assert!(day.syncs > 0, "{day:?}");
    assert_eq!(day.invalid_minutes, 0, "{day:?}");
}

/// A phone further off than the feeder's error ever grows (3 minutes, past
/// the 2-minute limit) never syncs a valid clock by itself, so it can't
/// teach the feeder its own error; the clock expires instead, and then the
/// phone sets it, as any client may a clock that needs the time.
#[test]
fn a_phone_off_by_more_than_the_error_never_syncs_a_valid_clock() {
    let mut sim = open();
    let mut memo = AutoSync::default();
    let off: u64 = 3 * u64::from(MINUTE_MS);
    sim.platform()
        .time_sync(0, sync(START), Origin::Owner, OWNER)
        .unwrap();

    let mut checks = Vec::new();
    for quarter in 1..=4 * 8 {
        let now = quarter * 15 * MINUTE_MS;
        let state = sim.platform().clock_state(now);
        let read = view(state, NtpServer::Off);
        let check = memo.read(
            &read,
            ClientRole::Owner,
            at(START + u64::from(now) + off),
            Duration::from_secs(30 * 60),
        );
        if read.clock_ms.is_some() {
            assert!(
                matches!(check, ClockCheck::Disagrees { .. }),
                "+{} min: {check:?}",
                now / MINUTE_MS
            );
        }
        checks.push(check);
    }
    assert!(
        checks.contains(&ClockCheck::Sync),
        "the expired clock is set: {checks:?}"
    );
}
