//! The closed sets Home Assistant reads, each converted from its
//! `wellfed-model` enum by an exhaustive match, so a variant the model adds
//! fails this crate's build until it is named here.

use wellfed_model as model;

/// A closed set: the Rust enum, its Python class under the `python`
/// feature, and the snake_case `key` of each variant.
macro_rules! view_enum {
    (
        $(#[$meta:meta])*
        $name:ident { $($(#[$vmeta:meta])* $variant:ident => $key:literal),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[cfg_attr(
            feature = "python",
            pyo3::pyclass(eq, eq_int, hash, frozen, module = "wellfed_homeassistant")
        )]
        pub enum $name {
            $($(#[$vmeta])* $variant),+
        }

        impl $name {
            /// The snake_case name: an entity's state and its translation key.
            pub fn key(&self) -> &'static str {
                match self {
                    $(Self::$variant => $key),+
                }
            }
        }

        #[cfg(feature = "python")]
        #[pyo3::pymethods]
        impl $name {
            /// The snake_case name: an entity's state and its translation key.
            #[getter(key)]
            fn python_key(&self) -> &'static str {
                self.key()
            }
        }
    };
}

view_enum! {
    /// The role the feeder enrolled this client with.
    Role {
        Owner => "owner",
        Member => "member",
        Server => "server",
        UpdateSigner => "update_signer",
    }
}

impl From<model::ClientRole> for Role {
    fn from(role: model::ClientRole) -> Self {
        match role {
            model::ClientRole::Owner => Self::Owner,
            model::ClientRole::Member => Self::Member,
            model::ClientRole::Server => Self::Server,
            model::ClientRole::UpdateSigner => Self::UpdateSigner,
        }
    }
}

view_enum! {
    /// The hopper as the food beam last read it.
    FoodLevel {
        Unread => "unread",
        Present => "present",
        /// About five portions remain.
        Low => "low",
        /// A value a newer feeder build added, which this integration
        /// cannot name: update it.
        NewerBuild => "newer_build",
    }
}

impl From<model::FoodLevel> for FoodLevel {
    fn from(level: model::FoodLevel) -> Self {
        match level {
            model::FoodLevel::Unread => Self::Unread,
            model::FoodLevel::Present => Self::Present,
            model::FoodLevel::Low => Self::Low,
            model::FoodLevel::Unknown => Self::NewerBuild,
        }
    }
}

view_enum! {
    /// Whether a feed can start now, and if not why: the feeder's
    /// `Readiness` with its blocked reason flattened in, one state for an
    /// enum sensor.
    Readiness {
        Ready => "ready",
        Starting => "starting",
        Journal => "journal",
        Fault => "fault",
        Resolving => "resolving",
        Feeding => "feeding",
        Joining => "joining",
        Updating => "updating",
        Setup => "setup",
        Unserved => "unserved",
        Battery => "battery",
        Restarting => "restarting",
        /// A value a newer feeder build added, which this integration
        /// cannot name: update it.
        NewerBuild => "newer_build",
    }
}

impl From<model::Readiness> for Readiness {
    fn from(readiness: model::Readiness) -> Self {
        let model::Readiness::Blocked(blocked) = readiness else {
            return Self::Ready;
        };
        match blocked {
            model::Blocked::Starting => Self::Starting,
            model::Blocked::Journal => Self::Journal,
            model::Blocked::Fault => Self::Fault,
            model::Blocked::Resolving => Self::Resolving,
            model::Blocked::Feeding => Self::Feeding,
            model::Blocked::Joining => Self::Joining,
            model::Blocked::Updating => Self::Updating,
            model::Blocked::Setup => Self::Setup,
            model::Blocked::Unserved => Self::Unserved,
            model::Blocked::Battery => Self::Battery,
            model::Blocked::Restarting => Self::Restarting,
            model::Blocked::Unknown => Self::NewerBuild,
        }
    }
}

view_enum! {
    /// How a feed ended.
    FeedOutcome {
        Done => "done",
        Cancelled => "cancelled",
        Expired => "expired",
        Failed => "failed",
        /// It may have run: the feeder cannot tell how much.
        Uncertain => "uncertain",
        /// A value a newer feeder build added, which this integration
        /// cannot name: update it.
        NewerBuild => "newer_build",
    }
}

impl From<model::FeedOutcome> for FeedOutcome {
    fn from(outcome: model::FeedOutcome) -> Self {
        match outcome {
            model::FeedOutcome::Done => Self::Done,
            model::FeedOutcome::Cancelled => Self::Cancelled,
            model::FeedOutcome::Expired => Self::Expired,
            model::FeedOutcome::Failed => Self::Failed,
            model::FeedOutcome::Uncertain => Self::Uncertain,
            model::FeedOutcome::Unknown => Self::NewerBuild,
        }
    }
}

view_enum! {
    /// Why a feed ended as it did.
    FeedReason {
        Done => "done",
        Cancelled => "cancelled",
        Clock => "clock",
        ObservationGap => "observation_gap",
        Timeout => "timeout",
        NoFeedback => "no_feedback",
        Power => "power",
        Sensor => "sensor",
        OutletBlocked => "outlet_blocked",
        Jammed => "jammed",
        MarkStuck => "mark_stuck",
        Overcurrent => "overcurrent",
        FeedbackStream => "feedback_stream",
        Output => "output",
        Interrupted => "interrupted",
        /// A value a newer feeder build added, which this integration
        /// cannot name: update it.
        NewerBuild => "newer_build",
    }
}

impl From<model::FeedReason> for FeedReason {
    fn from(reason: model::FeedReason) -> Self {
        match reason {
            model::FeedReason::Done => Self::Done,
            model::FeedReason::Cancelled => Self::Cancelled,
            model::FeedReason::Clock => Self::Clock,
            model::FeedReason::ObservationGap => Self::ObservationGap,
            model::FeedReason::Timeout => Self::Timeout,
            model::FeedReason::NoFeedback => Self::NoFeedback,
            model::FeedReason::Power => Self::Power,
            model::FeedReason::Sensor => Self::Sensor,
            model::FeedReason::OutletBlocked => Self::OutletBlocked,
            model::FeedReason::Jammed => Self::Jammed,
            model::FeedReason::MarkStuck => Self::MarkStuck,
            model::FeedReason::Overcurrent => Self::Overcurrent,
            model::FeedReason::FeedbackStream => Self::FeedbackStream,
            model::FeedReason::Output => Self::Output,
            model::FeedReason::Interrupted => Self::Interrupted,
            model::FeedReason::Unknown => Self::NewerBuild,
        }
    }
}

view_enum! {
    /// Who asked for a feed or a change.
    By {
        Panel => "panel",
        /// A client; the event names its fingerprint.
        Client => "client",
        Feeder => "feeder",
        /// A feeding plan; the event names its slot.
        Plan => "plan",
    }
}

impl From<&model::Source> for By {
    fn from(source: &model::Source) -> Self {
        match source {
            model::Source::Panel => Self::Panel,
            model::Source::Client { .. } => Self::Client,
            model::Source::Feeder => Self::Feeder,
            model::Source::Schedule { .. } => Self::Plan,
        }
    }
}

view_enum! {
    /// What an event records.
    EventKind {
        Boot => "boot",
        Lock => "lock",
        Settings => "settings",
        Wifi => "wifi",
        Enrolled => "enrolled",
        Revoked => "revoked",
        ClockSync => "clock_sync",
        Lost => "lost",
        Fed => "fed",
        FeedingBlocked => "feeding_blocked",
        Plans => "plans",
        Missed => "missed",
        MealSoon => "meal_soon",
        Supply => "supply",
        DispenseRefused => "dispense_refused",
        Food => "food",
        Skipped => "skipped",
        Unskipped => "unskipped",
        UpdateStop => "update_stop",
        UpdateAtBoot => "update_at_boot",
        Restarting => "restarting",
        UpdateConfirmed => "update_confirmed",
    }
}

impl From<&model::EventBody> for EventKind {
    fn from(body: &model::EventBody) -> Self {
        match body {
            model::EventBody::Core(model::CoreEvent::Boot { .. }) => Self::Boot,
            model::EventBody::Feeder(model::FeederEvent::Lock { .. }) => Self::Lock,
            model::EventBody::Feeder(model::FeederEvent::Settings { .. }) => Self::Settings,
            model::EventBody::Core(model::CoreEvent::Wifi { .. }) => Self::Wifi,
            model::EventBody::Core(model::CoreEvent::Enrolled { .. }) => Self::Enrolled,
            model::EventBody::Core(model::CoreEvent::Revoked { .. }) => Self::Revoked,
            model::EventBody::Core(model::CoreEvent::ClockSync { .. }) => Self::ClockSync,
            model::EventBody::Core(model::CoreEvent::Lost { .. }) => Self::Lost,
            model::EventBody::Feeder(model::FeederEvent::Fed { .. }) => Self::Fed,
            model::EventBody::Feeder(model::FeederEvent::FeedingBlocked { .. }) => {
                Self::FeedingBlocked
            }
            model::EventBody::Feeder(model::FeederEvent::Plans { .. }) => Self::Plans,
            model::EventBody::Feeder(model::FeederEvent::Missed { .. }) => Self::Missed,
            model::EventBody::Feeder(model::FeederEvent::MealSoon { .. }) => Self::MealSoon,
            model::EventBody::Feeder(model::FeederEvent::Supply { .. }) => Self::Supply,
            model::EventBody::Feeder(model::FeederEvent::DispenseRefused { .. }) => {
                Self::DispenseRefused
            }
            model::EventBody::Feeder(model::FeederEvent::Food { .. }) => Self::Food,
            model::EventBody::Feeder(model::FeederEvent::Skipped { .. }) => Self::Skipped,
            model::EventBody::Feeder(model::FeederEvent::Unskipped { .. }) => Self::Unskipped,
            model::EventBody::Core(model::CoreEvent::UpdateStop { .. }) => Self::UpdateStop,
            model::EventBody::Core(model::CoreEvent::UpdateAtBoot { .. }) => Self::UpdateAtBoot,
            model::EventBody::Core(model::CoreEvent::Restarting) => Self::Restarting,
            model::EventBody::Core(model::CoreEvent::UpdateConfirmed) => Self::UpdateConfirmed,
        }
    }
}

view_enum! {
    /// Why the feeder refused a write.
    RefusalKind {
        Forbidden => "forbidden",
        Stale => "stale",
        Busy => "busy",
        Unavailable => "unavailable",
        Invalid => "invalid",
        NotWritable => "not_writable",
        Full => "full",
        NotFound => "not_found",
        Muted => "muted",
        Time => "time",
        Fault => "fault",
        Update => "update",
        Unverified => "unverified",
        Portions => "portions",
        NoClock => "no_clock",
        /// A value a newer feeder build added, which this integration
        /// cannot name: update it.
        NewerBuild => "newer_build",
    }
}

impl From<&model::Refusal> for RefusalKind {
    fn from(refusal: &model::Refusal) -> Self {
        use model::Refusal as R;
        match refusal {
            R::Forbidden => Self::Forbidden,
            R::Stale { .. } => Self::Stale,
            R::Busy => Self::Busy,
            R::Unavailable => Self::Unavailable,
            R::Invalid => Self::Invalid,
            R::NotWritable => Self::NotWritable,
            R::Full => Self::Full,
            R::NotFound => Self::NotFound,
            R::Muted => Self::Muted,
            R::Time => Self::Time,
            R::Fault => Self::Fault,
            R::Update(_) => Self::Update,
            R::Unverified => Self::Unverified,
            R::Portions { .. } => Self::Portions,
            R::NoClock => Self::NoClock,
            R::Unknown => Self::NewerBuild,
        }
    }
}

view_enum! {
    /// How a write ended.
    WriteResult {
        /// A feed is reserved; its `Fed` event carries the result.
        Feeding => "feeding",
        /// The write took effect.
        Done => "done",
        Refused => "refused",
        /// It never went out; `not_sent` says why.
        NotSent => "not_sent",
        /// It went out and no answer came: it may have run. Never resend it.
        Uncertain => "uncertain",
    }
}

view_enum! {
    /// Why a write never went out. Each is safe to try again: the feeder
    /// saw nothing of it.
    NotSent {
        /// No session is open now.
        Disconnected => "disconnected",
        /// The session has stopped for good: its handle is gone, or the
        /// feeder revoked this client.
        Stopped => "stopped",
        /// A change to the same store is still waiting on its answer.
        Busy => "busy",
        /// This client's role may not do that.
        Forbidden => "forbidden",
        /// The feeder's firmware does not serve it.
        Unsupported => "unsupported",
        /// What the change is based on has not been read yet.
        NotRead => "not_read",
        /// A value outside what the feeder accepts.
        Invalid => "invalid",
    }
}

view_enum! {
    /// A tune the feeder plays on request.
    Tune {
        Chime => "chime",
        Meal => "meal",
        Coin => "coin",
        Alert => "alert",
        Test => "test",
        Twinkle => "twinkle",
    }
}

impl From<Tune> for model::Tune {
    fn from(tune: Tune) -> Self {
        match tune {
            Tune::Chime => Self::Chime,
            Tune::Meal => Self::Meal,
            Tune::Coin => Self::Coin,
            Tune::Alert => Self::Alert,
            Tune::Test => Self::Test,
            Tune::Twinkle => Self::Twinkle,
        }
    }
}

view_enum! {
    /// What a session update carries.
    UpdateKind {
        /// A session opened: its build, role and capabilities.
        Connected => "connected",
        /// What the session knows of the feeder changed.
        State => "state",
        /// An event; `replayed` when it came from the history read at
        /// connect rather than as it happened.
        Event => "event",
        /// Events this session never received: what happened meanwhile is
        /// not known.
        Gap => "gap",
        /// A feed whose answer never came has its `Fed` event.
        Settled => "settled",
        /// A feed whose answer never came has no `Fed` event this session
        /// could find: whether it ran is not known.
        StillUnknown => "still_unknown",
        /// The feeder no longer knows this client's key; the session will
        /// not reconnect.
        Revoked => "revoked",
        Disconnected => "disconnected",
        /// The feeder runs a different build than this integration, which
        /// takes nothing but an update; `build` names it. This session
        /// will not reconnect: the integration sets up a new one, retrying
        /// until the feeder is updated.
        OtherBuild => "other_build",
    }
}
