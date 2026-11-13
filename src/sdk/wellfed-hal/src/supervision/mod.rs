//! KM4 supervision: a device owner's beat, the watchdog and when it is fed,
//! the breadcrumb and call trail a silent reset leaves, and the
//! bench-derived constants they run with. The feeder's TIM2 tick and
//! feedback capture are its drivers', in `wellfed-hal-feeder`.
//!
//! **No store write starts into a reset.** Every store write sets the flash
//! attempt marker for its duration, and a boot that finds the marker set
//! skips arming the flash hooks (once per power cycle), so a write that
//! hangs cannot reset the board over and over. The marker cannot tell a
//! hang inside the write from a reset that merely landed during it, so a
//! reset the runtime decided must not land in one: while the watchdog
//! feeder withholds the watchdog ([`writes_held`]), from the first pass it
//! does not feed, no write turn is granted, no arming starts and an install
//! goes no further. A write already running when the feeding stops has the
//! rest of the watchdog's expiry (at least 2.4 s less one feed period) to
//! finish; writes take tens of milliseconds. So the marker stays set only
//! after a reset the runtime did not decide (a hang, a fault, a lease that
//! halted), which is what it exists for, and a withheld reset never needs
//! to clear it.

mod active;
pub mod beat;
pub mod config;
pub mod crumb;
pub mod supervise;
pub mod trail;
pub mod watchdog;

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// The watchdog feeder stops feeding for good (an intentional restart, a
/// refused checkpoint, or a supervision stop that outlasted the progress
/// gap: [`supervised`]), so a reset follows within the watchdog's expiry.
/// The breadcrumb records that the reset is the feeder's own, and the Wi-Fi task leaves the network
/// meanwhile, so the access point does not reject the next boot's
/// association with status 30. Idempotent; nothing here waits.
pub fn stopping() {
    STOPPING.store(true, Ordering::Release);
    crumb::stopping();
    crate::link::leave_for_restart();
}

/// An intentional restart's grace (one the device API or setup asks for):
/// for up to [`RESTART_GRACE_MS`] the watchdog is still fed and writes
/// still run, so a writer can save what it keeps, then the runtime's
/// watchdog feeder restarts ([`restart_grace_over`]). A writer that has
/// nothing more to save says so ([`restart_flushed`]) and the restart comes
/// at once. Saving inside the grace, before the feeding stops, keeps every
/// write out of the reset's way (see the module documentation). A second
/// request keeps the first one's start.
pub fn begin_restart_grace(now_ms: u32) {
    // One word holds both "asked" and the start, so no reader sees the one
    // without the other: a start of 0 would end the grace at once.
    let _ = GRACE_AT.compare_exchange(0, now_ms | 1, Ordering::AcqRel, Ordering::Acquire);
}

/// Whether an intentional restart was asked for this boot: its grace runs,
/// or has ended and the reset is coming.
pub fn restart_requested() -> bool {
    GRACE_AT.load(Ordering::Acquire) != 0
}

/// Whether an intentional restart's grace runs: save now.
pub fn restart_grace() -> bool {
    restart_requested() && !GRACE_ENDED.load(Ordering::Acquire)
}

/// The writer saved what the restart should keep, or has nothing to save.
pub fn restart_flushed() {
    FLUSHED.store(true, Ordering::Release);
}

/// The watchdog feeder's half: true once, when the grace has ended, the
/// writer flushed or the grace ran out; the caller then restarts. A `now_ms`
/// read before the grace began (another task's, a moment stale) is no time
/// at all into it.
pub fn restart_grace_over(now_ms: u32) -> bool {
    let at = GRACE_AT.load(Ordering::Acquire);
    if at == 0 {
        return false;
    }
    let elapsed = now_ms.wrapping_sub(at & !1);
    let ran_out = (RESTART_GRACE_MS..HALF_RANGE_MS).contains(&elapsed);
    let over = FLUSHED.load(Ordering::Acquire) || ran_out;
    over && !GRACE_ENDED.swap(true, Ordering::AcqRel)
}

/// The longest an intentional restart waits for the writer: a store write
/// at its worst after a writer poll, with room to spare.
pub const RESTART_GRACE_MS: u32 = 2_000;

/// Past this an uptime difference reads as negative: a time before the
/// grace began.
const HALF_RANGE_MS: u32 = 1 << 31;

/// When the intentional restart's grace began, with the low bit set so a
/// start at uptime 0 still reads as asked; 0 while none was asked.
static GRACE_AT: AtomicU32 = AtomicU32::new(0);
static FLUSHED: AtomicBool = AtomicBool::new(false);
static GRACE_ENDED: AtomicBool = AtomicBool::new(false);

/// Whether a stop that began at `since_ms` is certain to end
/// in a reset at `now_ms`. `supervise::feed` is no latch: feeding resumes
/// if it turns true again before the watchdog's progress gap passes, so a
/// sub-second false (a beat just past its gap, one pass of unverified
/// outputs) costs nothing. Once the stop has lasted the whole gap, the next
/// checkpoint is refused whatever `feed` says, and the reset comes.
pub fn stop_certain(since_ms: u32, now_ms: u32, gap_ms: u32) -> bool {
    now_ms.wrapping_sub(since_ms) >= gap_ms
}

/// The feeder task's open supervision stop: when it began, while one runs.
static STOP_SINCE: AtomicU32 = AtomicU32::new(0);
static STOP_OPEN: AtomicBool = AtomicBool::new(false);
/// [`stopping`] ran: the reset is certain.
static STOPPING: AtomicBool = AtomicBool::new(false);

/// One feeder pass's `supervise::feed` answer at uptime `now_ms`. A stop
/// that has lasted the watchdog's progress gap is certain to reset, and
/// only then calls [`stopping`]: a transient false must not take the
/// network down for a board that stays up. Feeder task only.
pub fn supervised(feeding: bool, now_ms: u32) {
    if feeding {
        STOP_OPEN.store(false, Ordering::Release);
        return;
    }
    if !STOP_OPEN.swap(true, Ordering::AcqRel) {
        STOP_SINCE.store(now_ms, Ordering::Relaxed);
    }
    let since = STOP_SINCE.load(Ordering::Relaxed);
    if stop_certain(since, now_ms, config::WATCHDOG.max_progress_gap_ms) {
        stopping();
    }
}

/// Whether store writes are held back: the watchdog feeder is not feeding,
/// or a reset is certain. A transient stop holds them for under a second;
/// a certain one until the reset. See the module documentation.
pub fn writes_held() -> bool {
    STOP_OPEN.load(Ordering::Acquire) || STOPPING.load(Ordering::Acquire)
}

/// The reset is coming whatever happens now, so what is left to say goes out
/// at any cost: the runtime is restarting, a checkpoint was refused (it
/// latches), or a stop outlasted the progress gap ([`stopping`]). A transient
/// stop is none of these: feeding resumes and the board stays up.
pub fn reset_certain(restarting: bool, checkpoint_refused: bool) -> bool {
    restarting || checkpoint_refused || STOPPING.load(Ordering::Acquire)
}

/// Checkpoints the watchdog feeder has made this boot.
static CHECKPOINTS: AtomicU32 = AtomicU32::new(0);
/// The watchdog feeder supervises this boot, so [`Pace`] waits for it.
static PACING: AtomicBool = AtomicBool::new(false);

/// The watchdog opened: from now on a [`Pace`]d run of writes waits for the
/// feeder's checkpoints. Feeder task only, once.
pub fn pace_writes() {
    PACING.store(true, Ordering::Release);
}

/// The watchdog feeder made a checkpoint. Feeder task only.
pub fn checkpointed() {
    CHECKPOINTS.fetch_add(1, Ordering::AcqRel);
}

/// A run of flash write steps taken back to back (a factory reset's, a
/// handover's), paced by the watchdog feeder: each step's leases (an erase,
/// or a store's erase and programs) in a feed period of their own.
///
/// Every flash lease masks IRQs and so stops SysTick: the feeder's wait
/// between checkpoints, counted in ticks, stretches by the masked time of
/// every lease inside it, while the watchdog's progress gap is measured
/// on the system timer. Steps a millisecond apart fit most of a second of
/// masked time into one feed period: a factory reset's forty erase steps did
/// (`gap_ms=1063` on a feed period of 250 ticks). So a paced run takes its
/// next step only once the feeder has checkpointed since its last, and one
/// feed period holds at most one step's leases.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pace {
    /// The feeder's checkpoint count when the last step's writes ended.
    after: Option<u32>,
}

impl Pace {
    /// Whether the next step may write: none ran yet, the feeder has
    /// checkpointed since the last, or no watchdog feeder supervises.
    pub fn ready(&self) -> bool {
        match self.after {
            None => true,
            Some(mark) => {
                !PACING.load(Ordering::Acquire) || CHECKPOINTS.load(Ordering::Acquire) != mark
            }
        }
    }

    /// The step's writes ended.
    pub fn wrote(&mut self) {
        self.after = Some(CHECKPOINTS.load(Ordering::Acquire));
    }
}

/// Host tests only: no stop open, none certain.
#[cfg(not(target_os = "none"))]
pub fn reset() {
    STOP_OPEN.store(false, Ordering::Relaxed);
    STOPPING.store(false, Ordering::Relaxed);
    GRACE_AT.store(0, Ordering::Relaxed);
    FLUSHED.store(false, Ordering::Relaxed);
    GRACE_ENDED.store(false, Ordering::Relaxed);
    CHECKPOINTS.store(0, Ordering::Relaxed);
    PACING.store(false, Ordering::Relaxed);
}
