//! The loop waking the store writer in the service task, when it queues a
//! mailbox, instead of leaving the work to the writer's next poll.
//!
//! **A wake is only ever a shortcut.** The writer's poll stays, and a mailbox
//! holds its work until the writer takes it, so a wake that is lost, refused
//! or spurious changes when a write happens and nothing else. It decides no
//! write: each still takes the write turn, which a feed's claim holds until
//! the owner is at rest.
//!
//! **It keeps the writer's rest.** Only a writer that rested on its last step
//! is stepped early. After a write it keeps the delay it chose, so a client
//! repeating a change wears the flash and masks interrupts no faster than the
//! poll let it. And it never hurries a writer still arming, or one waiting for
//! the write turn, since asking again sooner gets it no sooner.
//!
//! **Task context only**, as every mailbox is filled from a task:
//! [`sdk::task_notify`] refuses anything else, leaving the work to the poll.

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::sdk::{self, IrqGuard};

/// The service task's handle, 0 while it has none.
static SERVICE: AtomicUsize = AtomicUsize::new(0);
/// A mailbox was queued since the service task last looked.
static WANTED: AtomicBool = AtomicBool::new(false);
/// The writer's last step rested: it wrote nothing and waits for nothing.
static RESTED: AtomicBool = AtomicBool::new(false);

/// The service task, each pass: name the task to wake.
pub fn bind() {
    if SERVICE.load(Ordering::Acquire) == 0 {
        SERVICE.store(sdk::task_handle(), Ordering::Release);
    }
}

/// The service task, before it ends: its handle dies with it.
pub fn unbind() {
    SERVICE.store(0, Ordering::Release);
}

/// A mailbox holds new work. Masked from the handle's load to the notification,
/// so the service task cannot run, end and be deleted in between.
pub fn wake() {
    WANTED.store(true, Ordering::Release);

    let _guard = IrqGuard::lock();
    let service = SERVICE.load(Ordering::Acquire);
    if service != 0 {
        sdk::task_notify(service);
    }
}

/// The writer's step: whether it rested, so a wake may cut its delay short.
pub fn settle(rested: bool) {
    RESTED.store(rested, Ordering::Release);
}

/// The service task: whether to step the writer now. Clears before the step
/// reads its mailboxes, so work queued after this is either seen by that step
/// or wakes the next pass. A wake a writer that has not rested does not take is
/// dropped here, and its own delay brings it to the work.
pub fn take() -> bool {
    WANTED.swap(false, Ordering::AcqRel) && RESTED.load(Ordering::Acquire)
}

/// Forget everything (host tests).
#[cfg(not(target_os = "none"))]
pub fn reset() {
    SERVICE.store(0, Ordering::Release);
    WANTED.store(false, Ordering::Release);
    RESTED.store(false, Ordering::Release);
}
