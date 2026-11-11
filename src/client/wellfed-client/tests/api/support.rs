//! The fake device, the rig that steps the server one pass at a time, and
//! the helpers that pump bytes between it and a session or a connection.

use std::time::{Duration, Instant};

use wellfed_client::api::{Error, Incoming, Session};
use wellfed_client::connection::{Connection, Event, Timing};
use wellfed_core::clock::wall::WallClock;
use wellfed_core::log_tail::{Batch as LogBatch, LogTail};
use wellfed_core::tone::Tune;
use wellfed_core_feeder::desiccant::Count;
use wellfed_device_api::clients::{CLIENTS, Client, Label, Role};
use wellfed_device_api::mailbox::{Asked, FeederSettings, KEYS_FINISHED, KindSettings as _, Saved};
use wellfed_device_api::mailbox::{
    LockResult, Origin, PlayResult, SettingsFields, SettingsResult, SettingsView, TimeFields,
    TimeResult, TimeSettings, TimeSettingsResult, TimeSettingsView, TimeState, WifiResult,
    WifiSource, WifiView,
};
use wellfed_device_api::pairing::{Config, Pairing};
use wellfed_device_api::wifi::Credentials;
use wellfed_device_api::{
    Device, EventRead, FeederDevice, KeyChange, KeyOutcome, Pass, Reader, Reason, Record, Server,
    UpdateAnswer, UpdateKeyChange, UpdateRequest,
};
use wellfed_model as model;
use wellfed_model::{Answer, Hello, Op};
use wellfed_proto::api::{
    self as proto, DeviceStatus, EventBody, FeederStatus, FoodLevel, FrozenRefusal, Refusal, Stats,
    StatsSection, Status, UpdatePhase, UpdateRefusal, UpdateView, WANTED_MAX,
};
use wellfed_session::noise::Keypair;

pub(crate) const FEEDER: [u8; 32] = [0x10; 32];
pub(crate) const OWNER: [u8; 32] = [0x20; 32];
pub(crate) const MEMBER: [u8; 32] = [0x30; 32];
pub(crate) const STRANGER: [u8; 32] = [0x40; 32];
/// The fake ring's epoch.
pub(crate) const EPOCH: u32 = 0x0e90_c401;

pub(crate) fn public(secret: [u8; 32]) -> [u8; 32] {
    *Keypair::from_secret(secret).public()
}

/// The feeder's mailboxes and stores, with a writer the test runs by hand.
#[derive(Default)]
pub(crate) struct Fake {
    pub(crate) counter: u8,
    pub(crate) settings: SettingsView,
    pub(crate) settings_pending: Option<Asked<SettingsFields>>,
    /// Today's local calendar day; `None` while the clock is invalid.
    pub(crate) today: Option<i32>,
    pub(crate) wifi_generation: u32,
    pub(crate) wifi_pending: Option<Asked<Option<Credentials>>>,
    pub(crate) wifi_saved: Saved,
    /// The writer's next Wi-Fi save fails: nothing saved, no event.
    pub(crate) wifi_fails: bool,
    pub(crate) locked: Option<bool>,
    pub(crate) clients: Vec<Client>,
    pub(crate) keys_pending: Option<(u32, KeyChange)>,
    /// The last finished changes, newest last, as many as the firmware's
    /// mailbox remembers.
    pub(crate) keys_done: Vec<(u32, KeyOutcome)>,
    pub(crate) ticket: u32,
    pub(crate) events: Vec<proto::Event>,
    /// Events before this sequence number left the ring.
    pub(crate) oldest: u64,
    /// Events lost inside the ring.
    pub(crate) lost: Option<(u64, u64)>,
    /// Bodies a newer build recorded, by the event's number: served in
    /// place of the event's own.
    pub(crate) newer: Vec<(u64, proto::RawBody)>,
    /// `None` until the writer has opened the ring.
    pub(crate) epoch: Option<u32>,
    pub(crate) rebooted: bool,
    pub(crate) syncs: usize,
    /// Feeds queued, in order: feed `n` is `feeds[n - 1]`.
    pub(crate) feeds: Vec<(u8, proto::Source)>,
    /// Feeds up to this number are reserved (the owner's turn, by hand).
    pub(crate) reserved: u32,
    pub(crate) cancelled: Vec<u32>,
    /// Serves updates.
    pub(crate) update: bool,
    /// An install holds the flash.
    pub(crate) installing: bool,
    /// Serves the meal call's chunks.
    pub(crate) call: bool,
    /// The next chunk's mailbox is full: a chunk is refused `Busy`.
    pub(crate) chunks_busy: bool,
    /// Update requests queued, in order, and the next one's answer.
    pub(crate) updates: Vec<String>,
    pub(crate) update_done: Option<(u32, Result<UpdateAnswer, FrozenRefusal>)>,
    /// Keep the next request queued (its lease held) until `answer_held`.
    pub(crate) hold: bool,
    pub(crate) held: Option<(u32, Result<UpdateAnswer, FrozenRefusal>)>,
    /// Chunk offsets handed over.
    pub(crate) chunks: Vec<u32>,
    pub(crate) time: TimeSettingsView,
    pub(crate) time_pending: Option<TimeSettings>,
    /// The writer's next time save fails, leaving the store as it was.
    pub(crate) time_fails: bool,
    /// The writer's next settings save fails: nothing saved, no event.
    pub(crate) settings_fails: bool,
    /// Keeps a log tail, `tail`.
    pub(crate) log: bool,
    pub(crate) tail: LogTail<4096>,
    /// The feeding owner's own reason not to feed.
    pub(crate) blocked: Option<proto::Blocked>,
    /// The owner reserves no queued feed while set.
    pub(crate) hold_feeds: bool,
    /// The trace the feeder holds: its summary, body and whether it is
    /// stored; and a feed still being recorded.
    pub(crate) trace: Option<(proto::TraceSummary, Vec<u8>, bool)>,
    pub(crate) tracing: Option<u32>,
    /// Bank 1 as the bank-0 role reads it; served when set.
    pub(crate) bank1: Option<Vec<u8>>,
}

/// An install receiving its first sector.
pub(crate) fn receiving() -> UpdateView {
    UpdateView {
        phase: UpdatePhase::Receiving,
        bank: 0,
        min_version: 4,
        digest: Some([9; 32]),
        version: 5,
        image_len: 8192,
        written: 0,
        wanted: [Some(0); WANTED_MAX],
        stopped: None,
        bank0: wellfed_proto::api::Bank0Check::Unchecked,
    }
}

impl Fake {
    pub(crate) fn new() -> Self {
        let mut fake = Self {
            settings: SettingsView {
                lights: true,
                sound: true,
                volume: 40,
                kind: FeederSettings::DEFAULT,
                generation: 7,
                writable: true,
                ..Default::default()
            },
            wifi_generation: 3,
            locked: Some(false),
            oldest: 1,
            epoch: Some(EPOCH),
            ..Self::default()
        };
        fake.enroll(public(OWNER), Role::Owner);
        fake.enroll(public(MEMBER), Role::Member);
        fake
    }

    pub(crate) fn enroll(&mut self, key: [u8; 32], role: Role) {
        self.clients.push(Client {
            key,
            label: Label::new(b"test").unwrap(),
            role,
            enrolled: 1,
        });
    }

    /// The update stage answers the request it held.
    pub(crate) fn answer_held(&mut self) {
        self.update_done = self.held.take();
    }

    /// The settings writer's turn: apply what is queued, and record each
    /// client change's event as it saves it.
    pub(crate) fn write(&mut self) {
        if let Some(Asked { .. }) = self.settings_pending.take_if(|_| self.settings_fails) {
            self.settings_fails = false;
            self.settings.saved.finish(true);
        }
        if let Some(Asked { change: fields, by }) = self.settings_pending.take() {
            self.settings.lights = fields.lights.unwrap_or(self.settings.lights);
            self.settings.sound = fields.sound.unwrap_or(self.settings.sound);
            self.settings.volume = fields.volume.unwrap_or(self.settings.volume);
            self.settings.lights_window =
                fields.lights_window.unwrap_or(self.settings.lights_window);
            self.settings.sound_window = fields.sound_window.unwrap_or(self.settings.sound_window);
            self.settings.kind = self.settings.kind.changed(&fields.kind);
            self.settings.generation += 1;
            self.settings.saved.finish(false);
            let generation = self.settings.generation;
            self.record(0, EventBody::Settings { generation, by });
        }
        if let Some(Asked { .. }) = self.wifi_pending.take_if(|_| self.wifi_fails) {
            self.wifi_fails = false;
            self.wifi_saved.finish(true);
        }
        if let Some(Asked { by, .. }) = self.wifi_pending.take() {
            self.wifi_saved.finish(false);
            self.wifi_generation += 1;
            let generation = self.wifi_generation;
            self.record(0, EventBody::Wifi { generation, by });
        }
        if let Some(settings) = self.time_pending.take()
            && !std::mem::take(&mut self.time_fails)
            && settings != self.time.settings
        {
            self.time.settings = settings;
            self.time.generation += 1;
        }
        if let Some((ticket, change)) = self.keys_pending.take() {
            let outcome = match change {
                KeyChange::Enroll { key, role, .. } => {
                    if self.clients.iter().any(|c| c.key == key) {
                        KeyOutcome::Duplicate
                    } else {
                        self.enroll(key, role);
                        KeyOutcome::Done
                    }
                }
                KeyChange::Revoke { key, .. } => {
                    let before = self.clients.len();
                    self.clients.retain(|c| c.key != key);
                    if self.clients.len() < before {
                        KeyOutcome::Done
                    } else {
                        KeyOutcome::NotFound
                    }
                }
            };
            // The writer's event, in the turn that made the change.
            if outcome == KeyOutcome::Done {
                self.record(0, change.event());
            }
            if self.keys_done.len() == KEYS_FINISHED {
                self.keys_done.remove(0);
            }
            self.keys_done.push((ticket, outcome));
        }
        if !self.hold_feeds {
            self.reserved = u32::try_from(self.feeds.len()).unwrap();
        }
    }

    pub(crate) fn table(&self) -> [Option<Client>; CLIENTS] {
        let mut table = [None; CLIENTS];
        for (slot, client) in table.iter_mut().zip(&self.clients) {
            *slot = Some(*client);
        }
        table
    }
}

impl Device for Fake {
    type Feeder = Self;

    fn feeder(&mut self) -> Option<&mut Self> {
        Some(self)
    }

    fn random(&mut self, out: &mut [u8; 32]) -> bool {
        self.counter = self.counter.wrapping_add(1);
        *out = [self.counter; 32];
        true
    }

    fn build(&self) -> &'static str {
        "test-build"
    }

    fn model(&self) -> wellfed_proto::device::Model {
        wellfed_proto::device::Model::Plaf103
    }

    fn status(&mut self, _now_ms: u32) -> Status<'static> {
        Status {
            build: "test-build",
            bank: 1,
            uptime_ms: 1,
            boot_count: 1,
            reset_flags: 0,
            faults: 0,
            rssi: -50,
            wifi: proto::WifiSource::Stored.into(),
            link_losses: 0,
            dns_losses: 0,
            clock_ms: None,
            power: None,
            wifi_generation: self.wifi_generation,
            last_event: self.events.last().map(|e| e.seq),
            sessions: 0,
            alerts_generation: 0,
            device: DeviceStatus::Feeder(FeederStatus {
                locked: self.locked,
                settings_generation: self.settings.generation,
                desiccant: match self.settings.kind.desiccant(self.today) {
                    Count::Unset => proto::Desiccant::Unset,
                    Count::Unknown => proto::Desiccant::Unknown,
                    Count::Remaining(days) => proto::Desiccant::Remaining { days },
                },
                food: FoodLevel::Unread.into(),
                feed_fault: None,
                blocked: self.readiness().blocked(),
                supply: proto::Supply {
                    adapter: None,
                    battery: proto::Battery::Unread,
                    below_floor: None,
                },
                ext: wellfed_proto::api::Tail::EMPTY,
            }),
            ext: wellfed_proto::api::Tail::EMPTY,
        }
    }

    fn wifi(&mut self, change: Option<Asked<Option<Credentials>>>) -> WifiResult {
        if let Some(change) = change {
            if self.wifi_pending.is_some() {
                return WifiResult::Busy;
            }
            self.wifi_pending = Some(change);
        }
        WifiResult::Current {
            view: WifiView {
                stored_ssid_crc: None,
                generation: self.wifi_generation,
                writable: true,
                source: WifiSource::Stored,
                saved: self.wifi_saved,
            },
            pending: self.wifi_pending.is_some(),
        }
    }

    fn time_sync(&mut self, _fields: TimeFields, _origin: Origin) -> TimeResult {
        self.syncs += 1;
        TimeResult::Synced { jump_ms: Some(5) }
    }

    fn keys(&mut self, change: KeyChange) -> Option<u32> {
        if self.keys_pending.is_some() {
            return None;
        }
        self.ticket += 1;
        self.keys_pending = Some((self.ticket, change));
        Some(self.ticket)
    }

    fn keys_outcome(&mut self, ticket: u32) -> Option<KeyOutcome> {
        self.keys_done
            .iter()
            .find(|&&(done, _)| done == ticket)
            .map(|&(_, outcome)| outcome)
    }

    /// A small fixed answer per section, each told apart by its values.
    fn stats(&mut self, section: StatsSection, now_ms: u32) -> Stats<'_> {
        match section {
            StatsSection::Counters => Stats::Counters(proto::Counters {
                boot: proto::Counts {
                    uptime_s: now_ms / 1000,
                    sessions: 1,
                    ..proto::Counts::default()
                },
                ..proto::Counters::default()
            }),
            StatsSection::Memory => {
                let mut tasks = [None; proto::TASKS_MAX];
                tasks[0] = Some(proto::TaskStack {
                    name: "wellfed-loop",
                    free_min: 2_900,
                });
                Stats::Memory(proto::Memory {
                    heap_free: 125_088,
                    heap_min: 119_000,
                    tasks,
                    tasks_total: 1,
                })
            }
            StatsSection::Flash => Stats::Flash(proto::Flash {
                events_kept: self.events.len().try_into().unwrap(),
                events_capacity: 1_408,
                ..proto::Flash::default()
            }),
            StatsSection::Power => Stats::Power(proto::PowerStats {
                adapter: Some(true),
                period_s: 3_600,
                ..proto::PowerStats::default()
            }),
            StatsSection::Food => Stats::Food(proto::Food::default()),
            StatsSection::Feeds => Stats::Feeds(proto::FeedStats {
                traced: 1,
                kept: 1,
                capacity: 8,
                last: self.trace.as_ref().map(|(summary, ..)| *summary),
                ..proto::FeedStats::default()
            }),
            StatsSection::Portions => Stats::Portions(proto::Portions {
                portions: 3,
                portion_mg: Some(9_800),
                delivered_mg: Some(29_400),
                ..proto::Portions::default()
            }),
        }
    }

    fn bank1_page(&mut self, offset: u32) -> Result<proto::Bank1Page<'_>, Refusal> {
        if self.installing {
            return Err(Refusal::Busy);
        }
        let bank = self.bank1.as_deref().ok_or(Refusal::Unavailable)?;
        let rest = bank
            .get(offset as usize..)
            .filter(|rest| !rest.is_empty())
            .ok_or(Refusal::Invalid)?;
        let len = rest.len().min(proto::BANK1_PAGE_MAX);
        Ok(proto::Bank1Page {
            offset,
            total: u32::try_from(bank.len()).unwrap(),
            bytes: &rest[..len],
        })
    }

    fn log_after(&mut self, after: Option<u32>, text: &mut [u8]) -> Option<LogBatch> {
        if !self.log {
            return None;
        }
        self.tail.read(after, text)
    }

    fn time_settings(&mut self, change: Option<TimeSettings>) -> TimeSettingsResult {
        if let Some(settings) = change {
            if self.time_pending.is_some() {
                return TimeSettingsResult::Busy;
            }
            self.time_pending = Some(settings);
        }
        TimeSettingsResult::Current {
            view: TimeSettingsView {
                writable: true,
                ..self.time
            },
            pending: self.time_pending.is_some(),
        }
    }

    fn time_state(&mut self, now_ms: u32) -> Option<TimeState> {
        Some(TimeState {
            clock: WallClock::new().state(now_ms),
            now_ms,
            ntp_last: None,
        })
    }

    fn update(&mut self, request: UpdateRequest<'_>) -> Result<u32, FrozenRefusal> {
        if !self.update {
            return Err(FrozenRefusal::Unavailable);
        }
        self.ticket += 1;
        let answer = match request {
            UpdateRequest::Offer { device, .. } => {
                assert_eq!(device, public(FEEDER), "an offer names this feeder");
                self.installing = true;
                Ok(UpdateAnswer::View(receiving()))
            }
            UpdateRequest::Statement { statement } => Ok(UpdateAnswer::Key {
                change: UpdateKeyChange::Retired,
                id: statement[..8].try_into().unwrap(),
            }),
            UpdateRequest::Cancel => {
                self.installing = false;
                Ok(UpdateAnswer::View(UpdateView {
                    phase: UpdatePhase::Idle,
                    ..receiving()
                }))
            }
        };
        self.updates.push(format!("{request:?}"));
        if self.hold {
            self.held = Some((self.ticket, answer));
        } else {
            self.update_done = Some((self.ticket, answer));
        }
        Ok(self.ticket)
    }

    fn update_outcome(&mut self, ticket: u32) -> Option<Result<UpdateAnswer, FrozenRefusal>> {
        self.update_done
            .filter(|&(done, _)| done == ticket)
            .map(|(_, answer)| answer)
    }

    fn update_chunk(&mut self, offset: u32, _data: &[u8]) -> Result<UpdateView, FrozenRefusal> {
        if !self.update {
            return Err(FrozenRefusal::Unavailable);
        }
        if self.chunks_busy {
            return Err(FrozenRefusal::Busy);
        }
        if !self.installing {
            return Err(FrozenRefusal::Update(UpdateRefusal::NotInstalling));
        }
        self.chunks.push(offset);
        Ok(receiving())
    }

    fn update_view(&mut self) -> Option<UpdateView> {
        self.update.then(receiving)
    }

    fn installing(&self) -> bool {
        // A queued request holds the lease too.
        self.installing || self.held.is_some()
    }

    fn operating(&self) -> bool {
        self.reserved < u32::try_from(self.feeds.len()).unwrap()
    }

    fn record(&mut self, _now_ms: u32, body: EventBody) {
        let seq = self.events.last().map_or(1, |e| e.seq + 1);
        self.events.push(proto::Event {
            seq,
            at: None,
            body,
        });
    }

    fn event_after(&mut self, _reader: Reader, after: Option<u64>) -> EventRead {
        let after = after.unwrap_or(0);
        if after + 1 < self.oldest {
            return EventRead::Gap {
                from: after + 1,
                to: self.oldest - 1,
            };
        }
        if let Some((from, to)) = self.lost
            && after + 1 == from
        {
            return EventRead::Gap { from, to };
        }
        self.events
            .iter()
            .find(|e| e.seq > after && e.seq >= self.oldest)
            .map_or(EventRead::None, |e| {
                let mut raw = proto::RawEvent::encode(e).expect("a body fits the ring");
                if let Some((_, body)) = self.newer.iter().find(|(seq, _)| *seq == e.seq) {
                    raw.body = *body;
                }
                EventRead::Event(raw)
            })
    }

    fn epoch(&self) -> Option<u32> {
        self.epoch
    }

    fn reboot(&mut self) {
        self.rebooted = true;
    }
}

impl FeederDevice for Fake {
    fn today(&mut self, _now_ms: u32) -> Option<i32> {
        self.today
    }

    fn settings(&mut self, change: Option<Asked<SettingsFields>>) -> SettingsResult {
        let Some(change) = change.filter(|asked| asked.change.changes()) else {
            return SettingsResult::Current {
                view: self.settings,
                pending: self.settings_pending.is_some(),
            };
        };
        if self.settings_pending.is_some() {
            return SettingsResult::Busy;
        }
        self.settings_pending = Some(change);
        SettingsResult::Current {
            view: self.settings,
            pending: true,
        }
    }

    fn lock(&mut self, locked: bool) -> LockResult {
        self.locked = Some(locked);
        LockResult::Current {
            locked,
            pending: true,
        }
    }

    fn play(&mut self, _tune: Tune) -> PlayResult {
        PlayResult::Started
    }

    fn feed_trace(&mut self, feed: u32, offset: u16) -> Result<proto::TracePage<'_>, Refusal> {
        if self.tracing == Some(feed) {
            return Err(Refusal::Busy);
        }
        let Some((summary, body, stored)) = self.trace.as_ref().filter(|(s, ..)| s.feed == feed)
        else {
            return Err(Refusal::NotFound);
        };
        let rest = body.get(usize::from(offset)..).ok_or(Refusal::Invalid)?;
        let len = rest.len().min(wellfed_proto::trace::PAGE_MAX);
        Ok(proto::TracePage {
            summary: *summary,
            stored: *stored,
            offset,
            total: u16::try_from(body.len()).unwrap(),
            bytes: &rest[..len],
        })
    }

    fn readiness(&mut self) -> proto::Readiness {
        let blocked = if self.installing() {
            Some(proto::Blocked::Updating)
        } else if self.operating() {
            Some(proto::Blocked::Feeding)
        } else {
            self.blocked
        };
        blocked.map_or(proto::Readiness::Ready, proto::Readiness::Blocked)
    }

    fn call_chunk(&mut self, _offset: u32, _data: &[u8]) -> Result<(), Refusal> {
        if !self.call {
            Err(Refusal::Unavailable)
        } else if self.chunks_busy {
            Err(Refusal::Busy)
        } else {
            Err(Refusal::Invalid)
        }
    }

    fn feed(&mut self, portions: u8, by: proto::Source) -> Result<u32, Refusal> {
        if self.reserved < u32::try_from(self.feeds.len()).unwrap() {
            return Err(Refusal::Busy);
        }
        self.feeds.push((portions, by));
        Ok(u32::try_from(self.feeds.len()).unwrap())
    }

    fn feed_reserved(&mut self, feed: u32) -> Option<Result<(), Refusal>> {
        (feed <= self.reserved).then_some(Ok(()))
    }

    fn cancel_feed(&mut self, feed: u32) -> bool {
        let running = feed != 0 && feed <= self.reserved && !self.cancelled.contains(&feed);
        if running {
            self.cancelled.push(feed);
        }
        running
    }
}

/// The server, its device and pairing, stepped one pass at a time.
pub(crate) struct Rig {
    pub(crate) server: Box<Server>,
    pub(crate) device: Fake,
    pub(crate) pairing: Pairing,
    pub(crate) identity: Keypair,
    pub(crate) now: u32,
    pub(crate) feeding: bool,
    /// Pairing's decision on a recovery offer, for the next pass.
    pub(crate) recovery: Option<wellfed_device_api::pairing::Event>,
    /// Pairing's decision on an update-key offer, for the next pass.
    pub(crate) signer: Option<wellfed_device_api::pairing::Event>,
    pub(crate) records: Vec<Record>,
    pub(crate) clients_generation: u32,
}

impl Rig {
    pub(crate) fn new() -> Self {
        let mut rig = Self {
            server: Box::new(Server::new()),
            device: Fake::new(),
            pairing: Pairing::new(Config::DEFAULT),
            identity: Keypair::from_secret(FEEDER),
            now: 1_000,
            feeding: false,
            recovery: None,
            signer: None,
            records: Vec::new(),
            clients_generation: 0,
        };
        rig.publish();
        rig
    }

    /// The writer publishes the client table, as `keys::clients_since`.
    pub(crate) fn publish(&mut self) {
        self.clients_generation += 1;
        let table = self.device.table();
        *self.server.clients_table().0 = table;
        let records = &mut self.records;
        self.server
            .clients_changed(self.clients_generation, |r| records.push(r));
    }

    /// One pass; whether the server has work waiting.
    pub(crate) fn pass(&mut self) -> bool {
        self.now += 20;
        let records = &mut self.records;
        self.server.step(
            Pass {
                now_ms: self.now,
                device: &mut self.device,
                pairing: &mut self.pairing,
                identity: Some(&self.identity),
                feeding: self.feeding,
                recovery: self.recovery.take(),
                signer: self.signer.take(),
            },
            |r| records.push(r),
        )
    }

    pub(crate) fn accept(&mut self) -> Option<usize> {
        let records = &mut self.records;
        self.server.accept(self.now, |r| records.push(r))
    }

    /// Client bytes into `slot`, as far as its buffer has room.
    pub(crate) fn deliver(&mut self, slot: usize, bytes: &mut Vec<u8>) {
        let room = self.server.rx(slot);
        let n = room.len().min(bytes.len());
        room[..n].copy_from_slice(&bytes[..n]);
        self.server.received(slot, n);
        bytes.drain(..n);
    }

    /// Everything `slot` has to send.
    pub(crate) fn collect(&mut self, slot: usize) -> Vec<u8> {
        let out = self.server.tx(slot).to_vec();
        self.server.sent(slot, out.len());
        out
    }

    pub(crate) fn ended(&self, reason: Reason) -> bool {
        self.records
            .iter()
            .any(|r| matches!(r, Record::Ended { reason: got, .. } if *got == reason))
    }
}

/// One client on one slot.
pub(crate) struct Client_ {
    pub(crate) slot: usize,
    pub(crate) session: Session,
    pub(crate) outbox: Vec<u8>,
}

/// Run passes, moving bytes both ways, until `done` or a bound.
pub(crate) fn run(rig: &mut Rig, client: &mut Client_, mut done: impl FnMut(&mut Session) -> bool) {
    for _ in 0..200 {
        rig.deliver(client.slot, &mut client.outbox);
        rig.pass();
        let bytes = rig.collect(client.slot);
        if !bytes.is_empty() {
            client.session.receive(&bytes).unwrap();
        }
        rig.device.write();
        if done(&mut client.session) {
            return;
        }
    }
    panic!("no progress");
}

pub(crate) fn connect(rig: &mut Rig, secret: [u8; 32]) -> (Client_, Option<Incoming>) {
    let slot = rig.accept().unwrap();
    let (session, outbox) =
        Session::connect(&Keypair::from_secret(secret), public(FEEDER), [0x77; 32]).unwrap();
    let mut client = Client_ {
        slot,
        session,
        outbox,
    };
    let mut welcome = None;
    run(rig, &mut client, |session| {
        if let Some(incoming @ (Incoming::Opened(_) | Incoming::Refused(_))) =
            session.next_incoming().unwrap()
        {
            welcome = Some(incoming);
            return true;
        }
        false
    });
    (client, welcome)
}

pub(crate) fn open(rig: &mut Rig, secret: [u8; 32]) -> Client_ {
    let (client, welcome) = connect(rig, secret);
    assert!(matches!(welcome, Some(Incoming::Opened(_))), "{welcome:?}");
    client
}

/// Send one request and run until its answer.
pub(crate) fn ask(rig: &mut Rig, client: &mut Client_, id: u8, op: Op) -> Answer {
    let bytes = client.session.request([id; 16], &op).unwrap();
    client.outbox.extend(bytes);
    let mut answer = None;
    run(rig, client, |session| {
        while let Some(incoming) = session.next_incoming().unwrap() {
            if let Incoming::Answer {
                id: got,
                answer: got_answer,
            } = incoming
                && got == [id; 16]
            {
                answer = Some(got_answer);
                return true;
            }
        }
        false
    });
    answer.unwrap()
}

/// Send a request and run one pass without the writer's turn, so a write it
/// queues stays queued.
pub(crate) fn send_unwritten(rig: &mut Rig, client: &mut Client_, id: u8, op: Op) {
    let bytes = client.session.request([id; 16], &op).unwrap();
    client.outbox.extend(bytes);
    rig.deliver(client.slot, &mut client.outbox);
    rig.pass();
}

pub(crate) fn fingerprint(secret: [u8; 32]) -> proto::Fingerprint {
    let key = public(secret);
    [key[0], key[1], key[2], key[3]]
}

/// One sans-IO connection on one slot, with the client's clock tied to the
/// rig's passes.
pub(crate) struct Live {
    pub(crate) slot: usize,
    pub(crate) connection: Connection,
    pub(crate) base: Instant,
    pub(crate) outbox: Vec<u8>,
    pub(crate) events: Vec<Event>,
}

impl Live {
    pub(crate) fn now(&self, rig: &Rig) -> Instant {
        self.base + Duration::from_millis(u64::from(rig.now))
    }
}

pub(crate) fn live_with(rig: &mut Rig, secret: [u8; 32], hello: Hello) -> Live {
    let slot = rig.accept().unwrap();
    let base = Instant::now();
    let now = base + Duration::from_millis(u64::from(rig.now));
    let keys = Keypair::from_secret(secret);
    let connection = Connection::with(
        &keys,
        public(FEEDER),
        [0x77; 32],
        hello,
        Timing::DEFAULT,
        now,
    )
    .unwrap();
    Live {
        slot,
        connection,
        base,
        outbox: Vec::new(),
        events: Vec::new(),
    }
}

/// Passes, moving bytes both ways and collecting events, until `done` or
/// the connection fails.
pub(crate) fn drive(
    rig: &mut Rig,
    live: &mut Live,
    mut done: impl FnMut(&Live) -> bool,
) -> Result<(), Error> {
    for _ in 0..200 {
        while let Some(bytes) = live.connection.poll_transmit() {
            live.outbox.extend(bytes);
        }
        rig.deliver(live.slot, &mut live.outbox);
        rig.pass();
        let bytes = rig.collect(live.slot);
        rig.device.write();
        if !bytes.is_empty() {
            let now = live.now(rig);
            live.connection.handle_input(now, &bytes)?;
        }
        while let Some(event) = live.connection.poll_event() {
            live.events.push(event);
        }
        if done(live) {
            return Ok(());
        }
    }
    panic!("no progress");
}

pub(crate) fn live(rig: &mut Rig, secret: [u8; 32]) -> Live {
    let hello = Hello { major: 1 };
    let mut live = live_with(rig, secret, hello);
    drive(rig, &mut live, |live| live.connection.opened().is_some()).unwrap();
    live
}

pub(crate) fn pushed(live: &Live) -> Vec<model::Event> {
    live.events
        .iter()
        .filter_map(|event| match event {
            Event::Pushed(model::Pushed::Event(event)) => Some(*event),
            _ => None,
        })
        .collect()
}

/// The feeder's view as a client reads it.
pub(crate) fn view(view: UpdateView) -> Answer {
    Answer::Update(view.into())
}

pub(crate) fn poll_answer(
    rig: &mut Rig,
    client: &mut Client_,
    id: u8,
    passes: usize,
) -> Option<Answer> {
    for _ in 0..passes {
        rig.deliver(client.slot, &mut client.outbox);
        rig.pass();
        let bytes = rig.collect(client.slot);
        if !bytes.is_empty() {
            client.session.receive(&bytes).unwrap();
        }
        rig.device.write();
        while let Some(incoming) = client.session.next_incoming().unwrap() {
            if let Incoming::Answer { id: got, answer } = incoming
                && got == [id; 16]
            {
                return Some(answer);
            }
        }
    }
    None
}
