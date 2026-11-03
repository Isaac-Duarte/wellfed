//! The structs Home Assistant reads: a session's updates and what they
//! carry. Each is built from `wellfed-model`'s own value, never parsed.

use wellfed_model as model;

use super::enums::{
    By, EventKind, FeedOutcome, FeedReason, FoodLevel, Readiness, RefusalKind, Role, UpdateKind,
    WriteResult,
};

/// The client or plan a source names, beside its kind.
fn source_detail(source: &model::Source) -> (Option<String>, Option<u8>) {
    match source {
        model::Source::Client { client } => (Some(client.to_string()), None),
        model::Source::Schedule { plan } => (None, Some(*plan)),
        model::Source::Panel | model::Source::Feeder => (None, None),
    }
}

/// What the feeder reports about itself.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(frozen, get_all, eq, module = "wellfed_homeassistant")
)]
pub struct Status {
    pub build: String,
    /// The flash bank it booted from.
    pub bank: u8,
    pub uptime_ms: u32,
    pub boot_count: u32,
    /// Wi-Fi signal, dBm.
    pub rssi: i32,
    /// The feeder's clock, Unix milliseconds; `None` while it is invalid.
    pub clock_ms: Option<i64>,
    /// The panel's child lock; `None` before it is known.
    pub locked: Option<bool>,
    pub food: FoodLevel,
    /// `None` from a feeder that does not report it.
    pub readiness: Option<Readiness>,
    /// The last feed this boot that ended on a fault, until a later feed
    /// completes.
    pub feed_fault: Option<FeedFault>,
    /// On the adapter; `None` with no fresh sample or no supply report.
    pub adapter: Option<bool>,
    /// The pack's charge, 0-100 in steps of 10, while it reads one.
    pub battery_percent: Option<u8>,
    /// Whether the pack is below the floor a feed needs, once judged.
    pub below_floor: Option<bool>,
    /// Days left until the desiccant is due, below zero once overdue;
    /// `None` before a first replacement or while the clock is invalid.
    pub desiccant_days: Option<i32>,
    pub desiccant_due: bool,
    /// The newest event's sequence number.
    pub last_event: Option<u64>,
    /// Open device API sessions, this one included.
    pub sessions: u8,
}

impl From<&model::Status> for Status {
    fn from(status: &model::Status) -> Self {
        let supply = status.supply;
        let battery_percent = supply.and_then(|supply| match supply.battery {
            model::Battery::Reading { percent, .. } => Some(percent),
            _ => None,
        });
        let (desiccant_days, desiccant_due) = match status.desiccant {
            model::Desiccant::Remaining { days, due } => (Some(days), due),
            model::Desiccant::Unset | model::Desiccant::Unknown => (None, false),
        };

        Self {
            build: status.build.clone(),
            bank: status.bank,
            uptime_ms: status.uptime_ms,
            boot_count: status.boot_count,
            rssi: status.rssi,
            clock_ms: status.clock.map(|clock| clock.as_millisecond()),
            locked: status.locked,
            food: status.food.into(),
            readiness: status.readiness.map(Readiness::from),
            feed_fault: status.feed_fault.map(FeedFault::from),
            adapter: supply.and_then(|supply| supply.adapter),
            battery_percent,
            below_floor: supply.and_then(|supply| supply.below_floor),
            desiccant_days,
            desiccant_due,
            last_event: status.last_event,
            sessions: status.sessions,
        }
    }
}

/// A feed that ended on a fault; its `Fed` event has the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(frozen, get_all, eq, module = "wellfed_homeassistant")
)]
pub struct FeedFault {
    pub feed: u32,
    pub outcome: FeedOutcome,
    pub reason: FeedReason,
}

impl From<model::FeedFault> for FeedFault {
    fn from(fault: model::FeedFault) -> Self {
        Self {
            feed: fault.feed,
            outcome: fault.outcome.into(),
            reason: fault.reason.into(),
        }
    }
}

/// A feed that ended, from its `Fed` event.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(frozen, get_all, eq, module = "wellfed_homeassistant")
)]
pub struct Feed {
    /// The feeder's number for the feed.
    pub feed: u32,
    /// When it ended, by the feeder's clock in Unix milliseconds; `None`
    /// while that was invalid.
    pub at_ms: Option<i64>,
    pub requested: u8,
    /// The portions the feedback sensor confirmed.
    pub completed: u8,
    pub outcome: FeedOutcome,
    pub reason: FeedReason,
    /// What the food beam read during the feed.
    pub hopper: FoodLevel,
    pub by: By,
    /// The fingerprint of the client that asked, for [`By::Client`].
    pub client: Option<String>,
    /// The plan's slot, for [`By::Plan`].
    pub plan: Option<u8>,
}

impl Feed {
    /// The feed `event` records, if it is a `Fed` event.
    pub fn from_event(event: &model::Event) -> Option<Self> {
        let model::EventBody::Fed {
            feed,
            requested,
            completed,
            outcome,
            reason,
            hopper,
            by,
        } = &event.body
        else {
            return None;
        };
        let (client, plan) = source_detail(by);

        Some(Self {
            feed: *feed,
            at_ms: event.at.map(|at| at.as_millisecond()),
            requested: *requested,
            completed: *completed,
            outcome: (*outcome).into(),
            reason: (*reason).into(),
            hopper: (*hopper).into(),
            by: by.into(),
            client,
            plan,
        })
    }
}

/// One event from the feeder's history.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(frozen, get_all, eq, module = "wellfed_homeassistant")
)]
pub struct Event {
    /// The feeder's sequence number, rising across its reboots.
    pub seq: u64,
    /// When it happened, by the feeder's clock in Unix milliseconds;
    /// `None` while that was invalid.
    pub at_ms: Option<i64>,
    pub kind: EventKind,
    /// The feed, for [`EventKind::Fed`].
    pub fed: Option<Feed>,
    /// The panel's lock after a [`EventKind::Lock`].
    pub locked: Option<bool>,
    /// Who made the change or asked for the feed, when the event says.
    pub by: Option<By>,
    /// The client `by` names, or the client enrolled or revoked.
    pub client: Option<String>,
    /// The plan `by` names, or the plan missed or due soon.
    pub plan: Option<u8>,
}

impl From<&model::Event> for Event {
    fn from(event: &model::Event) -> Self {
        use model::EventBody as B;

        let source = match &event.body {
            B::Lock { by, .. }
            | B::Settings { by, .. }
            | B::Wifi { by, .. }
            | B::Revoked { by, .. }
            | B::ClockSync { by, .. }
            | B::Fed { by, .. }
            | B::Plans { by, .. } => Some(by),
            _ => None,
        };
        let (mut client, mut plan) = source.map_or((None, None), source_detail);
        match &event.body {
            B::Enrolled { client: named, .. } => client = Some(named.to_string()),
            B::Missed { plan: slot, .. } | B::MealSoon { plan: slot, .. } => plan = Some(*slot),
            _ => {}
        }
        let locked = match event.body {
            B::Lock { locked, .. } => Some(locked),
            _ => None,
        };

        Self {
            seq: event.seq,
            at_ms: event.at.map(|at| at.as_millisecond()),
            kind: (&event.body).into(),
            fed: Feed::from_event(event),
            locked,
            by: source.map(By::from),
            client,
            plan,
        }
    }
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
    pub reason: Option<String>,
}

impl Written {
    fn result(result: WriteResult) -> Self {
        Self {
            result,
            feed: None,
            refusal: None,
            reason: None,
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

    pub fn not_sent(reason: impl Into<String>) -> Self {
        Self {
            reason: Some(reason.into()),
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
    /// The build that answered, for [`UpdateKind::Connected`].
    pub build: Option<String>,
    /// The role this client holds, for [`UpdateKind::Connected`].
    pub role: Option<Role>,
    pub status: Option<Status>,
    pub event: Option<Event>,
    /// The first and last event lost, for a [`UpdateKind::Gap`] the feeder
    /// named; both `None` for a history that started over.
    pub gap_from: Option<u64>,
    pub gap_to: Option<u64>,
    /// Why the session ended, for [`UpdateKind::Disconnected`].
    pub reason: Option<String>,
}

impl Update {
    fn kind(kind: UpdateKind) -> Self {
        Self {
            kind,
            build: None,
            role: None,
            status: None,
            event: None,
            gap_from: None,
            gap_to: None,
            reason: None,
        }
    }

    pub fn connected(build: impl Into<String>, role: model::ClientRole) -> Self {
        Self {
            build: Some(build.into()),
            role: Some(role.into()),
            ..Self::kind(UpdateKind::Connected)
        }
    }

    pub fn status(status: &model::Status) -> Self {
        Self {
            status: Some(status.into()),
            ..Self::kind(UpdateKind::Status)
        }
    }

    pub fn event(event: &model::Event) -> Self {
        Self {
            event: Some(event.into()),
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

    pub fn disconnected(reason: impl Into<String>) -> Self {
        Self {
            reason: Some(reason.into()),
            ..Self::kind(UpdateKind::Disconnected)
        }
    }
}
