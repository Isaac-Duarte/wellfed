//! The feeding owner's UART records (`PLOWN1`), as the host tools read them
//! back: a kind's own records ([`KindRecord`]) beside the SDK's, read with
//! `Record::event_with::<Owner>()`. The words decode through the same
//! tables the owner prints from (its state, outcome and reason numbers).

use wellfed_core::record::{KindRecord, Record};

use crate::feed::controller::FeedReason;
use crate::feed::owner::OwnerState;
use crate::feed::storage::OwnerOutcome;

wellfed_core::named_enum! {
    /// The feeder's tags the host tools act on.
    pub enum Tag {
        /// The feeding owner: `state=…`, `feed id=…`, `fed id=…`,
        /// `interlock …`.
        Owner = "PLOWN1",
    }
}

/// The feeding owner (`PLOWN1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    State {
        state: OwnerState,
        stopped: bool,
    },
    /// A feed was handed to the owner, whether or not it took it
    /// (`submit=`); the bench waits on the record, not the answer. A
    /// refused submission names the next id without consuming it, so the
    /// next feed it takes logs the same id: only an `accepted` one is a
    /// feed.
    Submitted {
        id: u64,
        /// `submit=0`: the owner queued it.
        accepted: bool,
    },
    /// A feed was refused before the owner saw it.
    Held,
    Fed {
        id: u64,
        requested: u16,
        completed: u16,
        outcome: Option<OwnerOutcome>,
        reason: Option<FeedReason>,
    },
    Interlock {
        writes_deferred: Option<u32>,
    },
}

impl<'a> KindRecord<'a> for Owner {
    fn decode(record: &Record<'a>) -> Option<Self> {
        if Tag::parse(record.tag)? != Tag::Owner {
            return None;
        }

        if let Some(state) = record.number("state") {
            return Some(Owner::State {
                state: OwnerState::from_code(state)?,
                stopped: record.flag("stopped")?,
            });
        }
        Some(match record.first {
            "feed" if record.field("held").is_some() => Owner::Held,
            "feed" => Owner::Submitted {
                id: record.number("id")?,
                // The owner's `SubmitResult::Enqueued`, which this crate
                // cannot name.
                accepted: record.number::<u32>("submit")? == 0,
            },
            "fed" => Owner::Fed {
                id: record.number("id")?,
                requested: record.number("requested")?,
                completed: record.number("completed")?,
                outcome: record.number("outcome").and_then(OwnerOutcome::from_byte),
                reason: record.number("reason").and_then(FeedReason::from_byte),
            },
            "interlock" => Owner::Interlock {
                writes_deferred: record.number("writes_deferred"),
            },
            _ => return None,
        })
    }
}
