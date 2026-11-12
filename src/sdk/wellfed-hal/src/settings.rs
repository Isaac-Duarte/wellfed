//! The device settings as the device API sees them. The settings writer
//! (`wellfed_device_runtime::writer`) publishes what NOR holds; a settings
//! write from an owner's session queues one change for it. Nothing here
//! touches flash, so the API's loop may call it.
//!
//! The settings are the device's and its kind's together
//! (`wellfed_core::settings::KindSettings`), so the mailbox is generic over
//! the kind and the kind's runtime crate owns its one static ([`Mailbox`]).
//! The SDK's own services (the speaker) read the device's part alone, which
//! every publish also leaves here ([`current`]), so they need no kind.
//!
//! Each view and the queued change are whole structs, copied in and out
//! with interrupts masked, so a reader never sees half a publish.

use core::sync::atomic::{AtomicBool, Ordering};

pub use wellfed_core::settings::{
    KindSettings, SaveOutcome, Saved, SettingsFields, SettingsResult, SettingsView, Window,
};

use crate::cell::TaskCell;
use crate::handoff::{Handoff, Taken};
use crate::sdk::IrqGuard;

const NO_VIEW: SettingsView = view_before_load(());

/// A view no writer published: what a mailbox holds until its first.
const fn view_before_load<K>(kind: K) -> SettingsView<K> {
    SettingsView {
        lights: false,
        sound: false,
        volume: 0,
        lights_window: Window::Always,
        sound_window: Window::Always,
        kind,
        generation: 0,
        writable: false,
        saved: Saved {
            finished: 0,
            failed: 0,
        },
    }
}

/// The device's part of the last view published, for the SDK's services.
static DEVICE_VIEW: TaskCell<SettingsView> = TaskCell::new(NO_VIEW);
static DEVICE_LOADED: AtomicBool = AtomicBool::new(false);

/// The device's settings as the writer last published them, once it has
/// loaded them.
pub fn current() -> Option<SettingsView> {
    if !DEVICE_LOADED.load(Ordering::Acquire) {
        return None;
    }
    let _guard = IrqGuard::lock();
    // SAFETY: interrupts are masked, so this is the only reference.
    Some(unsafe { *DEVICE_VIEW.get() })
}

/// Publish the device's settings for the SDK's services; every
/// [`Mailbox::publish`] does it too.
pub fn publish(view: SettingsView) {
    {
        let _guard = IrqGuard::lock();
        // SAFETY: interrupts are masked, so this is the only reference.
        unsafe { *DEVICE_VIEW.get() = view };
    }
    DEVICE_LOADED.store(true, Ordering::Release);
}

/// Forget the device's published settings (host tests).
pub fn reset() {
    DEVICE_LOADED.store(false, Ordering::Release);
    let _guard = IrqGuard::lock();
    // SAFETY: interrupts are masked, so this is the only reference.
    unsafe { *DEVICE_VIEW.get() = NO_VIEW };
}

/// The settings mailbox between the device API and the writer, for a kind
/// whose own settings are `K`. One static per firmware, the kind runtime's.
pub struct Mailbox<K: KindSettings> {
    /// The queued change, from the device API to the writer.
    pending: Handoff<SettingsFields<K>>,
    /// What NOR holds, once the writer has loaded it.
    view: TaskCell<SettingsView<K>>,
    loaded: AtomicBool,
}

impl<K: KindSettings> Mailbox<K> {
    pub const fn new() -> Self {
        Self {
            pending: Handoff::new(SettingsFields::NONE),
            view: TaskCell::new(view_before_load(K::DEFAULT)),
            loaded: AtomicBool::new(false),
        }
    }

    /// What NOR holds, once the writer has loaded it.
    pub fn current(&self) -> Option<SettingsView<K>> {
        if !self.loaded.load(Ordering::Acquire) {
            return None;
        }
        let _guard = IrqGuard::lock();
        // SAFETY: interrupts are masked, so this is the only reference.
        Some(unsafe { *self.view.get() })
    }

    /// The device API's half: read, or queue one change for the writer.
    pub fn request(&self, fields: SettingsFields<K>) -> SettingsResult<K> {
        let Some(view) = self.current() else {
            return SettingsResult::Busy;
        };
        if !fields.changes() {
            let pending = self.pending.waiting();
            return SettingsResult::Current { view, pending };
        }
        if !view.writable {
            return SettingsResult::ReadOnly;
        }

        if !self.pending.offer(|pending| *pending = fields) {
            return SettingsResult::Busy;
        }
        crate::writer_wake::wake();
        SettingsResult::Current {
            view,
            pending: true,
        }
    }

    /// The writer's half: publish what NOR holds, the device's part for the
    /// SDK's services too. The outcomes of the changes finished so far
    /// stay as [`Self::finish`] left them, whatever `view` says.
    pub fn publish(&self, view: SettingsView<K>) {
        publish(view.device());
        {
            let _guard = IrqGuard::lock();
            // SAFETY: interrupts are masked, so this is the only reference.
            let held = unsafe { &mut *self.view.get() };
            *held = SettingsView {
                saved: held.saved,
                ..view
            };
        }
        self.loaded.store(true, Ordering::Release);
    }

    /// Whether a change is on its way to the store: queued, being written
    /// or taken by the writer.
    pub fn waiting(&self) -> bool {
        self.pending.waiting()
    }

    /// The writer's half: the queued change, held until [`Self::finish`];
    /// a guard dropped unfinished leaves it queued.
    pub fn take(&self) -> Option<Taken<'_, SettingsFields<K>>> {
        self.pending.take()
    }

    /// The writer applied (or gave up on) the taken change: `failed`
    /// when it saved none of it, or only part. The outcome is in the view
    /// before the mailbox frees, so a client's write waiting on it finds
    /// it as soon as nothing is pending ([`Saved::outcome`]).
    pub fn finish(&self, taken: Taken<'_, SettingsFields<K>>, failed: bool) {
        {
            let _guard = IrqGuard::lock();
            // SAFETY: interrupts are masked, so this is the only reference.
            unsafe { (*self.view.get()).saved.finish(failed) };
        }
        taken.finish();
    }

    /// Forget everything, the device's part included (host tests).
    pub fn reset(&self) {
        reset();
        self.loaded.store(false, Ordering::Release);
        {
            let _guard = IrqGuard::lock();
            // SAFETY: interrupts are masked, so this is the only reference.
            unsafe { *self.view.get() = view_before_load(K::DEFAULT) };
        }
        self.pending.clear();
    }
}

impl<K: KindSettings> Default for Mailbox<K> {
    fn default() -> Self {
        Self::new()
    }
}
