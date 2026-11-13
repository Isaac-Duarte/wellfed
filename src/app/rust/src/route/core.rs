//! The router's rules with no IO: what an attempt's end means, when a feed
//! may start, and what settles an uncertain write. [`super::io`] drives
//! requests around these and keeps the [`Book`] on disk, so every rule here
//! is tested without a socket.
//!
//! A write goes out once. Nothing here ever plans a second send of a key:
//! resolving an uncertain write only reads the feeder's history, as the
//! phone's own session read it ([`settled_by_history`]). A feed that may
//! have run holds the next one on its feeder back until the history settles
//! it or the owner, having looked at the bowl, puts it away.

use uuid::Uuid;
use wellfed_client::history::Feed;
use wellfed_client::settle::{self, After, Lost};

use crate::api::feeder::{FeederReply, NotAsked};
use crate::api::router::Settlement;
use crate::api::store::SetAside;
use crate::route::pins::{PinRecord, PinState, PinnedWrite, RunningFeed};

/// How long a pin the owner put away is kept, from the acknowledgement, for
/// the feeder's history to settle it. One still pending holds forever.
pub(crate) const KEEP_MS: i64 = 7 * 24 * 3_600_000;

/// How one attempt ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ended {
    /// The feeder answered.
    Answered,
    /// Refused, and nothing ran.
    Refused,
    /// Definitely never sent.
    Unsent,
    /// It may have run.
    Uncertain,
}

/// What the phone's own session's `reply` to a write says of it. Only an
/// answer that names the write's end is definite; anything else (no answer,
/// one this build can't read, or the wrong one) may follow a write that ran,
/// so it stays pinned, uncertain.
pub(crate) const fn direct_ended(reply: &FeederReply) -> Ended {
    match reply {
        FeederReply::Done { .. }
        | FeederReply::Settings { .. }
        | FeederReply::Time { .. }
        | FeederReply::Plans { .. }
        | FeederReply::Clients { .. }
        | FeederReply::Invite { .. }
        | FeederReply::Calibration { .. }
        | FeederReply::Call { .. }
        | FeederReply::Stats { .. }
        | FeederReply::Trace { .. }
        | FeederReply::Update { .. } => Ended::Answered,
        FeederReply::NotSent { .. }
        | FeederReply::NotAsked {
            reason: NotAsked::Held,
        } => Ended::Unsent,
        FeederReply::Refused { .. } | FeederReply::NotAsked { .. } => Ended::Refused,
        FeederReply::Unanswered | FeederReply::Unexpected | FeederReply::Unreadable => {
            Ended::Uncertain
        }
    }
}

/// What this phone's own feed ends on `feeder`, as its session read them
/// from the history (`seen`), settle of the feeds pinned there, by the rule
/// the app shares with Home Assistant ([`wellfed_client::settle`]): an
/// uncertain feed is never given another's end. Every feed pinned to the
/// feeder takes part, one still sending too, so none is given an end that
/// may be another's; only an uncertain one is settled. A feed sent before
/// its session knew any event (no mark) is never settled, since any `Fed`
/// could be from before it. `phone` is this phone's key's first four bytes,
/// as events name it.
pub(crate) fn settled_by_history(
    pins: &[PinRecord],
    feeder: &str,
    seen: &[Feed],
    phone: [u8; 4],
) -> Vec<(Uuid, Settlement)> {
    let feeds: Vec<(&PinRecord, u8)> = pins
        .iter()
        .filter(|pin| pin.feeder == feeder)
        .filter_map(|pin| match pin.write {
            PinnedWrite::Feed { portions } => Some((pin, portions)),
            PinnedWrite::Other { .. } => None,
        })
        .collect();
    let lost: Vec<Lost<'_>> = feeds
        .iter()
        .map(|(pin, portions)| Lost {
            portions: *portions,
            after: pin.mark.map_or(After::Unknown, After::Event),
            excluded: &pin.excluded,
        })
        .collect();

    let ends = settle::settled(&lost, seen, phone);
    feeds
        .iter()
        .zip(ends)
        .filter(|((pin, _), _)| pin.state == PinState::Uncertain)
        .filter_map(|((pin, _), fed)| Some((pin.key, fed_settlement(fed?))))
        .collect()
}

/// A feed's end as its pin's settlement: an uncertain one by the portions
/// it confirmed, never more.
fn fed_settlement(fed: &Feed) -> Settlement {
    let (completed, _) = fed.delivered.bounds();
    Settlement::Fed {
        feed: fed.feed,
        requested: fed.requested,
        completed,
        outcome: fed.outcome,
        reason: fed.reason,
    }
}

/// The pinned writes and this phone's running feeds, as the router keeps
/// them on disk.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Book {
    pub(crate) pins: Vec<PinRecord>,
    /// Feeds this phone saw the feeder reserve, whose `Fed` it has not read
    /// yet: none is an uncertain feed's end.
    pub(crate) running: Vec<RunningFeed>,
    /// The pins file before this one did not read and was set aside. Any
    /// pin it held is lost, and a lost pin is a write that may have run: a
    /// feed among them would hold the next one on its feeder back, and which
    /// feeders they were on is lost too. So every feed waits until the owner
    /// has looked at the bowls ([`Book::bowls_checked`]).
    pub(crate) set_aside: Option<SetAside>,
}

/// Why a write is not admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Held {
    /// A feed on its feeder holds it back: this pin.
    By(Uuid),
    /// The pins file was set aside, and the owner has not yet said the bowls
    /// are checked.
    SetAside,
}

impl Book {
    /// The feed pending on `feeder` that holds a new one back: sent and not
    /// yet ended, or uncertain and not yet put away by the owner. A new feed
    /// may follow an uncertain one only once the owner has looked at the
    /// bowl and closed it, as the feed sheet asks.
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

    fn get_mut(&mut self, key: Uuid) -> Option<&mut PinRecord> {
        self.pins.iter_mut().find(|pin| pin.key == key)
    }

    /// The feeds running on `feeder` at `now_ms`, which a new feed's pin
    /// never takes an end from.
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
    /// does, and while a set-aside pins file holds every feed back. A feed's
    /// pin takes no end from a feed already running.
    pub(crate) fn admit(&mut self, pin: PinRecord, now_ms: i64) -> Result<(), Held> {
        if matches!(pin.write, PinnedWrite::Feed { .. }) {
            if self.set_aside.is_some() {
                return Err(Held::SetAside);
            }
            if let Some(holding) = self.holding_feeds(&pin.feeder) {
                return Err(Held::By(holding.key));
            }
        }
        let mut pin = pin;
        pin.excluded.extend(self.running_on(&pin.feeder, now_ms));
        self.pins.push(pin);
        Ok(())
    }

    /// An attempt ended `ended`. A write that may have run stays pinned,
    /// uncertain. Any other end takes it out. Whether the end changed the
    /// book.
    pub(crate) fn ended(&mut self, key: Uuid, ended: Ended) -> bool {
        match ended {
            Ended::Uncertain => match self.get_mut(key) {
                Some(held) => {
                    held.state = PinState::Uncertain;
                    true
                }
                None => false,
            },
            Ended::Answered | Ended::Refused | Ended::Unsent => self.settle(key),
        }
    }

    /// The owner looked at every feeder's bowl after the pins file was set
    /// aside: feeds go again. Whether it changed.
    pub(crate) fn bowls_checked(&mut self) -> bool {
        self.set_aside.take().is_some()
    }

    /// Take `key` out; whether it was there.
    pub(crate) fn settle(&mut self, key: Uuid) -> bool {
        let before = self.pins.len();
        self.pins.retain(|pin| pin.key != key);
        self.pins.len() != before
    }

    /// The owner looked and put `key` away at `now_ms`: it no longer holds
    /// a new feed back, and it stays pinned until the history settles it.
    /// One nothing can settle goes: a write other than a feed (the session
    /// never sends it again and no history names it), or a feed sent before
    /// its session knew any event. Whether it changed.
    pub(crate) fn acknowledge(&mut self, key: Uuid, now_ms: i64) -> bool {
        let Some(pin) = self.get_mut(key) else {
            return false;
        };
        if pin.acknowledged_ms.is_some() || pin.state != PinState::Uncertain {
            return false;
        }
        let settles = matches!(pin.write, PinnedWrite::Feed { .. }) && pin.mark.is_some();
        if settles {
            pin.acknowledged_ms = Some(now_ms);
        } else {
            self.settle(key);
        }
        true
    }

    /// Put away pins the owner acknowledged more than [`KEEP_MS`] before
    /// `now_ms`, and running feeds past the time they are kept. Whether any
    /// went.
    pub(crate) fn expire(&mut self, now_ms: i64) -> bool {
        let before = (self.pins.len(), self.running.len());
        self.pins.retain(|pin| {
            pin.acknowledged_ms
                .is_none_or(|at| now_ms.saturating_sub(at) < KEEP_MS)
        });
        self.running.retain(|running| running.until_ms > now_ms);
        before != (self.pins.len(), self.running.len())
    }

    /// The feeder reserved this phone's feed `feed`, which may run until
    /// `until_ms`: no uncertain feed on it, now or sent while it runs, may
    /// take that one's end for its own.
    pub(crate) fn started(&mut self, feeder: &str, feed: u32, until_ms: i64) {
        for pin in &mut self.pins {
            if pin.feeder == feeder
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
    pub(crate) fn ended_running(&mut self, feeder: &str, feeds: &[u32]) -> bool {
        let before = self.running.len();
        self.running
            .retain(|running| running.feeder != feeder || !feeds.contains(&running.feed));
        self.running.len() != before
    }
}

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
