//! Who asked for the settings change queued in `wellfed_hal::settings`,
//! which the settings writer's `Settings` event names. The SDK's mailbox
//! carries the settings alone, so the stamp waits beside it here.
//!
//! The loop task is the only one that queues a settings change, and only it
//! moves the mailbox from empty to queued. So a mailbox it finds empty
//! stays empty until its own request: the stamp written then is the one
//! the writer takes with that change, and the writer takes it only while
//! the change is queued, before it frees the mailbox.

use wellfed_device_api::mailbox::{SettingsFields, SettingsResult, Stamp};
use wellfed_hal::cell::TaskCell;
use wellfed_hal::settings;

/// Written by the loop only while the mailbox is empty, taken by the writer
/// only while a change is queued.
static SETTINGS: TaskCell<Option<Stamp>> = TaskCell::new(None);

/// The loop's half: [`settings::request`], with who asked for a change.
pub fn settings_request(fields: SettingsFields, asked: Option<Stamp>) -> SettingsResult {
    if fields.changes() && settings::pending().is_none() {
        // SAFETY: the mailbox is empty, so the writer is not reading this
        // (see the module's docs), and only the loop task writes it.
        unsafe { *SETTINGS.get() = asked };
    }
    settings::request(fields)
}

/// The writer's half: take who asked for the queued settings change, while
/// [`settings::pending`] holds it, so a later change queued straight into
/// the SDK's mailbox, with no stamp, finds none.
pub fn take_settings() -> Option<Stamp> {
    settings::pending()?;
    // SAFETY: a change is queued, so the loop does not touch this until
    // the writer frees the mailbox.
    unsafe { (*SETTINGS.get()).take() }
}
