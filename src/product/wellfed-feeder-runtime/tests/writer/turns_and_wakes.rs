//! When the writer steps: store writes held by a feed until its release,
//! mailbox wakes that bring a step forward, resting, and an upload's pace.

use wellfed_device::store::events::Read;
use wellfed_device_api::mailbox::{SettingsFields, SettingsResult};
use wellfed_device_runtime::keys::{self, Requested};
use wellfed_device_runtime::writer::{self as writer, Identity, KeyChange, KeyResult};
use wellfed_device_runtime::{events, network};
use wellfed_feeder_runtime::writer::{RECEIVE_LINGER_POLLS, Record, Step, Writer};
use wellfed_hal::flash_lock::provider::Provider;
use wellfed_hal::interlock::INTERLOCK;
use wellfed_hal::sdk::fake;
use wellfed_hal::{rng, settings};

use crate::support::{A, Armed, NoMmio, changed, open_all, phone, start, step};

/// Every store the writer keeps defers its write while a feed holds the
/// outputs, and completes it after the release, in order, with nothing lost.
#[test]
fn every_store_write_waits_for_a_feed_and_completes_after_it() {
    use wellfed_device_api::mailbox::WifiResult;
    use wellfed_device_api::wifi::Credentials;

    let (_guard, mut writer, mut provider) = start();
    fake::with(|h| h.adc_noise = 0x5a5a_1234);
    assert_eq!(rng::start().result, Ok(()));
    step(&mut writer, &mut provider); // open
    step(&mut writer, &mut provider); // net open
    step(&mut writer, &mut provider); // time open
    let storage = || fake::with(|h| h.storage.clone());

    // A feed holds the outputs before the first boot's seed and identity.
    INTERLOCK.claim_feed(u32::MAX).unwrap();
    let before = storage();
    for _ in 0..5 {
        assert_eq!(step(&mut writer, &mut provider), None);
    }
    assert_eq!(storage(), before, "no write while the outputs are claimed");
    assert_eq!(keys::identity_since(0), None);
    INTERLOCK.release_feed();
    assert!(matches!(
        step(&mut writer, &mut provider),
        Some(Record::Keys {
            saved: true,
            identity: Identity::Created,
            ..
        })
    ));
    step(&mut writer, &mut provider); // events
    step(&mut writer, &mut provider); // stats
    step(&mut writer, &mut provider); // the boot's stats save

    // A feed, and during it every kind of write queued.
    INTERLOCK.claim_feed(u32::MAX).unwrap();
    for body in [[1; 16], [2; 16], [3; 16]] {
        assert!(events::record(None, body));
    }
    assert!(matches!(
        settings::request(SettingsFields {
            volume: Some(7),
            ..SettingsFields::default()
        }),
        SettingsResult::Current { pending: true, .. }
    ));
    let set = Credentials::new(b"net", b"password").unwrap();
    assert!(matches!(
        network::change(Some(set), None),
        WifiResult::Current { pending: true, .. }
    ));
    let phone = phone();
    assert_eq!(keys::request(phone), Requested::Queued);

    let before = storage();
    for _ in 0..20 {
        assert_eq!(step(&mut writer, &mut provider), None);
    }
    assert_eq!(storage(), before, "no write while the outputs are claimed");
    assert_eq!(events::last_seq(), None);
    // Reads take no lease: answered during the feed.
    assert_eq!(events::read_result(A, None), None, "asked");
    step(&mut writer, &mut provider);
    assert_eq!(events::read_result(A, None), Some(Read::End));

    // At rest: each write in turn, the first right after the release.
    INTERLOCK.release_feed();
    assert_eq!(
        step(&mut writer, &mut provider),
        Some(changed(KeyChange::Enroll, KeyResult::Done, 1))
    );
    assert!(matches!(
        step(&mut writer, &mut provider),
        Some(Record::NetSaved { stored: true, .. })
    ));
    assert!(matches!(
        step(&mut writer, &mut provider),
        Some(Record::Saved { settings, .. }) if settings.volume == 7
    ));
    for _ in 0..3 {
        step(&mut writer, &mut provider);
    }
    // The enrollment's own event, then the three queued.
    assert_eq!(events::last_seq(), Some(4));
    assert_eq!(events::pending(), None);
    assert!(!INTERLOCK.writing(), "every turn given back");
    assert_eq!(INTERLOCK.status().writes_deferred, 2);

    // The enrollment's event went in its own turn, first; the loop's
    // appends kept their order behind it.
    let mut after = Some(1);
    for body in [[1; 16], [2; 16], [3; 16]] {
        events::read_result(A, after);
        step(&mut writer, &mut provider);
        let Some(Read::Record(record)) = events::read_result(A, after) else {
            panic!("event after {after:?}");
        };
        assert_eq!(record.body, body);
        after = Some(record.seq);
    }
}

/// One step, whole.
fn whole_step(writer: &mut Writer<Armed>, provider: &mut Provider) -> Step {
    // SAFETY: the provider outlives the run.
    unsafe { writer.step::<NoMmio, Armed>(provider) }
}

fn notifies() -> u32 {
    fake::with(|h| h.task_notifies)
}

#[test]
fn every_mailbox_wakes_the_writer_once_when_queued_and_not_when_refused() {
    use wellfed_device_api::mailbox::{
        Calibration, CalibrationResult, CallChange, CallResult, NtpServer, PLANS, PlansChange,
        PlansResult, TimeSettings, TimeSettingsResult, WifiResult,
    };
    use wellfed_device_api::wifi::Credentials;
    use wellfed_device_runtime::time;
    use wellfed_feeder_runtime::{calibration, call, plans};
    use wellfed_hal::writer_wake;

    let (_guard, mut writer, mut provider) = start();
    calibration::reset();
    call::reset();
    plans::reset();
    time::reset();
    open_all(&mut writer, &mut provider);
    step(&mut writer, &mut provider); // the stats store's early save
    writer_wake::bind();

    let queue: [(&str, &mut dyn FnMut() -> bool); 7] = [
        ("settings", &mut || {
            matches!(
                settings::request(SettingsFields {
                    volume: Some(10),
                    ..SettingsFields::default()
                }),
                SettingsResult::Current { pending: true, .. }
            )
        }),
        ("wifi", &mut || {
            let set = Credentials::new(b"home", b"passphrase").unwrap();
            matches!(
                network::change(Some(set), None),
                WifiResult::Current { pending: true, .. }
            )
        }),
        ("keys", &mut || {
            let phone = phone();
            keys::request(phone) == Requested::Queued
        }),
        ("time", &mut || {
            let change = TimeSettings {
                zone: None,
                ntp: NtpServer::Off,
            };
            matches!(
                time::change(change),
                TimeSettingsResult::Current { pending: true, .. }
            )
        }),
        ("calibration", &mut || {
            let change = Calibration {
                portion_mg: Some(9_800),
            };
            matches!(
                calibration::change(change),
                CalibrationResult::Current { pending: true, .. }
            )
        }),
        ("plans", &mut || {
            let change = PlansChange::Replace {
                table: [None; PLANS],
                at_s: Some(1_000),
            };
            matches!(
                plans::change(change, None),
                PlansResult::Current { pending: true, .. }
            )
        }),
        ("call", &mut || {
            let start = CallChange::Start { bytes: 600, crc: 1 };
            matches!(
                call::change(start),
                CallResult::Current { pending: true, .. }
            )
        }),
    ];

    for (name, queue) in queue {
        let before = notifies();
        assert!(queue(), "{name} queued");
        assert_eq!(notifies(), before + 1, "{name} woke the writer");

        // The slot taken: refused, with no wake.
        assert!(!queue(), "{name} refused");
        assert_eq!(notifies(), before + 1, "{name} refused without a wake");
        assert!(step(&mut writer, &mut provider).is_some(), "{name} saved");
    }
}

/// A notification only brings a step forward: with none at all, each mailbox
/// is still taken by the next poll, as the tests above all do; and a writer
/// that just wrote keeps its delay.
#[test]
fn only_a_writer_at_rest_is_stepped_early() {
    use wellfed_hal::writer_wake;

    let (_guard, mut writer, mut provider) = start();
    open_all(&mut writer, &mut provider);
    step(&mut writer, &mut provider); // the stats store's early save
    writer_wake::bind();
    let quieter = SettingsFields {
        volume: Some(10),
        ..SettingsFields::default()
    };

    // At rest: a wake takes.
    let idle = whole_step(&mut writer, &mut provider);
    assert!(idle.rested);
    assert_eq!(idle.delay_ms, writer::POLL_MS);
    writer_wake::settle(idle.rested);
    assert!(!writer_wake::take(), "nothing queued");
    settings::request(quieter);
    assert!(writer_wake::take(), "steps the writer now");

    // After the write: a second change waits out the writer's delay.
    let saved = whole_step(&mut writer, &mut provider);
    assert!(matches!(saved.record, Some(Record::Saved { .. })));
    assert!(!saved.rested);
    assert_eq!(saved.delay_ms, writer::POLL_MS);
    writer_wake::settle(saved.rested);
    settings::request(SettingsFields {
        volume: Some(20),
        ..SettingsFields::default()
    });
    assert!(!writer_wake::take(), "the delay stands");

    // Its poll takes the change regardless: the wake was dropped, not the work.
    assert!(matches!(
        step(&mut writer, &mut provider),
        Some(Record::Saved { .. })
    ));
}

#[test]
fn a_wake_during_a_feed_writes_nothing_until_the_owner_is_at_rest() {
    use wellfed_hal::writer_wake;

    let (_guard, mut writer, mut provider) = start();
    open_all(&mut writer, &mut provider);
    step(&mut writer, &mut provider); // the stats store's early save
    writer_wake::bind();
    let idle = whole_step(&mut writer, &mut provider);
    writer_wake::settle(idle.rested);

    INTERLOCK.claim_feed(u32::MAX).unwrap();
    let before = fake::with(|h| h.storage.clone());
    let writes = notifies();
    assert!(matches!(
        settings::request(SettingsFields {
            volume: Some(10),
            ..SettingsFields::default()
        }),
        SettingsResult::Current { pending: true, .. }
    ));
    assert_eq!(notifies(), writes + 1, "woken");
    assert!(writer_wake::take(), "the writer was at rest");

    // Stepped at once, and at every poll after: it waits for the turn, and no
    // later wake hurries it, since it no longer counts as resting.
    for _ in 0..5 {
        let waiting = whole_step(&mut writer, &mut provider);
        assert_eq!(waiting.record, None);
        assert_eq!(waiting.delay_ms, writer::WRITE_WAIT_POLL_MS);
        assert!(!waiting.rested);
        writer_wake::settle(waiting.rested);
    }
    assert_eq!(
        fake::with(|h| h.storage.clone()),
        before,
        "no write while the outputs are claimed"
    );
    assert!(settings::current().is_some_and(|view| view.writable));

    INTERLOCK.release_feed();
    assert!(matches!(
        step(&mut writer, &mut provider),
        Some(Record::Saved { .. })
    ));
}

/// A client alternating a start and a delete erases a sector each time, so
/// neither lets a wake cut its delay; the chunks between them still may.
#[test]
fn a_call_change_keeps_the_poll_and_only_a_chunk_is_followed_at_once() {
    use wellfed_device_api::mailbox::CallChange;
    use wellfed_feeder_runtime::call;
    use wellfed_hal::writer_wake;

    let (_guard, mut writer, mut provider) = start();
    call::reset();
    wellfed_hal::audio::reset();
    open_all(&mut writer, &mut provider);
    step(&mut writer, &mut provider); // the stats store's early save
    writer_wake::bind();

    let upload = CallChange::Start { bytes: 600, crc: 1 };
    for change in [upload, CallChange::Delete, upload, CallChange::Delete] {
        let idle = whole_step(&mut writer, &mut provider);
        writer_wake::settle(idle.rested);
        call::change(change);
        assert!(
            idle.rested && writer_wake::take(),
            "the first starts at once"
        );

        let erased = whole_step(&mut writer, &mut provider);
        assert_eq!(erased.delay_ms, writer::POLL_MS);
        assert!(!erased.rested);
        writer_wake::settle(erased.rested);

        // The next change, straight behind it, waits out the delay.
        let other = match change {
            CallChange::Delete => upload,
            _ => CallChange::Delete,
        };
        call::change(other);
        assert!(!writer_wake::take(), "the delay stands");
        assert_eq!(
            whole_step(&mut writer, &mut provider).delay_ms,
            writer::POLL_MS
        );
        call::change(change);
        whole_step(&mut writer, &mut provider);
    }

    // A chunk: followed at the upload's pace, and then a wake takes.
    call::change(upload);
    whole_step(&mut writer, &mut provider);
    assert_eq!(call::chunk(0, &[0; 512]), Ok(()));
    let chunk = whole_step(&mut writer, &mut provider);
    assert_eq!(chunk.delay_ms, writer::EVENTS_POLL_MS);
    let between = whole_step(&mut writer, &mut provider);
    assert_eq!(between.delay_ms, writer::EVENTS_POLL_MS);
    writer_wake::settle(between.rested);
    assert_eq!(call::chunk(512, &[0; 88]), Ok(()));
    assert!(writer_wake::take(), "the next chunk starts at once");
}

#[test]
fn only_a_step_that_did_nothing_rests() {
    let (_guard, mut writer, mut provider) = start();
    open_all(&mut writer, &mut provider);
    step(&mut writer, &mut provider); // the stats store's early save

    assert!(
        whole_step(&mut writer, &mut provider).rested,
        "nothing to do"
    );

    // An event appended.
    assert!(events::record(None, [1; 16]));
    let appended = whole_step(&mut writer, &mut provider);
    assert_eq!(events::last_seq(), Some(1));
    assert!(!appended.rested, "appended");
    assert!(whole_step(&mut writer, &mut provider).rested);

    // A record served; the linger after it does nothing, at the read's pace.
    assert_eq!(events::read_result(A, None), None, "asked");
    let served = whole_step(&mut writer, &mut provider);
    assert_eq!(served.delay_ms, writer::EVENTS_POLL_MS);
    assert!(!served.rested, "served");
    let linger = whole_step(&mut writer, &mut provider);
    assert_eq!(linger.delay_ms, writer::EVENTS_POLL_MS);
    assert!(linger.rested);
    assert!(matches!(
        events::read_result(A, None),
        Some(Read::Record(_))
    ));

    // An event deferred, by a feed's turn.
    INTERLOCK.claim_feed(u32::MAX).unwrap();
    assert!(events::record(None, [2; 16]));
    let deferred = whole_step(&mut writer, &mut provider);
    assert_eq!(events::last_seq(), Some(1), "deferred");
    assert!(!deferred.rested, "deferred");
    INTERLOCK.release_feed();
    assert!(!whole_step(&mut writer, &mut provider).rested, "appended");
    assert_eq!(events::last_seq(), Some(2));
}

/// The upload's pace must not become a claim's rival: an event held back by a
/// feed's turn stops the writer asking for that turn every 5 ms.
#[test]
fn an_upload_does_not_linger_while_a_feed_defers_an_event() {
    use wellfed_device_api::mailbox::CallChange;
    use wellfed_feeder_runtime::call;

    let (_guard, mut writer, mut provider) = start();
    call::reset();
    wellfed_hal::audio::reset();
    open_all(&mut writer, &mut provider);
    step(&mut writer, &mut provider); // the stats store's early save

    call::change(CallChange::Start { bytes: 600, crc: 1 });
    whole_step(&mut writer, &mut provider);
    assert_eq!(
        whole_step(&mut writer, &mut provider).delay_ms,
        writer::EVENTS_POLL_MS,
        "at the upload's pace"
    );

    INTERLOCK.claim_feed(u32::MAX).unwrap();
    assert!(events::record(None, [1; 16]));
    assert_eq!(
        whole_step(&mut writer, &mut provider).delay_ms,
        writer::POLL_MS
    );

    // Reads take no turn, so a reader keeps its pace.
    assert_eq!(events::read_result(A, None), None, "asked");
    assert_eq!(
        whole_step(&mut writer, &mut provider).delay_ms,
        writer::EVENTS_POLL_MS
    );
    INTERLOCK.release_feed();
}

#[test]
fn an_upload_polls_at_the_read_pace_while_it_runs_and_not_for_ever() {
    use wellfed_core::store::slot::crc32;
    use wellfed_device_api::mailbox::CallChange;
    use wellfed_feeder_runtime::call;

    let (_guard, mut writer, mut provider) = start();
    call::reset();
    wellfed_hal::audio::reset();
    open_all(&mut writer, &mut provider);
    step(&mut writer, &mut provider); // the stats store's early save
    assert_eq!(
        whole_step(&mut writer, &mut provider).delay_ms,
        writer::POLL_MS,
        "idle before it starts"
    );

    let stream: Vec<u8> = (0..1_100u32).map(|i| (i * 7) as u8).collect();
    let crc = crc32(&stream);
    call::change(CallChange::Start { bytes: 1_100, crc });
    let started = whole_step(&mut writer, &mut provider);
    assert_eq!(started.delay_ms, writer::POLL_MS, "it erased");

    // Between chunks, the upload's pace, each chunk starting the count again.
    for _ in 0..3 {
        let between = whole_step(&mut writer, &mut provider);
        assert_eq!(between.delay_ms, writer::EVENTS_POLL_MS);
        assert!(between.rested, "and a wake may still cut it short");
    }
    assert_eq!(call::chunk(0, &stream[..512]), Ok(()));
    assert_eq!(
        whole_step(&mut writer, &mut provider).delay_ms,
        writer::EVENTS_POLL_MS
    );

    // A client that walks away: the pace lapses, with the clip still receiving.
    for _ in 1..RECEIVE_LINGER_POLLS {
        assert_eq!(
            whole_step(&mut writer, &mut provider).delay_ms,
            writer::EVENTS_POLL_MS
        );
    }
    let lapsed = whole_step(&mut writer, &mut provider);
    assert_eq!(lapsed.delay_ms, writer::POLL_MS);

    // It comes back, and goes on to the end. The last chunk leaves the clip
    // stored, and the writer idle after the one step that follows it.
    assert_eq!(call::chunk(512, &stream[512..1_024]), Ok(()));
    assert_eq!(
        whole_step(&mut writer, &mut provider).delay_ms,
        writer::EVENTS_POLL_MS
    );
    assert_eq!(call::chunk(1_024, &stream[1_024..]), Ok(()));
    assert_eq!(
        whole_step(&mut writer, &mut provider).delay_ms,
        writer::EVENTS_POLL_MS
    );
    assert_eq!(
        whole_step(&mut writer, &mut provider).delay_ms,
        writer::POLL_MS,
        "the clip is stored"
    );
}
