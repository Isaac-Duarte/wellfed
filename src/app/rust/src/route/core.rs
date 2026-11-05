//! The router's rules with no IO: which path a new request takes, what an
//! attempt's end means, when a feed may start, and what settles an uncertain
//! write. [`super::io`] drives requests around these and keeps the [`Book`]
//! on disk, so every rule here is tested without a socket.
//!
//! A write goes out once. Nothing here ever plans a second send of a key:
//! resolving an uncertain write only reads, the server's journal for one
//! pinned to the server ([`RetryPlan::ReadJournal`]) and the feeder's
//! history for one pinned to the phone's own session
//! ([`RetryPlan::ReadHistory`]). The server dedups a key only while its
//! journal holds it, and it holds none for a write it never recorded (a lost
//! `feeder_offline`, a failure before the insert, a request that died on the
//! way), so a resend could be a fresh feed after the owner fed again.

use uuid::Uuid;
use wellfed_api::{CommandOutcome, CommandState};

use crate::api::router::{RoutePath, Settlement};
use crate::route::pins::{PinRecord, PinState, PinnedWrite, RunningFeed, ServerMark};

/// How long a server that failed to connect is passed over before a new
/// request tries it again. The stream reopening or any call answering marks
/// it up sooner.
pub(crate) const RECHECK_MS: i64 = 30_000;

/// How long after its last attempt a server write the journal does not hold
/// is taken as never sent. The server records a write before it sends it,
/// so an entry it never made was never sent; the wait covers a request
/// still on its way to a server that is slow to take it, well past the
/// phone's own 30 s request timeout.
pub(crate) const UNFOUND_GRACE_MS: i64 = 10 * 60_000;

/// How long a pin the owner put away is kept, from the acknowledgement, for
/// its path to settle it. One still pending holds forever.
pub(crate) const KEEP_MS: i64 = 7 * 24 * 3_600_000;

/// What this phone may use to reach a feeder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Access {
    /// Not on the server (or no server): the phone's own key alone.
    DirectOnly,
    /// On the server, with no key of the phone's own to use: a viewer, an
    /// account with an expiry, or a phone that never paired with it.
    ServerOnly,
    /// On the server, and the phone holds its own key.
    Both,
}

/// Whether the logged-in account may use the phone's own keys. Viewers and
/// accounts with an expiry hold none: the server alone enforces their
/// limits ([two paths](../../../../docs/server/README.md#two-paths-one-feed)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Keys {
    Own,
    ServerOnly,
}

impl Keys {
    /// The keys the account may use while the server is `reach`. A server
    /// that refuses this phone's token (revoked, or the account disabled)
    /// closes the phone's own keys too: the server took this phone's
    /// standing away, so the router does not route around it on the LAN.
    /// Logging in again, or removing the server, opens them again.
    pub(crate) const fn under(self, reach: ServerReach) -> Self {
        match reach {
            ServerReach::Refused => Self::ServerOnly,
            ServerReach::Up | ServerReach::Down { .. } => self,
        }
    }
}

impl Access {
    /// The access to a feeder the server lists (`on_server`) and the phone
    /// holds a session for (`direct`); `None` when it may use neither. An
    /// account that holds no key of its own never uses one, even to a
    /// feeder the server does not list.
    pub(crate) const fn of(on_server: bool, direct: bool, keys: Keys) -> Option<Self> {
        match (on_server, direct, keys) {
            (true, true, Keys::Own) => Some(Self::Both),
            (true, _, _) => Some(Self::ServerOnly),
            (false, true, Keys::Own) => Some(Self::DirectOnly),
            (false, true, Keys::ServerOnly) | (false, false, _) => None,
        }
    }
}

/// The server as the phone last found it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ServerReach {
    Up,
    /// A connection failed at `since_ms` (the phone's clock).
    Down {
        since_ms: i64,
    },
    /// It answers, but no longer for this phone's token (revoked, expired,
    /// or the account disabled): the server path is closed, not down, and
    /// the phone's own keys close with it ([`Keys::under`]).
    Refused,
}

impl ServerReach {
    /// Whether a new request should try the server at `now_ms`.
    pub(crate) const fn worth_trying(self, now_ms: i64) -> bool {
        match self {
            Self::Up => true,
            Self::Down { since_ms } => now_ms.saturating_sub(since_ms) >= RECHECK_MS,
            Self::Refused => false,
        }
    }
}

/// What the phone knows of the two paths to one feeder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Reach {
    pub(crate) server: ServerReach,
    /// The server's link to the feeder: `Some(false)` when the server
    /// reports it offline (or still connecting); `None` before it has said.
    pub(crate) online_on_server: Option<bool>,
    /// The phone's own session with the feeder is open.
    pub(crate) direct_open: bool,
}

/// The path a new read or a new write takes: server first. It goes direct
/// when the server is unreachable, or reports the feeder offline while the
/// phone's own session reaches it. With neither path working it still tries
/// the server, whose failure then says why.
pub(crate) const fn choose(access: Access, reach: Reach, now_ms: i64) -> RoutePath {
    match access {
        Access::DirectOnly => RoutePath::Direct,
        Access::ServerOnly => RoutePath::Server,
        Access::Both => {
            let server_has_it =
                reach.server.worth_trying(now_ms) && !matches!(reach.online_on_server, Some(false));
            if !server_has_it && reach.direct_open {
                RoutePath::Direct
            } else {
                RoutePath::Server
            }
        }
    }
}

/// How one call to the server failed, as the router tells them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ServerFailure {
    /// No connection to the server: nothing left the phone.
    Connect,
    /// Sent, and the answer was cut off, timed out or could not be read,
    /// or the server failed inside: it may have run.
    Interrupted,
    /// `504 outcome_unknown`: the journal keeps it unknown.
    OutcomeUnknown,
    /// `503 feeder_offline`: the server's feeder task never took it, and a
    /// first attempt's entry is dropped. Nothing ran.
    FeederOffline,
    /// The feeder refused it (`409 feeder_refused`): final for the key.
    FeederRefused,
    /// A settings or plans write whose base the store moved past: the
    /// feeder refused it, and nothing ran.
    Stale,
    /// The key already names another request: nothing was sent.
    KeyReused,
    /// The phone's token no longer holds (`401`), or its account is
    /// disabled: nothing was sent, and the server is closed to this phone.
    Unauthorized,
    /// The server refused before sending anything: forbidden, invalid, a
    /// redirect.
    Rejected,
}

/// How one attempt on a path ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ended {
    /// The feeder answered, or the server answered for it.
    Answered,
    /// Refused, and nothing ran.
    Refused,
    /// Definitely never sent: safe to send on another path.
    Unsent,
    /// It may have run.
    Uncertain,
}

/// What a failed server call says of the write it carried, its only send.
pub(crate) const fn server_ended(failure: ServerFailure) -> Ended {
    match failure {
        ServerFailure::Connect | ServerFailure::FeederOffline => Ended::Unsent,
        ServerFailure::FeederRefused
        | ServerFailure::Stale
        | ServerFailure::KeyReused
        | ServerFailure::Unauthorized
        | ServerFailure::Rejected => Ended::Refused,
        ServerFailure::Interrupted | ServerFailure::OutcomeUnknown => Ended::Uncertain,
    }
}

/// What a failure says of the server itself.
pub(crate) const fn reach_after(failure: ServerFailure, now_ms: i64) -> ServerReach {
    match failure {
        ServerFailure::Connect => ServerReach::Down { since_ms: now_ms },
        ServerFailure::Unauthorized => ServerReach::Refused,
        _ => ServerReach::Up,
    }
}

/// Where an intent goes after an attempt that ended `ended` on `path`: done,
/// or once more on the phone's own session when the server definitely did
/// not send it. Only a definite non-send moves; an uncertain write never
/// does.
pub(crate) const fn fallback(
    path: RoutePath,
    ended: Ended,
    access: Access,
    direct_open: bool,
    fell_back: bool,
) -> Option<RoutePath> {
    match (path, ended, access) {
        (RoutePath::Server, Ended::Unsent, Access::Both) if direct_open && !fell_back => {
            Some(RoutePath::Direct)
        }
        _ => None,
    }
}

/// How an uncertain write is asked what became of it: only ever by
/// reading, on its own path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RetryPlan {
    /// `GET /commands/{key}` on the server it went to.
    ReadJournal,
    /// The feeder's history, as the phone's own session reads it.
    ReadHistory,
}

pub(crate) const fn retry_plan(pin: &PinRecord) -> RetryPlan {
    match pin.path {
        RoutePath::Server => RetryPlan::ReadJournal,
        RoutePath::Direct => RetryPlan::ReadHistory,
    }
}

/// What a journal entry, as the server holds it now, settles of a write
/// pinned to the server; `None` while it is still unknown or in flight.
pub(crate) fn settled_by_command(
    state: CommandState,
    outcome: Option<&CommandOutcome>,
) -> Option<Settlement> {
    match state {
        CommandState::Sent | CommandState::Unknown => None,
        CommandState::Answered | CommandState::Resolved => outcome.map(settlement_of),
    }
}

fn settlement_of(outcome: &CommandOutcome) -> Settlement {
    match outcome {
        CommandOutcome::Done
        | CommandOutcome::Settings { .. }
        | CommandOutcome::Plans { .. }
        | CommandOutcome::Calibration { .. }
        | CommandOutcome::Call { .. }
        | CommandOutcome::Invited { .. } => Settlement::Done,
        CommandOutcome::Superseded => Settlement::Superseded,
        CommandOutcome::Refused { refusal } => Settlement::Refused {
            refusal: (*refusal).into(),
        },
        CommandOutcome::Feeding { feed } => Settlement::Started { feed: *feed },
        CommandOutcome::Fed {
            feed,
            requested,
            completed,
            outcome,
            reason,
        } => Settlement::Fed {
            feed: *feed,
            requested: *requested,
            completed: *completed,
            outcome: *outcome,
            reason: *reason,
        },
        CommandOutcome::Unrecorded => Settlement::Unrecorded,
    }
}

/// A server write the journal does not hold settles as never sent, once its
/// last attempt is `grace_ms` old, and only when the journal that does not
/// hold it is the one it went to: the same token, so the same server, and
/// the same account, since another's entries are not found either.
pub(crate) fn settled_unfound(
    pin: &PinRecord,
    asked: Option<ServerMark>,
    now_ms: i64,
    grace_ms: i64,
) -> Option<Settlement> {
    let same = pin.server.is_some() && pin.server == asked;
    (same && now_ms.saturating_sub(pin.tried_ms) >= grace_ms).then_some(Settlement::NotSent)
}

/// A feed's end this phone's own session read from the feeder's history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Seen {
    pub(crate) seq: u64,
    pub(crate) feed: u32,
    pub(crate) requested: u8,
    pub(crate) completed: u8,
    pub(crate) outcome: wellfed_model::FeedOutcome,
    pub(crate) reason: wellfed_model::FeedReason,
}

/// What the feeds this phone's key asked for, as the feeder's history
/// records them (`seen`), settle of `pin`, a feed pinned to the phone's own
/// session. Its `Fed` is the one event after the newest the session knew
/// when it was sent, for the portions it asked, and not a feed this phone
/// saw started (before or since). Anything another unsettled direct feed on
/// the feeder could also claim settles neither: an uncertain feed is never
/// given another's end.
pub(crate) fn settled_by_history(
    pin: &PinRecord,
    others: &[&PinRecord],
    seen: &[Seen],
) -> Option<Settlement> {
    let PinnedWrite::Feed { portions } = pin.write else {
        return None;
    };
    let mark = pin.mark?;
    let mut candidates = seen.iter().filter(|seen| {
        seen.seq > mark && seen.requested == portions && !pin.excluded.contains(&seen.feed)
    });
    let found = candidates.next()?;
    if candidates.next().is_some() {
        return None;
    }

    let claimed = others.iter().any(|other| {
        other.key != pin.key
            && other.feeder == pin.feeder
            && other.path == RoutePath::Direct
            && matches!(other.write, PinnedWrite::Feed { portions } if portions == found.requested)
            && !other.excluded.contains(&found.feed)
            && other.mark.is_none_or(|mark| found.seq > mark)
    });
    if claimed {
        return None;
    }
    Some(Settlement::Fed {
        feed: found.feed,
        requested: found.requested,
        completed: found.completed,
        outcome: found.outcome,
        reason: found.reason,
    })
}

/// The pinned writes and this phone's running direct feeds, as the router
/// keeps them on disk.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Book {
    pub(crate) pins: Vec<PinRecord>,
    /// Direct feeds this phone saw the feeder reserve, whose `Fed` it has
    /// not read yet: none is an uncertain feed's end.
    pub(crate) running: Vec<RunningFeed>,
}

impl Book {
    /// The feed pending on `feeder` that holds a new one back: sent and not
    /// yet ended, or uncertain and not yet put away by the owner. A new feed
    /// may follow an uncertain one only once the owner has looked at the
    /// bowl and closed it, as the direct path's feed sheet has always asked,
    /// and that holds whichever path either goes on.
    pub(crate) fn holding_feeds(&self, feeder: &str) -> Option<&PinRecord> {
        self.pins.iter().find(|pin| {
            pin.feeder == feeder
                && matches!(pin.write, PinnedWrite::Feed { .. })
                && (pin.state == PinState::Sending || pin.acknowledged_ms.is_none())
        })
    }

    pub(crate) fn get(&self, key: Uuid) -> Option<&PinRecord> {
        self.pins.iter().find(|pin| pin.key == key)
    }

    pub(crate) fn get_mut(&mut self, key: Uuid) -> Option<&mut PinRecord> {
        self.pins.iter_mut().find(|pin| pin.key == key)
    }

    /// The direct feeds running on `feeder` at `now_ms`, which a new direct
    /// feed's pin never takes an end from.
    pub(crate) fn running_on(&self, feeder: &str, now_ms: i64) -> Vec<u32> {
        self.running
            .iter()
            .filter(|running| running.feeder == feeder && running.until_ms > now_ms)
            .map(|running| running.feed)
            .collect()
    }

    /// A write about to go, written down before it is sent, so a phone that
    /// stops mid-send still finds it pinned. A feed is refused, with the key
    /// of the feed that holds it back, while another feed on its feeder
    /// does. A direct feed's pin takes no end from a feed already running.
    pub(crate) fn admit(&mut self, pin: PinRecord, now_ms: i64) -> Result<(), Uuid> {
        if matches!(pin.write, PinnedWrite::Feed { .. })
            && let Some(holding) = self.holding_feeds(&pin.feeder)
        {
            return Err(holding.key);
        }
        let mut pin = pin;
        if pin.path == RoutePath::Direct {
            pin.excluded.extend(self.running_on(&pin.feeder, now_ms));
        }
        self.pins.push(pin);
        Ok(())
    }

    /// An attempt ended `ended`. A write that may have run stays pinned to
    /// the path it went on, uncertain. Any other end takes it out. Whether
    /// the end changed the book.
    pub(crate) fn ended(&mut self, key: Uuid, ended: Ended, now_ms: i64) -> bool {
        match ended {
            Ended::Uncertain => match self.get_mut(key) {
                Some(held) => {
                    held.state = PinState::Uncertain;
                    held.tried_ms = now_ms;
                    true
                }
                None => false,
            },
            Ended::Answered | Ended::Refused | Ended::Unsent => self.settle(key),
        }
    }

    /// A write the server definitely did not send goes on the phone's own
    /// session instead: its pin, still sending, names that path and the
    /// newest event the session knows, before it goes. Only a pin still
    /// sending moves; an uncertain one never does. Whether it moved.
    pub(crate) fn moved(&mut self, key: Uuid, mark: Option<u64>, now_ms: i64) -> bool {
        let running = self
            .get(key)
            .map(|pin| self.running_on(&pin.feeder, now_ms))
            .unwrap_or_default();
        match self.get_mut(key) {
            Some(pin) if pin.state == PinState::Sending && pin.path == RoutePath::Server => {
                pin.path = RoutePath::Direct;
                pin.mark = mark;
                pin.tried_ms = now_ms;
                pin.server = None;
                pin.excluded.extend(running);
                true
            }
            _ => false,
        }
    }

    /// Take `key` out; whether it was there.
    pub(crate) fn settle(&mut self, key: Uuid) -> bool {
        let before = self.pins.len();
        self.pins.retain(|pin| pin.key != key);
        self.pins.len() != before
    }

    /// The owner looked and put `key` away at `now_ms`: it no longer holds
    /// a new feed back, and it stays pinned to its path until it settles.
    /// One nothing can settle goes: a direct write other than a feed (the
    /// phone's own session never sends it again and no history names it),
    /// or a direct feed sent before its session knew any event. Whether it
    /// changed.
    pub(crate) fn acknowledge(&mut self, key: Uuid, now_ms: i64) -> bool {
        let Some(pin) = self.get_mut(key) else {
            return false;
        };
        if pin.acknowledged_ms.is_some() || pin.state != PinState::Uncertain {
            return false;
        }
        let settles = match pin.path {
            RoutePath::Server => true,
            RoutePath::Direct => {
                matches!(pin.write, PinnedWrite::Feed { .. }) && pin.mark.is_some()
            }
        };
        if settles {
            pin.acknowledged_ms = Some(now_ms);
        } else {
            self.settle(key);
        }
        true
    }

    /// Put away pins the owner acknowledged more than [`KEEP_MS`] before
    /// `now_ms`, and running feeds past the time they are kept. Whether any went.
    pub(crate) fn expire(&mut self, now_ms: i64) -> bool {
        let before = (self.pins.len(), self.running.len());
        self.pins.retain(|pin| {
            pin.acknowledged_ms
                .is_none_or(|at| now_ms.saturating_sub(at) < KEEP_MS)
        });
        self.running.retain(|running| running.until_ms > now_ms);
        before != (self.pins.len(), self.running.len())
    }

    /// The feeder reserved this phone's direct feed `feed`, which may run
    /// until `until_ms`: no uncertain direct feed on it, now or sent while
    /// it runs, may take that one's end for its own.
    pub(crate) fn started_direct(&mut self, feeder: &str, feed: u32, until_ms: i64) {
        for pin in &mut self.pins {
            if pin.feeder == feeder
                && pin.path == RoutePath::Direct
                && matches!(pin.write, PinnedWrite::Feed { .. })
                && !pin.excluded.contains(&feed)
            {
                pin.excluded.push(feed);
            }
        }
        if !self
            .running
            .iter()
            .any(|running| running.feeder == feeder && running.feed == feed)
        {
            self.running.push(RunningFeed {
                feeder: feeder.to_owned(),
                feed,
                until_ms,
            });
        }
    }

    /// The history gave the ends of `feeds` on `feeder`: they no longer run.
    /// Whether any did.
    pub(crate) fn ended_direct(&mut self, feeder: &str, feeds: &[u32]) -> bool {
        let before = self.running.len();
        self.running
            .retain(|running| running.feeder != feeder || !feeds.contains(&running.feed));
        self.running.len() != before
    }
}

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
