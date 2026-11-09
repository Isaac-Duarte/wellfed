//! The client table and the feeder's identity: the seed, a table this build
//! cannot read, key changes, and a reset that needs the generator.

use std::cell::Cell;

use wellfed_core::store::seed::SeedStatus;
use wellfed_device::store::clients::ClientStatus;
use wellfed_device_runtime::keys::{self, Change};
use wellfed_device_runtime::writer::{self as writer, Identity, KeyChange, KeyResult};
use wellfed_feeder_runtime::writer::{Record, Writer};
use wellfed_hal::flash::peer::MARKER_REGISTER;
use wellfed_hal::flash_lock::provider::Provider;
use wellfed_hal::rng;
use wellfed_hal::sdk::{self, fake};

use crate::support::{
    ARMED, ATTEMPT_SEEN, change, changed, phone, reset_marked, revoke, start, step,
};

/// The writer's first records on one boot: open, net open, time open, then
/// keys.
fn boot_keys() -> Record {
    let (mut writer, mut provider) = (Writer::default(), Provider::default());
    step(&mut writer, &mut provider);
    step(&mut writer, &mut provider);
    step(&mut writer, &mut provider);
    step(&mut writer, &mut provider).expect("the keys record")
}

#[test]
fn keys_carry_the_seed_and_create_the_identity_once() {
    let (_guard, _, _) = start();
    rng::reset();
    assert_eq!(
        boot_keys(),
        Record::Keys {
            seed: SeedStatus::Empty,
            mixed: false,
            saved: false,
            clients: ClientStatus::Empty,
            enrolled: 0,
            identity: Identity::Missing,
        },
        "no generator: nothing written"
    );

    fake::with(|h| h.adc_noise = 0x2468_ace1);
    assert_eq!(rng::start().result, Ok(()));
    let first = boot_keys();
    assert!(matches!(
        first,
        Record::Keys {
            seed: SeedStatus::Empty,
            mixed: false,
            saved: true,
            identity: Identity::Created,
            ..
        }
    ));
    assert!(ATTEMPT_SEEN.with(Cell::get));
    assert_eq!(sdk::backup_read(MARKER_REGISTER), 0);

    // The next boot finds both, and stores another seed.
    assert!(matches!(
        boot_keys(),
        Record::Keys {
            seed: SeedStatus::Loaded,
            mixed: true,
            saved: true,
            clients: ClientStatus::Loaded,
            identity: Identity::Loaded,
            ..
        }
    ));
}

/// A client table this build cannot read (here an older major) is never
/// written over: the boot draws no new identity, which would orphan every
/// client the build that wrote it paired, and the sectors stay as they
/// were until a factory reset erases them.
#[test]
fn an_unreadable_client_table_never_gets_a_new_identity() {
    use wellfed_device::store::clients::RECORD_BYTES;
    use wellfed_hal::board::storage;

    let (_guard, _, _) = start();
    fake::with(|h| h.adc_noise = 0x2468_ace1);
    assert_eq!(rng::start().result, Ok(()));
    assert!(matches!(
        boot_keys(),
        Record::Keys {
            identity: Identity::Created,
            ..
        }
    ));

    let sectors = |h: &mut fake::Hardware| {
        let first = storage::CLIENTS.first as usize * 0x1000;
        h.storage[first..first + 2 * 0x1000].to_vec()
    };
    fake::with(|h| {
        let first = storage::CLIENTS.first as usize * 0x1000;
        let record = &mut h.storage[first..first + RECORD_BYTES];
        record[8] = 0;
        let crc_at = RECORD_BYTES - 8;
        let crc = wellfed_core::store::slot::crc32(&record[..crc_at]);
        record[crc_at..crc_at + 4].copy_from_slice(&crc.to_le_bytes());
    });
    let before = fake::with(sectors);

    assert!(matches!(
        boot_keys(),
        Record::Keys {
            clients: ClientStatus::Corrupt,
            enrolled: 0,
            identity: Identity::Missing,
            ..
        }
    ));
    assert_eq!(fake::with(sectors), before, "the table untouched");
}

#[test]
fn key_changes_apply_and_a_reset_publishes_a_new_identity() {
    let (_guard, mut writer, mut provider) = start();
    fake::with(|h| h.adc_noise = 0x1357_9bdf);
    assert_eq!(rng::start().result, Ok(()));
    for _ in 0..7 {
        step(&mut writer, &mut provider); // open, net open, time open, keys, events, stats, its save
    }
    let (first, identity) = keys::identity_since(0).expect("the identity is published");

    let phone = phone();
    assert_eq!(
        change(&mut writer, &mut provider, phone),
        changed(KeyChange::Enroll, KeyResult::Done, 1)
    );
    let enrolled = keys::ticket();
    assert_eq!(keys::outcome(enrolled), Some(KeyResult::Done));
    assert!(ATTEMPT_SEEN.with(Cell::get));
    assert_eq!(sdk::backup_read(MARKER_REGISTER), 0);
    assert_eq!(
        change(&mut writer, &mut provider, phone),
        changed(KeyChange::Enroll, KeyResult::Duplicate, 1)
    );
    // A caller holding the earlier ticket still reads its own outcome.
    assert_eq!(keys::outcome(enrolled), Some(KeyResult::Done));
    assert_eq!(keys::outcome(keys::ticket()), Some(KeyResult::Duplicate));
    assert_eq!(
        keys::identity_since(first),
        None,
        "an enrollment keeps the identity"
    );

    let revoke = revoke([8; 32]);
    assert_eq!(
        change(&mut writer, &mut provider, revoke),
        changed(KeyChange::Revoke, KeyResult::NotFound, 1)
    );

    assert_eq!(
        change(&mut writer, &mut provider, Change::Reset),
        changed(KeyChange::Reset, KeyResult::Done, 0)
    );
    let (_, new) = keys::identity_since(first).expect("the new identity");
    assert_ne!(new, identity);
    assert!(reset_marked(), "the next boot finishes the factory reset");
}

#[test]
fn a_reset_needs_the_generator_and_an_armed_writer() {
    let (_guard, mut writer, mut provider) = start();
    fake::with(|h| h.adc_noise = 0x0bad_cafe);
    assert_eq!(rng::start().result, Ok(()));
    for _ in 0..7 {
        step(&mut writer, &mut provider);
    }
    let (first, _) = keys::identity_since(0).unwrap();

    rng::reset();
    assert_eq!(
        change(&mut writer, &mut provider, Change::Reset),
        changed(KeyChange::Reset, KeyResult::NoGenerator, 0)
    );

    assert_eq!(rng::start().result, Ok(()));
    ARMED.with(|a| a.set(false));
    assert_eq!(
        change(&mut writer, &mut provider, Change::Reset),
        changed(KeyChange::Reset, KeyResult::Unarmed, 0)
    );
    assert_eq!(keys::identity_since(first), None, "the identity stands");
}

#[test]
fn key_change_records_format() {
    let mut text = String::new();
    writer::describe(
        changed(KeyChange::Reset, KeyResult::NoGenerator, 0),
        &mut text,
    )
    .unwrap();
    assert_eq!(
        text,
        "keys change=reset result=no-generator enrolled=0 event=none"
    );
}
