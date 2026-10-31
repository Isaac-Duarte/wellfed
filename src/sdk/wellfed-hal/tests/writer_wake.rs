//! A mailbox waking the store writer: no notification without a task to
//! wake, none to one that has gone, and an early step only for a writer at
//! rest. A wake is a shortcut, so anything refused here falls to the poll.

use wellfed_hal::sdk::fake;
use wellfed_hal::writer_wake;

fn notifies() -> u32 {
    fake::with(|h| h.task_notifies)
}

/// One process: the wake's state is static.
#[test]
fn a_wake_reaches_only_a_bound_task_and_steps_only_a_rested_writer() {
    fake::reset_hardware();
    writer_wake::reset();

    // No service task yet: nothing to notify, and it is remembered.
    writer_wake::settle(true);
    writer_wake::wake();
    assert_eq!(notifies(), 0);
    assert!(writer_wake::take(), "seen by the task's first pass");
    assert!(!writer_wake::take(), "once");

    // Bound: each wake notifies.
    writer_wake::bind();
    writer_wake::bind();
    writer_wake::wake();
    writer_wake::wake();
    assert_eq!(notifies(), 2);
    assert!(writer_wake::take(), "two wakes, one step");
    assert!(!writer_wake::take());

    // A spurious notification finds nothing wanted: no early step.
    assert!(!writer_wake::take());

    // A writer that wrote or waits for a turn keeps its delay, and the wake
    // goes: the mailbox still holds the work for its next poll.
    writer_wake::settle(false);
    writer_wake::wake();
    assert_eq!(notifies(), 3);
    assert!(!writer_wake::take());
    writer_wake::settle(true);
    assert!(!writer_wake::take(), "dropped, not saved up");

    // A task that ended has no handle to notify.
    writer_wake::unbind();
    writer_wake::wake();
    assert_eq!(notifies(), 3);
}
