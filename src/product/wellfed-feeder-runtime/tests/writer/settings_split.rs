//! The two settings records, the device's (sectors 2 and 3) and the
//! feeder's (48 and 49), saved from one change at one shared generation:
//! a save that fails on either is refused, never `Done`, and records an
//! event only once the device's part landed, naming the generation the
//! view then reports; a power cut between the two saves leaves the device's
//! part changed with no event; a record that loses its newest slot never
//! brings back a generation that named other settings; and either record
//! unreadable makes the whole view read-only.

use std::sync::MutexGuard;

use wellfed_core::store::slot::crc32;
use wellfed_device::store::events::Read;
use wellfed_device::store::settings::SettingsStatus;
use wellfed_device_api::mailbox::{
    FeederFields, MealReminder, SaveOutcome, SettingsFields, SettingsResult, SettingsView, Stamp,
};
use wellfed_device_runtime::writer::{ErrorStage, SettingsPart};
use wellfed_device_runtime::{asked, events};
use wellfed_feeder_runtime::settings::SETTINGS;
use wellfed_feeder_runtime::writer::{Record, Writer};
use wellfed_hal::board::storage::{self, Sectors};
use wellfed_hal::flash_lock::provider::Provider;
use wellfed_hal::rng;
use wellfed_hal::sdk::fake;
use wellfed_proto::api::{EventBody, Source};

use crate::support::{
    A, ASKED_AT, Armed, ERASE_FAILS, EVENTS_PROGRAMS_FAIL, PROGRAMS_FAIL_IN, boot,
    restart_mailboxes, start, step,
};

/// The client every change here comes from, and when it asked.
const ASKED: Stamp = Stamp {
    by: Source::Client([9, 9, 9, 9]),
    at: Some(ASKED_AT),
};

/// The writer opened, and its first statistics save taken.
fn opened() -> (MutexGuard<'static, ()>, Writer<Armed>, Provider) {
    let (guard, mut writer, mut provider) = start();
    fake::with(|h| h.adc_noise = 0x1357_9bdf);
    assert_eq!(rng::start().result, Ok(()));
    for _ in 0..7 {
        step(&mut writer, &mut provider);
    }
    (guard, writer, provider)
}

/// The view the writer last published.
fn view() -> SettingsView {
    SETTINGS.current().expect("the settings are published")
}

/// A change to both records: the device's volume and the feeder's meal
/// reminder.
fn both(volume: u8, minutes: u8) -> SettingsFields {
    SettingsFields {
        volume: Some(volume),
        kind: FeederFields {
            meal_reminder: Some(MealReminder::Before { minutes }),
            ..FeederFields::default()
        },
        ..SettingsFields::default()
    }
}

/// Queue `change` as the client's; the number it will finish as.
fn ask(change: SettingsFields) -> u32 {
    let next = view().saved.next();
    assert!(matches!(
        asked::settings_request(&SETTINGS, change, Some(ASKED)),
        SettingsResult::Current { pending: true, .. }
    ));
    next
}

/// The settings events in the ring, oldest first, read as subscriber A
/// through the writer's read turns: each its generation.
fn settings_events(writer: &mut Writer<Armed>, provider: &mut Provider) -> Vec<u32> {
    let mut found = Vec::new();
    let mut after = None;
    for _ in 0..256 {
        match events::read_result(A, after) {
            Some(Read::Record(record)) => {
                after = Some(record.seq);
                if let Some(EventBody::Settings { generation, by }) =
                    wellfed_proto::decode_padded::<EventBody>(&record.body)
                {
                    assert_eq!(by, ASKED.by);
                    found.push(generation);
                }
            }
            Some(Read::End) if events::pending().is_none() => return found,
            Some(Read::Gap { from, to }) => panic!("events {from}..={to} lost"),
            Some(Read::End) | None => {
                step(writer, provider);
            }
        }
    }
    panic!("the ring never reached its end");
}

/// A restart: the mailboxes forgotten and a fresh writer opened over the
/// same flash.
fn restart(provider: &mut Provider) -> Writer<Armed> {
    restart_mailboxes();
    boot(provider).0
}

/// The byte at `at` of `sectors`' slot `slot` (0 or 1).
fn byte(sectors: Sectors, slot: u32, at: usize) -> usize {
    (sectors.first + slot) as usize * 0x1000 + at
}

/// Break the record in `sectors`' slot `slot`, as a slot that lost its
/// contents reads: its checksum no longer holds.
fn lose(sectors: Sectors, slot: u32) {
    fake::with(|h| h.storage[byte(sectors, slot, 60)] ^= 0x5a);
}

/// The device's part saved, the feeder's save failing after it: the
/// client is refused, the view reports the device's part changed at a
/// generation of its own and read-only (the feeder's store failed), and
/// the event names that generation, so the history says what the feeder
/// holds. The error record names the feeder's store.
#[test]
fn a_feeder_save_that_fails_after_the_device_s_is_refused_and_told_truly() {
    let (_guard, mut writer, mut provider) = opened();
    PROGRAMS_FAIL_IN.with(|f| f.set(Some(storage::FEEDER_SETTINGS)));

    let change = ask(both(20, 15));
    assert_eq!(
        step(&mut writer, &mut provider),
        Some(Record::Error {
            stage: ErrorStage::Save,
            store: SettingsPart::Kind,
            status: SettingsStatus::Io,
        })
    );

    let view = view();
    assert_eq!(view.saved.outcome(change), SaveOutcome::Failed);
    assert_eq!(
        (view.volume, view.kind.meal_reminder),
        (20, MealReminder::Off),
        "the device's part landed, the feeder's did not"
    );
    assert_eq!(
        (view.generation, view.writable),
        (2, false),
        "one past the device's 1: no save published it"
    );
    assert_eq!(settings_events(&mut writer, &mut provider), [2]);
}

/// The device's save failing first: nothing lands, nothing is recorded,
/// and the client is refused; the error record names the device's store.
#[test]
fn a_device_save_that_fails_lands_nothing_and_records_no_event() {
    let (_guard, mut writer, mut provider) = opened();
    PROGRAMS_FAIL_IN.with(|f| f.set(Some(storage::SETTINGS)));

    let change = ask(both(20, 15));
    assert_eq!(
        step(&mut writer, &mut provider),
        Some(Record::Error {
            stage: ErrorStage::Save,
            store: SettingsPart::Device,
            status: SettingsStatus::Io,
        })
    );

    let view = view();
    assert_eq!(view.saved.outcome(change), SaveOutcome::Failed);
    assert_eq!(
        (view.volume, view.kind.meal_reminder, view.generation),
        (50, MealReminder::Off, 0)
    );
    assert_eq!(settings_events(&mut writer, &mut provider), []);
}

/// A whole save is `Done` at one generation both records hold, so a
/// restart reads the same view.
#[test]
fn a_whole_save_is_done_and_both_records_hold_its_generation() {
    let (_guard, mut writer, mut provider) = opened();
    let change = ask(both(20, 15));
    assert!(matches!(
        step(&mut writer, &mut provider),
        Some(Record::Saved { generation: 1, .. })
    ));
    let saved = view();
    assert_eq!(saved.saved.outcome(change), SaveOutcome::Done);
    assert_eq!(
        (saved.volume, saved.kind.meal_reminder, saved.generation),
        (20, MealReminder::Before { minutes: 15 }, 1)
    );

    // A change to the device's part alone still writes both.
    let change = ask(SettingsFields {
        volume: Some(30),
        ..SettingsFields::default()
    });
    step(&mut writer, &mut provider);
    assert_eq!(view().saved.outcome(change), SaveOutcome::Done);
    assert_eq!(view().generation, 2);

    let mut writer = restart(&mut provider);
    let reopened = view();
    assert_eq!(
        (reopened.volume, reopened.generation, reopened.writable),
        (30, 2, true),
        "the records agree"
    );
    assert_eq!(settings_events(&mut writer, &mut provider), [1, 2]);
}

/// A power cut between the two saves: the device's record is written, the
/// feeder's and the event are not (the event is appended after both). The
/// next boot reads the device's part changed, with no event, at a
/// generation no save published; the client's write was never answered,
/// so it is uncertain. The next save brings the records together again.
#[test]
fn a_power_cut_between_the_two_saves_leaves_the_device_s_part_and_no_event() {
    let (_guard, mut writer, mut provider) = opened();
    PROGRAMS_FAIL_IN.with(|f| f.set(Some(storage::FEEDER_SETTINGS)));
    EVENTS_PROGRAMS_FAIL.with(|f| f.set(true));
    ask(both(20, 15));
    step(&mut writer, &mut provider);
    PROGRAMS_FAIL_IN.with(|f| f.set(None));
    EVENTS_PROGRAMS_FAIL.with(|f| f.set(false));

    let mut writer = restart(&mut provider);
    let after = view();
    assert_eq!(
        (
            after.volume,
            after.kind.meal_reminder,
            after.generation,
            after.writable
        ),
        (20, MealReminder::Off, 2, true)
    );
    assert_eq!(settings_events(&mut writer, &mut provider), []);

    let change = ask(both(30, 15));
    assert!(matches!(
        step(&mut writer, &mut provider),
        Some(Record::Saved { generation: 3, .. })
    ));
    assert_eq!(view().saved.outcome(change), SaveOutcome::Done);
    restart(&mut provider);
    assert_eq!(view().generation, 3, "the records agree again");
}

/// A record that loses its newest slot falls back to its older one, and
/// the view never reports a generation that named other settings. Saves
/// publish 1 and 2; a lost newest slot in either record reads as 6, four
/// past the larger, and the next save brings the records together past
/// it.
#[test]
fn a_lost_newest_slot_never_brings_back_a_generation_that_named_other_settings() {
    for (lost, volume, minutes) in [
        // The feeder's newest lost: the device's 30, the feeder's 15.
        (storage::FEEDER_SETTINGS, 30, 15),
        // The device's newest lost: the device's 20, the feeder's 45.
        (storage::SETTINGS, 20, 45),
    ] {
        let (_guard, mut writer, mut provider) = opened();
        ask(both(20, 15));
        step(&mut writer, &mut provider);
        ask(both(30, 45));
        step(&mut writer, &mut provider);
        assert_eq!(view().generation, 2);

        // The second save went to each record's slot 1.
        lose(lost, 1);
        let mut writer = restart(&mut provider);
        let decayed = view();
        assert_eq!(
            (decayed.volume, decayed.kind.meal_reminder),
            (volume, MealReminder::Before { minutes }),
            "{lost:?}"
        );
        assert_eq!(decayed.generation, 6, "{lost:?}: neither 1 nor 2");

        ask(both(40, 15));
        assert!(matches!(
            step(&mut writer, &mut provider),
            Some(Record::Saved { generation: 7, .. })
        ));
        restart(&mut provider);
        assert_eq!(view().generation, 7, "{lost:?}: the records agree again");
    }
}

/// The reviewer's sequence: two whole saves (generation 2), then a cut
/// right after save 3's device write, before the feeder's record is
/// touched, so the boot reads 4 (volume 40, reminder 45). The feeder's
/// newest slot then lost, the next boot reads other settings (reminder
/// 15), which must not read as 4, the generation a client may hold, nor
/// as anything published; the next save writes past it all.
#[test]
fn a_cut_then_a_lost_feeder_slot_never_repeats_a_generation() {
    let (_guard, mut writer, mut provider) = opened();
    ask(both(20, 15));
    step(&mut writer, &mut provider);
    ask(both(30, 45));
    step(&mut writer, &mut provider);

    // The cut: the feeder's save fails at its erase, before any byte of
    // its record changes, and the event is never written.
    ERASE_FAILS.with(|f| f.set(Some(storage::FEEDER_SETTINGS.first)));
    EVENTS_PROGRAMS_FAIL.with(|f| f.set(true));
    ask(both(40, 15));
    step(&mut writer, &mut provider);
    ERASE_FAILS.with(|f| f.set(None));
    EVENTS_PROGRAMS_FAIL.with(|f| f.set(false));

    restart(&mut provider);
    let cut = view();
    assert_eq!(
        (cut.volume, cut.kind.meal_reminder, cut.generation),
        (40, MealReminder::Before { minutes: 45 }, 4)
    );

    // The feeder's newest slot, generation 2, is slot 1.
    lose(storage::FEEDER_SETTINGS, 1);
    let mut writer = restart(&mut provider);
    let lost = view();
    assert_eq!(
        (lost.volume, lost.kind.meal_reminder, lost.writable),
        (40, MealReminder::Before { minutes: 15 }, true)
    );
    assert_eq!(
        lost.generation, 7,
        "past 1, 2 and 4: a client holding 4 is stale"
    );

    ask(both(50, 30));
    assert!(matches!(
        step(&mut writer, &mut provider),
        Some(Record::Saved { generation: 8, .. })
    ));
}

/// How a test spoils the feeder's settings record.
#[derive(Clone, Copy, Debug)]
enum Spoiled {
    /// Bytes no record holds, in both slots.
    Corrupt,
    /// A whole record of the next major.
    Newer,
}

impl Spoiled {
    fn spoil(self, provider: &mut Provider) {
        let first = byte(storage::FEEDER_SETTINGS, 0, 0);
        match self {
            Self::Corrupt => fake::with(|h| h.storage[first..first + 2 * 0x1000].fill(0x12)),
            Self::Newer => {
                let mut writer = restart(provider);
                ask(both(20, 15));
                step(&mut writer, provider);
                fake::with(|h| {
                    let record = &mut h.storage[first..first + 128];
                    record[8] += 1;
                    let crc = crc32(&record[..120]).to_le_bytes();
                    record[120..124].copy_from_slice(&crc);
                });
            }
        }
    }
}

/// The feeder's record unreadable, corrupt or of a newer major: the whole
/// view is read-only, so even a change to the device's part alone is
/// refused, until a factory reset erases it.
#[test]
fn an_unreadable_feeder_record_makes_the_whole_view_read_only() {
    for spoiled in [Spoiled::Corrupt, Spoiled::Newer] {
        let (_guard, _writer, mut provider) = opened();
        spoiled.spoil(&mut provider);

        restart(&mut provider);
        assert!(!view().writable, "{spoiled:?}");
        let device_only = SettingsFields {
            volume: Some(10),
            ..SettingsFields::default()
        };
        assert_eq!(
            asked::settings_request(&SETTINGS, device_only, Some(ASKED)),
            SettingsResult::ReadOnly,
            "{spoiled:?}"
        );
    }
}
