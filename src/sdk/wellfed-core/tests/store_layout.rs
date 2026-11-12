//! The on-flash bytes of every record the SDK's stores write, pinned: the
//! seed, and the update store's record in each of its states. These are
//! the layouts deployed feeders hold, so a change to the shared slot layer
//! must leave every byte where it was. The units are larger than the
//! records so the bytes after each record are pinned erased too. The feed
//! journal's are pinned beside it, in `wellfed-core-feeder`.
//!
//! The readers are pinned apart from the writers: each store opens the
//! pinned bytes, placed in erased flash, and reads back the values that
//! wrote them.

use wellfed_core::store::seed::{SeedStatus, SeedStore};
use wellfed_core::store::update::{Bank0, Candidate, State, UpdateStatus, UpdateStore};
use wellfed_signed::offer::ImageKind;
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

#[test]
fn seed_records() {
    let mut f = FakeJournalFlash::with_unit(512);
    let mut s = SeedStore::default();
    // SAFETY: the fake flash outlives the store.
    unsafe { s.open(f.io()) };
    let seed: [u8; 32] = core::array::from_fn(|i| (i as u8).wrapping_mul(7) ^ 0xa5);
    assert!(s.save(&seed, 0x5eed_0001));
    pinned("first seed", &f, 0, 128, FIRST_SEED);
    assert!(s.save(&[0xff; 32], 0x5eed_0002));
    pinned("second seed", &f, 1, 128, SECOND_SEED);
}

fn candidate(version: u32, build: u8, kind: ImageKind) -> Candidate {
    Candidate {
        digest: core::array::from_fn(|i| build ^ (i as u8)),
        version,
        build_id: core::array::from_fn(|i| build.wrapping_add(i as u8)),
        kind,
    }
}

#[test]
fn update_records() {
    let mut f = FakeJournalFlash::with_unit(1024);
    let mut s = UpdateStore::default();
    // SAFETY: the fake flash outlives the store.
    unsafe { s.open(f.io()) };

    s.set_bank0(Bank0 {
        build_id: [0x31; 32],
        len: 942_080,
        sha256: core::array::from_fn(|i| 0x80 | (i as u8)),
        key: None,
    })
    .unwrap();
    pinned("bank 0 without a key", &f, 0, 384, BANK_0_WITHOUT_A_KEY);

    s.set_pending(candidate(0x0001_0203, 0x40, ImageKind::Replacement))
        .unwrap();
    pinned("pending", &f, 1, 384, PENDING);

    s.set_activated(candidate(0x0001_0203, 0x40, ImageKind::Replacement))
        .unwrap();
    pinned("activated", &f, 0, 384, ACTIVATED);

    s.confirm(&candidate(0x0001_0203, 0x40, ImageKind::Replacement).build_id)
        .unwrap();
    pinned("confirmed", &f, 1, 384, CONFIRMED);

    s.add_key(&[0x5a; 32]).unwrap();
    s.revoke(&[1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
    s.revoke(&[9; 8]).unwrap();
    s.set_bank0(Bank0 {
        build_id: [0x32; 32],
        len: 0x0012_3456,
        sha256: [0x77; 32],
        key: Some([0xc3; 8]),
    })
    .unwrap();
    s.set_pending(candidate(0x0002_0000, 0x50, ImageKind::Stock))
        .unwrap();
    pinned(
        "keys, bank 0 with a key and a stock candidate",
        &f,
        0,
        384,
        KEYS_AND_A_STOCK_CANDIDATE,
    );
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

#[test]
fn seed_reads_the_pinned_records() {
    let mut f = flash(512, &[(0, FIRST_SEED)]);
    let mut s = SeedStore::default();
    // SAFETY: the fake flash outlives the store.
    assert_eq!(unsafe { s.open(f.io()) }, SeedStatus::Loaded);
    let seed: [u8; 32] = core::array::from_fn(|i| (i as u8).wrapping_mul(7) ^ 0xa5);
    assert_eq!((s.generation, s.seed()), (1, Some(&seed)));

    let mut f = flash(512, &[(0, FIRST_SEED), (1, SECOND_SEED)]);
    // SAFETY: as above.
    unsafe { s.open(f.io()) };
    assert_eq!((s.generation, s.slot, s.seed()), (2, 1, Some(&[0xff; 32])));
    assert_eq!(s.epoch(), 0x5eed_0002);

    // Minor 0, before the ring's epoch: read with none.
    let mut f = flash(512, &[(0, FIRST_SEED_MINOR_0)]);
    // SAFETY: as above.
    assert_eq!(unsafe { s.open(f.io()) }, SeedStatus::Loaded);
    assert_eq!((s.generation, s.seed(), s.epoch()), (1, Some(&seed), 0));
}

fn open_update(record: &[&str]) -> UpdateStore<FlashIo> {
    // The store outlives this function's flash, so the flash is leaked.
    let f = Box::leak(Box::new(flash(1024, &[(0, record)])));
    let mut s = UpdateStore::default();
    // SAFETY: the leaked flash lives for the rest of the test.
    assert_eq!(unsafe { s.open(f.io()) }, UpdateStatus::Loaded);
    s
}

#[test]
fn update_store_reads_the_pinned_records() {
    let first_bank0 = Bank0 {
        build_id: [0x31; 32],
        len: 942_080,
        sha256: core::array::from_fn(|i| 0x80 | (i as u8)),
        key: None,
    };
    let replacement = candidate(0x0001_0203, 0x40, ImageKind::Replacement);

    let s = open_update(BANK_0_WITHOUT_A_KEY);
    let r = *s.record();
    assert_eq!((s.generation, r.min_version, r.state), (1, 0, State::None));
    assert_eq!(
        (r.bank0, r.added, r.revoked().len()),
        (Some(first_bank0), None, 0)
    );

    let s = open_update(PENDING);
    assert_eq!(
        (s.generation, s.record().state),
        (2, State::Pending(replacement))
    );
    let s = open_update(ACTIVATED);
    assert_eq!(
        (s.generation, s.record().state),
        (3, State::Activated(replacement))
    );

    let s = open_update(CONFIRMED);
    let r = *s.record();
    assert_eq!(
        (s.generation, r.min_version, r.state),
        (4, 0x0001_0203, State::None)
    );
    assert_eq!(r.bank0, Some(first_bank0));

    let s = open_update(KEYS_AND_A_STOCK_CANDIDATE);
    let r = *s.record();
    assert_eq!(s.generation, 9);
    assert_eq!(r.min_version, 0x0001_0203);
    assert_eq!(
        r.state,
        State::Pending(candidate(0x0002_0000, 0x50, ImageKind::Stock))
    );
    assert_eq!(r.added, Some([0x5a; 32]));
    assert_eq!(r.revoked(), &[[1, 2, 3, 4, 5, 6, 7, 8], [9; 8]]);
    assert_eq!(
        r.bank0,
        Some(Bank0 {
            build_id: [0x32; 32],
            len: 0x0012_3456,
            sha256: [0x77; 32],
            key: Some([0xc3; 8]),
        })
    );
}

// The pinned records, as deployed feeders hold them.

/// First seed, as a minor 0 record holds it: no epoch.
const FIRST_SEED_MINOR_0: &[&str] = &[
    "504c465345454431010080000100000000000000a5a2abb0b9868f949d9ae3e8",
    "f1fec7ccd5d2db2029363f040d0a1318616e777c000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "000000000000000000000000000000000000000000000000536f144e434d4954",
];

/// First seed, with the ring's epoch (minor 1).
const FIRST_SEED: &[&str] = &[
    "504c465345454431010180000100000000000000a5a2abb0b9868f949d9ae3e8",
    "f1fec7ccd5d2db2029363f040d0a1318616e777c0100ed5e0000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "00000000000000000000000000000000000000000000000078859b72434d4954",
];

/// Second seed, with the next boot's epoch.
const SECOND_SEED: &[&str] = &[
    "504c465345454431010180000200000000000000ffffffffffffffffffffffff",
    "ffffffffffffffffffffffffffffffffffffffff0200ed5e0000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000001b672366434d4954",
];

/// Bank 0 without a key.
const BANK_0_WITHOUT_A_KEY: &[&str] = &[
    "504c465550443031020080010100000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0100000000600e00313131313131313131313131313131313131313131313131",
    "3131313131313131808182838485868788898a8b8c8d8e8f9091929394959697",
    "98999a9b9c9d9e9f000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000002a8c65b4434d4954",
];

/// Pending.
const PENDING: &[&str] = &[
    "504c465550443031020080010200000000000000000000000101000003020100",
    "404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f",
    "404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0100000000600e00313131313131313131313131313131313131313131313131",
    "3131313131313131808182838485868788898a8b8c8d8e8f9091929394959697",
    "98999a9b9c9d9e9f000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000005caa651c434d4954",
];

/// Activated.
const ACTIVATED: &[&str] = &[
    "504c465550443031020080010300000000000000000000000201000003020100",
    "404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f",
    "404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0100000000600e00313131313131313131313131313131313131313131313131",
    "3131313131313131808182838485868788898a8b8c8d8e8f9091929394959697",
    "98999a9b9c9d9e9f000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000006d62d8da434d4954",
];

/// Confirmed.
const CONFIRMED: &[&str] = &[
    "504c465550443031020080010400000000000000030201000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0100000000600e00313131313131313131313131313131313131313131313131",
    "3131313131313131808182838485868788898a8b8c8d8e8f9091929394959697",
    "98999a9b9c9d9e9f000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "000000000000000000000000000000000000000000000000dc7c9317434d4954",
];

/// Keys, bank 0 with a key and a stock candidate.
const KEYS_AND_A_STOCK_CANDIDATE: &[&str] = &[
    "504c465550443031020080010900000000000000030201000102010200000200",
    "505152535455565758595a5b5c5d5e5f404142434445464748494a4b4c4d4e4f",
    "505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f",
    "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a",
    "0101000056341200323232323232323232323232323232323232323232323232",
    "3232323232323232777777777777777777777777777777777777777777777777",
    "7777777777777777c3c3c3c3c3c3c3c301020304050607080909090909090909",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000008093f7db434d4954",
];
