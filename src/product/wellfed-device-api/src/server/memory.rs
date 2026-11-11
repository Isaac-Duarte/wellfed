//! The write ids each client's sessions have asked, kept per client key
//! so that a retry, even after a reconnect, gets the first answer instead of
//! running again: the answer once there is one, and the write under way
//! before.

use super::feeder::{FeederReply, FeederWait};
use super::handshake::invite;
use super::request::{Action, Reply};
use super::{Phase, Server, Wait};
use crate::pairing::Pairing;
use crate::{Device, REMEMBERED, SESSIONS};
use wellfed_proto::api::{Refusal, RequestId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Remembered {
    Done(Option<u32>),
    Refused(Refusal),
    Invite {
        id: u32,
    },
    /// A feed reserved under `feed`.
    Feeding {
        feed: u32,
    },
    /// An update op: a retry gets the update's state now.
    Update,
    /// A write queued and not yet finished: a retry waits for it as the
    /// first request did. Remembered from the moment the write is queued,
    /// so a client whose session ended before the answer still gets it.
    Waiting(Wait),
    /// A write that finished with its outcome gone before anyone read it:
    /// a retry's session ends unanswered (`Reason::Forgotten`), as the
    /// first's did, and the entry is free for a later id.
    Lost,
}

/// The write ids one client's sessions have asked.
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

    /// Whether it holds a write still under way.
    fn waiting(&self) -> bool {
        self.ids
            .iter()
            .flatten()
            .any(|(_, result)| matches!(result, Remembered::Waiting(_)))
    }

    /// The entry at `index`, for the pass that settles writes whose
    /// session has gone.
    pub(super) fn entry(&self, index: usize) -> Option<(RequestId, Remembered)> {
        self.ids.get(index).copied().flatten()
    }

    /// A feed remembered as queued was refused: a retry hears that.
    pub(super) fn refuse_queued(&mut self, feed: u32, refusal: Refusal) {
        for (_, result) in self.ids.iter_mut().flatten() {
            if let Remembered::Waiting(Wait::Feeder(FeederWait::Feed { feed: queued })) = *result
                && queued == feed
            {
                *result = Remembered::Refused(refusal);
            }
        }
    }

    /// Remember `id`'s answer, replacing what an earlier stage of the same
    /// write left (its wait). A new id takes the oldest entry that holds an
    /// answer: a write under way keeps its entry until it is settled, so a
    /// retry waiting on it never finds it gone. Writes under way number at
    /// most one a session and those whose session went since the last pass,
    /// far fewer than the entries.
    pub(super) fn remember(&mut self, id: RequestId, result: Remembered) {
        if let Some(entry) = self.ids.iter_mut().flatten().find(|(seen, _)| *seen == id) {
            entry.1 = result;
            return;
        }
        let free = (0..REMEMBERED)
            .map(|offset| (self.next + offset) % REMEMBERED)
            .find(|&index| !matches!(self.ids[index], Some((_, Remembered::Waiting(_)))))
            .unwrap_or(self.next);
        self.ids[free] = Some((id, result));
        self.next = (free + 1) % REMEMBERED;
    }
}

pub(super) fn remembered(reply: Reply) -> Remembered {
    match reply {
        Reply::Refused(refusal) => Remembered::Refused(refusal),
        Reply::Done(generation) => Remembered::Done(generation),
        Reply::Invite(invite) => Remembered::Invite { id: invite.id },
        Reply::Feeder(FeederReply::Feeding(feed)) => Remembered::Feeding { feed },
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

/// What a retry of a write hears from what its id holds.
pub(super) enum Recall {
    Answer(Reply),
    /// The write is still under way: the retry waits on it.
    Waiting,
    /// Nobody can say what the write did: the retry's session ends
    /// unanswered.
    Lost,
}

/// What a retry of a write hears from what its id holds.
pub(super) fn recall<D: Device>(
    remembered: Remembered,
    device: &mut D,
    pairing: &mut Pairing,
    now_ms: u32,
) -> Recall {
    Recall::Answer(match remembered {
        Remembered::Done(generation) => Reply::Done(generation),
        Remembered::Refused(refusal) => Reply::Refused(refusal),
        Remembered::Invite { id } => pairing
            .invite_secret(now_ms, id)
            .map_or(Reply::Refused(Refusal::NotFound), |secret| {
                Reply::Invite(invite(id, secret))
            }),
        Remembered::Feeding { feed } => Reply::Feeder(FeederReply::Feeding(feed)),
        Remembered::Update => device
            .update_view()
            .map_or(Reply::Refused(Refusal::Unavailable), Reply::Update),
        Remembered::Waiting(_) => return Recall::Waiting,
        Remembered::Lost => return Recall::Lost,
    })
}

impl Server {
    /// The memory for `key`, or the least recently used one not serving an
    /// open session, cleared for it, preferring one with no write under
    /// way.
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
        // A memory holding a write under way goes last: its client may yet
        // retry, and the pass is still to settle it.
        let index = (0..SESSIONS)
            .filter(|&index| !in_use(index))
            .min_by_key(|&index| (self.memories[index].waiting(), self.memories[index].used))
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
