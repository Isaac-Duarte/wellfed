//! The handover boot: what it clears and keeps, the update floor and the
//! confirmation, and how a cut or an unreadable store makes it wait.

use std::sync::MutexGuard;

use wellfed_core::store::handover;
use wellfed_device_runtime::keys::{self};
use wellfed_device_runtime::network;
use wellfed_device_runtime::writer::Identity;
use wellfed_feeder_runtime::writer::Record;
use wellfed_hal::flash_lock::provider::Provider;
use wellfed_hal::rng;
use wellfed_hal::sdk::fake;

use crate::support::{
    Armed, PROGRAM_FAILS, change, phone, reset_boot, sectors, start, step, storage_reset_first,
};

/// A paired feeder on Wi-Fi with one more boot to go: the identity it has.
fn paired_with_wifi() -> (MutexGuard<'static, ()>, Provider, [u8; 32]) {
    use wellfed_device_api::mailbox::{PLANS, Plan, PlansChange, Weekdays};
    use wellfed_device_api::wifi::Credentials;
    use wellfed_feeder_runtime::plans;

    let (guard, mut writer, mut provider) = start();
    fake::with(|h| h.adc_noise = 0x0f0f_5a5a);
    assert_eq!(rng::start().result, Ok(()));
    for _ in 0..7 {
        step(&mut writer, &mut provider);
    }
    network::change(
        Some(Credentials::new(b"home", b"passphrase").unwrap()),
        None,
    );
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
    assert!(
        sectors(wellfed_hal::board::storage::PLANS)
            .iter()
            .any(|&b| b != 0xff)
    );
    let phone = phone();
    change(&mut writer, &mut provider, phone);
    let (_, identity) = keys::identity_since(0).expect("the identity is published");
    (guard, provider, identity)
}

fn plant_handover() {
    let at = storage_reset_first();
    fake::with(|h| h.storage[at..at + 16].copy_from_slice(&handover::MARKER));
}

/// The boot that finds the handover mark clears the Wi-Fi, plans and clients,
/// keeps the identity the backup is filed by, publishes nothing stale, and
/// erases the mark, so a later boot keeps whatever pairing comes next.
#[test]
fn a_handover_clears_wifi_and_clients_and_keeps_the_identity() {
    use wellfed_hal::board::storage;

    let (_guard, mut provider, identity) = paired_with_wifi();
    plant_handover();

    let records = reset_boot(&mut provider, None);

    assert!(
        records.contains(&Record::HandoverDone { cleared: true }),
        "{records:?}"
    );
    assert!(sectors(storage::NET).iter().all(|&byte| byte == 0xff));
    assert!(sectors(storage::PLANS).iter().all(|&byte| byte == 0xff));
    assert!(sectors(storage::RESET).iter().all(|&byte| byte == 0xff));
    assert_eq!(keys::identity_since(0).map(|(_, key)| key), Some(identity));
    assert!(
        records.iter().any(|record| matches!(
            record,
            Record::Keys {
                enrolled: 0,
                identity: Identity::Loaded,
                ..
            }
        )),
        "{records:?}"
    );
}

/// The floor holds across an ordinary boot, and only the handover forgets it.
#[test]
fn a_handover_forgets_the_update_floor_and_an_ordinary_boot_does_not() {
    use wellfed_core::store::update::{Candidate, UpdateStore};
    use wellfed_hal::board::storage;
    use wellfed_signed::offer::ImageKind;

    type Slots = wellfed_sdk::storage::Region<Armed, { storage::UPDATE.first }, 2>;
    let floor = || {
        let mut update = UpdateStore::default();
        update.open(Slots::new());
        update.record().min_version
    };

    let (_guard, mut provider, _) = paired_with_wifi();
    let mut update = UpdateStore::default();
    update.open(Slots::new());
    let build = [0x55; 32];
    update
        .set_activated(Candidate {
            digest: [1; 32],
            version: 5,
            build_id: build,
            kind: ImageKind::Replacement,
        })
        .unwrap();
    update.confirm(&build).unwrap();

    reset_boot(&mut provider, None);
    assert_eq!(floor(), 5);

    plant_handover();
    let records = reset_boot(&mut provider, None);
    assert!(
        records.contains(&Record::HandoverDone { cleared: true }),
        "{records:?}"
    );
    assert_eq!(floor(), 0);
}

/// Bank 0 forgets the confirmation in a handover, so a reinstall of the same
/// build runs a real trial. Bank 1 keeps it: its next boot would otherwise run
/// a trial with no Wi-Fi left to pass one. An ordinary boot leaves it alone.
#[test]
fn only_a_handover_on_bank_0_forgets_the_confirmation() {
    use wellfed_core::ota::{self, CONFIRM_LEN};
    use wellfed_hal::board::storage;

    let confirmed = ota::confirm_record(&[0x55; 32], 5);
    let plant = || {
        let at = storage::CONFIRM.first as usize * 0x1000;
        fake::with(|h| h.storage[at..at + CONFIRM_LEN].copy_from_slice(&confirmed));
    };
    let held = || sectors(storage::CONFIRM)[..CONFIRM_LEN] == confirmed;

    let (_guard, mut provider, _) = paired_with_wifi();
    fake::with(|h| h.running_bank = Some(1));
    plant();
    plant_handover();
    let records = reset_boot(&mut provider, None);
    assert!(
        records.contains(&Record::HandoverDone { cleared: true }),
        "{records:?}"
    );
    assert!(held(), "bank 1 keeps its confirmation");

    fake::with(|h| h.running_bank = Some(0));
    reset_boot(&mut provider, None);
    assert!(held(), "no mark, nothing forgotten");

    plant_handover();
    let records = reset_boot(&mut provider, None);
    assert!(
        records.contains(&Record::HandoverDone { cleared: true }),
        "{records:?}"
    );
    assert!(sectors(storage::CONFIRM).iter().all(|&byte| byte == 0xff));
}

/// A cut at each step of the clearing leaves the mark, and the next boot does
/// the whole of it, keeping the identity. Bank 0 forgets the confirmation
/// whichever side of its erase the cut lands; bank 1 keeps it.
#[test]
fn a_cut_during_a_handover_repeats_it() {
    use wellfed_core::ota::{self, CONFIRM_LEN};
    use wellfed_hal::board::storage;

    let confirmed = ota::confirm_record(&[0x55; 32], 5);
    for bank in [0, 1] {
        let (mut before, mut after) = (0, 0);
        // Every step but the mark: the cleared sectors, the clients, the
        // update store and the confirmation.
        let steps = storage::HANDOVER_CLEARED_SECTORS as usize + 4;
        for cut in 1..=steps {
            let (guard, mut provider, identity) = paired_with_wifi();
            fake::with(|h| h.running_bank = Some(bank));
            let at = storage::CONFIRM.first as usize * 0x1000;
            fake::with(|h| h.storage[at..at + CONFIRM_LEN].copy_from_slice(&confirmed));
            plant_handover();

            let first = reset_boot(&mut provider, Some(cut));
            assert!(
                !first.contains(&Record::HandoverDone { cleared: true }),
                "the mark goes last: {first:?}"
            );
            if sectors(storage::CONFIRM)[..CONFIRM_LEN] == confirmed {
                before += 1;
            } else {
                after += 1;
            }
            let second = reset_boot(&mut provider, None);

            assert!(
                second.contains(&Record::HandoverDone { cleared: true }),
                "after {cut} steps: {second:?}"
            );
            assert!(sectors(storage::NET).iter().all(|&byte| byte == 0xff));
            assert_eq!(keys::identity_since(0).map(|(_, key)| key), Some(identity));
            if bank == 0 {
                assert!(
                    sectors(storage::CONFIRM).iter().all(|&byte| byte == 0xff),
                    "after {cut} steps"
                );
            } else {
                assert_eq!(sectors(storage::CONFIRM)[..CONFIRM_LEN], confirmed);
            }
            drop(guard);
        }
        assert!(before > 0, "bank {bank}: a cut before the erase");
        if bank == 0 {
            assert!(after > 0, "a cut after the erase and before the mark");
        } else {
            assert_eq!(after, 0, "bank 1 never erases it");
        }
    }
}

/// An erase that fails leaves the mark for the next boot, and this boot goes
/// on rather than serving nothing.
#[test]
fn a_handover_that_cannot_finish_waits_for_the_next_boot() {
    let (_guard, mut provider, _) = paired_with_wifi();
    plant_handover();
    PROGRAM_FAILS.with(|f| f.set(true));

    let records = reset_boot(&mut provider, None);

    assert!(
        records
            .iter()
            .any(|record| matches!(record, Record::HandoverStopped { .. })),
        "{records:?}"
    );
    let at = storage_reset_first();
    assert_eq!(
        fake::with(|h| h.storage[at..at + 16].to_vec()),
        handover::MARKER
    );
}

/// A client store that cannot be read may still hold clients, so the mark
/// stays for the next boot, which clears them.
#[test]
fn a_handover_waits_for_a_client_store_it_can_read() {
    use wellfed_hal::board::storage;

    for unit in 0..storage::CLIENTS.count {
        let (_guard, mut provider, identity) = paired_with_wifi();
        plant_handover();
        fake::with(|h| h.fail_storage_read = Some(storage::CLIENTS.first + unit));

        let stopped = reset_boot(&mut provider, None);

        assert!(
            stopped
                .iter()
                .any(|record| matches!(record, Record::HandoverStopped { stage }
                    if u32::from(*stage) == storage::HANDOVER_CLEARED_SECTORS)),
            "unit {unit}: {stopped:?}"
        );
        let at = storage_reset_first();
        assert_eq!(
            fake::with(|h| h.storage[at..at + 16].to_vec()),
            handover::MARKER,
            "unit {unit}"
        );

        fake::with(|h| h.fail_storage_read = None);
        let finished = reset_boot(&mut provider, None);

        assert!(
            finished.contains(&Record::HandoverDone { cleared: true }),
            "unit {unit}: {finished:?}"
        );
        assert_eq!(keys::identity_since(0).map(|(_, key)| key), Some(identity));
        assert!(
            finished.iter().any(|record| matches!(
                record,
                Record::Keys {
                    enrolled: 0,
                    identity: Identity::Loaded,
                    ..
                }
            )),
            "unit {unit}: {finished:?}"
        );
    }
}

/// An update store that cannot be read may still hold its floor, so the mark
/// stays for the next boot, which resets it.
#[test]
fn a_handover_waits_for_an_update_store_it_can_read() {
    use wellfed_core::store::update::{Candidate, UpdateStore};
    use wellfed_hal::board::storage;
    use wellfed_signed::offer::ImageKind;

    type Slots = wellfed_sdk::storage::Region<Armed, { storage::UPDATE.first }, 2>;

    let (_guard, mut provider, _) = paired_with_wifi();
    let mut update = UpdateStore::default();
    update.open(Slots::new());
    let build = [0x55; 32];
    update
        .set_activated(Candidate {
            digest: [1; 32],
            version: 5,
            build_id: build,
            kind: ImageKind::Replacement,
        })
        .unwrap();
    update.confirm(&build).unwrap();
    plant_handover();
    fake::with(|h| h.fail_storage_read = Some(storage::UPDATE.first));

    let stopped = reset_boot(&mut provider, None);

    assert!(
        stopped
            .iter()
            .any(|record| matches!(record, Record::HandoverStopped { stage }
                if u32::from(*stage) == storage::HANDOVER_CLEARED_SECTORS + 1)),
        "{stopped:?}"
    );
    let at = storage_reset_first();
    assert_eq!(
        fake::with(|h| h.storage[at..at + 16].to_vec()),
        handover::MARKER
    );

    fake::with(|h| h.fail_storage_read = None);
    let finished = reset_boot(&mut provider, None);

    assert!(finished.contains(&Record::HandoverDone { cleared: true }));
    let mut update = UpdateStore::default();
    update.open(Slots::new());
    assert_eq!(update.record().min_version, 0);
}
