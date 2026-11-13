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
use wellfed_feeder_runtime::settings::SETTINGS;
use wellfed_feeder_runtime::stats;
use wellfed_feeder_runtime::writer::{FeederRecord, Record, Writer};
use wellfed_hal::flash::peer::MARKER_REGISTER;
use wellfed_hal::flash_lock::provider::Provider;
use wellfed_hal::rng;
use wellfed_hal::sdk::{self, fake};

use crate::support::{
    ARMED, ATTEMPT_SEEN, Armed, ERASE_FAILS, PROGRAM_FAILS, STATS_PROGRAMS_FAIL, change, changed,
    current, phone, reset_boot, reset_marked, restart_mailboxes, sectors, start, step,
    storage_reset_first,
};

/// Write an update store record with the floor `version`, as a confirmed
/// update leaves it.
fn floor_update_store(version: u32) {
    use wellfed_core::store::update::{Candidate, UpdateStore};
    use wellfed_hal::board::storage;
    use wellfed_signed::offer::ImageKind;

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

    SETTINGS.request(SettingsFields {
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
        records.contains(&Record::ResetDone { erased: 38 }),
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
            SETTINGS.request(SettingsFields::default()),
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
            records.contains(&Record::ResetDone { erased: 38 }),
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
    SETTINGS.request(SettingsFields {
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
        SETTINGS.request(SettingsFields::default()),
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
        records.contains(&Record::ResetDone { erased: 38 }),
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
        records.contains(&Record::ResetDone { erased: 38 }),
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
        line(Record::ResetDone { erased: 38 }),
        "reset done erased=38"
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
        records.contains(&Record::ResetDone { erased: 38 }),
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
        .position(|record| *record == Record::ResetDone { erased: 38 });
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
            records.contains(&Record::ResetDone { erased: 38 }),
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
        records.contains(&Record::ResetDone { erased: 38 }),
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
        records.contains(&Record::ResetDone { erased: 38 }),
        "{records:?}"
    );
    assert_eq!(sectors(storage::UPDATE), before);
    assert_eq!(update_status(), UpdateStatus::Corrupt);
}

/// The reset erases the meal call's sectors without taking the clip hold
/// the settings writer takes, and needs none: it runs on a boot whose
/// speaker starts with no clip, and the writer publishes nothing until it
/// is done, so no call can start and read the sectors. The speaker keeps
/// the boot before's settings view here, so only the missing clip refuses
/// the call.
#[test]
fn no_call_plays_while_a_reset_erases_its_clip() {
    use wellfed_core::store::slot::crc32;
    use wellfed_device_api::mailbox::{CallChange, CallClip, CallResult};
    use wellfed_feeder_runtime::call;
    use wellfed_hal::audio::{self, PlayResult};
    use wellfed_hal::board::storage;

    let (_guard, mut writer, mut provider) = start();
    restart_mailboxes();
    audio::reset();
    fake::with(|h| h.adc_noise = 0x0f0f_5a5a);
    assert_eq!(rng::start().result, Ok(()));
    for _ in 0..7 {
        step(&mut writer, &mut provider);
    }
    let stream: Vec<u8> = (0..600u32).map(|i| (i * 7) as u8).collect();
    let crc = crc32(&stream);
    assert!(matches!(
        call::change(CallChange::Start { bytes: 600, crc }),
        CallResult::Current { pending: true, .. }
    ));
    step(&mut writer, &mut provider);
    assert_eq!(call::chunk(0, &stream[..512]), Ok(()));
    step(&mut writer, &mut provider);
    assert_eq!(call::chunk(512, &stream[512..]), Ok(()));
    assert_eq!(
        step(&mut writer, &mut provider),
        Some(Record::Product(FeederRecord::Call {
            clip: CallClip::Stored { bytes: 600, crc }
        }))
    );
    let phone = phone();
    change(&mut writer, &mut provider, phone);
    assert_eq!(
        change(&mut writer, &mut provider, Change::Reset),
        changed(KeyChange::Reset, KeyResult::Done, 0)
    );
    assert!(reset_marked());

    // The restart: a speaker with no clip, but the old settings view.
    let sound = wellfed_hal::settings::current().expect("the settings are published");
    restart_mailboxes();
    audio::reset();
    wellfed_hal::settings::publish(sound);
    let mut writer = Writer::default();
    let mut done = false;
    for taken in 0..60 {
        let record = step(&mut writer, &mut provider);
        if matches!(record, Some(Record::ResetDone { .. })) {
            done = true;
            break;
        }
        assert_eq!(audio::request_call(1), PlayResult::Busy, "step {taken}");
    }
    assert!(done);
    assert!(sectors(storage::CALL).iter().all(|&byte| byte == 0xff));
    assert_eq!(audio::request_call(1), PlayResult::Busy, "no clip after");
}

/// The feeder task's period between watchdog checkpoints, in FreeRTOS
/// ticks (the runtime's `FEED_MS`).
const FEED_TICKS: u32 = 250;
/// One erase lease with IRQs masked, as installs measured it on the bench
/// (`masked_max_ms` 42 to 46), and one page program's.
const ERASE_MASKED_MS: u32 = 43;
const PROGRAM_MASKED_MS: u32 = 3;

/// The service task stepping a reset boot's writer beside the watchdog
/// feeder, on two clocks: the system timer the watchdog's progress gap is
/// measured on, and the FreeRTOS tick the feeder waits in. A lease masks
/// IRQs, so its time passes on the system timer while the tick stands
/// (one pended tick at the unmask); the writer's waits pass on both. The
/// feeder, the highest priority, checkpoints as soon as its period has
/// passed in ticks. Runs the boot to `ResetDone`; its records and the
/// longest gap between checkpoints on the system timer.
fn reset_boot_beside_the_feeder(provider: &mut Provider, paced: bool) -> (Vec<Record>, u32) {
    use crate::support::{LEASES, Unreached};
    use wellfed_hal::supervision;

    restart_mailboxes();
    if paced {
        supervision::pace_writes();
    }
    let mut writer = Writer::<Armed>::default();
    let mut clocks = Clocks::default();

    let mut records = Vec::new();
    for _ in 0..10_000 {
        // A feeder already due runs before the next step.
        if clocks.tick - clocks.fed_tick >= FEED_TICKS {
            clocks.feed();
        }

        LEASES.with(|l| l.set((0, 0)));
        // SAFETY: the provider outlives the run.
        let step = unsafe { writer.step::<Unreached, Armed>(provider) };
        let (erases, programs) = LEASES.with(Cell::get);
        let masked = erases * ERASE_MASKED_MS + programs * PROGRAM_MASKED_MS;
        clocks.wall += masked;
        clocks.tick += u32::from(masked > 0);
        let done = matches!(step.record, Some(Record::ResetDone { .. }));
        records.extend(step.record);
        if done {
            return (records, clocks.worst);
        }

        // The step's wait, unmasked: the feeder runs at each period's end.
        let mut wait = step.delay_ms.max(1);
        while clocks.tick - clocks.fed_tick + wait >= FEED_TICKS {
            let to_due = FEED_TICKS.saturating_sub(clocks.tick - clocks.fed_tick);
            clocks.wall += to_due;
            clocks.tick += to_due;
            wait -= to_due;
            clocks.feed();
        }
        clocks.wall += wait;
        clocks.tick += wait;
    }
    panic!("the reset never finished: {records:?}");
}

/// [`reset_boot_beside_the_feeder`]'s clocks, in milliseconds.
#[derive(Default)]
struct Clocks {
    /// The system timer.
    wall: u32,
    /// The FreeRTOS tick.
    tick: u32,
    fed_wall: u32,
    fed_tick: u32,
    /// The longest gap between checkpoints on the system timer.
    worst: u32,
}

impl Clocks {
    fn feed(&mut self) {
        self.worst = self.worst.max(self.wall - self.fed_wall);
        self.fed_wall = self.wall;
        self.fed_tick = self.tick;
        wellfed_hal::supervision::checkpointed();
    }
}

/// The long erase path of a Reset hold over an unreadable update store
/// (simulation): forty erases a millisecond apart hold the feeder off its
/// checkpoint past the watchdog's progress gap, as on a beta feeder
/// (`watchdog_fault=4 gap_ms=1063`); paced by the feeder's checkpoints,
/// one feed period holds one step's leases, well inside the gap.
#[test]
fn a_reset_over_a_corrupt_update_store_keeps_the_watchdog_fed() {
    use wellfed_hal::supervision::config::WATCHDOG;

    let (_guard, mut provider) = filled_then_reset();
    corrupt_update_store();
    let image = fake::with(|h| h.storage.clone());

    let (_, unpaced) = reset_boot_beside_the_feeder(&mut provider, false);
    assert!(
        unpaced > WATCHDOG.max_progress_gap_ms,
        "the model reproduces the fault: {unpaced} ms"
    );

    fake::with(|h| h.storage = image);
    let (records, paced) = reset_boot_beside_the_feeder(&mut provider, true);
    assert!(records.contains(&Record::ResetUpdateCleared), "{records:?}");
    assert_eq!(
        records.last(),
        Some(&Record::ResetDone { erased: 38 }),
        "{records:?}"
    );
    // A feed period and the update store's two erases, at most.
    assert!(
        paced <= FEED_TICKS + 2 * ERASE_MASKED_MS + 1,
        "the longest gap between checkpoints: {paced} ms"
    );
}
