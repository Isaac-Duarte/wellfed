//! What a session hands Home Assistant: its updates, in order, and how
//! each write ended.

use wellfed_model as model;

use super::enums::{NotSent, RefusalKind, Role, UpdateKind, WriteResult};
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
        Self {
            refusal: Some(refusal.into()),
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
    /// client's role and what the feeder serves.
    pub build: Option<String>,
    pub role: Option<Role>,
    pub capabilities: Option<Capabilities>,
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
    /// The first and last event lost, for a [`UpdateKind::Gap`] the feeder
    /// named; both `None` for a history that started over.
    pub gap_from: Option<u64>,
    pub gap_to: Option<u64>,
    /// Why the session ended, for [`UpdateKind::Disconnected`]: for logs.
    pub reason: Option<String>,
}

impl Update {
    fn kind(kind: UpdateKind) -> Self {
        Self {
            kind,
            build: None,
            role: None,
            capabilities: None,
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
    ) -> Self {
        Self {
            build: Some(build.into()),
            role: Some(role.into()),
            capabilities: Some(capabilities),
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

    pub fn still_unknown(portions: u8) -> Self {
        Self {
            portions: Some(portions),
            ..Self::kind(UpdateKind::StillUnknown)
        }
    }

    pub fn revoked() -> Self {
        Self::kind(UpdateKind::Revoked)
    }

    pub fn disconnected(reason: impl Into<String>) -> Self {
        Self {
            reason: Some(reason.into()),
            ..Self::kind(UpdateKind::Disconnected)
        }
    }
}
