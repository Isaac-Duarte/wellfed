//! The structs Home Assistant reads: a session's updates and what they
//! carry. Each is built from `wellfed-model`'s own value, never parsed.

use wellfed_model as model;

use wellfed_client::history;

use super::enums::{By, EventKind, FeedOutcome, FeedReason, FoodLevel, Readiness};

/// The client or plan a source names, beside its kind.
pub(super) fn source_detail(source: &model::Source) -> (Option<String>, Option<u8>) {
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
    /// Whether a feed can start now.
    pub readiness: Readiness,
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
        // The integration serves feeders alone.
        let model::DeviceStatus::Feeder(feeder) = &status.device;
        let supply = feeder.supply;
        let battery_percent = match supply.battery {
            model::Battery::Reading { percent, .. } => Some(percent),
            _ => None,
        };
        let (desiccant_days, desiccant_due) = match feeder.desiccant {
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
            locked: feeder.locked,
            food: feeder.food.into(),
            readiness: feeder.readiness.into(),
            feed_fault: feeder.feed_fault.map(FeedFault::from),
            adapter: supply.adapter,
            battery_percent,
            below_floor: supply.below_floor,
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

/// A feed that ended, from its `Fed` event, as the feeding history reads
/// it: an uncertain feed is a range of portions, never one count.
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
    /// The most that may have dropped: `completed` unless the feed is
    /// uncertain, when it is what was asked for.
    pub at_most: u8,
    pub outcome: FeedOutcome,
    pub reason: FeedReason,
    /// What the food beam read during the feed.
    pub hopper: FoodLevel,
    /// Who asked; `None` for a feed a reset interrupted, whose requester
    /// the feeder's journal does not keep.
    pub by: Option<By>,
    /// The fingerprint of the client that asked, for [`By::Client`].
    pub client: Option<String>,
    /// The plan's slot, for [`By::Plan`].
    pub plan: Option<u8>,
}

impl From<&history::Feed> for Feed {
    fn from(fed: &history::Feed) -> Self {
        let (completed, at_most) = fed.delivered.bounds();
        let (client, plan) = fed.by.as_ref().map_or((None, None), source_detail);

        Self {
            feed: fed.feed,
            at_ms: fed.at.map(|at| at.as_millisecond()),
            requested: fed.requested,
            completed,
            at_most,
            outcome: fed.outcome.into(),
            reason: fed.reason.into(),
            hopper: fed.hopper.into(),
            by: fed.by.as_ref().map(By::from),
            client,
            plan,
        }
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
        let source = match &event.body {
            model::EventBody::Feeder(model::FeederEvent::Lock { by, .. })
            | model::EventBody::Feeder(model::FeederEvent::Settings { by, .. })
            | model::EventBody::Core(model::CoreEvent::Wifi { by, .. })
            | model::EventBody::Core(model::CoreEvent::Revoked { by, .. })
            | model::EventBody::Core(model::CoreEvent::ClockSync { by, .. })
            | model::EventBody::Feeder(model::FeederEvent::Fed { by, .. })
            | model::EventBody::Feeder(model::FeederEvent::Plans { by, .. }) => Some(by),
            _ => None,
        };
        let (mut client, mut plan) = source.map_or((None, None), source_detail);
        match &event.body {
            model::EventBody::Core(model::CoreEvent::Enrolled { client: named, .. }) => {
                client = Some(named.to_string())
            }
            model::EventBody::Feeder(model::FeederEvent::Missed { plan: slot, .. })
            | model::EventBody::Feeder(model::FeederEvent::MealSoon { plan: slot, .. }) => {
                plan = Some(*slot)
            }
            _ => {}
        }
        let locked = match event.body {
            model::EventBody::Feeder(model::FeederEvent::Lock { locked, .. }) => Some(locked),
            _ => None,
        };

        Self {
            seq: event.seq,
            at_ms: event.at.map(|at| at.as_millisecond()),
            kind: (&event.body).into(),
            fed: history::Feed::from_event(event).as_ref().map(Feed::from),
            locked,
            by: source.map(By::from),
            client,
            plan,
        }
    }
}
