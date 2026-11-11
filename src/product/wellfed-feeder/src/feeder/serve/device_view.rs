//! [`DeviceView`], the server's view of the rest of the feeder for one
//! pass, and its shared-core half ([`Device`]): the platform's mailboxes and
//! clock, the boot's facts, the statistics, events, alerts and updates. The
//! feeder's own half is in [`super::feeder_view`].

use wellfed_core::log_tail::Batch as LogBatch;
use wellfed_core::ota::install::{Answer, Phase};
use wellfed_core_feeder::desiccant::{self, Count};
use wellfed_device::store::events::Read;
use wellfed_device::update as wire;
use wellfed_device_api::mailbox::{
    Asked, CalibrationResult, ClientChange, KeyResult, NtpResult, Origin, Requested, Stamp,
    TimeFields, TimeResult, TimeSettings, TimeSettingsResult, TimeState, WifiResult, WifiSource,
};
use wellfed_device_api::wifi::Credentials;
use wellfed_device_api::{
    Device, EventRead, FeederDevice, KeyChange, KeyOutcome, Reader, UpdateAnswer, UpdateKeyChange,
    UpdateRequest,
};
use wellfed_proto::api::{
    self as proto, AlertsView, BANK1_PAGE_MAX, Bank1Page, Desiccant, DeviceStatus, EventBody,
    FeederStatus, FrozenRefusal, Refusal, Source, Stats, StatsSection, Status, Supply, Tail,
    UpdateRefusal, UpdateView,
};
use wellfed_proto::device::Model;
use wellfed_signed::BANK1_BYTES;
use wellfed_signed::offer::{KeyStatement, key_id};

use super::super::Feeds;
use super::super::events;
use super::super::lock::LockBox;
use super::super::platform::{Boot, Platform};
use super::PAGE_BYTES;
use super::telemetry::TaskCache;
use crate::stats::Keeper;

// A store write waits while a feed holds the outputs (a flash lease then
// would reset the board), longer than a client waits for its answer. So a
// client's write during a feed is refused `Busy`, which changed nothing,
// rather than left unanswered and applied after the feed.

/// The rest of the feeder, as the server sees it.
pub struct DeviceView<'a, P, F> {
    pub(in crate::feeder) platform: &'a mut P,
    pub(in crate::feeder) lock: &'a mut LockBox,
    pub(in crate::feeder) feeds: &'a mut F,
    pub(in crate::feeder) boot: Boot,
    pub(in crate::feeder) locked: Option<bool>,
    /// BLE setup mode runs.
    pub(in crate::feeder) setup: bool,
    /// The pass's time, which a clock sync is anchored to.
    pub(in crate::feeder) now_ms: u32,
    /// What the last NTP exchange came to.
    pub(in crate::feeder) ntp_last: Option<NtpResult>,
    pub(in crate::feeder) stats: &'a mut Keeper,
    pub(in crate::feeder) tasks: &'a mut TaskCache,
    /// The page a trace or bank-1 answer borrows its bytes from.
    pub(in crate::feeder) page: &'a mut [u8; PAGE_BYTES],
    /// A brownout holds feeding until the adapter is seen.
    pub(in crate::feeder) battery_hold: bool,
    /// The supply this pass judged.
    pub(in crate::feeder) supply: Supply,
    /// A plan's claimed feed must be the next one.
    pub(in crate::feeder) meal: bool,
    /// A factory reset is queued or marked: an install offer is refused,
    /// so the reset's boot never becomes an update's trial boot.
    pub(in crate::feeder) reset: bool,
    /// A plan's claim is in flight: counted as a feed, so a client's
    /// reboot (and an install) is refused `Busy` rather than cutting the
    /// meal before its feed is queued.
    pub(in crate::feeder) claim: bool,
    /// When the next meal call's lead begins (Unix ms), if one is to come.
    pub(in crate::feeder) call_lead: Option<u64>,
}

impl<P: Platform, F: Feeds> DeviceView<'_, P, F> {
    /// A client's change, stamped with the wall time now: what its history
    /// event, which the writer records as it saves the change, carries.
    pub(super) fn stamp(&mut self, by: Source) -> Stamp {
        Stamp {
            by,
            at: events::wall_s(self.platform, self.now_ms),
        }
    }

    /// Whether a change that writes must be refused as during a feed: the
    /// outputs are claimed, or a feed waits for the writes ahead of it, which
    /// this write would join.
    pub(super) fn outputs_claimed(&self) -> bool {
        self.platform.outputs_claimed() || self.feeds.claiming().is_some()
    }
}

impl<P: Platform, F: Feeds> Device for DeviceView<'_, P, F> {
    type Feeder = Self;

    fn feeder(&mut self) -> Option<&mut Self> {
        Some(self)
    }

    fn random(&mut self, out: &mut [u8; 32]) -> bool {
        self.platform.random(out)
    }

    fn build(&self) -> &'static str {
        self.platform.build()
    }

    fn model(&self) -> Model {
        self.platform.model()
    }

    fn status(&mut self, now_ms: u32) -> Status<'static> {
        let link = self.platform.link();
        let settings = self.platform.settings();
        let settings_generation = settings.map_or(0, |view| view.generation);
        let desiccant = desiccant_now(self.platform, now_ms);
        let wifi_generation = match self.platform.wifi() {
            WifiResult::Current { view, .. } => view.generation,
            _ => 0,
        };
        Status {
            build: self.platform.build(),
            bank: self.boot.bank,
            uptime_ms: now_ms,
            boot_count: self.boot.boot_count,
            reset_flags: self.boot.reset_flags,
            faults: 0,
            rssi: link.rssi,
            wifi: match self.platform.wifi_source() {
                WifiSource::Build => proto::WifiSource::Build,
                WifiSource::Stored => proto::WifiSource::Stored,
                WifiSource::Fallback => proto::WifiSource::Fallback,
            }
            .into(),
            link_losses: link.link_losses,
            dns_losses: link.check_losses,
            clock_ms: self.platform.wall_ms(now_ms),
            power: self
                .platform
                .power(now_ms)
                .map(|power| super::super::supply::power(power, self.supply.battery)),
            wifi_generation,
            last_event: self.platform.last_event(),
            sessions: 0,
            alerts_generation: self.platform.alerts().generation(),
            device: DeviceStatus::Feeder(FeederStatus {
                locked: self.locked,
                settings_generation,
                desiccant,
                food: crate::feed::food_level(self.feeds.food()).into(),
                feed_fault: self.stats.feed_fault(),
                blocked: FeederDevice::readiness(self).blocked(),
                supply: self.supply,
                ext: Tail::EMPTY,
            }),
            ext: Tail::EMPTY,
        }
    }

    fn wifi(&mut self, change: Option<Asked<Option<Credentials>>>) -> WifiResult {
        match change {
            None => self.platform.wifi(),
            Some(_) if self.outputs_claimed() => WifiResult::Busy,
            Some(Asked { change, by }) => {
                let asked = self.stamp(by);
                self.platform.wifi_change(change, Some(asked))
            }
        }
    }

    fn time_sync(&mut self, fields: TimeFields, origin: Origin) -> TimeResult {
        TimeResult::from_sync(self.platform.time_sync(self.now_ms, fields.sync(), origin))
    }

    fn time_settings(&mut self, change: Option<TimeSettings>) -> TimeSettingsResult {
        match change {
            None => self.platform.time_settings(),
            Some(_) if self.outputs_claimed() => TimeSettingsResult::Busy,
            Some(settings) => self.platform.time_settings_change(settings),
        }
    }

    fn time_state(&mut self, now_ms: u32) -> Option<TimeState> {
        Some(TimeState {
            clock: self.platform.clock_state(now_ms),
            now_ms,
            ntp_last: self.ntp_last,
        })
    }

    /// `None` while the outputs are claimed, as when the slot is taken: a
    /// revocation is refused `Busy`, and an enrollment asks again.
    fn keys(&mut self, change: KeyChange) -> Option<u32> {
        if self.outputs_claimed() {
            return None;
        }
        let at = events::wall_s(self.platform, self.now_ms);
        let change = ClientChange::Key { change, at };
        (self.platform.keys_request(change) == Requested::Queued)
            .then(|| self.platform.keys_ticket().max(1))
    }

    /// `Lost` once a later change was queued and this one's result is not
    /// among those the mailbox keeps: changes queue one at a time, so a
    /// later ticket means this one finished, and its result aged out.
    ///
    /// That holds only while every key request and every poll of a result
    /// runs in the loop's task, as they do now (the device API, BLE setup
    /// and the panel's factory reset). A requester in another task could queue a change between
    /// this result's read and the ticket's, and a result arriving just then
    /// would be called lost.
    fn keys_outcome(&mut self, ticket: u32) -> Option<KeyOutcome> {
        let Some(result) = self.platform.keys_outcome(ticket) else {
            return (self.platform.keys_ticket().max(1) != ticket).then_some(KeyOutcome::Lost);
        };
        Some(match result {
            KeyResult::Done => KeyOutcome::Done,
            KeyResult::Full => KeyOutcome::Full,
            KeyResult::Duplicate => KeyOutcome::Duplicate,
            KeyResult::NotFound => KeyOutcome::NotFound,
            KeyResult::Unarmed
            | KeyResult::NoGenerator
            | KeyResult::NotWritable
            | KeyResult::Io => KeyOutcome::NotWritable,
        })
    }

    fn record(&mut self, now_ms: u32, body: EventBody) {
        events::record(self.platform, now_ms, body);
    }

    fn event_after(&mut self, reader: Reader, after: Option<u64>) -> EventRead {
        match self.platform.event_after(reader, after) {
            None => EventRead::Wait,
            Some(Read::End) => EventRead::None,
            Some(Read::Gap { from, to }) => EventRead::Gap { from, to },
            Some(Read::Record(record)) => EventRead::Event(proto::RawEvent {
                seq: record.seq,
                at: record.at,
                body: record.body,
            }),
        }
    }

    fn reader_left(&mut self, reader: Reader) {
        self.platform.reader_left(reader);
    }

    fn alerts(&mut self) -> Option<AlertsView> {
        let epoch = self.platform.epoch()?;
        Some(self.platform.alerts().view(epoch))
    }

    fn epoch(&self) -> Option<u32> {
        // On bank 1 no session is welcomed while the alert rules' boot read
        // has its first seconds to itself. Bank 0 is the recovery bank, there
        // for installs: an installer reconnecting after a give-way is
        // welcomed as soon as the ring opens, and the read shares the turns.
        let epoch = self.platform.epoch()?;
        (self.boot.bank == 0 || self.platform.alerts().welcoming()).then_some(epoch)
    }

    fn reboot(&mut self) {
        // The writer's restart save takes this boot's counts as they are now.
        self.platform.hand_counts(self.stats.counts(self.now_ms));
        events::record(self.platform, self.now_ms, EventBody::Restarting);
        self.platform.reset();
    }

    fn stats(&mut self, section: StatsSection, now_ms: u32) -> Stats<'_> {
        match section {
            StatsSection::Counters => self.counters_section(now_ms),
            StatsSection::Memory => self.memory_section(now_ms),
            StatsSection::Flash => self.flash_section(),
            StatsSection::Power => Stats::Power(self.stats.power_stats()),
            StatsSection::Food => Stats::Food(self.stats.food_stats()),
            StatsSection::Feeds => Stats::Feeds(self.stats.feed_stats(self.platform.traces_fill())),
            StatsSection::Portions => {
                let portion_mg = match self.platform.calibration() {
                    CalibrationResult::Current { view, .. } => view.calibration.portion_mg,
                    _ => None,
                };
                Stats::Portions(self.stats.portions_stats(self.feeds.marks(), portion_mg))
            }
        }
    }

    /// Bank 1 as the flash holds it. Only the bank-0 role reads it: there
    /// it may still hold stock, the feeder's only copy until a promotion
    /// overwrites it, while on bank 1 it is the running build. An install
    /// rewrites it under the read, so none is served while one holds the
    /// flash. Nothing here vouches for the bytes: the installer checks the
    /// copy against the pinned stock image.
    fn bank1_page(&mut self, offset: u32) -> Result<Bank1Page<'_>, Refusal> {
        if self.boot.bank != 0 {
            return Err(Refusal::Unavailable);
        }
        if self.platform.installing() {
            return Err(Refusal::Busy);
        }

        let left = BANK1_BYTES
            .checked_sub(offset)
            .filter(|&left| left > 0)
            .ok_or(Refusal::Invalid)?;
        let len = usize::try_from(left).map_or(BANK1_PAGE_MAX, |left| left.min(BANK1_PAGE_MAX));
        let out = self.page.get_mut(..len).ok_or(Refusal::Busy)?;
        if !self.platform.bank1_read(offset, out) {
            return Err(Refusal::Unavailable);
        }
        Ok(Bank1Page {
            offset,
            total: BANK1_BYTES,
            bytes: out,
        })
    }

    /// Stock refuses an install on its batteries: one that browns out
    /// mid-write costs nothing (the marker comes last), but the radio goes
    /// off within seconds and the install would time out. A cancel and a
    /// key statement still go through.
    fn update(&mut self, request: UpdateRequest<'_>) -> Result<u32, FrozenRefusal> {
        if matches!(request, UpdateRequest::Offer { .. }) && self.supply.adapter == Some(false) {
            return Err(FrozenRefusal::Update(UpdateRefusal::Battery));
        }
        let ticket = match request {
            UpdateRequest::Offer { .. } if self.reset => return Err(FrozenRefusal::Busy),
            UpdateRequest::Offer {
                manifest,
                downgrade,
                device,
            } => self.platform.update_offer(manifest, downgrade, device),
            UpdateRequest::Statement { statement } => self.platform.update_statement(statement),
            UpdateRequest::Cancel => self.platform.update_cancel(),
        };
        ticket.map_err(wire::refusal)
    }

    fn update_outcome(&mut self, ticket: u32) -> Option<Result<UpdateAnswer, FrozenRefusal>> {
        let Some(answer) = self.platform.update_outcome(ticket) else {
            return self
                .platform
                .update_lost(ticket)
                .then_some(Ok(UpdateAnswer::Lost));
        };
        Some(match answer {
            Ok(Answer::Progress(progress)) => Ok(UpdateAnswer::View(wire::view(&progress))),
            Ok(Answer::Key(KeyStatement::Revoke(id))) => Ok(UpdateAnswer::Key {
                change: UpdateKeyChange::Retired,
                id,
            }),
            Ok(Answer::Key(KeyStatement::Add(key))) => Ok(UpdateAnswer::Key {
                change: UpdateKeyChange::Added,
                id: key_id(&key),
            }),
            Err(refused) => Err(wire::refusal(refused)),
        })
    }

    fn update_chunk(&mut self, offset: u32, data: &[u8]) -> Result<UpdateView, FrozenRefusal> {
        let progress = self.platform.update_progress().ok_or(FrozenRefusal::Busy)?;
        match progress.phase {
            Phase::Receiving => {}
            // The last chunk's answer may have been lost: the state says so.
            Phase::Activated => return Ok(wire::view(&progress)),
            Phase::Idle | Phase::Checking | Phase::GivingWay => {
                return Err(FrozenRefusal::Update(UpdateRefusal::NotInstalling));
            }
        }
        self.platform
            .update_chunk(offset, data)
            .map_err(wire::refusal)?;
        let progress = self.platform.update_progress().ok_or(FrozenRefusal::Busy)?;
        Ok(wire::view(&progress))
    }

    fn update_view(&mut self) -> Option<UpdateView> {
        self.platform
            .update_progress()
            .map(|progress| wire::view(&progress))
    }

    fn installing(&self) -> bool {
        self.platform.installing()
    }

    fn operating(&self) -> bool {
        self.feeds.busy() || self.claim
    }

    fn log_after(&mut self, after: Option<u32>, text: &mut [u8]) -> Option<LogBatch> {
        self.platform.log_after(after, text)
    }
}

/// The local calendar day, while the clock is valid.
pub(super) fn today<P: Platform>(platform: &mut P, now_ms: u32) -> Option<i32> {
    let reading = platform.clock_state(now_ms).reading?;
    desiccant::today(&reading)
}

/// The desiccant reminder as it stands at `now_ms`, as the status and the
/// alert rules read it.
pub(in crate::feeder) fn desiccant_now<P: Platform>(platform: &mut P, now_ms: u32) -> Desiccant {
    let today = today(platform, now_ms);
    match platform.settings().map(|view| view.kind.desiccant(today)) {
        Some(Count::Remaining(days)) => Desiccant::Remaining { days },
        Some(Count::Unset) => Desiccant::Unset,
        Some(Count::Unknown) | None => Desiccant::Unknown,
    }
}
