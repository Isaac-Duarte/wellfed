//! The stores every device keeps, on a simulated device with no kind's
//! policy linked: a settings change waits for the writer's turn, a client
//! change answers its ticket, and an event reaches a subscriber only
//! through the firmware's read turns, answered between passes.

use wellfed_device::store::events::{BODY_BYTES, Read};
use wellfed_device_api::Reader;
use wellfed_device_api::clients::{Label, Role};
use wellfed_device_api::mailbox::{
    ClientChange, KeyResult, Requested, SettingsFields, SettingsResult,
};
use wellfed_device_sim::flash::Flash;
use wellfed_device_sim::stores::{DeviceError, DeviceStores};

fn stores() -> DeviceStores {
    let mut next = 0u8;
    let random = Box::new(move || {
        next = next.wrapping_add(1);
        Some([next; 32])
    });
    DeviceStores::open(&Flash::memory(), random).expect("a blank image opens")
}

#[test]
fn a_settings_change_waits_for_the_writers_turn() {
    let mut device = stores();
    let before = device.settings().generation;

    let change = SettingsFields {
        volume: Some(20),
        ..SettingsFields::default()
    };
    assert!(matches!(
        device.settings_request(change),
        SettingsResult::Current { pending: true, .. }
    ));
    assert_eq!(device.settings_request(change), SettingsResult::Busy);
    assert_eq!(
        device.settings().generation,
        before,
        "queued, not yet saved"
    );

    device.apply_settings();
    let view = device.settings();
    assert_eq!((view.volume, view.generation), (20, before + 1));
}

#[test]
fn a_client_change_answers_its_ticket_on_the_writers_turn() {
    let mut device = stores();
    let label = Label::new(b"phone").expect("a short label");
    assert_eq!(
        device.keys_request(ClientChange::Enroll {
            key: [9; 32],
            label,
            role: Role::Owner,
        }),
        Requested::Queued
    );
    let ticket = device.keys_ticket();
    assert_eq!(device.keys_outcome(ticket), None);

    assert_eq!(device.apply_keys(), Some(KeyResult::Done));
    assert_eq!(device.keys_outcome(ticket), Some(KeyResult::Done));
    assert_eq!(
        device.enroll([9; 32], label, Role::Member),
        Err(DeviceError::EnrolledAs(Role::Owner)),
        "an enrolled key keeps its role"
    );
}

#[test]
fn an_event_reaches_a_subscriber_through_the_read_turns() {
    let mut device = stores();
    let reader = Reader::slot(0);
    assert!(device.queue_event(Some(1), &[7; BODY_BYTES]));

    // The first ask queues the read; the writer answers it between passes.
    assert_eq!(device.event_after(reader, None), None);
    device.answer_read();
    let Some(Read::Record(record)) = device.event_after(reader, None) else {
        panic!("the answered read gives the record");
    };
    assert_eq!(record.body, [7; BODY_BYTES]);
    assert_eq!(device.last_event(), Some(record.seq));
}
