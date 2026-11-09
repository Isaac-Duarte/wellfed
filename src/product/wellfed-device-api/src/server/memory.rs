//! The write ids each client's sessions have answered, kept per client key
//! so that a retry, even after a reconnect, gets the first answer instead of
//! running again.

use super::feeder::{FeederMemory, FeederReply};
use super::request::{Action, Reply};
use super::{Phase, Server};
use crate::{REMEMBERED, SESSIONS};
use wellfed_proto::api::{Refusal, RequestId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Remembered {
    Done(Option<u32>),
    Refused(Refusal),
    Invite {
        id: u32,
    },
    /// A feeder write's.
    Feeder(FeederMemory),
    /// An update op: a retry gets the update's state now.
    Update,
}

/// The write ids one client's sessions have answered.
#[derive(Clone, Copy)]
pub(super) struct Memory {
    key: [u8; 32],
    ids: [Option<(RequestId, Remembered)>; REMEMBERED],
    next: usize,
    /// For eviction: the pass that last used it.
    used: u32,
}

impl Memory {
    pub(super) const EMPTY: Self = Self {
        key: [0; 32],
        ids: [None; REMEMBERED],
        next: 0,
        used: 0,
    };

    pub(super) fn find(&self, id: &RequestId) -> Option<Remembered> {
        self.ids
            .iter()
            .flatten()
            .find(|(seen, _)| seen == id)
            .map(|&(_, result)| result)
    }

    /// A feed remembered as queued was refused: a retry hears that.
    pub(super) fn refuse_queued(&mut self, feed: u32, refusal: Refusal) {
        for (_, result) in self.ids.iter_mut().flatten() {
            if *result == (Remembered::Feeder(FeederMemory::Queued { feed })) {
                *result = Remembered::Refused(refusal);
            }
        }
    }

    /// Remember `id`'s answer, replacing what an earlier stage of the same
    /// write left (a queued feed's reservation).
    pub(super) fn remember(&mut self, id: RequestId, result: Remembered) {
        if let Some(entry) = self.ids.iter_mut().flatten().find(|(seen, _)| *seen == id) {
            entry.1 = result;
            return;
        }
        self.ids[self.next] = Some((id, result));
        self.next = (self.next + 1) % REMEMBERED;
    }
}

pub(super) fn remembered(reply: Reply) -> Remembered {
    match reply {
        Reply::Refused(refusal) => Remembered::Refused(refusal),
        Reply::Done(generation) => Remembered::Done(generation),
        Reply::Invite(invite) => Remembered::Invite { id: invite.id },
        Reply::Feeder(FeederReply::Feeding(feed)) => {
            Remembered::Feeder(FeederMemory::Feeding { feed })
        }
        Reply::Update(_) => Remembered::Update,
        Reply::Status
        | Reply::Feeder(FeederReply::Settings(_))
        | Reply::Clients
        | Reply::Time
        | Reply::Stats(_)
        | Reply::Feeder(FeederReply::FeedTrace { .. })
        | Reply::Bank1 { .. }
        | Reply::Feeder(FeederReply::Calibration)
        | Reply::Feeder(FeederReply::Plans)
        | Reply::Feeder(FeederReply::Call)
        | Reply::Alerts => Remembered::Done(None),
    }
}

impl Server {
    /// The memory for `key`, or the least recently used one not serving an
    /// open session, cleared for it.
    pub(super) fn memory(&mut self, key: &[u8; 32]) -> usize {
        let pass = self.pass;
        if let Some(index) = self
            .memories
            .iter()
            .position(|m| m.used != 0 && &m.key == key)
        {
            self.memories[index].used = pass;
            return index;
        }
        let in_use = |index: usize| {
            self.slots
                .iter()
                .any(|slot| matches!(slot.phase, Phase::Open { memory, .. } if memory == index))
        };
        let index = (0..SESSIONS)
            .filter(|&index| !in_use(index))
            .min_by_key(|&index| self.memories[index].used)
            .unwrap_or(0);
        self.memories[index] = Memory {
            key: *key,
            used: pass.max(1),
            ..Memory::EMPTY
        };
        index
    }

    /// Remember a write's outcome under its id.
    pub(super) fn remember(
        &mut self,
        memory: usize,
        id: RequestId,
        action: Action,
        outcome: Reply,
    ) -> Reply {
        if action.writes() {
            self.memories[memory].remember(id, remembered(outcome));
        }
        outcome
    }
}
