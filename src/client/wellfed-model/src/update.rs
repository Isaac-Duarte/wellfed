//! Updates over the device API as a host reads them: where an install
//! stands and why one was refused.

use serde::{Deserialize, Serialize};

use crate::Hex;

/// One page of bank 1, as the bank-0 recovery role answers `ReadBank1`:
/// `bytes` from `offset`, of the `total` it serves
/// (`wellfed_client::install::backup::Bank1Reader` asks for every page).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bank1Page {
    pub offset: u32,
    pub total: u32,
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for Bank1Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bank1Page")
            .field("offset", &self.offset)
            .field("total", &self.total)
            .field("len", &self.bytes.len())
            .finish()
    }
}

/// Where an update stands. The install does not depend on the session: a
/// client that lost its connection reconnects, reads the state and sends
/// what [`Self::wanted`] names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateView {
    pub phase: UpdatePhase,
    /// The bank this answer comes from.
    pub bank: u8,
    /// The highest confirmed version: an offer at or below it needs a
    /// downgrade statement.
    pub min_version: u32,
    /// The manifest being checked, installed or given way for, if any. At
    /// `Idle`, one a cut or kill left pending: the feeder refuses any other
    /// until it is cancelled.
    pub digest: Option<Hex<32>>,
    pub version: u32,
    pub image_len: u32,
    /// Bytes written and read back so far.
    pub written: u32,
    /// The chunk offsets wanted now, in order; empty between sectors or
    /// outside an install.
    pub wanted: Vec<u32>,
    /// Why the last install stopped before activation, if it did.
    pub stopped: Option<UpdateStop>,
    /// Bank 0, the recovery copy, as bank 1 last checked it; always
    /// unchecked from bank 0.
    pub bank0: Bank0Check,
}

/// Bank 0, the recovery copy, as bank 1 last checked it (once per boot,
/// and at every offer). Bank 1 gives way for an update only to a bank 0
/// that booted, recorded its own image and still holds exactly that image.
///
/// Read by hand: serde's catch-all on an adjacently tagged enum fails on a
/// newer state that carries a `reason`, and any newer state reads as
/// [`Self::Unknown`] here, whatever it carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "reason", rename_all = "snake_case")]
pub enum Bank0Check {
    /// Not checked yet this boot.
    Unchecked,
    /// Bank 0 holds the image it recorded: updates can go ahead.
    Match,
    /// Bank 1 will not give way to it, so every update is refused.
    Blocked(Bank0Blocked),
    /// A value a newer build added, which this one cannot name. Never
    /// written to the wire.
    #[serde(other)]
    Unknown,
}

impl<'de> Deserialize<'de> for Bank0Check {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum State {
            Unchecked,
            Match,
            Blocked,
            #[serde(other)]
            Unknown,
        }

        #[derive(Deserialize)]
        struct Raw {
            state: State,
            reason: Option<Reason>,
        }

        /// A known state's reason, or whatever a newer state carries.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Reason {
            Blocked(Bank0Blocked),
            Other(serde::de::IgnoredAny),
        }

        let raw = Raw::deserialize(deserializer)?;
        Ok(match (raw.state, raw.reason) {
            (State::Unchecked, _) => Self::Unchecked,
            (State::Match, _) => Self::Match,
            (State::Blocked, Some(Reason::Blocked(reason))) => Self::Blocked(reason),
            (State::Blocked, _) => return Err(serde::de::Error::missing_field("reason")),
            (State::Unknown, _) => Self::Unknown,
        })
    }
}

impl Bank0Check {
    /// What blocks updates, if anything does.
    pub const fn blocked(self) -> Option<Bank0Blocked> {
        match self {
            Self::Blocked(reason) => Some(reason),
            Self::Unchecked | Self::Match => None,
            // A state a newer build added: nothing says bank 0 may give way.
            Self::Unknown => Some(Bank0Blocked::Unknown),
        }
    }
}

/// Why bank 1 will not give way to bank 0, in the order it checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bank0Blocked {
    /// Bank 0's loader marker is not valid.
    Marker,
    /// Bank 0 never recorded itself: written over UART and not booted
    /// since. Booting it once records it.
    Unrecorded,
    /// Bank 0's built-in update key is not the one that signed the offer.
    Key,
    /// Bank 0's image headers do not give an image.
    Extent,
    /// Bank 0 changed since it recorded itself.
    Changed,
    /// Bank 0 could not be read.
    Read,
    /// A value a newer build added, which this one cannot name. Never
    /// written to the wire.
    #[serde(other)]
    Unknown,
}

impl Bank0Blocked {
    /// The JSON spelling, as the feeder's `PLWCH1 refused give_way
    /// reason=…` record spells it too.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Marker => "marker",
            Self::Unrecorded => "unrecorded",
            Self::Key => "key",
            Self::Extent => "extent",
            Self::Changed => "changed",
            Self::Read => "read",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePhase {
    Idle,
    /// The offer is being checked: ask again.
    Checking,
    /// Bank 1 recorded the offer and restarts into bank 0: reconnect and
    /// offer the same manifest again.
    GivingWay,
    /// Bank 0 is writing bank 1: send the wanted chunks.
    Receiving,
    /// Every byte verified and the marker written: the feeder restarts
    /// into the new build, which confirms itself once healthy.
    Activated,
}

/// What the update store held about a candidate as a boot came up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateCandidate {
    /// None, or a store that could not be read.
    None,
    /// Bank 1 gave way for one, or bank 0 accepted it.
    Pending,
    /// Bank 0 activated one, and no build has confirmed it.
    Activated,
    /// A value a newer build added, which this one cannot name. Never
    /// written to the wire.
    #[serde(other)]
    Unknown,
}

/// Why an install stopped. Bank 1's marker was never written, so the
/// feeder keeps running bank 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateStop {
    /// No chunk came for a minute.
    Timeout,
    Cancelled,
    /// A chunk that was not wanted, or of the wrong length.
    Order,
    /// The image's first bytes are not the loader's marker.
    Marker,
    /// A sector did not read back as written.
    Write,
    /// The whole image's SHA-256 differs from the manifest's.
    Hash,
    /// The marker was never programmed.
    Activate,
    /// The update store could not be written.
    Store,
    /// A value a newer build added, which this one cannot name. Never
    /// written to the wire.
    #[serde(other)]
    Unknown,
}

/// Why an update op was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", remote = "Self")]
pub enum UpdateRefusal {
    /// Not a version 2 manifest, or an unknown image kind.
    Manifest,
    /// No trusted key has the manifest's (or a statement's) key id.
    Key,
    /// That key was retired.
    Revoked,
    Signature,
    /// Not for bank 1, outside the reviewed size, or built for another
    /// device model.
    Target,
    /// Bank 1 already runs that build.
    Same,
    /// The version is not above the floor (or the image is stock): a
    /// downgrade statement must come with it.
    Downgrade,
    /// The downgrade or key statement does not check out.
    Statement,
    /// Bank 0 installs only the manifest bank 1 gave way for.
    Pending,
    /// This manifest was activated and failed its trial.
    Failed,
    /// Bank 0 holds no build that can install it, so bank 1 does not
    /// give way: `reason` names the check.
    Fallback {
        reason: Bank0Blocked,
    },
    /// An install reset the feeder this power cycle.
    Attempt,
    /// Another update is in progress.
    Installing,
    /// No install is receiving chunks.
    NotInstalling,
    /// The update store cannot be read or written.
    Store,
    /// Nobody pressed Dispense in time to confirm the key.
    Unconfirmed,
    /// The feeder runs on its batteries: no install starts until the
    /// adapter returns.
    Battery,
    /// The key is already enrolled as a session key, which is never an
    /// update signer too.
    HeldAsSessionKey,
    /// A value a newer build added, which this one cannot name. Never
    /// written to the wire.
    #[serde(other)]
    Unknown,
}

impl Serialize for UpdateRefusal {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Self::serialize(self, serializer)
    }
}

// Read by hand around the derived reader: serde's catch-all on an
// externally tagged enum takes only a bare name, so a newer refusal that
// carries data (`{"a_newer": {..}}`) would fail the whole body. Any refusal
// the derived reader can't read is `Unknown`.
impl<'de> Deserialize<'de> for UpdateRefusal {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Known(#[serde(with = "UpdateRefusal")] UpdateRefusal),
            Newer(serde::de::IgnoredAny),
        }

        Ok(match Wire::deserialize(deserializer)? {
            Wire::Known(refusal) => refusal,
            Wire::Newer(_) => Self::Unknown,
        })
    }
}
