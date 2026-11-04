//! What the boot test does and says: latch the pump off before anything
//! else, then report the build, the bank and the pump's readback, and beat.
//!
//! Nothing here drives the pump HIGH, configures another pin, or reaches
//! the network, a store, the updater or the trial confirmation; the image
//! links none of them. A beat that finds the pump not reading stopped
//! latches it LOW again, which is the only write this image ever makes to
//! it, and says so.

use core::ffi::CStr;
use core::fmt;

use wellfed_hal::board::pins::PUMP_BIT;
use wellfed_hal::sdk;
use wellfed_hal::startup::{self, Report};
use wellfed_hal::stopped::{self, Snapshot};

/// The tag of every line this image prints.
pub const TAG: &str = "PLBOOT1";
/// The model this image is for, as `wellfed_proto::device::Model::name`
/// spells the board's code.
pub const MODEL: &str = "plwf106";
const _: () = assert!(wellfed_hal::board::MODEL_CODE == 2);
/// Between beats.
pub const BEAT_MS: u32 = 5_000;

/// The pump enable as the snapshot read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pump {
    /// Every check of the stopped state passed: clocked, latched LOW, an
    /// output, reading LOW.
    pub stopped: bool,
    pub latch_high: bool,
    pub pad_high: bool,
    pub output: bool,
}

impl Pump {
    pub fn of(snapshot: Snapshot) -> Self {
        Self {
            stopped: snapshot.verified(),
            latch_high: snapshot.latch_a & PUMP_BIT != 0,
            pad_high: snapshot.pa & PUMP_BIT != 0,
            output: snapshot.dira & PUMP_BIT != 0,
        }
    }
}

impl fmt::Display for Pump {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "pump={} latch={} pad={} output={}",
            if self.stopped { "off" } else { "unverified" },
            u8::from(self.latch_high),
            u8::from(self.pad_high),
            u8::from(self.output),
        )
    }
}

/// The bank the image runs from, as the boot's flash mapping says.
fn bank(snapshot: startup::Snapshot) -> &'static str {
    match snapshot.bank() {
        Some(0) => "0",
        Some(1) => "1",
        _ => "unknown",
    }
}

/// `PLBOOT1 start …`: the model, the bank and the pump.
pub fn start_line(
    report: &mut Report,
    boot: startup::Snapshot,
    pump: Pump,
) -> Result<&CStr, fmt::Error> {
    report.record(format_args!(
        "{TAG} start model={MODEL} bank={} {pump}\r\n",
        bank(boot)
    ))
}

/// `PLBOOT1 beat …`: the count, the uptime and the pump, and whether this
/// beat latched it LOW again.
pub fn beat_line(
    report: &mut Report,
    count: u32,
    uptime_ms: u32,
    pump: Pump,
    relatched: bool,
) -> Result<&CStr, fmt::Error> {
    report.record(format_args!(
        "{TAG} beat n={count} uptime_ms={uptime_ms} {pump} relatched={}\r\n",
        u8::from(relatched)
    ))
}

/// The start: the pump latched LOW and made an output first, then the
/// banner the builder checks for, the build and bank (from `boot`, read
/// only after the latch), and this image's own line. Returns whether the
/// pump reads stopped.
pub fn start(boot: impl FnOnce() -> startup::Snapshot) -> bool {
    let pump = Pump::of(stopped::initialize());

    sdk::log(c"\r\nWellfed local firmware: Rust bringup\r\n");
    let boot = boot();
    let mut report = Report::new();
    if let Ok(line) = report.start(boot) {
        sdk::log(line);
    }
    if let Ok(line) = start_line(&mut report, boot, pump) {
        sdk::log(line);
    }
    pump.stopped
}

/// One beat, the `count`th. A pump that no longer reads stopped is latched
/// LOW again before the line, so the line says what the pump is now.
pub fn beat(count: u32) -> Pump {
    let mut pump = Pump::of(stopped::observe());
    let relatched = !pump.stopped;
    if relatched {
        pump = Pump::of(stopped::initialize());
    }

    let mut report = Report::new();
    if let Ok(line) = beat_line(&mut report, count, sdk::uptime_ms(), pump, relatched) {
        sdk::log(line);
    }
    pump
}
