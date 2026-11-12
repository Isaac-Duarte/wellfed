//! What a session hands Home Assistant: its updates, in order, and how
//! each write ended.

use wellfed_model as model;

use super::alerts::Alerts;
use super::enums::{NotSent, RefusalKind, Role, Skew, UpdateKind, WriteResult};
use super::state::{Capabilities, State};
use super::views::{Event, Feed};

/// A feed whose answer never came, until it is settled: what the
/// integration keeps across a restart, and hands back to the next session
/// so it is settled then.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(frozen, get_all, eq, from_py_object, module = "wellfed_homeassistant")
)]
pub struct Pending {
    /// Names it: through the owner's server, the idempotency key it went
    /// under, which the server's journal answers by.
    pub id: String,
    pub portions: u8,
    /// When it went out, in Unix milliseconds: it is still unknown once
    /// the settle window has passed since.
    pub sent_ms: i64,
    /// Directly: the newest event known when it went out, which its `Fed`
    /// comes after.
    pub after: Option<u64>,
}

/// How a write ended.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(frozen, get_all, eq, module = "wellfed_homeassistant")
)]
pub struct Written {
    pub result: WriteResult,
    /// The feeder's number for a feed it reserved.
    pub feed: Option<u32>,
    /// Why it refused, for [`WriteResult::Refused`].
    pub refusal: Option<RefusalKind>,
    /// Why it never went out, for [`WriteResult::NotSent`].
    pub not_sent: Option<NotSent>,
    /// For an uncertain feed: what to keep until an update settles it.
    pub pending: Option<Pending>,
}

impl Written {
    fn result(result: WriteResult) -> Self {
        Self {
            result,
            feed: None,
            refusal: None,
            not_sent: None,
            pending: None,
        }
    }

    pub fn feeding(feed: u32) -> Self {
        Self {
            feed: Some(feed),
            ..Self::result(WriteResult::Feeding)
        }
    }

    pub fn done() -> Self {
        Self::result(WriteResult::Done)
    }

    pub fn refused(refusal: &model::Refusal) -> Self {
        Self::refused_for(refusal.into())
    }

    /// Refused for `refusal`, as the owner's server names it.
    pub fn refused_for(refusal: RefusalKind) -> Self {
        Self {
            refusal: Some(refusal),
            ..Self::result(WriteResult::Refused)
        }
    }

    pub fn not_sent(why: NotSent) -> Self {
        Self {
            not_sent: Some(why),
            ..Self::result(WriteResult::NotSent)
        }
    }

    pub fn uncertain() -> Self {
        Self::result(WriteResult::Uncertain)
    }

    /// An uncertain feed, kept as `pending` until an update settles it.
    pub fn uncertain_feed(pending: Pending) -> Self {
        Self {
            pending: Some(pending),
            ..Self::uncertain()
        }
    }
}

/// What the session tells Home Assistant, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(frozen, get_all, eq, module = "wellfed_homeassistant")
)]
pub struct Update {
    pub kind: UpdateKind,
    /// For [`UpdateKind::Connected`]: the build that answered, this
    /// client's role, what the feeder serves and how its API revision
    /// stands against this integration's.
    pub build: Option<String>,
    pub role: Option<Role>,
    pub capabilities: Option<Capabilities>,
    pub skew: Option<Skew>,
    /// For [`UpdateKind::State`].
    pub state: Option<State>,
    /// For [`UpdateKind::Event`].
    pub event: Option<Event>,
    /// Whether the event came from the history read at connect: already
    /// part of the state, and not news.
    pub replayed: bool,
    /// For [`UpdateKind::Settled`]: the feed whose answer never came.
    pub feed: Option<Feed>,
    /// For [`UpdateKind::StillUnknown`]: the portions that feed asked for.
    pub portions: Option<u8>,
    /// For [`UpdateKind::Pending`]: the feed to keep. For
    /// [`UpdateKind::Settled`], [`UpdateKind::SettledUnfed`] and
    /// [`UpdateKind::StillUnknown`]: the pending feed it accounts for.
    pub pending: Option<Pending>,
    /// The first and last event lost, for a [`UpdateKind::Gap`] the feeder
    /// named; both `None` for a history that started over.
    pub gap_from: Option<u64>,
    pub gap_to: Option<u64>,
    /// Why the session ended, for [`UpdateKind::Disconnected`] and
    /// [`UpdateKind::ServerLostFeeder`]: for logs.
    pub reason: Option<String>,
    /// For [`UpdateKind::Alerts`].
    pub alerts: Option<Alerts>,
    /// For [`UpdateKind::Answered`]: the feed number to keep.
    pub answered: Option<u32>,
}

impl Update {
    fn kind(kind: UpdateKind) -> Self {
        Self {
            kind,
            build: None,
            role: None,
            capabilities: None,
            skew: None,
            state: None,
            event: None,
            replayed: false,
            feed: None,
            portions: None,
            pending: None,
            gap_from: None,
            gap_to: None,
            reason: None,
            alerts: None,
            answered: None,
        }
    }

    pub fn connected(
        build: impl Into<String>,
        role: model::ClientRole,
        capabilities: Capabilities,
        skew: model::Skew,
    ) -> Self {
        Self::connected_as(build, role.into(), capabilities, skew.into())
    }

    /// A session opened, with the side to update already decided: through
    /// the owner's server, its own revision counts too.
    pub fn connected_as(
        build: impl Into<String>,
        role: Role,
        capabilities: Capabilities,
        skew: Skew,
    ) -> Self {
        Self {
            build: Some(build.into()),
            role: Some(role),
            capabilities: Some(capabilities),
            skew: Some(skew),
            ..Self::kind(UpdateKind::Connected)
        }
    }

    pub fn state(state: State) -> Self {
        Self {
            state: Some(state),
            ..Self::kind(UpdateKind::State)
        }
    }

    pub fn event(event: &model::Event, replayed: bool) -> Self {
        Self {
            event: Some(event.into()),
            replayed,
            ..Self::kind(UpdateKind::Event)
        }
    }

    pub fn gap(gap: Option<&model::Gap>) -> Self {
        Self {
            gap_from: gap.map(|gap| gap.from),
            gap_to: gap.map(|gap| gap.to),
            ..Self::kind(UpdateKind::Gap)
        }
    }

    /// A feed whose answer never came, to keep until an update settles
    /// it: sent before the write's own answer, so before any such update.
    pub fn pending(pending: Pending) -> Self {
        Self {
            portions: Some(pending.portions),
            pending: Some(pending),
            ..Self::kind(UpdateKind::Pending)
        }
    }

    /// A feed number this client now has the end of.
    pub fn answered(feed: u32) -> Self {
        Self {
            answered: Some(feed),
            ..Self::kind(UpdateKind::Answered)
        }
    }

    pub fn settled(feed: Feed, pending: Pending) -> Self {
        Self {
            feed: Some(feed),
            pending: Some(pending),
            ..Self::kind(UpdateKind::Settled)
        }
    }

    /// A feed whose answer never came did not run after all: the
    /// feeder refused it, as the server's journal later learned.
    pub fn settled_unfed(pending: Pending) -> Self {
        Self {
            pending: Some(pending),
            ..Self::kind(UpdateKind::SettledUnfed)
        }
    }

    pub fn still_unknown(pending: Pending) -> Self {
        Self {
            portions: Some(pending.portions),
            pending: Some(pending),
            ..Self::kind(UpdateKind::StillUnknown)
        }
    }

    /// The feeder's alerts moved: what is open, and what opened and
    /// cleared.
    pub fn alerts(alerts: Alerts) -> Self {
        Self {
            alerts: Some(alerts),
            ..Self::kind(UpdateKind::Alerts)
        }
    }

    pub fn revoked() -> Self {
        Self::kind(UpdateKind::Revoked)
    }

    pub fn token_refused() -> Self {
        Self::kind(UpdateKind::TokenRefused)
    }

    pub fn server_lost_feeder(reason: impl Into<String>) -> Self {
        Self {
            reason: Some(reason.into()),
            ..Self::kind(UpdateKind::ServerLostFeeder)
        }
    }

    pub fn disconnected(reason: impl Into<String>) -> Self {
        Self {
            reason: Some(reason.into()),
            ..Self::kind(UpdateKind::Disconnected)
        }
    }
}
