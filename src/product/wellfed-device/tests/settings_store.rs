//! Tests for the two-slot settings store over the device's record:
//! alternating saves, value validation, a power cut at every byte,
//! newer-major, newer-minor and foreign-binding records, torn-slot fallback,
//! the wear limit and the fields' bytes. The feeder's record, over the same
//! store, is tested in `wellfed-feeder`.
//!
//! Unlike the feed journal, losing the newest change is acceptable: a cut
//! save must reopen with the old or the new settings and stay writable. A
//! newer-major record reads as defaults and is never overwritten until an
//! explicit reset; a newer minor reads, and its fields survive a save.

use wellfed_core::settings::Window;
use wellfed_core::store::slot::{RECORD_BYTES, crc32};
use wellfed_device::store::settings::{
    Settings, SettingsStatus, SettingsStore, SlotState, VERSION, WEAR_LIMIT,
};
use wellfed_testkit::journal_flash::{FakeJournalFlash, FlashIo, ORDERS, UNIT};

const BINDING: u64 = 0x5046_5731_3502_0001;
const UNITS: [usize; 3] = [UNIT, 4096, 148];

fn open(f: &mut FakeJournalFlash) -> SettingsStore<FlashIo> {
    let mut s: SettingsStore<_> = SettingsStore::default();
    // SAFETY: the fake flash outlives the store in every test.
    unsafe { s.open(f.io(), BINDING) };
    s
}

fn custom(volume: u8) -> Settings {
    Settings {
        lights: false,
        sound: true,
        volume,
        lights_window: Window::Between {
            start: 1320,
            end: 420,
        },
        sound_window: Window::Always,
    }
}

#[test]
fn empty_saves_alternate_and_reload() {
    for unit in UNITS {
        let mut f = FakeJournalFlash::with_unit(unit);
        let mut s = open(&mut f);
        assert_eq!(
            (s.status, s.settings),
            (SettingsStatus::Empty, Settings::default())
        );

        for (i, volume) in [10u8, 20, 30].into_iter().enumerate() {
            assert!(s.save(&custom(volume)));
            assert_eq!((s.generation, s.slot), (i as u64 + 1, (i % 2) as u32));

            let r = open(&mut f);
            assert_eq!(
                (r.status, r.settings, r.generation),
                (SettingsStatus::Loaded, custom(volume), i as u64 + 1)
            );
        }
    }
}

#[test]
fn invalid_values_are_refused_without_writing() {
    let mut f = FakeJournalFlash::default();
    let mut s = open(&mut f);
    let before = f.mutations;
    for bad in [
        Settings {
            volume: 101,
            ..Settings::default()
        },
        Settings {
            lights_window: Window::Between {
                start: 1440,
                end: 0,
            },
            ..Settings::default()
        },
    ] {
        assert!(!s.save(&bad));
    }

    assert_eq!(f.mutations, before);
}

/// Cut power at every byte of a save: the store must reopen with the old or
/// the new settings, writable, never anything else.
#[test]
fn power_cut_at_every_byte_keeps_old_or_new() {
    for (order, unit) in ORDERS.into_iter().flat_map(|o| [UNIT, 148].map(|u| (o, u))) {
        for history in 0..3u8 {
            let mut probe = FakeJournalFlash::with_unit(unit).ordered(order);
            let mut s = open(&mut probe);
            for v in 0..history {
                assert!(s.save(&custom(v)));
            }

            let start = probe.mutations;
            assert!(s.save(&custom(99)));
            let span = i64::from(probe.mutations - start);

            for cut in 0..=span {
                let mut f = FakeJournalFlash::with_unit(unit).ordered(order);
                let mut s = open(&mut f);
                for v in 0..history {
                    assert!(s.save(&custom(v)));
                }

                f.budget = cut;
                let _ = s.save(&custom(99));
                f.budget = -1;

                let r = open(&mut f);
                let old = if history == 0 {
                    Settings::default()
                } else {
                    custom(history - 1)
                };

                assert!(
                    r.settings == old || r.settings == custom(99),
                    "{order:?} unit {unit} history {history} cut {cut}: {:?}",
                    r.status
                );
                assert!(
                    matches!(
                        r.status,
                        SettingsStatus::Loaded | SettingsStatus::Recovered | SettingsStatus::Empty
                    ),
                    "{order:?} unit {unit} history {history} cut {cut}: {:?}",
                    r.status
                );

                // A save cut after its first mutation leaves the slot it was
                // writing visibly unfinished.
                if cut > 0 && history > 0 && r.settings == old {
                    assert_ne!(
                        r.slots[1 - r.slot as usize],
                        SlotState::Valid,
                        "{order:?} unit {unit} history {history} cut {cut}"
                    );
                }

                // Still writable afterwards.
                let mut r = r;
                assert!(r.save(&custom(77)));
                assert_eq!(open(&mut f).settings, custom(77));
            }
        }
    }
}

/// A record from a newer major with a valid CRC: read as defaults, never
/// overwritten until reset.
#[test]
fn newer_schema_is_read_only_until_reset() {
    let mut f = FakeJournalFlash::default();
    let mut s = open(&mut f);
    assert!(s.save(&custom(5)));

    let mut record = f.bytes[0][..RECORD_BYTES].to_vec();
    record[8] = VERSION.major + 1;
    let crc = crc32(&record[..120]).to_le_bytes();
    record[120..124].copy_from_slice(&crc);
    f.bytes[0][..RECORD_BYTES].copy_from_slice(&record);

    let mut s = open(&mut f);
    assert_eq!(
        (s.status, s.settings),
        (SettingsStatus::Future, Settings::default())
    );

    let before = f.mutations;
    assert!(!s.save(&custom(6)));
    assert_eq!(f.mutations, before);

    assert!(s.reset() && s.status == SettingsStatus::Empty);
    assert!(s.save(&custom(6)));
    assert_eq!(open(&mut f).settings, custom(6));
}

#[test]
fn foreign_binding_or_garbage_is_corrupt_and_read_only() {
    let mut f = FakeJournalFlash::default();
    let mut s: SettingsStore<_> = SettingsStore::default();
    // SAFETY: the fixture's callbacks are sound for its context, which outlives the call.
    unsafe { s.open(f.io(), BINDING ^ 1) };
    assert!(s.save(&custom(1)));

    let mut s = open(&mut f);
    assert_eq!(s.status, SettingsStatus::Corrupt);
    assert!(!s.save(&custom(2)));

    let mut f = FakeJournalFlash::default();
    f.bytes[1][7] = 0x12;
    let s = open(&mut f);
    assert_eq!(s.status, SettingsStatus::Corrupt);
}

#[test]
fn torn_newer_slot_falls_back_and_reports_recovered() {
    let mut f = FakeJournalFlash::default();
    let mut s = open(&mut f);
    assert!(s.save(&custom(1)) && s.save(&custom(2)));

    // Tear the newer record (slot 1): a save cut before its commit marker.
    f.bytes[1][124..128].fill(u8::MAX);
    let r = open(&mut f);
    assert_eq!(
        (r.status, r.settings, r.generation),
        (SettingsStatus::Recovered, custom(1), 1)
    );
    assert_eq!(r.slots, [SlotState::Valid, SlotState::Dirty]);

    // An erase cut short leaves old bits behind the record: invalid.
    f.bytes[1].fill(u8::MAX);
    f.bytes[1][200] = 0;
    assert_eq!(open(&mut f).slots, [SlotState::Valid, SlotState::Invalid]);

    f.bytes[1].fill(u8::MAX);
    let r = open(&mut f);
    assert_eq!(
        (r.status, r.slots),
        (
            SettingsStatus::Loaded,
            [SlotState::Valid, SlotState::Erased]
        )
    );
}

#[test]
fn wear_limit_refuses_saves() {
    let mut f = FakeJournalFlash::default();
    let mut s = open(&mut f);
    assert!(s.save(&custom(1)));
    s.generation = WEAR_LIMIT;
    let before = f.mutations;
    assert!(!s.save(&custom(2)));
    assert_eq!((s.status, f.mutations), (SettingsStatus::Worn, before));
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
fn windows_survive_a_reopen() {
    let mut f = FakeJournalFlash::default();
    let mut s = open(&mut f);
    let settings = Settings {
        sound_window: Window::Between {
            start: 480,
            end: 1200,
        },
        ..custom(5)
    };
    assert!(s.save(&settings));

    let s = open(&mut f);
    assert_eq!(s.status, SettingsStatus::Loaded);
    assert_eq!(s.settings, settings);
}

/// No migration: a record of an earlier major is superseded (major 4 held
/// the feeder's fields too, which moved to the feeder's record), so the
/// store opens writable on defaults and the next save replaces it.
#[test]
fn an_older_record_opens_empty_and_writable() {
    for major in [1u8, 2, 3, 4] {
        let mut f = FakeJournalFlash::default();
        let mut s = open(&mut f);
        assert!(s.save(&custom(5)));
        rewrite(&mut f, |record| record[8..10].copy_from_slice(&[major, 0]));

        let mut s = open(&mut f);
        assert_eq!(s.status, SettingsStatus::Empty, "major {major}");
        assert_eq!(s.slots[0], SlotState::Older);
        assert_eq!(s.settings, Settings::default());

        assert!(s.save(&custom(7)));
        let s = open(&mut f);
        assert_eq!(s.status, SettingsStatus::Loaded);
        assert_eq!(s.settings, custom(7));
    }
}

/// A newer minor only adds fields in the spare room: this build reads the
/// record, and its own save keeps the minor and the field it doesn't know,
/// so the later build finds them again.
#[test]
fn a_newer_minor_reads_and_keeps_its_fields() {
    let mut f = FakeJournalFlash::default();
    let mut s = open(&mut f);
    assert!(s.save(&custom(5)));
    rewrite(&mut f, |record| {
        record[9] = VERSION.minor + 1;
        record[60] = 0xab;
    });

    let mut s = open(&mut f);
    assert_eq!((s.status, s.settings), (SettingsStatus::Loaded, custom(5)));

    assert!(s.save(&custom(6)));
    assert_eq!(f.bytes[1][9], VERSION.minor + 1, "the minor is kept");
    assert_eq!(f.bytes[1][60], 0xab, "its field is kept");
    let s = open(&mut f);
    assert_eq!((s.status, s.settings), (SettingsStatus::Loaded, custom(6)));
}

#[test]
fn a_bad_field_makes_the_record_invalid() {
    for (at, byte) in [
        // A flag bit past lights and sound.
        (28, 4),
        // An unknown window kind.
        (30, 2),
        // An always window with a bound.
        (32, 1),
        // A start past the last minute of the day (1440 = 0x05a0).
        (37, 0x05),
        // The spare room, which this build's own minor keeps zero.
        (40, 1),
        (119, 1),
    ] {
        let mut f = FakeJournalFlash::default();
        let mut s = open(&mut f);
        let settings = Settings {
            lights_window: Window::Always,
            sound_window: Window::Between { start: 0, end: 0 },
            ..custom(5)
        };
        assert!(s.save(&settings));
        rewrite(&mut f, |record| {
            if at == 37 {
                record[36] = 0xa0;
            }
            record[at] = byte;
        });

        let s = open(&mut f);
        assert_eq!(s.status, SettingsStatus::Corrupt, "byte {at}");
    }
}
