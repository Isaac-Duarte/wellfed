//! The fountain's stopped state against the fake GPIO model: the pump enable
//! is latched LOW before it becomes an output, so it never drives HIGH on
//! the way, no other pin is configured, and a pump that reads on, or an
//! enable that is no output, never reports stopped. Every stop path, the
//! panic's included, clears the pump and leaves every other pin alone.
#![cfg(wellfed_board = "plwf106")]

use std::cell::Cell;

use wellfed_hal::board::pins::{self, PUMP, PUMP_BIT};
use wellfed_hal::sdk::{self, fake};
use wellfed_hal::stopped::{self, PA_MASK, PB_MASK};

std::thread_local! {
    /// Set once the pump enable has driven HIGH from an output.
    static DROVE_HIGH: Cell<bool> = const { Cell::new(false) };
}

fn check_never_driven_high() {
    fake::with(|h| {
        if h.direction[0] & h.output[0] & PUMP_BIT != 0 {
            DROVE_HIGH.set(true);
        }
    });
}

#[test]
fn the_safe_state_is_the_pump_alone() {
    assert_eq!((PA_MASK, PB_MASK), (1 << 30, 0));
}

/// From reset and from a latch left HIGH (a warm restart with the pump on),
/// startup latches LOW first, so the enable is never an output at HIGH.
#[test]
fn startup_latches_the_pump_off_before_it_is_an_output() {
    for (latch, direction) in [(0, 0), (PUMP_BIT, 0), (PUMP_BIT, PUMP_BIT)] {
        fake::reset_hardware();
        DROVE_HIGH.set(false);
        fake::with(|h| {
            h.output[0] = latch;
            h.direction[0] = direction;
            h.output_hook = Some(check_never_driven_high);
            h.direction_hook = Some(check_never_driven_high);
        });

        assert!(stopped::initialize().verified());
        assert!(
            !DROVE_HIGH.get(),
            "drove HIGH from {latch:#x}, {direction:#x}"
        );
        fake::with(|h| {
            assert_eq!(h.output[0] & PUMP_BIT, 0);
            assert_eq!(h.direction[0] & PUMP_BIT, PUMP_BIT);
            assert_eq!(h.pad_control[PUMP as usize] & 0x1f, 0, "selected as GPIO");
        });
    }
}

/// Only the pump is configured: the button, the scale, the indicators and
/// the outputs of unknown role keep their reset state, and SWD is left as
/// it was, for the fountain's own drivers.
#[test]
fn startup_configures_nothing_but_the_pump() {
    fake::reset_hardware();
    let before = fake::with(|h| (h.output, h.direction, h.swd_pmux));

    assert!(stopped::initialize().verified());

    fake::with(|h| {
        assert_eq!(h.output, before.0);
        assert_eq!(h.direction[0], before.1[0] | PUMP_BIT);
        assert_eq!(h.direction[1], before.1[1]);
        assert_eq!(h.swd_pmux, before.2);
    });
    let others = [
        pins::BUTTON,
        pins::SCALE_CLOCK,
        pins::SCALE_DATA,
        pins::LED_WATER_NORMAL,
        pins::LED_WATER_LOW,
    ]
    .into_iter()
    .chain(pins::UNKNOWN_HIGH);
    for pin in others {
        let bit = pins::bit(pin);
        fake::with(|h| assert_eq!(h.direction[(pin >> 5) as usize] & bit, 0, "pin {pin}"));
    }
}

#[test]
fn a_pump_that_reads_on_or_is_no_output_is_not_stopped() {
    fake::reset_hardware();
    assert!(stopped::initialize().verified());

    fake::with(|h| h.output[0] |= PUMP_BIT);
    assert!(!stopped::observe().verified());
    fake::with(|h| h.output[0] &= !PUMP_BIT);
    assert!(stopped::observe().verified());

    fake::with(|h| h.direction[0] &= !PUMP_BIT);
    assert!(!stopped::observe().verified());
}

/// The panic path clears the pump and writes no other pin, and writes
/// nothing at all while the GPIO block is unclocked.
#[test]
fn the_panic_stop_clears_only_the_pump() {
    let others = [1 << 19 | 1 << 26, 1 << 5 | 1 << 22];

    fake::reset_hardware();
    fake::with(|h| h.output = [others[0] | PUMP_BIT, others[1]]);
    sdk::panic_outputs_safe();
    fake::with(|h| assert_eq!(h.output, [others[0] | PUMP_BIT, others[1]]));

    fake::reset_hardware();
    assert!(stopped::initialize().verified());
    fake::with(|h| h.output = [others[0] | PUMP_BIT, others[1]]);
    sdk::panic_outputs_safe();
    fake::with(|h| {
        assert_eq!(h.interrupts, 1);
        assert_eq!(h.output, others);
    });
}

/// The fountain has no store: the boot trial has no record to read, and
/// every storage sector is refused.
#[test]
fn there_is_no_store_to_confirm_in() {
    assert_eq!(wellfed_hal::trial::CONFIRM_SECTOR, None);

    fake::reset_hardware();
    let mut byte = [0];
    assert!(!sdk::storage_read(0, 0, &mut byte));
    // SAFETY: the fake's storage, which nothing else uses here.
    assert!(!unsafe { sdk::storage_erase(0) });
}
