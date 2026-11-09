//! What a session hands Home Assistant: its updates, in order, and how
//! each write ended.

use wellfed_model as model;

use super::enums::{NotSent, RefusalKind, Role, Skew, UpdateKind, WriteResult};
use super::state::{Capabilities, State};
use super::views::{Event, Feed};

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
}

impl Written {
    fn result(result: WriteResult) -> Self {
        Self {
            result,
            feed: None,
            refusal: None,
            not_sent: None,
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
    /// For [`UpdateKind::Settled`]: the feed whose answer never came;
    /// `None` when the server's journal found it refused, so nothing ran.
    pub feed: Option<Feed>,
    /// For [`UpdateKind::StillUnknown`]: the portions that feed asked for.
    pub portions: Option<u8>,
    /// The first and last event lost, for a [`UpdateKind::Gap`] the feeder
    /// named; both `None` for a history that started over.
    pub gap_from: Option<u64>,
    pub gap_to: Option<u64>,
    /// Why the session ended, for [`UpdateKind::Disconnected`] and
    /// [`UpdateKind::ServerLostFeeder`]: for logs.
    pub reason: Option<String>,
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
            gap_from: None,
            gap_to: None,
            reason: None,
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

    pub fn settled(feed: Feed) -> Self {
        Self {
            feed: Some(feed),
            ..Self::kind(UpdateKind::Settled)
        }
    }

    /// A feed whose answer never came did not run after all: the
    /// feeder refused it, as the server's journal later learned.
    pub fn settled_unfed() -> Self {
        Self::kind(UpdateKind::Settled)
    }

    pub fn still_unknown(portions: u8) -> Self {
        Self {
            portions: Some(portions),
            ..Self::kind(UpdateKind::StillUnknown)
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
