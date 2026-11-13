//! Whether the server's `NTP` reply sets stock's clock (its
//! `calibrationTag`).
//!
//! Stock 1.1.15 refused an install with error 1214 while its clock stood two
//! days behind (physical; that the clock is the cause is likely, not confirmed
//! statically), so the reply sets it when no meal can come due in the jump:
//! after the Reset hold and setup frame, which erased stock's plans. What
//! stock does with meals that came due while its clock jumps forward is
//! unverified, so otherwise the clock stays where it reads: a stock that may
//! still hold its own plans because it reached this broker some other way.
//!
//! Stock's `NTP` request carries its own clock in `ts`, milliseconds since the
//! epoch, rounded to the second (physical, UART captures of 1.1.15). It is
//! logged, so a bench run records what the clock read.

use std::fmt;

use chrono::{DateTime, Utc};
use tracing::{info, warn};

/// What is known of the plans stock holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StockPlans {
    /// This run's setup frame followed the Reset hold, which erased them.
    Erased,
    /// Stock may still hold them.
    Unknown,
}

/// The reply's decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Calibration {
    Sent,
    /// Stock's own plans may run in the jump.
    HeldUnknown,
}

impl Calibration {
    /// The reply's `calibrationTag`.
    pub fn tag(self) -> bool {
        self == Self::Sent
    }
}

impl fmt::Display for Calibration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Sent => "sent",
            Self::HeldUnknown => "held: no setup frame, so stock may still hold plans",
        })
    }
}

/// Whether to set stock's clock, given what is known of its plans.
pub fn decide(stock: StockPlans) -> Calibration {
    match stock {
        StockPlans::Erased => Calibration::Sent,
        StockPlans::Unknown => Calibration::HeldUnknown,
    }
}

/// The decision for an `NTP` request dated `ts` and answered `now`, logged
/// so a bench run records what stock's clock read.
pub fn calibration(stock: StockPlans, ts: Option<i64>, now: DateTime<Utc>) -> Calibration {
    let calibration = decide(stock);

    // `ts` is whatever the device sent: out of range, it reads as no skew.
    let skew = ts
        .and_then(|ts| ts.checked_sub(now.timestamp_millis()))
        .map(|ms| ms / 1000);
    info!(stock_ts = ?ts, skew_seconds = ?skew, %calibration, "NTP request");
    // 1214 is wellfed_client's `install::STOCK_CLOCK_UNSET`, the code the
    // installer turns into its own error; this crate does not depend on it.
    if calibration != Calibration::Sent {
        warn!(
            "stock's clock is left as it reads ({calibration}); stock may refuse an install (error 1214) until it is set"
        );
    }
    calibration
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clock_is_set_only_once_no_meal_can_come_due() {
        assert_eq!(decide(StockPlans::Erased), Calibration::Sent);
        assert_eq!(decide(StockPlans::Unknown), Calibration::HeldUnknown);
    }
}
