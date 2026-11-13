//! The generator: HMAC-DRBG against NIST's published vector, the health
//! cutoffs against SP 800-90B's table, each health test tripped by a broken
//! source, and the pool refusing output until the start-up test and its
//! entropy claim are met.

use wellfed_core::rng::{
    APT_WINDOW, Drbg, Error, Health, MAX_REQUEST, Pool, RESEED_INTERVAL, STARTUP_SAMPLES,
};

fn hex(text: &str) -> Vec<u8> {
    let mut out = vec![0; text.len() / 2];
    wellfed_session::hex::decode(text.as_bytes(), &mut out).unwrap();
    out
}

/// A healthy stand-in source: xorshift bytes, about 8 bits a sample.
fn noise(state: &mut u32) -> u8 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    state.to_le_bytes()[0]
}

#[test]
fn hmac_drbg_matches_nist_cavp() {
    // HMAC_DRBG.rsp, [SHA-256], no prediction resistance, no reseed, no
    // personalization or additional input, 1024 bits, COUNT = 0: the
    // second of two generate calls.
    let mut drbg = Drbg::new(
        &hex("ca851911349384bffe89de1cbdc46e6831e44d34a4fb935ee285dd14b71a7488"),
        &hex("659ba96c601dc69fc902940805ec0ca8"),
        &[],
    );
    let mut out = [0; 128];
    drbg.generate(&mut out, &[]).unwrap();
    drbg.generate(&mut out, &[]).unwrap();
    assert_eq!(
        out.to_vec(),
        hex(concat!(
            "e528e9abf2dece54d47c7e75e5fe302149f817ea9fb4bee6f4199697d04d5b89",
            "d54fbb978a15b5c443c9ec21036d2460b6f73ebad0dc2aba6e624abf07745bc1",
            "07694bb7547bb0995f70de25d6b29e2d3011bb19d27676c07162c8b5ccde0668",
            "961df86803482cb37ed6d5c0bb8d50cf1f50d476aa0458bdaba806f48be9dcb8",
        ))
    );
}

#[test]
fn inputs_change_the_stream() {
    let base = || Drbg::new(&[1; 32], &[2; 16], b"wellfed");
    let first = |mut drbg: Drbg| drbg.bytes().unwrap();

    let plain = first(base());
    assert_eq!(plain, first(base()), "deterministic for the same inputs");
    assert_ne!(plain, first(Drbg::new(&[1; 32], &[3; 16], b"wellfed")));
    assert_ne!(plain, first(Drbg::new(&[1; 32], &[2; 16], b"other")));

    let mut extra = base();
    let mut out = [0; 32];
    extra.generate(&mut out, b"additional").unwrap();
    assert_ne!(plain, out);
}

#[test]
fn the_drbg_bounds_a_request() {
    let mut drbg = Drbg::new(&[1; 32], &[], &[]);
    assert_eq!(
        drbg.generate(&mut vec![0; MAX_REQUEST + 1], &[]),
        Err(Error::TooLarge)
    );
    drbg.bytes().unwrap();
}

/// The firmware never reseeds, so the generator must outlast any boot: far
/// more draws than the 2^16 that once stopped it (a few weeks of unanswered
/// NTP, or a LAN peer opening and dropping connections) still succeed.
#[test]
fn the_drbg_runs_past_twice_the_old_interval_without_a_reseed() {
    const { assert!(RESEED_INTERVAL >= 1 << 48) };
    let mut drbg = Drbg::new(&[1; 32], &[], &[]);
    for _ in 0..1 << 17 {
        drbg.generate(&mut [0; 1], &[]).unwrap();
    }
    drbg.bytes().unwrap();
}

#[test]
fn cutoffs_match_sp800_90b() {
    // SP 800-90B 4.4.1 (1 + ceil(20/H)) and Table 2 (W = 512).
    for (eighths, repetition, proportion) in [
        (4, 41, 410),
        (8, 21, 311),
        (16, 11, 177),
        (32, 6, 62),
        (64, 4, 13),
    ] {
        let health = Health::new(eighths);
        assert_eq!(health.repetition_cutoff(), repetition, "H = {eighths}/8");
        assert_eq!(health.proportion_cutoff(), proportion, "H = {eighths}/8");
    }
}

/// Every claim's Adaptive Proportion cutoff, 1 to 64 eighths, as an exact
/// binomial at 250 significant digits gives it (Python's `decimal` and
/// `math.comb`, outside the repository). The weakest claims are where a
/// direct sum from P(X = 0) underflowed and the cutoff came out 513.
const PROPORTION_CUTOFFS: [u32; 64] = [
    497, 468, 439, 410, 383, 357, 333, 311, 290, 270, 251, 234, 219, 204, 190, 177, 165, 154, 144,
    135, 126, 118, 110, 103, 97, 90, 85, 80, 75, 70, 66, 62, 58, 55, 52, 49, 46, 43, 41, 39, 37,
    35, 33, 31, 30, 28, 27, 25, 24, 23, 22, 21, 20, 19, 18, 18, 17, 16, 15, 15, 14, 14, 13, 13,
];

#[test]
fn every_claim_s_proportion_cutoff_matches_the_exact_binomial() {
    for (eighths, expected) in (1..=64).zip(PROPORTION_CUTOFFS) {
        let cutoff = Health::new(eighths).proportion_cutoff();
        assert_eq!(cutoff, expected, "{eighths} eighths");
        assert!(cutoff <= APT_WINDOW, "a window can fail it");
    }
}

/// At the weakest claims a source stuck on one value most of the time, in
/// runs too short for the Repetition Count, still fails the Adaptive
/// Proportion test within a few windows.
#[test]
fn a_biased_source_fails_at_the_weakest_claims() {
    for eighths in 1..=3 {
        let mut health = Health::new(eighths);
        let biased = (0..u32::MAX).map(|n| u8::from(n % 100 == 99));
        let failed = biased
            .take(4 * APT_WINDOW as usize)
            .any(|value| health.sample(value).is_err());
        assert!(failed, "{eighths} eighths");
    }
}

#[test]
fn a_stuck_source_fails_the_repetition_test_and_stays_failed() {
    let mut health = Health::new(8);
    for _ in 1..health.repetition_cutoff() {
        health.sample(7).unwrap();
    }
    assert_eq!(health.sample(7), Err(Error::Health));
    assert!(health.failed());
    assert_eq!(health.sample(1), Err(Error::Health), "latched");
}

#[test]
fn a_biased_source_fails_the_proportion_test() {
    // Three zeros, then another value: never a long run, but three quarters
    // of every window is the window's first value, above 311 of 512 for H = 1.
    let mut health = Health::new(8);
    let mut state = 1;
    let failed = (0..2 * 512).any(|i| {
        let value = if i % 4 != 3 { 0 } else { noise(&mut state) | 1 };
        health.sample(value).is_err()
    });
    assert!(failed);
}

#[test]
fn a_healthy_source_passes_many_windows() {
    let mut health = Health::new(8);
    let mut state = 0x1234_5678;
    for _ in 0..200_000 {
        health.sample(noise(&mut state)).unwrap();
    }
    assert!(health.started());
}

#[test]
fn the_pool_waits_for_start_up_and_its_claim() {
    // Claiming 8 bits a sample, 512 bits takes 64 samples, but the start-up
    // test needs 1024 first.
    let mut pool = Pool::new::<64>();
    let mut state = 99;
    for _ in 0..STARTUP_SAMPLES - 1 {
        pool.add(noise(&mut state)).unwrap();
    }
    assert_eq!(pool.take(), Err(Error::NotReady));
    pool.add(noise(&mut state)).unwrap();
    let first = pool.take().unwrap();

    // Each later input needs its own 512 claimed bits.
    assert_eq!(pool.take(), Err(Error::NotReady));
    for _ in 0..64 {
        pool.add(noise(&mut state)).unwrap();
    }
    assert_ne!(pool.take().unwrap(), first);

    // A weaker claim needs proportionally more samples.
    let mut pool = Pool::new::<2>();
    for _ in 0..STARTUP_SAMPLES {
        pool.add(noise(&mut state)).unwrap();
    }
    assert!(
        !pool.ready(),
        "1024 samples at a quarter bit is 256 bits, short of 512"
    );
}

#[test]
fn a_failed_pool_never_gives_output() {
    let mut pool = Pool::new::<8>();
    let mut state = 5;
    for _ in 0..STARTUP_SAMPLES * 4 {
        pool.add(noise(&mut state)).unwrap();
    }
    assert!(pool.ready());
    let stuck = (0..64).any(|_| pool.add(3).is_err());
    assert!(stuck && pool.failed());
    assert_eq!(pool.take(), Err(Error::Health));
}
