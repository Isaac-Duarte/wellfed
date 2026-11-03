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
    }
}

impl From<model::FoodLevel> for FoodLevel {
    fn from(level: model::FoodLevel) -> Self {
        match level {
            model::FoodLevel::Unread => Self::Unread,
            model::FoodLevel::Present => Self::Present,
            model::FoodLevel::Low => Self::Low,
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
    }
}

impl From<&model::EventBody> for EventKind {
    fn from(body: &model::EventBody) -> Self {
        use model::EventBody as B;
        match body {
            B::Boot { .. } => Self::Boot,
            B::Lock { .. } => Self::Lock,
            B::Settings { .. } => Self::Settings,
            B::Wifi { .. } => Self::Wifi,
            B::Enrolled { .. } => Self::Enrolled,
            B::Revoked { .. } => Self::Revoked,
            B::ClockSync { .. } => Self::ClockSync,
            B::Lost { .. } => Self::Lost,
            B::Fed { .. } => Self::Fed,
            B::FeedingBlocked { .. } => Self::FeedingBlocked,
            B::Plans { .. } => Self::Plans,
            B::Missed { .. } => Self::Missed,
            B::MealSoon { .. } => Self::MealSoon,
            B::Supply { .. } => Self::Supply,
            B::DispenseRefused { .. } => Self::DispenseRefused,
            B::Food { .. } => Self::Food,
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
        /// It never went out: no session was open.
        NotSent => "not_sent",
        /// It went out and no answer came: it may have run. Never resend it.
        Uncertain => "uncertain",
    }
}

view_enum! {
    /// What a session update carries.
    UpdateKind {
        Connected => "connected",
        Status => "status",
        Event => "event",
        /// Events this session never received: what happened meanwhile is
        /// not known.
        Gap => "gap",
        Disconnected => "disconnected",
    }
}
