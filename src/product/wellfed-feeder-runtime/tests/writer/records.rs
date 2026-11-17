//! The writer's `PLCFG1` lines: the error words, which store writes decode
//! as such for the bench, and that every record fits its line's buffer.

use std::collections::BTreeSet;

use wellfed_core::flash_coord::FlashReply;
use wellfed_core::log_tail::LogTail;
use wellfed_core::record::{self, Event, Record as Line, Tag};
use wellfed_core::settings::Window;
use wellfed_core::store::seed::SeedStatus;
use wellfed_device::store::clients::ClientStatus;
use wellfed_device::store::net::NetStatus;
use wellfed_device::store::settings::{Settings, SettingsStatus};
use wellfed_device::store::time::TimeStatus;
use wellfed_device_api::mailbox::{CallClip, FeederSettings, MealReminder};
use wellfed_device_runtime::writer::{
    self as writer, ErrorStage, Identity, KeyChange, KeyResult, SettingsPart, WriteEvent,
};
use wellfed_feeder::store::stats::StatsStatus;
use wellfed_feeder_runtime::writer::{FeederRecord, Record};
use wellfed_hal::flash_lock::arming;
use wellfed_hal::startup::{self, Report};

use crate::support::changed;

/// The `PLCFG1` error words, as they were when the stage was a string.
#[test]
fn error_records_format_as_before() {
    let words: Vec<_> = ErrorStage::ALL.iter().map(|s| s.as_str()).collect();
    assert_eq!(words, ["unarmed", "save", "erase", "repair", "apart"]);

    let mut text = String::new();
    writer::describe(
        Record::Error {
            stage: ErrorStage::Save,
            store: SettingsPart::Kind,
            status: SettingsStatus::Io,
        },
        &mut text,
    )
    .unwrap();
    assert_eq!(
        text,
        format!(
            "error stage=save store=kind status={}",
            SettingsStatus::Io.code()
        )
    );

    text.clear();
    writer::describe(
        Record::NetError {
            stage: ErrorStage::Unarmed,
        },
        &mut text,
    )
    .unwrap();
    assert_eq!(text, "net error stage=unarmed");
}

/// A writer record, as its `PLCFG1` line, decodes to `expected` through the
/// host's parser.
fn decodes(record: Record, expected: Option<Event<'_>>) {
    let mut text = String::new();
    writer::describe(record, &mut text).unwrap();
    let line = format!("{} {text}", Tag::Settings);
    let event = Line::parse(&line).and_then(|line| line.event());
    assert_eq!(event, expected, "{line}");
}

/// The writes the bench waits on decode as such; the rest decode as
/// nothing.
#[test]
fn store_writes_decode_for_the_bench() {
    decodes(
        Record::Saved {
            generation: 7,
            settings: Settings::default(),
            kind: None,
            event: WriteEvent::Appended,
        },
        Some(Event::Settings(record::Settings::Saved {
            generation: Some(7),
        })),
    );
    decodes(
        Record::NetSaved {
            generation: 3,
            stored: false,
            event: WriteEvent::None,
        },
        Some(Event::Settings(record::Settings::NetSaved {
            stored: false,
        })),
    );
    decodes(
        changed(KeyChange::Enroll, KeyResult::Done, 1),
        Some(Event::Settings(record::Settings::KeysChanged)),
    );
    decodes(
        Record::NetOpen {
            status: NetStatus::Loaded,
            generation: 3,
            stored: true,
        },
        None,
    );
}

/// Every value each word field of a record can take, so the longest is
/// among them.
const SETTINGS_STATUSES: [SettingsStatus; 6] = [
    SettingsStatus::Loaded,
    SettingsStatus::Empty,
    SettingsStatus::Recovered,
    SettingsStatus::Corrupt,
    SettingsStatus::Future,
    SettingsStatus::Io,
];
const NET_STATUSES: [NetStatus; 5] = [
    NetStatus::Unopened,
    NetStatus::Empty,
    NetStatus::Loaded,
    NetStatus::ReadOnly,
    NetStatus::Io,
];
const TIME_STATUSES: [TimeStatus; 5] = [
    TimeStatus::Unopened,
    TimeStatus::Empty,
    TimeStatus::Loaded,
    TimeStatus::ReadOnly,
    TimeStatus::Io,
];
const SEED_STATUSES: [SeedStatus; 5] = [
    SeedStatus::Unopened,
    SeedStatus::Empty,
    SeedStatus::Loaded,
    SeedStatus::ReadOnly,
    SeedStatus::Io,
];
const CLIENT_STATUSES: [ClientStatus; 6] = [
    ClientStatus::Unopened,
    ClientStatus::Empty,
    ClientStatus::Loaded,
    ClientStatus::ReadOnly,
    ClientStatus::Corrupt,
    ClientStatus::Io,
];
const STATS_STATUSES: [StatsStatus; 6] = [
    StatsStatus::Unopened,
    StatsStatus::Empty,
    StatsStatus::Loaded,
    StatsStatus::ReadOnly,
    StatsStatus::Lost,
    StatsStatus::Io,
];
const FLASH_REPLIES: [FlashReply; 6] = [
    FlashReply::Accepted,
    FlashReply::Duplicate,
    FlashReply::Stale,
    FlashReply::Busy,
    FlashReply::Invalid,
    FlashReply::Failed,
];

/// A window as long as its type can spell it: `1092:15-1092:15`, past any
/// valid minute of the day.
const LONGEST_WINDOW: Window = Window::Between {
    start: u16::MAX,
    end: u16::MAX,
};

/// The device's settings with every field at its longest.
fn longest_settings() -> Settings {
    Settings {
        lights: true,
        sound: true,
        volume: u8::MAX,
        lights_window: LONGEST_WINDOW,
        sound_window: LONGEST_WINDOW,
    }
}

/// The feeder's settings with every field at its longest.
fn longest_feeder() -> FeederRecord {
    FeederRecord::Settings {
        status: SettingsStatus::Future,
        generation: u64::MAX,
        settings: FeederSettings {
            meal_reminder: MealReminder::Before { minutes: u8::MAX },
            desiccant_days: u8::MAX,
            desiccant_reset: Some(i32::MIN),
            ..FeederSettings::DEFAULT
        },
    }
}

/// Which record kind `record` is. Exhaustive, so a new kind fails to
/// compile here until [`longest_records`] covers it.
fn kind(record: &Record) -> &'static str {
    match record {
        Record::Open { .. } => "open",
        Record::Arming(_) => "arming",
        Record::Saved { .. } => "saved",
        Record::Repaired { .. } => "repaired",
        Record::Error { .. } => "error",
        Record::NetOpen { .. } => "net open",
        Record::NetSaved { .. } => "net saved",
        Record::NetError { .. } => "net error",
        Record::TimeOpen { .. } => "time open",
        Record::TimeSaved { .. } => "time saved",
        Record::TimeError { .. } => "time error",
        Record::Keys { .. } => "keys",
        Record::KeysChanged { .. } => "keys change",
        Record::Events { .. } => "events",
        Record::EventError => "events error",
        Record::ResetPending => "reset pending",
        Record::ResetWearLost => "reset wear_lost",
        Record::ResetUpdateCleared => "reset update",
        Record::HandoverDone { .. } => "handover done",
        Record::HandoverStopped { .. } => "handover stopped",
        Record::ResetDone { .. } => "reset done",
        Record::ResetError { .. } => "reset error",
        Record::Product(product) => match product {
            FeederRecord::Settings { .. } => "feeder settings",
            FeederRecord::StatsOpen { .. } => "stats open",
            FeederRecord::StatsSaved { .. } => "stats saved",
            FeederRecord::StatsError { .. } => "stats error",
            FeederRecord::CalibrationSaved { .. } => "calibration saved",
            FeederRecord::CalibrationError { .. } => "calibration error",
            FeederRecord::PlansSaved { .. } => "plans saved",
            FeederRecord::PlansDropped => "plans dropped",
            FeederRecord::PlansError { .. } => "plans error",
            FeederRecord::Call { .. } => "call",
            FeederRecord::CallError { .. } => "call error",
            FeederRecord::Trace { .. } => "trace",
        },
    }
}

/// Every record kind the writer logs, each with every word field through
/// every value it takes and every number at its type's longest.
fn longest_records() -> Vec<Record> {
    let mut records = Vec::new();
    let feeder = longest_feeder();

    for kind in [None, Some(feeder)] {
        for status in SETTINGS_STATUSES {
            records.push(Record::Open {
                status,
                generation: u64::MAX,
                settings: longest_settings(),
                kind,
            });
        }
        for &event in WriteEvent::ALL {
            records.push(Record::Saved {
                generation: u64::MAX,
                settings: longest_settings(),
                kind,
                event,
            });
        }
    }

    for reply in FLASH_REPLIES {
        records.push(Record::Arming(arming::Event::Ready {
            reply,
            inactive: true,
        }));
    }
    records.extend([
        Record::Arming(arming::Event::Armed {
            watchdog: true,
            installed: true,
        }),
        Record::Arming(arming::Event::Skip { marker: u32::MAX }),
        Record::Arming(arming::Event::Timeout),
        Record::Arming(arming::Event::Clock),
        Record::Repaired {
            generation: u64::MAX,
        },
    ]);

    for &stage in ErrorStage::ALL {
        for &store in SettingsPart::ALL {
            for status in SETTINGS_STATUSES {
                records.push(Record::Error {
                    stage,
                    store,
                    status,
                });
            }
        }
        records.extend([
            Record::NetError { stage },
            Record::TimeError { stage },
            Record::ResetError {
                stage,
                sector: Some(u32::MAX),
                restart: true,
            },
            Record::Product(FeederRecord::StatsError { stage }),
            Record::Product(FeederRecord::CalibrationError { stage }),
            Record::Product(FeederRecord::PlansError { stage }),
            Record::Product(FeederRecord::CallError { stage }),
        ]);
    }

    for status in NET_STATUSES {
        records.push(Record::NetOpen {
            status,
            generation: u64::MAX,
            stored: true,
        });
    }
    for &event in WriteEvent::ALL {
        records.push(Record::NetSaved {
            generation: u64::MAX,
            stored: true,
            event,
        });
    }
    for status in TIME_STATUSES {
        records.push(Record::TimeOpen {
            status,
            generation: u64::MAX,
        });
    }
    records.push(Record::TimeSaved {
        generation: u64::MAX,
    });

    for seed in SEED_STATUSES {
        for clients in CLIENT_STATUSES {
            for &identity in Identity::ALL {
                records.push(Record::Keys {
                    seed,
                    mixed: true,
                    saved: true,
                    clients,
                    enrolled: usize::MAX,
                    identity,
                });
            }
        }
    }
    for &change in KeyChange::ALL {
        for &result in KeyResult::ALL {
            for &event in WriteEvent::ALL {
                records.push(Record::KeysChanged {
                    change,
                    result,
                    enrolled: usize::MAX,
                    event,
                });
            }
        }
    }

    records.extend([
        Record::Events {
            open: true,
            last: Some(u64::MAX),
            oldest: Some(u64::MAX),
            epoch: u32::MAX,
        },
        Record::EventError,
        Record::ResetPending,
        Record::ResetWearLost,
        Record::ResetUpdateCleared,
        Record::HandoverDone { cleared: true },
        Record::HandoverStopped { stage: u8::MAX },
        Record::ResetDone { erased: u32::MAX },
        Record::Product(feeder),
    ]);

    for status in STATS_STATUSES {
        records.push(Record::Product(FeederRecord::StatsOpen {
            status,
            generation: u64::MAX,
            early: u8::MAX,
            missed: true,
        }));
    }
    for &event in WriteEvent::ALL {
        records.push(Record::Product(FeederRecord::PlansSaved {
            sequence: u32::MAX,
            revision: u32::MAX,
            event,
        }));
    }
    records.extend([
        Record::Product(FeederRecord::StatsSaved {
            generation: u64::MAX,
            early: u8::MAX,
        }),
        Record::Product(FeederRecord::CalibrationSaved {
            generation: u64::MAX,
            portion_mg: Some(u32::MAX),
        }),
        Record::Product(FeederRecord::PlansDropped),
        Record::Product(FeederRecord::Call {
            clip: CallClip::None,
        }),
        Record::Product(FeederRecord::Call {
            clip: CallClip::Receiving {
                bytes: u32::MAX,
                crc: u32::MAX,
                received: u32::MAX,
            },
        }),
        Record::Product(FeederRecord::Call {
            clip: CallClip::Stored {
                bytes: u32::MAX,
                crc: u32::MAX,
            },
        }),
        Record::Product(FeederRecord::Trace {
            feed: u32::MAX,
            seq: Some(u32::MAX),
        }),
        Record::Product(FeederRecord::Trace {
            feed: u32::MAX,
            seq: None,
        }),
    ]);
    records
}

/// Every record the writer logs fits its line's buffer at its longest, so
/// none is cut, and reads back through the host's parser as the line it
/// is. The feeder's `saved` is 268 bytes at its longest (and 203 as the
/// bench first saw it) against the 192 of the buffer that dropped it.
#[test]
fn every_record_at_its_longest_fits_its_line_and_parses() {
    let records = longest_records();
    let mut kinds = BTreeSet::new();
    let mut longest = (0, String::new());

    for record in records {
        kinds.insert(kind(&record));
        let mut report = Report::<{ writer::LINE }>::sized();
        let line = writer::line(&mut report, record)
            .to_str()
            .unwrap()
            .to_owned();
        assert!(!report.truncated(), "cut: {record:?}");
        assert!(line.ends_with("\r\n"), "{line}");

        let mut body = String::new();
        writer::describe(record, &mut body).unwrap();
        assert_eq!(line, format!("PLCFG1 {body}\r\n"));

        let parsed = Line::parse(&line).unwrap();
        assert_eq!(parsed.kind(), Some(Tag::Settings), "{line}");
        assert_eq!(parsed.body().trim_end(), body.trim_end(), "{line}");
        if line.len() > longest.0 {
            longest = (line.len(), line);
        }
    }

    // Each kind is covered: the exhaustive `kind` lists 34.
    assert_eq!(kinds.len(), 34, "{kinds:?}");
    assert_eq!(longest.0, 268, "{}", longest.1);
    assert!(longest.0 < writer::LINE);
}

/// The feeder's settings lines at their longest name every value, as the
/// bench and the integration suite read them.
#[test]
fn the_longest_saved_and_open_lines_read_back_whole() {
    let max = u64::MAX.to_string();
    let saved = Record::Saved {
        generation: u64::MAX,
        settings: longest_settings(),
        kind: Some(longest_feeder()),
        event: WriteEvent::Appended,
    };
    let open = Record::Open {
        status: SettingsStatus::Future,
        generation: u64::MAX,
        settings: longest_settings(),
        kind: Some(longest_feeder()),
    };

    for record in [saved, open] {
        let mut report = Report::<{ writer::LINE }>::sized();
        let text = writer::line(&mut report, record)
            .to_str()
            .unwrap()
            .to_owned();
        let line = Line::parse(&text).unwrap();

        assert_eq!(line.field("generation"), Some(max.as_str()), "{text}");
        assert_eq!(line.field("volume"), Some("255"));
        assert_eq!(line.field("lights_window"), Some("1092:15-1092:15"));
        assert_eq!(line.field("sound_window"), Some("1092:15-1092:15"));
        assert_eq!(line.field("feeder_generation"), Some(max.as_str()));
        assert_eq!(line.field("meal_reminder"), Some("255"));
        assert_eq!(line.field("desiccant_days"), Some("255"));
        assert_eq!(line.field("desiccant_reset"), Some("-2147483648"));
    }

    let mut report = Report::<{ writer::LINE }>::sized();
    let text = writer::line(&mut report, saved)
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(text.len(), 268, "{text}");

    // `wellfed logs` reads it whole too, from the tail the SDK's `log`
    // copies every line into.
    let tail = LogTail::<16_384>::new();
    tail.push(text.as_bytes());
    let mut out = [0; 1024];
    let batch = tail.read(None, &mut out).unwrap();
    assert_eq!(batch.dropped, 0);
    assert_eq!(
        &out[..batch.len],
        format!("{}\n", text.trim_end()).as_bytes()
    );
    assert_eq!(
        Line::parse(&text).and_then(|line| line.event()),
        Some(Event::Settings(record::Settings::Saved {
            generation: Some(u64::MAX),
        }))
    );
}

/// The feeder's default settings saved is 203 bytes, past the 192 of a
/// task's usual record buffer, so the writer's own buffer must carry it.
#[test]
fn the_default_saved_line_outgrew_the_usual_buffer() {
    let record = Record::Saved {
        generation: 4,
        settings: Settings::default(),
        kind: Some(FeederRecord::Settings {
            status: SettingsStatus::Loaded,
            generation: 4,
            settings: FeederSettings::DEFAULT,
        }),
        event: WriteEvent::Appended,
    };

    let mut usual = Report::new();
    let mut body = String::new();
    writer::describe(record, &mut body).unwrap();
    let cut = usual.record(format_args!("PLCFG1 {body}\r\n"));
    assert_eq!(cut.to_str().unwrap(), "PLCFG1 saved truncated=1\r\n");
    assert_eq!(
        Line::parse(cut.to_str().unwrap()).and_then(|line| line.event()),
        Some(Event::Settings(record::Settings::Saved {
            generation: None
        }))
    );

    let mut report = Report::<{ writer::LINE }>::sized();
    let line = writer::line(&mut report, record)
        .to_str()
        .unwrap()
        .to_owned();
    assert!(!report.truncated());
    assert_eq!(
        line,
        "PLCFG1 saved generation=4 lights=1 sound=1 volume=50 lights_window=always \
         sound_window=always feeder_status=0 feeder_generation=4 meal_reminder=off \
         desiccant_days=14 desiccant_reset=none event=appended\r\n"
    );
    assert!(line.len() > startup::LINE);
}
