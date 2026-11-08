//! What the server needs from the rest of the device, and the values it
//! passes across: an update request for the installer, a client store
//! change, an event read from the ring.
//!
//! [`Device`] is what every kind provides for the shared core: identity,
//! sessions, Wi-Fi, time, events, updates, the log and statistics. Each
//! kind's module adds its own trait on top ([`FeederDevice`]), which the
//! server reaches only through [`Device::feeder`] (a [`NoFeeder`] for
//! another kind), for that module's
//! requests: the server itself is generic over [`Device`] alone, so a
//! device of another kind serves the shared core unchanged.

use crate::clients::{Label, Role};
use crate::mailbox::{
    Asked, Calibration, CalibrationResult, CallChange, CallPlay, CallResult, LockResult, MealSkip,
    Origin, PlansResult, PlayResult, SettingsFields, SettingsResult, Table, TimeFields, TimeResult,
    TimeSettings, TimeSettingsResult, TimeState, WifiResult,
};
use crate::wifi::Credentials;
use wellfed_core::log_tail::Batch as LogBatch;
use wellfed_core::named_enum;
use wellfed_core::ota::offer::KeyId;
use wellfed_core::tone::Tune as CoreTune;
use wellfed_proto::api::{
    self as proto, AlertsView, Bank1Page, Blocked, Bowl, EventBody, FrozenRefusal, Phase,
    Readiness, Refusal, Source, Stats, StatsSection, Status, TracePage, UpdateView,
};
use wellfed_proto::device::Model;

/// The rest of the device, as the server's shared core needs it. Every call
/// returns at once: a write is queued for its owner, and the server polls
/// for the result on later passes.
pub trait Device {
    /// Fill `out` from the vetted generator; false when it has no output.
    fn random(&mut self, out: &mut [u8; 32]) -> bool;
    /// The build identity, for the welcome and status.
    fn build(&self) -> &'static str;
    /// The device's model, for the welcome; its kind follows from it.
    fn model(&self) -> Model;
    /// The API revision, for the welcome. Every build sends its own; a
    /// simulator may play an older or a newer one.
    fn revision(&self) -> u32 {
        wellfed_proto::api::REVISION
    }
    /// The feeder's module of this device: [`NoFeeder`] for another kind,
    /// whose [`Self::feeder`] is always `None`. Generic rather than `dyn`,
    /// so the module's calls stay direct, as the stack audit requires.
    type Feeder: FeederDevice;
    /// The feeder's module, `None` for another kind: its requests are
    /// then refused `Unavailable`.
    fn feeder(&mut self) -> Option<&mut Self::Feeder>;
    /// Everything [`Status`] reports, the kind's part included, but the
    /// session count.
    fn status(&mut self, now_ms: u32) -> Status<'static>;
    /// The Wi-Fi store; with `Some`, one change (`None` credentials clear)
    /// queued, whose `Wifi` event the writer records as it saves it.
    fn wifi(&mut self, change: Option<Asked<Option<Credentials>>>) -> WifiResult;
    /// A client's sync; `origin` says whether it may step the clock.
    fn time_sync(&mut self, fields: TimeFields, origin: Origin) -> TimeResult;
    /// Queue a client store change; its ticket, or `None` while the mailbox
    /// is busy.
    fn keys(&mut self, change: KeyChange) -> Option<u32>;
    /// The result of the change with `ticket`, once written.
    fn keys_outcome(&mut self, ticket: u32) -> Option<KeyOutcome>;
    /// Record one durable event.
    fn record(&mut self, now_ms: u32, body: EventBody);
    /// The first durable event after `after` (`None`: from the oldest),
    /// for the subscriber in session slot `reader`. A device that reads
    /// in turns serves each reader its turn by the slot.
    fn event_after(&mut self, reader: Reader, after: Option<u64>) -> EventRead;
    /// The subscriber in slot `reader` ended: a read it waits on, or one
    /// fetched for it, is nobody's now.
    fn reader_left(&mut self, _reader: Reader) {}
    /// The event ring's epoch, `None` until the ring is open. A session is
    /// welcomed only once it is known.
    fn epoch(&self) -> Option<u32>;
    /// Restart the device.
    fn reboot(&mut self);
    /// One section of the statistics, read when its answer is sealed.
    fn stats(&mut self, section: StatsSection, now_ms: u32) -> Stats<'_>;
    /// One page of bank 1 from `offset`, read when its answer is sealed:
    /// served only by the bank-0 recovery role (`Unavailable` elsewhere),
    /// `Busy` while an install holds the flash, `Invalid` at or past the
    /// bank's end. Asked only for an owner.
    fn bank1_page(&mut self, _offset: u32) -> Result<Bank1Page<'_>, Refusal> {
        Err(Refusal::Unavailable)
    }
    /// The table of open alerts, some any device's and some its kind's. A
    /// device that keeps none answers `None`, refused `Unavailable`.
    fn alerts(&mut self) -> Option<AlertsView> {
        None
    }

    /// Queue an update request for the installer: its ticket, or why it
    /// cannot be queued now. A device without an installer serves none.
    ///
    /// The update path is frozen, so its refusals are [`FrozenRefusal`]: a
    /// client of any later build reads every one.
    fn update(&mut self, _request: UpdateRequest<'_>) -> Result<u32, FrozenRefusal> {
        Err(FrozenRefusal::Unavailable)
    }
    /// The result of the update request with `ticket`, once done.
    fn update_outcome(&mut self, _ticket: u32) -> Option<Result<UpdateAnswer, FrozenRefusal>> {
        None
    }
    /// Hand one chunk to the install: the state once it is taken (or was
    /// not wanted), `Busy` while the last ones are still being written.
    fn update_chunk(&mut self, _offset: u32, _data: &[u8]) -> Result<UpdateView, FrozenRefusal> {
        Err(FrozenRefusal::Unavailable)
    }
    /// The update's state now.
    fn update_view(&mut self) -> Option<UpdateView> {
        None
    }
    /// An install holds the flash (or is about to restart): no kind's
    /// operation (a feed) starts.
    fn installing(&self) -> bool {
        false
    }
    /// The kind runs an operation a restart or an install would cut (a
    /// feed is queued or runs): no install starts, and no reboot.
    fn operating(&self) -> bool {
        false
    }

    /// The time settings; with `Some`, the whole new settings queued for
    /// the writer. A device without a clock store keeps none.
    fn time_settings(&mut self, _change: Option<TimeSettings>) -> TimeSettingsResult {
        TimeSettingsResult::Unserved
    }
    /// The clock and the NTP exchange now.
    fn time_state(&mut self, _now_ms: u32) -> Option<TimeState> {
        None
    }
    /// The logged lines after line `after` (`None`: from the oldest kept)
    /// that fit `text` whole, each ending in `\n` and printable ASCII.
    /// `None` when there is nothing new. A device without a RAM tail keeps
    /// none.
    fn log_after(&mut self, _after: Option<u32>, _text: &mut [u8]) -> Option<LogBatch> {
        None
    }
}

/// The rest of a feeder, as the feeder's module of the device API needs
/// it ([`proto::FeederOp`]).
pub trait FeederDevice: Device {
    /// The settings; with `Some`, one change queued for the writer, which
    /// records its `Settings` event as it saves it.
    fn settings(&mut self, change: Option<Asked<SettingsFields>>) -> SettingsResult;
    fn lock(&mut self, locked: bool) -> LockResult;
    /// Queue a tune for the speaker, or say why it will not sound now.
    /// `Started` means queued, not heard: a codec that then will not open
    /// reaches only the log.
    fn play(&mut self, tune: CoreTune) -> PlayResult;
    /// Whether a feed can start now, and why not: the one decision both
    /// the status reports and [`Self::feed`] is asked under. A device with
    /// no feeding owner is [`Blocked::Unserved`].
    fn readiness(&mut self) -> Readiness {
        Readiness::Blocked(Blocked::Unserved)
    }
    /// Queue one feed for the feeding owner on `by`'s behalf: the number
    /// its journal will hold it under. Asked only while
    /// [`Self::readiness`] is ready; the owner still refuses one that lost
    /// a race (a join that began since).
    fn feed(&mut self, portions: u8, by: Source) -> Result<u32, Refusal>;
    /// Whether the feed `feed` queued is durably reserved yet: `None` while
    /// it waits, or why it ended before it was.
    fn feed_reserved(&mut self, feed: u32) -> Option<Result<(), Refusal>>;
    /// Stop the feed `feed` if it is still running; false once it has ended.
    fn cancel_feed(&mut self, feed: u32) -> bool;
    /// One page of feed `feed`'s trace from byte `offset` of its body, read
    /// when its answer is sealed: `NotFound` when no trace of it is kept,
    /// `Busy` while it is still recorded, `Invalid` for an offset past the
    /// body.
    fn feed_trace(&mut self, feed: u32, offset: u16) -> Result<TracePage<'_>, Refusal>;
    /// The portion calibration; with `Some`, the whole new calibration
    /// queued for the writer. A feeder without a calibration store keeps
    /// none.
    fn calibration(&mut self, _change: Option<Calibration>) -> CalibrationResult {
        CalibrationResult::Unserved
    }
    /// The feeding plans; with `Some`, a client's whole new table queued
    /// for the writer, which records its `Plans` event as it saves it. A
    /// feeder without a plans store keeps none.
    fn plans(&mut self, _change: Option<Asked<Table>>) -> PlansResult {
        PlansResult::Unserved
    }
    /// Queue a client's skip of slot `slot`'s meal due at `due_s` (Unix
    /// seconds), or its undo, for the plans' writer, after checking it is
    /// that plan's meal still to come today (`Invalid`), with a valid clock
    /// and offset (`NoClock`). The writer records the `Skipped` or
    /// `Unskipped` event, naming `by`, as it saves it. A feeder without a
    /// plans store keeps none.
    fn skip_meal(
        &mut self,
        _slot: u8,
        _due_s: u32,
        _skip: Asked<MealSkip>,
        _now_ms: u32,
    ) -> Result<(), Refusal> {
        Err(Refusal::Unavailable)
    }
    /// The meal call; with `Some`, an upload's start or a delete queued
    /// for the writer. A feeder without a call store keeps none.
    fn call(&mut self, _change: Option<CallChange>) -> CallResult {
        CallResult::Unserved
    }
    /// Queue one chunk of the upload under way, at `offset`: `Busy` while
    /// the last one is still being written, `Invalid` for an offset other
    /// than where the upload stands or a wrong length. A chunk already
    /// received is taken as done.
    fn call_chunk(&mut self, _offset: u32, _data: &[u8]) -> Result<(), Refusal> {
        Err(Refusal::Unavailable)
    }
    /// Queue the stored meal call to play once, so its owner can hear it.
    fn play_call(&mut self) -> CallPlay {
        CallPlay::Unserved
    }
    /// Today's local calendar day (days since 1970-01-01, UTC when the
    /// clock knows no offset) while the clock is valid: the day a desiccant
    /// replacement is recorded under. `None` refuses the reset.
    fn today(&mut self, _now_ms: u32) -> Option<i32> {
        None
    }
    /// The bowl mode this build runs: the only one a calibration may name,
    /// since a build runs only the mode it was verified for.
    fn bowl(&self) -> Bowl {
        Bowl::Single
    }
    /// Where the rotor rests, once the feeding owner knows; `None` from a
    /// device with no owner.
    fn phase(&self) -> Option<Phase> {
        None
    }
}

/// The feeder module of a device of another kind: uninhabited, so
/// [`Device::feeder`] can only answer `None`, and no feeder request reaches
/// anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoFeeder {}

impl Device for NoFeeder {
    type Feeder = Self;

    fn feeder(&mut self) -> Option<&mut Self> {
        match *self {}
    }
    fn random(&mut self, _out: &mut [u8; 32]) -> bool {
        match *self {}
    }
    fn build(&self) -> &'static str {
        match *self {}
    }
    fn model(&self) -> Model {
        match *self {}
    }
    fn status(&mut self, _now_ms: u32) -> Status<'static> {
        match *self {}
    }
    fn wifi(&mut self, _change: Option<Asked<Option<Credentials>>>) -> WifiResult {
        match *self {}
    }
    fn time_sync(&mut self, _fields: TimeFields, _origin: Origin) -> TimeResult {
        match *self {}
    }
    fn keys(&mut self, _change: KeyChange) -> Option<u32> {
        match *self {}
    }
    fn keys_outcome(&mut self, _ticket: u32) -> Option<KeyOutcome> {
        match *self {}
    }
    fn record(&mut self, _now_ms: u32, _body: EventBody) {
        match *self {}
    }
    fn event_after(&mut self, _reader: Reader, _after: Option<u64>) -> EventRead {
        match *self {}
    }
    fn epoch(&self) -> Option<u32> {
        match *self {}
    }
    fn reboot(&mut self) {
        match *self {}
    }
    fn stats(&mut self, _section: StatsSection, _now_ms: u32) -> Stats<'_> {
        match *self {}
    }
}

impl FeederDevice for NoFeeder {
    fn settings(&mut self, _change: Option<Asked<SettingsFields>>) -> SettingsResult {
        match *self {}
    }
    fn lock(&mut self, _locked: bool) -> LockResult {
        match *self {}
    }
    fn play(&mut self, _tune: CoreTune) -> PlayResult {
        match *self {}
    }
    fn feed(&mut self, _portions: u8, _by: Source) -> Result<u32, Refusal> {
        match *self {}
    }
    fn feed_reserved(&mut self, _feed: u32) -> Option<Result<(), Refusal>> {
        match *self {}
    }
    fn cancel_feed(&mut self, _feed: u32) -> bool {
        match *self {}
    }
    fn feed_trace(&mut self, _feed: u32, _offset: u16) -> Result<TracePage<'_>, Refusal> {
        match *self {}
    }
}

/// An update op, as the device's installer takes it. The bytes are copied
/// into its mailbox before `Device::update` returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateRequest<'a> {
    Offer {
        manifest: &'a [u8],
        downgrade: Option<&'a [u8]>,
        /// The feeder's identity key, which a downgrade statement names.
        device: [u8; 32],
    },
    Statement {
        statement: &'a [u8],
    },
    Cancel,
}

named_enum! {
    /// What became of an update key.
    pub enum UpdateKeyChange {
        /// An owner enrolled it, confirmed by a Dispense press.
        Enrolled = "enrolled",
        /// A key statement filled the release slot with it.
        Added = "added",
        /// A key statement retired it for good.
        Retired = "retired",
    }
}

/// What an update request came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateAnswer {
    View(UpdateView),
    /// A key statement took effect on the key with this id.
    Key {
        change: UpdateKeyChange,
        id: KeyId,
    },
}

/// A client store change the device API asks for. Each names what its
/// history event needs, since the settings writer, not the asking session,
/// records it ([`KeyChange::event`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyChange {
    Enroll {
        key: [u8; 32],
        label: Label,
        role: Role,
        /// How the key joined.
        path: proto::Path,
    },
    Revoke {
        key: [u8; 32],
        /// The client that asked.
        by: Source,
    },
}

impl KeyChange {
    /// The history event this change records once the writer has made it
    /// (`Done`). A device records it in the same turn as the store write,
    /// whether or not the asking session is still there to read the
    /// outcome; a change that was refused, or changed nothing (a key
    /// already enrolled), records none.
    pub fn event(&self) -> EventBody {
        match *self {
            Self::Enroll {
                key, role, path, ..
            } => EventBody::Enrolled {
                client: crate::fingerprint(&key),
                role: crate::proto_role(role),
                path,
            },
            Self::Revoke { key, by } => EventBody::Revoked {
                client: crate::fingerprint(&key),
                by,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyOutcome {
    Done,
    Full,
    Duplicate,
    NotFound,
    NotWritable,
}

/// The session slot a subscriber reads events from, below
/// [`SESSIONS`](crate::SESSIONS).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reader(usize);

impl Reader {
    /// The subscriber in slot `index`.
    pub const fn slot(index: usize) -> Self {
        Self(index)
    }

    pub const fn index(self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventRead {
    /// The event as the ring keeps it: a body this build cannot read (a
    /// newer build's, met by a bank 0 that kept an older one) goes to the
    /// client as it is, which reads it or skips it by its length.
    Event(proto::RawEvent),
    /// Events `from..=to` are gone from the ring.
    Gap { from: u64, to: u64 },
    /// Nothing newer yet.
    None,
    /// Ask again on a later pass (the writer is reading flash).
    Wait,
}
