//! The settings, Wi-Fi and time settings stores: each change saved once
//! inside a marked attempt and published, and an unwritable store that
//! still reads.

use std::cell::Cell;

use wellfed_device::store::settings::SettingsStatus;
use wellfed_device::store::time::TimeStatus;
use wellfed_device_api::mailbox::{SettingsFields, SettingsResult};
use wellfed_device_runtime::writer::{self as writer, WriteEvent};
use wellfed_device_runtime::{events, network};
use wellfed_feeder::store::stats::StatsStatus;
use wellfed_feeder_runtime::settings::SETTINGS;
use wellfed_feeder_runtime::writer::{FeederRecord, Record, Writer};
use wellfed_hal::flash::peer::MARKER_REGISTER;
use wellfed_hal::sdk::{self, fake};
use wellfed_sdk::storage::SETTINGS_BASE;

use crate::support::{ATTEMPT_SEEN, current, start, step};

#[test]
fn changes_are_saved_once_each_inside_a_marked_attempt() {
    let (_guard, mut writer, mut provider) = start();
    assert_eq!(
        SETTINGS.request(SettingsFields::default()),
        SettingsResult::Busy,
        "nothing published before the store opens"
    );

    let Some(Record::Open {
        status, generation, ..
    }) = step(&mut writer, &mut provider)
    else {
        panic!("open first")
    };
    assert_eq!((status, generation), (SettingsStatus::Empty, 0));
    assert_eq!(current(), (true, true, 50, 0, true));
    assert!(matches!(
        step(&mut writer, &mut provider),
        Some(Record::NetOpen { stored: false, .. })
    ));
    assert_eq!(
        step(&mut writer, &mut provider),
        Some(Record::TimeOpen {
            status: TimeStatus::Empty,
            generation: 0
        })
    );
    assert!(matches!(
        step(&mut writer, &mut provider),
        Some(Record::Keys { .. })
    ));
    assert_eq!(
        step(&mut writer, &mut provider),
        Some(Record::Events {
            open: true,
            last: None,
            oldest: None,
            epoch: events::epoch().unwrap(),
        }),
        "the event ring opens empty"
    );
    assert_eq!(
        step(&mut writer, &mut provider),
        Some(Record::Product(FeederRecord::StatsOpen {
            status: StatsStatus::Empty,
            generation: 0,
            early: 0,
            missed: false,
        }))
    );
    assert_eq!(
        step(&mut writer, &mut provider),
        Some(Record::Product(FeederRecord::StatsSaved {
            generation: 1,
            early: 1
        })),
        "the boot's own save, so its reset counts"
    );
    assert_eq!(
        step(&mut writer, &mut provider),
        None,
        "idle without a change"
    );

    let change = SettingsFields {
        lights: Some(false),
        volume: Some(20),
        ..Default::default()
    };
    assert!(matches!(
        SETTINGS.request(change),
        SettingsResult::Current { pending: true, .. }
    ));
    assert_eq!(
        SETTINGS.request(change),
        SettingsResult::Busy,
        "one change at a time"
    );

    let Some(Record::Saved {
        generation,
        settings: saved,
        ..
    }) = step(&mut writer, &mut provider)
    else {
        panic!("saved")
    };
    assert_eq!(
        (generation, saved.lights, saved.sound, saved.volume),
        (1, false, true, 20)
    );
    assert!(ATTEMPT_SEEN.with(Cell::get));
    assert_eq!(
        sdk::backup_read(MARKER_REGISTER),
        0,
        "the attempt is cleared after"
    );
    assert_eq!(current(), (false, true, 20, 1, true));
    assert_eq!(SETTINGS.pending(), None);

    let base = SETTINGS_BASE as usize * 4096;
    assert_eq!(
        fake::with(|h| h.storage[base..base + 8].to_vec()),
        b"PLFSET01"
    );

    // The next change goes to the other slot; a reopen sees it.
    SETTINGS.request(SettingsFields {
        sound: Some(false),
        ..Default::default()
    });
    assert!(matches!(
        step(&mut writer, &mut provider),
        Some(Record::Saved { generation: 2, .. })
    ));
    let mut reopened = Writer::default();
    SETTINGS.reset();
    assert!(matches!(
        step(&mut reopened, &mut provider),
        Some(Record::Open { generation: 2, .. })
    ));
    assert_eq!(current(), (false, false, 20, 2, true));
}

#[test]
fn an_unwritable_store_reads_but_refuses_changes() {
    let (_guard, mut writer, mut provider) = start();
    // Garbage in both slots: CORRUPT, defaults, read-only until a reset.
    let base = SETTINGS_BASE as usize * 4096;
    fake::with(|h| h.storage[base..base + 2 * 4096].fill(0x5a));
    let Some(Record::Open { status, .. }) = step(&mut writer, &mut provider) else {
        panic!("open")
    };
    assert_eq!(status, SettingsStatus::Corrupt);
    assert!(!current().4);
    assert_eq!(
        SETTINGS.request(SettingsFields {
            lights: Some(false),
            ..Default::default()
        }),
        SettingsResult::ReadOnly
    );
    assert_eq!(fake::with(|h| h.storage[base]), 0x5a, "nothing written");
}

#[test]
fn wifi_credentials_are_saved_inside_a_marked_attempt_and_cleared() {
    use wellfed_device_api::mailbox::WifiResult;
    use wellfed_device_api::wifi::Credentials;
    let (_guard, mut writer, mut provider) = start();
    step(&mut writer, &mut provider); // open
    step(&mut writer, &mut provider); // net open
    step(&mut writer, &mut provider); // time open
    step(&mut writer, &mut provider); // keys
    step(&mut writer, &mut provider); // events
    step(&mut writer, &mut provider); // stats
    step(&mut writer, &mut provider); // the boot's stats save

    let set = Credentials::new(b"net", b"password").unwrap();
    assert!(matches!(
        network::change(Some(set), None),
        WifiResult::Current { pending: true, .. }
    ));
    assert_eq!(
        step(&mut writer, &mut provider),
        Some(Record::NetSaved {
            generation: 1,
            stored: true,
            event: WriteEvent::None,
        })
    );
    assert!(ATTEMPT_SEEN.with(Cell::get));
    assert_eq!(sdk::backup_read(MARKER_REGISTER), 0);
    assert!(matches!(
        network::current(),
        WifiResult::Current { view, pending: false } if view.stored_ssid_crc.is_some()
    ));

    network::change(None, None);
    assert_eq!(
        step(&mut writer, &mut provider),
        Some(Record::NetSaved {
            generation: 2,
            stored: false,
            event: WriteEvent::None,
        })
    );
}

#[test]
fn time_settings_are_saved_inside_a_marked_attempt_and_published() {
    use wellfed_device_api::mailbox::{NtpServer, TimeSettings, TimeSettingsResult, Zone};
    use wellfed_device_runtime::time;

    let (_guard, mut writer, mut provider) = start();
    time::reset();
    assert_eq!(time::current(), TimeSettingsResult::Busy, "not opened yet");
    for _ in 0..7 {
        step(&mut writer, &mut provider); // open, net open, time open, keys, events, stats, its save
    }
    let TimeSettingsResult::Current { view, pending } = time::current() else {
        panic!("the store is published");
    };
    assert_eq!(
        (view.settings, view.generation, pending),
        (TimeSettings::default(), 0, false)
    );
    assert!(view.writable);

    let settings = TimeSettings {
        zone: Some(Zone::parse(b"MST7MDT,M3.2.0,M11.1.0").unwrap()),
        ntp: NtpServer::Address {
            ip: [10, 42, 103, 1],
            port: 1123,
        },
    };
    assert!(matches!(
        time::change(settings),
        TimeSettingsResult::Current { pending: true, .. }
    ));
    assert_eq!(time::change(settings), TimeSettingsResult::Busy);
    ATTEMPT_SEEN.with(|s| s.set(false));
    assert_eq!(
        step(&mut writer, &mut provider),
        Some(Record::TimeSaved { generation: 1 })
    );
    assert!(ATTEMPT_SEEN.with(Cell::get));
    assert_eq!(sdk::backup_read(MARKER_REGISTER), 0);
    let TimeSettingsResult::Current { view, pending } = time::current() else {
        panic!("the store is published");
    };
    assert_eq!(
        (view.settings, view.generation, pending),
        (settings, 1, false)
    );

    // The next boot reads them back.
    let mut writer = Writer::default();
    for _ in 0..2 {
        step(&mut writer, &mut provider);
    }
    assert_eq!(
        step(&mut writer, &mut provider),
        Some(Record::TimeOpen {
            status: TimeStatus::Loaded,
            generation: 1
        })
    );
    let mut text = String::new();
    writer::describe(Record::TimeSaved { generation: 1 }, &mut text).unwrap();
    assert_eq!(text, "time saved generation=1");
}
