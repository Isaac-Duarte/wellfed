//! The turns subscribers take reading the event ring ([`events`]), which
//! one read at a time answers.
//!
//! The device API asks for the record after a cursor with
//! [`Reads::result`]; whoever reads the ring takes the request from
//! [`Reads::request`] and answers with [`Reads::finish`], and the device
//! API collects it with [`Reads::result`] again. On the feeder the asker
//! is the loop task and the reader the settings writer, each its own task,
//! and the state sits in the runtime's shared cell; the simulator keeps
//! its own copy and answers between passes. Both run this one logic.
//!
//! - **Batches:** an answer is up to [`READ_BATCH`] records, so a
//!   subscriber catching up walks through all of it in one pass, into one
//!   frame of events, and several subscribers within it share it. A
//!   subscriber at the newest record is told so without a read, which
//!   leaves the slot to those still catching up.
//! - **Turns:** a batch is fetched for one subscriber, by its session
//!   slot, and stays until that subscriber has taken from it. Others whose
//!   cursor falls inside it read from it meanwhile; one it cannot answer
//!   waits its turn, kept by its slot with the cursor it last asked for.
//!   Once the batch's subscriber has taken from it, the next read goes to
//!   the next waiting slot after the last one served, round robin. So a
//!   subscriber catching up goes without a record for at most one pass
//!   more than there are subscribers, whatever order the loop visits them
//!   in. A subscriber that ends gives its turn and its batch up at once
//!   ([`Reads::left`]); one that stops asking without ending (its last
//!   frame still going out) keeps its batch for at most [`MISSES_MAX`]
//!   asks it turns away.
//!
//! The initial state is all zeros ([`Reads::new`]), so the firmware's copy
//! stays in `.bss`.
//!
//! [`events`]: super::events

use wellfed_device_api::{Reader, SESSIONS};

use super::events::{Batch, EMPTY, Read, Record};

/// Records one read answers at most. A frame holds at least 31 events (most
/// encode to 10-25 bytes), so one round trip through the writer, which polls
/// every few milliseconds, fills most of the frame a subscriber catching up
/// sends a pass.
pub const READ_BATCH: usize = 32;

/// Asks from other slots a batch turns away before it gives way untaken:
/// its subscriber stopped asking without ending (its last frame is still
/// going out). Each waiting slot asks at most once a loop pass, so this is
/// 50 passes, about a second at a 20 ms pass, with one other subscriber
/// waiting, and about 17 with three. A subscriber whose batch gave way
/// asks again in its turn.
pub const MISSES_MAX: u8 = 50;

// `repr(u8)` puts `Idle` at tag 0, so the initial state is all zeros.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum ReadSlot {
    Idle,
    /// A read for the subscriber in slot `reader`; `None` once it ended
    /// (or for a reader outside the slots), so nobody waits to collect it.
    Requested {
        reader: Option<usize>,
        after: Option<u64>,
    },
    /// The answer for slot `reader`'s cursor, `reader` as for `Requested`.
    /// Records stay valid; `End` only while the ring's newest record is
    /// still `last`.
    Ready {
        reader: Option<usize>,
        after: Option<u64>,
        batch: Batch,
        last: Option<u64>,
        /// Whether `reader` has taken anything from it.
        taken: bool,
        /// Asks from other slots it could not answer before then.
        misses: u8,
    },
}

/// A slot's place in the turns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum Turn {
    /// Not waiting: answered last time, at the newest record, or no
    /// subscriber.
    Idle,
    /// Turned away while asking for the record after this cursor.
    Waiting(Option<u64>),
}

/// The one outstanding read, its answer and every slot's turn.
#[derive(Debug)]
pub struct Reads {
    read: ReadSlot,
    /// Each session slot's turn.
    turns: [Turn; SESSIONS],
    /// The slot the next read looks for a waiting subscriber from.
    turn: usize,
    /// The records of a `Ready` batch, at the front.
    records: [Record; READ_BATCH],
}

impl Default for Reads {
    fn default() -> Self {
        Self::new()
    }
}

impl Reads {
    pub const fn new() -> Self {
        Self {
            read: ReadSlot::Idle,
            turns: [Turn::Idle; SESSIONS],
            turn: 0,
            records: [EMPTY; READ_BATCH],
        }
    }

    /// The answer for `after`, for the subscriber in slot `reader`, when
    /// the ring's newest record is `newest`, if a read has one that is
    /// still current. Otherwise this waits the subscriber's turn, asks for
    /// the next read if the slot is free, and returns `None`.
    pub fn result(
        &mut self,
        reader: Reader,
        after: Option<u64>,
        newest: Option<u64>,
    ) -> Option<Read> {
        let me = reader.index();

        // A subscriber at the newest record needs no read. Asking would
        // replace the answer another subscriber waits on, and one caught
        // up asks on every pass, so the other would never collect it.
        if after.is_some() && after == newest {
            self.set_turn(me, Turn::Idle);
            return Some(Read::End);
        }

        match &mut self.read {
            ReadSlot::Ready {
                reader: owner,
                after: answered,
                batch,
                last,
                taken,
                misses,
            } => {
                if let Some(found) = answer(*answered, *batch, *last, &self.records, newest, after)
                {
                    *taken |= *owner == Some(me);
                    self.set_turn(me, Turn::Idle);
                    return Some(found);
                }
                // Its own subscriber has moved past it, and an end the
                // ring has since moved past answers nobody.
                let stale = *batch == Batch::End && *last != newest;
                let held = !*taken && !stale && owner.is_some_and(|owner| owner != me);
                if held && *misses < MISSES_MAX {
                    *misses += 1;
                    self.set_turn(me, Turn::Waiting(after));
                    return None;
                }
            }
            ReadSlot::Requested { reader: owner, .. } => {
                if *owner != Some(me) {
                    self.set_turn(me, Turn::Waiting(after));
                }
                return None;
            }
            ReadSlot::Idle => {}
        }

        // The slot is free: the next waiting subscriber's read, this one
        // waiting among them.
        self.set_turn(me, Turn::Waiting(after));
        let next = (0..SESSIONS)
            .map(|offset| (self.turn + offset) % SESSIONS)
            .find_map(|slot| match self.turns[slot] {
                Turn::Waiting(cursor) => Some((slot, cursor)),
                Turn::Idle => None,
            });
        // A reader outside the slots has no turn: it is served at once,
        // for nobody to hold.
        let (owner, cursor) = match next {
            Some((slot, cursor)) => {
                self.set_turn(slot, Turn::Idle);
                self.turn = (slot + 1) % SESSIONS;
                (Some(slot), cursor)
            }
            None => (None, after),
        };
        self.read = ReadSlot::Requested {
            reader: owner,
            after: cursor,
        };
        None
    }

    /// The subscriber in slot `reader` ended. Its turn is given up, and a
    /// read fetched for it is nobody's, so the next ask may replace it.
    pub fn left(&mut self, reader: Reader) {
        let me = reader.index();
        self.set_turn(me, Turn::Idle);
        match &mut self.read {
            ReadSlot::Requested { reader: owner, .. } | ReadSlot::Ready { reader: owner, .. }
                if *owner == Some(me) =>
            {
                *owner = None;
            }
            _ => {}
        }
    }

    /// The cursor the outstanding read waits on.
    pub fn request(&self) -> Option<Option<u64>> {
        match self.read {
            ReadSlot::Requested { after, .. } => Some(after),
            _ => None,
        }
    }

    /// The answer to [`Self::request`], as
    /// [`EventLog::read_batch`](super::events::EventLog::read_batch) gave
    /// it, with its records at the front of `records`, while the ring's
    /// newest record is `newest`.
    pub fn finish(
        &mut self,
        after: Option<u64>,
        batch: Batch,
        records: &[Record],
        newest: Option<u64>,
    ) {
        let batch = match batch {
            Batch::Records(len) => {
                let len = len.min(records.len()).min(READ_BATCH);
                self.records[..len].copy_from_slice(&records[..len]);
                Batch::Records(len)
            }
            other => other,
        };
        let reader = match self.read {
            ReadSlot::Requested { reader, .. } => reader,
            _ => None,
        };
        self.read = ReadSlot::Ready {
            reader,
            after,
            batch,
            last: newest,
            taken: false,
            misses: 0,
        };
    }

    fn set_turn(&mut self, slot: usize, turn: Turn) {
        if let Some(held) = self.turns.get_mut(slot) {
            *held = turn;
        }
    }
}

/// What a `Ready` batch answers for `after`: the asked cursor's answer, or
/// the record after any of its records but the last (which needs the next
/// batch); `None` when it holds no answer for it.
fn answer(
    answered: Option<u64>,
    batch: Batch,
    last: Option<u64>,
    records: &[Record],
    newest: Option<u64>,
    after: Option<u64>,
) -> Option<Read> {
    match batch {
        Batch::Records(len) => {
            let records = records.get(..len)?;
            if answered == after {
                return records.first().copied().map(Read::Record);
            }
            let held = records.iter().position(|r| Some(r.seq) == after)?;
            records.get(held + 1).copied().map(Read::Record)
        }
        Batch::Gap { from, to } => (answered == after).then_some(Read::Gap { from, to }),
        Batch::End => (answered == after && last == newest).then_some(Read::End),
    }
}
