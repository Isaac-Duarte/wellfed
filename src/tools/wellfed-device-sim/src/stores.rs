//! [`DeviceStores`], the stores every device keeps, on a simulated device:
//! the firmware's own (`wellfed_device::store`) on a [`Flash`] image, with
//! the mailboxes the loop meets them through and a turn that stands in for
//! the settings writer.
//!
//! The firmware's mailboxes answer a write at once and complete it on the
//! settings task's next turn; a client polls for the result. This keeps that
//! shape: a change is queued, the device's turn applies it after the pass
//! ([`DeviceStores::apply_settings`], [`DeviceStores::apply_keys`]), and the
//! client sees it complete on a later pass. Events, which the firmware
//! queues for the writer too, are appended at once: nothing reads them
//! between the queue and the ring. Reads of the ring go through the
//! firmware's own turns ([`Reads`]), answered between passes
//! ([`DeviceStores::answer_read`]) as the writer answers them.
//!
//! A factory reset or a handover an earlier boot left marked is finished
//! before any store opens ([`finish_reset`], [`finish_handover`]), with the
//! device kind's own part of each given by the caller.

use std::collections::VecDeque;
use std::rc::Rc;

use wellfed_board::storage::{self, Sectors};
use wellfed_core::clock::wall::WallClock;
use wellfed_core::store::handover;
use wellfed_core::store::slot::{SlotIo, crc32};
use wellfed_core::store::update::{UpdateStatus, UpdateStore};
use wellfed_device::store::clients::{ClientError, ClientStatus, ClientStore};
use wellfed_device::store::events::{BODY_BYTES, EMPTY, EventLog, EventsFill, Read, Record};
use wellfed_device::store::net::NetStore;
use wellfed_device::store::reads::{READ_BATCH, Reads};
use wellfed_device::store::reset;
use wellfed_device::store::settings::{SettingsStatus, SettingsStore};
use wellfed_device::store::time::TimeStore;
use wellfed_device_api::clients::{CLIENTS, Client, Label, Role};
use wellfed_device_api::mailbox::{
    ClientChange, KEYS_FINISHED, KeyResult, Requested, SettingsFields, SettingsResult,
    SettingsView, Stamp, TimeSettings, TimeSettingsResult, TimeSettingsView, WifiResult,
    WifiSource, WifiView,
};
use wellfed_device_api::wifi::Credentials;
use wellfed_device_api::{KeyChange, Reader};
use wellfed_hal::ota::signers;
use wellfed_proto::api::EventBody;

use crate::flash::{Flash, Region};

/// This simulator's settings binding: a store written by one installation
/// is not read as another's.
const BINDING: u64 = 0x5046_5731_5349_4d01;

/// 32 bytes from the generator, or `None` when it has none.
pub type Random = Box<dyn FnMut() -> Option<[u8; 32]>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DeviceError {
    #[error("the client store cannot be written (status {0:?})")]
    Clients(ClientStatus),
    #[error("the generator has no output for the identity")]
    Random,
    #[error("enroll the client: {0:?}")]
    Enroll(ClientError),
    #[error("the client store holds no identity")]
    NoIdentity,
    #[error("the key is already enrolled as {0:?}; it keeps that role")]
    EnrolledAs(Role),
}

/// The device's stores and their mailboxes, for one boot.
pub struct DeviceStores {
    random: Random,
    flash: Rc<Flash>,
    settings: SettingsStore<Region>,
    settings_pending: Option<(SettingsFields, Option<Stamp>)>,
    net: NetStore<Region>,
    wifi_pending: Option<(Option<Credentials>, Option<Stamp>)>,
    time: TimeStore<Region>,
    time_pending: Option<TimeSettings>,
    clients: ClientStore<Region>,
    /// Bumped on every change, as the writer's published table is.
    clients_generation: u32,
    /// Bumped when the identity changes, as the writer publishes it.
    identity_generation: u32,
    keys_pending: Option<(u32, ClientChange)>,
    /// The last finished changes, newest last, as many as the firmware's
    /// mailbox remembers.
    keys_done: VecDeque<(u32, KeyResult)>,
    /// Events the writer appended itself, until the loop takes them.
    written: VecDeque<(Option<u32>, [u8; BODY_BYTES])>,
    ticket: u32,
    events: EventLog<Region>,
    /// The turns subscribers read the ring in, the firmware's own, and
    /// the writer's buffer for the batch it answers with.
    reads: Reads,
    read_batch: Box<[Record; READ_BATCH]>,
    clock: WallClock,
}

impl std::fmt::Debug for DeviceStores {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceStores")
            .field("clients", &self.clients)
            .field("last_event", &self.events.last_seq())
            .finish_non_exhaustive()
    }
}

impl DeviceStores {
    /// Open every store on `flash`, creating the identity on first use as
    /// the writer does.
    pub fn open(flash: &Rc<Flash>, mut random: Random) -> Result<Self, DeviceError> {
        let mut settings = SettingsStore::default();
        settings.open(Region::new(flash, storage::SETTINGS), BINDING);
        let mut net = NetStore::default();
        net.open(Region::new(flash, storage::NET));
        let mut time = TimeStore::default();
        time.open(Region::new(flash, storage::TIME));
        let mut clients = ClientStore::default();
        clients.open(Region::new(flash, storage::CLIENTS));
        if clients.identity().is_none() {
            if !clients.writable() {
                return Err(DeviceError::Clients(clients.status));
            }
            let secret = random().ok_or(DeviceError::Random)?;
            clients
                .set_identity(&secret)
                .map_err(|_| DeviceError::Clients(clients.status))?;
        }
        // A new ring's epoch, drawn as the writer draws it: every word of
        // the draw folded in, so a test's counting generator, which varies
        // only some bytes, still gives each new ring its own.
        let fresh = random()
            .map(|bytes| {
                bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .fold(0, |epoch, &word| epoch ^ u32::from_le_bytes(word))
            })
            .map_or(1, |epoch: u32| epoch.max(1));
        let mut events = EventLog::default();
        events.open(
            Region::new(flash, storage::EVENTS),
            storage::EVENTS.count as usize,
            fresh,
        );

        let stores = Self {
            random,
            flash: Rc::clone(flash),
            settings,
            settings_pending: None,
            net,
            wifi_pending: None,
            time,
            time_pending: None,
            clients,
            clients_generation: 1,
            identity_generation: 1,
            keys_pending: None,
            keys_done: VecDeque::with_capacity(KEYS_FINISHED),
            written: VecDeque::new(),
            ticket: 0,
            events,
            reads: Reads::new(),
            read_batch: Box::new([EMPTY; READ_BATCH]),
            clock: WallClock::new(),
        };
        stores.publish_signers();
        Ok(stores)
    }

    /// 32 bytes from the generator.
    pub fn random(&mut self) -> Option<[u8; 32]> {
        (self.random)()
    }

    /// The identity secret the client store holds.
    pub fn identity(&self) -> Option<&[u8; 32]> {
        self.clients.identity()
    }

    /// The identity, when it changed since generation `seen`.
    pub fn identity_since(&self, seen: u32) -> Option<(u32, [u8; 32])> {
        let secret = self.clients.identity()?;
        (self.identity_generation != seen).then_some((self.identity_generation, *secret))
    }

    /// Enroll `key` straight into the store, outside the protocol: how the
    /// simulator admits its first owner without BLE setup. A key already
    /// enrolled keeps the role it has, as the protocol never changes one;
    /// a different role is refused so the operator learns why.
    pub fn enroll(&mut self, key: [u8; 32], label: Label, role: Role) -> Result<(), DeviceError> {
        match self.clients.find(&key) {
            Some(client) if client.role == role => return Ok(()),
            Some(client) => return Err(DeviceError::EnrolledAs(client.role)),
            None => {}
        }
        self.clients
            .enroll(key, label, role)
            .map_err(DeviceError::Enroll)?;
        self.clients_changed();
        Ok(())
    }

    /// The wall clock the loop keeps.
    pub fn clock(&mut self) -> &mut WallClock {
        &mut self.clock
    }

    /// The clock, to read.
    pub fn wall(&self) -> &WallClock {
        &self.clock
    }

    pub fn settings(&self) -> SettingsView {
        let writable = matches!(
            self.settings.status,
            SettingsStatus::Loaded | SettingsStatus::Empty | SettingsStatus::Recovered
        );
        self.settings
            .settings
            .view(self.settings.generation, writable)
    }

    pub fn settings_request(
        &mut self,
        change: SettingsFields,
        asked: Option<Stamp>,
    ) -> SettingsResult {
        let view = self.settings();
        if !change.changes() {
            let pending = self.settings_pending.is_some();
            return SettingsResult::Current { view, pending };
        }
        if !view.writable {
            return SettingsResult::ReadOnly;
        }
        if self.settings_pending.is_some() {
            return SettingsResult::Busy;
        }
        self.settings_pending = Some((change, asked));
        SettingsResult::Current {
            view,
            pending: true,
        }
    }

    pub fn wifi(&self) -> WifiResult {
        WifiResult::Current {
            view: self.wifi_view(),
            pending: self.wifi_pending.is_some(),
        }
    }

    pub fn wifi_change(
        &mut self,
        credentials: Option<Credentials>,
        asked: Option<Stamp>,
    ) -> WifiResult {
        let view = self.wifi_view();
        if !view.writable {
            return WifiResult::ReadOnly;
        }
        if self.wifi_pending.is_some() {
            return WifiResult::Busy;
        }
        self.wifi_pending = Some((credentials, asked));
        WifiResult::Current {
            view,
            pending: true,
        }
    }

    pub fn wifi_saving(&self) -> bool {
        self.wifi_pending.is_some()
    }

    /// The device reports the source of its last join, which follows a save
    /// only at the next join; there is no join here, so the store stands in
    /// for it.
    pub fn wifi_source(&self) -> WifiSource {
        if self.net.credentials.is_some() {
            WifiSource::Stored
        } else {
            WifiSource::Build
        }
    }

    fn wifi_view(&self) -> WifiView {
        let stored = self.net.credentials.as_ref();
        WifiView {
            stored_ssid_crc: stored.map(|c| crc32(c.ssid())),
            generation: u32::try_from(self.net.generation).unwrap_or(u32::MAX),
            writable: self.net.writable(),
            source: self.wifi_source(),
        }
    }

    pub fn time_settings(&self) -> TimeSettingsResult {
        TimeSettingsResult::Current {
            view: self.time_view(),
            pending: self.time_pending.is_some(),
        }
    }

    pub fn time_settings_change(&mut self, settings: TimeSettings) -> TimeSettingsResult {
        let view = self.time_view();
        if !view.writable {
            return TimeSettingsResult::ReadOnly;
        }
        if self.time_pending.is_some() {
            return TimeSettingsResult::Busy;
        }
        self.time_pending = Some(settings);
        TimeSettingsResult::Current {
            view,
            pending: true,
        }
    }

    fn time_view(&self) -> TimeSettingsView {
        TimeSettingsView {
            settings: self.time.settings,
            generation: u32::try_from(self.time.generation).unwrap_or(u32::MAX),
            writable: self.time.writable(),
        }
    }

    pub fn keys_request(&mut self, change: ClientChange) -> Requested {
        if self.keys_pending.is_some() {
            return Requested::Busy;
        }
        self.ticket = self.ticket.wrapping_add(1).max(1);
        self.keys_pending = Some((self.ticket, change));
        Requested::Queued
    }

    pub fn keys_ticket(&self) -> u32 {
        self.ticket
    }

    pub fn keys_outcome(&self, ticket: u32) -> Option<KeyResult> {
        self.keys_done
            .iter()
            .find(|&&(done, _)| done == ticket)
            .map(|&(_, result)| result)
    }

    /// The client table, when it changed since generation `seen`.
    pub fn clients_since(&self, seen: u32, table: &mut [Option<Client>; CLIENTS]) -> Option<u32> {
        if self.clients_generation == seen {
            return None;
        }
        *table = [None; CLIENTS];
        for (slot, client) in table.iter_mut().zip(self.clients.clients()) {
            *slot = Some(*client);
        }
        Some(self.clients_generation)
    }

    /// Append an event to the ring at once.
    pub fn queue_event(&mut self, at: Option<u32>, body: &[u8; BODY_BYTES]) -> bool {
        self.events.append(at, body).is_ok()
    }

    /// Append an event the writer records itself, for a change it just
    /// made, and keep it for the loop's alert rules
    /// ([`Self::take_written`]).
    pub fn append_written(&mut self, at: Option<u32>, body: &EventBody) {
        let mut bytes = [0; BODY_BYTES];
        if wellfed_proto::encode(body, &mut bytes).is_ok() && self.events.append(at, &bytes).is_ok()
        {
            self.written.push_back((at, bytes));
        }
    }

    /// The oldest event the writer appended itself that the loop has not
    /// yet taken.
    pub fn take_written(&mut self) -> Option<(Option<u32>, [u8; BODY_BYTES])> {
        self.written.pop_front()
    }

    /// A subscriber's read, through the firmware's turns.
    pub fn event_after(&mut self, reader: Reader, after: Option<u64>) -> Option<Read> {
        self.reads.result(reader, after, self.events.last_seq())
    }

    pub fn reader_left(&mut self, reader: Reader) {
        self.reads.left(reader);
    }

    pub fn epoch(&self) -> u32 {
        self.events.epoch()
    }

    pub fn last_event(&self) -> Option<u64> {
        self.events.last_seq()
    }

    pub fn events_fill(&self) -> EventsFill {
        EventsFill {
            kept: self.events.kept(),
            capacity: self.events.capacity(),
        }
    }

    /// The ring's record after `after`, read directly, for a test to
    /// inspect without taking a subscriber's turn.
    pub fn ring_after(&self, after: Option<u64>) -> Read {
        self.events.read_after(after)
    }

    /// The writer's half of an event read: answer the one the device API
    /// asked for, as the settings writer does between the loop's passes.
    pub fn answer_read(&mut self) {
        let Some(after) = self.reads.request() else {
            return;
        };
        let batch = self.events.read_batch(after, &mut self.read_batch[..]);
        self.reads
            .finish(after, batch, &self.read_batch[..], self.events.last_seq());
    }

    /// The writer's turn for the settings, Wi-Fi and time settings queued:
    /// each applied. A failed save leaves the generation where it was; the
    /// client sees no change, as on the device. A client's save records its
    /// event in the same turn, after it, as the firmware's writer does.
    pub fn apply_settings(&mut self) {
        if let Some((fields, asked)) = self.settings_pending.take() {
            let current = self.settings.settings;
            let target = current.changed(fields);
            if target != current
                && self.settings.save(&target)
                && let Some(asked) = asked
            {
                let generation = u32::try_from(self.settings.generation).unwrap_or(u32::MAX);
                let body = EventBody::Settings {
                    generation,
                    by: asked.by,
                };
                self.append_written(asked.at, &body);
            }
        }
        if let Some((credentials, asked)) = self.wifi_pending.take()
            && self.net.save(credentials.as_ref())
            && let Some(asked) = asked
        {
            let generation = u32::try_from(self.net.generation).unwrap_or(u32::MAX);
            let body = EventBody::Wifi {
                generation,
                by: asked.by,
            };
            self.append_written(asked.at, &body);
        }
        if let Some(settings) = self.time_pending.take() {
            self.time.save(&settings);
        }
    }

    /// The writer's turn for a queued client change: its result, for the
    /// log. A change bumps the table's generation, which the next pass
    /// copies to the server.
    pub fn apply_keys(&mut self) -> Option<KeyResult> {
        let (ticket, change) = self.keys_pending.take()?;
        let result = match change {
            ClientChange::Key {
                change:
                    KeyChange::Enroll {
                        key, label, role, ..
                    },
                ..
            } => applied(self.clients.enroll(key, label, role)),
            ClientChange::Key {
                change: KeyChange::Revoke { key, .. },
                ..
            } => applied(self.clients.revoke(&key)),
            // The factory reset, as the writer's: it draws its identity from
            // the generator or does not happen, marks the reset, then
            // forgets every client; the loop restarts and the next boot
            // ([`finish_reset`]) erases the cleared stores.
            ClientChange::Reset => match (self.random)() {
                Some(_) if !reset::mark(&Region::new(&self.flash, storage::RESET)) => KeyResult::Io,
                // Marked, so committed: `Done` whatever the client store
                // took, and the next boot erases it.
                Some(secret) => {
                    if applied(self.clients.reset(&secret)) == KeyResult::Done {
                        self.identity_generation += 1;
                    }
                    KeyResult::Done
                }
                None => KeyResult::NoGenerator,
            },
        };
        // The event in the same turn as the store write, before the result
        // is published, as the firmware's writer appends it.
        if let ClientChange::Key { change, at } = change
            && result == KeyResult::Done
        {
            self.append_written(at, &change.event());
        }
        if self.keys_done.len() == KEYS_FINISHED {
            self.keys_done.pop_front();
        }
        self.keys_done.push_back((ticket, result));
        self.clients_changed();
        Some(result)
    }

    /// The client table changed: the loop copies it on its next pass, and
    /// the update stage reads the owners' update keys where the firmware's
    /// settings writer publishes them.
    fn clients_changed(&mut self) {
        self.clients_generation += 1;
        self.publish_signers();
    }

    fn publish_signers(&self) {
        signers::publish(
            self.clients
                .clients()
                .filter(|client| client.role == Role::UpdateSigner)
                .map(|client| client.key),
        );
    }
}

/// A client store write's result, as the writer reports it.
fn applied(result: Result<(), ClientError>) -> KeyResult {
    result.map_or_else(KeyResult::from, |()| KeyResult::Done)
}

/// Finish a factory reset an earlier boot marked, before any store opens,
/// as the writer does: erase every sector the board's reset clears, an
/// update store this build cannot read when the reset was asked for by
/// hand, let the device kind rewrite what it keeps (`keep`), then erase the
/// marker. The simulator's image cannot lose power part-way through a
/// boot's erases; the firmware's writer, which can, is tested for it.
pub fn finish_reset(flash: &Rc<Flash>, keep: impl FnOnce(&Rc<Flash>)) {
    let marker = Region::new(flash, storage::RESET);
    if !reset::pending(&marker) {
        return;
    }
    for sector in (0..storage::FACTORY_CLEARED_SECTORS).filter_map(storage::factory_cleared) {
        erase(
            flash,
            Sectors {
                first: sector,
                count: 1,
            },
        );
    }
    if reset::by_hand(&marker) {
        let mut update = UpdateStore::default();
        update.open(Region::new(flash, storage::UPDATE));
        if update.clear_corrupt() == Ok(true) {
            wellfed_hal::ota::update::store_started_over();
        }
    }
    keep(flash);
    reset::clear(&marker);
}

/// A handover from stock's activation, as the settings writer finishes it: a
/// consumer image's, for the simulator has no build Wi-Fi. Every sector the
/// board's handover clears, then the clients, the update store's floor
/// and, on bank 0, the confirmation. A cut in the middle repeats it, since
/// the mark goes last.
pub fn finish_handover(flash: &Rc<Flash>, bank: u8) {
    let marker = Region::new(flash, storage::RESET);
    if !handover::pending(&marker) {
        return;
    }
    for sector in (0..storage::HANDOVER_CLEARED_SECTORS).filter_map(storage::handover_cleared) {
        erase(
            flash,
            Sectors {
                first: sector,
                count: 1,
            },
        );
    }
    let mut clients = ClientStore::default();
    clients.open(Region::new(flash, storage::CLIENTS));
    if let Some(identity) = clients.identity().copied() {
        let _ = clients.reset(&identity);
    }
    let mut update = UpdateStore::default();
    update.open(Region::new(flash, storage::UPDATE));
    // As the firmware's handover: a record this build cannot read, or a
    // newer major's, is left alone, and the stage reads the store again
    // only when it started over.
    let started = match update.status {
        UpdateStatus::ReadOnly | UpdateStatus::Corrupt | UpdateStatus::Io => false,
        _ => update.start_over().is_ok(),
    };
    if started {
        wellfed_hal::ota::update::store_started_over();
    }
    if bank == 0 {
        Region::new(flash, storage::CONFIRM).erase(0);
    }
    handover::clear(&marker);
}

/// Erase every unit of `store`.
pub fn erase(flash: &Rc<Flash>, store: Sectors) {
    let region = Region::new(flash, store);
    for unit in 0..store.count {
        region.erase(unit);
    }
}
