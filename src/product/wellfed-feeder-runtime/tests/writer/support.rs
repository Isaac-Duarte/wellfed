//! The fake flash hooks and MMIO, the per-thread failure switches, and the
//! helpers that start a writer under the process-wide lock and step it.

use std::cell::Cell;
use std::sync::{Mutex, MutexGuard};

use wellfed_device_api::clients::{Label, Role};
use wellfed_device_api::mailbox::{SettingsFields, SettingsResult};
use wellfed_device_api::{KeyChange as ApiChange, Reader};
use wellfed_device_runtime::keys::{self, Change, Requested};
use wellfed_device_runtime::writer::{KeyChange, KeyResult, WriteEvent};
use wellfed_device_runtime::{events, network};
use wellfed_feeder_runtime::settings::SETTINGS;
use wellfed_feeder_runtime::stats;
use wellfed_feeder_runtime::writer::{FeederRecord, Record, Writer};
use wellfed_hal::board::storage::Sectors;
pub(crate) use wellfed_hal::flash::park::fake::Unreached;
use wellfed_hal::flash::peer::MARKER_REGISTER;
use wellfed_hal::flash_lock::arming::Hooks;
use wellfed_hal::flash_lock::provider::Provider;
use wellfed_hal::interlock::INTERLOCK;
use wellfed_hal::sdk::{self, fake};
use wellfed_hal::supervision;
use wellfed_proto::api::{Path, Source};
use wellfed_sdk::storage::FlashOps;

thread_local! {
    pub(crate) static ARMED: Cell<bool> = const { Cell::new(true) };
    pub(crate) static ATTEMPT_SEEN: Cell<bool> = const { Cell::new(false) };
    /// Programs fail, as a write cut short reports.
    pub(crate) static PROGRAM_FAILS: Cell<bool> = const { Cell::new(false) };
    /// Erases of this sector fail.
    pub(crate) static ERASE_FAILS: Cell<Option<u32>> = const { Cell::new(None) };
    /// Programs of the statistics' sectors fail, as a worn-out pair's do.
    pub(crate) static STATS_PROGRAMS_FAIL: Cell<bool> = const { Cell::new(false) };
    /// Programs of the event ring's sectors fail, as a power cut after
    /// another store's write would leave them unwritten.
    pub(crate) static EVENTS_PROGRAMS_FAIL: Cell<bool> = const { Cell::new(false) };
    /// Programs of these sectors fail, as a store whose writes stopped
    /// working (or one a power cut stopped before) would.
    pub(crate) static PROGRAMS_FAIL_IN: Cell<Option<Sectors>> = const { Cell::new(None) };
}

pub(crate) struct Armed;

impl Hooks for Armed {
    fn watchdog() -> bool {
        true
    }
    fn install(_: *mut Provider) -> bool {
        true
    }
    fn armed() -> bool {
        ARMED.with(Cell::get)
    }
}

impl FlashOps for Armed {
    fn install(_: *mut Provider) -> bool {
        true
    }
    fn read_id() -> [u8; 3] {
        [0; 3]
    }
    fn armed() -> bool {
        ARMED.with(Cell::get)
    }
    fn erase(sector: u32) -> bool {
        // Every write runs inside a marked attempt.
        ATTEMPT_SEEN.with(|s| s.set(sdk::backup_read(MARKER_REGISTER) != 0));
        if ERASE_FAILS.with(Cell::get) == Some(sector) {
            return false;
        }
        // SAFETY: the fake's storage, which this test owns.
        unsafe { sdk::storage_erase(sector) }
    }
    fn program(sector: u32, offset: u32, data: &[u8]) -> bool {
        let stats = wellfed_hal::board::storage::STATS;
        let worn = STATS_PROGRAMS_FAIL.with(Cell::get)
            && (stats.first..stats.first + stats.count).contains(&sector);
        let events = wellfed_hal::board::storage::EVENTS;
        let cut = EVENTS_PROGRAMS_FAIL.with(Cell::get)
            && (events.first..events.first + events.count).contains(&sector);
        let failing = PROGRAMS_FAIL_IN
            .with(Cell::get)
            .is_some_and(|s| (s.first..s.first + s.count).contains(&sector));
        if PROGRAM_FAILS.with(Cell::get) || worn || cut || failing {
            return false;
        }
        // SAFETY: as in `erase`.
        unsafe { sdk::storage_program(sector, offset, data) }
    }
    fn read(sector: u32, offset: u32, bytes: &mut [u8]) -> bool {
        sdk::storage_read(sector, offset, bytes)
    }
}

/// The settings and network views are process-wide; the fake hardware is per thread.
pub(crate) static SERIAL: Mutex<()> = Mutex::new(());

/// The subscriber most tests read for.
pub(crate) const A: Reader = Reader::slot(0);
/// A second subscriber, in another slot.
pub(crate) const B: Reader = Reader::slot(1);

pub(crate) fn start() -> (MutexGuard<'static, ()>, Writer<Armed>, Provider) {
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    fake::reset_hardware();
    SETTINGS.reset();
    // SAFETY: the test holds `SERIAL` and runs no Wi-Fi task.
    unsafe { network::reset() };
    keys::reset();
    events::reset();
    stats::reset();
    INTERLOCK.reset();
    supervision::reset();
    wellfed_hal::writer_wake::reset();
    ARMED.with(|a| a.set(true));
    PROGRAM_FAILS.with(|f| f.set(false));
    ERASE_FAILS.with(|f| f.set(None));
    STATS_PROGRAMS_FAIL.with(|f| f.set(false));
    EVENTS_PROGRAMS_FAIL.with(|f| f.set(false));
    PROGRAMS_FAIL_IN.with(|f| f.set(None));
    (guard, Writer::default(), Provider::default())
}

pub(crate) fn step(writer: &mut Writer<Armed>, provider: &mut Provider) -> Option<Record> {
    // SAFETY: the provider outlives the run.
    unsafe { writer.step::<Unreached, Armed>(provider) }.record
}

pub(crate) fn current() -> (bool, bool, u8, u32, bool) {
    match SETTINGS.request(SettingsFields::default()) {
        SettingsResult::Current { view, .. } => (
            view.lights,
            view.sound,
            view.volume,
            view.generation,
            view.writable,
        ),
        other => panic!("{other:?}"),
    }
}

/// One queued key change, applied on the writer's next step.
pub(crate) fn change(
    writer: &mut Writer<Armed>,
    provider: &mut Provider,
    change: Change,
) -> Record {
    assert_eq!(keys::request(change), Requested::Queued);
    let record = step(writer, provider).expect("the change record");
    assert_eq!(
        keys::take().map(|taken| *taken.get()),
        None,
        "the slot is free again"
    );
    record
}

/// The record of a key change; one that went through, but the reset, has
/// its event appended.
pub(crate) fn changed(change: KeyChange, result: KeyResult, enrolled: usize) -> Record {
    let event = if result == KeyResult::Done && change != KeyChange::Reset {
        WriteEvent::Appended
    } else {
        WriteEvent::None
    };
    Record::KeysChanged {
        change,
        result,
        enrolled,
        event,
    }
}

/// The time the suite's client changes are asked at.
pub(crate) const ASKED_AT: u32 = 1_700_000_000;

/// An owner's phone joining through an invite.
pub(crate) fn phone() -> Change {
    Change::Key {
        change: ApiChange::Enroll {
            key: [7; 32],
            label: Label::new(b"phone").unwrap(),
            role: Role::Owner,
            path: Path::Invite,
            inviter: None,
        },
        at: Some(ASKED_AT),
    }
}

/// The panel's owner revoking `key`.
pub(crate) fn revoke(key: [u8; 32]) -> Change {
    Change::Key {
        change: ApiChange::Revoke {
            key,
            by: Source::Client([7, 7, 7, 7]),
        },
        at: Some(ASKED_AT),
    }
}

/// Every opening step, the stats store's last; its record. The boot's own
/// save is the next step.
pub(crate) fn open_all(writer: &mut Writer<Armed>, provider: &mut Provider) -> Option<Record> {
    for _ in 0..5 {
        step(writer, provider); // open, net open, time open, keys, events
    }
    step(writer, provider)
}

pub(crate) fn saved(generation: u64, early: u8) -> Option<Record> {
    Some(Record::Product(FeederRecord::StatsSaved {
        generation,
        early,
    }))
}

/// One boot: a fresh writer opened, its own early save taken; the writer,
/// the open record and the save's.
pub(crate) fn boot(provider: &mut Provider) -> (Writer<Armed>, Option<Record>, Option<Record>) {
    stats::reset();
    supervision::reset();
    let mut writer = Writer::default();
    let open = open_all(&mut writer, provider);
    let save = step(&mut writer, provider);
    (writer, open, save)
}

/// Whether the factory reset's marker is down.
pub(crate) fn reset_marked() -> bool {
    use wellfed_device::store::reset::{MARKER, marked};

    let first = wellfed_hal::board::storage::RESET.first as usize * 0x1000;
    let bytes: [u8; MARKER.len()] =
        fake::with(|h| h.storage[first..first + MARKER.len()].try_into().unwrap());
    marked(&bytes)
}

/// The bytes of `sectors`, as the fake holds them.
pub(crate) fn sectors(sectors: wellfed_hal::board::storage::Sectors) -> Vec<u8> {
    let first = sectors.first as usize * 0x1000;
    let end = first + sectors.count as usize * 0x1000;
    fake::with(|h| h.storage[first..end].to_vec())
}

/// Every mailbox the writer publishes to, as a restart leaves them.
pub(crate) fn restart_mailboxes() {
    use wellfed_device_runtime::time;
    use wellfed_feeder_runtime::{calibration, plans, traces};

    SETTINGS.reset();
    // SAFETY: the test holds `SERIAL` and runs no Wi-Fi task.
    unsafe { network::reset() };
    keys::reset();
    events::reset();
    stats::reset();
    calibration::reset();
    plans::reset();
    wellfed_feeder_runtime::call::reset();
    time::reset();
    traces::reset();
    supervision::reset();
}

/// A boot that finds the marker: `steps` writer steps, or until the stores
/// are open and the boot's stats save is done; its records.
pub(crate) fn reset_boot(provider: &mut Provider, steps: Option<usize>) -> Vec<Record> {
    restart_mailboxes();
    let mut writer = Writer::default();
    let mut records = Vec::new();
    let mut left: Option<usize> = None;
    for taken in 0..100 {
        if steps == Some(taken) || left == Some(0) {
            return records;
        }
        let record = step(&mut writer, provider);
        left = left.map(|n| n - 1);
        if matches!(
            record,
            Some(Record::ResetDone { .. } | Record::Open { .. } | Record::HandoverDone { .. })
        ) {
            // Net, time, keys, events, stats and the stats save, or the
            // handover's steps: its cleared sectors, the clients, the
            // update store, the confirmation and the mark.
            let handover = wellfed_hal::board::storage::HANDOVER_CLEARED_SECTORS as usize + 4;
            left = Some(handover.max(8));
        }
        records.extend(record);
    }
    panic!("the boot never settled: {records:?}");
}

pub(crate) fn storage_reset_first() -> usize {
    wellfed_hal::board::storage::RESET.first as usize * 0x1000
}
