//! The device settings writer, normal role, in the SDK runtime's service
//! task (`wellfed_store_step`, which the device kind's runtime defines over
//! its [`Product`]). It opens the device's settings record in storage
//! sectors 2 and 3, and the kind's beside it ([`Product::Settings`]),
//! publishes both as one view for the device API (the kind runtime's
//! `wellfed_hal::settings::Mailbox`), arms the flash-lock hooks once as the
//! confirmation does, and then saves each change the device API queues: the
//! device's part to its record and the kind's to the kind's, in one write
//! turn. A boot that finds the two records apart saves both again once,
//! before it takes any change (`wellfed_device::store::settings::repair`),
//! and so does a failed save that leaves them apart, after a pause.
//! Each save sets the arming attempt marker and clears it after, so a hang
//! inside a save resets at most once per power cycle.
//!
//! Once armed it also keeps the keys. It mixes the seed stored across
//! boots into the generator and stores the next one (sectors 7 and 8), and
//! opens the identity and client store (sectors 9 and 10), creating the
//! device's identity secret on the first boot that has a generator. It
//! publishes the identity for the loop task and applies the client changes
//! the loop queues ([`keys`]): enrollments, revocations and the
//! Reset, the factory reset.
//!
//! The factory reset (`wellfed_device::store::reset`) is split across a
//! restart so a power cut anywhere leaves it safe. The Reset change
//! programs the marker (sector 36) and forgets every client; the loop then
//! restarts. A boot that finds the marker opens and publishes nothing the
//! reset clears, erases each cleared sector (every one off
//! `storage::FACTORY_KEPT`) as its own step, erases an update store it
//! cannot read when the owner asked for the reset by hand, lets the product
//! rewrite what it keeps of a cleared store if it keeps any
//! ([`Product::keep_on_reset`]), erases the marker last, and only then opens
//! the stores, empty. The feed journal, a readable update store and the
//! seed are never touched, nor any store the product opens as kept. Each
//! of its steps, and each of a handover's that writes, waits until the
//! watchdog feeder has checkpointed since the last (`supervision::Pace`), so
//! a feed period holds one step's leases: every lease stops the tick the
//! feeder's wait is counted in, so forty erase steps a millisecond apart
//! once held off its checkpoint past the watchdog's progress gap.
//!
//! It keeps the time settings too (sectors 27 and 28,
//! `wellfed_device::store::time`): the owner's zone and the NTP server,
//! published for the loop ([`time`]).
//!
//! It also keeps the event history (sectors 13-24,
//! `wellfed_device::store::events`). It appends the records the loop queues
//! and answers the device API's reads ([`events`]), polling
//! faster while either waits, so a reconnecting client catches up in
//! seconds.
//!
//! A device kind's own stores (the feeder's calibration, plans, meal call,
//! traces and statistics) are its [`Product`]'s: the writer opens,
//! publishes and steps them at fixed points of its own order, statically,
//! so the stack audit follows every call.
//!
//! No write starts while a feed may move the outputs: a flash lease then
//! fails its stopped-outputs check and resets the board. So each store write
//! first takes the write turn in `wellfed_hal::interlock`. While a feed
//! holds the outputs the work stays queued (a client change, a Wi-Fi save,
//! the event queue in order) and the writer asks again each poll, so it runs
//! once the owner is at rest with its outputs stopped, before the next feed.
//! Reads of the event ring are answered meanwhile: they take no lease.
//!
//! Queuing a mailbox wakes a writer at rest (`wellfed_hal::writer_wake`), so
//! a write starts without waiting out [`POLL_MS`]; that poll stays, and is
//! all the work relies on. A wake cuts short only the delay of a step that did
//! nothing, so a write's own delay stands: [`POLL_MS`] after every mailbox
//! write that erases, an upload's chunks alone following sooner. The turn above
//! and the attempt marker are the same either way.

use core::fmt;

use wellfed_core::named_enum;

use wellfed_core::store::handover;
use wellfed_core::store::seed::{SeedStatus, SeedStore};
use wellfed_core::store::update::{UpdateStatus, UpdateStore};
use wellfed_device::store::clients::{ClientError, ClientStatus, ClientStore};
use wellfed_device::store::events::{self as ring, EMPTY, EventLog, EventsFill};
use wellfed_device::store::net::{NetStatus, NetStore};
use wellfed_device::store::reset as reset_marker;
use wellfed_device::store::settings::{
    self as settings_store, KindStore, Part, Repair, SaveBoth, Settings, SettingsStatus,
    SettingsStore, Stored, shared_generation,
};
use wellfed_device::store::time::{TimeStatus, TimeStore};
use wellfed_hal::board::storage::{
    FACTORY_CLEARED_SECTORS, HANDOVER_CLEARED_SECTORS, factory_cleared, handover_cleared,
};
use wellfed_hal::flash::park::Mmio;
use wellfed_hal::flash_lock::arming::{self, Arming, Hooks, Poll};
use wellfed_hal::flash_lock::provider::Provider;
use wellfed_hal::handoff::Taken;
use wellfed_hal::interlock::{INTERLOCK, WRITER_POLL_MS, WriteStep, WriteTurn};
use wellfed_hal::settings::{KindSettings, Mailbox};
use wellfed_hal::{rng, sdk, supervision};
use wellfed_sdk::storage::{
    CLIENTS_BASE, EVENTS_BASE, EVENTS_UNITS, FlashOps, Region, SEED_BASE, SETTINGS_BASE,
    SETTINGS_BINDING, Slots,
};

pub use crate::keys::KeyResult;
use crate::keys::{self, Change};
use crate::network::{self, NET_BASE};
use crate::time::{self, TIME_BASE};
use crate::{asked, events};
use wellfed_device_api::KeyChange as ApiChange;
use wellfed_device_api::mailbox::tells_alerts;
use wellfed_proto::api::EventBody;

/// The factory reset's marker sector (`storage::RESET`).
pub const RESET_BASE: u32 = wellfed_hal::board::storage::RESET.first;
const UPDATE_BASE: u32 = wellfed_hal::board::storage::UPDATE.first;
const CONFIRM_SECTOR: u32 = wellfed_hal::board::storage::CONFIRM.first;

pub const POLL_MS: u32 = 200;
/// Between a factory reset's erases, each its own step and write turn.
pub const RESET_POLL_MS: u32 = 1;
/// While a factory reset's or a handover's next step waits for the
/// watchdog feeder to checkpoint (`supervision::Pace`): a feed period is
/// 250 ms, so this costs at most a few milliseconds a step.
pub const PACE_POLL_MS: u32 = 10;
/// While events wait to be written or read.
pub const EVENTS_POLL_MS: u32 = 5;
/// Polls at [`EVENTS_POLL_MS`] after the last answered read: about 1 s.
/// A subscriber takes a batch one loop pass a record, and a pass that
/// sends a frame runs about 16 ms (measured), so a batch of 16 outlasts
/// a shorter linger and its next read would wait out the idle poll.
pub const READ_LINGER_POLLS: u32 = 200;
/// While a write waits for a feed to end: within the poll a claim allows a
/// waiting writer before passing it.
pub const WRITE_WAIT_POLL_MS: u32 = POLL_MS;
const _: () = assert!(WRITE_WAIT_POLL_MS <= WRITER_POLL_MS);

/// The write turn for one store write, given back when dropped.
pub struct WriteHeld;

impl Drop for WriteHeld {
    fn drop(&mut self) {
        INTERLOCK.end_write();
    }
}

named_enum! {
    /// Why a settings or Wi-Fi save did not happen (`error stage=`).
    pub enum ErrorStage {
        Unarmed = "unarmed",
        Save = "save",
        /// A factory reset's erase failed.
        Erase = "erase",
        /// The boot's repair of the settings records failed.
        Repair = "repair",
        /// The settings records share no generation, so no change is
        /// taken until a boot repairs them.
        Apart = "apart",
    }
}

named_enum! {
    /// Which settings record a settings error names (`store=`).
    pub enum SettingsPart {
        Device = "device",
        /// The device kind's own record.
        Kind = "kind",
    }
}

impl From<Part> for SettingsPart {
    fn from(part: Part) -> Self {
        match part {
            Part::Device => Self::Device,
            Part::Kind => Self::Kind,
        }
    }
}

named_enum! {
    /// What became of the device's identity secret this boot.
    pub enum Identity {
        Loaded = "loaded",
        Created = "created",
        /// No generator, a store it may not write, or unarmed.
        Missing = "missing",
        Failed = "failed",
    }
}

named_enum! {
    /// A client store change the loop queued.
    pub enum KeyChange {
        Enroll = "enroll",
        Revoke = "revoke",
        Reset = "reset",
    }
}

named_enum! {
    /// What became of the history event of a change the writer saved for
    /// someone who asked (`event=` on its record).
    pub enum WriteEvent {
        /// None was due: the change did not go through, changed nothing,
        /// was the loop's own, or was the factory reset, whose restart
        /// starts a new ring.
        None = "none",
        /// Appended in the turn that made the change.
        Appended = "appended",
        /// The ring refused it (closed, or a failed write): the change
        /// stands without its event.
        Lost = "lost",
    }
}

/// A product's own record, as a `PLCFG1` line's body says it.
pub trait Describe: Copy + fmt::Debug + PartialEq + Eq {
    /// The body of this record's `PLCFG1` line.
    fn describe(self, f: &mut impl fmt::Write) -> fmt::Result;
}

/// What a writer step logs: the device's own stores' records, or its
/// product's (`R`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Record<R> {
    /// The settings as opened: the device's record's status and
    /// generation, the device's values and the kind's as its record says
    /// them (with its own status and generation).
    Open {
        status: SettingsStatus,
        generation: u64,
        settings: Settings,
        kind: Option<R>,
    },
    Arming(arming::Event),
    Saved {
        generation: u64,
        settings: Settings,
        kind: Option<R>,
        event: WriteEvent,
    },
    /// The boot found the settings records apart and saved both again at
    /// `generation`, which they now share.
    Repaired {
        generation: u64,
    },
    /// A settings change not saved, or only the device's part, or the
    /// boot's repair failed: the store that refused or failed, and its
    /// status as the failure left it.
    Error {
        stage: ErrorStage,
        store: SettingsPart,
        status: SettingsStatus,
    },
    /// Wi-Fi credentials (sectors 5 and 6): what the store holds.
    NetOpen {
        status: NetStatus,
        generation: u64,
        stored: bool,
    },
    NetSaved {
        generation: u64,
        stored: bool,
        event: WriteEvent,
    },
    NetError {
        stage: ErrorStage,
    },
    /// The time settings (sectors 27 and 28).
    TimeOpen {
        status: TimeStatus,
        generation: u64,
    },
    TimeSaved {
        generation: u64,
    },
    TimeError {
        stage: ErrorStage,
    },
    /// The stored seed and the client store.
    Keys {
        seed: SeedStatus,
        /// The stored seed went into the generator.
        mixed: bool,
        /// The next boot's seed was stored.
        saved: bool,
        clients: ClientStatus,
        enrolled: usize,
        identity: Identity,
    },
    KeysChanged {
        change: KeyChange,
        result: KeyResult,
        enrolled: usize,
        event: WriteEvent,
    },
    /// The event ring as opened.
    Events {
        open: bool,
        last: Option<u64>,
        oldest: Option<u64>,
        epoch: u32,
    },
    /// An append failed; the record is dropped.
    EventError,
    /// The boot found the factory reset's marker: no cleared store opens
    /// until the reset has erased them all.
    ResetPending,
    /// What the product keeps of a cleared store took neither its save nor,
    /// failing that, its erase's fallback, so the reset erased it: what it
    /// meant to keep is lost, and the reset goes on.
    ResetWearLost,
    /// The factory reset, asked for by hand, erased an update store this
    /// build cannot read: the floor is 0 and no key id is retired.
    ResetUpdateCleared,
    /// The handover from stock's activation is done: Wi-Fi, the product's
    /// cleared stores and clients went, unless the build has its own Wi-Fi
    /// and keeps its bench key (`cleared`).
    HandoverDone {
        cleared: bool,
    },
    /// The handover cannot go on this boot; the next one tries again.
    HandoverStopped {
        stage: u8,
    },
    /// The factory reset erased every cleared store, then its marker.
    ResetDone {
        erased: u32,
    },
    /// The factory reset stopped for this boot: unarmed, or an erase of
    /// `sector` failed. The marker stays, so the next boot tries again.
    ResetError {
        stage: ErrorStage,
        sector: Option<u32>,
        /// The loop was asked for a restart to try again.
        restart: bool,
    },
    /// One of the product's own stores.
    Product(R),
}

/// The body of a `PLCFG1` line.
pub fn describe<R: Describe>(record: Record<R>, f: &mut impl fmt::Write) -> fmt::Result {
    let values = |f: &mut dyn fmt::Write, s: &Settings| {
        write!(
            f,
            "lights={} sound={} volume={} lights_window={} sound_window={}",
            u8::from(s.lights),
            u8::from(s.sound),
            s.volume,
            s.lights_window,
            s.sound_window,
        )
    };
    match record {
        Record::Open {
            status,
            generation,
            settings,
            kind,
        } => {
            write!(f, "open status={} generation={generation} ", status.code())?;
            values(f, &settings)?;
            if let Some(kind) = kind {
                f.write_char(' ')?;
                kind.describe(f)?;
            }
            Ok(())
        }
        Record::Arming(arming::Event::Ready { reply, inactive }) => {
            write!(
                f,
                "ready reply={} inactive={}",
                reply.code(),
                u8::from(inactive)
            )
        }
        Record::Arming(arming::Event::Armed {
            watchdog,
            installed,
        }) => write!(
            f,
            "armed watchdog={} installed={}",
            u8::from(watchdog),
            u8::from(installed)
        ),
        Record::Arming(arming::Event::Skip { marker }) => write!(f, "skip marker={marker:#x}"),
        Record::Arming(arming::Event::Timeout) => f.write_str("error stage=timeout"),
        Record::Arming(arming::Event::Clock) => f.write_str("error stage=clock"),
        Record::Saved {
            generation,
            settings,
            kind,
            event,
        } => {
            write!(f, "saved generation={generation} ")?;
            values(f, &settings)?;
            if let Some(kind) = kind {
                f.write_char(' ')?;
                kind.describe(f)?;
            }
            write!(f, " event={event}")
        }
        Record::Repaired { generation } => write!(f, "repaired generation={generation}"),
        Record::Error {
            stage,
            store,
            status,
        } => write!(
            f,
            "error stage={stage} store={store} status={}",
            status.code()
        ),
        // Never the SSID or passphrase.
        Record::NetOpen {
            status,
            generation,
            stored,
        } => write!(
            f,
            "net open status={} generation={generation} stored={}",
            status as u8,
            u8::from(stored)
        ),
        Record::NetSaved {
            generation,
            stored,
            event,
        } => write!(
            f,
            "net saved generation={generation} stored={} event={event}",
            u8::from(stored)
        ),
        Record::NetError { stage } => write!(f, "net error stage={stage}"),
        Record::TimeOpen { status, generation } => write!(
            f,
            "time open status={} generation={generation}",
            status.name()
        ),
        Record::TimeSaved { generation } => write!(f, "time saved generation={generation}"),
        Record::TimeError { stage } => write!(f, "time error stage={stage}"),
        Record::Keys {
            seed,
            mixed,
            saved,
            clients,
            enrolled,
            identity,
        } => write!(
            f,
            "keys seed={} mixed={} saved={} clients={} enrolled={enrolled} identity={identity}",
            seed_status(seed),
            u8::from(mixed),
            u8::from(saved),
            client_status(clients),
        ),
        Record::KeysChanged {
            change,
            result,
            enrolled,
            event,
        } => write!(
            f,
            "keys change={change} result={result} enrolled={enrolled} event={event}"
        ),
        Record::Events {
            open,
            last,
            oldest,
            epoch,
        } => write!(
            f,
            "events open={} last={} oldest={} epoch={epoch:08x}",
            u8::from(open),
            last.unwrap_or(0),
            oldest.unwrap_or(0)
        ),
        Record::EventError => f.write_str("events error stage=append"),
        Record::ResetPending => f.write_str("reset pending"),
        Record::ResetWearLost => f.write_str("reset wear_lost=1"),
        Record::ResetUpdateCleared => f.write_str("reset update=cleared"),
        Record::ResetDone { erased } => write!(f, "reset done erased={erased}"),
        Record::HandoverDone { cleared } => {
            write!(f, "handover done cleared={}", u8::from(cleared))
        }
        Record::HandoverStopped { stage } => write!(f, "handover stopped stage={stage}"),
        Record::ResetError {
            stage,
            sector,
            restart,
        } => {
            write!(f, "reset error stage={stage}")?;
            if let Some(sector) = sector {
                write!(f, " sector={sector}")?;
            }
            write!(f, " restart={}", u8::from(restart))
        }
        Record::Product(record) => record.describe(f),
    }
}

fn seed_status(status: SeedStatus) -> &'static str {
    match status {
        SeedStatus::Unopened => "unopened",
        SeedStatus::Empty => "empty",
        SeedStatus::Loaded => "loaded",
        SeedStatus::ReadOnly => "read-only",
        SeedStatus::Io => "io",
    }
}

fn client_status(status: ClientStatus) -> &'static str {
    match status {
        ClientStatus::Unopened => "unopened",
        ClientStatus::Empty => "empty",
        ClientStatus::Loaded => "loaded",
        ClientStatus::ReadOnly => "read-only",
        ClientStatus::Corrupt => "corrupt",
        ClientStatus::Io => "io",
    }
}

/// `delay_ms == 0` ends the task; `armed` reports the arming outcome once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step<R> {
    pub delay_ms: u32,
    pub record: Option<Record<R>>,
    /// Arming finished this step, armed or not: later writers may start.
    pub settled: bool,
    /// No record was served or appended and nothing waits for a turn, so a
    /// queued mailbox may cut this step's delay short
    /// (`wellfed_hal::writer_wake`).
    pub rested: bool,
}

/// A step after `delay_ms` that logs `record`.
pub fn next<R>(delay_ms: u32, record: Option<Record<R>>) -> Step<R> {
    Step {
        delay_ms,
        record,
        settled: false,
        rested: false,
    }
}

/// What became of a store the product keeps through a factory reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wear {
    /// Rewritten with what it keeps, or nothing to rewrite.
    Kept,
    /// The save failed, so the store was erased: what it kept is lost.
    Lost,
    /// Neither the save nor the erase took.
    Failed,
}

/// A device kind's own stores, which the writer keeps beside the device's
/// at fixed points of its order. Every call is static: the writer is
/// generic over its product, never `dyn`, so the stack audit follows each.
pub trait Product<F: FlashOps>: Default {
    /// Its own records, logged among the writer's.
    type Record: Describe;

    /// Its own settings, kept in a record of its own beside the device's
    /// (`()` for none). A `settings` request carries both; the writer saves
    /// the kind's part through [`Self::save_settings`].
    type Settings: KindSettings + 'static;

    /// The settings mailbox between the device API and the writer: the
    /// kind runtime's one static.
    fn settings_mailbox() -> &'static Mailbox<Self::Settings>;

    /// What its settings record holds, opened with its cleared stores
    /// ([`Self::open_cleared`]).
    fn settings(&self) -> Stored<Self::Settings>;

    /// Save its settings at `generation`, the one the device's record was
    /// just saved at, under the caller's write turn and attempt marker;
    /// false when the save failed or was refused.
    fn save_settings(&mut self, settings: &Self::Settings, generation: u64) -> bool;

    /// Read its settings record again from flash, after a save of either
    /// record failed.
    fn reopen_settings(&mut self);

    /// Its settings as its own record says them, for the writer's settings
    /// lines (`None` for a kind with none).
    fn settings_record(&self) -> Option<Self::Record>;

    /// First thing each boot: the stores a factory reset keeps.
    fn open_kept(&mut self);

    /// With the device's: the stores a factory reset clears.
    fn open_cleared(&mut self);

    /// After the handover: the stores it cleared, read again from their
    /// erased sectors (those [`Self::open_cleared`] opens).
    fn reopen_handed_over(&mut self);

    /// Publish each store's view; `armed` as the writer is.
    fn publish(&self, armed: bool);

    /// The event ring opened: its own rings open with it, unlogged.
    fn open_with_events(&mut self);

    /// Once the device's stores are open: its own store that logs its
    /// opening, if one has not yet (each call opens at most one).
    fn open_late(&mut self) -> Option<Self::Record>;

    /// The event ring's fill, as it changes.
    fn events_filled(fill: EventsFill);

    /// Its mailboxes, after the device's and before the settings: the step
    /// that took one, or `None` when none waits (or one waits for something
    /// other than a write turn and lets the work below go on).
    fn mailboxes<H: Hooks>(&mut self, core: &mut Core<F>) -> Option<Step<Self::Record>>;

    /// With no settings queued and before the event ring's step: its work
    /// at rest, if any. `writable` when this boot may write.
    fn rest(&mut self, core: &mut Core<F>, writable: bool) -> Option<Step<Self::Record>>;

    /// The factory reset's step after the cleared sectors and the update
    /// store's: rewrite what it
    /// keeps of a cleared store, named by its first sector, or `None` when
    /// it keeps nothing, and the marker's erase follows at once.
    fn keep_on_reset() -> Option<(u32, Wear)>;
}

/// The factory reset's update store step: erase the store if this build
/// cannot read it and the owner asked for the reset at the panel
/// (`reset::by_hand`), and leave it alone otherwise. Whether it erased,
/// and whether the step is done; an erase that fails stops the reset, and
/// one cut short leaves a store the next boot finds corrupt and erases
/// again.
fn clear_corrupt_update<F: FlashOps>(marker: &Region<F, RESET_BASE, 1>) -> (bool, bool) {
    if !reset_marker::by_hand(marker) {
        return (false, true);
    }
    let mut update = UpdateStore::default();
    update.open(Region::<F, UPDATE_BASE, 2>::new());
    match update.clear_corrupt() {
        Ok(cleared) => (cleared, true),
        Err(_) => (false, false),
    }
}

/// Whether `sector` reads erased, in small reads to spare the stack: an
/// erase that reported success but left bits behind stops the reset.
pub fn blank<F: FlashOps>(sector: u32) -> bool {
    const CHUNK: u32 = 64;
    let mut chunk = [0u8; CHUNK as usize];
    (0..sdk::SECTOR_BYTES / CHUNK).all(|i| {
        F::read(sector, i * CHUNK, &mut chunk) && chunk.iter().all(|&byte| byte == u8::MAX)
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Stage {
    #[default]
    Open,
    Arm,
    Idle,
    /// Read-only for this boot (unarmed or a store it may not write).
    Serve,
}

/// The device's own stores and the writer's state, which a [`Product`]'s
/// steps borrow for the write turn and the event ring's readers.
pub struct Core<F> {
    stage: Stage,
    arming: Arming,
    store: SettingsStore<Slots<F, SETTINGS_BASE>>,
    net: NetStore<Slots<F, NET_BASE>>,
    /// The credential store's open record, logged on the next step.
    net_opened: bool,
    time: TimeStore<Slots<F, TIME_BASE>>,
    /// The time store's open record, logged on a later step.
    time_opened: bool,
    armed: bool,
    seed: SeedStore<Slots<F, SEED_BASE>>,
    /// The epoch an event ring created this boot takes, once stored with
    /// the boot's seed ([`ring_epoch`]).
    ring_epoch: Option<u32>,
    clients: ClientStore<Slots<F, CLIENTS_BASE>>,
    keys_opened: bool,
    events: EventLog<Region<F, EVENTS_BASE, EVENTS_UNITS>>,
    events_opened: bool,
    /// A read's records, before they are copied to the loop: static, as
    /// the writer is, since the service task's stack has no room for it.
    read_batch: [ring::Record; events::READ_BATCH],
    /// Polls left at the read pace after the last answered read.
    reading: u32,
    /// Polls left at a product upload's pace ([`Core::linger`]).
    receiving: u32,
    /// This writer's wait for the write turn.
    turn: WriteTurn,
    /// A factory reset this boot finishes: the index (in
    /// `storage::factory_cleared`) of the next sector to erase; past the
    /// last, the product's keep step, then the marker itself. Until
    /// it is `None` no cleared store opens and nothing is published.
    resetting: Option<u32>,
    /// The reset could not go on this boot (unarmed, or an erase failed).
    reset_stopped: bool,
    /// A handover this boot finishes: the next of its steps. Until it is
    /// `None` nothing is published, so no one sees the Wi-Fi it clears.
    handing_over: Option<u8>,
    /// This boot recorded its attempt in the marker sector.
    reset_attempted: bool,
    /// The factory reset's and the handover's erases, one step per feed
    /// period of the watchdog feeder.
    pace: supervision::Pace,
    /// Arming skipped this boot: the boot before reset inside a write.
    skipped: bool,
    /// When the settings records are next checked for a shared generation,
    /// and repaired if they have none.
    settings_repair: RepairDue,
}

/// When the writer next checks the settings records share a generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RepairDue {
    /// At once: the boot's check.
    Boot,
    /// Once the uptime reaches this: the one retry a failed save left
    /// apart earns.
    At(u32),
    /// Not until the next boot, or the next failed save.
    Done,
}

/// How long after a failed save left the settings records apart the
/// writer repairs them again, so a transient flash failure does not leave
/// the settings read-only until a restart.
pub const REPAIR_RETRY_MS: u32 = 10_000;

impl<F> Default for Core<F> {
    fn default() -> Self {
        Self {
            stage: Stage::default(),
            arming: Arming::default(),
            store: SettingsStore::default(),
            net: NetStore::default(),
            net_opened: false,
            time: TimeStore::default(),
            time_opened: false,
            armed: false,
            seed: SeedStore::default(),
            ring_epoch: None,
            clients: ClientStore::default(),
            keys_opened: false,
            events: EventLog::default(),
            events_opened: false,
            read_batch: [EMPTY; events::READ_BATCH],
            reading: 0,
            receiving: 0,
            turn: WriteTurn::new(),
            resetting: None,
            reset_stopped: false,
            handing_over: None,
            reset_attempted: false,
            pace: supervision::Pace::default(),
            skipped: false,
            settings_repair: RepairDue::Boot,
        }
    }
}

/// Writes through `F`, the storage region's mediated flash, for the device
/// and its product `P`.
pub struct Writer<F, P> {
    core: Core<F>,
    product: P,
}

impl<F, P: Default> Default for Writer<F, P> {
    fn default() -> Self {
        Self {
            core: Core::default(),
            product: P::default(),
        }
    }
}

/// A store's generation as its event names it: the low 32 bits, saturated
/// rather than wrapped back to an old value, as the published views do.
fn generation(generation: u64) -> u32 {
    u32::try_from(generation).unwrap_or(u32::MAX)
}

/// The epoch a ring created after `last` takes: past it by a random
/// stride, so the epochs the seed record carries only rise and no ring
/// takes one an earlier ring had (a client's history keys its events by
/// epoch and number, and keeps the first it hears of each). With none
/// stored yet it is drawn whole. Never 0, which a ring written before
/// epochs reads as.
fn ring_epoch(last: u32, random: u32) -> u32 {
    let next = if last == 0 {
        random
    } else {
        last.wrapping_add(1 + (random & 0xffff))
    };
    next.max(1)
}

/// A random word from the generator, 0 without one.
fn random_word() -> u32 {
    let mut bytes = [0; 4];
    if rng::fill(&mut bytes) {
        u32::from_le_bytes(bytes)
    } else {
        0
    }
}

impl<F: FlashOps> Core<F> {
    /// The write turn for one store write, or `None` while a feed holds the
    /// outputs: the caller then leaves its work queued.
    pub fn take_turn(&mut self) -> Option<WriteHeld> {
        let stopped = || wellfed_hal::stopped::observe().verified();
        match self.turn.begin(&INTERLOCK, sdk::uptime_ms(), stopped) {
            WriteStep::Go(_) => Some(WriteHeld),
            WriteStep::Wait => None,
        }
    }

    /// The write turn asked for as if the outputs never stopped, so a claim
    /// past its bound, which lets another store's write go ahead, keeps
    /// this one waiting; `None` while it waits. `now` is the caller's
    /// uptime, which its write also goes by.
    pub fn take_released_turn(&mut self, now: u32) -> Option<WriteHeld> {
        match self.turn.begin(&INTERLOCK, now, || false) {
            WriteStep::Go(_) => Some(WriteHeld),
            WriteStep::Wait => None,
        }
    }

    /// This boot may not write: read-only for the boot, or unarmed.
    pub fn unarmed<H: Hooks>(&self) -> bool {
        self.stage == Stage::Serve || !H::armed()
    }

    /// The hooks were armed when the stores were last published.
    pub fn armed(&self) -> bool {
        self.armed
    }

    /// Keep the event ring's step at the upload pace for `polls` more polls,
    /// so a product's next chunk follows within a round trip.
    pub fn linger(&mut self, polls: u32) {
        self.receiving = polls;
    }

    /// The step while a write waits for its turn: the event ring's readers
    /// are still answered, since a read takes no lease.
    pub fn waiting<R>(&mut self) -> Step<R> {
        let delay = if self.answer_read() {
            EVENTS_POLL_MS
        } else {
            WRITE_WAIT_POLL_MS
        };
        next(delay, None)
    }

    /// Publish every store's view, the product's after the device's;
    /// nothing while a factory reset or the handover is unfinished, so no
    /// client, store or credential it clears is served.
    pub fn publish<P: Product<F>>(&self, product: &P) {
        if self.resetting.is_some() || self.handing_over.is_some() {
            return;
        }
        let writable = self.armed && self.store.status.writable();
        let view = self
            .store
            .settings
            .view(self.store.read(), writable, product.settings());
        P::settings_mailbox().publish(view);
        network::publish(&self.net, self.armed);
        time::publish(&self.time, self.armed);
        product.publish(self.armed);
    }

    /// Answer the read the device API asked for, if any; true while a
    /// reader is about, so the next poll comes at the read pace. A reader
    /// catching up asks again within a few loop passes of its last batch,
    /// and the idle poll would make it wait [`POLL_MS`] for each.
    fn answer_read(&mut self) -> bool {
        let Some(after) = events::read_request() else {
            self.reading = self.reading.saturating_sub(1);
            return self.reading > 0;
        };
        let batch = self.events.read_batch(after, &mut self.read_batch);
        events::finish_read(after, batch, &self.read_batch);
        self.reading = READ_LINGER_POLLS;
        true
    }

    /// The keys: mix the stored seed in and store the next, then open
    /// the client store and create the identity if it has none. Writes only
    /// when `writable` (armed, normal stage), each under its own attempt
    /// marker.
    fn keys<R>(&mut self, writable: bool) -> Record<R> {
        self.seed.open(Slots::new());
        let mixed = self.seed.seed().is_some_and(|seed| rng::mix(seed));
        // The ring's epoch moves on with every boot's seed, stored before
        // any ring could take it. Without a generator the stored seed stays
        // (no boot has used it yet) and only the epoch moves. With neither,
        // nothing is stored: such a boot opens no session (every handshake
        // needs the generator), and no earlier boot did either, so no client
        // has heard a ring an epoch of 1 could repeat.
        let epoch = ring_epoch(self.seed.epoch(), random_word());
        let next = rng::bytes();
        let seed = next.or(self.seed.seed().copied());
        let stored = writable
            && seed.is_some_and(|seed| {
                arming::set_attempt();
                let stored = self.seed.save(&seed, epoch);
                arming::clear_attempt();
                stored
            });
        let saved = stored && next.is_some();
        self.ring_epoch = stored.then_some(epoch);

        self.clients.open(Slots::new());
        let identity = if self.clients.identity().is_some() {
            Identity::Loaded
        } else if let Some(secret) = rng::bytes().filter(|_| writable && self.clients.writable()) {
            arming::set_attempt();
            let created = self.clients.set_identity(&secret).is_ok();
            arming::clear_attempt();
            if created {
                Identity::Created
            } else {
                Identity::Failed
            }
        } else {
            Identity::Missing
        };
        if let Some(secret) = self.clients.identity() {
            keys::publish_identity(secret);
        }
        keys::publish_clients(self.clients.clients().copied());

        Record::Keys {
            seed: self.seed.status,
            mixed,
            saved,
            clients: self.clients.status,
            enrolled: self.clients.clients().count(),
            identity,
        }
    }

    /// Apply one queued client change, each write under its own attempt
    /// marker, and free the mailbox with its result.
    ///
    /// A reset is the factory reset. It draws its new identity from the
    /// generator or does not happen: a weaker key is never written. It
    /// programs the reset's marker first, so whatever happens next the boot
    /// after finishes it, then forgets every client at once (their sessions
    /// end now, not at the restart). Once the marker is down the answer is
    /// `Done`: the loop restarts the device, and that boot erases the
    /// cleared stores.
    ///
    /// An enrollment or revocation that went through records its history
    /// event here, in the same write turn, before the result is published:
    /// whoever asked may have gone (a session that dropped, a BLE link that
    /// closed), and the device, not their survival, decides what the
    /// history holds. A waiter that reads `Done` finds the event already in
    /// the ring. The store write goes first and the event second, so a
    /// power cut between the two programs leaves the change without its
    /// event, never an event for a change that did not land: that one
    /// would be false, and the client's retry would record it twice. The
    /// event comes straight to the ring, not through the loop's queue, so a
    /// full queue cannot drop it either; the loop's alert rules hear of it
    /// through [`events::take_written`].
    fn change<P: Product<F>, R>(
        &mut self,
        taken: Taken<'static, Change>,
        writable: bool,
    ) -> Record<R> {
        let change = *taken.get();
        let kind = match change {
            Change::Key {
                change: ApiChange::Enroll { .. },
                ..
            } => KeyChange::Enroll,
            Change::Key {
                change: ApiChange::Revoke { .. },
                ..
            } => KeyChange::Revoke,
            Change::Reset => KeyChange::Reset,
        };
        let result = match change {
            _ if !writable => KeyResult::Unarmed,
            Change::Key {
                change:
                    ApiChange::Enroll {
                        key, label, role, ..
                    },
                ..
            } => self.apply(|clients| clients.enroll(key, label, role)),
            Change::Key {
                change: ApiChange::Revoke { key, .. },
                ..
            } => self.apply(|clients| clients.revoke(&key)),
            Change::Reset => match rng::bytes() {
                Some(identity) => {
                    arming::set_attempt();
                    let marked = reset_marker::mark(&Region::<F, RESET_BASE, 1>::new());
                    arming::clear_attempt();
                    if marked {
                        // Committed: the next boot erases the client store
                        // whatever this write does, so a store it cannot
                        // write (read-only, torn) still answers `Done` and
                        // the loop restarts. A marker left down unanswered
                        // would clear everything on some later, unrelated
                        // boot.
                        self.apply(|clients| clients.reset(&identity));
                        KeyResult::Done
                    } else {
                        KeyResult::Io
                    }
                }
                None => KeyResult::NoGenerator,
            },
        };

        let event = match change {
            Change::Key { change, at } if result == KeyResult::Done => match change.invited() {
                Some(invited) => match self.append_written::<P>(at, &invited) {
                    WriteEvent::Appended => self.append_written::<P>(at, &change.event()),
                    lost => lost,
                },
                None => self.append_written::<P>(at, &change.event()),
            },
            Change::Key { .. } | Change::Reset => WriteEvent::None,
        };

        let reset = kind == KeyChange::Reset && result == KeyResult::Done;
        if reset && let Some(secret) = self.clients.identity() {
            keys::publish_identity(secret);
        }
        if reset {
            // No session outlives a reset, even one the store kept.
            keys::publish_clients(core::iter::empty());
        } else {
            keys::publish_clients(self.clients.clients().copied());
        }
        keys::finish(taken, result);
        Record::KeysChanged {
            change: kind,
            result,
            enrolled: self.clients.clients().count(),
            event,
        }
    }

    /// Append `body`, the history event of a change the writer has just
    /// saved for someone who asked (a client, or BLE setup), straight to the
    /// ring under its own attempt marker; [`WriteEvent::Lost`] if the ring
    /// refused it. The caller holds the write turn it saved the change
    /// under. A client's enrollment or revocation is also handed to the
    /// loop's alert rules ([`events::wrote`]), the only events written here
    /// that they act on.
    ///
    /// Every such event goes after its store write, in the same turn: a cut
    /// between the two leaves the change without its event, never an event
    /// for a change that did not land, which a retry would then record
    /// again. It comes straight to the ring, not through the loop's queue,
    /// so a full queue cannot drop it, and nobody has to stay to read the
    /// result for it to be recorded.
    ///
    /// It is in the ring before the result is published, so a waiter that
    /// reads `Done` finds it. That costs the turn one more record program
    /// (under a millisecond), and, when the append opens the ring's next
    /// unit, that unit's sector erase (about 30 to 45 ms): the write turn,
    /// and a feed or a client's answer waiting on it, is held that much
    /// longer.
    pub fn append_written<P: Product<F>>(
        &mut self,
        at: Option<u32>,
        body: &EventBody,
    ) -> WriteEvent {
        let mut bytes = [0; ring::BODY_BYTES];
        if wellfed_proto::encode(body, &mut bytes).is_err() {
            events::append_failed();
            return WriteEvent::Lost;
        }

        arming::set_attempt();
        let appended = self.events.append(at, &bytes);
        arming::clear_attempt();

        if appended.is_err() {
            events::append_failed();
            return WriteEvent::Lost;
        }
        events::landed();
        events::wrote(at, bytes, self.events.last_seq(), tells_alerts(body));
        P::events_filled(EventsFill {
            kept: self.events.kept(),
            capacity: self.events.capacity(),
        });
        WriteEvent::Appended
    }

    fn apply(
        &mut self,
        write: impl FnOnce(&mut ClientStore<Slots<F, CLIENTS_BASE>>) -> Result<(), ClientError>,
    ) -> KeyResult {
        arming::set_attempt();
        let result = write(&mut self.clients);
        arming::clear_attempt();
        result.map_or_else(KeyResult::from, |()| KeyResult::Done)
    }

    /// Open the device's stores the Open stage serves that a factory reset
    /// clears; the settings store's status.
    fn open_cleared(&mut self) -> SettingsStatus {
        let status = self.store.open(Slots::new(), SETTINGS_BINDING);
        self.net.open(Slots::new());
        self.time.open(Slots::new());
        status
    }

    /// The generation both settings records share, which the device API
    /// reports; `None` while they share none
    /// (`wellfed_device::store::settings::shared_generation`).
    fn settings_generation<P: Product<F>>(&self, product: &P) -> Option<u64> {
        shared_generation(self.store.read(), &product.settings())
    }

    /// The reset cannot go on this boot. A boot that serves nothing has no
    /// identity, API, recovery window or BLE setup, so it may ask the loop
    /// for a restart (the loop waits for a feed in flight), but only once
    /// per reset: while fewer than `reset::ATTEMPTS` armed boots have
    /// started the erases (counted in the marker sector, since a boot's
    /// reset flags cannot tell a hang from an intentional restart), and only
    /// when a restart can help. That is a failed erase, or an arming skip:
    /// the boot before hung inside a write, the skip has cleared the
    /// attempt marker, and the restart arms again. Arming that failed for
    /// any other reason would fail again, so that boot waits for a power
    /// cycle.
    fn reset_stop<R>(&mut self, stage: ErrorStage, sector: Option<u32>) -> Step<R> {
        self.reset_stopped = true;
        let helps = stage == ErrorStage::Erase || self.skipped;
        let marker = Region::<F, RESET_BASE, 1>::new();
        let restart = helps && reset_marker::attempts(&marker) < reset_marker::ATTEMPTS;
        if restart {
            keys::want_restart();
        }
        let record = Record::ResetError {
            stage,
            sector,
            restart,
        };
        next(POLL_MS, Some(record))
    }
}

/// The count a queued body reports lost, if it is the loop's `Lost`.
fn lost_count(body: &[u8; ring::BODY_BYTES]) -> Option<u16> {
    match wellfed_proto::decode_padded::<EventBody>(body) {
        Some(EventBody::Lost { count }) => Some(count),
        _ => None,
    }
}

impl<F: FlashOps, P: Product<F>> Writer<F, P> {
    /// Append one queued event, or answer one read; the faster poll while
    /// either has more waiting.
    fn events_step(&mut self, writable: bool) -> Step<P::Record> {
        let core = &mut self.core;
        let mut record = None;
        let mut appended = false;
        let mut deferred = false;
        if let Some(queued) = events::pending() {
            if !writable {
                events::finish_pending(core.events.last_seq());
            } else if let Some(_turn) = core.take_turn() {
                appended = true;
                arming::set_attempt();
                let appended = core.events.append(queued.at, &queued.body);
                arming::clear_attempt();
                if appended.is_err() {
                    record = Some(Record::EventError);
                    match lost_count(&queued.body) {
                        Some(count) => events::lost_refused(count),
                        None => events::append_failed(),
                    }
                } else {
                    events::landed();
                }
                events::finish_pending(core.events.last_seq());
                self.publish_fill();
            } else {
                // Stays at the queue's head, so the records keep their order.
                deferred = true;
            }
        }
        let core = &mut self.core;
        // A reader catching up asks again as soon as it has the answer.
        let served = events::read_request().is_some();
        let read = core.answer_read();
        // An upload's next chunk comes within a round trip of the last one.
        core.receiving = core.receiving.saturating_sub(1);

        // An event a feed's turn deferred holds off everything that would
        // ask for that turn again, or the writer would contest the loop's
        // claim every poll as the feed starts. Reads take no turn.
        let writes = !deferred && (events::pending().is_some() || core.receiving > 0);
        let delay_ms = if writes || read {
            EVENTS_POLL_MS
        } else {
            POLL_MS
        };
        let mut step = next(delay_ms, record);
        step.rested = !served && !appended && !deferred;
        if step.rested {
            // Nothing left that waits for a turn (an event this boot may
            // not write was dropped): stop counting among the waiters.
            core.turn.withdraw(&INTERLOCK);
        }
        step
    }

    /// One step of the handover: every sector off `storage::HANDOVER_KEPT`
    /// (the Wi-Fi's, the plans', and any a later build added), the
    /// clients, the update store's floor, the boot confirmation, then the
    /// mark, each under its own write turn and attempt marker. A cut
    /// anywhere repeats it, since the mark goes last. A build with Wi-Fi of
    /// its own is a bench image, which pairs through the recovery window
    /// rather than Bluetooth, so it keeps the cleared sectors and its
    /// clients, but any image installed from stock has to take the same
    /// version again, and earn its floor with a real trial.
    ///
    /// Only bank 0 forgets the confirmation. KM0 reads the record at every boot
    /// but decides by bank 1's marker: with it not intact, as bank 0 boots after
    /// stock, the record changes nothing, so a cut through the erase changes no
    /// boot. On bank 1 this build's next boot would run a trial, with the Wi-Fi
    /// it needs to pass just cleared, and give way. A step that fails leaves the
    /// mark for the next boot, and this one serves what it has.
    fn handover_step<H: Hooks>(&mut self) -> Step<P::Record> {
        let Some(step) = self.core.handing_over else {
            return next(POLL_MS, None);
        };
        if self.core.unarmed::<H>() {
            return self.handover_end(Record::HandoverStopped { stage: step });
        }
        if !self.core.pace.ready() {
            return next(PACE_POLL_MS, None);
        }
        let Some(turn) = self.core.take_turn() else {
            return next(WRITE_WAIT_POLL_MS, None);
        };

        // The steps: the cleared sectors, the clients, the update store,
        // the confirmation, the mark.
        let clients = HANDOVER_CLEARED_SECTORS;
        let index = u32::from(step);
        let clears = !cfg!(build_wifi);
        // The steps that write nothing: a build with its own Wi-Fi keeps
        // its stores, and only bank 0 erases the confirmation. They need
        // no feed period after them.
        let idle = (index <= clients && !clears)
            || (index == clients + 2 && sdk::running_bank() != Some(0));
        arming::set_attempt();
        let done = if idle {
            true
        } else if let Some(sector) = handover_cleared(index) {
            F::erase(sector) && blank::<F>(sector)
        } else if index == clients {
            // A store that could not be read may still hold clients, so the
            // mark stays and the next boot tries again.
            let status = self.core.clients.open(Slots::new());
            match (status, self.core.clients.identity().copied()) {
                (ClientStatus::Io, _) => false,
                (_, Some(identity)) => self.core.clients.reset(&identity).is_ok(),
                (_, None) => true,
            }
        } else if index == clients + 1 {
            let mut update = UpdateStore::default();
            update.open(Region::<F, UPDATE_BASE, 2>::new());
            let started = match update.status {
                // A newer major's floor, or a record this build cannot
                // read, is not this build's to touch.
                UpdateStatus::ReadOnly | UpdateStatus::Corrupt => true,
                // Its floor is unknown and may still stand, so the mark
                // stays and the next boot tries again.
                UpdateStatus::Io => false,
                _ => update.start_over().is_ok(),
            };
            if started {
                wellfed_hal::ota::update::store_started_over();
            }
            started
        } else if index == clients + 2 {
            F::erase(CONFIRM_SECTOR) && blank::<F>(CONFIRM_SECTOR)
        } else {
            F::erase(RESET_BASE) && blank::<F>(RESET_BASE)
        };
        arming::clear_attempt();
        drop(turn);
        if !idle {
            self.core.pace.wrote();
        }

        if !done {
            return self.handover_end(Record::HandoverStopped { stage: step });
        }
        if index < clients + 3 {
            self.core.handing_over = Some(step + 1);
            return next(RESET_POLL_MS, None);
        }
        self.handover_end(Record::HandoverDone { cleared: clears })
    }

    fn handover_end(&mut self, record: Record<P::Record>) -> Step<P::Record> {
        self.core.handing_over = None;
        if matches!(record, Record::HandoverStopped { .. }) {
            network::handover_put_off();
        }
        // The Open stage read these before the handover erased them. The
        // event ring and the traces open after it, from erased sectors.
        self.core.store = SettingsStore::default();
        self.core.net = NetStore::default();
        self.core.time = TimeStore::default();
        self.core.open_cleared();
        self.product.reopen_handed_over();
        self.core.publish(&self.product);
        next(POLL_MS, Some(record))
    }

    /// One step of a factory reset this boot finishes: one erase, under
    /// its own write turn (so it waits for a feed's outputs to stop) and
    /// attempt marker; then, after the marker's, the cleared stores open
    /// empty and everything is published. A reset that cannot go on stops
    /// for the boot with nothing served, and the marker brings it back on
    /// the next.
    fn reset_step<H: Hooks>(&mut self) -> Step<P::Record> {
        let core = &mut self.core;
        if core.reset_stopped {
            if supervision::restart_grace() {
                // Nothing to save before a restart.
                supervision::restart_flushed();
            }
            return next(POLL_MS, None);
        }
        if core.unarmed::<H>() {
            return core.reset_stop(ErrorStage::Unarmed, None);
        }
        let Some(index) = core.resetting else {
            return next(POLL_MS, None);
        };
        if !core.pace.ready() {
            return next(PACE_POLL_MS, None);
        }
        let Some(turn) = core.take_turn() else {
            return next(WRITE_WAIT_POLL_MS, None);
        };

        let marker = Region::<F, RESET_BASE, 1>::new();
        arming::set_attempt();
        let noted = core.reset_attempted || reset_marker::note_attempt(&marker);
        let mut wear = Wear::Kept;
        let mut keeping = None;
        // Past the cleared sectors: the product's keep step, if it has one
        // (asked once, at its index), then the marker.
        let kept = if noted && index == FACTORY_CLEARED_SECTORS + 1 {
            P::keep_on_reset()
        } else {
            None
        };
        let mut marked = false;
        let mut update_cleared = false;
        let step = match factory_cleared(index) {
            _ if !noted => None,
            Some(sector) => Some((sector, F::erase(sector) && blank::<F>(sector))),
            None if index == FACTORY_CLEARED_SECTORS => {
                let (cleared, done) = clear_corrupt_update::<F>(&marker);
                // The update stage runs on the service task and may have
                // opened the corrupt record already; it reads the erased
                // store again on its next step rather than serve Corrupt
                // until the next boot.
                if cleared {
                    wellfed_hal::ota::update::store_started_over();
                }
                update_cleared = cleared;
                Some((UPDATE_BASE, done))
            }
            None => match kept {
                Some((sector, result)) => {
                    wear = result;
                    keeping = Some(sector);
                    Some((sector, wear != Wear::Failed))
                }
                None => {
                    marked = true;
                    Some((RESET_BASE, F::erase(RESET_BASE) && blank::<F>(RESET_BASE)))
                }
            },
        };
        arming::clear_attempt();
        drop(turn);
        core.pace.wrote();
        core.reset_attempted = noted;

        // An attempt that did not land is a store this boot cannot count
        // in, so it stops without asking for a restart; so does a kept
        // store that took neither its save nor its erase (an erase
        // elsewhere that failed may be retried).
        let Some((sector, done)) = step else {
            return core.reset_stop(ErrorStage::Save, Some(RESET_BASE));
        };
        if !done {
            let stage = if keeping == Some(sector) {
                ErrorStage::Save
            } else {
                ErrorStage::Erase
            };
            return core.reset_stop(stage, Some(sector));
        }
        if !marked {
            core.resetting = Some(index + 1);
            let record = if update_cleared {
                Some(Record::ResetUpdateCleared)
            } else {
                (wear == Wear::Lost).then_some(Record::ResetWearLost)
            };
            return next(RESET_POLL_MS, record);
        }
        core.resetting = None;
        self.open_cleared();
        self.core.publish(&self.product);
        let record = Record::ResetDone {
            erased: FACTORY_CLEARED_SECTORS,
        };
        next(POLL_MS, Some(record))
    }

    /// Open the stores the Open stage serves that a factory reset clears,
    /// the device's then the product's; the settings store's status.
    fn open_cleared(&mut self) -> SettingsStatus {
        let status = self.core.open_cleared();
        self.product.open_cleared();
        status
    }

    fn publish_fill(&self) {
        P::events_filled(EventsFill {
            kept: self.core.events.kept(),
            capacity: self.core.events.capacity(),
        });
    }

    fn publish(&self) {
        self.core.publish(&self.product);
    }

    /// One task iteration.
    ///
    /// # Safety
    /// `provider` is valid for the program's lifetime, stays at a fixed
    /// address and is not otherwise borrowed.
    pub unsafe fn step<M: Mmio, H: Hooks>(&mut self, provider: *mut Provider) -> Step<P::Record> {
        match self.core.stage {
            Stage::Open => {
                self.product.open_kept();
                let record = if reset_marker::pending(&Region::<F, RESET_BASE, 1>::new()) {
                    self.core.resetting = Some(0);
                    Record::ResetPending
                } else {
                    if handover::pending(&Region::<F, RESET_BASE, 1>::new()) {
                        self.core.handing_over = Some(0);
                    }
                    let status = self.open_cleared();
                    Record::Open {
                        status,
                        generation: self.core.store.generation,
                        settings: self.core.store.settings,
                        kind: self.product.settings_record(),
                    }
                };
                self.core.armed = H::armed();
                self.core.stage = if self.core.armed {
                    Stage::Idle
                } else {
                    Stage::Arm
                };
                self.publish();
                let mut step = next(POLL_MS, Some(record));
                step.settled = self.core.armed;
                step
            }

            // SAFETY: the caller's contract.
            Stage::Arm => match unsafe { self.core.arming.step::<M, H>(provider) } {
                Poll::Pending(event) => next(arming::POLL_MS, event.map(Record::Arming)),
                Poll::Armed(event) => {
                    // Armed for the rest of the boot; each save marks its own attempt.
                    arming::clear_attempt();
                    self.core.armed = H::armed();
                    self.core.stage = Stage::Idle;
                    self.publish();
                    let mut step = next(POLL_MS, Some(Record::Arming(event)));
                    step.settled = true;
                    step
                }
                Poll::Failed(event) => {
                    self.core.skipped = matches!(event, arming::Event::Skip { .. });
                    self.core.stage = Stage::Serve;
                    self.publish();
                    let mut step = next(POLL_MS, Some(Record::Arming(event)));
                    step.settled = true;
                    step
                }
            },

            Stage::Idle | Stage::Serve => self.serve::<H>(),
        }
    }

    /// The Idle and Serve stages' step.
    fn serve<H: Hooks>(&mut self) -> Step<P::Record> {
        // Before the keys, the event ring and the product's late stores
        // open: each is a store the reset clears.
        if self.core.resetting.is_some() {
            return self.reset_step::<H>();
        }
        if self.core.handing_over.is_some() {
            return self.handover_step::<H>();
        }
        let core = &mut self.core;
        if !core.net_opened {
            core.net_opened = true;
            return next(
                POLL_MS,
                Some(Record::NetOpen {
                    status: core.net.status,
                    generation: core.net.generation,
                    stored: core.net.credentials.is_some(),
                }),
            );
        }
        if !core.time_opened {
            core.time_opened = true;
            return next(
                POLL_MS,
                Some(Record::TimeOpen {
                    status: core.time.status,
                    generation: core.time.generation,
                }),
            );
        }
        let repair_due = match core.settings_repair {
            RepairDue::Boot => true,
            RepairDue::At(at) => sdk::uptime_ms().wrapping_sub(at) < u32::MAX / 2,
            RepairDue::Done => false,
        };
        if repair_due {
            // The boot's check, or the retry after a failed save: both find
            // the records as read from flash (a failed save reads them
            // again), so the repair is the same.
            let writable = core.stage == Stage::Idle && H::armed();
            let apart = core.settings_generation(&self.product).is_none();
            let turn = if writable && apart {
                let Some(turn) = core.take_turn() else {
                    return core.waiting();
                };
                Some(turn)
            } else {
                None
            };
            core.settings_repair = RepairDue::Done;
            if let Some(turn) = turn {
                let record = self.repair_settings();
                drop(turn);
                self.publish();
                if let Some(record) = record {
                    return next(POLL_MS, Some(record));
                }
            }
        }
        let core = &mut self.core;
        if !core.keys_opened {
            let writable = core.stage == Stage::Idle && H::armed();
            // The seed and, on a first boot, the identity: one turn.
            let turn = if writable {
                let Some(turn) = core.take_turn() else {
                    return core.waiting();
                };
                Some(turn)
            } else {
                None
            };
            core.keys_opened = true;
            let record = core.keys(writable);
            drop(turn);
            return next(POLL_MS, Some(record));
        }
        if !core.events_opened {
            core.events_opened = true;
            // The product's rings open with the event ring, unlogged: each
            // record they take logs its own line.
            self.product.open_with_events();
            let core = &mut self.core;
            // Without a stored one (a boot that may not write), the epoch
            // still moves past the last stored; only the next boot's could
            // then meet it, by a 1 in 65,536 stride.
            let fresh = core
                .ring_epoch
                .unwrap_or_else(|| ring_epoch(core.seed.epoch(), random_word()));
            core.events
                .open(Region::new(), EVENTS_UNITS as usize, fresh);
            events::publish_open(core.events.epoch(), core.events.last_seq());
            self.publish_fill();
            let core = &self.core;
            let record = Record::Events {
                open: core.events.is_open(),
                last: core.events.last_seq(),
                oldest: core.events.oldest_seq(),
                epoch: core.events.epoch(),
            };
            return next(POLL_MS, Some(record));
        }
        if let Some(record) = self.product.open_late() {
            return next(POLL_MS, Some(Record::Product(record)));
        }
        let core = &mut self.core;
        // A change left untaken (its guard dropped) waits for the next pass.
        if let Some(taken) = keys::take() {
            let writable = core.stage == Stage::Idle && H::armed();
            let turn = if writable {
                let Some(turn) = core.take_turn() else {
                    return core.waiting();
                };
                Some(turn)
            } else {
                None
            };
            let record = core.change::<P, _>(taken, writable);
            drop(turn);
            return next(POLL_MS, Some(record));
        }
        if let Some(taken) = network::take() {
            let network::Asked { credentials, asked } = *taken.get();
            let record = if core.unarmed::<H>() {
                Record::NetError {
                    stage: ErrorStage::Unarmed,
                }
            } else {
                let Some(_turn) = core.take_turn() else {
                    return core.waiting();
                };
                arming::set_attempt();
                let saved = core.net.save(credentials.as_ref());
                arming::clear_attempt();
                if saved {
                    // The client's event in the same turn, after the save
                    // ([`Core::append_written`]).
                    let event = match asked {
                        Some(asked) => {
                            let generation = generation(core.net.generation);
                            let body = EventBody::Wifi {
                                generation,
                                by: asked.by,
                            };
                            core.append_written::<P>(asked.at, &body)
                        }
                        None => WriteEvent::None,
                    };
                    Record::NetSaved {
                        generation: core.net.generation,
                        stored: core.net.credentials.is_some(),
                        event,
                    }
                } else {
                    Record::NetError {
                        stage: ErrorStage::Save,
                    }
                }
            };
            let failed = matches!(record, Record::NetError { .. });
            self.publish();
            network::finish(taken, failed);
            return next(POLL_MS, Some(record));
        }
        if let Some(taken) = time::take() {
            let settings = *taken.get();
            let record = if core.unarmed::<H>() {
                Record::TimeError {
                    stage: ErrorStage::Unarmed,
                }
            } else if core.time.unchanged(&settings) {
                Record::TimeSaved {
                    generation: core.time.generation,
                }
            } else {
                let Some(_turn) = core.take_turn() else {
                    return core.waiting();
                };
                arming::set_attempt();
                let saved = core.time.save(&settings);
                arming::clear_attempt();
                if saved {
                    Record::TimeSaved {
                        generation: core.time.generation,
                    }
                } else {
                    Record::TimeError {
                        stage: ErrorStage::Save,
                    }
                }
            };
            self.publish();
            taken.finish();
            return next(POLL_MS, Some(record));
        }
        if let Some(step) = self.product.mailboxes::<H>(&mut self.core) {
            return step;
        }
        let mailbox = P::settings_mailbox();
        let Some(taken) = mailbox.take() else {
            let core = &mut self.core;
            let writable = core.stage == Stage::Idle && H::armed();
            if let Some(step) = self.product.rest(&mut self.core, writable) {
                return step;
            }
            if !writable && supervision::restart_grace() {
                // Nothing to save before a restart.
                supervision::restart_flushed();
            }
            return self.events_step(writable);
        };
        let fields = *taken.get();
        // A change that waits for its turn is left queued as its guard drops.
        let Some((record, failed)) = self.save_settings::<H>(&fields) else {
            return self.core.waiting();
        };
        self.publish();
        asked::finish_settings(mailbox, taken, failed);
        next(POLL_MS, Some(record))
    }

    /// The boot's repair of settings records found apart, under the
    /// caller's write turn and one attempt marker: its record, if it wrote.
    /// Until a repair lands the view is read-only, so no change is taken.
    fn repair_settings(&mut self) -> Option<Record<P::Record>> {
        let core = &mut self.core;
        arming::set_attempt();
        let repair = settings_store::repair(&mut core.store, &mut KindOf::new(&mut self.product));
        arming::clear_attempt();

        match repair {
            Repair::Agreed | Repair::Refused => None,
            Repair::Repaired(generation) => Some(Record::Repaired { generation }),
            Repair::Failed { part, status } => Some(Record::Error {
                stage: ErrorStage::Repair,
                store: part.into(),
                status,
            }),
        }
    }

    /// Save a queued settings change, and whether it failed: nothing saved,
    /// or only part, or (a write may land and still report failure) all of
    /// it. `None` while the turn is not ours.
    ///
    /// Both records are written at one past the generation they share, the
    /// device's first, in one write turn and attempt marker, whichever of
    /// them the change touches (`wellfed_core::settings` says why both).
    /// When either save fails, both are read again from flash and the view
    /// names what they then hold: a generation they share, or none,
    /// read-only, until a repair [`REPAIR_RETRY_MS`] later or the next
    /// boot's. A failed save records no
    /// history event, and the client's write is refused, never `Done`.
    ///
    /// A power cut between the two saves leaves the records apart, with no
    /// event (it is appended after both) and the client's write unanswered,
    /// so uncertain: the next boot repairs them.
    fn save_settings<H: Hooks>(
        &mut self,
        fields: &wellfed_hal::settings::SettingsFields<P::Settings>,
    ) -> Option<(Record<P::Record>, bool)> {
        let core = &mut self.core;
        let device = core.store.settings.changed(fields);
        let kind_now = self.product.settings().settings;
        let kind = kind_now.changed(&fields.kind);

        if core.unarmed::<H>() {
            let record = Record::Error {
                stage: ErrorStage::Unarmed,
                store: SettingsPart::Device,
                status: core.store.status,
            };
            return Some((record, true));
        }
        let Some(shared) = core.settings_generation(&self.product) else {
            let record = Record::Error {
                stage: ErrorStage::Apart,
                store: SettingsPart::Device,
                status: core.store.status,
            };
            return Some((record, true));
        };
        if device == core.store.settings && kind == kind_now {
            let record = Record::Saved {
                generation: shared,
                settings: device,
                kind: self.product.settings_record(),
                event: WriteEvent::None,
            };
            return Some((record, false));
        }

        let _turn = core.take_turn()?;
        arming::set_attempt();
        let saved = settings_store::save_both(
            &mut core.store,
            &mut KindOf::new(&mut self.product),
            &device,
            &kind,
        );
        arming::clear_attempt();

        // The client's event in the same turn, after both saves landed, at
        // the generation they now share. The stamp is read only now, with
        // the turn held, and stays queued through any wait for it.
        let asked = asked::settings(P::settings_mailbox());
        let record = match saved {
            SaveBoth::Saved(generation) => {
                let event = match asked {
                    Some(asked) => {
                        let body = EventBody::Settings {
                            generation: self::generation(generation),
                            by: asked.by,
                        };
                        core.append_written::<P>(asked.at, &body)
                    }
                    None => WriteEvent::None,
                };
                let record = Record::Saved {
                    generation,
                    settings: core.store.settings,
                    kind: self.product.settings_record(),
                    event,
                };
                return Some((record, false));
            }
            SaveBoth::Apart => Record::Error {
                stage: ErrorStage::Apart,
                store: SettingsPart::Device,
                status: core.store.status,
            },
            SaveBoth::Failed { part, status } => {
                // Records left apart get one repair after a pause, not just
                // the next boot's; one that fails waits for the boot.
                if core.settings_generation(&self.product).is_none() {
                    let at = sdk::uptime_ms().wrapping_add(REPAIR_RETRY_MS);
                    core.settings_repair = RepairDue::At(at);
                }
                Record::Error {
                    stage: ErrorStage::Save,
                    store: part.into(),
                    status,
                }
            }
        };
        Some((record, true))
    }
}

/// A product's settings record as the settings store's [`KindStore`], for
/// the repair and the save of both records.
struct KindOf<'a, F, P> {
    product: &'a mut P,
    flash: core::marker::PhantomData<fn() -> F>,
}

impl<'a, F, P> KindOf<'a, F, P> {
    fn new(product: &'a mut P) -> Self {
        Self {
            product,
            flash: core::marker::PhantomData,
        }
    }
}

impl<F: FlashOps, P: Product<F>> KindStore for KindOf<'_, F, P> {
    type Settings = P::Settings;

    fn stored(&self) -> Stored<P::Settings> {
        self.product.settings()
    }

    fn save(&mut self, settings: &P::Settings, generation: u64) -> bool {
        self.product.save_settings(settings, generation)
    }

    fn reopen(&mut self) {
        self.product.reopen_settings();
    }
}
