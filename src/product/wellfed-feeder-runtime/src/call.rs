//! The meal call between the loop and the writer.
//!
//! One slot carries the device API's work to the writer: an upload's
//! start, a delete, or one chunk of the upload, whose bytes wait in the
//! slot ([`Work`]) so neither task holds them on its stack. The writer applies it
//! to storage sectors 40-47 (`wellfed_feeder::store::call`) and publishes
//! the result ([`publish`]), and tells the speaker which clip it may play.
//! Nothing here writes flash.

use wellfed_core::clip::CHUNK_MAX;
use wellfed_core::store::slot::SlotIo;
use wellfed_device_api::mailbox::{CallChange, CallClip, CallResult, CallView, chunk_wanted};
use wellfed_feeder::store::call::CallStore;
use wellfed_hal::handoff::{Handoff, Published, Taken};
use wellfed_proto::api::Refusal;

/// The call's sectors within the storage region.
pub const CALL_BASE: u32 = wellfed_hal::board::storage::CALL.first;
pub const CALL_UNITS: u32 = wellfed_hal::board::storage::CALL.count;

/// What waits for the writer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Queued {
    Change(CallChange),
    /// `len` bytes of the work's data at stream offset `offset`.
    Chunk {
        offset: u32,
        len: u16,
    },
}

/// The work in the slot, and a chunk's bytes beside it, so neither task
/// holds them on its stack.
pub struct Work {
    pub queued: Queued,
    data: [u8; CHUNK_MAX as usize],
}

impl Work {
    /// A chunk's bytes; none for a change.
    pub fn bytes(&self) -> &[u8] {
        match self.queued {
            Queued::Chunk { len, .. } => self.data.get(..usize::from(len)).unwrap_or(&[]),
            Queued::Change(_) => &[],
        }
    }
}

static PENDING: Handoff<Work> = Handoff::new(Work {
    queued: Queued::Change(CallChange::Delete),
    data: [0; CHUNK_MAX as usize],
});

/// What NOR holds, published by the writer.
static PUBLISHED: Published<CallView> = Published::new(EMPTY);

const EMPTY: CallView = CallView {
    writable: false,
    clip: CallClip::None,
};

/// The call as the writer last published it, and whether work waits;
/// `Busy` until the writer has opened the store.
pub fn current() -> CallResult {
    let Some(view) = PUBLISHED.get() else {
        return CallResult::Busy;
    };
    CallResult::Current {
        view,
        pending: PENDING.waiting(),
    }
}

/// Claim the slot and fill it; false while it is taken.
fn queue(fill: impl FnOnce(&mut Work)) -> bool {
    let queued = PENDING.offer(fill);
    if queued {
        wellfed_hal::writer_wake::wake();
    }
    queued
}

/// Queue an upload's start or a delete for the writer.
pub fn change(change: CallChange) -> CallResult {
    let Some(view) = PUBLISHED.get() else {
        return CallResult::Busy;
    };
    if !view.writable {
        return CallResult::ReadOnly;
    }
    // Refused rather than queued for the seconds a call plays: the writer
    // could not take the sectors until it ends.
    if wellfed_hal::audio::call_playing() {
        return CallResult::Busy;
    }
    if !queue(|work| work.queued = Queued::Change(change)) {
        return CallResult::Busy;
    }
    CallResult::Current {
        view,
        pending: true,
    }
}

/// Queue one chunk of the upload under way: `Busy` while the last one is
/// still being written (the upload stands where it was until then).
pub fn chunk(offset: u32, data: &[u8]) -> Result<(), Refusal> {
    let Some(view) = PUBLISHED.get() else {
        return Err(Refusal::Busy);
    };
    if !view.writable {
        return Err(Refusal::NotWritable);
    }
    match chunk_wanted(view.clip, offset, data.len()) {
        Ok(false) => Ok(()),
        Ok(true) if wellfed_hal::audio::call_playing() => Err(Refusal::Busy),
        Ok(true) => {
            let len = u16::try_from(data.len()).map_err(|_| Refusal::Invalid)?;
            let queued = queue(|work| {
                work.queued = Queued::Chunk { offset, len };
                if let Some(head) = work.data.get_mut(..data.len()) {
                    head.copy_from_slice(data);
                }
            });
            if queued { Ok(()) } else { Err(Refusal::Busy) }
        }
        // The next chunk, sent while the one before is still queued.
        Err(_) if PENDING.waiting() => Err(Refusal::Busy),
        Err(refusal) => Err(refusal),
    }
}

/// The writer's half: the queued work and its bytes, held until the guard
/// is finished.
pub fn take() -> Option<Taken<'static, Work>> {
    PENDING.take()
}

/// The writer publishes what NOR holds, writable only once it is `armed`,
/// and the clip the speaker may play.
pub fn publish<I: SlotIo>(store: &CallStore<I>, armed: bool) {
    // The speaker learns the clip before a reader sees the view that names
    // it, at every publish: a client never reads a clip the speaker would
    // not yet play.
    wellfed_hal::audio::publish_call(store.stored());
    PUBLISHED.publish(store.view(armed));
}

/// Forget everything (host tests).
#[cfg(not(target_os = "none"))]
pub fn reset() {
    PUBLISHED.clear(EMPTY);
    PENDING.clear();
}
