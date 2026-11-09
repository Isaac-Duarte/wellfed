//! The factory reset on the boot after the marker: what it clears and
//! keeps, power cuts and torn markers, unwritable stores, its records and
//! the corrupt update store.

use std::cell::Cell;
use std::sync::MutexGuard;

use wellfed_device::store::clients::ClientStatus;
use wellfed_device_api::mailbox::{SettingsFields, SettingsResult};
use wellfed_device_runtime::keys::{self, Change};
use wellfed_device_runtime::writer::{self as writer, ErrorStage, Identity, KeyChange, KeyResult};
use wellfed_device_runtime::{events, network};
use wellfed_feeder::store::stats::StatsStatus;
use wellfed_feeder_runtime::stats;
use wellfed_feeder_runtime::writer::{FeederRecord, Record, Writer};
use wellfed_hal::flash::peer::MARKER_REGISTER;
use wellfed_hal::flash_lock::provider::Provider;
use wellfed_hal::sdk::{self, fake};
use wellfed_hal::{rng, settings};

use crate::support::{
    ARMED, ATTEMPT_SEEN, Armed, ERASE_FAILS, PROGRAM_FAILS, STATS_PROGRAMS_FAIL, change, changed,
    current, phone, reset_boot, reset_marked, restart_mailboxes, sectors, start, step,
    storage_reset_first,
};

/// Write an update store record with the floor `version`, as a confirmed
/// update leaves it.
fn floor_update_store(version: u32) {
    use wellfed_core::ota::offer::ImageKind;
    use wellfed_core::store::update::{Candidate, UpdateStore};
    use wellfed_hal::board::storage;

    let mut update = UpdateStore::default();
    update.open(wellfed_sdk::storage::Region::<
        Armed,
        { storage::UPDATE.first },
        2,
    >::new());
    let build = [0x55; 32];
    update
        .set_activated(Candidate {
            digest: [1; 32],
            version,
            build_id: build,
            kind: ImageKind::Replacement,
        })
        .unwrap();
    update.confirm(&build).unwrap();
}

/// A first boot with a generator that fills the stores the reset clears
/// and the calibration it keeps, then takes an unlocked Reset hold's
/// change. Sentinels stand in for the journal and the confirmation record,
/// which this writer never writes, and for a trace; the update store holds
/// a floor, which a reset keeps.
fn filled_then_reset() -> (MutexGuard<'static, ()>, Provider) {
    use wellfed_device_api::mailbox::{Calibration, PLANS, Plan, PlansChange, Weekdays};
    use wellfed_device_api::wifi::Credentials;
    use wellfed_feeder_runtime::{calibration, plans};
    use wellfed_hal::board::storage;

    let (guard, mut writer, mut provider) = start();
    restart_mailboxes();
    fake::with(|h| {
        h.adc_noise = 0x0f0f_5a5a;
        for kept in [storage::JOURNAL, storage::CONFIRM] {
            let first = kept.first as usize * 0x1000;
            h.storage[first..first + 64].fill(0x42);
        }
        let traces = storage::TRACES.first as usize * 0x1000;
        h.storage[traces..traces + 64].fill(0x24);
        // Bytes in the meal call's first and last sectors.
        for sector in [storage::CALL.first, storage::CALL.first + 7] {
            let first = sector as usize * 0x1000;
            h.storage[first..first + 64].fill(0x77);
        }
    });
    floor_update_store(7);
    assert_eq!(rng::start().result, Ok(()));
    for _ in 0..7 {
        step(&mut writer, &mut provider);
    }

    settings::request(SettingsFields {
        volume: Some(7),
        ..SettingsFields::default()
    });
    step(&mut writer, &mut provider);
    network::change(
        Some(Credentials::new(b"home", b"passphrase").unwrap()),
        None,
    );
    step(&mut writer, &mut provider);
    calibration::change(Calibration {
        portion_mg: Some(9_800),
    });
    step(&mut writer, &mut provider);
    let mut table = [None; PLANS];
    table[0] = Some(Plan {
        minute: 8 * 60,
        portions: 2,
        days: Weekdays::EVERY_DAY,
        enabled: true,
        call: 0,
    });
    plans::change(
        PlansChange::Replace {
            table,
            at_s: Some(1_000),
        },
        None,
    );
    step(&mut writer, &mut provider);
    let phone = phone();
    change(&mut writer, &mut provider, phone);
    assert!(events::record(Some(1_700_000_000), [1; 16]));
    step(&mut writer, &mut provider);
    for cleared in [
        storage::SETTINGS,
        storage::NET,
        storage::PLANS,
        storage::EVENTS,
        storage::TRACES,
        storage::STATS,
    ] {
        assert!(
            sectors(cleared).iter().any(|&byte| byte != 0xff),
            "{cleared:?} holds something to clear"
        );
    }

    assert_eq!(
        change(&mut writer, &mut provider, Change::Reset),
        changed(KeyChange::Reset, KeyResult::Done, 0)
    );
    assert!(reset_marked());
    (guard, provider)
}

/// What the reset keeps, byte for byte: the journal (its high water and
/// any uncertain feed's record), the confirmation record, the update store
/// and the calibration.
fn kept() -> Vec<Vec<u8>> {
    use wellfed_hal::board::storage;

    [
        storage::JOURNAL,
        storage::CONFIRM,
        storage::UPDATE,
        storage::CALIBRATION,
    ]
    .into_iter()
    .map(sectors)
    .collect()
}

/// After a finished reset: every cleared store opens empty and writable,
/// the clients are gone under a new identity, the history starts again
/// under a new epoch, the totals start from this boot, and what it keeps is
/// unchanged.
fn assert_cleared(kept_before: &[Vec<u8>], old_epoch: u32) {
    use wellfed_device_api::mailbox::{
        Calibration, CalibrationResult, PLANS, PlansResult, TimeSettingsResult, WifiResult,
    };
    use wellfed_device_runtime::time;
    use wellfed_feeder_runtime::{calibration, plans};
    use wellfed_hal::board::storage;

    assert!(!reset_marked());
    assert_eq!(current(), (true, true, 50, 0, true), "default settings");
    let WifiResult::Current { view, .. } = network::current() else {
        panic!("the Wi-Fi store is published");
    };
    assert_eq!((view.stored_ssid_crc, view.generation), (None, 0));
    let TimeSettingsResult::Current { view, .. } = time::current() else {
        panic!("the time settings are published");
    };
    assert_eq!((view.generation, view.writable), (0, true));
    let PlansResult::Current { view, .. } = plans::current() else {
        panic!("the plans are published");
    };
    assert_eq!((view.table, view.revision), ([None; PLANS], 0));
    let CalibrationResult::Current { view, .. } = calibration::current() else {
        panic!("the calibration is published");
    };
    let measured = Calibration {
        portion_mg: Some(9_800),
    };
    assert_eq!((view.calibration, view.generation), (measured, 1));

    let mut table = [None; wellfed_device_api::clients::CLIENTS];
    assert!(keys::clients_since(0, &mut table).is_some());
    assert!(table.iter().all(Option::is_none), "every client forgotten");
    assert!(keys::identity_since(0).is_some(), "a new identity");
    let epoch = events::epoch().expect("the ring is open");
    assert_ne!(epoch, old_epoch, "a new history, under its own epoch");
    assert_eq!(events::last_seq(), None, "the history is empty");
    let totals = stats::totals().expect("the totals are published");
    assert_eq!(totals.resets.power_on, 1, "the totals start from this boot");
    assert_eq!(totals.counts.uptime_s, 0);
    assert!(
        totals.storage.iter().any(|&erases| erases > 0),
        "the flash erase counts, the hardware's wear, are kept"
    );

    for cleared in [
        storage::SETTINGS,
        storage::NET,
        storage::TIME,
        storage::PLANS,
        storage::TRACES,
        storage::EVENTS,
        storage::CALL,
    ] {
        assert!(
            sectors(cleared).iter().all(|&byte| byte == 0xff),
            "{cleared:?} erased"
        );
    }
    assert_eq!(kept(), kept_before, "the kept stores are unchanged");
}

#[test]
fn a_factory_reset_clears_the_decided_stores_on_the_next_boot_and_keeps_the_rest() {
    let (_guard, mut provider) = filled_then_reset();
    let old_epoch = events::epoch().unwrap();
    let kept_before = kept();

    ATTEMPT_SEEN.with(|s| s.set(false));
    let records = reset_boot(&mut provider, None);
    assert_eq!(records.first(), Some(&Record::ResetPending));
    assert!(
        records.contains(&Record::ResetDone { erased: 36 }),
        "{records:?}"
    );
    assert!(
        records.iter().any(|record| matches!(
            record,
            Record::Keys {
                clients: ClientStatus::Empty,
                identity: Identity::Created,
                ..
            }
        )),
        "{records:?}"
    );
    assert_cleared(&kept_before, old_epoch);
    assert!(ATTEMPT_SEEN.with(Cell::get));
    assert_eq!(sdk::backup_read(MARKER_REGISTER), 0);

    // The boot after is an ordinary one.
    let records = reset_boot(&mut provider, None);
    assert!(
        matches!(records.first(), Some(Record::Open { .. })),
        "{records:?}"
    );
}

/// A power cut after any number of the reset's erases, the marker's
/// included, with the erase it cut torn: the next boot finishes the reset
/// with the same result. Nothing is published while it runs.
#[test]
fn a_power_cut_at_any_step_of_the_reset_leaves_it_to_the_next_boot() {
    use wellfed_hal::board::storage;

    let (_guard, mut provider) = filled_then_reset();
    let old_epoch = events::epoch().unwrap();
    let kept_before = kept();
    let marked = fake::with(|h| h.storage.clone());

    // `done` steps landed (each sector's erase, the update store's step,
    // the statistics' wear-only save, then the marker's erase) and the next
    // was cut: an erase left half its sector, the save is all or nothing.
    let erases = storage::FACTORY_CLEARED_SECTORS;
    for done in 0..=erases + 2 {
        fake::with(|h| h.storage.clone_from(&marked));
        let steps = usize::try_from(done).unwrap() + 1;
        let records = reset_boot(&mut provider, Some(steps));
        assert_eq!(records, [Record::ResetPending], "cut after {done} steps");
        assert_eq!(
            settings::request(SettingsFields::default()),
            SettingsResult::Busy,
            "nothing is served mid-reset"
        );
        assert_eq!(keys::identity_since(0), None, "no identity mid-reset");

        let torn = match done {
            _ if done < erases => storage::factory_cleared(done),
            _ if done <= erases + 1 => None,
            _ => Some(storage::RESET.first),
        };
        if let Some(torn) = torn {
            fake::with(|h| {
                let first = torn as usize * 0x1000;
                h.storage[first..first + 0x800].fill(0xff);
            });
        }

        // A torn marker reads as no reset: every step before it is done.
        let records = reset_boot(&mut provider, None);
        let finished = done <= erases + 1;
        assert_eq!(
            records.contains(&Record::ResetDone { erased: 36 }),
            finished,
            "cut after {done} steps: {records:?}"
        );
        assert_cleared(&kept_before, old_epoch);
    }
}

/// A cut while the marker is programmed leaves it torn, which is no reset:
/// nothing had been erased, and the stores open as they were.
#[test]
fn a_torn_marker_is_no_reset() {
    use wellfed_device::store::reset::MARKER;
    use wellfed_hal::board::storage;

    let (_guard, mut writer, mut provider) = start();
    for _ in 0..7 {
        step(&mut writer, &mut provider);
    }
    settings::request(SettingsFields {
        volume: Some(9),
        ..SettingsFields::default()
    });
    step(&mut writer, &mut provider);
    fake::with(|h| {
        let first = storage::RESET.first as usize * 0x1000;
        h.storage[first..first + 8].copy_from_slice(&MARKER[..8]);
    });

    let records = reset_boot(&mut provider, None);
    assert!(
        matches!(records.first(), Some(Record::Open { generation: 1, .. })),
        "{records:?}"
    );
    assert_eq!(current().2, 9);
}

/// An unarmed boot that no arming skip explains cannot erase and could
/// not after a restart either: it stops the reset, serves nothing, keeps
/// the marker and waits for a power cycle, asking for no restart.
#[test]
fn an_unarmed_boot_serves_nothing_and_keeps_the_marker() {
    let (_guard, mut provider) = filled_then_reset();
    restart_mailboxes();
    let mut writer = Writer::default();
    assert_eq!(step(&mut writer, &mut provider), Some(Record::ResetPending));
    // The hooks went away after the open (arming itself is the
    // confirmation's to test).
    ARMED.with(|a| a.set(false));

    assert_eq!(
        step(&mut writer, &mut provider),
        Some(Record::ResetError {
            stage: ErrorStage::Unarmed,
            sector: None,
            restart: false,
        })
    );
    assert!(!keys::restart_wanted());
    assert_eq!(step(&mut writer, &mut provider), None, "stopped");
    assert_eq!(
        settings::request(SettingsFields::default()),
        SettingsResult::Busy
    );
    assert_eq!(keys::identity_since(0), None);
    assert!(reset_marked());
}

/// A boot whose erase fails asks for one restart; the restarted boot (the
/// watchdog's flags, as every intentional restart on this board) fails
/// again and asks for none, since the marker sector counts both attempts.
/// A later boot that can erase finishes the reset and clears the marker
/// with its attempts.
#[test]
fn a_reset_that_cannot_finish_asks_for_one_restart_only() {
    use wellfed_device::store::reset::{ATTEMPT, MARKER};
    use wellfed_hal::board::storage;

    let (_guard, mut provider) = filled_then_reset();
    let stuck = storage::factory_cleared(3).unwrap();
    ERASE_FAILS.with(|f| f.set(Some(stuck)));
    let error = |restart| Record::ResetError {
        stage: ErrorStage::Erase,
        sector: Some(stuck),
        restart,
    };

    for (flags, restart) in [(0, true), (0x0010_0000, false), (0x0010_0000, false)] {
        fake::with(|h| h.boot_reason = flags);
        let records = reset_boot(&mut provider, Some(10));
        assert_eq!(
            records,
            [Record::ResetPending, error(restart)],
            "flags {flags:#x}"
        );
        assert_eq!(keys::restart_wanted(), restart);
        assert!(reset_marked());
    }
    let first = storage::RESET.first as usize * 0x1000;
    let words = fake::with(|h| h.storage[first..first + 48].to_vec());
    assert_eq!(words, [MARKER, ATTEMPT, ATTEMPT].concat(), "two attempts");

    ERASE_FAILS.with(|f| f.set(None));
    let records = reset_boot(&mut provider, None);
    assert!(
        records.contains(&Record::ResetDone { erased: 36 }),
        "{records:?}"
    );
    assert!(!keys::restart_wanted());
    let words = fake::with(|h| h.storage[first..first + 48].to_vec());
    assert!(
        words.iter().all(|&b| b == 0xff),
        "marker and attempts cleared"
    );
}

/// A client store the writer cannot write (a newer schema reads as
/// read-only) is when an owner reaches for a reset: once the marker is down
/// the answer is `Done`, so the loop restarts, every session ends, and the
/// next boot erases the store and draws a new identity.
#[test]
fn a_reset_over_an_unwritable_client_store_still_completes() {
    use wellfed_device::store::clients::RECORD_BYTES;
    use wellfed_hal::board::storage;

    let (_guard, mut writer, mut provider) = start();
    fake::with(|h| h.adc_noise = 0x3c3c_a5a5);
    assert_eq!(rng::start().result, Ok(()));
    for _ in 0..7 {
        step(&mut writer, &mut provider);
    }
    let phone = phone();
    change(&mut writer, &mut provider, phone);

    // Every valid slot claims a newer schema.
    fake::with(|h| {
        for slot in 0..2 {
            let first = (storage::CLIENTS.first as usize + slot) * 0x1000;
            let record = &mut h.storage[first..first + RECORD_BYTES];
            if record[..4] == [0xff; 4] {
                continue;
            }
            record[8] = 0x7f;
            let crc_at = RECORD_BYTES - 8;
            let crc = wellfed_core::store::slot::crc32(&record[..crc_at]);
            record[crc_at..crc_at + 4].copy_from_slice(&crc.to_le_bytes());
        }
    });
    restart_mailboxes();
    let mut writer = Writer::default();
    let records: Vec<Record> = (0..7)
        .filter_map(|_| step(&mut writer, &mut provider))
        .collect();
    assert!(
        records.iter().any(|record| matches!(
            record,
            Record::Keys {
                clients: ClientStatus::ReadOnly,
                ..
            }
        )),
        "{records:?}"
    );

    let Record::KeysChanged {
        change: kind,
        result,
        ..
    } = change(&mut writer, &mut provider, Change::Reset)
    else {
        panic!("the reset's record");
    };
    assert_eq!((kind, result), (KeyChange::Reset, KeyResult::Done));
    assert!(reset_marked());
    let mut table = [None; wellfed_device_api::clients::CLIENTS];
    assert!(keys::clients_since(0, &mut table).is_some());
    assert!(table.iter().all(Option::is_none), "no session outlives it");

    let records = reset_boot(&mut provider, None);
    assert!(
        records.contains(&Record::ResetDone { erased: 36 }),
        "{records:?}"
    );
    assert!(
        records.iter().any(|record| matches!(
            record,
            Record::Keys {
                clients: ClientStatus::Empty,
                identity: Identity::Created,
                ..
            }
        )),
        "{records:?}"
    );
}

/// Events the loop records while the boot erases (its `Boot` and `Supply`,
/// a resolved uncertain feed) wait in the queue, and land in the new ring
/// once it opens, in order.
#[test]
fn events_recorded_during_the_reset_wait_for_the_new_ring() {
    let (_guard, mut provider) = filled_then_reset();
    restart_mailboxes();
    let mut writer = Writer::default();
    assert_eq!(step(&mut writer, &mut provider), Some(Record::ResetPending));
    assert!(events::record(None, [1; 16]));
    step(&mut writer, &mut provider);
    assert!(events::record_outcome(None, [2; 16]));

    let mut done = false;
    for _ in 0..100 {
        let record = step(&mut writer, &mut provider);
        done |= matches!(record, Some(Record::ResetDone { .. }));
        if done && events::last_seq() == Some(2) {
            break;
        }
    }
    assert!(done);
    assert_eq!(events::last_seq(), Some(2), "both landed");
    assert_eq!(events::take_lost(), 0);
}

#[test]
fn reset_records_format() {
    let line = |record: Record| {
        let mut text = String::new();
        writer::describe(record, &mut text).unwrap();
        text
    };
    assert_eq!(line(Record::ResetPending), "reset pending");
    assert_eq!(line(Record::ResetWearLost), "reset wear_lost=1");
    assert_eq!(
        line(Record::Product(FeederRecord::StatsOpen {
            status: StatsStatus::Lost,
            generation: 0,
            early: 0,
            missed: false,
        })),
        "stats open status=lost generation=0 early=0 missed=0 wear_lost=1"
    );
    assert_eq!(
        line(Record::ResetDone { erased: 36 }),
        "reset done erased=36"
    );
    assert_eq!(
        line(Record::ResetError {
            stage: ErrorStage::Erase,
            sector: Some(13),
            restart: true,
        }),
        "reset error stage=erase sector=13 restart=1"
    );
    assert_eq!(
        line(Record::ResetError {
            stage: ErrorStage::Unarmed,
            sector: None,
            restart: false,
        }),
        "reset error stage=unarmed restart=0"
    );
}

/// Flash that takes no program cannot count an attempt: the boot stops
/// the reset without asking for a restart, so dead flash never restarts
/// the feeder over and over.
#[test]
fn an_attempt_that_does_not_land_asks_for_no_restart() {
    use wellfed_hal::board::storage;

    let (_guard, mut provider) = filled_then_reset();
    PROGRAM_FAILS.with(|f| f.set(true));
    let records = reset_boot(&mut provider, Some(5));
    assert_eq!(
        records,
        [
            Record::ResetPending,
            Record::ResetError {
                stage: ErrorStage::Save,
                sector: Some(storage::RESET.first),
                restart: false,
            }
        ]
    );
    assert!(!keys::restart_wanted());
    assert!(reset_marked());

    PROGRAM_FAILS.with(|f| f.set(false));
    let records = reset_boot(&mut provider, None);
    assert!(
        records.contains(&Record::ResetDone { erased: 36 }),
        "{records:?}"
    );
}

/// Statistics whose pair takes no program (worn out) do not stop the reset
/// for good: the wear-only save fails, the pair is erased instead (only the
/// erase counts are lost), and the marker is erased. A cut after the pair's
/// first erase, or after both, still converges on the next boot.
#[test]
fn a_worn_statistics_pair_loses_the_wear_counts_but_not_the_reset() {
    use wellfed_hal::board::storage;

    let (_guard, mut provider) = filled_then_reset();
    let marked = fake::with(|h| h.storage.clone());
    STATS_PROGRAMS_FAIL.with(|f| f.set(true));

    let records = reset_boot(&mut provider, None);
    let lost = records
        .iter()
        .position(|record| *record == Record::ResetWearLost);
    let done = records
        .iter()
        .position(|record| *record == Record::ResetDone { erased: 36 });
    assert!(lost.is_some() && lost < done, "{records:?}");
    assert!(!reset_marked());
    assert!(
        sectors(storage::STATS).iter().all(|&byte| byte == 0xff),
        "the worn pair is erased, and its boot save fails"
    );

    // Cut after the pair's first erase, and after both: the next boot
    // erases what is left, or finds nothing to keep, and finishes.
    let erases = usize::try_from(storage::FACTORY_CLEARED_SECTORS).unwrap();
    for erased in 1..=storage::STATS.count {
        fake::with(|h| h.storage.clone_from(&marked));
        let records = reset_boot(&mut provider, Some(erases + 1));
        assert_eq!(records, [Record::ResetPending]);
        for sector in storage::STATS.first..storage::STATS.first + erased {
            // SAFETY: the fake's storage, which this test owns.
            assert!(unsafe { sdk::storage_erase(sector) });
        }

        let records = reset_boot(&mut provider, None);
        assert!(
            records.contains(&Record::ResetDone { erased: 36 }),
            "{erased} erased: {records:?}"
        );
        if erased == storage::STATS.count {
            assert!(!records.contains(&Record::ResetWearLost), "nothing to keep");
        }
        assert!(!reset_marked());
    }

    // A pair that takes neither its save nor its erase stops the reset,
    // as before.
    fake::with(|h| h.storage.clone_from(&marked));
    ERASE_FAILS.with(|f| f.set(Some(storage::STATS.first)));
    let records = reset_boot(&mut provider, Some(erases + 5));
    assert_eq!(
        records,
        [
            Record::ResetPending,
            Record::ResetError {
                stage: ErrorStage::Save,
                sector: Some(storage::STATS.first),
                restart: false,
            }
        ]
    );
    assert!(reset_marked());
}

/// Fill both update store sectors with bytes no build wrote, so the store
/// opens corrupt.
fn corrupt_update_store() {
    use wellfed_hal::board::storage;

    fake::with(|h| {
        for unit in 0..2 {
            let first = (storage::UPDATE.first as usize + unit) * 0x1000;
            h.storage[first..first + 64].fill(0x42);
        }
    });
}

fn update_status() -> wellfed_core::store::update::UpdateStatus {
    use wellfed_core::store::update::UpdateStore;
    use wellfed_hal::board::storage;

    let mut update = UpdateStore::default();
    update.open(wellfed_sdk::storage::Region::<
        Armed,
        { storage::UPDATE.first },
        2,
    >::new())
}

/// The Reset hold over an update store this build cannot read erases it,
/// so the owner gets back to updates without a cable: the floor is 0.
#[test]
fn a_reset_by_hand_erases_a_corrupt_update_store() {
    use wellfed_core::store::update::UpdateStatus;
    use wellfed_hal::board::storage;

    let (_guard, mut provider) = filled_then_reset();
    corrupt_update_store();
    assert_eq!(update_status(), UpdateStatus::Corrupt);

    let records = reset_boot(&mut provider, None);
    assert!(records.contains(&Record::ResetUpdateCleared), "{records:?}");
    assert!(
        records.contains(&Record::ResetDone { erased: 36 }),
        "{records:?}"
    );
    assert!(sectors(storage::UPDATE).iter().all(|&byte| byte == 0xff));
    assert_eq!(update_status(), UpdateStatus::Empty);
}

/// A reset whose marker lacks the by-hand word (one a later build asked
/// for some other way) finishes, but leaves a corrupt update store as it
/// is: only a person at the feeder may reopen a downgrade.
#[test]
fn a_reset_not_asked_by_hand_keeps_a_corrupt_update_store() {
    use wellfed_core::store::update::UpdateStatus;
    use wellfed_hal::board::storage;

    let (_guard, mut provider) = filled_then_reset();
    corrupt_update_store();
    let at = storage_reset_first();
    fake::with(|h| h.storage[at + 48..at + 64].fill(0xff));
    let before = sectors(storage::UPDATE);

    let records = reset_boot(&mut provider, None);
    assert!(
        !records.contains(&Record::ResetUpdateCleared),
        "{records:?}"
    );
    assert!(
        records.contains(&Record::ResetDone { erased: 36 }),
        "{records:?}"
    );
    assert_eq!(sectors(storage::UPDATE), before);
    assert_eq!(update_status(), UpdateStatus::Corrupt);
}
