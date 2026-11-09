//! Tests for the feeder's own settings record (sectors 48 and 49), over the
//! device's two-slot settings store: its values read back as saved, a value
//! out of its limits is refused without writing, a field this build doesn't
//! write makes the record invalid, a newer minor reads and keeps its
//! fields, and a newer major opens read-only. The store's torn-write rule
//! is tested once, over the device's record, in `wellfed-device`.

use wellfed_core::store::slot::{RECORD_BYTES, crc32};
use wellfed_core_feeder::feed::progress::BowlMode;
use wellfed_core_feeder::settings::{FeederSettings, MealReminder};
use wellfed_device::store::settings::SettingsStatus;
use wellfed_feeder::store::settings::{FeederSettingsStore, VERSION};
use wellfed_testkit::journal_flash::{FakeJournalFlash, FlashIo};

const BINDING: u64 = 0x5046_5731_3502_0001;

fn open(f: &mut FakeJournalFlash) -> FeederSettingsStore<FlashIo> {
    let mut s = FeederSettingsStore::default();
    // SAFETY: the fake flash outlives the store in every test.
    unsafe { s.open(f.io(), BINDING) };
    s
}

fn custom(days: u8) -> FeederSettings {
    FeederSettings {
        bowl_mode: BowlMode::Double,
        meal_reminder: MealReminder::Before { minutes: 5 },
        desiccant_days: days,
        desiccant_reset: Some(20_389),
    }
}

/// Rewrite slot 0's record with `edit` and a fresh CRC.
fn rewrite(f: &mut FakeJournalFlash, edit: impl FnOnce(&mut [u8])) {
    let mut record = f.bytes[0][..RECORD_BYTES].to_vec();
    edit(&mut record);
    let crc = crc32(&record[..120]).to_le_bytes();
    record[120..124].copy_from_slice(&crc);
    f.bytes[0][..RECORD_BYTES].copy_from_slice(&record);
}

#[test]
fn a_fresh_store_holds_the_defaults_and_saves_alternate() {
    let mut f = FakeJournalFlash::default();
    let mut s = open(&mut f);
    assert_eq!(
        (s.status, s.settings),
        (SettingsStatus::Empty, FeederSettings::DEFAULT)
    );
    assert_eq!(
        (
            s.settings.meal_reminder,
            s.settings.desiccant_days,
            s.settings.desiccant_reset
        ),
        (MealReminder::Off, 14, None)
    );

    for (i, days) in [10u8, 20, 30].into_iter().enumerate() {
        assert!(s.save(&custom(days)));
        assert_eq!((s.generation, s.slot), (i as u64 + 1, (i % 2) as u32));
        let r = open(&mut f);
        assert_eq!(
            (r.status, r.settings, r.generation),
            (SettingsStatus::Loaded, custom(days), i as u64 + 1)
        );
    }
}

#[test]
fn the_meal_reminder_survives_a_reopen_and_turns_off() {
    let mut f = FakeJournalFlash::default();
    let mut s = open(&mut f);
    for reminder in [
        MealReminder::Before { minutes: 1 },
        MealReminder::Before { minutes: 60 },
        MealReminder::Off,
    ] {
        let settings = FeederSettings {
            meal_reminder: reminder,
            ..custom(5)
        };
        assert!(s.save(&settings));
        s = open(&mut f);
        assert_eq!(s.status, SettingsStatus::Loaded);
        assert_eq!(s.settings, settings);
    }
}

/// The desiccant reminder's period and replacement day read back as saved,
/// whatever the day's sign.
#[test]
fn the_desiccant_reminder_survives_a_reopen() {
    let mut f = FakeJournalFlash::default();
    let mut s = open(&mut f);
    for (days, reset) in [
        (1, Some(0)),
        (90, Some(-1)),
        (7, Some(i32::MAX)),
        (14, None),
    ] {
        let settings = FeederSettings {
            desiccant_days: days,
            desiccant_reset: reset,
            ..custom(5)
        };
        assert!(s.save(&settings));
        let r = open(&mut f);
        assert_eq!(r.status, SettingsStatus::Loaded);
        assert_eq!(r.settings, settings);
    }
}

#[test]
fn values_outside_their_limits_are_refused_without_writing() {
    let mut f = FakeJournalFlash::default();
    let mut s = open(&mut f);
    let before = f.mutations;
    for bad in [
        FeederSettings {
            desiccant_days: 0,
            ..custom(5)
        },
        FeederSettings {
            desiccant_days: 91,
            ..custom(5)
        },
        FeederSettings {
            meal_reminder: MealReminder::Before { minutes: 0 },
            ..custom(5)
        },
        FeederSettings {
            meal_reminder: MealReminder::Before { minutes: 61 },
            ..custom(5)
        },
    ] {
        assert!(!s.save(&bad), "{bad:?}");
    }
    assert_eq!(f.mutations, before);
}

#[test]
fn a_bad_field_makes_the_record_invalid() {
    for (at, byte) in [
        // An unknown bowl mode.
        (28, 0xff),
        // A meal reminder past its hour.
        (29, 61),
        // A period of 0 days, and one past the limit.
        (30, 0),
        (30, 91),
        // A replacement flag that is neither 0 nor 1.
        (31, 2),
        // A day with the flag clear.
        (32, 1),
        // The spare room, which this build's own minor keeps zero.
        (36, 1),
        (119, 1),
    ] {
        let mut f = FakeJournalFlash::default();
        let mut s = open(&mut f);
        let settings = FeederSettings {
            desiccant_reset: None,
            ..custom(5)
        };
        assert!(s.save(&settings));
        rewrite(&mut f, |record| record[at] = byte);

        let s = open(&mut f);
        assert_eq!(s.status, SettingsStatus::Corrupt, "byte {at} = {byte}");
    }
}

/// A newer minor reads, and this build's save keeps the minor and the
/// field it doesn't know.
#[test]
fn a_newer_minor_reads_and_keeps_its_fields() {
    let mut f = FakeJournalFlash::default();
    let mut s = open(&mut f);
    assert!(s.save(&custom(5)));
    rewrite(&mut f, |record| {
        record[9] = VERSION.minor + 1;
        record[40] = 0x5a;
    });

    let mut s = open(&mut f);
    assert_eq!((s.status, s.settings), (SettingsStatus::Loaded, custom(5)));
    assert!(s.save(&custom(6)));
    assert_eq!(f.bytes[1][9], VERSION.minor + 1, "the minor is kept");
    assert_eq!(f.bytes[1][40], 0x5a, "its field is kept");
    assert_eq!(open(&mut f).settings, custom(6));
}

/// A newer major: defaults, read-only until an explicit reset.
#[test]
fn a_newer_major_is_read_only_until_reset() {
    let mut f = FakeJournalFlash::default();
    let mut s = open(&mut f);
    assert!(s.save(&custom(5)));
    rewrite(&mut f, |record| record[8] = VERSION.major + 1);

    let mut s = open(&mut f);
    assert_eq!(
        (s.status, s.settings),
        (SettingsStatus::Future, FeederSettings::DEFAULT)
    );
    let before = f.mutations;
    assert!(!s.save(&custom(6)));
    assert_eq!(f.mutations, before);

    assert!(s.reset() && s.status == SettingsStatus::Empty);
    assert!(s.save(&custom(6)));
    assert_eq!(open(&mut f).settings, custom(6));
}
