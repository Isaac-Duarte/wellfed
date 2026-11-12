//! The single-slot hand-off between the task that asks for a change and
//! the one task that applies it, and the view the applier publishes back.
//!
//! Every store the writer task saves (the plans, the calibration, the Wi-Fi
//! and time settings, the client table, the device settings) takes its
//! changes through one [`Handoff`], and so does the update stage's request:
//! the asking side fills the one slot and queues it, and the consumer takes
//! it, applies it and finishes it. The
//! slot's states make each step exclusive, so no caller needs `unsafe`:
//!
//! - `Empty` to `Filling` is the producer's claim. Only the claimant writes
//!   the value, and a second producer is refused until the slot is empty
//!   again ([`Handoff::offer`]).
//! - `Queued` to `Taken` is the consumer's claim ([`Handoff::take`]). Only
//!   the holder of the [`Taken`] guard reads the value, and no producer can
//!   claim the slot while it is held.
//! - [`Taken::finish`] is the only way back to `Empty`. A guard dropped
//!   unfinished puts the value back, queued, for the next take: a writer
//!   that cannot apply a change this pass leaves it waiting, never lost.
//!
//! The applier publishes what its store then holds through [`Published`],
//! one whole view under the interrupt guard, so a reader never pairs half
//! of one view with half of another.

use core::cell::Cell;
use core::marker::PhantomData;
#[cfg(any(not(target_os = "none"), feature = "km4"))]
use core::sync::atomic::AtomicBool;
use core::sync::atomic::{AtomicU8, Ordering};

use crate::cell::TaskCell;
#[cfg(any(not(target_os = "none"), feature = "km4"))]
use crate::sdk::IrqGuard;

/// Who holds a [`Handoff`]'s value, as its state stores it.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Hand {
    Empty = 0,
    /// The producer that claimed the slot is writing the value.
    Filling = 1,
    /// Handed to the consumer, not taken yet.
    Queued = 2,
    /// The consumer holds it through a [`Taken`] guard.
    Taken = 3,
}

/// One slot from any number of producers to one consumer at a time.
pub struct Handoff<T> {
    state: AtomicU8,
    /// Written only by the producer in `Filling`, read only through the
    /// [`Taken`] guard in `Taken`.
    value: TaskCell<T>,
}

impl<T> Handoff<T> {
    /// An empty slot; `empty` is a placeholder no consumer ever reads.
    pub const fn new(empty: T) -> Self {
        Self {
            state: AtomicU8::new(Hand::Empty as u8),
            value: TaskCell::new(empty),
        }
    }

    /// Claim the slot, write the value with `fill` and queue it. `false`,
    /// with `fill` never called, while another value is being written,
    /// waits or is taken.
    pub fn offer(&self, fill: impl FnOnce(&mut T)) -> bool {
        self.try_offer(|value| {
            fill(value);
            Ok::<(), ()>(())
        })
        .is_some()
    }

    /// [`Self::offer`] for a producer that may still decline once it holds
    /// the claim: `fill`'s `Err` leaves the slot empty again, nothing
    /// queued. `None`, with `fill` never called, while the slot is not
    /// empty.
    pub fn try_offer<R, E>(
        &self,
        fill: impl FnOnce(&mut T) -> Result<R, E>,
    ) -> Option<Result<R, E>> {
        if self
            .state
            .compare_exchange(
                Hand::Empty as u8,
                Hand::Filling as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return None;
        }
        // SAFETY: `Filling` makes this the only accessor: no other producer
        // claims a slot that is not empty, and the consumer takes only a
        // queued one.
        let filled = fill(unsafe { &mut *self.value.get() });
        let next = if filled.is_ok() {
            Hand::Queued
        } else {
            Hand::Empty
        };
        self.state.store(next as u8, Ordering::Release);
        Some(filled)
    }

    /// Whether a value is being written, waits or is taken: a change still
    /// on its way to the store.
    pub fn waiting(&self) -> bool {
        self.state.load(Ordering::Acquire) != Hand::Empty as u8
    }

    /// Take the queued value, if any, until the guard is finished or
    /// dropped. A second take while a guard is held finds nothing.
    pub fn take(&self) -> Option<Taken<'_, T>> {
        self.state
            .compare_exchange(
                Hand::Queued as u8,
                Hand::Taken as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .ok()
            .map(|_| Taken {
                handoff: self,
                not_sync: PhantomData,
            })
    }

    /// Drop a queued value nobody has taken (a host test's fresh start).
    /// A value being written or held through a guard stays as it is:
    /// emptying either would let the next producer write the slot while its
    /// claimant or the guard still reaches it. Host builds alone, where tests
    /// start fresh; no firmware forgets a change it was asked for.
    #[cfg(not(target_os = "none"))]
    pub fn clear(&self) {
        let _ = self.state.compare_exchange(
            Hand::Queued as u8,
            Hand::Empty as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

/// The consumer's hold on a [`Handoff`]'s value.
///
/// A guard may move to another task, but it is never `Sync`: the slot is
/// exclusive to whoever holds the guard, and a guard shared by reference
/// would let two tasks read through it at once (a `T` with interior
/// mutability then races). The marker takes `Sync` away whatever `T` is;
/// the workspace runs no compile-fail tests, so this comment is its record.
pub struct Taken<'a, T> {
    handoff: &'a Handoff<T>,
    not_sync: PhantomData<Cell<()>>,
}

impl<T> Taken<'_, T> {
    /// The value, read in place.
    pub fn get(&self) -> &T {
        // SAFETY: the slot is `Taken` while this guard lives, so no producer
        // writes it, and `take` hands out one guard at a time.
        unsafe { &*self.handoff.value.get() }
    }

    /// The value is applied or refused: free the slot for the next.
    pub fn finish(self) {
        self.handoff
            .state
            .store(Hand::Empty as u8, Ordering::Release);
        core::mem::forget(self);
    }
}

impl<T> Drop for Taken<'_, T> {
    /// Unfinished, the value waits for the next take.
    fn drop(&mut self) {
        let _ = self.handoff.state.compare_exchange(
            Hand::Taken as u8,
            Hand::Queued as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

/// A view one task publishes whole and others read whole, copied in and
/// out with interrupts masked; none until the first publish. KM4's alone:
/// the interrupt guard is KM4's, and KM0 runs no writer.
#[cfg(any(not(target_os = "none"), feature = "km4"))]
pub struct Published<V> {
    view: TaskCell<V>,
    loaded: AtomicBool,
}

#[cfg(any(not(target_os = "none"), feature = "km4"))]
impl<V: Copy> Published<V> {
    /// Nothing published; `empty` is what [`Self::clear`] leaves behind and
    /// no reader sees.
    pub const fn new(empty: V) -> Self {
        Self {
            view: TaskCell::new(empty),
            loaded: AtomicBool::new(false),
        }
    }

    pub fn publish(&self, view: V) {
        {
            let _guard = IrqGuard::lock();
            // SAFETY: interrupts are masked, so this is the only reference.
            unsafe { *self.view.get() = view };
        }
        self.loaded.store(true, Ordering::Release);
    }

    /// The view last published; `None` before the first.
    pub fn get(&self) -> Option<V> {
        if !self.loaded.load(Ordering::Acquire) {
            return None;
        }
        let _guard = IrqGuard::lock();
        // SAFETY: interrupts are masked, so this is the only reference.
        Some(unsafe { *self.view.get() })
    }

    /// Forget the view, as before the first publish (a host test's fresh
    /// start).
    #[cfg(not(target_os = "none"))]
    pub fn clear(&self, empty: V) {
        self.loaded.store(false, Ordering::Release);
        let _guard = IrqGuard::lock();
        // SAFETY: interrupts are masked, so this is the only reference.
        unsafe { *self.view.get() = empty };
    }
}
