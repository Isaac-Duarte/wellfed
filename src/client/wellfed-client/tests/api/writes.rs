//! Store writes over the API: settings, windows, the desiccant count, time
//! settings and the Wi-Fi clear, with stale writes refused, retries
//! answered once and each write's event in the history.

use wellfed_device_api::Record;
use wellfed_model as model;
use wellfed_model::{Answer, FeederSettingsPatch, Generation, Op};
use wellfed_proto::api::{self as proto, EventBody};

use crate::support::{
    Client_, MEMBER, OWNER, Rig, ask, fingerprint, open, poll_answer, send_unwritten,
};

#[test]
fn a_stale_settings_write_is_refused_and_a_current_one_applies() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    let changes = FeederSettingsPatch {
        volume: Some(60),
        ..FeederSettingsPatch::default()
    };

    let done = ask(
        &mut rig,
        &mut owner,
        1,
        Op::PutSettings {
            base: Generation(7),
            changes,
        },
    );
    assert_eq!(
        done,
        Answer::Done {
            generation: Some(Generation(8))
        }
    );
    assert_eq!(rig.device.settings.volume, 60);

    let stale = ask(
        &mut rig,
        &mut owner,
        2,
        Op::PutSettings {
            base: Generation(7),
            changes,
        },
    );
    assert_eq!(
        stale,
        Answer::Refused(model::Refusal::Stale {
            current: Generation(8)
        })
    );
    assert!(
        rig.device
            .events
            .iter()
            .any(|e| matches!(e.body, EventBody::Settings { generation: 8, .. }))
    );
}

/// The history's settings and Wi-Fi events: each generation and who asked.
fn store_events(rig: &Rig) -> Vec<EventBody> {
    rig.device
        .events
        .iter()
        .map(|e| e.body)
        .filter(|body| matches!(body, EventBody::Settings { .. } | EventBody::Wifi { .. }))
        .collect()
}

/// The owner's settings change or Wi-Fi clear, sent and left for the
/// writer; the event it records.
fn send_store_write(rig: &mut Rig, owner: &mut Client_, wifi: bool) -> EventBody {
    let by = proto::Source::Client(fingerprint(OWNER));
    if wifi {
        let base = rig.device.wifi_generation;
        let op = Op::PutWifi {
            base: Generation(base),
            wifi: None,
        };
        send_unwritten(rig, owner, 1, op);
        assert!(rig.device.wifi_pending.is_some(), "queued");
        let generation = base + 1;
        return EventBody::Wifi { generation, by };
    }
    let changes = FeederSettingsPatch {
        volume: Some(60),
        ..FeederSettingsPatch::default()
    };
    let op = Op::PutSettings {
        base: Generation(7),
        changes,
    };
    send_unwritten(rig, owner, 1, op);
    assert!(rig.device.settings_pending.is_some(), "queued");
    EventBody::Settings { generation: 8, by }
}

/// A session that goes once its settings or Wi-Fi change is queued: the
/// writer saves it and records its event, once and naming who asked, as it
/// does for a session that stays to read the answer, which adds nothing of
/// its own.
#[test]
fn store_writes_whose_session_left_are_still_in_the_history() {
    for wifi in [false, true] {
        let mut rig = Rig::new();
        let mut owner = open(&mut rig, OWNER);
        let expected = send_store_write(&mut rig, &mut owner, wifi);
        let records = &mut rig.records;
        rig.server.closed(owner.slot, |r| records.push(r));
        rig.pass();
        assert_eq!(store_events(&rig), [], "not before the writer has saved");
        rig.device.write();
        rig.pass();
        rig.pass();
        assert_eq!(store_events(&rig), [expected]);

        let mut rig = Rig::new();
        let mut owner = open(&mut rig, OWNER);
        send_store_write(&mut rig, &mut owner, wifi);
        rig.device.write();
        let answer = poll_answer(&mut rig, &mut owner, 1, 5);
        assert!(matches!(answer, Some(Answer::Done { .. })), "{answer:?}");
        rig.pass();
        assert_eq!(store_events(&rig), [expected]);
    }
}

/// A window crosses the wire both ways, and a bound past the last minute
/// of the day is refused before it reaches the writer.
#[test]
fn a_window_is_written_and_read_back_and_a_bad_one_refused() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    let night = model::Window::Between {
        start: 22 * 60,
        end: 6 * 60,
    };

    let done = ask(
        &mut rig,
        &mut owner,
        1,
        Op::PutSettings {
            base: Generation(7),
            changes: FeederSettingsPatch {
                sound_window: Some(night),
                ..FeederSettingsPatch::default()
            },
        },
    );
    assert_eq!(
        done,
        Answer::Done {
            generation: Some(Generation(8))
        }
    );
    let Answer::Settings(settings) = ask(&mut rig, &mut owner, 2, Op::GetSettings) else {
        panic!("no settings");
    };
    assert_eq!(settings.feeder().unwrap().sound_window, night);
    assert_eq!(
        settings.feeder().unwrap().lights_window,
        model::Window::Always
    );

    let refused = ask(
        &mut rig,
        &mut owner,
        3,
        Op::PutSettings {
            base: Generation(8),
            changes: FeederSettingsPatch {
                lights_window: Some(model::Window::Between {
                    start: 0,
                    end: 1440,
                }),
                ..FeederSettingsPatch::default()
            },
        },
    );
    assert_eq!(refused, Answer::Refused(model::Refusal::Invalid));
    assert_eq!(rig.device.settings.generation, 8, "nothing written");
}

/// The reset is dated from the feeder's clock alone: refused by name
/// without one, and once the clock is valid recorded under today, which
/// the status then counts from. A member may reset it, as it may change
/// the plans.
#[test]
fn a_desiccant_reset_needs_the_clock_and_starts_the_count() {
    let mut rig = Rig::new();
    let mut member = open(&mut rig, MEMBER);
    let Answer::Status(status) = ask(&mut rig, &mut member, 1, Op::Status) else {
        panic!("no status");
    };
    assert_eq!(status.feeder().unwrap().desiccant, model::Desiccant::Unset);

    let refused = ask(&mut rig, &mut member, 2, Op::ResetDesiccant);
    assert_eq!(refused, Answer::Refused(model::Refusal::NoClock));
    assert_eq!(rig.device.settings.generation, 7, "nothing written");

    // 2025-10-28.
    rig.device.today = Some(20_389);
    let done = ask(&mut rig, &mut member, 3, Op::ResetDesiccant);
    assert_eq!(
        done,
        Answer::Done {
            generation: Some(Generation(8))
        }
    );
    let Answer::Settings(settings) = ask(&mut rig, &mut member, 4, Op::GetSettings) else {
        panic!("no settings");
    };
    assert_eq!(
        settings.feeder().unwrap().desiccant_replaced,
        Some(jiff::civil::date(2025, 10, 28))
    );
    assert_eq!(settings.feeder().unwrap().desiccant_days, 14);

    // Fifteen days on, one past due; then the clock is lost.
    rig.device.today = Some(20_389 + 15);
    let Answer::Status(status) = ask(&mut rig, &mut member, 5, Op::Status) else {
        panic!("no status");
    };
    assert_eq!(
        status.feeder().unwrap().desiccant,
        model::Desiccant::Remaining {
            days: -1,
            due: true
        }
    );
    rig.device.today = None;
    let Answer::Status(status) = ask(&mut rig, &mut member, 6, Op::Status) else {
        panic!("no status");
    };
    assert_eq!(
        status.feeder().unwrap().desiccant,
        model::Desiccant::Unknown
    );
}

/// The period is a setting like any other, checked against its limits.
#[test]
fn the_desiccant_period_is_a_setting_within_its_limits() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    for (id, days) in [(1, 0), (2, 91)] {
        let refused = ask(
            &mut rig,
            &mut owner,
            id,
            Op::PutSettings {
                base: Generation(7),
                changes: FeederSettingsPatch {
                    desiccant_days: Some(days),
                    ..FeederSettingsPatch::default()
                },
            },
        );
        assert_eq!(refused, Answer::Refused(model::Refusal::Invalid), "{days}");
    }

    let done = ask(
        &mut rig,
        &mut owner,
        3,
        Op::PutSettings {
            base: Generation(7),
            changes: FeederSettingsPatch {
                desiccant_days: Some(90),
                ..FeederSettingsPatch::default()
            },
        },
    );
    assert_eq!(
        done,
        Answer::Done {
            generation: Some(Generation(8))
        }
    );
    let Answer::Settings(settings) = ask(&mut rig, &mut owner, 4, Op::GetSettings) else {
        panic!("no settings");
    };
    assert_eq!(settings.feeder().unwrap().desiccant_days, 90);
}

#[test]
fn a_retried_write_gets_its_first_answer_even_after_a_reconnect() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    let changes = FeederSettingsPatch {
        lights: Some(false),
        ..FeederSettingsPatch::default()
    };
    let first = ask(
        &mut rig,
        &mut owner,
        9,
        Op::PutSettings {
            base: Generation(7),
            changes,
        },
    );
    assert_eq!(
        first,
        Answer::Done {
            generation: Some(Generation(8))
        }
    );

    // The connection drops before the client sees the answer; it
    // reconnects and retries with the same id.
    let records = &mut rig.records;
    rig.server.closed(owner.slot, |r| records.push(r));
    let mut owner = open(&mut rig, OWNER);
    let retry = ask(
        &mut rig,
        &mut owner,
        9,
        Op::PutSettings {
            base: Generation(7),
            changes,
        },
    );
    assert_eq!(retry, first);
    assert_eq!(rig.device.settings.generation, 8, "written once");
    assert!(
        rig.records
            .iter()
            .any(|r| matches!(r, Record::Retried { .. }))
    );
}

/// A time settings write answers `Done` with the new generation once the
/// store holds it, `Done` too when it changed nothing, and is refused when
/// the writer could not make it, rather than answered with the old
/// generation as if it had.
#[test]
fn a_time_settings_write_answers_what_the_writer_did() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    let put = |base, port| Op::PutTime {
        base: Generation(base),
        changes: model::TimeChanges {
            zone: Some("MST7MDT,M3.2.0,M11.1.0".to_owned()),
            ntp: Some(model::NtpServer::Gateway { port }),
        },
    };

    let done = ask(&mut rig, &mut owner, 1, put(0, 1123));
    assert_eq!(
        done,
        Answer::Done {
            generation: Some(Generation(1))
        }
    );
    let unchanged = ask(&mut rig, &mut owner, 2, put(1, 1123));
    assert_eq!(
        unchanged,
        Answer::Done {
            generation: Some(Generation(1))
        }
    );

    rig.device.time_fails = true;
    let failed = ask(&mut rig, &mut owner, 3, put(1, 123));
    assert_eq!(failed, Answer::Refused(model::Refusal::NotWritable));
    let Answer::Time(view) = ask(&mut rig, &mut owner, 4, Op::GetTime) else {
        panic!("the time view");
    };
    assert_eq!(view.generation, Generation(1));
    assert_eq!(view.ntp, model::NtpServer::Gateway { port: 1123 });
    assert_eq!(view.clock_ms, None, "the feeder needs the time");
}
