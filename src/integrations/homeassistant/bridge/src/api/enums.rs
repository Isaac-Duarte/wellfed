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
    /// The role the feeder enrolled this client with, or through the
    /// owner's server the account's role there.
    Role {
        Owner => "owner",
        Member => "member",
        Server => "server",
        UpdateSigner => "update_signer",
        /// A role a newer build added: it grants nothing here.
        NewerBuild => "newer_build",
        /// Through the owner's server, an account that reads and changes
        /// nothing.
        Viewer => "viewer",
    }
}

impl From<model::ClientRole> for Role {
    fn from(role: model::ClientRole) -> Self {
        match role {
            model::ClientRole::Owner => Self::Owner,
            model::ClientRole::Member => Self::Member,
            model::ClientRole::Server => Self::Server,
            model::ClientRole::UpdateSigner => Self::UpdateSigner,
            model::ClientRole::Unknown => Self::NewerBuild,
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
        /// A value a newer build added, which this integration cannot name.
        NewerBuild => "newer_build",
    }
}

impl From<&model::Source> for By {
    fn from(source: &model::Source) -> Self {
        match source {
            model::Source::Panel => Self::Panel,
            model::Source::Client { .. } => Self::Client,
            model::Source::Feeder => Self::Feeder,
            model::Source::Schedule { .. } => Self::Plan,
            model::Source::Unknown => Self::NewerBuild,
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
        Invited => "invited",
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
            model::EventBody::Core(model::CoreEvent::Invited { .. }) => Self::Invited,
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
        /// The feeder runs a build from before API revisions, which the
        /// server sends nothing but an update: update the feeder.
        Legacy => "legacy",
        /// Another feed of this client's is still being answered or
        /// settled. A new feed waits for it (at most 15 minutes, the settle
        /// window), so the next `Fed` this client is named in can settle only
        /// that one, and a feed refused unseen is never taken for one that ran.
        Held => "held",
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
        /// A feed's answer never came: it is pending until a later update
        /// settles it, and comes before any update that does.
        Pending => "pending",
        /// A feed whose answer never came ran: its `Fed` event, or the
        /// server's journal, says how.
        Settled => "settled",
        /// A feed whose answer never came did not run: the feeder refused
        /// it, as the server's journal learned, or the journal it went to
        /// never had it, so it was never sent.
        SettledUnfed => "settled_unfed",
        /// A feed whose answer never came has no `Fed` event, or journal
        /// entry, this session could find yet: whether it ran is not known.
        /// It stays pending, and holds new feeds back until a later update
        /// settles it or the owner releases it ([`UpdateKind::Released`]).
        /// A lost feed from the other route comes so at each start: only
        /// the owner's release ends it there.
        StillUnknown => "still_unknown",
        /// The feeder no longer knows this client's key; the session will
        /// not reconnect.
        Revoked => "revoked",
        Disconnected => "disconnected",
        /// The server no longer takes this integration's token (revoked,
        /// expired, or its account disabled); the session will not
        /// reconnect until a new login.
        TokenRefused => "token_refused",
        /// The server no longer reaches the feeder for good: the feeder
        /// dropped the server's key, or the server no longer holds it.
        /// The owner adopts it again.
        ServerLostFeeder => "server_lost_feeder",
        /// The feeder's alerts moved: what is open, and what opened and
        /// cleared since.
        Alerts => "alerts",
        /// This client has the end of one more feed number: its write was
        /// answered, or its `Fed` settled a lost feed. `answered` lists
        /// every number to keep for the next session, where a `Fed` for one
        /// of them settles nothing, in place of those kept before.
        Answered => "answered",
        /// A feed whose answer never came, reported still unknown, waited
        /// as long as one is kept with no end found: no longer pending, and
        /// nothing settles it now.
        Forgotten => "forgotten",
        /// A direct feed about to go out, in `pending`: keep it as pending,
        /// with no repair yet. A restart before its answer finds it lost.
        Sending => "sending",
        /// A direct feed kept while it went out has its answer: no longer
        /// kept. A feed it ran comes as [`UpdateKind::Answered`] first.
        Cleared => "cleared",
        /// The owner released a lost feed, in `pending` with when: it holds
        /// new feeds back no longer, and stays pending, so a later update
        /// may still settle it, until [`UpdateKind::Forgotten`].
        Released => "released",
        /// The owner checked the bowls after the integration's store did
        /// not read: feeds go again.
        BowlsChecked => "bowls_checked",
    }
}

view_enum! {
    /// How the feeder's device API revision stands against this
    /// integration's. Either way the session reads and writes everything
    /// both know; this says which side to update for the rest.
    Skew {
        Same => "same",
        /// The feeder is older: what it lacks is refused until it is
        /// updated, from the app.
        FeederBehind => "feeder_behind",
        /// The feeder is newer: what it added reads as `newer_build` until
        /// this integration is updated.
        FeederAhead => "feeder_ahead",
        /// The feeder runs a build from before API revisions, which takes
        /// nothing this integration reads or writes: it must be updated,
        /// from the app, before the session can serve it.
        Legacy => "legacy",
        /// A value a newer build added, which this integration cannot name.
        NewerBuild => "newer_build",
        /// Through the owner's server: the server is older than the feeder
        /// or this integration, so update the server.
        ServerBehind => "server_behind",
        /// Through the owner's server: the server is newer than this
        /// integration, so update the integration.
        ServerAhead => "server_ahead",
    }
}

impl From<model::Skew> for Skew {
    fn from(skew: model::Skew) -> Self {
        match skew {
            model::Skew::Same => Self::Same,
            model::Skew::FeederBehind => Self::FeederBehind,
            model::Skew::FeederAhead => Self::FeederAhead,
            model::Skew::Legacy => Self::Legacy,
            model::Skew::Unknown => Self::NewerBuild,
        }
    }
}

impl From<wellfed_server_client::api::FeederRefusal> for RefusalKind {
    fn from(refusal: wellfed_server_client::api::FeederRefusal) -> Self {
        use wellfed_server_client::api::FeederRefusal as R;
        match refusal {
            R::Forbidden => Self::Forbidden,
            R::Busy => Self::Busy,
            R::Stale => Self::Stale,
            R::Unavailable => Self::Unavailable,
            R::Invalid => Self::Invalid,
            R::NotWritable => Self::NotWritable,
            R::Full => Self::Full,
            R::NotFound => Self::NotFound,
            R::Muted => Self::Muted,
            R::Time => Self::Time,
            R::Fault => Self::Fault,
            R::Portions => Self::Portions,
            R::NoClock => Self::NoClock,
            R::Unverified => Self::Unverified,
            R::Unknown => Self::NewerBuild,
        }
    }
}

view_enum! {
    /// An account's role on the owner's server.
    AccountRole {
        Owner => "owner",
        Member => "member",
        /// Reads, and changes nothing: Home Assistant refuses it at setup.
        Viewer => "viewer",
        /// A role a newer server added: it grants nothing here.
        NewerBuild => "newer_build",
    }
}

impl From<wellfed_server_client::api::Role> for Role {
    fn from(role: wellfed_server_client::api::Role) -> Self {
        use wellfed_server_client::api::Role as R;
        match role {
            R::Owner => Self::Owner,
            R::Member => Self::Member,
            R::Viewer => Self::Viewer,
            R::Unknown => Self::NewerBuild,
        }
    }
}

impl From<wellfed_server_client::api::Role> for AccountRole {
    fn from(role: wellfed_server_client::api::Role) -> Self {
        use wellfed_server_client::api::Role as R;
        match role {
            R::Owner => Self::Owner,
            R::Member => Self::Member,
            R::Viewer => Self::Viewer,
            R::Unknown => Self::NewerBuild,
        }
    }
}

view_enum! {
    /// Why redeeming an invite did not enroll Home Assistant's key, as the
    /// setup flow words it.
    JoinProblem {
        /// The feeder was not found, did not answer, or failed before the
        /// join went: nothing was enrolled.
        Unreachable => "unreachable",
        /// The feeder did not take the invite (unknown, expired or spent):
        /// the key is not enrolled.
        NotAccepted => "not_accepted",
        /// The join went, its answer was lost, and no session could check
        /// it: the key may be enrolled, so the flow keeps it and asks again
        /// before it spends another invite.
        Uncertain => "uncertain",
    }
}

view_enum! {
    /// Why a call to the owner's server at setup failed, as the flow words
    /// it.
    ServerProblem {
        /// Nothing answered at that address.
        Unreachable => "unreachable",
        /// Something answered, but not as a Wellfed server of this API.
        NotAServer => "not_a_server",
        /// The address redirects elsewhere: use the address it names.
        Redirected => "redirected",
        /// The name or password is wrong.
        BadCredentials => "bad_credentials",
        /// Too many failed logins: wait a minute.
        LoginLimited => "login_limited",
        /// The account is past its expiry.
        AccountDisabled => "account_disabled",
        /// The token no longer holds: log in again.
        Unauthorized => "unauthorized",
        /// The account's role may not do that.
        Forbidden => "forbidden",
        /// The server refused for another reason.
        Refused => "refused",
    }
}
