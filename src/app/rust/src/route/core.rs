//! The router's rules with no IO: which path a new request takes, what an
//! attempt's end means, when a feed may start, and what settles an uncertain
//! write. [`super::io`] drives requests around these and keeps the [`Book`]
//! on disk, so every rule here is tested without a socket.

use uuid::Uuid;
use wellfed_api::{CommandOutcome, CommandState};

use crate::api::router::{RoutePath, Settlement};
use crate::route::pins::{PinRecord, PinState, PinnedWrite};

/// How long a server that failed to connect is passed over before a new
/// request tries it again. The stream reopening or any call answering marks
/// it up sooner.
pub(crate) const RECHECK_MS: i64 = 30_000;

/// How long after its last attempt a server write the journal does not hold
/// is taken as never sent. The server records a write before it sends it,
/// so an entry it never made was never sent; the wait covers a request still
/// on its way to a server that is slow to take it.
pub(crate) const UNFOUND_GRACE_MS: i64 = 60_000;

/// How long a pin the owner put away is kept for its path to settle it.
/// One still pending holds forever.
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

impl Access {
    /// The access to a feeder the server lists (`on_server`) and the phone
    /// holds a session for (`direct`); `None` when the phone has neither.
    pub(crate) const fn of(on_server: bool, direct: bool, keys: Keys) -> Option<Self> {
        match (on_server, direct, keys) {
            (true, true, Keys::Own) => Some(Self::Both),
            (true, _, _) => Some(Self::ServerOnly),
            (false, true, _) => Some(Self::DirectOnly),
            (false, false, _) => None,
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
}

impl ServerReach {
    /// Whether a new request should try the server at `now_ms`.
    pub(crate) const fn worth_trying(self, now_ms: i64) -> bool {
        match self {
            Self::Up => true,
            Self::Down { since_ms } => now_ms.saturating_sub(since_ms) >= RECHECK_MS,
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
    /// The server has no session with the feeder, or could not reach it.
    /// On a first attempt the server forgets the entry, so nothing ran; on
    /// a retry of an unknown write it says nothing of the first.
    FeederOffline,
    /// The feeder refused it (`409 feeder_refused`): final for the key,
    /// and replayed to a retry.
    FeederRefused,
    /// A settings or plans write whose base the store moved past. On a
    /// first attempt nothing ran; on a retry the first may have made the
    /// very change that moved it.
    Stale,
    /// The key already names another request: nothing was sent.
    KeyReused,
    /// The server refused before sending anything: forbidden, invalid, a
    /// token that no longer holds, a redirect.
    Rejected,
}

/// Whether this is the key's first attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Attempt {
    First,
    Retry,
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
    /// A retry found the store moved past the write's base, which the
    /// write itself may have done: final, and the store is to be read.
    Superseded,
}

/// What a failed server call says of the write it carried.
pub(crate) const fn server_ended(failure: ServerFailure, attempt: Attempt) -> Ended {
    match (failure, attempt) {
        (ServerFailure::FeederRefused, _) => Ended::Refused,
        (ServerFailure::Stale, Attempt::First) => Ended::Refused,
        (ServerFailure::Stale, Attempt::Retry) => Ended::Superseded,
        (ServerFailure::Connect, Attempt::First)
        | (ServerFailure::FeederOffline, Attempt::First) => Ended::Unsent,
        (ServerFailure::KeyReused | ServerFailure::Rejected, Attempt::First) => Ended::Refused,
        // A retry that fails any other way says nothing of the first send.
        (ServerFailure::Connect, Attempt::Retry)
        | (ServerFailure::FeederOffline, Attempt::Retry)
        | (ServerFailure::KeyReused | ServerFailure::Rejected, Attempt::Retry)
        | (ServerFailure::Interrupted | ServerFailure::OutcomeUnknown, _) => Ended::Uncertain,
    }
}

/// Whether a failure says the server itself is out of reach.
pub(crate) const fn server_down(failure: ServerFailure) -> bool {
    matches!(failure, ServerFailure::Connect)
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
        | CommandOutcome::Invited { .. } => Settlement::Done,
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
/// last attempt is [`UNFOUND_GRACE_MS`] old.
pub(crate) const fn settled_unfound(pin: &PinRecord, now_ms: i64) -> Option<Settlement> {
    if now_ms.saturating_sub(pin.tried_ms) >= UNFOUND_GRACE_MS {
        Some(Settlement::NotSent)
    } else {
        None
    }
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
/// when it was sent, not a feed this phone heard answered since. Anything
/// another unsettled direct feed on the feeder could also claim settles
/// neither: an uncertain feed is never given another's end.
pub(crate) fn settled_by_history(
    pin: &PinRecord,
    others: &[&PinRecord],
    seen: &[Seen],
) -> Option<Settlement> {
    let PinnedWrite::Feed { .. } = pin.write else {
        return None;
    };
    let mark = pin.mark?;
    let mut candidates = seen
        .iter()
        .filter(|seen| seen.seq > mark && !pin.excluded.contains(&seen.feed));
    let found = candidates.next()?;
    if candidates.next().is_some() {
        return None;
    }

    let claimed = others.iter().any(|other| {
        other.key != pin.key
            && other.feeder == pin.feeder
            && other.path == RoutePath::Direct
            && matches!(other.write, PinnedWrite::Feed { .. })
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

/// The pinned writes, as the router keeps them on disk.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Book {
    pub(crate) pins: Vec<PinRecord>,
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
                && (pin.state == PinState::Sending || !pin.acknowledged)
        })
    }

    pub(crate) fn get(&self, key: Uuid) -> Option<&PinRecord> {
        self.pins.iter().find(|pin| pin.key == key)
    }

    pub(crate) fn get_mut(&mut self, key: Uuid) -> Option<&mut PinRecord> {
        self.pins.iter_mut().find(|pin| pin.key == key)
    }

    /// A feed about to go: refused, with the key of the feed that holds it
    /// back, while another feed on its feeder does, else written down before
    /// it is sent, so a phone that stops mid-send still finds it pinned.
    pub(crate) fn admit(&mut self, pin: PinRecord) -> Result<(), Uuid> {
        if let Some(holding) = self.holding_feeds(&pin.feeder) {
            return Err(holding.key);
        }
        self.pins.push(pin);
        Ok(())
    }

    /// An attempt ended `ended`. A write that may have run is pinned to the
    /// path it went on: `pin` is written down as uncertain, or the pin it
    /// already has keeps everything but its state and its last attempt. Any
    /// other end takes it out. Whether the end changed the book.
    pub(crate) fn ended(&mut self, pin: PinRecord, ended: Ended) -> bool {
        match ended {
            Ended::Uncertain => match self.get_mut(pin.key) {
                Some(held) => {
                    let changed =
                        held.state != PinState::Uncertain || held.tried_ms != pin.tried_ms;
                    held.state = PinState::Uncertain;
                    held.tried_ms = pin.tried_ms;
                    changed
                }
                None => {
                    self.pins.push(PinRecord {
                        state: PinState::Uncertain,
                        ..pin
                    });
                    true
                }
            },
            Ended::Answered | Ended::Refused | Ended::Unsent | Ended::Superseded => {
                self.settle(pin.key)
            }
        }
    }

    /// A feed the server definitely did not send goes on the phone's own
    /// session instead: its pin, still sending, names that path and the
    /// newest event the session knows, before it goes. Only a pin still
    /// sending moves; an uncertain one never does. Whether it moved.
    pub(crate) fn moved(&mut self, key: Uuid, mark: Option<u64>, now_ms: i64) -> bool {
        match self.get_mut(key) {
            Some(pin) if pin.state == PinState::Sending && pin.path == RoutePath::Server => {
                pin.path = RoutePath::Direct;
                pin.mark = mark;
                pin.tried_ms = now_ms;
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

    /// The owner looked and put `key` away: it no longer holds a new feed
    /// back, and it stays pinned to its path until it settles. One nothing
    /// can settle goes: a direct write other than a feed (the phone's own
    /// session never sends it again and no history names it), or a direct
    /// feed sent before its session knew any event. Whether it changed.
    pub(crate) fn acknowledge(&mut self, key: Uuid) -> bool {
        let Some(pin) = self.get_mut(key) else {
            return false;
        };
        if pin.acknowledged || pin.state != PinState::Uncertain {
            return false;
        }
        let settles = match pin.path {
            RoutePath::Server => true,
            RoutePath::Direct => {
                matches!(pin.write, PinnedWrite::Feed { .. }) && pin.mark.is_some()
            }
        };
        if settles {
            pin.acknowledged = true;
        } else {
            self.settle(key);
        }
        true
    }

    /// Put away pins the owner acknowledged more than [`KEEP_MS`] before
    /// `now_ms`: their own path has had its chance to settle them. Whether
    /// any went.
    pub(crate) fn expire(&mut self, now_ms: i64) -> bool {
        let before = self.pins.len();
        self.pins
            .retain(|pin| !pin.acknowledged || now_ms.saturating_sub(pin.sent_ms) < KEEP_MS);
        self.pins.len() != before
    }

    /// The feeder answered this phone's direct feed `feed`: no uncertain
    /// direct feed on it may take that one's end for its own.
    pub(crate) fn answered_direct(&mut self, feeder: &str, feed: u32) -> bool {
        let mut changed = false;
        for pin in &mut self.pins {
            if pin.feeder == feeder
                && pin.path == RoutePath::Direct
                && matches!(pin.write, PinnedWrite::Feed { .. })
                && !pin.excluded.contains(&feed)
            {
                pin.excluded.push(feed);
                changed = true;
            }
        }
        changed
    }
}

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
