//! What a wait on the feeder's device API status reports while it is not
//! ready. The replacement firmware speaks no MQTT and `wellfed-stock` relays
//! nothing, so this is all the bench asks of the feeder's state between jobs.

use wellfed_model::WifiSource;

/// Why the feeder's device-API status does not show what a wait is for, yet.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NotReady {
    /// The first 12 digits of the build the device API reports.
    #[error("the feeder reports build {0}")]
    Running(String),
    #[error("the feeder runs from bank {0}")]
    OnBank(u8),
    #[error("the feeder's clock is not set")]
    ClockUnset,
    #[error("the firmware clock is {0} ms behind this host")]
    Skew(i64),
    #[error("Wi-Fi source {0:?}")]
    Source(WifiSource),
}
