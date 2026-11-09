//! Who asked for the settings change queued in the settings mailbox
//! (`wellfed_hal::settings::Mailbox`, the kind runtime's one static), which
//! the settings writer's `Settings` event names. The SDK's mailbox carries
//! the settings alone, so the stamp waits beside it here.
//!
//! The loop task is the only one that queues a settings change, and only it
//! moves the mailbox from empty to queued. So a mailbox it finds empty
//! stays empty until its own request: the stamp written then is the one
//! the writer reads with that change. The writer reads it while the change
//! is queued, as many times as it must wait for its write turn, and clears
//! it as it frees the mailbox ([`finish_settings`]). A firmware has one
//! mailbox, so one stamp serves it.

use wellfed_device_api::mailbox::Stamp;
use wellfed_hal::cell::TaskCell;
use wellfed_hal::settings::{KindSettings, Mailbox, SettingsFields, SettingsResult};

/// Written by the loop only while the mailbox is empty, read and cleared
/// by the writer only while a change is queued.
static SETTINGS: TaskCell<Option<Stamp>> = TaskCell::new(None);

/// The loop's half: [`Mailbox::request`], with who asked for a change.
///
/// The stamp goes in before the request, on purpose: the writer may take
/// the change the moment it is queued, and must find its stamp there. A
/// request then refused (a read-only store, or no published settings yet)
/// leaves a stamp beside an empty mailbox. That is harmless: the loop is
/// the only caller, and overwrites it with its next change's before that
/// change is queued.
pub fn settings_request<K: KindSettings>(
    mailbox: &Mailbox<K>,
    fields: SettingsFields<K>,
    asked: Option<Stamp>,
) -> SettingsResult<K> {
    if fields.changes() && mailbox.pending().is_none() {
        // SAFETY: the mailbox is empty, so the writer is not reading this
        // (see the module's docs), and only the loop task writes it.
        unsafe { *SETTINGS.get() = asked };
    }
    mailbox.request(fields)
}

/// The writer's half: who asked for the queued settings change, while
/// [`Mailbox::pending`] holds it. It stays until [`finish_settings`], so
/// a change that waits a turn for the outputs still has it when it saves.
pub fn settings<K: KindSettings>(mailbox: &Mailbox<K>) -> Option<Stamp> {
    mailbox.pending()?;
    // SAFETY: a change is queued, so the loop does not touch this until
    // the writer frees the mailbox.
    unsafe { *SETTINGS.get() }
}

/// The writer's half: [`Mailbox::finish`] with whether the change
/// `failed`, clearing the stamp first. A
/// stamp left behind would be read with the next change queued straight
/// into the SDK's mailbox without one (a host test's), and name a client
/// who never asked for it.
pub fn finish_settings<K: KindSettings>(mailbox: &Mailbox<K>, failed: bool) {
    if mailbox.pending().is_some() {
        // SAFETY: the change is still queued, so the loop does not touch
        // this until `finish` below frees the mailbox.
        unsafe { *SETTINGS.get() = None };
    }
    mailbox.finish(failed);
}
