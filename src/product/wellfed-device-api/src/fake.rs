//! A fake device for host tests of the [`Server`](crate::Server): the
//! feeder's mailboxes and stores, with a writer the test runs by hand
//! ([`Fake::write`]).
//!
//! This crate's own tests, the fuzz targets (which include those tests'
//! support module by path) and the client's tests all drive the server
//! against this one fake, so a change to the [`Device`] contract is met in
//! one place. A rig that wants every write to land runs the writer after
//! each pass; one that holds the writer off sees each write stay queued.
//!
//! [`Fake::widest`] serves every value the device reports at its widest, so
//! a frame that holds those holds any: the property tests and the fuzz
//! targets check that every frame fits its limit against it.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use wellfed_core::clock::wall::WallClock;
use wellfed_core::log_tail::{Batch as LogBatch, LogTail};
use wellfed_core::tone::Tune;
use wellfed_core_feeder::desiccant::Count;
use wellfed_proto::api::{
    self as proto, DeviceStatus, EventBody, FeederStatus, FoodLevel, FrozenRefusal, Refusal, Stats,
    StatsSection, Status, UpdatePhase, UpdateRefusal, UpdateView, WANTED_MAX,
};
use wellfed_proto::device::Model;
use wellfed_session::noise::Keypair;

use crate::clients::{CLIENTS, Client, Label, Role};
use crate::mailbox::{
    Asked, Calibration, CalibrationResult, CalibrationView, FeederSettings, KEYS_FINISHED,
    KindSettings as _, LockResult, MealSkip, Origin, PlansResult, PlansView, PlayResult, Saved,
    SettingsFields, SettingsResult, SettingsView, Table, TimeFields, TimeResult, TimeSettings,
    TimeSettingsResult, TimeSettingsView, TimeState, WifiResult, WifiSource, WifiView,
};
use crate::wifi::Credentials;
use crate::{
    Device, EventRead, FeederDevice, KeyChange, KeyOutcome, NoFeeder, Reader, UpdateAnswer,
    UpdateKeyChange, UpdateRequest,
};

/// The feeder's static secret every fake rig uses.
pub const FEEDER: [u8; 32] = [0x10; 32];
/// The enrolled owner's static secret.
pub const OWNER: [u8; 32] = [0x20; 32];
/// An enrolled member's static secret.
pub const MEMBER: [u8; 32] = [0x30; 32];
/// The epoch of [`Fake::new`]'s ring.
pub const EPOCH: u32 = 0x0e90_c401;

/// The public key of the static secret `secret`.
#[must_use]
pub fn public(secret: [u8; 32]) -> [u8; 32] {
    *Keypair::from_secret(secret).public()
}

/// The feeder's mailboxes and stores. Every write queues, and lands at the
/// writer's turn ([`Fake::write`]); a skip lands at once.
#[derive(Default)]
pub struct Fake {
    pub counter: u8,
    /// Serve every value at its widest ([`Fake::widest`]).
    pub widest: bool,
    pub settings: SettingsView,
    pub settings_pending: Option<Asked<SettingsFields>>,
    /// Today's local calendar day; `None` while the clock is invalid.
    pub today: Option<i32>,
    pub wifi_generation: u32,
    pub wifi_pending: Option<Asked<Option<Credentials>>>,
    pub wifi_saved: Saved,
    /// The writer's next Wi-Fi save fails: nothing saved, no event.
    pub wifi_fails: bool,
    pub locked: Option<bool>,
    pub clients: Vec<Client>,
    pub keys_pending: Option<(u32, KeyChange)>,
    /// The last finished changes, newest last, as many as the firmware's
    /// mailbox remembers.
    pub keys_done: Vec<(u32, KeyOutcome)>,
    pub ticket: u32,
    pub events: Vec<proto::Event>,
    /// Events before this sequence number left the ring.
    pub oldest: u64,
    /// Events lost inside the ring.
    pub lost: Option<(u64, u64)>,
    /// Bodies a newer build recorded, by the event's number: served in
    /// place of the event's own.
    pub newer: Vec<(u64, proto::RawBody)>,
    /// `None` until the writer has opened the ring.
    pub epoch: Option<u32>,
    pub rebooted: bool,
    /// Time syncs handed to the clock.
    pub syncs: usize,
    pub clock: WallClock,
    /// Feeds queued, in order: feed `n` is `feeds[n - 1]`.
    pub feeds: Vec<(u8, proto::Source)>,
    /// Feeds up to this number are reserved (the owner's turn, by hand).
    pub reserved: u32,
    pub cancelled: Vec<u32>,
    /// Serves updates.
    pub update: bool,
    /// An install holds the flash.
    pub installing: bool,
    /// Serves the meal call's chunks.
    pub call: bool,
    /// The next chunk's mailbox is full: a chunk is refused `Busy`.
    pub chunks_busy: bool,
    /// Update requests queued, in order, and the next one's answer.
    pub updates: Vec<String>,
    pub update_done: Option<(u32, Result<UpdateAnswer, FrozenRefusal>)>,
    /// Keep the next request queued (its lease held) until `answer_held`.
    pub hold: bool,
    pub held: Option<(u32, Result<UpdateAnswer, FrozenRefusal>)>,
    /// Chunk offsets handed over.
    pub chunks: Vec<u32>,
    pub time: TimeSettingsView,
    pub time_pending: Option<TimeSettings>,
    /// The writer's next time save fails, leaving the store as it was.
    pub time_fails: bool,
    /// The writer's next settings save fails: nothing saved, no event.
    pub settings_fails: bool,
    /// Keeps a log tail, `tail`.
    pub log: bool,
    pub tail: LogTail<4096>,
    /// The feeding owner's own reason not to feed.
    pub blocked: Option<proto::Blocked>,
    /// The owner reserves no queued feed while set.
    pub hold_feeds: bool,
    /// The trace the feeder holds: its summary, body and whether it is
    /// stored; and a feed still being recorded.
    pub trace: Option<(proto::TraceSummary, Vec<u8>, bool)>,
    pub tracing: Option<u32>,
    /// Bank 1 as the bank-0 role reads it; served when set.
    pub bank1: Option<Vec<u8>>,
    pub calibration: CalibrationView,
    pub plans: PlansView,
    /// Skips (and undos) the device was asked for.
    pub skips: u32,
    /// Why the device refuses a skip, if it does.
    pub skip_refusal: Option<Refusal>,
    /// The writer drops a queued skip: the store is left as it was.
    pub skip_dropped: bool,
}

/// An install receiving its first sector.
#[must_use]
pub fn receiving() -> UpdateView {
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
        bank0: proto::Bank0Check::Unchecked,
    }
}

impl Fake {
    /// A feeder with settings at generation 7, its owner (`OWNER`) and a
    /// member (`MEMBER`) enrolled, and its ring open at [`EPOCH`].
    #[must_use]
    pub fn new() -> Self {
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

    /// A feeder whose stores are empty (every generation 0), whose clock
    /// knows the day, which keeps a log tail and which serves every status,
    /// stats section and trace page at its widest. It refuses feeds as
    /// unserved until a test clears `blocked`.
    #[must_use]
    pub fn widest() -> Self {
        Self {
            widest: true,
            locked: Some(false),
            today: Some(20_389),
            epoch: Some(1),
            log: true,
            blocked: Some(proto::Blocked::Unserved),
            ..Self::default()
        }
    }

    pub fn enroll(&mut self, key: [u8; 32], role: Role) {
        let Some(label) = Label::new(b"test") else {
            unreachable!("a fixed label within the limit");
        };
        self.clients.push(Client {
            key,
            label,
            role,
            enrolled: 1,
        });
    }

    /// The update stage answers the request it held.
    pub fn answer_held(&mut self) {
        self.update_done = self.held.take();
    }

    /// The writer's turn: apply what is queued, and record each client
    /// change's event as it saves it.
    pub fn write(&mut self) {
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
            && !core::mem::take(&mut self.time_fails)
            && settings != self.time.settings
        {
            self.time.settings = settings;
            self.time.generation += 1;
        }

        if let Some((ticket, change)) = self.keys_pending.take() {
            self.change_keys(ticket, change);
        }

        if !self.hold_feeds {
            self.reserved = self.queued_feeds();
        }
    }

    fn change_keys(&mut self, ticket: u32, change: KeyChange) {
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
            if let Some(invited) = change.invited() {
                self.record(0, invited);
            }
            self.record(0, change.event());
        }
        if self.keys_done.len() == KEYS_FINISHED {
            self.keys_done.remove(0);
        }
        self.keys_done.push((ticket, outcome));
    }

    /// The client table the writer would publish.
    #[must_use]
    pub fn table(&self) -> [Option<Client>; CLIENTS] {
        let mut table = [None; CLIENTS];
        for (slot, client) in table.iter_mut().zip(&self.clients) {
            *slot = Some(*client);
        }
        table
    }

    fn queued_feeds(&self) -> u32 {
        u32::try_from(self.feeds.len()).unwrap_or(u32::MAX)
    }
}

/// Every section at its widest, so a frame that holds these holds any.
fn widest(section: StatsSection) -> Stats<'static> {
    let counts = proto::Counts {
        uptime_s: u32::MAX,
        feeds: u32::MAX,
        feeds_failed: u32::MAX,
        portions: u32::MAX,
        presses: u32::MAX,
        sessions: u32::MAX,
        reconnects: u32::MAX,
        dhcp_renewals: u32::MAX,
        outages: u32::MAX,
        battery_s: u32::MAX,
    };
    let erases = proto::Erases {
        storage: [u32::MAX; proto::FLASH_SECTORS],
        update: u32::MAX,
    };

    match section {
        StatsSection::Counters => Stats::Counters(proto::Counters {
            boot: counts,
            reset: proto::ResetCause::Brownout.into(),
            lifetime: Some(counts),
            resets: Some(proto::Resets {
                power_on: u32::MAX,
                watchdog: u32::MAX,
                software: u32::MAX,
                brownout: u32::MAX,
                other: u32::MAX,
            }),
            handshake_max_ms: u32::MAX,
            handshake_mean_ms: u32::MAX,
        }),
        StatsSection::Memory => Stats::Memory(proto::Memory {
            heap_free: u32::MAX,
            heap_min: u32::MAX,
            tasks: [Some(proto::TaskStack {
                name: "0123456789abcdef",
                free_min: u32::MAX,
            }); proto::TASKS_MAX],
            tasks_total: u32::MAX,
        }),
        StatsSection::Flash => Stats::Flash(proto::Flash {
            boot: erases,
            lifetime: Some(erases),
            events_kept: u32::MAX,
            events_capacity: u32::MAX,
        }),
        StatsSection::Power => Stats::Power(proto::PowerStats {
            adapter: Some(false),
            adapter_changes: u32::MAX,
            outages: u32::MAX,
            battery_s: u32::MAX,
            battery_min_mv: Some(u32::MAX),
            battery_max_mv: Some(u32::MAX),
            battery_mv: [Some(u16::MAX); proto::POWER_HISTORY],
            period_s: u32::MAX,
        }),
        StatsSection::Food => Stats::Food(proto::Food {
            present: Some(true),
            ran_low: u32::MAX,
            refilled: u32::MAX,
            feeds_since_refill: Some(u32::MAX),
        }),
        StatsSection::Feeds => Stats::Feeds(proto::FeedStats {
            traced: u32::MAX,
            drive_ms: u32::MAX,
            samples: u32::MAX,
            unit: proto::CurrentUnit::MilliAmps.into(),
            current_peak: u32::MAX,
            current_mean: Some(u32::MAX),
            rising: u32::MAX,
            falling: u32::MAX,
            beam_breaks: u32::MAX,
            reversals: u32::MAX,
            kept: u32::MAX,
            capacity: u32::MAX,
            unsaved: u32::MAX,
            last: Some(widest_summary()),
        }),
        StatsSection::Portions => Stats::Portions(proto::Portions {
            portions: u32::MAX,
            homed: u32::MAX,
            outlet_breaks: u32::MAX,
            portion_mg: Some(u32::MAX),
            delivered_mg: Some(u64::MAX),
        }),
    }
}

/// A small fixed answer per section, each told apart by its values.
fn small(fake: &Fake, section: StatsSection, now_ms: u32) -> Stats<'static> {
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
            events_kept: u32::try_from(fake.events.len()).unwrap_or(u32::MAX),
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
            last: fake.trace.as_ref().map(|(summary, ..)| *summary),
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

/// A trace's summary at its widest.
fn widest_summary() -> proto::TraceSummary {
    proto::TraceSummary {
        feed: u32::MAX,
        requested: u8::MAX,
        completed: u8::MAX,
        outcome: proto::FeedOutcome::Uncertain.into(),
        reason: proto::FeedReason::Output.into(),
        drive_ms: u32::MAX,
        samples: u32::MAX,
        unit: proto::CurrentUnit::MilliAmps.into(),
        current_peak: u32::MAX,
        current_mean: Some(u32::MAX),
        rising: u16::MAX,
        falling: u16::MAX,
        first_edge_ms: Some(u32::MAX),
        beam_breaks: u16::MAX,
        reversals: u16::MAX,
        battery_rest_mv: Some(u16::MAX),
        battery_min_mv: Some(u16::MAX),
        stride: u16::MAX,
        truncated: true,
    }
}

/// The widest trace page: a whole page of body, at the largest offset.
static TRACE_PAGE: [u8; wellfed_proto::trace::PAGE_MAX] = [0xff; wellfed_proto::trace::PAGE_MAX];

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

    fn model(&self) -> Model {
        Model::Plaf103
    }

    fn status(&mut self, _now_ms: u32) -> Status<'static> {
        let (desiccant, feed_fault) = if self.widest {
            // The widest count and fault, so a frame that holds them holds
            // any.
            let fault = proto::FeedFault {
                feed: u32::MAX,
                outcome: proto::FeedOutcome::Uncertain.into(),
                reason: proto::FeedReason::MarkStuck.into(),
            };
            (proto::Desiccant::Remaining { days: i32::MIN }, Some(fault))
        } else {
            let desiccant = match self.settings.kind.desiccant(self.today) {
                Count::Unset => proto::Desiccant::Unset,
                Count::Unknown => proto::Desiccant::Unknown,
                Count::Remaining(days) => proto::Desiccant::Remaining { days },
            };
            (desiccant, None)
        };

        Status {
            build: self.build(),
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
                desiccant,
                food: FoodLevel::Unread.into(),
                feed_fault,
                blocked: self.readiness().blocked(),
                supply: proto::Supply {
                    adapter: None,
                    battery: proto::Battery::Unread,
                    below_floor: None,
                },
                ext: proto::Tail::EMPTY,
            }),
            ext: proto::Tail::EMPTY,
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

    fn time_sync(&mut self, fields: TimeFields, origin: Origin) -> TimeResult {
        self.syncs += 1;
        TimeResult::from_sync(self.clock.sync(1, fields.sync(), origin))
    }

    fn keys(&mut self, change: KeyChange) -> Option<u32> {
        if self.keys_pending.is_some() {
            return None;
        }
        self.ticket += 1;
        self.keys_pending = Some((self.ticket, change));
        Some(self.ticket)
    }

    /// `Lost` once the change finished and left the ring, as the device
    /// tells it.
    fn keys_outcome(&mut self, ticket: u32) -> Option<KeyOutcome> {
        if self
            .keys_pending
            .is_some_and(|(queued, _)| queued == ticket)
        {
            return None;
        }
        Some(
            self.keys_done
                .iter()
                .find(|&&(done, _)| done == ticket)
                .map_or(KeyOutcome::Lost, |&(_, outcome)| outcome),
        )
    }

    fn stats(&mut self, section: StatsSection, now_ms: u32) -> Stats<'_> {
        if self.widest {
            widest(section)
        } else {
            small(self, section, now_ms)
        }
    }

    fn bank1_page(&mut self, offset: u32) -> Result<proto::Bank1Page<'_>, Refusal> {
        if self.installing {
            return Err(Refusal::Busy);
        }
        let bank = self.bank1.as_deref().ok_or(Refusal::Unavailable)?;
        let rest = usize::try_from(offset)
            .ok()
            .and_then(|offset| bank.get(offset..))
            .filter(|rest| !rest.is_empty())
            .ok_or(Refusal::Invalid)?;
        let len = rest.len().min(proto::BANK1_PAGE_MAX);
        Ok(proto::Bank1Page {
            offset,
            total: u32::try_from(bank.len()).map_err(|_| Refusal::Invalid)?,
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
            clock: self.clock.state(now_ms),
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
            UpdateRequest::Statement { statement } => {
                let mut id = [0; 8];
                id.copy_from_slice(&statement[..8]);
                Ok(UpdateAnswer::Key {
                    change: UpdateKeyChange::Retired,
                    id,
                })
            }
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

    /// `Lost` once the request finished and a later one's answer replaced
    /// its own, as the device tells it.
    fn update_outcome(&mut self, ticket: u32) -> Option<Result<UpdateAnswer, FrozenRefusal>> {
        if self.held.is_some_and(|(queued, _)| queued == ticket) {
            return None;
        }
        Some(
            self.update_done
                .filter(|&(done, _)| done == ticket)
                .map_or(Ok(UpdateAnswer::Lost), |(_, answer)| answer),
        )
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
        self.reserved < self.queued_feeds()
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
                let Ok(mut raw) = proto::RawEvent::encode(e) else {
                    unreachable!("the fake records only bodies that fit the ring");
                };
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

    fn calibration(&mut self, change: Option<Calibration>) -> CalibrationResult {
        if let Some(calibration) = change {
            self.calibration.calibration = calibration;
            self.calibration.generation += 1;
        }
        self.calibration.writable = true;
        CalibrationResult::Current {
            view: self.calibration,
            pending: false,
        }
    }

    fn phase(&self) -> Option<proto::Phase> {
        Some(proto::Phase::Unknown)
    }

    fn plans(&mut self, _change: Option<Asked<Table>>) -> PlansResult {
        self.plans.writable = true;
        PlansResult::Current {
            view: self.plans,
            pending: false,
        }
    }

    /// The writer lands it at once: the mark moves to the meal, or back a
    /// day.
    fn skip_meal(
        &mut self,
        slot: u8,
        due_s: u32,
        skip: Asked<MealSkip>,
        _now_ms: u32,
    ) -> Result<(), Refusal> {
        let Asked { change: skip, by } = skip;
        if let Some(refusal) = self.skip_refusal {
            return Err(refusal);
        }
        self.skips += 1;
        let Some(mark) = self.plans.marks.get_mut(usize::from(slot)) else {
            return Err(Refusal::NotFound);
        };
        if self.skip_dropped {
            return Ok(());
        }

        let bit = 1 << slot;
        (*mark, self.plans.skips) = match skip {
            MealSkip::Skip => (due_s, self.plans.skips | bit),
            MealSkip::Unskip => (due_s - 1, self.plans.skips & !bit),
        };
        // The writer's event, as it saves the change.
        let body = match skip {
            MealSkip::Skip => EventBody::Skipped {
                plan: slot,
                due: due_s,
                by,
            },
            MealSkip::Unskip => EventBody::Unskipped {
                plan: slot,
                due: due_s,
                by,
            },
        };
        self.record(0, body);
        Ok(())
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

    fn feed_trace(&mut self, feed: u32, offset: u16) -> Result<proto::TracePage<'_>, Refusal> {
        if self.widest {
            return Ok(proto::TracePage {
                summary: widest_summary(),
                stored: true,
                offset,
                total: u16::MAX,
                bytes: &TRACE_PAGE,
            });
        }
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
            total: u16::try_from(body.len()).map_err(|_| Refusal::Invalid)?,
            bytes: &rest[..len],
        })
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
        if self.operating() {
            return Err(Refusal::Busy);
        }
        self.feeds.push((portions, by));
        Ok(self.queued_feeds())
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

/// A rig's device: the fake, whatever kind it is served as.
pub trait Faked: Device {
    fn fake(&mut self) -> &mut Fake;
    fn fake_ref(&self) -> &Fake;
}

impl Faked for Fake {
    fn fake(&mut self) -> &mut Fake {
        self
    }
    fn fake_ref(&self) -> &Fake {
        self
    }
}

/// A device of another kind: the fake's shared core, and no feeder module
/// ([`NoFeeder`]).
#[derive(Default)]
pub struct Fountain(pub Fake);

impl Faked for Fountain {
    fn fake(&mut self) -> &mut Fake {
        &mut self.0
    }
    fn fake_ref(&self) -> &Fake {
        &self.0
    }
}

impl Device for Fountain {
    type Feeder = NoFeeder;

    fn feeder(&mut self) -> Option<&mut NoFeeder> {
        None
    }
    fn random(&mut self, out: &mut [u8; 32]) -> bool {
        self.0.random(out)
    }
    fn build(&self) -> &'static str {
        self.0.build()
    }
    fn model(&self) -> Model {
        Model::Plwf106
    }
    fn status(&mut self, now_ms: u32) -> Status<'static> {
        self.0.status(now_ms)
    }
    fn wifi(&mut self, change: Option<Asked<Option<Credentials>>>) -> WifiResult {
        self.0.wifi(change)
    }
    fn time_sync(&mut self, fields: TimeFields, origin: Origin) -> TimeResult {
        self.0.time_sync(fields, origin)
    }
    fn time_settings(&mut self, change: Option<TimeSettings>) -> TimeSettingsResult {
        self.0.time_settings(change)
    }
    fn time_state(&mut self, now_ms: u32) -> Option<TimeState> {
        self.0.time_state(now_ms)
    }
    fn keys(&mut self, change: KeyChange) -> Option<u32> {
        self.0.keys(change)
    }
    fn keys_outcome(&mut self, ticket: u32) -> Option<KeyOutcome> {
        self.0.keys_outcome(ticket)
    }
    fn stats(&mut self, section: StatsSection, now_ms: u32) -> Stats<'_> {
        self.0.stats(section, now_ms)
    }
    fn record(&mut self, now_ms: u32, body: EventBody) {
        self.0.record(now_ms, body);
    }
    fn event_after(&mut self, reader: Reader, after: Option<u64>) -> EventRead {
        self.0.event_after(reader, after)
    }
    fn epoch(&self) -> Option<u32> {
        self.0.epoch()
    }
    fn reboot(&mut self) {
        self.0.reboot();
    }
    fn log_after(&mut self, after: Option<u32>, text: &mut [u8]) -> Option<LogBatch> {
        self.0.log_after(after, text)
    }
}
