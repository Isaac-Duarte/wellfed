//! The on-flash bytes of the feed journal's records, pinned: its genesis,
//! reservation and each kind of result (the uncertain resolution among
//! them). These are the layouts deployed feeders hold, so a change to the
//! shared slot layer (`wellfed_core::store::slot`) must leave every byte
//! where it was. The units are larger than the records so the bytes after
//! each record are pinned erased too.
//!
//! The reader is pinned apart from the writer: the journal opens the
//! pinned bytes, placed in erased flash, and reads back the values that
//! wrote them.

use wellfed_core_feeder::feed::controller::{FeedReason, FeedRequest};
use wellfed_core_feeder::feed::progress::{BowlMode, Direction};
use wellfed_core_feeder::feed::storage::{OwnerOutcome, OwnerResult};
use wellfed_core_feeder::feed_journal::{FeedJournal, JournalConfig, JournalError};
use wellfed_testkit::journal_flash::{FakeJournalFlash, FlashIo};

/// `bytes` as lowercase hex, 32 bytes a line.
fn lines(bytes: &[u8]) -> Vec<String> {
    bytes
        .chunks(32)
        .map(|line| line.iter().map(|b| format!("{b:02x}")).collect())
        .collect()
}

/// Unit `slot` must hold exactly `golden` followed by erased bytes; a
/// mismatch prints the record found in the form this file pins it.
fn pinned(name: &str, f: &FakeJournalFlash, slot: usize, len: usize, golden: &[&str]) {
    let found = lines(&f.bytes[slot][..len]);
    if found != golden {
        let shown: Vec<String> = found
            .iter()
            .map(|l| format!("            \"{l}\","))
            .collect();
        panic!("{name} changed; found:\n{}", shown.join("\n"));
    }
    assert!(
        f.bytes[slot][len..].iter().all(|&b| b == u8::MAX),
        "{name}: bytes after the record"
    );
}

const JOURNAL: JournalConfig = JournalConfig {
    binding: 0x91a5_7320_ae11_c008,
    mode: BowlMode::Double,
    max_portions: 3,
};

fn request(j: &FeedJournal<FlashIo>, id: u64, portions: u16) -> FeedRequest {
    FeedRequest {
        id,
        portions,
        mode: j.config.mode,
        direction: j.view.recovery.direction,
        credit_next_event: j.view.recovery.credit_next_event,
        phase_known: j.view.recovery.phase_known,
        homing: false,
    }
}

/// A finished double-bowl feed of `requested` portions.
fn finish(j: &mut FeedJournal<FlashIo>, outcome: OwnerOutcome) -> bool {
    let mut phase = j.view.recovery;
    phase.pending_request = false;
    let mut result = j.view.result;
    match outcome {
        OwnerOutcome::Done => {
            result.completed = result.requested;
            result.reason = FeedReason::Done;
            result.motion_possible = true;
            if result.requested & 1 == 0 {
                phase.direction = phase.direction.toggled();
            }
            phase.credit_next_event = true;
        }
        OwnerOutcome::Cancelled => {
            result.reason = FeedReason::Cancelled;
            result.motion_possible = false;
        }
        _ => {
            result.reason = FeedReason::TotalTimeout;
            result.motion_possible = false;
        }
    }
    result.outcome = outcome;
    j.finish(&result, &phase)
}

#[test]
fn journal_records() {
    let mut f = FakeJournalFlash::with_unit(512);
    let mut j = FeedJournal::default();
    // SAFETY: the fake flash outlives the journal.
    let created = unsafe { j.create(f.io(), JOURNAL, Direction::D1, false, true) };
    assert_eq!(created, JournalError::Ok);
    pinned("genesis", &f, 0, 128, GENESIS);

    assert!(j.reserve(&request(&j, 0x0102_0304_0506_0708, 2)));
    pinned("reservation", &f, 1, 128, RESERVATION);
    assert!(finish(&mut j, OwnerOutcome::Done));
    pinned("done", &f, 0, 128, DONE);

    assert!(j.reserve(&request(&j, 0x0102_0304_0506_0709, 3)));
    assert!(finish(&mut j, OwnerOutcome::Cancelled));
    pinned("cancelled", &f, 0, 128, CANCELLED);

    assert!(j.reserve(&request(&j, 0x0102_0304_0506_070a, 1)));
    assert!(finish(&mut j, OwnerOutcome::Expired));
    pinned("expired", &f, 0, 128, EXPIRED);

    // A reservation the next boot resolves as uncertain.
    assert!(j.reserve(&request(&j, 0x0102_0304_0506_070b, 2)));
    let mut j = FeedJournal::default();
    // SAFETY: as above.
    assert_eq!(unsafe { j.open(f.io(), JOURNAL) }, JournalError::Ok);
    let resolved = j.resolve().expect("the reservation resolves");
    assert_eq!(
        resolved,
        OwnerResult {
            id: 0x0102_0304_0506_070b,
            requested: 2,
            completed: 0,
            outcome: OwnerOutcome::Uncertain,
            reason: FeedReason::None,
            motion_possible: true,
        }
    );
    pinned("uncertain", &f, 0, 128, UNCERTAIN);

    // A single bowl's genesis carries its mode and the known phase.
    let mut f = FakeJournalFlash::with_unit(512);
    let mut j = FeedJournal::default();
    let single = JournalConfig {
        mode: BowlMode::Single,
        ..JOURNAL
    };
    // SAFETY: as above.
    let created = unsafe { j.create(f.io(), single, Direction::D0, true, true) };
    assert_eq!(created, JournalError::Ok);
    pinned("single genesis", &f, 0, 128, SINGLE_GENESIS);
}

/// Erased units of `unit` bytes holding each `(slot, record)` given.
fn flash(unit: usize, records: &[(usize, &[&str])]) -> FakeJournalFlash {
    let mut f = FakeJournalFlash::with_unit(unit);
    for &(slot, lines) in records {
        let hex: String = lines.concat();
        for (i, pair) in hex.as_bytes().chunks(2).enumerate() {
            let pair = core::str::from_utf8(pair).unwrap();
            f.bytes[slot][i] = u8::from_str_radix(pair, 16).unwrap();
        }
    }
    f
}

fn open_journal(f: &mut FakeJournalFlash, config: JournalConfig) -> FeedJournal<FlashIo> {
    let mut j = FeedJournal::default();
    // SAFETY: the fake flash outlives the journal.
    assert_eq!(unsafe { j.open(f.io(), config) }, JournalError::Ok);
    j
}

#[test]
fn journal_reads_the_pinned_records() {
    let mut f = flash(512, &[(0, GENESIS)]);
    let v = open_journal(&mut f, JOURNAL).read().unwrap();
    assert_eq!((v.sequence, v.has_result), (1, false));
    assert_eq!(v.recovery.direction, Direction::D1);
    assert!(!v.recovery.credit_next_event && v.recovery.phase_known);
    assert!(!v.recovery.pending_request && v.recovery.high_water_id == 0);

    let mut f = flash(512, &[(0, GENESIS), (1, RESERVATION)]);
    let j = open_journal(&mut f, JOURNAL);
    let v = j.view;
    assert_eq!(
        (v.sequence, v.recovery.high_water_id),
        (2, 0x0102_0304_0506_0708)
    );
    assert!(v.recovery.pending_request && v.result.requested == 2);

    let mut f = flash(512, &[(1, RESERVATION), (0, DONE)]);
    let v = open_journal(&mut f, JOURNAL).view;
    assert_eq!(v.sequence, 3);
    assert_eq!(
        v.result,
        OwnerResult {
            id: 0x0102_0304_0506_0708,
            requested: 2,
            completed: 2,
            outcome: OwnerOutcome::Done,
            reason: FeedReason::Done,
            motion_possible: true,
        }
    );
    assert_eq!(
        v.recovery.direction,
        Direction::D0,
        "two portions turn the bowl"
    );
    assert!(v.recovery.credit_next_event && !v.recovery.pending_request);

    for (record, sequence, id, outcome, reason) in [
        (
            CANCELLED,
            5,
            0x0102_0304_0506_0709,
            OwnerOutcome::Cancelled,
            FeedReason::Cancelled,
        ),
        (
            EXPIRED,
            7,
            0x0102_0304_0506_070a,
            OwnerOutcome::Expired,
            FeedReason::TotalTimeout,
        ),
        (
            UNCERTAIN,
            9,
            0x0102_0304_0506_070b,
            OwnerOutcome::Uncertain,
            FeedReason::None,
        ),
    ] {
        let mut f = flash(512, &[(0, record)]);
        let v = open_journal(&mut f, JOURNAL).view;
        assert_eq!(
            (v.sequence, v.result.id, v.recovery.high_water_id),
            (sequence, id, id)
        );
        assert_eq!((v.result.outcome, v.result.reason), (outcome, reason));
        assert_eq!(v.result.motion_possible, outcome == OwnerOutcome::Uncertain);
        assert_eq!(v.result.completed, 0);
    }
    // An uncertain double-bowl result leaves the phase unknown.
    let mut f = flash(512, &[(0, UNCERTAIN)]);
    assert!(!open_journal(&mut f, JOURNAL).view.recovery.phase_known);

    let single = JournalConfig {
        mode: BowlMode::Single,
        ..JOURNAL
    };
    let mut f = flash(512, &[(0, SINGLE_GENESIS)]);
    let v = open_journal(&mut f, single).view;
    assert_eq!((v.sequence, v.recovery.direction), (1, Direction::D0));
    assert!(v.recovery.credit_next_event && v.recovery.phase_known);
}

/// Genesis.
const GENESIS: &[&str] = &[
    "504c464a4e4c30310100800000010102010000000000000008c011ae2073a591",
    "0000000000000000000000000000030000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "000000000000000000000000000000000000000000000000b45dd5cb434d4954",
];

/// Reservation.
const RESERVATION: &[&str] = &[
    "504c464a4e4c30310100800001010102020000000000000008c011ae2073a591",
    "0807060504030201020000000000030000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "00000000000000000000000000000000000000000000000068efeb55434d4954",
];

/// Done.
const DONE: &[&str] = &[
    "504c464a4e4c30310100800002010007030000000000000008c011ae2073a591",
    "0807060504030201020002000101030000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "00000000000000000000000000000000000000000000000098dbedf8434d4954",
];

/// Cancelled.
const CANCELLED: &[&str] = &[
    "504c464a4e4c30310100800002010003050000000000000008c011ae2073a591",
    "0907060504030201030000000202030000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "00000000000000000000000000000000000000000000000082b4a4e2434d4954",
];

/// Expired.
const EXPIRED: &[&str] = &[
    "504c464a4e4c30310100800002010003070000000000000008c011ae2073a591",
    "0a07060504030201010000000305030000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000007730902d434d4954",
];

/// Uncertain.
const UNCERTAIN: &[&str] = &[
    "504c464a4e4c30310100800002010005090000000000000008c011ae2073a591",
    "0b07060504030201020000000500030000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "000000000000000000000000000000000000000000000000e7e0c692434d4954",
];

/// Single genesis.
const SINGLE_GENESIS: &[&str] = &[
    "504c464a4e4c30310100800000000003010000000000000008c011ae2073a591",
    "0000000000000000000000000000030000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000002434cf23434d4954",
];
