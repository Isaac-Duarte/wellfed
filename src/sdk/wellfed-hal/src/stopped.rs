//! The outputs' stopped state, the board's safe state (`board::safe`) read
//! back: requested at startup before any driver opens, and checked by every
//! caller that needs the outputs still (the LEDs, the buzzer, the flash
//! leases, the updater and the boot confirmation). Readback describes GPIO
//! logic; physical polarity and transients need measurement.
//!
//! - **PLAF103:** the stock-derived motor STOP and infrared OFF, without
//!   opening the feeder's board. Infrared OFF requires both a LOW latch and a
//!   LOW pad input: a dead input path reads LOW whatever is driven, so input
//!   alone cannot prove OFF. PB3 is also SWD_CLK: until SWD is released (as
//!   stock does before it configures PB3) the pad ignores the latch and
//!   reads its pull-down, so the release is part of the verified state.
//! - **PLWF106:** the pump off, PA30 latched LOW before its output is
//!   enabled, as stock does. Nothing else is configured here: the
//!   indicators, the button and the scale belong to the fountain's drivers.

use wellfed_core::flash_coord::FlashCoord;

use crate::board::safe::{PORT_A_HIGH, PORT_A_LOW, PORT_B_LOW};
use crate::sdk::{self, IrqGuard};

#[cfg(wellfed_board = "plaf103")]
pub use feeder::*;
#[cfg(wellfed_board = "plwf106")]
pub use fountain::*;

/// Every output the board's safe state covers, per port: the stopped check
/// reads exactly the pins a fault or a flash lease writes safe.
pub const PA_MASK: u32 = PORT_A_HIGH | PORT_A_LOW;
pub const PB_MASK: u32 = PORT_B_LOW;

#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub clock: sdk::LpClock,
    pub pa: u32,
    pub pb: u32,
    pub dira: u32,
    pub dirb: u32,
    /// Output latches of the pins the safe state drives LOW: the infrared
    /// emitters (PA15, PB3) on the PLAF103, the pump (PA30) on the PLWF106.
    pub latch_a: u32,
    pub latch_b: u32,
    /// LP `REG_SWD_PMUX_EN`; `SWD_PMUX_BIT` must be clear where the safe
    /// state drives PB3, SWD_CLK.
    pub swd: u32,
}

impl Snapshot {
    pub fn verified(self) -> bool {
        self.clock.verified()
            && self.pa == PORT_A_HIGH
            && self.pb == 0
            && self.dira == PA_MASK
            && self.dirb == PB_MASK
            && self.latch_a == 0
            && self.latch_b == 0
            && (!SWD_HELD_PB3 || self.swd & sdk::SWD_PMUX_BIT == 0)
    }
}

/// PB3, SWD_CLK, is among the safe state's pins, so SWD's release is part of
/// it: while SWD holds the pad, the latch never reaches it.
const SWD_HELD_PB3: bool = PB_MASK & 1 << 3 != 0;

pub fn observe() -> Snapshot {
    let _guard = IrqGuard::lock();
    Snapshot {
        clock: sdk::gpio_clock_state(),
        pa: sdk::gpio_port_read(0, PA_MASK) & PA_MASK,
        pb: sdk::gpio_port_read(1, PB_MASK) & PB_MASK,
        dira: sdk::gpio_direction(0) & PA_MASK,
        dirb: sdk::gpio_direction(1) & PB_MASK,
        latch_a: sdk::gpio_output_latch(0) & PORT_A_LOW,
        latch_b: sdk::gpio_output_latch(1) & PB_MASK,
        swd: sdk::swd_pinmux(),
    }
}

/// Whether `flash`'s lease blocks the outputs now. A null or not-yet-attached
/// authority blocks every output: callers pass an attached authority, which
/// the board's `attach_flash` requires to stay live.
#[expect(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "the authority is null or an attached one, which the board keeps live at a fixed address: the same contract as the board's own callers'"
)]
pub fn outputs_blocked(flash: *const FlashCoord) -> bool {
    // SAFETY: the attached authority stays at a fixed address while in use.
    unsafe { flash.as_ref() }.is_none_or(FlashCoord::outputs_blocked)
}

/// The feeder's motor pair and emitters.
#[cfg(wellfed_board = "plaf103")]
mod feeder {
    use core::sync::atomic::{AtomicU32, Ordering};

    use super::{PA_MASK, PB_MASK, Snapshot, observe};
    use crate::board::pins::{IR_PA_BIT, MOTOR_MASK};
    use crate::sdk::{self, IrqGuard};

    /// Emitters held on at rest under an armed supervisor budget: bit 0 PA15,
    /// bit 1 PB3; read by health. Written by the optics probe and by the feeding
    /// owner, whose `quiet` clears it. They never share a firmware: only `feeder`
    /// opens an owner, and it does not link `wellfed-probes`, which only `qa` and
    /// `bringup` do.
    static SUPERVISED: AtomicU32 = AtomicU32::new(0);

    /// Both emitters, as [`set_supervised`] takes them.
    pub const BOTH_EMITTERS: u32 = 3;

    /// Publish the emitters now on under supervision (0 before any emitter change).
    pub fn set_supervised(emitters: u32) {
        SUPERVISED.store(emitters & 3, Ordering::Release);
    }

    impl Snapshot {
        /// Motor STOP and every output direction, emitters aside.
        pub fn motor_verified(self) -> bool {
            self.clock.verified()
                && self.pa & MOTOR_MASK == MOTOR_MASK
                && self.dira == PA_MASK
                && self.dirb == PB_MASK
                && self.swd & sdk::SWD_PMUX_BIT == 0
        }

        /// Motor STOP, and the emitters' latches and pads exactly those published
        /// as supervised. Never true with none published.
        pub fn sensing(self) -> bool {
            let emitters = SUPERVISED.load(Ordering::Acquire);
            let a = if emitters & 1 != 0 { IR_PA_BIT } else { 0 };
            let b = if emitters & 2 != 0 { PB_MASK } else { 0 };
            emitters != 0
                && self.motor_verified()
                && self.pa & IR_PA_BIT == a
                && self.latch_a == a
                && self.pb == b
                && self.latch_b == b
        }
    }

    /// Enable both motor pins as outputs at STOP, without the driver ever seeing
    /// its inputs differ, whatever its undriven inputs read.
    ///
    /// Neither undriven level is safe to assume. `gpio_init` enables one pin per
    /// call, so whichever level the first pin drives, the driver sees it against
    /// the other's undriven level for one call: about 105 µs of drive, measured
    /// at the motor with the latches preloaded HIGH against inputs at their
    /// pull-down's LOW, and on the inputs with the latches preloaded LOW against
    /// inputs that read HIGH (with the analyzer's hooks on them). So both latches go HIGH
    /// first, which the data register holds while the pins are still inputs, and
    /// one direction write then makes both outputs at once: the driver goes from
    /// whatever it saw straight to brake. When both are already outputs (the
    /// board opening after stopped startup), the latch write is the whole change,
    /// one port write, so the motor never passes through coast.
    pub fn enable_motor_stopped() {
        // SAFETY: the motor pins, which this module and the board own: called at
        // startup and as the board opens, before its gate can drive them.
        unsafe {
            sdk::gpio_port_write(0, MOTOR_MASK, MOTOR_MASK);
            sdk::gpio_enable_outputs(0, MOTOR_MASK);
        }
    }

    /// Preload the infrared latches LOW before enabling their outputs, and bring
    /// the motor pair to STOP with both pins enabled together.
    pub fn initialize() -> Snapshot {
        let _guard = IrqGuard::lock();
        let clock = sdk::gpio_clock_enable();
        if !clock.verified() {
            return Snapshot { clock, ..observe() };
        }

        sdk::swd_off();
        // SAFETY: the emitter latches, which this module owns until the board opens.
        unsafe {
            sdk::gpio_port_write(0, IR_PA_BIT, 0);
            sdk::gpio_port_write(1, PB_MASK, 0);
        }
        enable_motor_stopped();
        use crate::board::pins::{IR_PA, IR_PB};
        for pin in [IR_PA, IR_PB] {
            // SAFETY: as above, with each latch already LOW.
            unsafe { sdk::gpio_init(pin, true) };
        }
        Snapshot { clock, ..observe() }
    }
}

/// The fountain's pump.
#[cfg(wellfed_board = "plwf106")]
mod fountain {
    use super::{PA_MASK, Snapshot, observe};
    use crate::board::pins::PUMP_BIT;
    use crate::sdk::{self, IrqGuard};

    // The pump is the safe state's only pin.
    const _: () = assert!(PA_MASK == PUMP_BIT && super::PB_MASK == 0);

    /// Latch the pump enable LOW, then make it an output, as stock does, so
    /// the pin never drives HIGH on the way: the pump is off from the first
    /// edge. Already an output, the latch write is the whole change.
    pub fn initialize() -> Snapshot {
        let _guard = IrqGuard::lock();
        let clock = sdk::gpio_clock_enable();
        if !clock.verified() {
            return Snapshot { clock, ..observe() };
        }

        // SAFETY: the pump enable, which this module owns until the
        // fountain's pump driver opens.
        unsafe {
            sdk::gpio_port_write(0, PUMP_BIT, 0);
            sdk::gpio_enable_outputs(0, PUMP_BIT);
        }
        Snapshot { clock, ..observe() }
    }
}
