//! The PLWF106 fountain board, as the replacement firmware sees it: the pins
//! its stock firmware drives and reads, the outputs' safe state (the pump
//! off), and the stock flash a replacement must leave alone. These are facts
//! of this model, not of the AmebaD SoC it shares with the PLAF103, so a
//! build chooses this board through `wellfed-board` and never infers it from
//! the chip or by probing pins.
//!
//! Every role here is recovered from stock 0.0.17's initialization and call
//! sites ([board signals](../../../../docs/devices/plwf106/board.md)), not
//! measured: no polarity, continuity or timing has been checked on the bench.
//!
//! **No store yet.** The feeder's storage approval does not transfer, and no
//! Wellfed allocation has been reviewed for this model
//! ([flash layout](../../../../docs/devices/plwf106/flash-layout.md)). So the
//! board has a storage region of zero sectors and no sector map: every
//! storage read, program and erase is refused at its bounds check, and code
//! that names a store does not build for this board.
#![no_std]

/// The model this board is, as `wellfed_proto::device::Model::code` numbers
/// it: the PLWF106. The SDK takes no product schema, so the board names its
/// model by code and the firmware checks it against `Model`.
pub const MODEL_CODE: u8 = 2;

/// The outputs' safe state, which a fault or a flash lease writes straight
/// to the data registers without asking any driver: the pump enable cleared
/// LOW, and nothing else. Stock's every pump-off path is that one write, and
/// a fault path must not drive any other pin: the indicators are harmless
/// either way, and the scale clock is left where its transaction put it.
pub mod safe {
    use super::pins;

    pub const PORT_A_HIGH: u32 = 0;
    pub const PORT_A_LOW: u32 = pins::PUMP_BIT;
    pub const PORT_B_LOW: u32 = 0;
}

/// Every GPIO the stock firmware configures, in one place. A GPIO is
/// `port * 32 + bit` (PA30 is 30, PB5 is 37). Startup levels are stock's
/// latch values, not measured voltages.
pub mod pins {
    /// Pump enable: HIGH runs the pump, LOW stops it. Stock latches it LOW
    /// before it enables the output, with no internal pull.
    pub const PUMP: u32 = 30;
    /// The one button, an input with no internal pull, active LOW (stock's
    /// long-hold handler reads LOW as held).
    pub const BUTTON: u32 = 15;
    /// The weight sensor's two-wire transaction: the clock an output that
    /// idles LOW, the data an input. Its part is unidentified.
    pub const SCALE_CLOCK: u32 = 32 + 5;
    pub const SCALE_DATA: u32 = 32 + 6;
    /// Indicators, HIGH when lit, LOW at startup: the network pair, the
    /// three water levels (normal, low, lack) and the filter reminder. Which
    /// light pipe each lights, and its color, is unmeasured. PB3 is also
    /// SWD_CLK, which stock releases first.
    pub const LED_NETWORK: [u32; 2] = [26, 28];
    pub const LED_WATER_NORMAL: u32 = 19;
    pub const LED_WATER_LOW: u32 = 32 + 3;
    pub const LED_WATER_LACK: u32 = 25;
    pub const LED_FILTER: u32 = 32 + 7;
    /// Outputs stock initializes HIGH among its LED group (PB26, PB23,
    /// PB22) whose roles are not established: nothing it was seen to do
    /// lights them. No Wellfed driver configures them until they are
    /// measured. The unpopulated radar footprint's pins are unknown and are
    /// not listed at all.
    pub const UNKNOWN_HIGH: [u32; 3] = [32 + 26, 32 + 23, 32 + 22];

    /// The pin's bit in its port register.
    pub const fn bit(pin: u32) -> u32 {
        1 << (pin & 31)
    }

    /// The pump enable PA30 (port A).
    pub const PUMP_BIT: u32 = bit(PUMP);
}

/// The replacement firmware's storage region: none. Zero sectors, so
/// `wellfed-board`'s bounds refuse every sector, and the offset is never
/// added to anything. A store here needs its own allocation review against
/// [`stock::PRESERVED`] first.
pub const STORAGE_OFFSET: u32 = 0;
pub const STORAGE_SECTORS: u32 = 0;

/// The stock flash, as physical offsets: what a replacement image and any
/// future store must keep clear of. A minimum inventory, recovered from
/// stock's readers, writers and erases, not proof that anything else is
/// free.
pub mod stock {
    /// Flash bytes `start..end`, whole 4 KiB sectors.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Span {
        pub start: u32,
        pub end: u32,
    }

    const fn at(start: u32, end: u32) -> Span {
        Span { start, end }
    }

    /// The flash chip's size: 8 MiB.
    pub const FLASH_BYTES: u32 = 0x80_0000;
    /// The two application banks the factory loader chooses between, the
    /// same loader and bank table as the PLAF103's.
    pub const BANK0: u32 = 0x00_6000;
    pub const BANK1: u32 = 0x18_6000;
    /// Stock's bank-0 image rounded up to a sector, from its start: a
    /// reference bound for reviewing a first image, not a bank's capacity.
    pub const IMAGE_FOOTPRINT: u32 = 0xe_b000;

    /// The factory weight calibration: the zero and the coefficient, a state
    /// byte and a little-endian `u32` each. Read-only to a replacement.
    pub const WEIGHT_ZERO: u32 = 0x7c_e000;
    pub const WEIGHT_SCALE: u32 = 0x7c_f000;

    /// Everything stock keeps, in flash order: its banks at their reference
    /// size and every record its readers, writers and erases reach. The
    /// clock writer erases the whole 64 KiB block, so that block is kept
    /// whole.
    pub const PRESERVED: [Span; 11] = [
        // The loaders, the bank table and the system data.
        at(0x00_0000, BANK0),
        at(BANK0, BANK0 + IMAGE_FOOTPRINT),
        // The flash translation layer's three pages.
        at(0x18_2000, 0x18_5000),
        at(BANK1, BANK1 + IMAGE_FOOTPRINT),
        // The sector cleared on a Wi-Fi reset, the Wi-Fi settings and the
        // attributes' two sectors.
        at(0x3e_8000, 0x3e_c000),
        // The identity and its backup.
        at(0x3e_d000, 0x3e_f000),
        // The block holding the clock checkpoint.
        at(0x3f_0000, 0x40_0000),
        // The binding counter.
        at(0x40_0000, 0x40_1000),
        // The device log list, the system information and the report log.
        at(0x7c_b000, 0x7c_e000),
        at(WEIGHT_ZERO, WEIGHT_ZERO + 0x1000),
        at(WEIGHT_SCALE, WEIGHT_SCALE + 0x1000),
    ];

    /// Whole sectors, inside the chip, in order and disjoint.
    const fn sound() -> bool {
        let mut i = 0;
        while i < PRESERVED.len() {
            let span = PRESERVED[i];
            let ordered = i == 0 || PRESERVED[i - 1].end <= span.start;
            if !ordered
                || !span.start.is_multiple_of(0x1000)
                || !span.end.is_multiple_of(0x1000)
                || span.start >= span.end
                || span.end > FLASH_BYTES
            {
                return false;
            }
            i += 1;
        }
        true
    }
    const _: () = assert!(sound());
}
