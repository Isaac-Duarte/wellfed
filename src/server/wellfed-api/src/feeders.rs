//! Adopted feeders, their status, settings, plans and lock.

use std::fmt;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use wellfed_model::{
    ClientRole, DeviceKind, Hex, MaybeKnown, Model, NextMeal, PLANS, Plan, Skew, Status,
};

use crate::{FeederId, HomeId};

/// Adopt a feeder by redeeming an invite its owner created for the server
/// (`wellfed invite --role server`), or one the owner's phone made with its
/// own key.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AdoptFeeder {
    /// The invite's text, `wellfed-invite:1:…`. It holds a secret, so it
    /// is never stored or logged.
    pub invite: String,
    /// What the household calls the feeder (1-64 characters).
    pub name: String,
    /// The feeder's IP address (the device API's port) or `IP:port`; a DNS
    /// name is refused. Found over mDNS when absent, and required while
    /// discovery is off.
    #[serde(default)]
    pub address: Option<String>,
    /// The public key the caller's phone already holds on the feeder, when
    /// the phone adopts it: once the feeder lists that key, the server
    /// records it as this account's phone key, as if it had invited it, so
    /// deleting, demoting or putting an expiry on the account revokes it.
    /// Only a phone's token names one, for an account and token with no
    /// expiry. Absent when nothing but the server joins (the CLI).
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub phone_key: Option<Hex<32>>,
}

impl fmt::Debug for AdoptFeeder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdoptFeeder")
            .field("name", &self.name)
            .field("address", &self.address)
            .field("phone_key", &self.phone_key)
            .finish_non_exhaustive()
    }
}

/// The server's session with a feeder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Connection {
    Offline,
    Connecting,
    Online,
    /// The feeder no longer knows the server's key (revoked, or reset).
    Refused,
    /// A state a newer server added, which this client cannot name. The
    /// server never sends it.
    #[serde(other)]
    Unknown,
}

/// A device the server mirrors. The API names every one a feeder for now;
/// `kind` says which it is, and an op of another kind is refused
/// `not_this_kind` before it is sent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Feeder {
    pub id: FeederId,
    /// The home it belongs to.
    pub home: HomeId,
    /// Which module of the device API it serves: its model's kind.
    /// `unknown` is a kind a newer server added: the client shows the
    /// device as needing it updated.
    #[schema(schema_with = wellfed_model::schema::maybe_device_kind)]
    pub kind: MaybeKnown<DeviceKind>,
    /// Its model, as the welcome of the session at adoption named it;
    /// `unknown` as for `kind`.
    #[schema(schema_with = wellfed_model::schema::maybe_model)]
    pub model: MaybeKnown<Model>,
    pub name: String,
    /// The feeder's identity key.
    #[schema(value_type = String)]
    pub identity: Hex<32>,
    /// The role the feeder enrolled the server's key with.
    pub role: ClientRole,
    /// Where the server last reached it.
    pub address: Option<String>,
    pub connection: Connection,
    /// How the feeder's device API revision stands against this server's,
    /// from its last welcome: `feeder_behind` asks for the feeder to be
    /// updated, `feeder_ahead` for the server. Either way it is mirrored and
    /// sent every command: what one side lacks is refused or read as
    /// unknown. `None` before a session has opened since the server started.
    pub skew: Option<Skew>,
    /// The newest event the server holds, `None` before the first.
    pub synced: Option<u64>,
    /// The feeder's history epoch `synced` counts in.
    pub epoch: Option<u32>,
    /// When it was removed from the server; `None` while it is mirrored.
    pub archived: Option<Timestamp>,
}

/// One feeder and its latest status.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FeederDetail {
    pub feeder: Feeder,
    /// `None` until the server has read it once.
    pub status: Option<StatusSnapshot>,
    /// The plans' next meal in the home's zone, from the plans the
    /// server last read; `None` with no enabled plan, no plans read yet,
    /// or no home.
    pub next_meal: Option<NextMeal>,
}

/// The feeding plans to replace every slot with, as one table: a slot left
/// `null` is emptied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PlansChange {
    /// All ten slots.
    #[schema(min_items = 10, max_items = 10)]
    pub plans: [Option<Plan>; PLANS],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct StatusSnapshot {
    /// When the server read it.
    pub at: Timestamp,
    /// Read for this request, rather than the last copy from before the
    /// feeder went offline.
    pub live: bool,
    pub status: Status,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct LockRequest {
    pub locked: bool,
}

/// One of today's meals: its plan's slot and the instant it is due, as
/// the plans' next meal or their `skipped` list names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct MealRequest {
    /// The plan's slot, 0 to 9.
    #[schema(maximum = 9)]
    pub slot: u8,
    pub due: Timestamp,
}
