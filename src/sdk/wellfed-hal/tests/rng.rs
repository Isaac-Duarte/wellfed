//! The runtime generator on the fake: a noisy ADC starts it after the
//! 1,024-sample start-up test, a steady one fails the health tests and an
//! unreadable one fails to open, and without a generator nothing is drawn.

use wellfed_hal::rng::{self, FIRST, Failure};
use wellfed_hal::sdk::fake;

fn noisy() {
    fake::reset_hardware();
    rng::reset();
    fake::with(|h| {
        h.adc_noise = 0x1234_5678;
        h.ticks_per_read = 1;
        h.systick_reload = 9_999;
        h.systick_step = 7;
    });
}

#[test]
fn a_noisy_adc_starts_the_generator() {
    noisy();
    assert!(!rng::ready());
    assert!(rng::bytes().is_none(), "nothing before start");

    let started = rng::start();
    assert_eq!(started.result, Ok(()));
    assert_eq!(started.samples, 1_024, "the start-up test's length");
    assert!(started.first.iter().any(|&b| b != started.first[0]));
    assert_eq!(started.first.len(), FIRST);

    let (a, b) = (rng::bytes().unwrap(), rng::bytes().unwrap());
    assert_ne!(a, b);
    assert!(rng::mix(b"stored seed"));
    assert_ne!(rng::bytes().unwrap(), a);
}

/// Nothing reseeds the generator after boot, so it serves every draw a
/// boot makes: past twice the 2^16 requests that once stopped it, which an
/// unanswered NTP server's nonces reached in weeks and a LAN peer opening
/// and dropping connections in minutes, it still fills.
#[test]
fn the_generator_outlasts_a_boot_s_draws() {
    noisy();
    assert_eq!(rng::start().result, Ok(()));

    let mut nonce = [0; 8];
    for _ in 0..1 << 17 {
        assert!(rng::fill(&mut nonce));
    }
    assert!(rng::bytes().is_some());
}

#[test]
fn a_steady_adc_fails_health_and_leaves_no_generator() {
    noisy();
    fake::with(|h| h.adc_noise = 0);
    let started = rng::start();
    assert_eq!(started.result, Err(Failure::Health));
    assert!(started.samples < 64, "the repetition test trips early");
    assert!(!rng::ready());
    assert!(!rng::fill(&mut [0; 8]));
    assert!(!rng::mix(b"seed"));
}

#[test]
fn an_adc_that_does_not_open_fails() {
    noisy();
    fake::with(|h| h.efuse_fail = true);
    assert_eq!(rng::start().result, Err(Failure::Adc));
    assert!(!rng::ready());
}

#[test]
fn the_same_noise_on_two_boots_still_differs_after_the_stored_seed() {
    noisy();
    rng::start();
    rng::mix(&[1; 32]);
    let first = rng::bytes();

    noisy();
    rng::start();
    rng::mix(&[2; 32]);
    assert_ne!(rng::bytes(), first);
}
