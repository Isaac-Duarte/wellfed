//! The UART records the bench acts on, as the firmware formats them and the
//! host parser (`wellfed_core::record`) decodes them back. Each line is
//! built the way the first-boot app writes it, tag and all, after SDK noise,
//! so a renamed field or a changed word fails here rather than on the bench.
//!
//! The PLAF103's records, the updater's and the audio's among them.
#![cfg(wellfed_board = "plaf103")]

use std::fmt;

use wellfed_core::ota::SectorStep;
use wellfed_core::ota::install::{
    Bank0Check, Bank0Error, Blocked, Marking, Refused, RefusedReason, Stop, StopStage,
};
use wellfed_core::record::{Confirm, Event, Record, Tag, Update, Watch};
use wellfed_hal::flash_lock::arming;
use wellfed_hal::ota::{confirm, update};
use wellfed_hal::startup::Report;
use wellfed_hal::{audio, ble};
use wellfed_signed::offer::{KEY_ID_LEN, KeyStatement, OfferError};

struct Body<F>(F);

impl<F: Fn(&mut fmt::Formatter<'_>) -> fmt::Result> fmt::Display for Body<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (self.0)(f)
    }
}

fn line(tag: &str, body: impl Fn(&mut fmt::Formatter<'_>) -> fmt::Result) -> String {
    format!("[sdk] {tag} {}\r\n", Body(body))
}

fn read(line: &str) -> Record<'_> {
    Record::parse(line).unwrap_or_else(|| panic!("no record in {line:?}"))
}

/// An update record, under the tag the stage gives it, decodes to
/// `expected`.
fn update_decodes(record: update::Record, expected: Option<Event<'_>>) {
    let text = line(record.tag(), |f| update::describe(record, f));
    assert_eq!(read(&text).event(), expected, "{text}");
}

fn confirm_decodes(record: confirm::Record, expected: Option<Event<'_>>) {
    let text = line(Tag::Confirm.as_str(), |f| confirm::describe(record, f));
    assert_eq!(read(&text).event(), expected, "{text}");
}

#[test]
fn update_records_decode_to_what_they_say() {
    let cases = [
        (
            update::Record::Manifest {
                version: 33,
                bytes: 696_320,
            },
            Event::Update(Update::Manifest {
                version: Some(33),
                bytes: Some(696_320),
            }),
        ),
        (
            update::Record::Progress {
                sectors: 48,
                total: 170,
            },
            Event::Update(Update::Progress {
                sectors: 48,
                total: 170,
            }),
        ),
        (
            update::Record::Activated {
                version: 33,
                bytes: 696_320,
                sectors: 170,
                marking: Marking::ReadBack,
            },
            Event::Update(Update::Activated { version: Some(33) }),
        ),
        (
            update::Record::Activated {
                version: 33,
                bytes: 696_320,
                sectors: 170,
                marking: Marking::Uncertain,
            },
            Event::Update(Update::Activated { version: Some(33) }),
        ),
        (
            update::Record::Refused {
                refused: Refused::Offer(OfferError::Same),
            },
            Event::Update(Update::Refused {
                reason: Refused::Offer(OfferError::Same).reason(),
            }),
        ),
        (
            update::Record::GaveWay { version: 34 },
            Event::Watch(Watch::GiveWay { version: Some(34) }),
        ),
        (
            update::Record::Offer { version: 34 },
            Event::Watch(Watch::Offer { version: Some(34) }),
        ),
        (
            update::Record::GiveWayRefused {
                blocked: Blocked::Changed,
                found: update::Found::default(),
            },
            Event::Watch(Watch::GiveWayRefused {
                reason: Blocked::Changed,
            }),
        ),
        (
            update::Record::GiveWayFailed,
            Event::Watch(Watch::GiveWayFailed),
        ),
        (
            update::Record::Bank0 {
                len: 69_632,
                recorded: true,
            },
            Event::Update(Update::Bank0Recorded {
                bytes: 69_632,
                written: true,
            }),
        ),
    ];
    for error in Bank0Error::ALL.iter().copied() {
        update_decodes(
            update::Record::Bank0Unrecorded { error },
            Some(Event::Update(Update::Bank0Unrecorded { error })),
        );
    }
    let checks = [Bank0Check::Unchecked, Bank0Check::Match]
        .into_iter()
        .chain(Blocked::ALL.iter().copied().map(Bank0Check::Blocked));
    for check in checks {
        update_decodes(
            update::Record::Bank0Check {
                check,
                found: update::Found::default(),
            },
            Some(Event::Update(Update::Bank0Check { check })),
        );
    }
    for blocked in Blocked::ALL.iter().copied() {
        update_decodes(
            update::Record::GiveWayRefused {
                blocked,
                found: update::Found::default(),
            },
            Some(Event::Watch(Watch::GiveWayRefused { reason: blocked })),
        );
    }
    for (record, expected) in cases {
        update_decodes(record, Some(expected));
    }

    for stop in [
        Stop::Timeout,
        Stop::Cancelled,
        Stop::Order,
        Stop::Marker,
        Stop::Write,
        Stop::Hash,
        Stop::Activate,
        Stop::Store,
    ] {
        update_decodes(
            update::Record::Stopped { stop, write: None },
            Some(Event::Update(Update::Stopped {
                stage: stop.stage(),
            })),
        );
    }
}

#[test]
fn confirmation_records_decode_to_what_they_say() {
    confirm_decodes(
        confirm::Record::Confirmed {
            healthy_ms: 30_000,
            version: 12,
        },
        Some(Event::Confirm(Confirm::Confirmed {
            healthy_ms: Some(30_000),
        })),
    );
    confirm_decodes(
        confirm::Record::Already,
        Some(Event::Confirm(Confirm::Confirmed { healthy_ms: None })),
    );
    confirm_decodes(
        confirm::Record::Error {
            stage: confirm::ErrorStage::Write,
        },
        Some(Event::Confirm(Confirm::Error { stage: "write" })),
    );
    for &reason in confirm::GiveWayReason::ALL {
        confirm_decodes(
            confirm::Record::GiveWay { reason },
            Some(Event::Confirm(Confirm::GiveWay {
                reason: reason.as_str(),
            })),
        );
    }
}

/// Only a confirmation no later boot could pass either gives the trial up
/// at once: a floor it cannot raise. A failed read or write of the record,
/// or arming that failed (a skip after a hang arms again next boot), keeps
/// the trial's deadline and KM0's count.
#[test]
fn only_a_lasting_failure_gives_the_trial_up() {
    use confirm::{ErrorStage, GiveWayReason, Record};
    use wellfed_hal::flash_lock::arming::Event as Arming;

    let floor = Record::Error {
        stage: ErrorStage::Floor,
    };
    assert_eq!(GiveWayReason::lasting(floor), Some(GiveWayReason::Floor));

    let confirmed = Record::Confirmed {
        healthy_ms: 30_000,
        version: 0,
    };
    let passing = [
        Record::Already,
        confirmed,
        Record::Error {
            stage: ErrorStage::Read,
        },
        Record::Error {
            stage: ErrorStage::Write,
        },
        Record::Arming(Arming::Timeout),
        Record::Arming(Arming::Clock),
        Record::Arming(Arming::Skip { marker: 0x100 }),
    ];
    for record in passing {
        assert_eq!(GiveWayReason::lasting(record), None, "{record:?}");
    }
}

/// The words the bench matches, as they were when these fields were plain
/// strings.
#[test]
fn update_words_are_unchanged() {
    assert_eq!(
        displayed(StopStage::ALL, StopStage::as_str),
        [
            "timeout",
            "cancelled",
            "order",
            "image",
            "write",
            "hash",
            "marker",
            "store"
        ]
    );
    assert_eq!(
        displayed(RefusedReason::ALL, RefusedReason::as_str),
        [
            "downgrade",
            "same",
            "key",
            "signature",
            "statement",
            "manifest",
            "store",
            "busy",
            "fallback",
            "pending",
            "failed",
            "attempt",
            "installing",
            "not-installing",
            "model"
        ]
    );
    assert_eq!(
        displayed(Blocked::ALL, Blocked::as_str),
        ["marker", "unrecorded", "key", "extent", "changed", "read"]
    );
    assert_eq!(
        displayed(Bank0Error::ALL, Bank0Error::as_str),
        ["extent", "read", "store"]
    );
    assert_eq!(
        displayed(SectorStep::ALL, SectorStep::as_str),
        ["erase", "program", "read", "readback"]
    );

    // The bench acts on these two: the offer is the running build, and an
    // earlier attempt reset this power cycle.
    assert_eq!(
        Refused::Offer(OfferError::Same).reason(),
        RefusedReason::Same
    );
    assert_eq!(Refused::Attempt.reason(), RefusedReason::Attempt);
}

fn text(write: impl FnOnce(&mut String) -> fmt::Result) -> String {
    let mut out = String::new();
    write(&mut out).unwrap();
    out
}

fn words<T: Copy>(all: &[T], as_str: fn(T) -> &'static str) -> Vec<&'static str> {
    all.iter().map(|&value| as_str(value)).collect()
}

/// `named_enum!` types also print through `Display`, which `describe` uses.
fn displayed<T: fmt::Display + Copy>(
    all: &[T],
    as_str: fn(T) -> &'static str,
) -> Vec<&'static str> {
    for &value in all {
        assert_eq!(value.to_string(), as_str(value));
    }
    words(all, as_str)
}

/// The words the stage and reason fields carry on the UART, as they were
/// when these fields were plain strings: the host tools match them.
#[test]
fn stage_and_reason_words_are_unchanged() {
    assert_eq!(
        displayed(confirm::ErrorStage::ALL, confirm::ErrorStage::as_str),
        ["read", "write", "floor"]
    );
    assert_eq!(
        displayed(audio::ErrorStage::ALL, audio::ErrorStage::as_str),
        ["open", "clip"]
    );
    assert_eq!(
        displayed(ble::FailStage::ALL, ble::FailStage::as_str),
        ["task", "radio", "bte", "service", "stack", "timeout"]
    );
    assert_eq!(
        displayed(ble::DropReason::ALL, ble::DropReason::as_str),
        ["notify", "too_long", "timeout", "protocol", "done"]
    );
}

#[test]
fn ble_records_carry_what_a_bench_run_reads() {
    let dropped = line(Tag::Ble.as_str(), |f| {
        ble::describe(
            ble::Record::Drop {
                reason: ble::DropReason::Protocol,
            },
            f,
        )
    });
    let record = read(&dropped);
    assert_eq!(record.first, "drop");
    assert_eq!(record.kind(), Some(Tag::Ble));
    assert_eq!(record.field("reason"), Some("protocol"));

    let advertising = line(Tag::Ble.as_str(), |f| {
        ble::describe(ble::Record::Advertising { on: true }, f)
    });
    assert_eq!(read(&advertising).flag("on"), Some(true));

    let disconnect = line(Tag::Ble.as_str(), |f| {
        ble::describe(ble::Record::Disconnect { cause: 0x113 }, f)
    });
    assert_eq!(read(&disconnect).field("cause"), Some("0x113"));
}

#[test]
fn stage_and_reason_records_format_as_before() {
    let cases = [
        (
            text(|f| {
                update::describe(
                    update::Record::Stopped {
                        stop: Stop::Activate,
                        write: None,
                    },
                    f,
                )
            }),
            "error stage=marker",
        ),
        (
            text(|f| {
                update::describe(
                    update::Record::Stopped {
                        stop: Stop::Hash,
                        write: None,
                    },
                    f,
                )
            }),
            "error stage=hash",
        ),
        (
            text(|f| {
                update::describe(
                    update::Record::GiveWayRefused {
                        blocked: Blocked::Changed,
                        found: update::Found {
                            recorded_len: Some(1_048_576),
                            found_len: Some(1_052_672),
                            recorded: Some([0xde, 0xad, 0xbe, 0xef]),
                            found: Some([0x01, 0x02, 0x03, 0x04]),
                        },
                    },
                    f,
                )
            }),
            "refused give_way reason=changed recorded_len=1048576 found_len=1052672 \
             recorded=deadbeef found=01020304",
        ),
        (
            text(|f| {
                update::describe(
                    update::Record::GiveWayRefused {
                        blocked: Blocked::Unrecorded,
                        found: update::Found::default(),
                    },
                    f,
                )
            }),
            "refused give_way reason=unrecorded",
        ),
        (
            text(|f| update::describe(update::Record::GiveWayFailed, f)),
            "error stage=give_way_write",
        ),
        (
            text(|f| {
                update::describe(
                    update::Record::Bank0Check {
                        check: Bank0Check::Blocked(Blocked::Read),
                        found: update::Found {
                            recorded_len: Some(4096),
                            found_len: Some(4096),
                            recorded: Some([0xab; 4]),
                            found: None,
                        },
                    },
                    f,
                )
            }),
            "bank0 check=read recorded_len=4096 found_len=4096 recorded=abababab",
        ),
        (
            text(|f| {
                update::describe(
                    update::Record::Bank0Unrecorded {
                        error: Bank0Error::Extent,
                    },
                    f,
                )
            }),
            "bank0 recorded=0 error=extent",
        ),
        (
            text(|f| {
                confirm::describe(
                    confirm::Record::Error {
                        stage: confirm::ErrorStage::Write,
                    },
                    f,
                )
            }),
            "error stage=write",
        ),
        (
            text(|f| {
                audio::describe(
                    audio::Record::Error {
                        stage: audio::ErrorStage::Open,
                    },
                    f,
                )
            }),
            "error stage=open",
        ),
    ];
    for (got, expected) in cases {
        assert_eq!(got, expected);
    }
}

/// The update and confirmation records at their longest fit the record
/// buffer the runtime logs them through (`startup::LINE`), so none is cut
/// on the UART, and each reads back as its tag.
#[test]
fn update_and_confirm_records_at_their_longest_fit_the_line() {
    let found = update::Found {
        recorded_len: Some(u32::MAX),
        found_len: Some(u32::MAX),
        recorded: Some([0xff; 4]),
        found: Some([0xff; 4]),
    };
    let mut updates = vec![
        update::Record::Bank0 {
            len: u32::MAX,
            recorded: true,
        },
        update::Record::Bank0Check {
            check: Bank0Check::Unchecked,
            found,
        },
        update::Record::Offer { version: u32::MAX },
        update::Record::GaveWay { version: u32::MAX },
        update::Record::GiveWayFailed,
        update::Record::Manifest {
            version: u32::MAX,
            bytes: u32::MAX,
        },
        update::Record::Progress {
            sectors: u32::MAX,
            total: u32::MAX,
        },
        update::Record::Activated {
            version: u32::MAX,
            bytes: u32::MAX,
            sectors: u32::MAX,
            marking: Marking::Uncertain,
        },
        update::Record::Key {
            statement: KeyStatement::Add([0xff; 32]),
        },
        update::Record::Key {
            statement: KeyStatement::Revoke([0xff; KEY_ID_LEN]),
        },
    ];
    for &blocked in Blocked::ALL {
        updates.push(update::Record::Bank0Check {
            check: Bank0Check::Blocked(blocked),
            found,
        });
        updates.push(update::Record::GiveWayRefused { blocked, found });
    }
    for &error in Bank0Error::ALL {
        updates.push(update::Record::Bank0Unrecorded { error });
    }
    for &step in SectorStep::ALL {
        updates.push(update::Record::Stopped {
            stop: Stop::Write,
            write: Some(update::WriteFault {
                sector: u32::MAX,
                step,
                offset: u32::MAX,
                expected: u32::MAX,
                read: u32::MAX,
                reread: u32::MAX,
                erase_ms: u32::MAX,
            }),
        });
    }

    for record in updates {
        let mut report = Report::new();
        let line = report.record(format_args!(
            "{} {}\r\n",
            record.tag(),
            Body(|f: &mut fmt::Formatter<'_>| update::describe(record, f))
        ));
        let text = line.to_str().unwrap().to_owned();
        assert!(!report.truncated(), "cut: {record:?}");
        assert_eq!(read(&text).tag, record.tag(), "{text}");
    }

    let confirms = [
        confirm::Record::Already,
        confirm::Record::Arming(arming::Event::Skip { marker: u32::MAX }),
        confirm::Record::Arming(arming::Event::Armed {
            watchdog: true,
            installed: true,
        }),
        confirm::Record::Confirmed {
            healthy_ms: u32::MAX,
            version: u32::MAX,
        },
    ];
    for record in confirms {
        let mut report = Report::new();
        let line = report.record(format_args!(
            "{} {}\r\n",
            Tag::Confirm,
            Body(|f: &mut fmt::Formatter<'_>| confirm::describe(record, f))
        ));
        let text = line.to_str().unwrap().to_owned();
        assert!(!report.truncated(), "cut: {record:?}");
        assert_eq!(read(&text).kind(), Some(Tag::Confirm), "{text}");
    }
}
