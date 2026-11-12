//! What the server is started with: its key, label, timing and what it
//! keeps.

use std::num::NonZeroU32;
use std::time::Duration;

use crate::{AlertReads, Backups, Diagnostics, Discovery, ReleasesConfig, WebhookTarget};

pub struct Config {
    /// Seals the server's feeder keys at rest.
    pub key: [u8; 32],
    /// The host's IANA zone, offered at setup as the household's.
    pub host_zone: Option<String>,
    /// The label the server enrolls under on each feeder (1-16 bytes).
    pub label: String,
    pub discovery: Discovery,
    pub timing: Timing,
    /// Whether each feeder's feed traces and statistics are read.
    pub diagnostics: Diagnostics,
    /// Whether each feeder's alert table is read and kept.
    pub alerts: AlertReads,
    /// The release cache, the keys a release may be signed with, and
    /// whether releases are fetched.
    pub releases: ReleasesConfig,
    /// How many days raw samples are kept; forever when `None`. Hourly
    /// rollups are always kept.
    pub raw_sample_days: Option<NonZeroU32>,
    pub backups: Backups,
    /// Where each alert's opening and clearing is POSTed; none when
    /// `None`.
    pub webhook: Option<WebhookTarget>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("host_zone", &self.host_zone)
            .field("label", &self.label)
            .field("discovery", &self.discovery)
            .field("timing", &self.timing)
            .field("diagnostics", &self.diagnostics)
            .field("alerts", &self.alerts)
            .field("releases", &self.releases)
            .field("raw_sample_days", &self.raw_sample_days)
            .field("backups", &self.backups)
            .field("webhook", &self.webhook)
            .finish_non_exhaustive()
    }
}

impl Config {
    /// What the mirror needs of it: no key, no label, no logins.
    pub(crate) fn mirror(&self) -> wellfed_mirror::Config {
        let timing = self.timing;
        let mut mirror = wellfed_mirror::Timing {
            retry_min: timing.retry_min,
            retry_max: timing.retry_max,
            time_sync: timing.time_sync,
            queue: timing.queue,
            sample_wait: timing.sample_wait,
            pace: timing.pace,
            join_settle: timing.join_settle,
            trial: timing.trial,
            // The rest of the feeder's own session clocks; nothing the
            // owner sets.
            ..wellfed_mirror::Timing::default()
        };
        mirror.session.answer = timing.answer;
        wellfed_mirror::Config {
            discovery: self.discovery,
            timing: mirror,
            diagnostics: self.diagnostics,
            alerts: self.alerts,
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
    /// How long a command waits for its turn on a feeder's session before
    /// it is turned away unsent, and how long a step that needs the session
    /// waits for one coming up.
    pub queue: Duration,
    /// How long the feeder has to answer a request sent to it, after which
    /// whether it ran is unknown.
    pub answer: Duration,
    /// The longest a received sample waits to be written with others.
    pub sample_wait: Duration,
    /// How long a login lasts after its last use.
    pub session: Duration,
    /// The longest an import's upload may go without a byte, or an
    /// export's download without taking one: the import holds the one
    /// writer meanwhile, and the export the one export slot and a read
    /// snapshot that keeps the database's log from being checkpointed.
    pub transfer_idle: Duration,
    /// How an install or a stock restore waits between its steps.
    pub pace: wellfed_mirror::Pace,
    /// How long an adoption whose join answer was lost looks for its key
    /// enrolled, as a running feed holds the enrollment.
    pub join_settle: Duration,
    /// How long after an activation the server follows the new build's
    /// restart and trial, up to its verdict.
    pub trial: Duration,
    /// How long after it starts a server with no account takes a setup or
    /// an import: whoever reaches it first could claim it, so the window
    /// is short, and a restart opens it again.
    pub setup_window: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        let mirror = wellfed_mirror::Timing::default();
        Self {
            retry_min: mirror.retry_min,
            retry_max: mirror.retry_max,
            time_sync: mirror.time_sync,
            queue: mirror.queue,
            answer: mirror.session.answer,
            sample_wait: mirror.sample_wait,
            session: Duration::from_secs(30 * 24 * 3600),
            transfer_idle: Duration::from_secs(30),
            pace: mirror.pace,
            join_settle: mirror.join_settle,
            trial: mirror.trial,
            setup_window: Duration::from_secs(3600),
        }
    }
}
