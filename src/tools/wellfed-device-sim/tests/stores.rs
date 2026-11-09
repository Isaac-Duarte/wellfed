//! The stores every device keeps, on a simulated device with no kind's
//! policy linked: a settings change waits for the writer's turn, a client
//! change answers its ticket, and an event reaches a subscriber only
//! through the firmware's read turns, answered between passes.

use wellfed_core::settings::{SettingsFields, SettingsResult};
use wellfed_device::store::events::{BODY_BYTES, Read};
use wellfed_device_api::clients::{Label, Role};
use wellfed_device_api::mailbox::{ClientChange, KeyResult, Requested};
use wellfed_device_api::{KeyChange, Reader};
use wellfed_device_sim::flash::Flash;
use wellfed_device_sim::stores::{DeviceError, DeviceStores};
use wellfed_proto::api::{self as proto, EventBody, Path};

fn stores() -> DeviceStores {
    let mut next = 0u8;
    let random = Box::new(move || {
        next = next.wrapping_add(1);
        Some([next; 32])
    });
    DeviceStores::open(&Flash::memory(), random, ()).expect("a blank image opens")
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
        device.settings_request(change, None),
        SettingsResult::Current { pending: true, .. }
    ));
    assert_eq!(device.settings_request(change, None), SettingsResult::Busy);
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
        device.keys_request(ClientChange::Key {
            change: KeyChange::Enroll {
                key: [9; 32],
                label,
                role: Role::Owner,
                path: Path::Setup,
            },
            at: Some(1_700_000_000),
        }),
        Requested::Queued
    );
    let ticket = device.keys_ticket();
    assert_eq!(device.keys_outcome(ticket), None);

    assert_eq!(device.apply_keys(), Some(KeyResult::Done));
    assert_eq!(device.keys_outcome(ticket), Some(KeyResult::Done));
    // Its event is in the ring with the change, at the time it was asked
    // for, and kept for the loop's alert rules.
    let Read::Record(record) = device.ring_after(None) else {
        panic!("the enrollment's event is in the ring");
    };
    assert_eq!(record.at, Some(1_700_000_000));
    let enrolled = EventBody::Enrolled {
        client: [9; 4],
        role: proto::Role::Owner,
        path: Path::Setup,
    };
    assert_eq!(
        wellfed_proto::decode_padded::<EventBody>(&record.body),
        Some(enrolled)
    );
    assert_eq!(
        device.take_written(),
        Some((Some(1_700_000_000), record.body))
    );
    assert_eq!(device.take_written(), None);
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
