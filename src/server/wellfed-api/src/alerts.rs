//! A feeder's open alerts, as the server last read them from its alert
//! table ([alerts]). The feeder decides every alert; the server only reads
//! the table over its own session, keeps the latest, and judges one thing
//! itself, that it can't reach the feeder.
//!
//! A client away from home reads this in place of the feeder's own table,
//! under the same cursor, so what it tracks hands over to its own reads at
//! home with no alert shown twice.
//!
//! [alerts]: ../../../../docs/firmware/alerts.md

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use wellfed_model::{Generation, OpenAlert};

/// `GET /feeders/{id}/alerts`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FeederAlerts {
    /// The read the alerts come from; `None` before the server has read
    /// the feeder's table.
    pub read: Option<AlertsRead>,
    /// The open alerts, oldest first, as that read found them. An entry a
    /// client can't read (a kind a newer feeder added) is left out by
    /// `wellfed-server-client`, which counts it in `unread`.
    pub alerts: Vec<OpenAlert>,
    /// Since when the server has not reached the feeder, by the rule a
    /// phone at home uses (three failed tries in a row over at least ten
    /// minutes); `None` while it reaches it.
    pub unreachable: Option<Timestamp>,
}

/// One read of the feeder's alert table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AlertsRead {
    /// The ring's epoch: a feed's alert counts its number in it.
    pub epoch: u32,
    /// The feeder's boot count: the generation counts from 0 at each boot.
    pub boot: u32,
    /// Changes whenever anything in the table does.
    pub generation: Generation,
    /// A report left for want of room, or the boot read missed part of the
    /// ring: an absent report may not have ended.
    pub more: bool,
    /// An absent condition may not have ended yet.
    pub settling: bool,
    /// Alerts the server's build could not read: a newer feeder build's.
    pub unread: u8,
    /// Whether the feeder's status showed a valid clock at the read.
    pub clock_valid: bool,
    /// When the server read it. The table may have moved since while the
    /// feeder was out of reach.
    pub at: Timestamp,
    /// When the server last found the table unchanged, or read it: the
    /// table was the feeder's own at least until then. A client weighs a
    /// copy against its own last read by this, not `at`, since a table
    /// that stays the same keeps its `at` for good.
    pub checked: Timestamp,
}
