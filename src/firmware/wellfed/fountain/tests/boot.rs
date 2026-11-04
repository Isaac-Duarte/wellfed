//! The boot test against the fake GPIO model: the pump is latched LOW before
//! it is an output and no other pin is configured, the lines name the model,
//! the bank and the pump, and a beat that finds the pump not stopped latches
//! it LOW again and says so.
#![cfg(wellfed_board = "plwf106")]

use std::cell::Cell;

use wellfed_fw_fountain::boot::{self, Pump};
use wellfed_hal::board::pins::{PUMP, PUMP_BIT};
use wellfed_hal::sdk::fake;
use wellfed_hal::startup::{Report, Snapshot};

std::thread_local! {
    static DROVE_HIGH: Cell<bool> = const { Cell::new(false) };
}

fn watch_pump() {
    fake::with(|h| {
        if h.direction[0] & h.output[0] & PUMP_BIT != 0 {
            DROVE_HIGH.set(true);
        }
    });
}

/// KM0's image mapped from bank 1 (0x186000), as the loader leaves it.
const BANK1: Snapshot = Snapshot {
    control: 3,
    start: 0x0c00_0000,
    offset: 0x0c00_0000 - 0x0818_6000,
    reset: 0,
};

const OFF: Pump = Pump {
    stopped: true,
    latch_high: false,
    pad_high: false,
    output: true,
};

/// From a latch left HIGH, the start never makes the pump an output at HIGH,
/// and touches no pin but the pump.
#[test]
fn the_start_latches_the_pump_off_first_and_nothing_else() {
    fake::reset_hardware();
    DROVE_HIGH.set(false);
    fake::with(|h| {
        h.output[0] = PUMP_BIT;
        h.output_hook = Some(watch_pump);
        h.direction_hook = Some(watch_pump);
    });
    let before = fake::with(|h| (h.direction, h.pad_control, h.swd_pmux));

    assert!(boot::start(|| BANK1));

    assert!(!DROVE_HIGH.get());
    fake::with(|h| {
        assert_eq!(h.output[0] & PUMP_BIT, 0);
        assert_eq!(h.direction, [before.0[0] | PUMP_BIT, before.0[1]]);
        for (pin, (now, then)) in h.pad_control.iter().zip(before.1).enumerate() {
            if pin != PUMP as usize {
                assert_eq!(*now, then, "pin {pin}");
            }
        }
        assert_eq!(h.swd_pmux, before.2);
    });
}

#[test]
fn a_beat_relatches_a_pump_that_reads_on() {
    fake::reset_hardware();
    assert!(boot::start(|| BANK1));
    assert_eq!(boot::beat(1), OFF);

    fake::with(|h| h.output[0] |= PUMP_BIT);
    assert_eq!(boot::beat(2), OFF);
    fake::with(|h| assert_eq!(h.output[0] & PUMP_BIT, 0));
}

#[test]
fn the_lines_name_the_model_bank_and_pump() {
    let mut report = Report::new();

    let line = boot::start_line(&mut report, BANK1, OFF).unwrap();
    assert_eq!(
        line.to_str().unwrap(),
        "PLBOOT1 start model=plwf106 bank=1 pump=off latch=0 pad=0 output=1\r\n"
    );

    let on = Pump {
        stopped: false,
        latch_high: true,
        pad_high: true,
        output: true,
    };
    let line = boot::beat_line(&mut report, 7, 35_000, on, true).unwrap();
    assert_eq!(
        line.to_str().unwrap(),
        "PLBOOT1 beat n=7 uptime_ms=35000 pump=unverified latch=1 pad=1 output=1 relatched=1\r\n"
    );
}
