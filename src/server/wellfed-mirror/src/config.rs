//! The mirror's settings, from the service's config.

use std::time::Duration;

use wellfed_connect::update::Pace;

use crate::AlertReads;

/// Whether a feeder whose address fails is looked up over mDNS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Discovery {
    Mdns,
    Off,
}

#[derive(Debug)]
pub struct Config {
    pub discovery: Discovery,
    pub timing: Timing,
    pub diagnostics: Diagnostics,
    pub alerts: AlertReads,
}

/// Whether each feeder's feed traces and statistics are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Diagnostics {
    /// Each feed's trace after its `Fed` event, and the statistics at each
    /// connect and then every `stats_every`.
    Read {
        stats_every: Duration,
    },
    Off,
}

impl Default for Diagnostics {
    fn default() -> Self {
        Self::Read {
            stats_every: Duration::from_secs(3600),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Timing {
    /// The first reconnect delay, doubled after each failure.
    pub retry_min: Duration,
    pub retry_max: Duration,
    /// How often a connected feeder's clock is set again.
    pub time_sync: Duration,
    /// How long a command waits for its turn on the session before it is
    /// turned away unsent: room to queue behind a step or another command,
    /// each answered within the session's own window. Once sent, it waits
    /// for that window.
    pub queue: Duration,
    /// The longest a sample waits in its batch before the batch is
    /// written.
    pub sample_wait: Duration,
    /// Each session's own clocks: how long the feeder has to answer a
    /// request, and the keepalive and silence limits.
    pub session: wellfed_connect::Timing,
    /// How an install waits between its steps: while bank 0 writes a
    /// sector, and after bank 1 gives way.
    pub pace: Pace,
    /// How long an adoption whose join answer was lost looks for its key
    /// enrolled. The feeder holds a join's enrollment, a key-store write,
    /// until a running feed ends, and makes it whether or not the
    /// connection that asked is still there.
    pub join_settle: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            retry_min: Duration::from_secs(1),
            retry_max: Duration::from_secs(60),
            time_sync: Duration::from_secs(6 * 3600),
            queue: wellfed_connect::ANSWER + Duration::from_secs(5),
            sample_wait: Duration::from_secs(10),
            session: wellfed_connect::Timing::DEFAULT,
            pace: Pace::DEFAULT,
            // The longest feed (48 portions at up to 12 s each), and margin.
            join_settle: wellfed_api::LOST_JOIN_SEARCH,
        }
    }
}
