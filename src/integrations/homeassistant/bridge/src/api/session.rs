//! What a session hands Home Assistant: its updates, in order, and how
//! each write ended.

use wellfed_model as model;

use super::alerts::Alerts;
use super::enums::{NotSent, RefusalKind, Role, Skew, UpdateKind, WriteResult};
use super::state::{Capabilities, State};
use super::views::{Event, Feed};

/// A feed whose end is not known, until it is settled: what the
/// integration keeps across a restart, and hands back to the next session
/// so it is settled then. A feed is kept from just before it goes out
/// ([`UpdateKind::Sending`]), and goes only once the store says it saved
/// it, so a restart before its answer finds it lost rather than forgotten.
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
    /// When it went out, in Unix milliseconds: it is reported still
    /// unknown once the settle window has passed since, if nothing showed
    /// sooner that it never ran.
    pub sent_ms: i64,
    /// Directly: the newest event known when it went out, which its `Fed`
    /// comes after.
    pub after: Option<u64>,
    /// Reported still unknown ([`UpdateKind::StillUnknown`]): it is not
    /// reported again.
    pub reported: bool,
    /// When the owner released it ([`UpdateKind::Released`]), in Unix
    /// milliseconds. Directly, a lost feed holds new feeds back until it
    /// settles or is released, however long that takes; once released it
    /// is kept open to a late end for a week from then.
    pub released_ms: Option<i64>,
    /// Through the owner's server, the id of the token it went under, which
    /// names the journal that answers for it: a token belongs to one server
    /// and one account, and only that journal's lack of it says it was
    /// never sent. `None` for a direct feed. A route holds on the other
    /// route's lost feeds without settling them.
    pub token_id: Option<String>,
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

    pub(crate) fn feeding(feed: u32) -> Self {
        Self {
            feed: Some(feed),
            ..Self::result(WriteResult::Feeding)
        }
    }

    pub fn done() -> Self {
        Self::result(WriteResult::Done)
    }

    pub(crate) fn refused(refusal: &model::Refusal) -> Self {
        Self::refused_for(refusal.into())
    }

    /// Refused for `refusal`, as the owner's server names it.
    pub(crate) fn refused_for(refusal: RefusalKind) -> Self {
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

    pub(crate) fn uncertain() -> Self {
        Self::result(WriteResult::Uncertain)
    }

    /// An uncertain feed, kept as `pending` until an update settles it.
    pub(crate) fn uncertain_feed(pending: Pending) -> Self {
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
    /// [`UpdateKind::Settled`], [`UpdateKind::SettledUnfed`],
    /// [`UpdateKind::StillUnknown`], [`UpdateKind::Released`],
    /// [`UpdateKind::Forgotten`], [`UpdateKind::Sending`] and
    /// [`UpdateKind::Cleared`]: the pending feed it accounts for.
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
    /// For [`UpdateKind::Answered`]: every feed number to keep, in place of
    /// those kept before.
    pub answered: Vec<u32>,
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
            answered: Vec::new(),
        }
    }

    pub(crate) fn connected(
        build: impl Into<String>,
        role: model::ClientRole,
        capabilities: Capabilities,
        skew: model::Skew,
    ) -> Self {
        Self::connected_as(build, role.into(), capabilities, skew.into())
    }

    /// A session opened, with the side to update already decided: through
    /// the owner's server, its own revision counts too.
    pub(crate) fn connected_as(
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

    pub(crate) fn state(state: State) -> Self {
        Self {
            state: Some(state),
            ..Self::kind(UpdateKind::State)
        }
    }

    pub(crate) fn event(event: &model::Event, replayed: bool) -> Self {
        Self {
            event: Some(event.into()),
            replayed,
            ..Self::kind(UpdateKind::Event)
        }
    }

    pub(crate) fn gap(gap: Option<&model::Gap>) -> Self {
        Self {
            gap_from: gap.map(|gap| gap.from),
            gap_to: gap.map(|gap| gap.to),
            ..Self::kind(UpdateKind::Gap)
        }
    }

    /// A feed whose answer never came, to keep until an update settles
    /// it: sent before the write's own answer, so before any such update.
    pub(crate) fn pending(pending: Pending) -> Self {
        Self {
            portions: Some(pending.portions),
            pending: Some(pending),
            ..Self::kind(UpdateKind::Pending)
        }
    }

    /// The feed numbers this client has the end of, now that it has one
    /// more: what the integration keeps for the next session.
    pub(crate) fn answered(feeds: Vec<u32>) -> Self {
        Self {
            answered: feeds,
            ..Self::kind(UpdateKind::Answered)
        }
    }

    pub(crate) fn settled(feed: Feed, pending: Pending) -> Self {
        Self {
            feed: Some(feed),
            pending: Some(pending),
            ..Self::kind(UpdateKind::Settled)
        }
    }

    /// A feed whose answer never came did not run after all: the
    /// feeder refused it, as the server's journal later learned.
    pub(crate) fn settled_unfed(pending: Pending) -> Self {
        Self {
            pending: Some(pending),
            ..Self::kind(UpdateKind::SettledUnfed)
        }
    }

    pub(crate) fn still_unknown(pending: Pending) -> Self {
        Self {
            portions: Some(pending.portions),
            pending: Some(pending),
            ..Self::kind(UpdateKind::StillUnknown)
        }
    }

    /// A feed about to go out: kept, so a restart before its answer finds it
    /// lost. It goes once the store says it saved the record.
    pub(crate) fn sending(pending: Pending) -> Self {
        Self {
            portions: Some(pending.portions),
            pending: Some(pending),
            ..Self::kind(UpdateKind::Sending)
        }
    }

    /// A feed kept while it went out has its answer, or never went: no
    /// longer kept.
    pub(crate) fn cleared(pending: Pending) -> Self {
        Self {
            portions: Some(pending.portions),
            pending: Some(pending),
            ..Self::kind(UpdateKind::Cleared)
        }
    }

    /// The owner checked the bowls after the integration's store did not
    /// read: feeds go again.
    pub(crate) fn bowls_checked() -> Self {
        Self::kind(UpdateKind::BowlsChecked)
    }

    /// The owner released a lost feed: it holds new feeds back no longer.
    pub(crate) fn released(pending: Pending) -> Self {
        Self {
            portions: Some(pending.portions),
            pending: Some(pending),
            ..Self::kind(UpdateKind::Released)
        }
    }

    /// A feed whose answer never came is no longer waited on: still
    /// unknown, and nothing settles it now.
    pub(crate) fn forgotten(pending: Pending) -> Self {
        Self {
            portions: Some(pending.portions),
            pending: Some(pending),
            ..Self::kind(UpdateKind::Forgotten)
        }
    }

    /// The feeder's alerts moved: what is open, and what opened and
    /// cleared.
    pub(crate) fn alerts(alerts: Alerts) -> Self {
        Self {
            alerts: Some(alerts),
            ..Self::kind(UpdateKind::Alerts)
        }
    }

    pub(crate) fn revoked() -> Self {
        Self::kind(UpdateKind::Revoked)
    }

    pub(crate) fn token_refused() -> Self {
        Self::kind(UpdateKind::TokenRefused)
    }

    pub(crate) fn server_lost_feeder(reason: impl Into<String>) -> Self {
        Self {
            reason: Some(reason.into()),
            ..Self::kind(UpdateKind::ServerLostFeeder)
        }
    }

    pub(crate) fn disconnected(reason: impl Into<String>) -> Self {
        Self {
            reason: Some(reason.into()),
            ..Self::kind(UpdateKind::Disconnected)
        }
    }
}
