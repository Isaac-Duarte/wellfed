//! Wi-Fi credentials without a rebuild.
//!
//! - **Request half:** credentials from the device API ([`change`]) or
//!   from BLE setup ([`save`]), checked by the store's own rules, are
//!   queued for the settings writer, which saves them to NOR (storage
//!   sectors 5 and 6).
//! - **Join half:** before each join attempt the Wi-Fi task asks for
//!   credentials ([`select`]) and then reports the outcome ([`outcome`]),
//!   through the [`Joins`] this crate hands the task's driver
//!   (`wellfed_hal::link`) from the task's entry, [`wellfed_wifi_run`].
//!   Stored credentials are used until
//!   [`FALLBACK_AFTER`] attempts in a row fail without one success this boot;
//!   then the build's credentials take over, so a bad save cannot strand the
//!   feeder. A save during the boot (a device API write, or BLE setup)
//!   takes effect at the next join: the new credentials are read again and
//!   get their own run of attempts before the build's take over.
//! - **No build network:** a consumer image has no build credentials
//!   ([`BUILD_WIFI`]). Stored credentials are then used however often they
//!   fail, and with none stored there is nothing to join: the select hands
//!   the task empty credentials, which it reports as a failed attempt
//!   without scanning.
//! - **Turn:** the select takes the join's turn in the [`interlock`], waiting
//!   while a feed holds the outputs, and the outcome ends it, so no feed
//!   starts while the attempt scans, associates or waits for DHCP.
//!
//! [`interlock`]: wellfed_hal::interlock
//!
//! Nothing here writes flash.

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use wellfed_core::settings::Saved;
use wellfed_core::store::handover;
use wellfed_core::store::slot::SlotIo;
use wellfed_device::store::net::NetStore;
use wellfed_device::store::reset;
use wellfed_device_api::mailbox::{Stamp, WifiResult, WifiSource, WifiView};
use wellfed_device_api::wifi::Credentials;

use wellfed_core::wifi_task::Attempt;
use wellfed_hal::cell::TaskCell;
use wellfed_hal::handoff::{Handoff, Published, Taken};
use wellfed_hal::interlock::INTERLOCK;
pub use wellfed_hal::sdk::JoinCredentials;
use wellfed_hal::{link, sdk};

/// The credential slots within the storage region (`0x345000`).
pub const NET_BASE: u32 = wellfed_hal::board::storage::NET.first;
/// The factory reset's marker sector.
const RESET_BASE: u32 = wellfed_hal::board::storage::RESET.first;
/// Consecutive failed joins before the build's credentials take over.
pub const FALLBACK_AFTER: u32 = 5;

pub use wellfed_device::store::net::BuildWifi;

/// This image's build Wi-Fi. `wellfed-build` sets `WELLFED_BUILD_WIFI`
/// exactly when the generated header the C shim prefills from names a
/// network; the build script turns its presence into `cfg(build_wifi)`.
/// The loop reads it too (the firmware's `Platform::build_wifi`): an image
/// without one and nothing stored starts BLE setup mode at boot.
pub const BUILD_WIFI: BuildWifi = if cfg!(build_wifi) {
    BuildWifi::Present
} else {
    BuildWifi::Absent
};

/// The link-time check that the C shim and this crate agree: the shim
/// reads the one of these two symbols its header's `WELLFED_BUILD_WIFI`
/// picks, and only the matching build defines it, so a header and a
/// runtime that disagree about a build network fail the link.
#[cfg(build_wifi)]
#[unsafe(no_mangle)]
pub static WELLFED_RUNTIME_BUILD_WIFI: u8 = 1;
/// See [`BUILD_WIFI`]: the image has no build network.
#[cfg(not(build_wifi))]
#[unsafe(no_mangle)]
pub static WELLFED_RUNTIME_NO_BUILD_WIFI: u8 = 0;

/// Which credentials to use for the next join; `None` when there is
/// nothing to join (no build network and none stored).
pub fn choose(
    build: BuildWifi,
    stored: bool,
    failures: u32,
    joined_once: bool,
) -> Option<WifiSource> {
    match (build, stored, joined_once) {
        (BuildWifi::Absent, false, _) => None,
        (BuildWifi::Absent, true, _) => Some(WifiSource::Stored),
        (BuildWifi::Present, false, _) => Some(WifiSource::Build),
        (BuildWifi::Present, true, true) => Some(WifiSource::Stored),
        (BuildWifi::Present, true, false) if failures >= FALLBACK_AFTER => {
            Some(WifiSource::Fallback)
        }
        (BuildWifi::Present, true, false) => Some(WifiSource::Stored),
    }
}

/// The queued change (`None` clears), and who asked for it, for its `Wifi`
/// event; `None` for BLE setup's save.
#[derive(Clone, Copy)]
pub struct Asked {
    pub credentials: Option<Credentials>,
    pub asked: Option<Stamp>,
}

static PENDING: Handoff<Asked> = Handoff::new(Asked {
    credentials: None,
    asked: None,
});

/// What NOR holds, published by the writer as one view: the stored SSID,
/// its generation and whether it may change travel together, so a reader
/// never pairs one generation with another's credentials.
#[derive(Clone, Copy)]
struct Stored {
    ssid_crc: Option<u32>,
    generation: u32,
    writable: bool,
}

static PUBLISHED: Published<Stored> = Published::new(EMPTY);

const EMPTY: Stored = Stored {
    ssid_crc: None,
    generation: 0,
    writable: false,
};

static HANDOVER_PUT_OFF: AtomicBool = AtomicBool::new(false);
/// Set by a save after the first load; the join half then reads again.
static RELOAD: AtomicBool = AtomicBool::new(false);
/// `WifiSource` of this boot's joins, as `WifiSource::code`.
static SOURCE: AtomicU8 = AtomicU8::new(WifiSource::Build.code());
/// How the writer finished the changes it took this boot, so a client's
/// write learns whether its own save landed rather than only that the
/// writer is done. Written by the writer and read by the loop, each with
/// the scheduler paused, so the count and its failures are read together.
static SAVED: TaskCell<Saved> = TaskCell::new(Saved {
    finished: 0,
    failed: 0,
});

pub fn source() -> WifiSource {
    WifiSource::from_code(SOURCE.load(Ordering::Acquire))
}

fn view(stored: Stored) -> WifiView {
    WifiView {
        stored_ssid_crc: stored.ssid_crc,
        generation: stored.generation,
        writable: stored.writable,
        source: source(),
        // SAFETY: with the scheduler paused the writer is not inside
        // `finish`, and no interrupt handler reaches it.
        saved: sdk::unscheduled(|| unsafe { *SAVED.get() }),
    }
}

/// The store as the writer last published it, and whether a change waits.
pub fn current() -> WifiResult {
    let Some(stored) = PUBLISHED.get() else {
        return WifiResult::Busy;
    };
    WifiResult::Current {
        view: view(stored),
        pending: PENDING.waiting(),
    }
}

/// BLE setup's half: queue checked credentials for the writer.
pub fn save(credentials: Credentials) -> WifiResult {
    change(Some(credentials), None)
}

/// The device API's half: queue checked credentials, or a clear with
/// `None`, for the writer, with who asked (`asked`), whose `Wifi` event the
/// writer records as it saves them.
pub fn change(credentials: Option<Credentials>, asked: Option<Stamp>) -> WifiResult {
    let Some(stored) = PUBLISHED.get() else {
        return WifiResult::Busy;
    };
    if !stored.writable {
        return WifiResult::ReadOnly;
    }
    queue(credentials, asked, stored)
}

/// Whether a queued change still waits for the writer.
pub fn saving() -> bool {
    PENDING.waiting()
}

/// Queue the change; `loaded` is the view the caller found published,
/// which no publish withdraws.
fn queue(credentials: Option<Credentials>, asked: Option<Stamp>, loaded: Stored) -> WifiResult {
    // The view is read while the slot is claimed, before the writer can
    // take the change: its `saved.next()` is the number this change
    // finishes as, which a client's write waits on.
    let mut view = None;
    let queued = PENDING.offer(|pending| {
        *pending = Asked { credentials, asked };
        view = Some(self::view(PUBLISHED.get().unwrap_or(loaded)));
    });
    let Some(view) = view.filter(|_| queued) else {
        return WifiResult::Busy;
    };
    wellfed_hal::writer_wake::wake();
    WifiResult::Current {
        view,
        pending: true,
    }
}

/// The writer's half: the queued change and who asked, held until
/// [`finish`].
pub fn take() -> Option<Taken<'static, Asked>> {
    PENDING.take()
}

/// The writer's half: the taken change is done, saved or `failed`
/// (unarmed, or the store refused the save). Its outcome is published
/// before the slot frees, so the next change queued finishes as the next
/// number.
pub fn finish(taken: Taken<'static, Asked>, failed: bool) {
    sdk::unscheduled(|| {
        // SAFETY: with the scheduler paused no loop read of it runs, and
        // no interrupt handler reaches it.
        unsafe { (*SAVED.get()).finish(failed) };
    });
    taken.finish();
}

/// The writer publishes what NOR holds; writable only once it is `armed`.
pub fn publish<I: SlotIo>(store: &NetStore<I>, armed: bool) {
    // Health reports the low 32 bits; saturate rather than wrap to an old value.
    let generation = u32::try_from(store.generation).unwrap_or(u32::MAX);
    // Only the writer publishes, so the view it replaces is its own last.
    let previous = PUBLISHED.get();
    PUBLISHED.publish(Stored {
        ssid_crc: store
            .credentials
            .map(|c| wellfed_core::store::slot::crc32(c.ssid())),
        generation,
        writable: armed && store.writable(),
    });

    // A save after the first load: the next join reads the new credentials
    // and gives them a fresh run of attempts before any fallback.
    if previous.is_some_and(|previous| previous.generation != generation) {
        RELOAD.store(true, Ordering::Release);
    }
}

/// Forget everything (host tests).
///
/// # Safety
/// No Wi-Fi task or settings writer is running: the join state is the Wi-Fi
/// task's own, and the save outcomes the writer's.
#[cfg(not(target_os = "none"))]
pub unsafe fn reset() {
    PUBLISHED.clear(EMPTY);
    for flag in [&RELOAD, &HANDOVER_PUT_OFF] {
        flag.store(false, Ordering::Release);
    }
    SOURCE.store(WifiSource::Build.code(), Ordering::Release);
    PENDING.clear();
    // SAFETY: the caller's contract: no writer task is running either.
    unsafe { *SAVED.get() = Saved::default() };
    // SAFETY: the caller's contract: no Wi-Fi task holds `JOIN`.
    unsafe { *JOIN.get() = Join::default() };
    INTERLOCK.reset();
}

// ---- The join half, called by the Wi-Fi task only.

#[derive(Default)]
struct Join {
    opened: bool,
    stored: Option<Credentials>,
    failures: u32,
    joined_once: bool,
}

/// Touched only by the Wi-Fi task, through [`select`] and [`outcome`].
static JOIN: TaskCell<Join> = TaskCell::new(Join {
    opened: false,
    stored: None,
    failures: 0,
    joined_once: false,
});

/// The credential sectors through the flash map: reads need no lease, and
/// the join half never writes.
struct MappedNet;

impl SlotIo for MappedNet {
    fn unit_bytes(&self) -> u32 {
        sdk::SECTOR_BYTES
    }

    fn read(&self, slot: u32, offset: u32, bytes: &mut [u8]) -> bool {
        slot <= 1 && sdk::storage_read(NET_BASE + slot, offset, bytes)
    }

    fn program(&self, _slot: u32, _offset: u32, _bytes: &[u8]) -> bool {
        false
    }

    fn erase(&self, _slot: u32) -> bool {
        false
    }

    fn writable(&self) -> bool {
        false
    }
}

/// The stored credentials, read once through the flash map (no lease).
/// None while a factory reset or a handover is unfinished: the Wi-Fi task may
/// ask before the settings writer has erased the credentials it clears. An
/// image with build Wi-Fi keeps its stores through a handover.
fn stored(join: &mut Join) -> Option<Credentials> {
    if !join.opened {
        let mut marker = [0; reset::MARKER.len()];
        let waiting = sdk::storage_read(RESET_BASE, 0, &mut marker)
            && (reset::marked(&marker)
                || (!cfg!(build_wifi)
                    && handover::marked(&marker)
                    && !HANDOVER_PUT_OFF.load(Ordering::Acquire)));
        // Asked again at the next join, by when the writer has finished.
        if waiting {
            return None;
        }
        join.opened = true;
        let mut store = NetStore::default();
        store.open(MappedNet);
        join.stored = store.credentials;
    }
    join.stored
}

/// The writer could not finish a handover this boot, so what is stored stands
/// until the next: this boot joins with it rather than with nothing.
pub fn handover_put_off() {
    HANDOVER_PUT_OFF.store(true, Ordering::Release);
}

/// Before each join attempt: take the join's turn (waiting while a feed
/// holds the outputs), then replace the build's credentials in `out` with
/// stored ones unless the fallback has taken over. With nothing to join,
/// `out` is emptied (a zero SSID length), which the task reports as a
/// failed attempt without scanning.
///
/// # Safety
/// Called only from the Wi-Fi task, which may block here.
pub unsafe fn select(build: BuildWifi, out: &mut JoinCredentials) {
    INTERLOCK.begin_join(sdk::uptime_ms, sdk::delay_ms);

    // SAFETY: only the Wi-Fi task calls this.
    let join = unsafe { &mut *JOIN.get() };
    if RELOAD.swap(false, Ordering::AcqRel) {
        *join = Join::default();
    }
    let stored = stored(join);
    let choice = choose(build, stored.is_some(), join.failures, join.joined_once);
    // Nothing to join reads as the build's source, as before any join: the
    // device API's view has no value for it.
    let code = choice.unwrap_or(WifiSource::Build).code();
    SOURCE.store(code, Ordering::Release);
    out.source = u32::from(code);
    let Some(choice) = choice else {
        out.ssid = core::ptr::null();
        out.ssid_len = 0;
        out.psk = core::ptr::null();
        out.psk_len = 0;
        return;
    };
    // JOIN is static, so the pointers live on.
    if let (WifiSource::Stored, Some(c)) = (choice, join.stored.as_ref()) {
        out.ssid = c.ssid().as_ptr();
        out.ssid_len = u32::try_from(c.ssid().len()).unwrap_or(0);
        out.psk = c.psk().as_ptr();
        out.psk_len = u32::try_from(c.psk().len()).unwrap_or(0);
    }
}

/// After each join attempt: ends the join's turn, and counts the attempt
/// toward the fallback.
///
/// # Safety
/// Called only from the Wi-Fi task, which owns `JOIN`, and never while a
/// [`select`] call is running.
pub unsafe fn outcome(attempt: Attempt) {
    INTERLOCK.end_join();

    // SAFETY: the caller's contract makes this the only reference to `JOIN`.
    let join = unsafe { &mut *JOIN.get() };
    if source() != WifiSource::Stored {
        return;
    }
    if attempt == Attempt::Joined {
        join.joined_once = true;
        join.failures = 0;
    } else if !link::radio_off() {
        // An attempt battery mode cut short says nothing about the
        // credentials, so it never counts toward the fallback.
        join.failures = join.failures.saturating_add(1);
    }
}

/// The product's half of the Wi-Fi task's joins, for its driver.
pub struct Joins;

impl link::Join for Joins {
    unsafe fn select(&mut self, out: &mut JoinCredentials) {
        // SAFETY: the trait's contract: the Wi-Fi task.
        unsafe { select(BUILD_WIFI, out) }
    }

    unsafe fn outcome(&mut self, attempt: Attempt) {
        // SAFETY: the trait's contract: the Wi-Fi task, after its select.
        unsafe { outcome(attempt) }
    }
}

/// The C Wi-Fi task's body (`wellfed_wifi` in `example_entry.c`), which
/// ends the task when this returns. Exported from here, not from
/// `wellfed_hal::link` where the driver is, because the task needs this
/// crate's [`Joins`] and the SDK may not depend on the product; every image
/// with the Wi-Fi shim links this crate.
///
/// # Safety
/// Called once, from the C Wi-Fi task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wellfed_wifi_run() {
    // SAFETY: the caller's contract.
    unsafe { link::run(Joins) }
}
