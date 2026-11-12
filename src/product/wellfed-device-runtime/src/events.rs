//! The event history, carried between the loop task and the settings
//! writer, which alone writes the ring in storage sectors 13-24
//! (`wellfed_device::store::events`).
//!
//! - **Appends:** the loop queues a record with [`record`] and goes on; the
//!   writer drains the queue with [`pending`] and [`finish_pending`]. A full
//!   queue drops the record and counts it ([`take_lost`]), so a burst can
//!   never stall the loop; the loop records the count as an event of its own.
//!   The writer counts a record the ring refused there too
//!   ([`append_failed`]).
//!   The last slot is kept for a feed's outcome ([`record_outcome`]): appends
//!   wait while a feed holds the outputs, so the events of a long feed pile
//!   up here, and the feed's own result must not be the one lost.
//! - **Reads:** the device API asks for the record after a cursor with
//!   [`read_result`]; the writer takes the request from [`read_request`]
//!   and answers with [`finish_read`], and the loop collects it with
//!   [`read_result`] again. One read is outstanding at a time, in the
//!   turns [`wellfed_device::store::reads`] keeps (the simulator runs the
//!   same turns): a batch per read, fetched for one subscriber's slot and
//!   shared with any other whose cursor falls in it, round robin among
//!   those waiting, and given up at once by a subscriber that ends
//!   ([`reader_left`]).
//! - **The writer's own:** the event of a change the writer saves for
//!   someone who asked (a client's settings, Wi-Fi, plans or keys, or BLE
//!   setup's enrollment) goes straight to the ring in the turn that saved
//!   the change, never through the queue. [`wrote`] then publishes the
//!   ring's newest record, and hands an enrollment or revocation back for
//!   the loop's alert rules ([`take_written`]), the only ones they act on.
//!   One the outbox has no room for is counted ([`take_written_lost`]): it
//!   is in the ring, and only its alert is missed until the next boot's
//!   replay.
//! - **Epoch:** the writer publishes the ring's epoch once it has opened
//!   it ([`publish_open`]); the device API welcomes no session before then,
//!   since a welcome names the epoch its event numbers count in.
//!
//! Every access copies a few dozen bytes with the scheduler paused; both
//! sides are tasks, never interrupt handlers.

use wellfed_device::store::events::{BODY_BYTES, Batch, Read, Record};
use wellfed_device::store::reads::Reads;
pub use wellfed_device::store::reads::{MISSES_MAX, READ_BATCH};
use wellfed_device_api::Reader;

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
    /// Events the writer appended itself, oldest first, `written_len` of
    /// them, until the loop takes them.
    written: [Queued; WRITTEN],
    written_len: usize,
    /// Enrollments and revocations the outbox had no room for.
    written_lost: u32,
    lost: u32,
    last: Option<u64>,
    epoch: Option<u32>,
    /// The ring's newest record when it was opened.
    opened: Option<u64>,
    reads: Reads,
}

static SHARED: TaskCell<Shared> = TaskCell::new(Shared {
    queue: [UNQUEUED; QUEUE],
    head: 0,
    len: 0,
    written: [UNQUEUED; WRITTEN],
    written_len: 0,
    written_lost: 0,
    lost: 0,
    last: None,
    epoch: None,
    opened: None,
    reads: Reads::new(),
});
fn with<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
    sdk::unscheduled(|| {
        // SAFETY: with the scheduler paused no other task runs, and no
        // interrupt handler reaches this state.
        f(unsafe { &mut *SHARED.get() })
    })
}

/// Enrollments and revocations the writer appended that the loop has yet
/// to take. The writer makes one client store change per step, and the
/// loop takes them every pass, so a few is plenty.
pub const WRITTEN: usize = 4;

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

/// Records dropped since the last call, once the queue has a shared slot
/// for the `Lost` event that reports them: 0 while it has none, and the
/// count keeps growing until it does. Handed over sooner, the `Lost` would
/// itself be dropped and its count with it.
pub fn take_lost() -> u32 {
    with(|shared| {
        if shared.len >= QUEUE - RESERVED {
            return 0;
        }
        core::mem::take(&mut shared.lost)
    })
}

/// The writer's half: an event it could not append to the ring (a failed
/// program) is dropped, and counted with the queue's, so a `Lost` follows
/// it into the history.
pub fn append_failed() {
    with(|shared| shared.lost = shared.lost.saturating_add(1));
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

/// The writer's half: it appended `body` itself, and `last` is the ring's
/// newest record now. With `alerting`, `body` also waits for the loop's
/// alert rules, or is counted when the outbox is full.
pub fn wrote(at: Option<u32>, body: [u8; BODY_BYTES], last: Option<u64>, alerting: bool) {
    with(|shared| {
        shared.last = last;
        if !alerting {
            return;
        }
        match shared.written.get_mut(shared.written_len) {
            Some(slot) => {
                *slot = Queued { at, body };
                shared.written_len += 1;
            }
            None => shared.written_lost = shared.written_lost.saturating_add(1),
        }
    });
}

/// Enrollments and revocations the outbox had no room for since the last
/// call: in the ring, but never told to the alert rules this boot.
pub fn take_written_lost() -> u32 {
    with(|shared| core::mem::take(&mut shared.written_lost))
}

/// The loop's half: the oldest event the writer appended itself and the
/// loop has not yet taken, for the alert rules.
pub fn take_written() -> Option<Queued> {
    with(|shared| {
        if shared.written_len == 0 {
            return None;
        }
        let first = shared.written[0];
        shared.written.copy_within(1.., 0);
        shared.written_len -= 1;
        shared.written[shared.written_len] = UNQUEUED;
        Some(first)
    })
}

/// The writer's half: the ring's epoch and newest record, after opening it.
pub fn publish_open(epoch: u32, last: Option<u64>) {
    with(|shared| {
        shared.epoch = Some(epoch);
        shared.opened = last;
        shared.last = last;
    });
}

/// The ring's epoch, once the writer has opened it.
pub fn epoch() -> Option<u32> {
    with(|shared| shared.epoch)
}

/// The ring's newest record when the writer opened it: the boots before this
/// one's, whatever this boot has appended since.
pub fn opened_at() -> Option<u64> {
    with(|shared| shared.opened)
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
/// [`EventLog::read_batch`](wellfed_device::store::events::EventLog::read_batch)
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
        shared.written = [UNQUEUED; WRITTEN];
        shared.written_len = 0;
        shared.written_lost = 0;
        shared.lost = 0;
        shared.last = None;
        shared.epoch = None;
        shared.opened = None;
        shared.reads = Reads::new();
    });
}
