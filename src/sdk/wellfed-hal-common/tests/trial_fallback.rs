//! Bank 0 as the trial's fallback: its marker counts only with the five core
//! headers the loader maps behind it, so a trial or a storm never gives way
//! onto a bank 0 the loader would not boot.

use wellfed_core::ota::{CONFIRM_LEN, confirm_record};
use wellfed_hal_common::trial::{BANK0_BYTES, Decision, LIMIT, STORM_LIMIT, decide, fallback};
use wellfed_signed::{MARKER, SECTOR};

const ID: [u8; 32] = [0x5a; 32];
const NONE: [u8; CONFIRM_LEN] = [0xff; CONFIRM_LEN];

/// A bank laid out as the reviewed builds and stock are: KM0's XIP and RAM
/// parts, then from the next sector KM4's XIP, RAM and empty PSRAM parts.
fn bank() -> Vec<u8> {
    let mut out = Vec::new();
    let part = |out: &mut Vec<u8>, address: u32, body: usize| {
        out.extend_from_slice(&MARKER);
        out.extend_from_slice(&u32::try_from(body).unwrap().to_le_bytes());
        out.extend_from_slice(&address.to_le_bytes());
        out.extend_from_slice(&[0xff; 16]);
        out.extend(std::iter::repeat_n(0x42, body));
    };
    part(&mut out, 0x0C00_0020, 5000);
    part(&mut out, 0x0008_3000, 300);
    out.resize(out.len().next_multiple_of(SECTOR), 0xff);
    part(&mut out, 0x0E00_0020, 20_000);
    part(&mut out, 0x1000_5000, 700);
    part(&mut out, 0x0200_0020, 0);
    out.resize(out.len().next_multiple_of(SECTOR), 0xff);
    out
}

/// Where KM4's first header starts: the sector after KM0's parts.
const KM4_HEADER: usize = 2 * SECTOR;

/// The marker the trial decides on for `bank`, read as KM0's glue maps it.
fn marker_of(bank: &[u8]) -> [u8; 8] {
    fallback(|at, header| {
        let Some(bytes) = bank.get(at as usize..).and_then(<[u8]>::first_chunk) else {
            return false;
        };
        *header = *bytes;
        true
    })
}

/// Each way bank 0 can keep its marker and still not boot.
fn damaged() -> Vec<(&'static str, Vec<u8>)> {
    let good = bank();
    let mut cases = Vec::new();
    let mut change = |label, at: usize, value: u8| {
        let mut bank = good.clone();
        bank[at] = value;
        cases.push((label, bank));
    };
    change("KM4 signature", KM4_HEADER, b'0');
    change("KM4 signature, last byte", KM4_HEADER + 7, 0);
    change("KM4 load address", KM4_HEADER + 12, 0x21);
    change("KM0 RAM signature", 32 + 5000, 0);
    change("KM4 RAM erased word", KM4_HEADER + 32 + 20_000 + 16, 0);
    change(
        "PSRAM part not empty",
        KM4_HEADER + 32 + 20_000 + 32 + 700 + 8,
        1,
    );

    let mut erased = good.clone();
    erased[KM4_HEADER..].fill(0xff);
    cases.push(("KM4 erased", erased));
    // A KM0 body so long the image would pass bank 0's end.
    let mut long = good;
    long[8..12].copy_from_slice(&BANK0_BYTES.to_le_bytes());
    cases.push(("past the bank", long));
    cases
}

#[test]
fn bank_0_counts_only_with_its_core_headers() {
    let good = bank();
    assert_eq!(&good[..8], &MARKER);
    assert_eq!(marker_of(&good), MARKER);

    for (label, bank) in damaged() {
        assert_eq!(&bank[..8], &MARKER, "{label}: the marker is intact");
        assert_ne!(marker_of(&bank), MARKER, "{label}");
    }

    let mut cleared = good;
    cleared[0] = 0;
    assert_ne!(marker_of(&cleared), MARKER);
    assert_ne!(marker_of(&[0xff; 4 * SECTOR]), MARKER);
    // A read that fails is no fallback either.
    assert_ne!(fallback(|_, _| false), MARKER);
}

/// Boot an unconfirmed (or with `record`, confirmed) bank 1 `n` times, each
/// boot ending in a reset nobody asked for, with bank 0 as `bank`.
fn boots(bank: &[u8], record: &[u8; CONFIRM_LEN], n: usize) -> Vec<Decision> {
    let bank0 = marker_of(bank);
    let mut register = 0u32;
    let mut out = Vec::new();
    for _ in 0..n {
        register = (register & 0xffff_0000) | ((register + 1) & 0xffff);
        let (decision, value) = decide(&MARKER, &bank0, record, register, &ID);
        register = value;
        out.push(decision);
    }
    out
}

#[test]
fn a_trial_never_gives_way_onto_a_bank_0_with_damaged_headers() {
    let out = boots(&bank(), &NONE, LIMIT as usize + 1);
    assert_eq!(out.last(), Some(&Decision::GiveWay));

    for (label, bank) in damaged() {
        let out = boots(&bank, &NONE, 8);
        assert!(!out.iter().any(|d| d.gives_way()), "{label}: {out:?}");
        assert_eq!(out[LIMIT as usize], Decision::NoFallback(LIMIT), "{label}");
    }
}

#[test]
fn a_storm_never_gives_way_onto_a_bank_0_with_damaged_headers() {
    let record = confirm_record(&ID, 0);
    let n = STORM_LIMIT as usize + 1;
    let out = boots(&bank(), &record, n);
    assert!(matches!(out[n - 1], Decision::StormGiveWay { .. }));

    for (label, bank) in damaged() {
        let out = boots(&bank, &record, n + 3);
        assert!(!out.iter().any(|d| d.gives_way()), "{label}: {out:?}");
        assert!(
            matches!(out[n - 1], Decision::Held { .. }),
            "{label}: held, not given away"
        );
    }
}
