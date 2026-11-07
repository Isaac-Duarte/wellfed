//! [`SimPlatform`], the product loop's [`Platform`] in the simulator: the
//! firmware's real stores on a [`Flash`] image, a writer turn that stands in
//! for the settings task, the update stage's real mailbox, and the device
//! API's sockets on the host.
//!
//! The firmware's mailboxes answer a write at once and complete it on the
//! settings task's next turn; the server polls for the result. This keeps
//! that shape: a change is queued, [`SimPlatform::write`] applies it after
//! the pass, and the server sees it complete on a later pass, so clients
//! meet the same asynchronous answers they get from hardware. Events, which
//! the firmware queues for the writer too, are appended at once: nothing
//! reads them between the queue and the ring.
//!
//! The listener belongs to the serve loop, which awaits its connections and
//! hands each one over ([`SimPlatform::accepted`]); the loop's driver then
//! takes it as it takes one from lwIP. What the board provides and the
//! simulator does not have (the indicator, the speaker, a feeding owner)
//! answers as an idle board would. Setup mode's BLE link is a pair of
//! message queues a test plays the phone on ([`SimPlatform::ble_connect`]):
//! whole messages each way, with no framing or radio.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::convert::Infallible;
use std::net::Ipv4Addr;
use std::rc::Rc;
use std::time::{Duration, Instant};

use tokio::net::{TcpStream, UdpSocket};
use wellfed_board::storage;
use wellfed_core::buttons::Gesture;
use wellfed_core::clock::wall::{Fix, Jump, Origin, State as ClockState, Sync, SyncError};
use wellfed_core::clock::zone::Rule;
use wellfed_core::indicator::Light;
use wellfed_core::log_tail::{Batch as LogBatch, LogTail};
use wellfed_core::ota::install::{Answer, CandidateState, Progress, Refused};
use wellfed_core::supply::{self, Reading, Sample};
use wellfed_core::tone::Tune;
use wellfed_device::io::{Accepted, BleDrop, Datagram, Received};
use wellfed_device::store::events::{BODY_BYTES, EventsFill, Read};
use wellfed_device::store::net::BuildWifi;
use wellfed_device::store::traces::{self, TraceRead, TraceRing, TracesFill};
use wellfed_device_api::Reader;
use wellfed_device_api::clients::{CLIENTS, Client, Label, Role};
use wellfed_device_api::mailbox::{
    Calibration, CalibrationResult, CalibrationView, CallChange, CallResult, ClientChange,
    KeyResult, PlansChange, PlansResult, PlayResult, Requested, SettingsFields, SettingsResult,
    SettingsView, TimeSettings, TimeSettingsResult, WifiResult, WifiSource, chunk_wanted,
};
use wellfed_device_api::wifi::Credentials;
use wellfed_feeder::feeder::Platform;
use wellfed_feeder::feeder::alerts::Alerts;
use wellfed_feeder::feeder::platform::{
    BleStep, Boot, Check, DriverStep, EventSlot, Link, TaskCount, TaskStack,
};
use wellfed_feeder::feeder::supply::Radio;
use wellfed_feeder::stats::{SAVE_MS, TOTAL_SECTORS, Totals, reset_cause};
use wellfed_feeder::store::calibration::CalibrationStore;
use wellfed_feeder::store::call::CallStore;
use wellfed_feeder::store::plans::PlansStore;
use wellfed_feeder::store::stats::{SaveKind, StatsStatus, StatsStore};
use wellfed_hal::interlock::INTERLOCK;
use wellfed_hal::ota::mailbox::{self, Request};
use wellfed_hal::power;
use wellfed_hal::sdk::LOG_TAIL_BYTES;
use wellfed_proto::api::{self as proto, Counts, FLASH_SECTORS, TASKS_MAX};
use wellfed_proto::device::Model;

/// The model of the board the simulator stands in for, by its code; one that
/// names no model does not build.
const MODEL: Model = match Model::from_byte(wellfed_board::MODEL_CODE) {
    Some(model) => model,
    None => panic!("the board's code names no model"),
};
use wellfed_proto::trace::PAGE_MAX;

use wellfed_device_sim::ble::Ble;
pub use wellfed_device_sim::sockets::Socket;
use wellfed_device_sim::sockets::Sockets;
use wellfed_device_sim::stores::{self, DeviceStores};
pub use wellfed_device_sim::stores::{DeviceError, Random};

use crate::banks::Banks;
use crate::flash::{Flash, Region};

// The stats record counts erases of every sector the region has; growing
// the region changes the record and takes a new version of it.
const _: () = assert!(wellfed_board::STORAGE_SECTORS as usize == TOTAL_SECTORS);

/// What the simulated board's supply reads: the adapter's detect pin, and
/// the battery channel's millivolts at the pin (`None` with no pack), on
/// the PLAF103's divider and curve (`wellfed_hal::power::PACK`). The
/// operator sets it ([`crate::serve::Control::Supply`]); it is fresh at
/// every pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SimSupply {
    pub adapter: bool,
    pub pin_mv: Option<u32>,
}

impl SimSupply {
    /// On the adapter, with no batteries: the simulator's default.
    pub const MAINS: Self = Self {
        adapter: true,
        pin_mv: None,
    };

    /// The latest sample, as the monitor publishes it.
    fn sample(self) -> Sample {
        Sample {
            pin_mv: self.pin_mv.unwrap_or(0),
            adapter: self.adapter,
            age_ms: 0,
        }
    }
}

/// A signal the simulator reports, as a feeder near its access point.
pub(crate) const RSSI: i32 = -48;
/// The channel the simulator reports its access point on.
pub(crate) const CHANNEL: u8 = 6;

/// How often the simulator's supply "is sampled", as the feeder's monitor.
const POWER_SAMPLE_MS: u32 = 5_000;

/// Setup mode's link waits this long between steps: there is no radio.
const NO_RADIO_MS: u32 = 1_000;

pub struct SimPlatform {
    /// The stores every device keeps, and their mailboxes.
    device: DeviceStores,
    /// The device API's sockets on the host.
    sockets: Sockets,
    /// Setup mode's BLE link.
    ble: Ble,
    flash: Rc<Flash>,
    /// The image's erases when this boot opened it: the statistics count
    /// this boot's.
    erases_before: [u32; TOTAL_SECTORS],
    /// The lifetime statistics, as the writer keeps them: the stored
    /// totals with this boot counted, this boot's counts as the loop last
    /// handed them, and when the next save is due.
    stats: StatsStore<Region>,
    stats_base: Totals,
    counts: Counts,
    stats_due: Instant,
    /// When this boot opened, for a restart's save kind.
    booted: Instant,
    calibration: CalibrationStore<Region>,
    calibration_pending: Option<Calibration>,
    plans: PlansStore<Region>,
    plans_pending: Option<PlansChange>,
    /// The writer leaves a queued plans change queued: a slow write.
    plans_held: bool,
    call: CallStore<Region>,
    /// An upload's start or a delete, or a chunk at its offset with its
    /// bytes, as the writer's one slot holds them.
    call_pending: Option<CallWork>,
    reboot: bool,
    /// This process's boots so far, this one included; status reports it as
    /// the feeder reports its boot count.
    boot_count: u32,
    /// The bank this boot runs, and its build's identity as hex.
    bank: u8,
    build: &'static str,
    /// Gestures from the operator's console, for the loop to take.
    gestures: VecDeque<Gesture>,
    /// The lines this boot logged, as the firmware's RAM tail keeps them;
    /// the simulator copies its records in ([`crate::sim::Sim::pass`]).
    log: Rc<LogTail<LOG_TAIL_BYTES>>,
    /// The two banks, which the bank-0 role reads bank 1 of for the stock
    /// backup; `None` for a platform opened without them.
    banks: Option<Rc<RefCell<Banks>>>,
    /// What the supply reads.
    supply: SimSupply,
    /// The ROM's reset flags this boot reports: none, a power-on, unless a
    /// test sets them before the first pass.
    reset_flags: u32,
    /// The radio the loop last asked for: while off, no connection is
    /// taken and those open are dropped, as the network leaves.
    radio: Radio,
    /// A bench image's build network, unless a test boots it as a
    /// consumer image ([`Self::set_build_wifi`]).
    build_wifi: BuildWifi,
    alerts: Alerts,
    alerts_on: bool,
    /// The ring's newest record when this boot opened it.
    ring_opened_at: Option<u64>,
    /// What a test holds back: the plans' load, the ring's answers and its
    /// opening.
    plans_loading: bool,
    ring_unanswered: bool,
    ring_closed: bool,
    /// The update store's candidate as the progress shows it, in place of
    /// the stage's: a trial that has not confirmed yet, or has.
    candidate_held: Option<CandidateState>,
    /// Answers the ring gave a reader outside the session slots.
    outside_reads: u32,
}

impl std::fmt::Debug for SimPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimPlatform")
            .field("device", &self.device)
            .finish_non_exhaustive()
    }
}

impl SimPlatform {
    /// Open every store on `flash`, creating the identity on first use as
    /// the writer does, for boot number `boot_count`. With `mdns`, the
    /// device API is advertised on it as `ip`.
    pub fn open(
        flash: &Rc<Flash>,
        random: Random,
        boot_count: u32,
        bank: u8,
        build: [u8; 32],
        mdns: Option<(UdpSocket, Ipv4Addr)>,
    ) -> Result<Self, DeviceError> {
        stores::finish_reset(flash, keep_wear);
        stores::finish_handover(flash, bank, &[storage::PLANS]);
        let device = DeviceStores::open(flash, random)?;
        let ring_opened_at = device.last_event();
        let mut stats = StatsStore::default();
        stats.open(Region::new(flash, storage::STATS));
        let mut stats_base = stats.totals;
        // The simulator's boots carry no reset flags: each is a power-on.
        stats_base.count_boot(reset_cause(0));
        let mut calibration = CalibrationStore::default();
        calibration.open(Region::new(flash, storage::CALIBRATION));
        let mut plans = PlansStore::default();
        plans.open(Region::new(flash, storage::PLANS));
        let mut call = CallStore::default();
        call.open(Region::new(flash, storage::CALL));

        let mut platform = Self {
            device,
            sockets: Sockets::new(mdns),
            ble: Ble::default(),
            flash: Rc::clone(flash),
            erases_before: flash.erases(),
            stats,
            stats_base,
            counts: Counts::default(),
            stats_due: Instant::now() + Duration::from_millis(u64::from(SAVE_MS)),
            booted: Instant::now(),
            calibration,
            calibration_pending: None,
            plans,
            plans_pending: None,
            plans_held: false,
            call,
            call_pending: None,
            reboot: false,
            boot_count,
            bank,
            // One per boot, for the welcome's borrowed build.
            build: Box::leak(
                wellfed_session::hex::Lower(&build)
                    .to_string()
                    .into_boxed_str(),
            ),
            gestures: VecDeque::new(),
            log: Rc::new(LogTail::new()),
            banks: None,
            supply: SimSupply::MAINS,
            reset_flags: 0,
            radio: Radio::On,
            build_wifi: BuildWifi::Present,
            alerts: Alerts::new(),
            alerts_on: true,
            ring_opened_at,
            plans_loading: false,
            candidate_held: None,
            ring_unanswered: false,
            ring_closed: false,
            outside_reads: 0,
        };
        // The boot's own early save, so its reset counts however short it is.
        platform.save_now(SaveKind::Early);
        Ok(platform)
    }

    /// Read bank 1 from these banks, as the firmware reads it through the
    /// flash window (from bank 0 only).
    pub fn attach_banks(&mut self, banks: &Rc<RefCell<Banks>>) {
        self.banks = Some(Rc::clone(banks));
    }

    /// Every client's window shut (`true`), so no send takes a byte, or open.
    pub fn block_sends(&mut self, blocked: bool) {
        self.sockets.block_sends(blocked);
    }

    /// What the supply reads from now on.
    pub fn set_supply(&mut self, supply: SimSupply) {
        self.supply = supply;
    }

    /// Report `flags` as this boot's reset flags (bit 18 a brownout), from
    /// the first pass on.
    pub fn set_reset_flags(&mut self, flags: u32) {
        self.reset_flags = flags;
    }

    /// Boot as an image with or without build Wi-Fi, from the first pass
    /// on: without, and with nothing stored, the boot starts setup mode.
    pub fn set_build_wifi(&mut self, build_wifi: BuildWifi) {
        self.build_wifi = build_wifi;
    }

    /// Whether setup mode advertises, so a phone can connect.
    pub fn ble_advertising(&self) -> bool {
        self.ble.advertising()
    }

    /// A phone connects and subscribes, as the firmware's driver reports
    /// both; false while nothing advertises or a phone is connected.
    pub fn ble_connect(&mut self) -> bool {
        self.ble.connect()
    }

    /// The connected phone writes one whole message.
    pub fn ble_write(&mut self, message: &[u8]) {
        self.ble.write(message);
    }

    /// What the feeder notified since the last call, one message each.
    pub fn ble_notified(&mut self) -> Vec<Vec<u8>> {
        self.ble.notified()
    }

    /// What the supply reads.
    pub fn sim_supply(&self) -> SimSupply {
        self.supply
    }

    /// The radio the loop last asked for.
    pub fn radio_state(&self) -> Radio {
        self.radio
    }

    /// The trace ring's sectors.
    fn traces(&self) -> Region {
        Region::new(&self.flash, storage::TRACES)
    }

    /// The identity secret the client store holds.
    pub fn identity(&self) -> Option<&[u8; 32]> {
        self.device.identity()
    }

    /// Enroll `key` straight into the store, outside the protocol: how the
    /// simulator admits its first owner without BLE setup
    /// ([`DeviceStores::enroll`]).
    pub fn enroll(&mut self, key: [u8; 32], label: Label, role: Role) -> Result<(), DeviceError> {
        self.device.enroll(key, label, role)
    }

    /// A gesture from the buttons, for the next pass.
    pub fn gesture_in(&mut self, gesture: Gesture) {
        self.gestures.push_back(gesture);
    }

    /// Whether the serve loop should take connections: the driver holds
    /// the listener open.
    pub(crate) fn listening(&self) -> bool {
        self.sockets.listening()
    }

    /// A connection the serve loop accepted, for the driver's next accept.
    pub(crate) fn accepted(&mut self, stream: TcpStream) {
        self.sockets.accepted(stream);
    }

    /// The serve loop's accept failed: the driver's next accept reports it
    /// and pauses the listener.
    pub(crate) fn accept_failed(&mut self) {
        self.sockets.accept_failed();
    }

    /// Save the lifetime totals as the writer does: a restart's save
    /// (`restarting`; the simulator's restart has no watchdog to race), of
    /// the kind its uptime gives, otherwise the timed save when due. A
    /// failed save makes the store read-only for the boot, as on the
    /// feeder.
    pub(crate) fn save_stats(&mut self, restarting: bool) {
        let now = Instant::now();
        if restarting {
            let uptime = now.duration_since(self.booted).as_millis();
            self.save_now(SaveKind::restart(u32::try_from(uptime).unwrap_or(u32::MAX)));
        } else if now >= self.stats_due {
            self.save_now(SaveKind::Settled);
            self.stats_due = now + Duration::from_millis(u64::from(SAVE_MS));
        }
    }

    /// One save of `kind`, if the store allows it, with this boot's counts
    /// and erases, its own erase included.
    fn save_now(&mut self, kind: SaveKind) {
        if !self.stats.may_save(kind) {
            return;
        }
        let erases = self.flash.erases();
        let mut boot = [0; TOTAL_SECTORS];
        for ((out, now), before) in boot.iter_mut().zip(erases).zip(self.erases_before) {
            *out = now.saturating_sub(before);
        }
        let own = storage::STATS.first + self.stats.next_slot();
        if let Some(count) = usize::try_from(own).ok().and_then(|own| boot.get_mut(own)) {
            *count = count.saturating_add(1);
        }
        let totals = self.stats_base.plus(&self.counts, &boot, 0);
        self.stats.save(&totals, kind);
    }

    /// Whether the feeder asked for a restart, taken once.
    pub(crate) fn take_reboot(&mut self) -> bool {
        core::mem::take(&mut self.reboot)
    }

    /// Restart once this pass is done, as a watchdog reset would.
    pub(crate) fn request_reboot(&mut self) {
        self.reboot = true;
    }

    /// The ring's record after `after`, read directly, for a test to
    /// inspect without taking a subscriber's turn.
    pub fn ring_after(&self, after: Option<u64>) -> Read {
        self.device.ring_after(after)
    }

    /// The writer's half of an event read, between the loop's passes.
    pub fn answer_read(&mut self) {
        self.device.answer_read();
    }

    /// Hold a queued plans change in the writer's slot, as a slow flash
    /// write would, or let the writer take it.
    pub fn hold_plans(&mut self, held: bool) {
        self.plans_held = held;
    }

    /// Run with the alert rules (`true`, the default) or without, from the
    /// first pass on: a control for the rules changing nothing about a meal.
    pub fn set_alerts_on(&mut self, on: bool) {
        self.alerts_on = on;
    }

    /// The plans stay loading, as a store that never opens.
    pub fn hold_plans_loading(&mut self, loading: bool) {
        self.plans_loading = loading;
    }

    /// The ring answers no read, as a writer that gives it no turn.
    pub fn hold_ring_reads(&mut self, unanswered: bool) {
        self.ring_unanswered = unanswered;
    }

    /// The ring has not opened yet.
    pub fn hold_ring_closed(&mut self, closed: bool) {
        self.ring_closed = closed;
    }

    /// The progress shows `candidate` for the update store's, or the
    /// stage's own with `None`: the simulator's trials confirm at once, so
    /// this is how a test sees one that takes its time.
    pub fn hold_candidate(&mut self, candidate: Option<CandidateState>) {
        self.candidate_held = candidate;
    }

    /// Answers the ring gave a reader outside the session slots, as the boot
    /// read is.
    pub const fn outside_reads(&self) -> u32 {
        self.outside_reads
    }

    /// The writer's turn: apply what is queued, the device's settings
    /// first, then the feeder's stores, then a client change, whose result
    /// it gives for the log.
    pub fn write(&mut self) -> Option<KeyResult> {
        self.device.apply_settings();
        self.save_stats(false);
        if let Some(change) = self.calibration_pending.take() {
            self.calibration.save(&change);
        }
        // A change the writer drops (the loop's, against plans that moved
        // on) writes nothing, as on the feeder.
        if !self.plans_held
            && let Some(change) = self.plans_pending.take()
        {
            self.plans.apply(&change);
        }
        match self.call_pending.take() {
            Some(CallWork::Change(CallChange::Start { bytes, crc })) => {
                self.call.start(bytes, crc);
            }
            Some(CallWork::Change(CallChange::Delete)) => {
                self.call.delete();
            }
            Some(CallWork::Chunk { offset, data }) => {
                self.call.chunk(offset, &data);
            }
            None => {}
        }
        self.device.apply_keys()
    }

    fn calibration_view(&self) -> CalibrationView {
        CalibrationView {
            calibration: self.calibration.calibration,
            generation: u32::try_from(self.calibration.generation).unwrap_or(u32::MAX),
            writable: self.calibration.writable(),
        }
    }

    /// This boot's log tail, which the simulator's records go into.
    pub(crate) fn log_tail(&self) -> Rc<LogTail<LOG_TAIL_BYTES>> {
        Rc::clone(&self.log)
    }
}

/// A factory reset's keep step, as the writer's: rewrite the statistics
/// keeping only the flash erase counts. A save that fails erases the pair
/// instead, losing only the wear counts, as the writer does.
fn keep_wear(flash: &Rc<Flash>) {
    let mut stats = StatsStore::default();
    stats.open(Region::new(flash, storage::STATS));
    if stats.status == StatsStatus::Loaded && stats.writable() {
        let wear = stats.totals.wear_only();
        if stats.totals != wear && !stats.save(&wear, SaveKind::Restart) {
            stores::erase(flash, storage::STATS);
        }
    }
}

/// The meal call's work waiting for the writer's turn.
#[derive(Clone, Debug)]
enum CallWork {
    Change(CallChange),
    Chunk { offset: u32, data: Vec<u8> },
}

impl Platform for SimPlatform {
    type Socket = Socket;
    type Record = Infallible;

    const INDICATOR_SETTLE_MS: u32 = 10;
    const POWER_SAMPLE_MS: u32 = POWER_SAMPLE_MS;

    fn build(&self) -> &'static str {
        self.build
    }

    fn boot(&self) -> Boot {
        Boot {
            bank: self.bank,
            reset_flags: self.reset_flags,
            boot_count: self.boot_count,
        }
    }

    fn model(&self) -> Model {
        MODEL
    }

    fn build_wifi(&self) -> BuildWifi {
        self.build_wifi
    }

    /// The simulator reports a storm (`Banks::boot`) but never holds its
    /// feeding owner closed for one.
    fn held(&self) -> bool {
        false
    }

    fn reset(&mut self) {
        self.reboot = true;
    }

    fn restarting(&self) -> bool {
        self.reboot
    }

    /// The simulator's reset never stops part-way: its boot erases at once.
    fn restart_wanted(&self) -> bool {
        false
    }

    fn random(&mut self, out: &mut [u8; 32]) -> bool {
        match self.device.random() {
            Some(bytes) => {
                *out = bytes;
                true
            }
            None => false,
        }
    }

    fn identity_since(&mut self, seen: u32) -> Option<(u32, [u8; 32])> {
        self.device.identity_since(seen)
    }

    fn gesture(&mut self) -> Option<Gesture> {
        self.gestures.pop_front()
    }

    fn indicator(&mut self, _light: Light, _on: bool) -> bool {
        true
    }

    fn indicator_check(&mut self, _light: Light, _on: bool) -> Check {
        Check::Held
    }

    fn indicators_off(&mut self) {}

    fn power(&mut self, _now_ms: u32) -> Option<proto::Power> {
        Some(proto::Power {
            adapter: self.supply.adapter,
            battery_mv: self.supply.pin_mv.and_then(power::pack_mv),
        })
    }

    fn supply(&mut self, _now_ms: u32) -> Reading {
        supply::classify(Some(self.supply.sample()), &power::PACK)
    }

    /// Off drops every connection and the NTP socket, and takes no new
    /// one until it is on again, as the network leaving does.
    fn radio(&mut self, radio: Radio) {
        self.sockets.radio(radio == Radio::On);
        self.radio = radio;
    }

    fn link(&mut self) -> Link {
        Link {
            rssi: RSSI,
            channel: Some(CHANNEL),
            ..Link::default()
        }
    }

    fn ipv4(&mut self) -> Option<u32> {
        self.sockets.ipv4()
    }

    /// The simulator has no router: an NTP setting names an address.
    fn gateway(&mut self) -> Option<u32> {
        None
    }

    fn time_settings(&mut self) -> TimeSettingsResult {
        self.device.time_settings()
    }

    fn time_settings_change(&mut self, settings: TimeSettings) -> TimeSettingsResult {
        self.device.time_settings_change(settings)
    }

    fn calibration(&mut self) -> CalibrationResult {
        CalibrationResult::Current {
            view: self.calibration_view(),
            pending: self.calibration_pending.is_some(),
        }
    }

    fn plans(&mut self) -> PlansResult {
        if self.plans_loading {
            return PlansResult::Busy;
        }
        PlansResult::Current {
            view: self.plans.view(true),
            pending: self.plans_pending.is_some(),
        }
    }

    fn plans_change(&mut self, change: PlansChange) -> PlansResult {
        let view = self.plans.view(true);
        if !view.writable {
            return PlansResult::ReadOnly;
        }
        if self.plans_pending.is_some() {
            return PlansResult::Busy;
        }
        self.plans_pending = Some(change);
        PlansResult::Current {
            view,
            pending: true,
        }
    }

    fn call(&mut self) -> CallResult {
        CallResult::Current {
            view: self.call.view(true),
            pending: self.call_pending.is_some(),
        }
    }

    fn call_change(&mut self, change: CallChange) -> CallResult {
        let view = self.call.view(true);
        if !view.writable {
            return CallResult::ReadOnly;
        }
        if self.call_pending.is_some() {
            return CallResult::Busy;
        }
        self.call_pending = Some(CallWork::Change(change));
        CallResult::Current {
            view,
            pending: true,
        }
    }

    fn call_chunk(&mut self, offset: u32, data: &[u8]) -> Result<(), proto::Refusal> {
        let view = self.call.view(true);
        if !view.writable {
            return Err(proto::Refusal::NotWritable);
        }
        match chunk_wanted(view.clip, offset, data.len()) {
            Ok(false) => Ok(()),
            Ok(true) if self.call_pending.is_none() => {
                let data = data.to_vec();
                self.call_pending = Some(CallWork::Chunk { offset, data });
                Ok(())
            }
            Ok(true) => Err(proto::Refusal::Busy),
            // The next chunk, sent while the one before is still queued.
            Err(_) if self.call_pending.is_some() => Err(proto::Refusal::Busy),
            Err(refusal) => Err(refusal),
        }
    }

    fn calibration_change(&mut self, change: Calibration) -> CalibrationResult {
        let view = self.calibration_view();
        if !view.writable {
            return CalibrationResult::ReadOnly;
        }
        if self.calibration_pending.is_some() {
            return CalibrationResult::Busy;
        }
        self.calibration_pending = Some(change);
        CalibrationResult::Current {
            view,
            pending: true,
        }
    }

    fn settings(&mut self) -> Option<SettingsView> {
        Some(self.device.settings())
    }

    fn settings_request(&mut self, change: SettingsFields) -> SettingsResult {
        self.device.settings_request(change)
    }

    fn wifi(&mut self) -> WifiResult {
        self.device.wifi()
    }

    fn wifi_change(&mut self, credentials: Option<Credentials>) -> WifiResult {
        self.device.wifi_change(credentials)
    }

    fn wifi_saving(&self) -> bool {
        self.device.wifi_saving()
    }

    /// The feeder reports the source of its last join, which follows a save
    /// only at the next join; there is no join here, so the store stands in
    /// for it.
    fn wifi_source(&self) -> WifiSource {
        self.device.wifi_source()
    }

    fn keys_request(&mut self, change: ClientChange) -> Requested {
        self.device.keys_request(change)
    }

    fn keys_ticket(&self) -> u32 {
        self.device.keys_ticket()
    }

    fn keys_outcome(&self, ticket: u32) -> Option<KeyResult> {
        self.device.keys_outcome(ticket)
    }

    fn clients_since(&mut self, seen: u32, table: &mut [Option<Client>; CLIENTS]) -> Option<u32> {
        self.device.clients_since(seen, table)
    }

    fn queue_event(&mut self, at: Option<u32>, body: [u8; BODY_BYTES], _: EventSlot) -> bool {
        self.device.queue_event(at, &body)
    }

    fn take_lost(&mut self) -> u32 {
        0
    }

    fn event_after(&mut self, reader: Reader, after: Option<u64>) -> Option<Read> {
        if self.ring_unanswered {
            return None;
        }
        let read = self.device.event_after(reader, after);
        if read.is_some() && reader.index() >= wellfed_device_api::SESSIONS {
            self.outside_reads += 1;
        }
        read
    }

    fn reader_left(&mut self, reader: Reader) {
        self.device.reader_left(reader);
    }

    fn epoch(&self) -> Option<u32> {
        (!self.ring_closed).then(|| self.device.epoch())
    }

    fn ring_opened_at(&self) -> Option<u64> {
        self.ring_opened_at
    }

    fn alerts(&self) -> &Alerts {
        &self.alerts
    }

    fn alerts_mut(&mut self) -> &mut Alerts {
        &mut self.alerts
    }

    fn alerts_on(&self) -> bool {
        self.alerts_on
    }

    fn last_event(&self) -> Option<u64> {
        self.device.last_event()
    }

    fn log_after(&self, after: Option<u32>, text: &mut [u8]) -> Option<LogBatch> {
        self.log.read(after, text)
    }

    /// The simulated uptime never stops.
    fn clock_ms(&mut self, now_ms: u32) -> u32 {
        now_ms
    }

    fn wall_ms(&mut self, now_ms: u32) -> Option<u64> {
        self.device
            .clock()
            .read(now_ms)
            .map(|reading| reading.unix_ms)
    }

    fn time_sync(&mut self, now_ms: u32, sync: Sync, origin: Origin) -> Result<Jump, SyncError> {
        self.device.clock().sync(now_ms, sync, origin)
    }

    fn time_fix(&mut self, now_ms: u32, fix: Fix) -> Result<Jump, SyncError> {
        self.device.clock().fix(now_ms, fix)
    }

    fn clock_state(&mut self, now_ms: u32) -> ClockState {
        self.device.clock().state(now_ms)
    }

    fn offset_at(&self, unix_ms: u64) -> Option<i32> {
        self.device.wall().offset_at(unix_ms)
    }

    fn set_zone_rule(&mut self, rule: Option<Rule>) {
        self.device.clock().set_rule(rule);
    }

    fn outputs_claimed(&self) -> bool {
        INTERLOCK.outputs_claimed()
    }

    fn claim_in_bound(&self, now_ms: u32) -> bool {
        INTERLOCK.claim_in_bound(now_ms)
    }

    // As the firmware's: the update stage behind the mailbox.
    fn update_offer(
        &mut self,
        manifest: &[u8],
        downgrade: Option<&[u8]>,
        device: [u8; 32],
    ) -> Result<u32, Refused> {
        Request::offer(manifest, downgrade, device).and_then(mailbox::request)
    }

    fn update_statement(&mut self, statement: &[u8]) -> Result<u32, Refused> {
        Request::statement(statement).and_then(mailbox::request)
    }

    fn update_cancel(&mut self) -> Result<u32, Refused> {
        mailbox::request(Request::Cancel)
    }

    fn update_outcome(&mut self, ticket: u32) -> Option<Result<Answer, Refused>> {
        mailbox::outcome(ticket)
    }

    fn update_chunk(&mut self, offset: u32, data: &[u8]) -> Result<(), Refused> {
        mailbox::chunk(offset, data)
    }

    fn update_progress(&mut self) -> Option<Progress> {
        let mut progress = mailbox::progress()?;
        if let Some(candidate) = self.candidate_held {
            progress.candidate = candidate;
        }
        Some(progress)
    }

    fn installing(&self) -> bool {
        mailbox::installing()
    }

    fn bank1_read(&mut self, offset: u32, out: &mut [u8]) -> bool {
        let Some(banks) = self.banks.as_ref().filter(|_| self.bank == 0) else {
            return false;
        };
        let start = offset as usize;
        let banks = banks.borrow();
        let Some(bytes) = banks.bank1().get(start..start + out.len()) else {
            return false;
        };
        out.copy_from_slice(bytes);
        true
    }

    fn listen(&mut self, _: u16) -> Option<Socket> {
        // The serve loop bound its listener before the first boot.
        Some(self.sockets.listen())
    }

    fn accept(&mut self, _: Socket) -> Accepted<Socket> {
        self.sockets.accept()
    }

    fn receive(&mut self, socket: Socket, into: &mut [u8]) -> Received {
        self.sockets.receive(socket, into)
    }

    fn send(&mut self, socket: Socket, bytes: &[u8]) -> Option<usize> {
        self.sockets.send(socket, bytes)
    }

    fn close(&mut self, socket: Socket) {
        self.sockets.close(socket);
    }

    fn mdns_open(&mut self) -> Option<Socket> {
        Some(Socket::Mdns)
    }

    /// A failed send is only a lost answer: the asker asks again.
    fn mdns_send(&mut self, _: Socket, bytes: &[u8], ip: u32, port: u16) -> bool {
        self.sockets.mdns_send(bytes, ip, port)
    }

    fn mdns_receive(&mut self, _socket: Socket, into: &mut [u8]) -> Datagram {
        self.sockets.mdns_receive(into)
    }

    fn ntp_open(&mut self) -> Option<Socket> {
        self.sockets.ntp_open()
    }

    /// A failed send is only a lost request: the exchange asks again.
    fn ntp_send(&mut self, _socket: Socket, bytes: &[u8], ip: u32, port: u16) -> bool {
        self.sockets.ntp_send(bytes, ip, port)
    }

    fn ntp_receive(&mut self, _socket: Socket, into: &mut [u8]) -> Datagram {
        self.sockets.ntp_receive(into)
    }

    fn heap(&self) -> (u32, u32) {
        (0, 0)
    }

    /// The simulator runs no RTOS tasks.
    fn task_stacks(&mut self, _out: &mut [TaskStack; TASKS_MAX]) -> TaskCount {
        TaskCount::default()
    }

    fn erases(&self) -> proto::Erases {
        let now = self.flash.erases();
        let mut storage = [0; FLASH_SECTORS];
        for ((out, now), before) in storage.iter_mut().zip(now).zip(self.erases_before) {
            *out = now.saturating_sub(before);
        }
        proto::Erases { storage, update: 0 }
    }

    fn events_fill(&self) -> EventsFill {
        self.device.events_fill()
    }

    fn traces_fill(&self) -> TracesFill {
        let mut ring = TraceRing::default();
        ring.open(self.traces(), storage::TRACES.count as usize);
        TracesFill {
            kept: ring.kept(),
            capacity: ring.capacity(),
            unsaved: 0,
        }
    }

    fn trace_page(
        &mut self,
        feed: u32,
        offset: u16,
        out: &mut [u8; PAGE_MAX],
    ) -> Result<TraceRead, proto::Refusal> {
        let units = storage::TRACES.count as usize;
        traces::page(&self.traces(), units, feed, offset, out)
    }

    fn totals(&self) -> Option<Totals> {
        self.stats.writable().then_some(self.stats_base)
    }

    fn hand_counts(&mut self, counts: Counts) {
        self.counts = counts;
    }

    fn ble_restart(&mut self) {
        self.ble = Ble::default();
    }

    /// A connection or a leave first, then the phone's next message once
    /// the loop has consumed the one it holds, as the driver reports them.
    fn ble_step(&mut self, advertise: bool) -> BleStep<Infallible> {
        BleStep {
            delay_ms: NO_RADIO_MS,
            record: None,
            link: self.ble.step(advertise),
        }
    }

    fn ble_message(&self) -> Option<&[u8]> {
        self.ble.message()
    }

    fn ble_consume(&mut self) {
        self.ble.consume();
    }

    fn ble_send(&mut self, body: &[u8]) -> bool {
        self.ble.send(body)
    }

    fn ble_close(&mut self, _: BleDrop) -> Option<Infallible> {
        self.ble.close();
        None
    }

    fn speaker_restart(&mut self) {}

    fn speaker_idle(&self) -> bool {
        true
    }

    fn play(&mut self, _tune: Tune) -> PlayResult {
        // No speaker: as if `sound` were off.
        PlayResult::Muted
    }

    fn play_call(&mut self, _plays: u8) -> PlayResult {
        PlayResult::Muted
    }

    fn speaker_step(&mut self) -> DriverStep<Infallible> {
        DriverStep {
            delay_ms: NO_RADIO_MS,
            record: None,
        }
    }

    /// The host has no buzzer; a beep is silent.
    fn buzzer_enable(&mut self) {}

    fn buzzer(&mut self, _on: bool) {}
}
