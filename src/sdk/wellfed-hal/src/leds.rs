//! Stock-derived indicator GPIOs. Colors and physical polarity are bench results.

use crate::sdk::{self, IrqGuard};
use crate::stopped;

/// Each port's indicator bits, from the board's pins.
pub const PA_MASK: u32 = port_mask(0);
pub const PB_MASK: u32 = port_mask(1);

const fn port_mask(port: u32) -> u32 {
    let mut mask = 0;
    let mut index = 0;
    while index < Led::ALL.len() {
        let pin = Led::ALL[index].pin();
        if pin >> 5 == port {
            mask |= 1 << (pin & 31);
        }
        index += 1;
    }
    mask
}

/// What an indicator's pad must not show while it drives the light: a
/// function other than GPIO, a pull, or the pad shut down.
const PAD_NOT_INDICATOR: u32 =
    sdk::PAD_FUNCTION_MASK | sdk::PAD_PULL_UP | sdk::PAD_PULL_DOWN | sdk::PAD_SHUTDOWN;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Led {
    Connection,
    Food,
    Lock,
    Fault,
}

impl Led {
    pub const ALL: [Self; 4] = [Self::Connection, Self::Food, Self::Lock, Self::Fault];

    pub const fn pin(self) -> u32 {
        match self {
            Self::Connection => crate::board::pins::LED_CONNECTION,
            Self::Food => crate::board::pins::LED_FOOD,
            Self::Lock => crate::board::pins::LED_LOCK,
            Self::Fault => crate::board::pins::LED_FAULT,
        }
    }

    /// Its pin's name, as the logs and prompts print it (`PA0`, `PB22`).
    pub const fn name(self) -> PinName {
        PinName(self.pin())
    }

    /// PA0-PA7 read LOW at KM0 startup before any supply step and while lit, so
    /// these indicators are output-only on the tested unit. Only PB22 has input readback.
    pub const fn input_readable(self) -> bool {
        matches!(self, Self::Fault)
    }
}

/// A pin's name: its port's letter and its bit (`PA0`, `PB22`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PinName(u32);

impl core::fmt::Display for PinName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let port = if self.0 >> 5 == 0 { 'A' } else { 'B' };
        write!(f, "P{port}{}", self.0 & 31)
    }
}

fn directions() -> bool {
    sdk::gpio_direction(0) & PA_MASK == PA_MASK && sdk::gpio_direction(1) & PB_MASK == PB_MASK
}

/// LOW is the stock software's disabled-indicator request, pending polarity check.
#[must_use]
pub fn all_low() -> bool {
    let _guard = IrqGuard::lock();
    // SAFETY: the indicator pins only; this module owns them.
    unsafe {
        sdk::gpio_port_write(0, PA_MASK, 0);
        sdk::gpio_port_write(1, PB_MASK, 0);
    }
    directions()
        && sdk::gpio_output_latch(0) & PA_MASK == 0
        && sdk::gpio_output_latch(1) & PB_MASK == 0
        && sdk::gpio_port_read(0, PA_MASK) == 0
        && sdk::gpio_port_read(1, PB_MASK) == 0
}

pub fn initialize() -> bool {
    let _guard = IrqGuard::lock();
    if !stopped::observe().verified() {
        return false;
    }

    let _ = all_low(); // Preload before output direction; verify after enabling.
    for led in Led::ALL {
        sdk::gpio_no_pull(led.pin());
        // SAFETY: an indicator pin; this module owns them.
        unsafe { sdk::gpio_init(led.pin(), true) };
    }
    directions() && all_low() && Led::ALL.into_iter().all(|led| observe(led).accepted(false))
}

/// Request an indicator level. The caller must wait and verify with `observe`.
/// Only indicator bits are writable; stopped-output drift refuses the request.
pub fn request(led: Led, high: bool) -> bool {
    write(led, high, Stopped::Required)
}

// `show` writes these bits while the motor may run and the emitters may be
// lit: the masks must never reach a pin the stopped-output owners hold.
const _: () = assert!(
    PA_MASK & crate::stopped::PA_MASK == 0 && PB_MASK & crate::stopped::PB_MASK == 0,
    "an indicator mask overlaps a motor or emitter pin"
);

/// Drive one indicator for the panel's indicator service, whatever the
/// motor and emitters are doing: a feed must not freeze or fail the lights,
/// and the lights must not hold up a feed. The write is one masked port
/// write under the interrupt lock, so it cannot interleave with the
/// supervision timer's motor writes, and its mask holds only indicator
/// bits (checked at compile time above). An indicator pin that is no
/// longer an output refuses the request and turns every indicator off.
/// The caller waits [`SETTLE_MS`] and checks with [`shown`].
pub fn show(led: Led, high: bool) -> bool {
    write(led, high, Stopped::Ignored)
}

/// Whether a write or a readback looks at the stopped outputs: the probe's
/// do, the indicator service's (during feeds) do not.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stopped {
    Required,
    Ignored,
}

/// One masked write of `led`'s bit under the interrupt lock, refused (with
/// every indicator turned off) when an indicator is no longer an output or,
/// if `outputs` requires it, the stopped outputs drifted.
fn write(led: Led, high: bool, outputs: Stopped) -> bool {
    let _guard = IrqGuard::lock();
    if !directions() || (outputs == Stopped::Required && !stopped::observe().verified()) {
        let _ = all_low();
        return false;
    }

    let pin = led.pin();
    let mask = 1 << (pin & 31);
    // SAFETY: one indicator pin; this module owns them.
    unsafe { sdk::gpio_port_write(pin >> 5, mask, if high { mask } else { 0 }) };
    true
}

#[derive(Clone, Copy, Debug)]
pub struct Readback {
    pub latch_high: bool,
    pub input_high: bool,
    /// Extra input samples that read HIGH; informational, never used to verify.
    pub input_highs: u32,
    pub input_readable: bool,
    pub directions_ok: bool,
    pub stopped_ok: bool,
    pub pad: u32,
}

/// Whether a settled request is verified, output-only, remains a bounded input
/// experiment, or must stop immediately. An input mismatch is never a successful
/// step, and an output-only step is never a verified electrical level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Verified,
    /// Latch, pad, direction and stopped state agree; the pad has no input readback.
    OutputOnly,
    InputMismatch,
    Abort,
}

impl Readback {
    pub fn decision(self, high: bool) -> Decision {
        if self.latch_high != high
            || !self.directions_ok
            || !self.stopped_ok
            || self.pad & PAD_NOT_INDICATOR != 0
        {
            return Decision::Abort;
        }
        if !self.input_readable {
            Decision::OutputOnly
        } else if self.input_high == high {
            Decision::Verified
        } else if high {
            // Hold only the scheduled HIGH interval, then require LOW readback.
            Decision::InputMismatch
        } else {
            Decision::Abort
        }
    }

    /// True for an input-verified level or a consistent output-only request.
    pub fn accepted(self, high: bool) -> bool {
        matches!(
            self.decision(high),
            Decision::Verified | Decision::OutputOnly
        )
    }
}

pub fn observe(led: Led) -> Readback {
    readback(led, Stopped::Required)
}

/// Check a level [`show`] wrote: latch, pad, direction and (PB22) one input
/// read, as [`Readback::decision`] judges them, but not the stopped outputs,
/// which a feed legitimately leaves. It runs during feeds, up to once per
/// blink edge per light, so it holds the interrupt lock only for what the
/// decision needs: five register reads (two directions, the latch, one
/// input sample, the pad), against the probe's [`observe`] with its 64
/// extra input samples and the stopped-output snapshot.
pub fn shown(led: Led, high: bool) -> Decision {
    readback(led, Stopped::Ignored).decision(high)
}

/// `led`'s readback under the interrupt lock. The probe's (stopped outputs
/// `Required`) adds [`INPUT_SAMPLES`] input reads and the stopped-output
/// snapshot; the indicator service's takes neither (see [`show`]).
fn readback(led: Led, outputs: Stopped) -> Readback {
    let probe = outputs == Stopped::Required;
    let _guard = IrqGuard::lock();
    let pin = led.pin();
    Readback {
        latch_high: sdk::gpio_output_latch(pin >> 5) & (1 << (pin & 31)) != 0,
        input_high: sdk::gpio_read_bit(pin),
        input_highs: if probe {
            (0..INPUT_SAMPLES).fold(0, |highs, _| highs + u32::from(sdk::gpio_read_bit(pin)))
        } else {
            0
        },
        input_readable: led.input_readable(),
        directions_ok: directions(),
        stopped_ok: !probe || stopped::observe().verified(),
        pad: sdk::pad_control(pin),
    }
}

pub const SETTLE_MS: u32 = 10;
/// Back-to-back input reads per observation, to separate a dead path from a marginal one.
pub const INPUT_SAMPLES: u32 = 64;
