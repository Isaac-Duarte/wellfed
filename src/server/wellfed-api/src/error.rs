//! Errors as RFC 9457 `application/problem+json`, each with a typed code.

use wellfed_model::Refusal;

use crate::ReleaseRefusal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Every way a request fails. Clients branch on this, never on the title.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The first owner exists; setup is closed.
    SetupClosed,
    /// No valid session or token.
    Unauthenticated,
    BadCredentials,
    /// Too many failed logins for this name or from this address (5 a
    /// minute per name, 20 per address). `Retry-After` says when the next
    /// is taken. No account is locked, and sessions and tokens carry on.
    LoginLimited,
    /// The account is past its expiry: it cannot log in and its sessions
    /// stop, until an owner extends or deletes it.
    AccountDisabled,
    /// This user's role may not do that.
    Forbidden,
    /// Only a login, not a token, may do that: making a token, setting a
    /// password or adding an account, so a leaked token cannot turn itself
    /// into a login or more tokens that outlive its revocation.
    SessionRequired,
    /// The change would leave the household without an owner.
    LastOwner,
    /// Another account has that name.
    NameTaken,
    /// A feeder's food history names the food, so deleting it would lose
    /// past feeds' calories.
    FoodInUse,
    /// A full-bag refill on a feeder with no food set: set its food first.
    NoCurrentFood,
    /// The zone is real, but no POSIX rule describes it (it changes
    /// irregularly), so a feeder could not keep it; `detail` names it.
    ZoneUnsupported,
    /// A cookie-authenticated write came from another origin. A write
    /// with a bearer token needs no `Origin`.
    CrossOrigin,
    /// The request named a host this server does not answer to: an IP
    /// address, `localhost` or a name it was started with.
    UnknownHost,
    NotFound,
    /// A field or header failed the server's checks.
    Invalid,
    /// An edit came without `If-Match`.
    PreconditionRequired,
    /// The request's body is larger than the route takes.
    TooLarge,
    /// The feeder is already adopted.
    AlreadyAdopted,
    /// The feeder was removed from the server: its history stays readable,
    /// and adopting it again brings it back.
    FeederArchived,
    /// The request is one kind of device's, and this device is another
    /// kind (`Feeder.kind`): nothing was journaled or sent.
    OtherKind,
    /// Only a phone's token, not a browser session, gets an invite for a
    /// phone: the key is recorded as that token's.
    TokenRequired,
    /// The account or token has an expiry, so its phone holds no key of its
    /// own and reaches the feeders through the server only.
    DirectKeyRefused,
    /// The key is already one of the feeder's clients, or another phone's.
    ClientKeyTaken,
    /// The feeder does not list the phone key claimed, so it is not
    /// recorded as the caller's. An adoption that named it stands: the
    /// feeder is in `GET /feeders`.
    PhoneKeyNotListed,
    /// The server could not read the feeder's clients to confirm the phone
    /// key claimed (the feeder offline, or slow to connect), so it is not
    /// recorded yet. An adoption that named it stands; claim the key again
    /// (`POST /feeders/{id}/phones`).
    PhoneKeyUnconfirmed,
    /// An owner kept the key claimed as a client of its own (the CLI, say),
    /// so it is not a phone's to claim.
    PhoneKeyKept,
    /// The feeder lists the key claimed with a role no phone holds: a
    /// server's or an update signer's.
    PhoneKeyNotAPhone,
    /// Only an owner claims a key the server has not recorded: the feeder
    /// cannot prove whose a key is, and a member's claim could take another
    /// person's key and then revoke it. A member's phone that paired before
    /// the server gets a key of its own through an invite
    /// (`POST /feeders/{id}/invites`).
    PhoneClaimOwnersOnly,
    /// The feeder dropped the invite unanswered: unknown, used or expired.
    InviteExpired,
    /// The server could not reach the feeder.
    Unreachable,
    /// The server has no session with the feeder right now.
    FeederOffline,
    /// The feeder refused the request; `refusal` says why. A refused write is
    /// final for its `Idempotency-Key`: the feeder replays the refusal while
    /// it remembers the id, and a later attempt needs a new key.
    FeederRefused,
    /// No answer arrived in time. The feeder may have run the request:
    /// retry with the same `Idempotency-Key`, which it answers once.
    OutcomeUnknown,
    /// The `Idempotency-Key` already names another request (another
    /// body, feeder, kind of write or user). Nothing was sent.
    KeyReused,
    /// Another change to the feeder's meal call came between a write's
    /// first send and its retry, so the retry sent nothing: it would have
    /// undone that change. Final for the key.
    Superseded,
    /// The file is not a release the server keeps; `release` says why.
    ReleaseRefused,
    /// Fetching releases is off (`releases.fetch`); uploads still work.
    FetchDisabled,
    /// The server could not reach the project's releases, or they answered
    /// with something it could not read.
    ReleasesUnreachable,
    /// Another install is running on this server: one runs at a time. A
    /// stock restore counts as one, and a feeder's stock is not read while
    /// one runs on it.
    InstallRunning,
    /// The server keeps no stock backup for the feeder, or none that
    /// still verifies.
    NoStockBackup,
    /// The image is not stock's pinned image (damaged, partial or another
    /// release): nothing was kept.
    StockRefused,
    /// The feeder runs from bank 0 with stock still in bank 1, its only
    /// copy, and the server keeps no verified backup of it: an install
    /// would overwrite it. Read it (`POST /feeders/{id}/stock/read`) or
    /// upload the installer's copy first.
    StockBackupNeeded,
    /// The server already keeps a different copy of the feeder's stock;
    /// a kept copy is never replaced.
    StockDiffers,
    /// The restore's manifest does not offer the kept backup as the stock
    /// image: nothing was sent.
    RestoreMismatch,
    /// An account exists: only a fresh server takes an import.
    ImportClosed,
    /// The upload is not an archive this server imports, or does not
    /// match its own manifest; `detail` says how. Nothing was imported.
    ArchiveInvalid,
    /// Another export is being written: one runs at a time.
    ExportRunning,
    Internal,
}

/// Why the feeder refused, as it said.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FeederRefusal {
    /// The server's key lacks the role.
    Forbidden,
    /// Another change or feed is in progress, or the stores are loading.
    /// Nothing ran. Trying again later is a new request with a new key.
    Busy,
    /// The store moved past the generation the request named.
    Stale,
    /// The firmware does not serve this yet.
    Unavailable,
    Invalid,
    NotWritable,
    /// Every client slot or invite is taken.
    Full,
    NotFound,
    Muted,
    /// The feeder's clock refused the sync.
    Time,
    /// The feeding owner stopped on a fault; the feeder needs a restart.
    Fault,
    /// A feed, or a plan, asked for portions outside 1-48.
    Portions,
    /// The feeder's clock is invalid, so it cannot date the change.
    NoClock,
    /// A calibration named a bowl mode the feeder has not verified.
    Unverified,
}

impl From<Refusal> for FeederRefusal {
    fn from(refusal: Refusal) -> Self {
        match refusal {
            Refusal::Forbidden => Self::Forbidden,
            Refusal::Stale { .. } => Self::Stale,
            Refusal::Busy => Self::Busy,
            Refusal::Unavailable => Self::Unavailable,
            Refusal::Invalid => Self::Invalid,
            Refusal::NotWritable => Self::NotWritable,
            Refusal::Full => Self::Full,
            Refusal::NotFound => Self::NotFound,
            Refusal::Muted => Self::Muted,
            Refusal::Time => Self::Time,
            Refusal::Fault => Self::Fault,
            Refusal::Portions { .. } => Self::Portions,
            Refusal::NoClock => Self::NoClock,
            Refusal::Unverified => Self::Unverified,
            // An install reads its own refusals from the update driver;
            // through a journalled write one could only be a session
            // without the owner role, refused before these could arrive.
            Refusal::Update(_) => Self::Forbidden,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Problem {
    /// Always `about:blank`: `code` carries the type.
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub status: u16,
    pub code: ErrorCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal: Option<FeederRefusal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<ReleaseRefusal>,
}
