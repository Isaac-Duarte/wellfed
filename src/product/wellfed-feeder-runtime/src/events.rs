//! The event history, carried between the loop task and the settings
//! writer, which alone writes the ring in storage sectors 13-24
//! (`wellfed_feeder::store::events`).
//!
//! - **Appends:** the loop queues a record with [`record`] and goes on; the
//!   writer drains the queue with [`pending`] and [`finish_pending`]. A full
//!   queue drops the record and counts it ([`take_lost`]), so a burst can
//!   never stall the loop; the loop records the count as an event of its own.
//!   The last slot is kept for a feed's outcome ([`record_outcome`]): appends
//!   wait while a feed holds the outputs, so the events of a long feed pile
//!   up here, and the feed's own result must not be the one lost.
//! - **Reads:** the device API asks for the record after a cursor with
//!   [`read_result`]; the writer takes the request from [`read_request`]
//!   and answers with [`finish_read`], and the loop collects it with
//!   [`read_result`] again. One read is outstanding at a time, in the
//!   turns [`wellfed_feeder::store::reads`] keeps (the simulator runs the
//!   same turns): a batch per read, fetched for one subscriber's slot and
//!   shared with any other whose cursor falls in it, round robin among
//!   those waiting, and given up at once by a subscriber that ends
//!   ([`reader_left`]).
//! - **Epoch:** the writer publishes the ring's epoch once it has opened
//!   it ([`publish_open`]); the device API welcomes no session before then,
//!   since a welcome names the epoch its event numbers count in.
//!
//! Every access copies a few dozen bytes with the scheduler paused; both
//! sides are tasks, never interrupt handlers.

use wellfed_device_api::Reader;
use wellfed_feeder::store::events::{BODY_BYTES, Batch, Read, Record};
use wellfed_feeder::store::reads::Reads;
pub use wellfed_feeder::store::reads::{MISSES_MAX, READ_BATCH};

use wellfed_hal::cell::TaskCell;
use wellfed_hal::sdk;

/// Records the loop may queue before the writer takes them.
pub const QUEUE: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Queued {
    pub at: Option<u32>,
    pub body: [u8; BODY_BYTES],
}

/// A queue slot not yet filled. All zeros, as every initial field of
/// [`SHARED`] is, so the state goes to `.bss` rather than the image.
const UNQUEUED: Queued = Queued {
    at: None,
    body: [0; BODY_BYTES],
};

struct Shared {
    /// Records from `head`, `len` of them.
    queue: [Queued; QUEUE],
    head: usize,
    len: usize,
    lost: u32,
    last: Option<u64>,
    epoch: Option<u32>,
    reads: Reads,
}

static SHARED: TaskCell<Shared> = TaskCell::new(Shared {
    queue: [UNQUEUED; QUEUE],
    head: 0,
    len: 0,
    lost: 0,
    last: None,
    epoch: None,
    reads: Reads::new(),
});
fn with<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
    sdk::unscheduled(|| {
        // SAFETY: with the scheduler paused no other task runs, and no
        // interrupt handler reaches this state.
        f(unsafe { &mut *SHARED.get() })
    })
}

/// Slots only [`record_outcome`] may fill.
pub const RESERVED: usize = 1;

/// The loop's half: queue one record; false (and counted) when only the
/// reserved slots are left.
pub fn record(at: Option<u32>, body: [u8; BODY_BYTES]) -> bool {
    push(at, body, QUEUE - RESERVED)
}

/// As [`record`] for a feed's outcome, which may take a reserved slot. One
/// feed runs at a time and its outcome is recorded once, before the writes
/// it held off resume, so one slot is enough.
pub fn record_outcome(at: Option<u32>, body: [u8; BODY_BYTES]) -> bool {
    push(at, body, QUEUE)
}

fn push(at: Option<u32>, body: [u8; BODY_BYTES], limit: usize) -> bool {
    with(|shared| {
        if shared.len >= limit {
            shared.lost = shared.lost.saturating_add(1);
            return false;
        }
        let index = (shared.head + shared.len) % QUEUE;
        shared.queue[index] = Queued { at, body };
        shared.len += 1;
        true
    })
}

/// Records dropped since the last call.
pub fn take_lost() -> u32 {
    with(|shared| core::mem::take(&mut shared.lost))
}

/// The writer's half: the oldest queued record, if any; it stays queued
/// until [`finish_pending`].
pub fn pending() -> Option<Queued> {
    with(|shared| (shared.len > 0).then(|| shared.queue[shared.head]))
}

/// The writer appended (or gave up on) the oldest queued record; `last` is
/// the ring's newest record now.
pub fn finish_pending(last: Option<u64>) {
    with(|shared| {
        if shared.len > 0 {
            shared.queue[shared.head] = UNQUEUED;
            shared.head = (shared.head + 1) % QUEUE;
            shared.len -= 1;
        }
        shared.last = last;
    });
}

/// The writer's half: the ring's epoch and newest record, after opening it.
pub fn publish_open(epoch: u32, last: Option<u64>) {
    with(|shared| {
        shared.epoch = Some(epoch);
        shared.last = last;
    });
}

/// The ring's epoch, once the writer has opened it.
pub fn epoch() -> Option<u32> {
    with(|shared| shared.epoch)
}

/// The ring's newest record, as the writer last published it.
pub fn last_seq() -> Option<u64> {
    with(|shared| shared.last)
}

/// The loop's half: the answer for `after`, for the subscriber in slot
/// `reader`, if the writer has one that is still current. Otherwise this
/// waits the subscriber's turn, asks for the next read if the slot is
/// free, and returns `None`.
pub fn read_result(reader: Reader, after: Option<u64>) -> Option<Read> {
    with(|shared| shared.reads.result(reader, after, shared.last))
}

/// The loop's half: the subscriber in slot `reader` ended. Its turn is
/// given up, and a read fetched for it is nobody's, so the next ask may
/// replace it.
pub fn reader_left(reader: Reader) {
    with(|shared| shared.reads.left(reader));
}

/// The writer's half: the cursor a read waits on.
pub fn read_request() -> Option<Option<u64>> {
    with(|shared| shared.reads.request())
}

/// The writer's half: the answer to [`read_request`], as
/// [`EventLog::read_batch`](wellfed_feeder::store::events::EventLog::read_batch)
/// gave it, with its records at the front of `records`.
pub fn finish_read(after: Option<u64>, batch: Batch, records: &[Record]) {
    with(|shared| shared.reads.finish(after, batch, records, shared.last));
}

/// Forget everything (host tests).
#[cfg(not(target_os = "none"))]
pub fn reset() {
    with(|shared| {
        shared.queue = [UNQUEUED; QUEUE];
        shared.head = 0;
        shared.len = 0;
        shared.lost = 0;
        shared.last = None;
        shared.epoch = None;
        shared.reads = Reads::new();
    });
}
