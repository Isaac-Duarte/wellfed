//! Power inputs for the device API's status: the battery pack on ADC channel 5 and the
//! adapter on PA19, sampled every `SAMPLE_MS` by the runtime's power task,
//! or, when a feeding owner holds the ADC, by the owner's own captures.
//!
//! The status reports a sample only while it is fresh (`STALE_MS`), so a stalled
//! task, a failed conversion or an ADC the monitor never opened all read as
//! unknown rather than as the last value. The pack voltage is the pin's
//! stock-scaled millivolts through the board divider, measured at 2.058 and
//! 2.068 against a meter on three D cells (physical, October 23 2026).
//!
//! [`supply`] reports the same sample judged, with its uncertainty named
//! (unread, stale, implausible). The pack's floor a feeding owner holds a
//! feed to is the feeder's (`wellfed_hal_feeder::power`,
//! [power](../../../../docs/firmware/power.md)).

use core::sync::atomic::{AtomicU32, Ordering};

use wellfed_core::adc_scale::{AdcCalibration, SAMPLES};
use wellfed_core::supply::{self, Curve, PackLimits};

use crate::{adc, sdk};

pub const SAMPLE_MS: u32 = 5_000;
/// Three missed samples.
pub const STALE_MS: u32 = 3 * SAMPLE_MS;
/// Pack millivolts per thousand pin millivolts.
pub const DIVIDER_PER_MILLE: u32 = 2_063;
/// Below this at the pin no pack is fitted: with none, the channel read
/// 16 mV, and three D cells read 2306–2357 mV.
pub const NO_PACK_PIN_MV: u32 = 500;

/// What a reading of this board's pack can be. Three D cells read 4.86 V
/// with the adapter and 4.77 V without (the meter, October 23); alkaline
/// cells start near 1.65 V each, so above 5.5 V the reading is wrong. On
/// the pack alone the board is running, so it reads at least what the
/// 3.3 V rail needs with any regulator; 2.5 V is well under that.
/// The charge is stock's curve on the same scaled unit: 1650 (pack about
/// 3.40 V, below which stock stops its outputs on battery, and the
/// feeder's floor for feeding) is 0 % and 2250 (about 4.64 V) 100 %, in
/// 10 % steps of 60. Fresh cells read about 2306 at rest, so 100 %.
pub const PACK: PackLimits = PackLimits {
    divider_per_mille: DIVIDER_PER_MILLE,
    absent_pin_mv: NO_PACK_PIN_MV,
    max_pack_mv: 5_500,
    min_running_mv: 2_500,
    stale_ms: STALE_MS + 1,
    curve: Curve {
        empty_pin_mv: 1650,
        full_pin_mv: 2250,
    },
};

const BATTERY_CHANNEL: u32 = crate::board::pins::ADC_BATTERY;
const ADAPTER_PIN: u32 = crate::board::pins::ADAPTER;
/// Polls per conversion; a conversion takes well under 1 ms.
const ADC_POLLS: u32 = 10_000;

/// A fresh power sample, as the device API's status reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PowerHealth {
    /// PA19: the adapter is connected.
    pub adapter: bool,
    /// The pack voltage, `None` when no pack is fitted.
    pub battery_mv: Option<u32>,
    /// Milliseconds since the sample was taken.
    pub age_ms: u32,
}

/// Bit 31: a sample is held; bit 30: the adapter; bits 0..16: pin millivolts.
static SAMPLE: AtomicU32 = AtomicU32::new(0);
static SAMPLED_AT_MS: AtomicU32 = AtomicU32::new(0);
const HELD: u32 = 1 << 31;
const ADAPTER: u32 = 1 << 30;
const MV_MASK: u32 = 0xffff;

/// What a monitor step reports on the UART: the open, the first sample
/// and each adapter change, or the failure that stopped it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Record {
    Opened { ok: bool },
    Sample { pin_mv: u32, adapter: bool },
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    /// Zero once the monitor has given up.
    pub delay_ms: u32,
    pub record: Option<Record>,
}

/// `PLBAT1 …` body.
pub fn describe(record: Record, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
    match record {
        Record::Opened { ok } => write!(f, "open ok={}", u8::from(ok)),
        Record::Sample { pin_mv, adapter } => {
            write!(f, "sample pin_mv={pin_mv} adapter={}", u8::from(adapter))
        }
        Record::Failed => f.write_str("error adc"),
    }
}

/// The power task's state; the runtime keeps one and steps it.
#[derive(Debug, Default)]
pub struct Monitor {
    calibration: Option<AdcCalibration>,
    /// The adapter level last reported, `None` before the first sample.
    reported: Option<bool>,
    /// The ADC failed to open or convert; the monitor stops for this boot.
    failed: bool,
}

impl Monitor {
    /// Opens the ADC on the first call, then samples.
    pub fn step(&mut self) -> Step {
        if self.failed {
            return Step {
                delay_ms: 0,
                record: None,
            };
        }

        let Some(calibration) = self.calibration else {
            // SAFETY: the adapter input, which only ever reads; nothing drives it.
            unsafe { sdk::gpio_init(ADAPTER_PIN, false) };
            let calibration = adc::open().calibration;
            self.calibration = calibration;
            if calibration.is_none() {
                return self.fail(Record::Opened { ok: false });
            }
            return Step {
                delay_ms: 1,
                record: Some(Record::Opened { ok: true }),
            };
        };

        let mut samples = [0u16; SAMPLES];
        let begin = sdk::uptime_ms();
        if !adc::convert(BATTERY_CHANNEL, ADC_POLLS, &mut samples) {
            return self.fail(Record::Failed);
        }
        let Some(pin_mv) = calibration.scale_16(&samples) else {
            return self.fail(Record::Failed);
        };

        let adapter = sdk::gpio_read_bit(ADAPTER_PIN);
        publish(pin_mv, adapter, begin);
        let changed = self.reported != Some(adapter);
        self.reported = Some(adapter);
        Step {
            delay_ms: SAMPLE_MS,
            record: changed.then_some(Record::Sample { pin_mv, adapter }),
        }
    }

    fn fail(&mut self, record: Record) -> Step {
        self.failed = true;
        SAMPLE.store(0, Ordering::Release);
        adc::close();
        Step {
            delay_ms: 0,
            record: Some(record),
        }
    }
}

/// The time is the start of the conversion, so its duration counts toward
/// the age. The sample is withdrawn while its time changes, so a reader
/// never pairs one sample's value with another's time.
pub fn publish(pin_mv: u32, adapter: bool, at_ms: u32) {
    let packed = HELD | if adapter { ADAPTER } else { 0 } | pin_mv.min(MV_MASK);
    SAMPLE.store(0, Ordering::Release);
    SAMPLED_AT_MS.store(at_ms, Ordering::Release);
    SAMPLE.store(packed, Ordering::Release);
}

/// The latest sample, if fresh at `now_ms`. A publish between the two reads
/// of the value reads as unknown for that one health.
pub fn health(now_ms: u32) -> Option<PowerHealth> {
    let packed = SAMPLE.load(Ordering::Acquire);
    let at = SAMPLED_AT_MS.load(Ordering::Acquire);
    if SAMPLE.load(Ordering::Acquire) != packed || packed & HELD == 0 {
        return None;
    }

    let age_ms = now_ms.wrapping_sub(at);
    if age_ms > STALE_MS {
        return None;
    }
    let pin_mv = packed & MV_MASK;
    Some(PowerHealth {
        adapter: packed & ADAPTER != 0,
        battery_mv: pack_mv(pin_mv),
        age_ms,
    })
}

/// The pack's millivolts from the pin's, `None` when no pack is fitted:
/// the conversion [`supply`] judges, so a trace's battery entries and the
/// supply never disagree on the pack.
pub fn pack_mv(pin_mv: u32) -> Option<u32> {
    PACK.pack_mv(pin_mv.min(MV_MASK))
}

/// The latest sample, however old: `None` before the first, or after the
/// monitor stopped on a failed ADC.
pub fn latest(now_ms: u32) -> Option<supply::Sample> {
    let packed = SAMPLE.load(Ordering::Acquire);
    let at = SAMPLED_AT_MS.load(Ordering::Acquire);
    if SAMPLE.load(Ordering::Acquire) != packed || packed & HELD == 0 {
        return None;
    }
    Some(supply::Sample {
        pin_mv: packed & MV_MASK,
        adapter: packed & ADAPTER != 0,
        age_ms: now_ms.wrapping_sub(at),
    })
}

/// The supply at `now_ms`, judged against this board's pack.
pub fn supply(now_ms: u32) -> supply::Reading {
    supply::classify(latest(now_ms), &PACK)
}

/// Tests only: forget the published sample.
#[doc(hidden)]
pub fn reset() {
    SAMPLE.store(0, Ordering::Release);
    SAMPLED_AT_MS.store(0, Ordering::Release);
}
