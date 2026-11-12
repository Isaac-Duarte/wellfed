//! The time settings between the loop and the writer: the owner's zone and
//! the NTP server.
//!
//! The device API queues a whole new settings value ([`change`]); the
//! writer saves it to NOR (storage sectors 27 and 28) and publishes what the
//! store holds ([`publish`]), which the loop reads each pass to set the
//! clock's zone rule and the NTP exchange's server. Nothing here writes
//! flash.

use wellfed_core::store::slot::SlotIo;
use wellfed_device::store::time::TimeStore;
use wellfed_device_api::mailbox::{NtpServer, TimeSettings, TimeSettingsResult, TimeSettingsView};
use wellfed_hal::handoff::{Handoff, Published, Taken};

/// The time settings' slots within the storage region.
pub const TIME_BASE: u32 = wellfed_hal::board::storage::TIME.first;

/// The queued change, from the device API to the writer.
static PENDING: Handoff<TimeSettings> = Handoff::new(TimeSettings {
    zone: None,
    ntp: NtpServer::Off,
});

/// What NOR holds, published by the writer as one view, so a reader never
/// pairs one generation with another's settings.
static PUBLISHED: Published<TimeSettingsView> = Published::new(EMPTY);

const EMPTY: TimeSettingsView = TimeSettingsView {
    settings: TimeSettings {
        zone: None,
        ntp: NtpServer::Off,
    },
    generation: 0,
    writable: false,
};

/// The settings as the writer last published them, and whether a change
/// waits; `Busy` until the writer has opened the store.
pub fn current() -> TimeSettingsResult {
    let Some(view) = PUBLISHED.get() else {
        return TimeSettingsResult::Busy;
    };
    TimeSettingsResult::Current {
        view,
        pending: PENDING.waiting(),
    }
}

/// The device API's half: queue the whole new settings for the writer.
pub fn change(settings: TimeSettings) -> TimeSettingsResult {
    let Some(view) = PUBLISHED.get() else {
        return TimeSettingsResult::Busy;
    };
    if !view.writable {
        return TimeSettingsResult::ReadOnly;
    }
    if !PENDING.offer(|pending| *pending = settings) {
        return TimeSettingsResult::Busy;
    }
    wellfed_hal::writer_wake::wake();
    TimeSettingsResult::Current {
        view,
        pending: true,
    }
}

/// The writer's half: the queued change, held until the guard is finished.
pub fn take() -> Option<Taken<'static, TimeSettings>> {
    PENDING.take()
}

/// The writer publishes what NOR holds; writable only once it is `armed`.
pub fn publish<I: SlotIo>(store: &TimeStore<I>, armed: bool) {
    PUBLISHED.publish(TimeSettingsView {
        settings: store.settings,
        // Reported as 32 bits; saturate rather than wrap to an old value.
        generation: u32::try_from(store.generation).unwrap_or(u32::MAX),
        writable: armed && store.writable(),
    });
}

/// Forget everything (host tests).
pub fn reset() {
    PUBLISHED.clear(EMPTY);
    PENDING.clear();
}
