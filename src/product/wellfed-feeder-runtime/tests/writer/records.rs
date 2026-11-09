//! The writer's `PLCFG1` lines: the error words, and which store writes
//! decode as such for the bench.

use wellfed_core::record::{self, Event, Record as Line, Tag};
use wellfed_device::store::net::NetStatus;
use wellfed_device::store::settings::{Settings, SettingsStatus};
use wellfed_device_runtime::writer::{
    self as writer, ErrorStage, KeyChange, KeyResult, SettingsPart, WriteEvent,
};
use wellfed_feeder_runtime::writer::Record;

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
