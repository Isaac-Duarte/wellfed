//! Events a client's change records in the writer's turn: in order, past a
//! full queue, kept with their change across a restart, and lost alone by a
//! cut between the two.

use std::sync::MutexGuard;

use wellfed_device::store::events::Read;
use wellfed_device_api::KeyChange as ApiChange;
use wellfed_device_api::clients::{Label, Role};
use wellfed_device_api::mailbox::{SettingsFields, SettingsResult, Stamp};
use wellfed_device_runtime::keys::{self, Change, Requested};
use wellfed_device_runtime::writer::{self as writer, KeyChange, KeyResult, WriteEvent};
use wellfed_device_runtime::{asked, events, network};
use wellfed_feeder_runtime::settings::SETTINGS;
use wellfed_feeder_runtime::writer::{Record, Writer};
use wellfed_hal::flash_lock::provider::Provider;
use wellfed_hal::interlock::INTERLOCK;
use wellfed_hal::rng;
use wellfed_hal::sdk::fake;
use wellfed_proto::api::{EventBody, Path, Source};

use crate::support::{
    A, ASKED_AT, Armed, EVENTS_PROGRAMS_FAIL, boot, change, changed, current, phone,
    restart_mailboxes, revoke, start, step,
};

/// The phone's enrollment event, as the writer records it.
fn phone_joined() -> EventBody {
    EventBody::Enrolled {
        client: [7; 4],
        role: wellfed_proto::api::Role::Owner,
        path: Path::Invite,
    }
}

/// The revocation's event, as the writer records it.
fn phone_revoked() -> EventBody {
    EventBody::Revoked {
        client: [7; 4],
        by: Source::Client([7, 7, 7, 7]),
    }
}

/// Every record in the ring, oldest first, read as subscriber A through
/// the writer's read turns: each its time and body.
fn ring(writer: &mut Writer<Armed>, provider: &mut Provider) -> Vec<(Option<u32>, EventBody)> {
    let mut records = Vec::new();
    let mut after = None;
    for _ in 0..256 {
        match events::read_result(A, after) {
            Some(Read::Record(record)) => {
                after = Some(record.seq);
                let body = wellfed_proto::decode_padded::<EventBody>(&record.body)
                    .expect("a body this build reads");
                records.push((record.at, body));
            }
            Some(Read::End) if events::pending().is_none() => return records,
            Some(Read::Gap { from, to }) => panic!("events {from}..={to} lost"),
            Some(Read::End) | None => {
                step(writer, provider);
            }
        }
    }
    panic!("the ring never reached its end");
}

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

/// An enrollment that goes through records its event in the turn that made
/// it, at the time it was asked for, before its result is published, so
/// no one has to read the result for the history to hold it; one that
/// changed nothing records none. It goes straight to the ring: a full
/// queue of the loop's records neither drops it nor waits for it.
#[test]
fn a_client_change_records_its_event_in_the_writers_turn() {
    let (_guard, mut writer, mut provider) = opened();
    let locked = EventBody::Lock {
        locked: true,
        by: Source::Panel,
    };
    let mut body = [0; 16];
    wellfed_proto::encode(&locked, &mut body).unwrap();
    let mut queued = 0;
    while events::record(None, body) {
        queued += 1;
    }
    assert_eq!(
        queued,
        events::QUEUE - events::RESERVED,
        "the loop's queue is full"
    );
    let last = events::last_seq();

    assert_eq!(
        change(&mut writer, &mut provider, phone()),
        changed(KeyChange::Enroll, KeyResult::Done, 1)
    );
    assert_eq!(keys::outcome(keys::ticket()), Some(KeyResult::Done));
    assert_eq!(events::last_seq(), last.map_or(Some(1), |l| Some(l + 1)));
    let Some(written) = events::take_written() else {
        panic!("the alert rules hear of it");
    };
    assert_eq!(written.at, Some(ASKED_AT));
    assert_eq!(
        wellfed_proto::decode_padded::<EventBody>(&written.body),
        Some(phone_joined())
    );
    assert_eq!(events::take_written(), None);

    assert_eq!(
        change(&mut writer, &mut provider, phone()),
        changed(KeyChange::Enroll, KeyResult::Duplicate, 1)
    );
    assert_eq!(events::take_written(), None, "a duplicate changed nothing");

    let records = ring(&mut writer, &mut provider);
    let joined: Vec<_> = records
        .iter()
        .filter(|(_, body)| *body == phone_joined())
        .collect();
    assert_eq!(joined, [&(Some(ASKED_AT), phone_joined())], "once");
    assert_eq!(
        records.len(),
        queued + 1,
        "the loop's queued records land too, after it: {records:?}"
    );
}

/// A key that joined through another client's invite records who invited
/// it just before its enrollment, both in the writer's turn; only the
/// enrollment goes to the alert rules.
#[test]
fn an_invited_key_records_its_inviter_before_its_enrollment() {
    let (_guard, mut writer, mut provider) = opened();
    let Change::Key { change: enroll, at } = phone() else {
        unreachable!("the phone's change enrolls a key");
    };
    let ApiChange::Enroll {
        key,
        label,
        role,
        path,
        ..
    } = enroll
    else {
        unreachable!("the phone's change enrolls a key");
    };
    let invited = Change::Key {
        change: ApiChange::Enroll {
            key,
            label,
            role,
            path,
            inviter: Some([5; 7]),
        },
        at,
    };

    assert_eq!(
        change(&mut writer, &mut provider, invited),
        changed(KeyChange::Enroll, KeyResult::Done, 1)
    );
    let Some(written) = events::take_written() else {
        panic!("the alert rules hear of the enrollment");
    };
    assert_eq!(
        wellfed_proto::decode_padded::<EventBody>(&written.body),
        Some(phone_joined())
    );
    assert_eq!(events::take_written(), None, "the inviter is no alert");

    let inviter = EventBody::Invited {
        client: [7; 7],
        by: [5; 7],
    };
    assert_eq!(
        ring(&mut writer, &mut provider),
        [(Some(ASKED_AT), inviter), (Some(ASKED_AT), phone_joined())]
    );
}

/// Two changes finished before the loop takes either: both are in the
/// ring, in the order they were made, and the alert rules hear of both.
#[test]
fn changes_that_overlap_each_keep_their_event() {
    let (_guard, mut writer, mut provider) = opened();
    change(&mut writer, &mut provider, phone());
    change(&mut writer, &mut provider, revoke([7; 32]));

    let taken: Vec<_> = std::iter::from_fn(events::take_written)
        .map(|written| wellfed_proto::decode_padded::<EventBody>(&written.body))
        .collect();
    assert_eq!(taken, [Some(phone_joined()), Some(phone_revoked())]);
    let bodies: Vec<_> = ring(&mut writer, &mut provider)
        .into_iter()
        .map(|(_, body)| body)
        .collect();
    assert_eq!(bodies, [phone_joined(), phone_revoked()]);
}

/// A restart before the writer takes a queued change loses the change
/// with its event; one after the turn that made it keeps both, and the
/// next boot records nothing again.
#[test]
fn a_restart_keeps_a_change_and_its_event_together() {
    let (_guard, _writer, mut provider) = opened();
    assert_eq!(keys::request(phone()), Requested::Queued);
    restart_mailboxes();
    let (mut writer, _, _) = boot(&mut provider);
    let mut table = [None; wellfed_device_api::clients::CLIENTS];
    keys::clients_since(0, &mut table).expect("the table is published");
    assert!(table.iter().all(Option::is_none), "never written");
    assert_eq!(ring(&mut writer, &mut provider), []);

    change(&mut writer, &mut provider, phone());
    restart_mailboxes();
    let (mut writer, _, _) = boot(&mut provider);
    keys::clients_since(0, &mut table).expect("the table is published");
    assert!(table.iter().flatten().any(|client| client.key == [7; 32]));
    assert_eq!(
        ring(&mut writer, &mut provider),
        [(Some(ASKED_AT), phone_joined())],
        "once, and nothing replays it"
    );
    assert_eq!(events::take_written(), None);
}

/// The store write goes before its event: a cut between the two programs
/// leaves the change standing without its event, never an event for a
/// change that did not land.
#[test]
fn a_cut_between_the_change_and_its_event_loses_only_the_event() {
    let (_guard, mut writer, mut provider) = opened();
    EVENTS_PROGRAMS_FAIL.with(|f| f.set(true));
    let Record::KeysChanged { result, event, .. } = change(&mut writer, &mut provider, phone())
    else {
        panic!("the change's record");
    };
    assert_eq!((result, event), (KeyResult::Done, WriteEvent::Lost));
    assert_eq!(events::take_written(), None);
    assert_eq!(events::take_lost(), 1, "a Lost follows into the ring");
    EVENTS_PROGRAMS_FAIL.with(|f| f.set(false));

    let mut table = [None; wellfed_device_api::clients::CLIENTS];
    keys::clients_since(0, &mut table).expect("the table is published");
    assert!(table.iter().flatten().any(|client| client.key == [7; 32]));
    assert!(
        !ring(&mut writer, &mut provider)
            .iter()
            .any(|(_, body)| *body == phone_joined())
    );

    let mut text = String::new();
    writer::describe(changed(KeyChange::Enroll, KeyResult::Done, 1), &mut text).unwrap();
    assert_eq!(
        text,
        "keys change=enroll result=done enrolled=1 event=appended"
    );
}

/// The client every slice 2 test's writes come from, and when it asked.
const ASKED: Stamp = Stamp {
    by: Source::Client([9, 9, 9, 9]),
    at: Some(ASKED_AT),
};

/// A plans table with one plan, at 07:30 every day.
fn one_plan() -> wellfed_device_api::mailbox::Table {
    use wellfed_device_api::mailbox::{PLANS, Plan, Weekdays};

    let mut table = [None; PLANS];
    table[0] = Some(Plan {
        minute: 7 * 60 + 30,
        portions: 2,
        days: Weekdays::EVERY_DAY,
        enabled: true,
        call: 0,
    });
    table
}

/// Queue a client's Wi-Fi, plans and settings change together, as three
/// sessions might.
fn queue_client_writes() {
    use wellfed_device_api::mailbox::{PlansChange, PlansResult, WifiResult};
    use wellfed_device_api::wifi::Credentials;

    let set = Credentials::new(b"net", b"password").unwrap();
    assert!(matches!(
        network::change(Some(set), Some(ASKED)),
        WifiResult::Current { pending: true, .. }
    ));
    let replace = PlansChange::Replace {
        table: one_plan(),
        at_s: Some(ASKED_AT),
    };
    assert!(matches!(
        wellfed_feeder_runtime::plans::change(replace, Some(ASKED)),
        PlansResult::Current { pending: true, .. }
    ));
    let louder = SettingsFields {
        volume: Some(7),
        ..SettingsFields::default()
    };
    assert!(matches!(
        asked::settings_request(&SETTINGS, louder, Some(ASKED)),
        SettingsResult::Current { pending: true, .. }
    ));
}

/// The events a client's Wi-Fi, plans and settings change record, in the
/// order the writer saves them.
fn client_writes_events() -> [EventBody; 3] {
    let by = ASKED.by;
    [
        EventBody::Wifi { generation: 1, by },
        EventBody::Plans { generation: 1, by },
        EventBody::Settings { generation: 1, by },
    ]
}

/// A client's Wi-Fi, plans and settings change, queued together and all
/// saved before the loop looks, each record their event in the turn that
/// saved them, with the client's time on it, straight to the ring past a
/// full queue of the loop's records.
#[test]
fn client_writes_record_their_events_in_the_writers_turn() {
    let (_guard, mut writer, mut provider) = opened();
    let locked = EventBody::Lock {
        locked: true,
        by: Source::Panel,
    };
    let mut body = [0; 16];
    wellfed_proto::encode(&locked, &mut body).unwrap();
    while events::record(None, body) {}
    let last = events::last_seq().unwrap_or(0);

    queue_client_writes();
    for _ in 0..3 {
        step(&mut writer, &mut provider);
    }
    assert!(!SETTINGS.waiting());
    assert_eq!(events::last_seq(), Some(last + 3), "past the full queue");

    assert_eq!(events::take_written(), None, "no alert rule acts on these");
    let ours: Vec<_> = ring(&mut writer, &mut provider)
        .into_iter()
        .filter(|(_, body)| *body != locked)
        .collect();
    let expected: Vec<_> = client_writes_events()
        .into_iter()
        .map(|body| (Some(ASKED_AT), body))
        .collect();
    assert_eq!(ours, expected);
}

/// A change nobody asked for records nothing: the schedule's own plans
/// change, BLE setup's Wi-Fi, settings queued without a stamp, or a
/// settings change that leaves them as they were.
#[test]
fn writes_nobody_asked_for_record_nothing() {
    use wellfed_device_api::mailbox::PlansChange;
    use wellfed_device_api::wifi::Credentials;

    let (_guard, mut writer, mut provider) = opened();
    let before = ring(&mut writer, &mut provider);

    network::save(Credentials::new(b"net", b"password").unwrap());
    step(&mut writer, &mut provider);
    let replace = PlansChange::Replace {
        table: one_plan(),
        at_s: Some(ASKED_AT),
    };
    wellfed_feeder_runtime::plans::change(replace, None);
    step(&mut writer, &mut provider);
    let unchanged = SettingsFields {
        volume: Some(current().2),
        ..SettingsFields::default()
    };
    asked::settings_request(&SETTINGS, unchanged, Some(ASKED));
    step(&mut writer, &mut provider);
    let louder = SettingsFields {
        volume: Some(7),
        ..SettingsFields::default()
    };
    SETTINGS.request(louder);
    step(&mut writer, &mut provider);
    assert_eq!(current().2, 7, "saved");

    assert_eq!(events::take_written(), None);
    assert_eq!(ring(&mut writer, &mut provider), before);
}

/// A client's settings change queued while a feed holds the outputs waits
/// for the write turn with its stamp, and records its event once the feed
/// releases them and the writer saves it.
#[test]
fn a_settings_change_that_waits_out_a_feed_keeps_its_event() {
    let (_guard, mut writer, mut provider) = opened();
    INTERLOCK.claim_feed(u32::MAX).unwrap();
    let louder = SettingsFields {
        volume: Some(7),
        ..SettingsFields::default()
    };
    asked::settings_request(&SETTINGS, louder, Some(ASKED));
    for _ in 0..5 {
        assert_eq!(step(&mut writer, &mut provider), None, "waits its turn");
    }
    assert!(SETTINGS.waiting());

    INTERLOCK.release_feed();
    let Some(Record::Saved { event, .. }) = step(&mut writer, &mut provider) else {
        panic!("saved once the feed released the outputs");
    };
    assert_eq!(event, WriteEvent::Appended);
    let by = ASKED.by;
    assert_eq!(
        ring(&mut writer, &mut provider),
        [(Some(ASKED_AT), EventBody::Settings { generation: 1, by })]
    );

    // Its stamp went with it: a later change queued with none records none.
    let quieter = SettingsFields {
        volume: Some(3),
        ..SettingsFields::default()
    };
    SETTINGS.request(quieter);
    let Some(Record::Saved { event, .. }) = step(&mut writer, &mut provider) else {
        panic!("saved");
    };
    assert_eq!(event, WriteEvent::None);
}

/// Only enrollments and revocations go back to the loop's alert rules, and
/// one past the outbox's room is counted, its event still in the ring.
#[test]
fn the_alert_outbox_takes_key_events_and_counts_an_overflow() {
    let (_guard, mut writer, mut provider) = opened();
    let enroll = |key: u8| Change::Key {
        change: ApiChange::Enroll {
            key: [key; 32],
            label: Label::new(b"phone").unwrap(),
            role: Role::Member,
            path: Path::Invite,
            inviter: None,
        },
        at: Some(ASKED_AT),
    };
    for key in 1..=u8::try_from(events::WRITTEN).unwrap() + 1 {
        change(&mut writer, &mut provider, enroll(key));
    }
    queue_client_writes();
    for _ in 0..3 {
        step(&mut writer, &mut provider);
    }

    let taken = std::iter::from_fn(events::take_written).count();
    assert_eq!(taken, events::WRITTEN);
    assert_eq!(events::take_written_lost(), 1);
    assert_eq!(events::take_written_lost(), 0, "taken once");
    let recorded = ring(&mut writer, &mut provider).len();
    assert_eq!(recorded, events::WRITTEN + 1 + 3, "every event in the ring");
}

/// A restart before the writer saves a client's changes loses them with
/// their events; one after keeps both, once, and nothing replays them.
#[test]
fn a_restart_keeps_client_writes_and_their_events_together() {
    let (_guard, _writer, mut provider) = opened();
    queue_client_writes();
    restart_mailboxes();
    let (mut writer, _, _) = boot(&mut provider);
    assert_eq!(current().3, 0, "the settings never saved");
    assert_eq!(ring(&mut writer, &mut provider), []);

    queue_client_writes();
    for _ in 0..3 {
        step(&mut writer, &mut provider);
    }
    restart_mailboxes();
    let (mut writer, _, _) = boot(&mut provider);
    assert_eq!(current().3, 1, "the settings kept");
    let bodies: Vec<_> = ring(&mut writer, &mut provider)
        .into_iter()
        .map(|(_, body)| body)
        .collect();
    assert_eq!(bodies, client_writes_events(), "once each");
}

/// The save goes before its event: a cut between the two leaves the
/// settings saved without their event, never an event for a save that did
/// not land.
#[test]
fn a_cut_after_a_client_write_loses_only_its_event() {
    let (_guard, mut writer, mut provider) = opened();
    EVENTS_PROGRAMS_FAIL.with(|f| f.set(true));
    let louder = SettingsFields {
        volume: Some(7),
        ..SettingsFields::default()
    };
    asked::settings_request(&SETTINGS, louder, Some(ASKED));
    assert!(matches!(
        step(&mut writer, &mut provider),
        Some(Record::Saved { .. })
    ));
    EVENTS_PROGRAMS_FAIL.with(|f| f.set(false));

    assert_eq!(current().2, 7);
    assert_eq!(events::take_written(), None);
    assert_eq!(ring(&mut writer, &mut provider), []);
}
