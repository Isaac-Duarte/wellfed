//! The lifetime totals store: when it saves (boot, interval, feeds,
//! restarts), how its early tokens bound the wear, and never beside a feed.

use std::cell::Cell;

use wellfed_device_runtime::writer::{self as writer, ErrorStage};
use wellfed_feeder::stats::{EARLY_SAVES, FEED_SAVE_MS, RESTART_SETTLED_MS, SAVE_MS, Totals};
use wellfed_feeder::store::stats::StatsStatus;
use wellfed_feeder_runtime::stats;
use wellfed_feeder_runtime::writer::{FeederRecord, Record, Writer};
use wellfed_hal::flash::peer::MARKER_REGISTER;
use wellfed_hal::flash_lock::provider::Provider;
use wellfed_hal::interlock::INTERLOCK;
use wellfed_hal::sdk::{self, fake};
use wellfed_hal::{stopped, supervision};
use wellfed_proto::api::{Counts, Resets};

use crate::support::{
    ARMED, ATTEMPT_SEEN, Armed, PROGRAM_FAILS, boot, open_all, saved, start, step,
};

/// The stats store's sector pair, as the fake holds it.
fn stats_sectors() -> Vec<u8> {
    let first = wellfed_hal::board::storage::STATS.first as usize * 0x1000;
    fake::with(|h| h.storage[first..first + 0x2000].to_vec())
}

/// Step `n` times with nothing written to the stats store.
fn idle(writer: &mut Writer<Armed>, provider: &mut Provider, n: usize) {
    let before = stats_sectors();
    for _ in 0..n {
        assert_eq!(step(writer, provider), None);
    }
    assert_eq!(stats_sectors(), before, "no save");
}

/// The stats store's erases so far, both sectors.
fn stats_erases() -> u32 {
    fake::with(|h| h.storage_erases[32] + h.storage_erases[33])
}

/// The totals load at boot with this boot's reset counted and are saved at
/// once inside a marked attempt (an early save, taking a token), then every
/// `SAVE_MS` of uptime and no more often (settled, giving one back), each
/// save carrying this boot's counts and erases, its own erase included;
/// the next boot reads them back.
#[test]
fn lifetime_totals_count_the_boot_and_save_at_a_bounded_rate() {
    let (_guard, mut writer, mut provider) = start();
    fake::with(|h| h.boot_reason = 1 << 17);
    assert_eq!(
        open_all(&mut writer, &mut provider),
        Some(Record::Product(FeederRecord::StatsOpen {
            status: StatsStatus::Empty,
            generation: 0,
            early: 0,
            missed: false,
        }))
    );
    let mut base = Totals::ZERO;
    base.count_boot(wellfed_proto::api::ResetCause::Watchdog);
    assert_eq!(stats::totals(), Some(base), "published at once");
    ATTEMPT_SEEN.with(|s| s.set(false));
    assert_eq!(step(&mut writer, &mut provider), saved(1, 1));
    assert!(ATTEMPT_SEEN.with(Cell::get));
    assert_eq!(sdk::backup_read(MARKER_REGISTER), 0);

    let counts = Counts {
        uptime_s: 3_600,
        presses: 3,
        ..Counts::default()
    };
    stats::hand_counts(counts);
    idle(&mut writer, &mut provider, 3);
    sdk::delay_ms(SAVE_MS - 1);
    idle(&mut writer, &mut provider, 1);
    sdk::delay_ms(1);
    assert_eq!(step(&mut writer, &mut provider), saved(2, 0), "settled");
    assert_eq!(stats::totals(), Some(base), "the base stays this boot's");

    // The rate: nothing until a full interval later, however much changed.
    stats::hand_counts(Counts {
        presses: 4,
        ..counts
    });
    idle(&mut writer, &mut provider, 5);
    sdk::delay_ms(SAVE_MS - 1);
    idle(&mut writer, &mut provider, 5);
    sdk::delay_ms(1);
    assert_eq!(step(&mut writer, &mut provider), saved(3, 0));
    let erases = |sector: usize| fake::with(|h| h.storage_erases[sector]);
    assert_eq!((erases(32), erases(33)), (2, 1), "one erase per save");

    // The next boot, a power-on, adds itself to what was saved.
    fake::with(|h| h.boot_reason = 0);
    let (_, open, save) = boot(&mut provider);
    assert_eq!(
        open,
        Some(Record::Product(FeederRecord::StatsOpen {
            status: StatsStatus::Loaded,
            generation: 3,
            early: 0,
            missed: false,
        }))
    );
    assert_eq!(save, saved(4, 1));
    let totals = stats::totals().expect("the stored totals");
    assert_eq!(totals.counts.presses, 4);
    assert_eq!(totals.counts.uptime_s, 3_600);
    assert_eq!(
        totals.resets,
        Resets {
            power_on: 1,
            watchdog: 1,
            ..Resets::default()
        }
    );
    assert_eq!(
        (totals.storage[32], totals.storage[33]),
        (2, 1),
        "each save counts its own erase"
    );

    let mut text = String::new();
    writer::describe(saved(2, 0).unwrap(), &mut text).unwrap();
    assert_eq!(text, "stats saved generation=2 early=0");
}

/// A watchdog reset loop whose boots end within seconds still counts each
/// reset while early tokens last; the save that takes the last sets the
/// sticky flag, later short boots save nothing, and each settled save of a
/// boot that lasts gives back one token.
#[test]
fn short_boots_count_their_resets_while_tokens_last() {
    let (_guard, _, mut provider) = start();
    fake::with(|h| h.boot_reason = 1 << 17);

    for n in 1..=EARLY_SAVES {
        let (_, _, save) = boot(&mut provider);
        assert_eq!(save, saved(u64::from(n), n), "boot {n}");
    }
    let before = stats_sectors();
    let (mut writer, open, save) = boot(&mut provider);
    let generation = u64::from(EARLY_SAVES);
    assert_eq!(
        open,
        Some(Record::Product(FeederRecord::StatsOpen {
            status: StatsStatus::Loaded,
            generation,
            early: EARLY_SAVES,
            missed: true,
        }))
    );
    assert_eq!(save, None, "no token left");
    assert_eq!(stats_sectors(), before);
    let watchdog = stats::totals().unwrap().resets.watchdog;
    assert_eq!(watchdog, u32::from(EARLY_SAVES) + 1, "this boot, in RAM");

    // This boot lasts an hour: its settled save counts it and gives one
    // token back, which the next short boot takes.
    sdk::delay_ms(SAVE_MS);
    assert_eq!(
        step(&mut writer, &mut provider),
        saved(generation + 1, EARLY_SAVES - 1)
    );
    let (_, open, save) = boot(&mut provider);
    assert!(
        matches!(
            open,
            Some(Record::Product(FeederRecord::StatsOpen {
                missed: true,
                ..
            }))
        ),
        "the flag stays"
    );
    assert_eq!(save, saved(generation + 2, EARLY_SAVES));
    assert_eq!(
        stats::totals().unwrap().resets.watchdog,
        u32::from(EARLY_SAVES) + 2
    );
    let (_, _, save) = boot(&mut provider);
    assert_eq!(save, None);
}

/// A boot's feeds survive a power cut long before the hourly save: the
/// first at once and later ones at most every `FEED_SAVE_MS`, with no early
/// token spent, so a spent budget does not lose them.
#[test]
fn a_finished_feed_saves_the_totals_at_a_bounded_rate() {
    let (_guard, _, mut provider) = start();
    fake::with(|h| h.boot_reason = 1 << 17);
    for _ in 0..u32::from(EARLY_SAVES) {
        boot(&mut provider);
    }
    let (mut writer, open, _) = boot(&mut provider);
    assert!(matches!(
        open,
        Some(Record::Product(FeederRecord::StatsOpen {
            missed: true,
            ..
        }))
    ));
    let generation = |record| match record {
        Some(Record::Product(FeederRecord::StatsSaved { generation, early })) => {
            assert_eq!(early, EARLY_SAVES, "no token moves");
            generation
        }
        other => panic!("not a save: {other:?}"),
    };

    idle(&mut writer, &mut provider, 3);
    let fed = |feeds| Counts {
        feeds,
        ..Counts::default()
    };
    stats::hand_counts(fed(1));
    let first = generation(step(&mut writer, &mut provider));
    idle(&mut writer, &mut provider, 3);

    // Another feed inside the interval waits for its end.
    stats::hand_counts(fed(2));
    idle(&mut writer, &mut provider, 3);
    sdk::delay_ms(FEED_SAVE_MS - 1);
    idle(&mut writer, &mut provider, 3);
    sdk::delay_ms(1);
    assert_eq!(generation(step(&mut writer, &mut provider)), first + 1);

    // With no feed since, the interval's end asks for nothing.
    sdk::delay_ms(FEED_SAVE_MS);
    idle(&mut writer, &mut provider, 3);

    // A cut now: the next boot starts from what the feeds left.
    let (_, _, _) = boot(&mut provider);
    assert_eq!(stats::totals().unwrap().counts.feeds, 2);
}

/// Whatever the crash pattern, the store's erases stay within the early
/// tokens plus two per settled save: here bursts of short boots, each
/// followed by one boot that lasts a single save interval.
#[test]
fn a_mixed_crash_pattern_wears_the_store_at_most_two_erases_an_hour() {
    let (_guard, _, mut provider) = start();
    fake::with(|h| h.boot_reason = 1 << 17);
    let rounds = 20;
    for _ in 0..rounds {
        for _ in 0..u32::from(EARLY_SAVES) {
            boot(&mut provider);
        }
        let (mut writer, _, _) = boot(&mut provider);
        sdk::delay_ms(SAVE_MS);
        assert!(matches!(
            step(&mut writer, &mut provider),
            Some(Record::Product(FeederRecord::StatsSaved { .. }))
        ));
    }
    let settled = rounds;
    assert!(
        stats_erases() <= u32::from(EARLY_SAVES) + 2 * settled,
        "{} erases",
        stats_erases()
    );
}

/// An intentional restart's grace: the writer saves at once with the
/// counts the loop handed over (an early save) and says it is done, so the
/// restart need not wait out the grace; once per boot.
#[test]
fn an_intentional_restart_saves_in_its_grace() {
    let (_guard, mut writer, mut provider) = start();
    open_all(&mut writer, &mut provider);
    assert_eq!(step(&mut writer, &mut provider), saved(1, 1));

    stats::hand_counts(Counts {
        feeds: 2,
        ..Counts::default()
    });
    let asked = sdk::uptime_ms();
    supervision::begin_restart_grace(asked);
    assert!(!supervision::restart_grace_over(asked), "the writer first");
    assert_eq!(step(&mut writer, &mut provider), saved(2, 2));
    assert!(supervision::restart_grace_over(asked), "flushed: at once");
    assert!(!supervision::restart_grace_over(asked), "only once");
    idle(&mut writer, &mut provider, 3);

    let (_, _, save) = boot(&mut provider);
    assert_eq!(stats::totals().unwrap().counts.feeds, 2);
    assert_eq!(save, saved(3, 3));
}

/// With the budget spent, a restart once the boot has settled still saves
/// its counts, neither taking a token nor giving one back.
#[test]
fn a_settled_restart_saves_with_the_budget_spent() {
    let (_guard, _, mut provider) = start();
    for _ in 1..=EARLY_SAVES {
        boot(&mut provider);
    }
    let (mut writer, _, save) = boot(&mut provider);
    assert_eq!(save, None, "no token left");

    sdk::delay_ms(RESTART_SETTLED_MS);
    stats::hand_counts(Counts {
        feeds: 2,
        ..Counts::default()
    });
    let asked = sdk::uptime_ms();
    supervision::begin_restart_grace(asked);
    assert_eq!(
        step(&mut writer, &mut provider),
        saved(u64::from(EARLY_SAVES) + 1, EARLY_SAVES)
    );
    assert!(supervision::restart_grace_over(asked), "flushed: at once");

    let (_, _, save) = boot(&mut provider);
    assert_eq!(stats::totals().unwrap().counts.feeds, 2);
    assert_eq!(save, None, "still no token");
}

/// A restart's save held by a feed waits only through the grace; then the
/// feeding stops and no write turn is granted, so it never starts into the
/// reset (which would leave the arming marker set and the next boot
/// without its stores). An unarmed writer has nothing to save and lets the
/// restart go at once.
#[test]
fn a_restart_save_never_starts_after_the_grace() {
    let (_guard, mut writer, mut provider) = start();
    open_all(&mut writer, &mut provider);
    assert_eq!(step(&mut writer, &mut provider), saved(1, 1));

    INTERLOCK.claim_feed(u32::MAX).unwrap();
    let asked = sdk::uptime_ms();
    supervision::begin_restart_grace(asked);
    idle(&mut writer, &mut provider, 3);
    let over = asked + supervision::RESTART_GRACE_MS;
    assert!(!supervision::restart_grace_over(over - 1));
    assert!(supervision::restart_grace_over(over), "ran out");
    // The runtime's restart: the feeding stops, and writes are held.
    supervision::stopping();
    INTERLOCK.release_feed();
    idle(&mut writer, &mut provider, 5);

    let (_guard2, mut writer, mut provider) = {
        drop(_guard);
        start()
    };
    open_all(&mut writer, &mut provider);
    ARMED.with(|a| a.set(false));
    supervision::begin_restart_grace(0);
    idle(&mut writer, &mut provider, 1);
    assert!(supervision::restart_grace_over(0), "nothing to wait for");
}

/// A save that falls due during a feed waits for the release and never
/// writes beside a claim, not even one past its bound.
#[test]
fn a_due_save_waits_for_a_feed_and_never_writes_beside_a_claim() {
    let (_guard, mut writer, mut provider) = start();
    assert!(stopped::initialize().verified());
    open_all(&mut writer, &mut provider);

    // The boot's own save, due at once, waits for a feed already running.
    INTERLOCK.claim_feed(u32::MAX).unwrap();
    idle(&mut writer, &mut provider, 5);
    INTERLOCK.release_feed();
    assert_eq!(step(&mut writer, &mut provider), saved(1, 1));

    sdk::delay_ms(SAVE_MS);
    INTERLOCK.claim_feed(u32::MAX).unwrap();
    idle(&mut writer, &mut provider, 10);
    INTERLOCK.release_feed();
    assert_eq!(step(&mut writer, &mut provider), saved(2, 0));

    // A claim never released: past its bound another store's write may go
    // ahead with the outputs stopped, but the totals keep waiting.
    sdk::delay_ms(SAVE_MS);
    INTERLOCK.claim_feed(1_000).unwrap();
    idle(&mut writer, &mut provider, 1);
    sdk::delay_ms(10_000);
    idle(&mut writer, &mut provider, 3);
    assert_eq!(INTERLOCK.status().writes_forced, 0);
    assert!(!INTERLOCK.writing());
    INTERLOCK.release_feed();
    assert_eq!(step(&mut writer, &mut provider), saved(3, 0));
}

/// An unarmed writer never saves the totals, though it reports them; a
/// failed save is not retried for the rest of the boot.
#[test]
fn the_totals_are_saved_only_while_armed_and_never_retried() {
    let (_guard, mut writer, mut provider) = start();
    open_all(&mut writer, &mut provider);
    ARMED.with(|a| a.set(false));
    sdk::delay_ms(SAVE_MS);
    idle(&mut writer, &mut provider, 3);
    assert!(stats::totals().is_some());

    ARMED.with(|a| a.set(true));
    PROGRAM_FAILS.with(|f| f.set(true));
    assert_eq!(
        step(&mut writer, &mut provider),
        Some(Record::Product(FeederRecord::StatsError {
            stage: ErrorStage::Save
        }))
    );
    PROGRAM_FAILS.with(|f| f.set(false));
    sdk::delay_ms(SAVE_MS);
    idle(&mut writer, &mut provider, 3);
}
