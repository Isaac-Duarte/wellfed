//! Device API: one Noise session per client over TCP.
//!
//! A connection starts with one clear [`Open`] frame naming the session:
//! - [`Open::Session`]: an enrolled client, Noise IK under [`PROLOGUE`]. The
//!   client knows the device's identity key and sends its own encrypted in
//!   the first message, carrying a [`Hello`]; the device answers with a
//!   [`Welcome`] naming its kind and model (or refuses an unknown key, an
//!   old major version or a full table) in the second.
//! - [`Open::Invite`]: a new client redeems an invite, Noise XXpsk3 with the
//!   invite's secret as the PSK. The third message carries a [`Join`] and
//!   the device's first transport message a [`Joined`]; the client then
//!   reconnects with [`Open::Session`].
//! - [`Open::Recovery`]: the button-hold window. The connection runs BLE setup's
//!   setup exchange ([`crate::setup`]) unchanged, code proof included, so a
//!   LAN neighbor cannot slip in while the window is open.
//!
//! Every frame is a u16 little-endian length and that many bytes
//! (`wellfed_session::frame`), at most [`FRAME_MAX`]. After the handshake each frame
//! is one Noise transport message sealing one [`ClientMessage`] or
//! [`DeviceMessage`]. The device sends a session one frame per loop pass, so
//! what a client reads in bulk fills a frame: events in an [`EventBatch`],
//! log lines in a [`DeviceMessage::Log`].
//!
//! The ops are a shared core every kind serves (status, Wi-Fi, clients,
//! time, updates, the log, statistics) and one typed module per
//! [`DeviceKind`](crate::device::DeviceKind): [`Op::Feeder`] carries a [`FeederOp`], answered with an
//! [`Outcome::Feeder`]. A device refuses another kind's module
//! `Unavailable`.
//!
//! Evolution ([device API](../../../../docs/firmware/device-api.md#evolution)):
//! - [`PROLOGUE`] never changes. A new major is negotiated in
//!   [`Hello::major`], and a device refuses one it does not serve with
//!   [`OpenRefusal::Version`].
//! - What a session serves follows from the welcome's kind, model and
//!   major alone: there are no feature bits. An op a build does not serve,
//!   or cannot decode, is refused `Unavailable`.
//! - **The update path is frozen.** Every update passes through bank 0,
//!   which keeps the build it was written with, so a client of any later
//!   build must still update it. The connection's opening ([`Open`],
//!   [`Hello`], [`Welcome`]), the frames, discovery, the update ops
//!   ([`Op::Update`]), the keepalive ([`Op::Alive`]), invites
//!   ([`Op::CreateInvite`], [`Join`], [`Joined`]), the recovery window's
//!   setup exchange ([`crate::setup`]) and every answer they get never
//!   change: no field is added, removed, reordered or retyped,
//!   and an enum only gains variants at its end. The frozen variants of
//!   [`ClientMessage`], [`DeviceMessage`], [`Op`], [`Outcome`] and
//!   [`Refusal`] come first, so the rest can change behind them. Only a
//!   new major changes the frozen part, and a device keeps serving major
//!   1's for good. So does every client, even once a new major exists: a
//!   feeder never updated since speaks nothing else.
//!   `tests/fixtures/update-v1.txt` pins it, and its test only appends.
//! - Everything else is positional too, but changes in place while the
//!   research unit is the only device running Wellfed: the firmware and
//!   every client are built from this repository together. The change
//!   rewrites the message's lines in `tests/fixtures/v1.txt` by hand
//!   (blessing only appends). Any change to a type changes [`SCHEMA`],
//!   which the welcome carries: a client of another revision sends that
//!   build only frozen ops.

use core::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use crate::device::Model;
pub use crate::setup::Wifi;

/// Frozen. Bound into the IK and invite handshakes, so a session meant
/// for another protocol or version fails at its first encrypted message.
pub const PROLOGUE: &[u8] = b"wellfed-api-v1";

/// The major this build speaks. Major 1's update path is frozen: a new
/// major is the only way to change it, and a device keeps serving major 1's
/// update path for good.
pub const PROTOCOL_MAJOR: u16 = 1;
/// The oldest major this device still serves.
pub const PROTOCOL_MAJOR_MIN: u16 = 1;

/// The revision of everything outside the frozen part: a hash of every
/// message type's structure, traced variant by variant and field by field
/// (`tests/schema.rs`), so any change to a type, fixture or no fixture,
/// makes a new revision. The test fails, naming the new value, until this
/// is updated. The welcome carries it ([`Welcome::Open::schema`]), and a
/// client of another revision sends that build only frozen ops.
pub const SCHEMA: u32 = 0xf6ab_21a3;

/// Frozen. The largest frame either way, Noise tag included.
pub const FRAME_MAX: usize = 1024;
/// The largest message a frame can seal.
pub const MESSAGE_MAX: usize = FRAME_MAX - 16;

/// Frozen. The mDNS service the device advertises, with TXT `v=1` and
/// `id=` its identity key in hex.
pub const SERVICE: &str = "_wellfed._tcp";
/// Frozen. The TCP port the device listens on.
pub const PORT: u16 = 4747;

/// A client-chosen request id: random, so a retry after a reconnect is
/// still recognized.
pub type RequestId = [u8; 16];

/// Frozen, with every variant: the first frame on a connection, in clear.
/// A session, an invite's redemption and the recovery window all reach a
/// feeder of any build.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Open {
    Session,
    Invite { id: u32 },
    Recovery,
}

/// Frozen. The client's IK first-message payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub major: u16,
}

/// Frozen, with everything it carries. The device's IK second-message
/// payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Welcome<'a> {
    Open {
        major: u16,
        /// The device's model: its board, the images it installs, and its
        /// kind ([`Model::kind`]), which names the module of ops it serves
        /// beside the shared core.
        model: Model,
        /// The running image's build identity.
        build: &'a str,
        /// The role this key holds.
        role: Role,
        /// The event history's epoch: drawn at random when the ring is
        /// created and kept with it. Event numbers count within one epoch,
        /// so a cursor from another epoch means nothing here: the client
        /// starts again from the oldest event kept.
        epoch: u32,
        /// The build's [`SCHEMA`]: which revision of everything outside the
        /// frozen update path it speaks. A client of another revision sends
        /// only frozen ops on this session, since its other ops may decode
        /// here as something else.
        schema: u32,
    },
    Refused(OpenRefusal),
}

/// Frozen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpenRefusal {
    /// The key is not enrolled (or was revoked).
    Unknown,
    /// The client's major version is one this device no longer serves, or
    /// does not know yet; `supported` is the newest it serves.
    Version { supported: u16 },
    /// Every session slot is taken.
    Full,
}

/// Frozen, since the welcome carries it. A client's role, as the client
/// store keeps it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Owner,
    Member,
    Server,
    UpdateSigner,
}

/// Frozen. An invite redemption's third-message payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Join<'a> {
    /// A name for the client, shown when the owner lists clients (1-16
    /// bytes).
    pub label: &'a str,
}

/// Frozen, with what it carries. The device's answer to a [`Join`],
/// sealed; the connection then closes.
/// An enrollment names the device's model, as a session's welcome does, so
/// a client that redeems an invite knows what it joined without opening one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Joined {
    Enrolled { role: Role, model: Model },
    Refused(Refusal),
}

/// What a client sends once the session is open. [`Self::Request`] is
/// frozen at index 0, so a request's id is always readable: a device
/// answers an op it cannot decode `Unavailable` rather than end the
/// session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientMessage<'a> {
    Request {
        id: RequestId,
        #[serde(borrow)]
        op: Op<'a>,
    },
    /// Push every durable event after `after` (`None` for the oldest kept),
    /// then each new one as it happens, in [`DeviceMessage::Events`]
    /// batches, and the live samples if `samples` is `On`.
    Subscribe {
        after: Option<u64>,
        samples: Samples,
    },
}

/// Whether a subscriber is sent the live [`Metric`]s, every kind of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Samples {
    Off,
    On,
}

/// One operation. Reads may be repeated freely; a write's id makes a retry
/// return the first answer instead of running again.
///
/// The first three variants are frozen ([`Self::is_frozen`]): the update
/// ops, the keepalive and the invite, which every build of major 1 serves
/// at these indices for good. The rest is the shared core, then one variant per device
/// kind's module, and changes freely.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op<'a> {
    /// Frozen at index 0: an update, and the keys that sign one.
    #[serde(borrow)]
    Update(UpdateOp<'a>),
    /// Frozen at index 1: the keepalive, answered [`Outcome::Done`] to
    /// every session (an update signer's key never opens one). Its answer
    /// never changes, so a client keeps a session alive even with a build
    /// whose other answers it cannot read.
    Alive,
    /// Frozen at index 2: a single-use invite for a new client, held in RAM
    /// for 10 minutes, answered [`Outcome::Invite`]. Frozen so a new phone
    /// can still join a feeder of an older build, and then update it.
    CreateInvite {
        role: Role,
    },
    /// Answered [`Outcome::Status`]: the shared status and the kind's.
    Status,
    /// Stores credentials for the next join, or clears them with `None`.
    /// Applies only if the Wi-Fi store is still at `base` generation.
    PutWifi {
        base: u32,
        #[serde(borrow)]
        wifi: Option<Wifi<'a>>,
    },
    TimeSync(TimeSync),
    ListClients,
    RevokeClient {
        key: [u8; 32],
    },
    /// Refused `Busy` while a kind's operation that a restart would cut
    /// (a feed) is in flight.
    Reboot,
    /// The time settings and the clock's state.
    GetTime,
    /// Applies only if the time settings are still at `base` generation.
    /// Owner sessions only: the NTP server decides which fixes the clock
    /// hears.
    PutTime {
        base: u32,
        #[serde(borrow)]
        changes: TimeChanges<'a>,
    },
    /// Push the device's record lines after line `after` (`None`: from
    /// the oldest its RAM tail keeps), then each new one, as
    /// [`DeviceMessage::Log`] batches. Answered `Done`; asking again moves
    /// the cursor. Lines count from 1 each boot, so a cursor past the
    /// newest line (an earlier boot's) starts from the oldest. Owner
    /// sessions only. The lines are diagnostics, not a contract: their
    /// text may change with any build.
    TailLog {
        after: Option<u32>,
    },
    /// One section of the device's statistics, answered
    /// [`Outcome::Stats`]. Each section fits one frame on its own.
    Stats {
        section: StatsSection,
    },
    /// Bank 1's bytes from `offset`, answered [`Outcome::Bank1`] with at
    /// most [`BANK1_PAGE_MAX`] bytes, so a client asks again from where
    /// each page ends until it has the whole bank. Read-only and owner
    /// sessions only. Served only from bank 0 (`Unavailable` from bank 1,
    /// whose own image is the running build), `Busy` while an install
    /// holds the flash, `Invalid` for an offset at or past the bank's end.
    /// The bytes are as the flash holds them: after stock installed a
    /// build into bank 0, stock with its marker's first byte zeroed.
    ReadBank1 {
        offset: u32,
    },
    /// The feeder's module; refused `Unavailable` by another kind.
    #[serde(borrow)]
    Feeder(FeederOp<'a>),
}

/// The update ops ([`Op::Update`]), frozen with everything they carry and
/// every answer they get: no variant or field is removed, reordered or
/// retyped, and a new op goes at the end. An update always goes through
/// bank 0, which keeps the build it was written with, so a client of any
/// later build must still speak these to it. Owner sessions only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateOp<'a> {
    /// Offer a signed version 2 manifest, with the downgrade statement it
    /// needs when its version is not above the floor (always, for stock).
    /// Bank 1 checks it, records it and gives way to bank 0
    /// ([`UpdatePhase::GivingWay`]): the client reconnects and offers it
    /// again, and bank 0, which installs only that manifest, starts
    /// receiving. Refused `Busy` while a feed runs.
    Offer {
        #[serde(borrow)]
        manifest: &'a [u8],
        #[serde(borrow)]
        downgrade: Option<&'a [u8]>,
    },
    /// Image bytes at `offset`, one of the offsets [`UpdateView::wanted`]
    /// names, at most [`UPDATE_CHUNK_MAX`] bytes. Idempotent by offset: a
    /// chunk already written is answered with the state and changes
    /// nothing, so a retry needs no remembered id.
    Chunk {
        offset: u32,
        #[serde(borrow)]
        data: &'a [u8],
    },
    /// The update's state, for a client that reconnected.
    State,
    /// Stop the install with bank 1 left invalid, and forget the
    /// candidate.
    Cancel,
    /// Enroll an owner's Ed25519 update-signing key. Answered once a
    /// press at the device confirms it, or refused `Update(Unconfirmed)`
    /// when nobody presses within the window.
    EnrollKey {
        key: [u8; 32],
        #[serde(borrow)]
        label: &'a str,
    },
    /// A key statement a built-in or the release key signed: fill the
    /// release slot, or retire a key id for good.
    KeyStatement {
        #[serde(borrow)]
        statement: &'a [u8],
    },
}

/// The feeder's ops ([`DeviceKind::Feeder`](crate::device::DeviceKind::Feeder)).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeederOp<'a> {
    GetSettings,
    /// Applies only if the settings are still at `base` generation.
    PutSettings {
        base: u32,
        changes: SettingsChanges,
    },
    SetLock {
        locked: bool,
    },
    Play {
        tune: Tune,
    },
    /// Answered [`FeederOutcome::Feeding`] once the feed is durably
    /// reserved; its result is a [`EventBody::Fed`] in the history, with
    /// the portions asked for and those delivered. `portions` outside 1 to
    /// [`PORTIONS_MAX`] is refused [`Refusal::Portions`] before anything
    /// else is checked.
    Feed {
        portions: u8,
    },
    /// Stop the feed [`FeederOutcome::Feeding`] named, if it is still
    /// running. `NotFound` once it has ended (its `Fed` event says how).
    CancelFeed {
        feed: u32,
    },
    /// The trace of feed `feed` (the number [`FeederOutcome::Feeding`] gave
    /// it), from byte `offset` of its body: answered
    /// [`FeederOutcome::FeedTrace`] with at most
    /// [`PAGE_MAX`](crate::trace::PAGE_MAX) bytes, so a client asks again
    /// from where each page ends until it has the whole body. `NotFound`
    /// when the feeder keeps no trace of it, `Busy` while it still runs,
    /// `Invalid` for an offset past the body's end.
    FeedTrace {
        feed: u32,
        offset: u16,
    },
    /// The portion calibration and where the rotor rests, answered
    /// [`FeederOutcome::Calibration`].
    GetCalibration,
    /// Store `calibration` if the calibration is still at `base`
    /// generation, answered `Done` once the store holds it. Owner and
    /// member sessions; the server's is refused `Forbidden`. A bowl mode
    /// this feeder has not verified is refused
    /// [`Refusal::Unverified`], and a portion weight outside
    /// [`PORTION_MG_MIN`]..=[`PORTION_MG_MAX`] `Invalid`.
    PutCalibration {
        base: u32,
        calibration: Calibration,
    },
    /// The feeding plans, answered [`FeederOutcome::Plans`].
    GetPlans,
    /// Replace every plan slot with `plans` if the plans are still at
    /// `base` generation, answered `Done` once the store holds them. A
    /// plan asking for portions outside 1 to [`PORTIONS_MAX`] is refused
    /// [`Refusal::Portions`] (the first such slot's), one with another
    /// field outside [`Plan`]'s limits (a call over [`CALL_PLAYS_MAX`]
    /// plays among them) `Invalid`. A
    /// plan whose minute or days changed (or a new one) runs only from
    /// its next occurrence after the save.
    PutPlans {
        base: u32,
        plans: [Option<Plan>; PLANS_MAX],
    },
    /// Record that the desiccant was replaced today, by the feeder's local
    /// calendar, answered `Done` with the settings' generation once the
    /// store holds it. Refused [`Refusal::NoClock`] while the clock is
    /// invalid: the feeder never records a date it does not know.
    ResetDesiccant,
    /// The stored meal call, answered [`FeederOutcome::Call`].
    GetCall,
    /// Start uploading a new meal call of `bytes` stream bytes (1 to
    /// [`CALL_BYTES_MAX`]) whose CRC-32/ISO-HDLC is `crc`: the clip stored
    /// now is erased, and the answer is `Done` once the store waits for
    /// the first chunk. Owner sessions only. The stream is the format
    /// [`CALL_RATE_HZ`] describes; the feeder checks only its length and
    /// CRC.
    StartCall {
        bytes: u32,
        crc: u32,
    },
    /// Stream bytes at `offset`, which must be where the upload stands
    /// ([`CallClip::Receiving::received`]): [`CALL_CHUNK_MAX`] bytes, or
    /// the rest of the stream when fewer are left. Answered
    /// [`FeederOutcome::Call`] once queued for the store (its `received`
    /// counts the chunk once written), `Busy` while the last chunk is still
    /// being written, `Invalid` for any other offset or length. A chunk at
    /// an offset already received is answered with the state and changes
    /// nothing, so a retry needs no remembered id. Once the last chunk is
    /// written the feeder reads the whole stream back against the CRC and
    /// stores the clip ([`CallClip::Stored`]), or keeps none
    /// ([`CallClip::None`]).
    CallChunk {
        offset: u32,
        #[serde(borrow)]
        data: &'a [u8],
    },
    /// Erase the stored meal call, answered `Done` once the store holds
    /// none. A plan's call then plays nothing. Owner sessions only.
    DeleteCall,
    /// Play the stored meal call once, so the owner can hear it, answered
    /// `Done` once queued for the speaker. Refused [`Refusal::NotFound`]
    /// with no clip stored, [`Refusal::Muted`] when the sound settings
    /// silence it now, and `Busy` while a feed, a tune, a meal's own call,
    /// setup, an install, a restart or a queued change to the clip has the
    /// speaker or the store, or when the clip would still sound as a meal's
    /// call is due to begin.
    PlayCall,
    /// Skip the meal of plan `slot` due at `due_s` (Unix seconds): its next
    /// occurrence, still to come today by the feeder's local calendar.
    /// Answered `Done` once the store holds the skip, recorded as
    /// [`EventBody::Skipped`]; the meal then never feeds, and no reminder
    /// or meal call comes for it. A plan that runs once is disabled with it.
    /// Owner, member and server sessions, as for [`Self::PutPlans`].
    /// Refused [`Refusal::NoClock`] without a valid clock and zone offset,
    /// [`Refusal::NotFound`] for an empty or disabled slot,
    /// [`Refusal::Invalid`] when `due_s` is not that occurrence (already
    /// due, tomorrow's, or already skipped), [`Refusal::Busy`] while
    /// another plans change is being written, a feed holds the outputs, a
    /// meal is being claimed or the plan's last occurrence is still
    /// running or waiting, and [`Refusal::NotWritable`] when the store
    /// cannot be written or the writer could not save it. A change to the
    /// plan's minute or days drops the skip. [`Plans::skipped`] names the
    /// skip while it stands.
    SkipMeal {
        slot: u8,
        due_s: u32,
    },
    /// Undo the skip of plan `slot`'s meal due at `due_s`, while it is
    /// still to come: answered `Done` once the store holds it, recorded as
    /// [`EventBody::Unskipped`], and the meal feeds as planned. Owner,
    /// member and server sessions. A disabled plan is no refusal: a plan
    /// that runs once is disabled by its skip, and enabled again by the
    /// undo. Refused [`Refusal::NoClock`] without a valid clock and zone
    /// offset, [`Refusal::NotFound`] for an empty slot,
    /// [`Refusal::Invalid`] when the store holds no skip of that meal (a
    /// meal that ran or was missed among them, even after the clock moved
    /// back before it), or the meal is already due or no longer the plan's
    /// next, [`Refusal::Busy`] while another plans change is being written,
    /// a feed holds the outputs or a meal is being claimed, and
    /// [`Refusal::NotWritable`] when the store cannot be written or the
    /// writer could not save it.
    UnskipMeal {
        slot: u8,
        due_s: u32,
    },
    /// The feeder's table of open alerts, answered
    /// [`FeederOutcome::Alerts`]. Owner, member and server sessions.
    /// [`FeederStatus::alerts_generation`] says when to read it again.
    GetAlerts,
}

/// The meal call's stream: 8 kHz mono, 4-bit IMA ADPCM, two samples a
/// byte with the earlier in the low nibble, starting from a predictor and
/// step index of 0. A client converts the owner's recording to it.
pub const CALL_RATE_HZ: u32 = 8_000;
/// The longest stream the feeder stores: about 8 s.
pub const CALL_BYTES_MAX: u32 = 32_256;
/// The stream bytes one [`FeederOp::CallChunk`] carries.
pub const CALL_CHUNK_MAX: usize = 512;
/// The most times one meal plays the call.
pub const CALL_PLAYS_MAX: u8 = 5;

/// The meal call as the feeder stores it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallClip {
    None,
    /// An upload of `bytes` with CRC `crc` has `received` of them. Lost
    /// with a restart, which leaves no clip.
    Receiving {
        bytes: u32,
        crc: u32,
        received: u32,
    },
    /// A clip of `bytes` stream bytes (`bytes / 4` ms a play) whose CRC
    /// is `crc`.
    Stored {
        bytes: u32,
        crc: u32,
    },
}

/// [`FeederOp::GetCall`]'s answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallView {
    pub writable: bool,
    pub clip: CallClip,
}

/// The most bank-1 bytes one [`Outcome::Bank1`] carries: well within a
/// frame beside the response's id and numbers, and a whole number of pages
/// in the bank (1,792 of them).
pub const BANK1_PAGE_MAX: usize = 768;

/// The lightest portion a calibration may store: well under any dry-food
/// compartment's load.
pub const PORTION_MG_MIN: u32 = 1_000;
/// The heaviest: well over what one compartment of the PLAF103's rotor holds.
pub const PORTION_MG_MAX: u32 = 50_000;

/// What the owner measured, as the feeder keeps it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Calibration {
    /// The mean weight of one portion, `None` until measured.
    pub portion_mg: Option<u32>,
    /// The bowl mode the feeder runs.
    pub bowl: Bowl,
}

/// A single bowl turns one way and drops every compartment into one bowl;
/// a double bowl alternates directions between two.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Bowl {
    Single,
    Double,
}

/// Where the rotor rests between feeds, as the feeding owner knows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    /// Just past a mark, where a completed feed stops.
    AtMark,
    /// Anywhere between two marks: the next feed homes it first.
    Unknown,
}

/// [`FeederOp::GetCalibration`]'s answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalibrationView {
    pub generation: u32,
    pub writable: bool,
    pub calibration: Calibration,
    /// `None` from a feeder with no feeding owner, or before it opens.
    pub phase: Option<Phase>,
}

/// Plan slots a feeder keeps, as stock keeps at most ten plans.
pub const PLANS_MAX: usize = 10;
/// The most portions one feed asks for, whoever asks (a client, a plan or
/// Dispense): the vendor app's cap on this model. Every feed asks for 1 to
/// this many; anything else is refused [`Refusal::Portions`].
pub const PORTIONS_MAX: u8 = 48;

/// One feeding plan: `portions` at minute `minute` of the local day
/// (0-1439), on the weekdays `days` lists (bit 0 Monday to bit 6 Sunday),
/// or once at its next occurrence when it lists none. A plan that ran once
/// stays in its slot, disabled. With no valid clock, plans hold; an
/// occurrence more than 15 minutes late never runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub minute: u16,
    /// 1 to [`PORTIONS_MAX`].
    pub portions: u8,
    pub days: u8,
    pub enabled: bool,
    /// How many times the meal call plays before the feed, 0 to
    /// [`CALL_PLAYS_MAX`]; 0 plays none. It plays only with a clip stored
    /// and sound allowed, and never delays the feed.
    pub call: u8,
}

/// [`FeederOp::GetPlans`]'s answer: the plan in each slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plans {
    pub generation: u32,
    pub writable: bool,
    pub plans: [Option<Plan>; PLANS_MAX],
    /// Each slot's skipped meal (Unix seconds) while its skip stands
    /// ([`FeederOp::SkipMeal`]): `None` once it is undone, or once the
    /// plan's next meal is taken or passed, or its minute or days change.
    /// A skip stays named after its meal's time, until that next meal, so
    /// a client compares it with the time: one in the past is spent.
    pub skipped: [Option<u32>; PLANS_MAX],
}

/// Frozen. The largest [`UpdateOp::Chunk`]: an eighth of a flash sector, which
/// a sealed request carries with room to spare.
pub const UPDATE_CHUNK_MAX: usize = 512;
/// Frozen, since it sizes [`UpdateView::wanted`]. The chunk offsets one
/// [`UpdateView`] names: a sector's worth.
pub const WANTED_MAX: usize = 8;

impl Op<'_> {
    /// Whether the op changes something, so its id is remembered. No
    /// wildcard, so a new op cannot land without its decision.
    pub const fn writes(&self) -> bool {
        match self {
            Self::Alive
            | Self::Status
            | Self::ListClients
            | Self::GetTime
            | Self::TailLog { .. }
            | Self::Stats { .. }
            | Self::ReadBank1 { .. } => false,
            Self::PutWifi { .. }
            | Self::TimeSync(_)
            | Self::CreateInvite { .. }
            | Self::RevokeClient { .. }
            | Self::Reboot
            | Self::PutTime { .. } => true,
            Self::Update(op) => op.writes(),
            Self::Feeder(op) => op.writes(),
        }
    }

    /// Whether the op is frozen (the update path, the keepalive and the
    /// invite), which every build of major 1 decodes and answers alike.
    pub const fn is_frozen(&self) -> bool {
        matches!(
            self,
            Self::Update(_) | Self::Alive | Self::CreateInvite { .. }
        )
    }
}

impl UpdateOp<'_> {
    /// [`Op::writes`] for the update ops. A chunk is idempotent by its
    /// offset instead, so a whole image does not push every other write
    /// out of the device's memory.
    pub const fn writes(&self) -> bool {
        match self {
            Self::State | Self::Chunk { .. } => false,
            Self::Offer { .. }
            | Self::Cancel
            | Self::EnrollKey { .. }
            | Self::KeyStatement { .. } => true,
        }
    }
}

impl FeederOp<'_> {
    /// [`Op::writes`] for the feeder's ops; a meal call's chunk is
    /// idempotent by its offset, as an update's is.
    pub const fn writes(&self) -> bool {
        match self {
            Self::GetSettings
            | Self::FeedTrace { .. }
            | Self::GetCalibration
            | Self::GetPlans
            | Self::GetCall
            | Self::CallChunk { .. }
            | Self::GetAlerts => false,
            Self::PutSettings { .. }
            | Self::SetLock { .. }
            | Self::Play { .. }
            | Self::Feed { .. }
            | Self::CancelFeed { .. }
            | Self::PutCalibration { .. }
            | Self::PutPlans { .. }
            | Self::ResetDesiccant
            | Self::StartCall { .. }
            | Self::DeleteCall
            | Self::PlayCall
            | Self::SkipMeal { .. }
            | Self::UnskipMeal { .. } => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsChanges {
    pub lights: Option<bool>,
    pub sound: Option<bool>,
    /// 0-100.
    pub volume: Option<u8>,
    pub lights_window: Option<Window>,
    pub sound_window: Option<Window>,
    /// [`DESICCANT_DAYS_MIN`] to [`DESICCANT_DAYS_MAX`].
    pub desiccant_days: Option<u8>,
    pub meal_reminder: Option<MealReminder>,
}

/// Whether the feeder sends a [`EventBody::MealSoon`] ahead of each
/// scheduled meal, and how many minutes ([`MEAL_REMINDER_MINUTES_MIN`] to
/// [`MEAL_REMINDER_MINUTES_MAX`]) ahead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MealReminder {
    #[default]
    Off,
    Before {
        minutes: u8,
    },
}

/// The shortest meal reminder lead, in minutes.
pub const MEAL_REMINDER_MINUTES_MIN: u8 = 1;
/// The longest meal reminder lead, in minutes.
pub const MEAL_REMINDER_MINUTES_MAX: u8 = 60;

/// The shortest desiccant reminder period, in days.
pub const DESICCANT_DAYS_MIN: u8 = 1;
/// The longest desiccant reminder period, in days.
pub const DESICCANT_DAYS_MAX: u8 = 90;
/// The period a feeder starts with: stock's cloud used 14 days.
pub const DESICCANT_DAYS_DEFAULT: u8 = 14;

/// When lights or sound that are on may show or play: always, or only from
/// minute `start` to minute `end` of the local day (0-1439, both included;
/// an end before the start wraps past midnight). With no valid clock every
/// window is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Window {
    #[default]
    Always,
    Between {
        start: u16,
        end: u16,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tune {
    Chime,
    Meal,
    Coin,
    Alert,
    Test,
    Twinkle,
}

/// A change to the time settings; `None` leaves a field as it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeChanges<'a> {
    /// The owner's zone as a POSIX TZ rule (at most 48 bytes, the footer
    /// of the zone's TZif file), or `""` to clear it.
    #[serde(borrow)]
    pub zone: Option<&'a str>,
    pub ntp: Option<NtpServer>,
}

/// Where the feeder asks for the time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NtpServer {
    Off,
    /// The router that gave the feeder its address.
    Gateway {
        port: u16,
    },
    Address {
        ip: [u8; 4],
        port: u16,
    },
}

/// Who last set the clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeSource {
    /// An owner's session, which may step the clock freely.
    Owner,
    /// A member's or the server's session, which may only narrow it.
    Client,
    Ntp,
}

/// What the last NTP exchange came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NtpResult {
    /// The clock took the answer.
    Used,
    /// The answer disagreed with a valid clock beyond both error bounds.
    Bounded,
    /// No answer in time.
    Lost,
    /// An answer that could not be used (unsynchronized, kiss-o'-death,
    /// too loose or malformed).
    Refused,
}

/// The time settings and the clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeView<'a> {
    pub generation: u32,
    pub writable: bool,
    /// The zone's POSIX rule, `""` when none is set.
    pub zone: &'a str,
    pub ntp: NtpServer,
    /// Unix milliseconds; `None` while the clock is invalid: the feeder
    /// needs the time, and scheduled meals hold.
    pub clock_ms: Option<u64>,
    /// How far off the clock could be.
    pub error_ms: Option<u32>,
    /// East of UTC, when a zone or a client sync gave one.
    pub offset_s: Option<i32>,
    /// The uptime clock's learned correction, parts per billion.
    pub rate_ppb: i32,
    /// How far the true rate could be from it.
    pub rate_bound_ppb: u32,
    pub learned: bool,
    pub syncs: u32,
    pub last: Option<TimeSource>,
    /// Seconds since the last fix.
    pub last_age_s: Option<u32>,
    pub ntp_last: Option<NtpResult>,
}

/// The wall clock and the zone's next change, as the wall clock takes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeSync {
    pub unix_ms: u64,
    pub offset_s: i32,
    pub next_change_s: u64,
    pub next_offset_s: i32,
}

/// What the device sends once the session is open. [`Self::Response`] is
/// frozen at index 0; the rest changes freely.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[expect(
    clippy::large_enum_variant,
    reason = "a wire message, built in place: the device cannot box"
)]
pub enum DeviceMessage<'a> {
    Response {
        id: RequestId,
        #[serde(borrow)]
        result: Outcome<'a>,
    },
    /// The next durable events after the session's cursor, oldest first:
    /// as many as fit one frame, or as the feeder had to hand.
    #[serde(borrow)]
    Events(EventBatch<'a>),
    /// Events `from..=to` left the ring before this client read them.
    Gap {
        from: u64,
        to: u64,
    },
    Sample(Sample),
    /// Record lines after [`Op::TailLog`]'s cursor, each ending in `\n`,
    /// as many whole lines as fit one frame (at most [`LOG_TEXT_MAX`]
    /// bytes). `seq` is the last line's number, the cursor to resume
    /// after; `dropped` counts the lines after the previous cursor that
    /// the batch does not hold, because the tail overwrote them before
    /// they were sent. Only to a session that asked, and only when it has
    /// sent everything before.
    Log {
        seq: u32,
        dropped: u32,
        text: &'a str,
    },
}

/// The most text one [`DeviceMessage::Log`] carries: a frame, less the
/// message's tag, numbers and the text's length prefix.
pub const LOG_TEXT_MAX: usize = MESSAGE_MAX - 16;

/// The most bytes an [`EventBody`] encodes to: what the event ring keeps
/// of one.
pub const EVENT_BODY_MAX: usize = 16;
/// The most bytes an [`Event`] encodes to: a `u64` varint's 10, an
/// `Option<u32>`'s 6 and the body.
pub const EVENT_BYTES_MAX: usize = 10 + 6 + EVENT_BODY_MAX;
/// The most encoded events one [`DeviceMessage::Events`] carries: a
/// frame's message, less the message's tag and the bytes' two-byte length.
pub const EVENTS_BYTES_MAX: usize = MESSAGE_MAX - 3;

/// Events as [`DeviceMessage::Events`] carries them: each [`Event`]
/// encoded in turn, borrowed from the message as [`DeviceMessage::Log`]'s
/// text is. The feeder fills one with an [`EventBatchWriter`] in a buffer
/// it already holds, so it never keeps an array of events; a client reads
/// it with [`Self::iter`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventBatch<'a> {
    #[serde(borrow)]
    encoded: &'a [u8],
}

impl<'a> EventBatch<'a> {
    /// The events in order. A batch whose bytes do not decode as events
    /// yields one error and then nothing.
    pub const fn iter(&self) -> EventBatchIter<'a> {
        EventBatchIter { rest: self.encoded }
    }

    pub const fn is_empty(&self) -> bool {
        self.encoded.is_empty()
    }

    /// The encoded events, as the message carries them.
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.encoded
    }
}

impl<'a> IntoIterator for &EventBatch<'a> {
    type Item = Result<Event, crate::Error>;
    type IntoIter = EventBatchIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// [`EventBatch::iter`].
#[derive(Clone, Debug)]
pub struct EventBatchIter<'a> {
    rest: &'a [u8],
}

impl Iterator for EventBatchIter<'_> {
    type Item = Result<Event, crate::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.rest.is_empty() {
            return None;
        }
        match postcard::take_from_bytes::<Event>(self.rest) {
            Ok((event, rest)) => {
                self.rest = rest;
                Some(Ok(event))
            }
            Err(_) => {
                self.rest = &[];
                Some(Err(crate::Error::Malformed))
            }
        }
    }
}

/// [`EventBatchWriter::push`]'s refusal: the event does not fit the room
/// left. Encoding an [`Event`] fails for no other reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the event does not fit the batch")]
pub struct BatchFull;

/// Fills an [`EventBatch`] in a borrowed buffer, at most
/// [`EVENTS_BYTES_MAX`] of it.
#[derive(Debug)]
pub struct EventBatchWriter<'b> {
    buf: &'b mut [u8],
    len: usize,
}

impl<'b> EventBatchWriter<'b> {
    pub fn new(buf: &'b mut [u8]) -> Self {
        let room = buf.len().min(EVENTS_BYTES_MAX);
        Self {
            buf: &mut buf[..room],
            len: 0,
        }
    }

    /// Whether any event still fits, however large.
    pub const fn has_room(&self) -> bool {
        self.buf.len() - self.len >= EVENT_BYTES_MAX
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Append `event`; an event that does not fit leaves the batch as it
    /// was.
    pub fn push(&mut self, event: &Event) -> Result<(), BatchFull> {
        let rest = self.buf.get_mut(self.len..).ok_or(BatchFull)?;
        self.len += crate::encode(event, rest).map_err(|_| BatchFull)?;
        Ok(())
    }

    /// The events written so far.
    pub fn batch(&self) -> EventBatch<'_> {
        EventBatch {
            encoded: self.buf.get(..self.len).unwrap_or_default(),
        }
    }
}

/// A request's answer. The first four variants are frozen
/// ([`Self::is_frozen`]): every answer a frozen op gets is one of them, at
/// these indices, for good. The rest changes freely.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome<'a> {
    /// Frozen at index 0. A write took effect (or a read of nothing); for
    /// a settings or Wi-Fi write, `generation` is the store's after it.
    Done { generation: Option<u32> },
    /// Frozen at index 1, with the frozen part of [`Refusal`].
    Refused(Refusal),
    /// Frozen at index 2: where the update stands, after an update op.
    Update(UpdateView),
    /// Frozen at index 3: the invite [`Op::CreateInvite`] made.
    Invite(Invite),
    /// [`Op::Status`]'s answer: the shared status and the kind's.
    #[serde(borrow)]
    Status(Status<'a>),
    #[serde(borrow)]
    Clients(Clients<'a>),
    #[serde(borrow)]
    Time(TimeView<'a>),
    /// The section [`Op::Stats`] asked for.
    #[serde(borrow)]
    Stats(Stats<'a>),
    /// One page of bank 1, after [`Op::ReadBank1`].
    #[serde(borrow)]
    Bank1(Bank1Page<'a>),
    /// The answer to an [`Op::Feeder`].
    #[serde(borrow)]
    Feeder(FeederOutcome<'a>),
}

impl Outcome<'_> {
    /// Whether the answer is one a frozen op may get.
    pub const fn is_frozen(&self) -> bool {
        match self {
            Self::Done { .. } | Self::Update(_) | Self::Invite(_) => true,
            Self::Refused(refusal) => refusal.is_frozen(),
            Self::Status(_)
            | Self::Clients(_)
            | Self::Time(_)
            | Self::Stats(_)
            | Self::Bank1(_)
            | Self::Feeder(_) => false,
        }
    }
}

/// The answers only a [`FeederOp`] gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeederOutcome<'a> {
    Settings(Settings),
    /// The feed is reserved in the journal under `feed` and runs now; a
    /// retry of the request returns this again rather than feeding twice.
    Feeding {
        feed: u32,
    },
    /// One page of the trace [`FeederOp::FeedTrace`] asked for.
    #[serde(borrow)]
    FeedTrace(TracePage<'a>),
    /// The calibration, after [`FeederOp::GetCalibration`].
    Calibration(CalibrationView),
    /// The plans, after [`FeederOp::GetPlans`].
    Plans(Plans),
    /// The meal call, after [`FeederOp::GetCall`] or
    /// [`FeederOp::CallChunk`].
    Call(CallView),
    /// The table of open alerts, after [`FeederOp::GetAlerts`].
    Alerts(AlertsView),
}

/// One page of bank 1: `bytes` from `offset`, of the `total` the feeder
/// serves (the bank's whole extent).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bank1Page<'a> {
    pub offset: u32,
    pub total: u32,
    #[serde(borrow)]
    pub bytes: &'a [u8],
}

/// Whether the feeder can start a feed now. One decision, which both
/// [`Op::Status`] reports ([`FeederStatus::readiness`]) and
/// [`FeederOp::Feed`] obeys: a feed asked for while
/// [`Self::Blocked`] is refused with [`Blocked::refusal`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Readiness {
    Ready,
    Blocked(Blocked),
}

/// Why a feed cannot start now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Blocked {
    /// The feeding owner has not opened yet (it opens once the boot's
    /// storage is ready), or a newly installed build has not confirmed
    /// itself yet.
    Starting,
    /// The feed journal could not be opened: no feed this boot.
    Journal,
    /// The feeding owner stopped on a fault: no feed until the feeder
    /// restarts.
    Fault,
    /// A feed a reset interrupted is being resolved as uncertain.
    Resolving,
    /// A feed is queued or runs.
    Feeding,
    /// A Wi-Fi join runs or waits for the outputs.
    Joining,
    /// An install holds the flash.
    Updating,
    /// BLE setup mode runs, which ends in a restart.
    Setup,
    /// This firmware has no feeding owner.
    Unserved,
    /// The feeder runs on its batteries and they are below the floor a
    /// feed needs, or unread; or this boot followed a brownout and the
    /// adapter has not been seen since. It clears when the adapter returns
    /// or the pack recovers past the floor's hysteresis.
    Battery,
    /// A restart was asked for (a client's reboot, setup mode's end, the
    /// factory reset) and comes within seconds: a feed started now would be
    /// cut, so none starts.
    Restarting,
}

impl Blocked {
    /// What a feed asked for now is refused with.
    pub const fn refusal(self) -> Refusal {
        match self {
            Self::Journal | Self::Fault => Refusal::Fault,
            Self::Unserved => Refusal::Unavailable,
            Self::Starting
            | Self::Resolving
            | Self::Feeding
            | Self::Joining
            | Self::Updating
            | Self::Setup
            | Self::Battery
            | Self::Restarting => Refusal::Busy,
        }
    }

    /// Whether the reason lasts: it holds until a restart or an operator
    /// ends it, rather than clearing by itself within seconds or one feed.
    /// Only a lasting reason is recorded in the history
    /// ([`EventBody::FeedingBlocked`]), so joins and feeds do not fill it.
    /// [`Self::Unserved`] is the build's, never a change, so the status
    /// alone reports it.
    pub const fn lasting(self) -> bool {
        match self {
            Self::Journal | Self::Fault | Self::Updating | Self::Setup => true,
            // The supply's changes are their own events (`Supply`), rate
            // limited, so an adapter that comes and goes does not fill the
            // history twice over.
            Self::Starting
            | Self::Resolving
            | Self::Feeding
            | Self::Joining
            | Self::Unserved
            | Self::Battery
            | Self::Restarting => false,
        }
    }
}

/// Why a request was refused. The first eight variants are frozen
/// ([`Self::is_frozen`], and [`FrozenRefusal`] names them): they are every
/// refusal a frozen op gets, at these indices, for good. The rest changes
/// freely.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    /// This client's role may not do that.
    Forbidden,
    /// Another change is still being written, or the stores are loading.
    Busy,
    /// This build does not serve the op: one of another kind, or one a
    /// newer client sent that this build cannot decode. Update the feeder
    /// first.
    Unavailable,
    /// A field failed the feeder's checks.
    Invalid,
    /// The store cannot be written now.
    NotWritable,
    /// Every client slot (or invite) is taken.
    Full,
    /// An update op was refused: why.
    Update(UpdateRefusal),
    /// No such thing: a remembered invite that has expired, among others.
    NotFound,
    /// The store moved past the request's `base`: re-read and decide again.
    Stale { current: u32 },
    /// The `sound` setting is off.
    Muted,
    /// The clock refused the sync.
    Time,
    /// The feeding owner stopped on a fault and serves no feed until the
    /// feeder restarts.
    Fault,
    /// A bowl mode this feeder has not verified on hardware.
    Unverified,
    /// A feed, or a plan's, asked for `asked` portions, outside 1 to
    /// [`PORTIONS_MAX`].
    Portions { asked: u8 },
    /// The clock is invalid, so the feeder cannot date the change: sync
    /// the time first.
    NoClock,
}

impl Refusal {
    /// Whether the refusal is one a frozen op may get.
    pub const fn is_frozen(&self) -> bool {
        self.frozen().is_some()
    }

    /// The refusal as the frozen part names it, if it is one.
    pub const fn frozen(&self) -> Option<FrozenRefusal> {
        Some(match *self {
            Self::Forbidden => FrozenRefusal::Forbidden,
            Self::Busy => FrozenRefusal::Busy,
            Self::Unavailable => FrozenRefusal::Unavailable,
            Self::Invalid => FrozenRefusal::Invalid,
            Self::NotWritable => FrozenRefusal::NotWritable,
            Self::Full => FrozenRefusal::Full,
            Self::Update(refusal) => FrozenRefusal::Update(refusal),
            Self::NotFound => FrozenRefusal::NotFound,
            Self::Stale { .. }
            | Self::Muted
            | Self::Time
            | Self::Fault
            | Self::Unverified
            | Self::Portions { .. }
            | Self::NoClock => return None,
        })
    }
}

/// The frozen refusals: the first variants of [`Refusal`], in its order.
/// What the device answers on the frozen paths (an update, a key, an
/// invite) is typed as this, so a refusal outside the frozen part cannot
/// reach a client of another build. It becomes a [`Refusal`] only to go on
/// the wire, where both encode alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrozenRefusal {
    Forbidden,
    Busy,
    Unavailable,
    Invalid,
    NotWritable,
    Full,
    Update(UpdateRefusal),
    NotFound,
}

impl From<FrozenRefusal> for Refusal {
    fn from(refusal: FrozenRefusal) -> Self {
        match refusal {
            FrozenRefusal::Forbidden => Self::Forbidden,
            FrozenRefusal::Busy => Self::Busy,
            FrozenRefusal::Unavailable => Self::Unavailable,
            FrozenRefusal::Invalid => Self::Invalid,
            FrozenRefusal::NotWritable => Self::NotWritable,
            FrozenRefusal::Full => Self::Full,
            FrozenRefusal::Update(refusal) => Self::Update(refusal),
            FrozenRefusal::NotFound => Self::NotFound,
        }
    }
}

/// Frozen. Where an update stands. The install itself does not depend on the
/// session: a client that lost its connection reconnects, reads
/// [`UpdateOp::State`] and sends what [`Self::wanted`] names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateView {
    pub phase: UpdatePhase,
    /// The bank this answer comes from.
    pub bank: u8,
    /// The highest confirmed version: an offer at or below it needs a
    /// downgrade statement.
    pub min_version: u32,
    /// The manifest being checked, installed or given way for, if any.
    pub digest: Option<[u8; 32]>,
    pub version: u32,
    pub image_len: u32,
    /// Bytes written and read back so far.
    pub written: u32,
    /// The chunk offsets wanted now, each [`UPDATE_CHUNK_MAX`] bytes (the
    /// last may be shorter); all `None` between sectors or outside an
    /// install.
    pub wanted: [Option<u32>; WANTED_MAX],
    /// Why the last install stopped before activation, if it did.
    pub stopped: Option<UpdateStop>,
    /// Bank 0 as bank 1 last checked it, at boot or for an offer; always
    /// [`Bank0Check::Unchecked`] from bank 0.
    pub bank0: Bank0Check,
}

/// Frozen. Bank 0, the recovery copy, as bank 1 last checked it. Bank 1 gives way
/// for an update only to a bank 0 that booted, recorded its own image and
/// still holds exactly that image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Bank0Check {
    /// Not checked yet this boot.
    Unchecked,
    /// Bank 0 holds the image it recorded: updates can go ahead.
    Match,
    /// Bank 1 will not give way to it, so every update is refused.
    Blocked { reason: Bank0Blocked },
}

/// Frozen. Why bank 1 will not give way to bank 0, in the order it checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Bank0Blocked {
    /// Bank 0's loader marker is not valid.
    Marker,
    /// Bank 0 never recorded itself: written over UART and not booted
    /// since. Booting it once records it.
    Unrecorded,
    /// Bank 0's built-in update key is not the one the offer leans on.
    Key,
    /// Bank 0's image headers do not give an image.
    Extent,
    /// Bank 0 changed since it recorded itself, in length or hash.
    Changed,
    /// Bank 0 could not be read.
    Read,
}

/// Frozen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdatePhase {
    /// No update in progress.
    Idle,
    /// The offer is being checked; ask [`UpdateOp::State`] again.
    Checking,
    /// Bank 1 recorded the offer and restarts into bank 0 once this
    /// answer is out: reconnect and offer the same manifest again.
    GivingWay,
    /// Bank 0 is writing bank 1: send the chunks [`UpdateView::wanted`]
    /// names.
    Receiving,
    /// Every byte verified and the marker written: the feeder restarts
    /// into the new build, which confirms itself once it is healthy.
    Activated,
}

/// What the update store held about a candidate as a boot came up. Not part
/// of the frozen update path: only [`EventBody::UpdateAtBoot`] carries it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateCandidate {
    /// None, or a store that could not be read.
    None,
    /// Bank 1 gave way for one, or bank 0 accepted it.
    Pending,
    /// Bank 0 activated one, and no build has confirmed it.
    Activated,
}

/// Frozen. Why an install stopped. Bank 1's marker was never written, so
/// the feeder keeps running bank 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateStop {
    /// No chunk came for a minute.
    Timeout,
    Cancelled,
    /// A chunk at an offset that was not wanted, or of the wrong length.
    Order,
    /// The image's first bytes are not the loader's marker.
    Marker,
    /// A sector did not read back as written.
    Write,
    /// The whole image's SHA-256 differs from the manifest's.
    Hash,
    /// The marker did not read back.
    Activate,
    /// The update store could not be written.
    Store,
}

/// Frozen. Why an update op was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateRefusal {
    /// Not a version 2 manifest (a version 1 one, wrong length or magic),
    /// or an unknown image kind.
    Manifest,
    /// No trusted key has the manifest's (or a statement's) key id.
    Key,
    /// That key was retired.
    Revoked,
    Signature,
    /// Not for bank 1, outside the reviewed size, or built for another
    /// device model.
    Target,
    /// That build is already running.
    Same,
    /// The version is not above the floor (or the image is stock): a
    /// downgrade statement must come with it.
    Downgrade,
    /// The downgrade or key statement does not check out (another feeder,
    /// another manifest, an old floor, a key that may not sign it).
    Statement,
    /// Bank 0 installs only the manifest bank 1 gave way for; cancel
    /// that update first.
    Pending,
    /// This manifest was activated and failed its trial: sign another.
    Failed,
    /// Bank 0 does not hold a build that can install it, so bank 1 does
    /// not give way: `reason` names the check.
    Fallback {
        reason: Bank0Blocked,
    },
    /// An install reset the feeder this power cycle: no new install until
    /// the next power cycle.
    Attempt,
    /// Another update is in progress.
    Installing,
    /// No install is receiving chunks.
    NotInstalling,
    /// The update store cannot be read or written.
    Store,
    /// Nobody pressed Dispense in time to confirm the key.
    Unconfirmed,
    /// The feeder runs on its batteries: no install starts until the
    /// adapter returns.
    Battery,
    /// The key an enrollment names is already enrolled as a session key
    /// (owner, member or server), which is never an update signer too. It
    /// stays what it was.
    HeldAsSessionKey,
}

/// What every device reports about itself, and its kind's part.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status<'a> {
    pub build: &'a str,
    pub bank: u8,
    pub uptime_ms: u32,
    pub boot_count: u32,
    pub reset_flags: u32,
    pub faults: u32,
    pub rssi: i32,
    pub wifi: WifiSource,
    /// Every time this boot a joined link was lost and the device went on
    /// to rejoin, whatever noticed it.
    pub link_losses: u32,
    /// Those of `link_losses` the link check noticed (the DHCP lease ran out
    /// or the gateway stopped answering) rather than the association. The
    /// name is from the internet DNS check that check replaced.
    pub dns_losses: u32,
    /// Unix milliseconds, `None` while the clock is invalid.
    pub clock_ms: Option<u64>,
    pub power: Option<Power>,
    pub wifi_generation: u32,
    /// The newest event's sequence number, `None` before the first.
    pub last_event: Option<u64>,
    pub sessions: u8,
    pub device: DeviceStatus,
}

/// The part of [`Status`] only one kind of device reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceStatus {
    Feeder(FeederStatus),
}

/// What a feeder reports beside the shared status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeederStatus {
    pub locked: Option<bool>,
    pub settings_generation: u32,
    /// The hopper as the food beam last read it.
    pub food: FoodLevel,
    /// The last feed this boot that ended on a fault (`Failed` or
    /// `Uncertain`), until a later feed completes: a jam, a blocked outlet
    /// or a sensor that still wants seeing to. Its `Fed` event has the rest.
    pub feed_fault: Option<FeedFault>,
    pub desiccant: Desiccant,
    /// Whether a feed can start now.
    pub readiness: Readiness,
    /// The supply as the feeder judges it: an unread or stale one says
    /// so ([`Battery`]).
    pub supply: Supply,
    /// Changes when the table of open alerts does
    /// ([`AlertsView::generation`]), so a session sees a change on its next
    /// status. Counts from 0 at each boot.
    pub alerts_generation: u32,
}

/// The desiccant reminder, counted in the feeder's local calendar days.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Desiccant {
    /// No replacement is recorded yet.
    Unset,
    /// The clock is invalid, so the days cannot be counted.
    Unknown,
    /// Days left of the period since the last replacement: due at 0, and
    /// counting on below zero after that.
    Remaining { days: i32 },
}

impl Desiccant {
    /// Whether the replacement is due; `None` while it cannot be counted.
    pub const fn due(self) -> Option<bool> {
        match self {
            Self::Remaining { days } => Some(days <= 0),
            Self::Unset | Self::Unknown => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WifiSource {
    /// The build's credentials. An image without build Wi-Fi (a consumer
    /// image) also reports `Build` while nothing is stored, when it joins
    /// nothing at all.
    Build,
    Stored,
    Fallback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Power {
    pub adapter: bool,
    pub battery_mv: Option<u32>,
}

/// The supply as the feeder judges it: never an
/// unknown passed off as a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Supply {
    /// The adapter's detect pin; `None` with no fresh sample.
    pub adapter: Option<bool>,
    pub battery: Battery,
    /// Whether the feeding owner judges the pack below the floor a feed
    /// needs (with its hold and hysteresis); `None` before it has judged,
    /// in a firmware without one, or with no pack fitted. Judged on the
    /// adapter too, where it does not refuse feeds.
    pub below_floor: Option<bool>,
}

/// The battery pack in a [`Supply`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Battery {
    /// No pack fitted: the channel reads near zero.
    Absent,
    /// The pack's millivolts, and its charge (0–100 in steps of 10) on
    /// stock's curve of the battery channel.
    Reading { mv: u16, percent: u8 },
    /// A value no pack of this board can read (above fresh cells, or on
    /// the pack alone too low to be running), as read.
    Implausible { mv: u16 },
    /// Nothing sampled: the monitor has not run, its ADC failed, or the
    /// firmware does not sample the supply.
    Unread,
    /// The last sample is this old, past the monitor's limit.
    Stale { age_ms: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub generation: u32,
    pub lights: bool,
    pub sound: bool,
    pub volume: u8,
    pub lights_window: Window,
    pub sound_window: Window,
    /// The desiccant reminder's period in days.
    pub desiccant_days: u8,
    /// The local calendar day (days since 1970-01-01) the desiccant was
    /// last replaced; `None` before the first [`FeederOp::ResetDesiccant`].
    pub desiccant_reset: Option<i32>,
    pub meal_reminder: MealReminder,
    pub writable: bool,
}

/// The enrolled clients, in store order.
pub const CLIENTS_MAX: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clients<'a> {
    #[serde(borrow)]
    pub clients: [Option<Client<'a>>; CLIENTS_MAX],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Client<'a> {
    pub key: [u8; 32],
    pub label: &'a str,
    pub role: Role,
    /// Whether this is the session asking.
    pub you: bool,
}

/// Frozen, as [`Outcome::Invite`] carries it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    pub id: u32,
    /// The PSK the new client's handshake must hold. Show it once (QR or
    /// link); anyone holding it within `seconds` joins with the role.
    pub secret: [u8; 32],
    pub seconds: u16,
}

/// One durable record of the feeder's history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Monotonic across reboots; a client's cursor.
    pub seq: u64,
    /// Unix seconds, `None` when the clock was invalid.
    pub at: Option<u32>,
    pub body: EventBody,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventBody {
    Boot {
        bank: u8,
        reset_flags: u32,
    },
    Lock {
        locked: bool,
        by: Source,
    },
    Settings {
        generation: u32,
        by: Source,
    },
    Wifi {
        generation: u32,
        by: Source,
    },
    Enrolled {
        client: Fingerprint,
        role: Role,
        path: Path,
    },
    Revoked {
        client: Fingerprint,
        by: Source,
    },
    ClockSync {
        by: Source,
        /// How far the clock moved, saturated to i32 (about 24 days).
        jump_ms: Option<i32>,
    },
    /// Events the feeder could not record (its queue overflowed).
    Lost {
        count: u16,
    },
    /// A feed ended. `completed` counts the portions the feedback sensor
    /// confirmed. `hopper` is what the food beam read during it: a feed
    /// runs whatever the hopper reads, as stock's does, so a misreading
    /// beam never skips a meal, and an empty one is recorded here.
    Fed {
        feed: u32,
        requested: u8,
        completed: u8,
        outcome: FeedOutcome,
        reason: FeedReason,
        hopper: FoodLevel,
        by: Source,
    },
    /// The lasting reason the feeder cannot feed changed ([`Blocked::lasting`]):
    /// `Some` names the new one, `None` says it can feed again.
    FeedingBlocked {
        reason: Option<Blocked>,
    },
    /// The adapter came or went, or the pack crossed its floor. At most
    /// one a minute: changes inside that minute are recorded together,
    /// as the supply stands at its end.
    Supply(Supply),
    /// A client replaced the plans; `generation` is the store's after it.
    Plans {
        generation: u32,
        by: Source,
    },
    /// The occurrence of plan `plan` due at `due` (Unix seconds) did not
    /// run, and never will.
    Missed {
        plan: u8,
        due: u32,
        reason: MissedReason,
    },
    /// Dispense was pressed on an unlocked panel while a feed could not
    /// start (`reason`), during a feed among them: it started nothing and
    /// is not queued.
    DispenseRefused {
        reason: Blocked,
    },
    /// The occurrence of plan `plan` due at `due` (Unix seconds), at
    /// minute `minute` of the local day, is expected to feed `portions`:
    /// sent the reminder's lead ahead of it. A restart inside the lead may
    /// send it again, with the same `plan` and `due`. It is never
    /// withdrawn: the meal may still be missed, or the plan disabled,
    /// deleted or moved, so the `Fed` or `Missed` for that `plan` and `due`
    /// (or the plans) say what happened.
    MealSoon {
        plan: u8,
        due: u32,
        minute: u16,
        portions: u8,
    },
    /// The food beam's level changed: at most once a feed or a reading at
    /// rest, however it flickers. A boot's first reading counts as a change
    /// only if a feed took it.
    Food {
        level: FoodLevel,
    },
    /// `by` skipped the occurrence of plan `plan` due at `due` (Unix
    /// seconds): it will not feed unless the skip is undone before it is
    /// due ([`Self::Unskipped`]).
    Skipped {
        plan: u8,
        due: u32,
        by: Source,
    },
    /// `by` undid the skip of plan `plan`'s occurrence due at `due`: it
    /// feeds as planned.
    Unskipped {
        plan: u8,
        due: u32,
        by: Source,
    },
    /// An install stopped before activation: what the update's state says
    /// only until the next offer, kept past a restart.
    UpdateStop {
        stop: UpdateStop,
    },
    /// What the update store held as this boot came up, recorded once in
    /// every boot. With the bank of the boots around it, this tells how a
    /// boot got there: an install's own restarts from a fall back to bank 0.
    /// Recorded after the restart, as an install holds the write turn up to
    /// the reset and nothing queued before it could land.
    UpdateAtBoot {
        candidate: UpdateCandidate,
    },
    /// A restart someone asked for is on its way. The writer appends what is
    /// queued in the restart's grace, so this normally lands; if it doesn't,
    /// the restart counts as unplanned.
    Restarting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeedOutcome {
    Done,
    Cancelled,
    /// It never started: the feeder could not get ready in time.
    Expired,
    Failed,
    /// The feeder stopped without recording a result: it may have
    /// dispensed anything up to `requested`. Never retried automatically.
    /// When a power cut lost the reservation itself, `requested` is the
    /// feeder's cap on one feed, not what was asked.
    Uncertain,
}

/// A feed that ended on a fault, as [`Status::feed_fault`] names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedFault {
    pub feed: u32,
    pub outcome: FeedOutcome,
    pub reason: FeedReason,
}

/// The most open alerts the feeder holds.
pub const ALERTS_MAX: usize = 12;

/// The feeder's table of open alerts, whole: a read is one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertsView {
    /// The ring's epoch, which a feed's number counts in. It holds for a
    /// boot, so it comes once rather than with each entry.
    pub epoch: u32,
    pub generation: u32,
    /// A report left the table for want of room, or the boot read missed some
    /// of the ring, and what is missing could still have been listed. An
    /// absent alert may not have ended.
    pub more: bool,
    /// The feeder can't vouch for an absent condition yet: its thresholds are
    /// running again after a boot, or a reading one needs is unknown.
    pub settling: bool,
    /// Oldest first, the free slots last.
    pub alerts: [Option<OpenAlert>; ALERTS_MAX],
}

/// One entry of the feeder's table of open alerts: what its rules say is
/// wrong now. About 16 B, since the table lives in the loop's RAM.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAlert {
    /// A report's own time, or when a condition opened (Unix seconds); `None`
    /// if the clock was not valid then.
    pub at: Option<NonZeroU32>,
    pub alert: Alert,
}

impl OpenAlert {
    /// What makes this the same alert across reads and restarts, within one
    /// feeder. A feed's number counts within the ring's epoch.
    pub const fn key(&self) -> AlertKey {
        let kind = self.alert.kind();
        match self.alert {
            Alert::MealFailed { feed, .. } | Alert::MealUncertain { feed, .. } => {
                AlertKey::Feed { feed }
            }
            Alert::MealMissed { plan, due, .. } | Alert::MealNoRecord { plan, due } => {
                AlertKey::Meal { plan, due }
            }
            Alert::ClientJoined { client, .. } | Alert::ClientRevoked { client, .. } => {
                AlertKey::Client {
                    kind,
                    client,
                    at: self.at,
                }
            }
            Alert::PowerWasOut | Alert::UpdateFailed { .. } | Alert::RolledBack => {
                AlertKey::Timed { kind, at: self.at }
            }
            Alert::RepeatedRestarts
            | Alert::NoClock
            | Alert::CannotFeed { .. }
            | Alert::FoodLow
            | Alert::FoodLikelyOut { .. }
            | Alert::DesiccantDue { .. }
            | Alert::BatteryBelowFloor
            | Alert::PlansStuck => AlertKey::Kind(kind),
        }
    }
}

/// An alert's identity. A feed's key spans its outcomes, so an uncertain
/// feed's second `Fed` replaces the first alert, and a meal's spans a report
/// of no record and the `Missed` that answers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertKey {
    Feed {
        feed: u32,
    },
    Meal {
        plan: u8,
        due: u32,
    },
    /// So a later rejoin alerts again.
    Client {
        kind: AlertKind,
        client: Fingerprint,
        at: Option<NonZeroU32>,
    },
    Timed {
        kind: AlertKind,
        at: Option<NonZeroU32>,
    },
    /// A condition, present or absent.
    Kind(AlertKind),
}

/// What is wrong, and what the event or reading adds to its kind. Fields sit
/// in the variants rather than in a shared struct, whose padding would make
/// an entry larger.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Alert {
    /// A feed ended `Failed` or `Expired`. `plan` is its plan, if one asked.
    MealFailed {
        feed: u32,
        plan: Option<u8>,
        requested: u8,
        completed: u8,
        outcome: FeedOutcome,
        reason: FeedReason,
    },
    /// A feed ended `Uncertain`, and may have dispensed up to `requested`.
    MealUncertain {
        feed: u32,
        plan: Option<u8>,
        requested: u8,
        completed: u8,
        reason: FeedReason,
    },
    /// The occurrence did not run, and never will.
    MealMissed {
        plan: u8,
        due: u32,
        reason: MissedReason,
    },
    /// The occurrence is well past due and the schedule has no record of it,
    /// which should be a firmware bug.
    MealNoRecord {
        plan: u8,
        due: u32,
    },
    PowerWasOut,
    UpdateFailed {
        stop: UpdateStop,
    },
    /// Bank 0 kept running after bank 1.
    RolledBack,
    ClientJoined {
        client: Fingerprint,
        role: Role,
        path: Path,
    },
    ClientRevoked {
        client: Fingerprint,
        by: Source,
    },
    RepeatedRestarts,
    NoClock,
    CannotFeed {
        reason: Blocked,
    },
    FoodLow,
    /// There is no empty reading: `confirmed` portions have gone since the
    /// beam, which sits about five above the rotor, read low.
    FoodLikelyOut {
        confirmed: u8,
    },
    DesiccantDue {
        days: i32,
    },
    BatteryBelowFloor,
    /// With a valid clock, the plans have stayed out of reach, or a change to
    /// them has never finished, so nothing can say whether meals ran.
    PlansStuck,
}

impl Alert {
    pub const fn kind(&self) -> AlertKind {
        match self {
            Self::MealFailed { .. } => AlertKind::MealFailed,
            Self::MealUncertain { .. } => AlertKind::MealUncertain,
            Self::MealMissed { .. } => AlertKind::MealMissed,
            Self::MealNoRecord { .. } => AlertKind::MealNoRecord,
            Self::PowerWasOut => AlertKind::PowerWasOut,
            Self::UpdateFailed { .. } => AlertKind::UpdateFailed,
            Self::RolledBack => AlertKind::RolledBack,
            Self::ClientJoined { .. } => AlertKind::ClientJoined,
            Self::ClientRevoked { .. } => AlertKind::ClientRevoked,
            Self::RepeatedRestarts => AlertKind::RepeatedRestarts,
            Self::NoClock => AlertKind::NoClock,
            Self::CannotFeed { .. } => AlertKind::CannotFeed,
            Self::FoodLow => AlertKind::FoodLow,
            Self::FoodLikelyOut { .. } => AlertKind::FoodLikelyOut,
            Self::DesiccantDue { .. } => AlertKind::DesiccantDue,
            Self::BatteryBelowFloor => AlertKind::BatteryBelowFloor,
            Self::PlansStuck => AlertKind::PlansStuck,
        }
    }

    pub const fn level(&self) -> AlertLevel {
        self.kind().level()
    }
}

/// An alert's kind, without what it says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertKind {
    MealFailed,
    MealUncertain,
    MealMissed,
    MealNoRecord,
    PowerWasOut,
    UpdateFailed,
    RolledBack,
    ClientJoined,
    ClientRevoked,
    RepeatedRestarts,
    NoClock,
    CannotFeed,
    FoodLow,
    FoodLikelyOut,
    DesiccantDue,
    BatteryBelowFloor,
    PlansStuck,
}

impl AlertKind {
    pub const fn level(self) -> AlertLevel {
        match self {
            Self::MealFailed
            | Self::MealUncertain
            | Self::MealMissed
            | Self::MealNoRecord
            | Self::NoClock
            | Self::CannotFeed
            | Self::FoodLikelyOut
            | Self::PlansStuck => AlertLevel::Urgent,
            Self::FoodLow
            | Self::DesiccantDue
            | Self::PowerWasOut
            | Self::BatteryBelowFloor
            | Self::UpdateFailed
            | Self::RolledBack
            | Self::RepeatedRestarts => AlertLevel::Soon,
            Self::ClientJoined | Self::ClientRevoked => AlertLevel::Security,
        }
    }

    /// Whether it is present or absent by its kind alone, ending with its
    /// state rather than lapsing. Repeated restarts report something that
    /// happened, but are tracked this way so a later episode alerts again.
    pub const fn is_condition(self) -> bool {
        matches!(
            self,
            Self::RepeatedRestarts
                | Self::NoClock
                | Self::CannotFeed
                | Self::FoodLow
                | Self::FoodLikelyOut
                | Self::DesiccantDue
                | Self::BatteryBelowFloor
                | Self::PlansStuck
        )
    }
}

/// How soon an alert wants the owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertLevel {
    Urgent,
    Soon,
    Security,
}

/// What the food beam across the hopper reads, filtered as stock filters
/// its `surplusGrain`: one reading with food shows food at once, and the
/// hopper reads low only after a run of readings without. Stock counts 20
/// readings; its poll rate is not recovered, and the firmware assumes the
/// nominal 50 ms its outlet filter implies, so a second. The beam is lit
/// during feeds and in a short window at rest, so the level is the latest of
/// either's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FoodLevel {
    /// No qualified reading: nothing has read the beam this boot, or this
    /// feed ended before the beam settled.
    Unread,
    Present,
    /// The beam read no food, which leaves about five portions, since it sits
    /// above the rotor (at any time during a feed, for a `Fed`).
    Low,
}

/// Why a feed ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeedReason {
    Done,
    Cancelled,
    /// The feeder's clock jumped.
    Clock,
    /// The feeder could not watch the motor often enough.
    ObservationGap,
    /// The whole feed ran past its limit.
    Timeout,
    /// No rotor mark for too long: the mark sensor lost, or a rotor stalled
    /// with no current to show it.
    NoFeedback,
    Power,
    /// A sensor could not be read: the ADC, the beams, or current at rest
    /// over the jam limit.
    Sensor,
    /// The outlet beam read blocked. Mid-feed, the feed ends with the
    /// portions delivered so far and is never resumed (stock pauses up to
    /// about 10 minutes instead); at the start it ends with no motion.
    OutletBlocked,
    /// One current reading over the ceiling, or the limit held while the
    /// motor is stopped (a pause): stopped at once, no reversal.
    Overcurrent,
    /// The feedback sensor's edges could not be trusted.
    FeedbackStream,
    /// An output failed to apply or verify.
    Output,
    /// A jam no reversal freed: the motor backed off three times, as
    /// stock's does (its code 2031), or the jam held while backing off.
    /// The feed ends, never retried; stock counts it done.
    Jammed,
    /// The mark sensor read a mark for longer than any mark lasts.
    MarkStuck,
    /// A reset (power cut, watchdog, fault) cut the feed short, and the
    /// next boot resolved it: always `Uncertain`.
    Interrupted,
}

/// The first 4 bytes of a client's key: enough to tell clients apart in
/// the history, which has 16 bytes for a body.
pub type Fingerprint = [u8; 4];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    Panel,
    Client(Fingerprint),
    Feeder,
    /// The plan in this slot.
    Schedule(u8),
}

/// Why a plan's occurrence did not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissedReason {
    /// It fell while the feeder was off or had no valid time, or the clock
    /// moved past it, more than the grace (15 minutes) ago.
    Late,
    /// It waited past the grace for a feed it could not start: why.
    Blocked(Blocked),
    /// The feeder stopped after it took the occurrence and before the
    /// feed was reserved.
    Interrupted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Path {
    Setup,
    Invite,
    Recovery,
    /// An owner's session asked and a press on the panel confirmed: how an
    /// update signer's key joins.
    Press,
}

/// A live measurement, sent only to subscribers and never stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sample {
    /// Feeder uptime.
    pub at_ms: u32,
    pub metric: Metric,
}

/// One live value. A feeder sends each kind on its own cadence, and one
/// not sent before the next of its kind is replaced by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Metric {
    Power(Power),
    Rssi(i32),
    /// The heap's free bytes now, and the fewest since boot.
    Heap {
        free: u32,
        min: u32,
    },
    /// The loop task's passes since the last loop sample: how many, the
    /// longest and the 99th percentile (an upper bound, within an eighth).
    Loop {
        passes: u32,
        max_us: u32,
        p99_us: u32,
    },
    /// The gateway probe's round trip since the last gateway sample, the
    /// latest that was answered; `None` when none was.
    Gateway {
        round_trip_ms: Option<u32>,
    },
    /// The joined access point's channel (`None` while not joined), and how
    /// often this boot a join landed on another access point than the join
    /// before it.
    Wifi {
        channel: Option<u8>,
        ap_changes: u32,
    },
    /// The supply, judged, each time it is sampled.
    Supply(Supply),
}

/// What [`Op::Stats`] asks for. The food, feeds and portions sections are
/// the feeder's, and the counters count its feeds: a kind's own sections
/// move to its module when a second kind keeps statistics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StatsSection {
    Counters,
    Memory,
    Flash,
    Power,
    Food,
    /// This boot's feeds as their traces measured them.
    Feeds,
    Portions,
}

impl StatsSection {
    pub const ALL: [Self; 7] = [
        Self::Counters,
        Self::Memory,
        Self::Flash,
        Self::Power,
        Self::Food,
        Self::Feeds,
        Self::Portions,
    ];
}

/// One section of the statistics, each within a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stats<'a> {
    Counters(Counters),
    #[serde(borrow)]
    Memory(Memory<'a>),
    Flash(Flash),
    Power(PowerStats),
    Food(Food),
    Feeds(FeedStats),
    Portions(Portions),
}

/// What the feeder counts, since this boot and over its life.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counters {
    pub boot: Counts,
    /// Why this boot started.
    pub reset: ResetCause,
    /// Over the feeder's life, this boot's included, from a feeder that
    /// keeps its totals in storage; `None` from one that keeps none (no
    /// storage sectors hold them yet).
    pub lifetime: Option<Counts>,
    /// Boots by their reset's cause, this one included; `None` as
    /// `lifetime`.
    pub resets: Option<Resets>,
    /// This boot's sessions: the longest and the mean time from accepting
    /// the connection to the open session.
    pub handshake_max_ms: u32,
    pub handshake_mean_ms: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub uptime_s: u32,
    /// Feeds that ended, however they ended.
    pub feeds: u32,
    /// Feeds that ended other than done.
    pub feeds_failed: u32,
    /// Portions the feedback sensor confirmed.
    pub portions: u32,
    /// Button gestures the panel took.
    pub presses: u32,
    /// Sessions opened.
    pub sessions: u32,
    /// Wi-Fi links lost, each followed by a rejoin.
    pub reconnects: u32,
    /// DHCP leases renewed (or rebound) without a rejoin.
    pub dhcp_renewals: u32,
    /// Times the adapter went away.
    pub outages: u32,
    /// Seconds run on the batteries.
    pub battery_s: u32,
}

/// Why a boot started, from the ROM's reset flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResetCause {
    /// No reset flag: the supply came up.
    #[default]
    PowerOn,
    Watchdog,
    /// A system reset the firmware asked for (a restart, an update).
    Software,
    Brownout,
    /// A flag none of the above names.
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resets {
    pub power_on: u32,
    pub watchdog: u32,
    /// A system reset the firmware asked for (a restart, an update).
    pub software: u32,
    pub brownout: u32,
    pub other: u32,
}

/// The task stacks one [`Memory`] lists.
pub const TASKS_MAX: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Memory<'a> {
    pub heap_free: u32,
    pub heap_min: u32,
    /// Each task's fewest free stack bytes since it started, as the RTOS
    /// reports them; empty where it reports none.
    #[serde(borrow)]
    pub tasks: [Option<TaskStack<'a>>; TASKS_MAX],
    /// The tasks the RTOS runs: more than `tasks` lists when they did not
    /// all fit.
    pub tasks_total: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskStack<'a> {
    /// At most 16 bytes.
    pub name: &'a str,
    pub free_min: u32,
}

/// The storage sectors [`Erases`] counts one by one, from the region's
/// first; the board's map says which store owns each. The region has grown
/// past them (to 48 sectors), but a message never changes a field's type,
/// so the later sectors' erases reach the wire only in a new variant.
pub const FLASH_SECTORS: usize = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flash {
    pub boot: Erases,
    /// Over the feeder's life; `None` as [`Counters::lifetime`].
    pub lifetime: Option<Erases>,
    /// Records the event ring holds, and the most it holds.
    pub events_kept: u32,
    pub events_capacity: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Erases {
    /// Each storage sector's erases, by its index in the region.
    pub storage: [u32; FLASH_SECTORS],
    /// Bank 1's sectors, erased by updates.
    pub update: u32,
}

/// Points in [`PowerStats::battery_mv`].
pub const POWER_HISTORY: usize = 24;

/// The supply since this boot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerStats {
    /// `None` before the supply was first sampled.
    pub adapter: Option<bool>,
    pub adapter_changes: u32,
    pub outages: u32,
    pub battery_s: u32,
    pub battery_min_mv: Option<u32>,
    pub battery_max_mv: Option<u32>,
    /// The battery's voltage at the end of each period, oldest first and
    /// the newest last; `None` where no reading was taken.
    pub battery_mv: [Option<u16>; POWER_HISTORY],
    pub period_s: u32,
}

/// The food beam since this boot. It is read while the emitters are on:
/// during feeds, and in a short window at rest on the adapter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Food {
    /// The beam's last reading: `Some(true)` with food, `None` before one.
    pub present: Option<bool>,
    /// The beam went from food to none.
    pub ran_low: u32,
    /// The beam went from none to food.
    pub refilled: u32,
    /// Feeds since the last refill this boot saw; `None` before one.
    pub feeds_since_refill: Option<u32>,
}

/// What a trace's current readings count in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CurrentUnit {
    /// The current channel scaled as stock scales it (the ADC's millivolts
    /// at the sense input): proportional to the motor current, but not yet
    /// calibrated to milliamps.
    #[default]
    Scaled,
    MilliAmps,
}

/// One feed's trace, summed up: what the drive measured, from every poll,
/// whether or not the body kept each reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceSummary {
    pub feed: u32,
    pub requested: u8,
    pub completed: u8,
    pub outcome: FeedOutcome,
    pub reason: FeedReason,
    /// How long the motor was commanded to move; 0 when it never started.
    pub drive_ms: u32,
    /// Polls that read the current while the motor was driven.
    pub samples: u32,
    pub unit: CurrentUnit,
    pub current_peak: u32,
    pub current_mean: Option<u32>,
    /// Feedback edges, rising and falling, and the first one's time from
    /// the drive's start.
    pub rising: u16,
    pub falling: u16,
    pub first_edge_ms: Option<u32>,
    /// Times a poll found the outlet beam blocked after finding it clear.
    pub beam_breaks: u16,
    /// Turns of the motor command away from the feed's own direction:
    /// one reverse-and-resume counts once, as the owner counts reverse
    /// attempts.
    pub reversals: u16,
    /// The battery pack just before the drive, and its lowest during it;
    /// `None` with no pack fitted, or no reading.
    pub battery_rest_mv: Option<u16>,
    pub battery_min_mv: Option<u16>,
    /// The body keeps every `stride`th current reading: 1 for each poll,
    /// doubling each time a full body halved them.
    pub stride: u16,
    /// The body ran out of room for marks and other entries, and ends
    /// before the drive did.
    pub truncated: bool,
}

/// One page of a trace's body ([`crate::trace`]): `bytes` from `offset`,
/// of `total` in all, and the trace's summary with each page, so a client
/// can tell when the trace it was reading has gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TracePage<'a> {
    pub summary: TraceSummary,
    /// The trace is in the feeder's storage, so it outlives a restart.
    pub stored: bool,
    pub offset: u16,
    pub total: u16,
    #[serde(borrow)]
    pub bytes: &'a [u8],
}

/// This boot's feeds, from their traces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedStats {
    /// Feeds traced.
    pub traced: u32,
    /// Their drive time, polls and current, as each summary counts them.
    pub drive_ms: u32,
    pub samples: u32,
    pub unit: CurrentUnit,
    pub current_peak: u32,
    pub current_mean: Option<u32>,
    pub rising: u32,
    pub falling: u32,
    pub beam_breaks: u32,
    pub reversals: u32,
    /// Traces the storage ring holds, and the most it is sure to hold.
    pub kept: u32,
    pub capacity: u32,
    /// Traces that never reached storage: the next feed's took the RAM
    /// trace first, or the write failed.
    pub unsaved: u32,
    /// The latest feed's summary.
    pub last: Option<TraceSummary>,
}

/// The portions since this boot: what the feedback sensor credited, what
/// the outlet beam saw, and what they weigh by the calibration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Portions {
    /// Portions credited, one a mark (as [`Counts::portions`]).
    pub portions: u32,
    /// Feeds that began with a homing mark, uncredited: each may have
    /// dropped part of a compartment more than it credited.
    pub homed: u32,
    /// Times the outlet beam was seen broken during feeds, sampled every
    /// 10 ms: a cross-check on the credited portions, and a lower bound
    /// (shorter passages fall between samples).
    pub outlet_breaks: u32,
    /// The calibration's weight of one portion, `None` before it is set.
    pub portion_mg: Option<u32>,
    /// `portions` times `portion_mg`: what was delivered by the
    /// calibration, never what the pet ate.
    pub delivered_mg: Option<u64>,
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::vec::Vec;

    use super::*;
    use crate::{decode, encode};

    #[test]
    fn the_largest_messages_fit_a_frame() {
        let label = "0123456789abcdef";
        let client = Client {
            key: [0xff; 32],
            label,
            role: Role::UpdateSigner,
            you: true,
        };
        let clients = DeviceMessage::Response {
            id: [0xff; 16],
            result: Outcome::Clients(Clients {
                clients: [Some(client); CLIENTS_MAX],
            }),
        };
        let mut out = [0; MESSAGE_MAX];
        let len = encode(&clients, &mut out).unwrap();
        assert_eq!(decode::<DeviceMessage<'_>>(&out[..len]), Ok(clients));

        let alerts = DeviceMessage::Response {
            id: [0xff; 16],
            result: Outcome::Feeder(FeederOutcome::Alerts(AlertsView {
                epoch: u32::MAX,
                generation: u32::MAX,
                more: true,
                settling: true,
                alerts: [Some(OpenAlert {
                    at: NonZeroU32::new(u32::MAX),
                    alert: Alert::MealFailed {
                        feed: u32::MAX,
                        plan: Some(u8::MAX),
                        requested: u8::MAX,
                        completed: u8::MAX,
                        outcome: FeedOutcome::Failed,
                        reason: FeedReason::Interrupted,
                    },
                }); ALERTS_MAX],
            })),
        };
        let len = encode(&alerts, &mut out).unwrap();
        assert_eq!(decode::<DeviceMessage<'_>>(&out[..len]), Ok(alerts));

        let wifi = ClientMessage::Request {
            id: [0xff; 16],
            op: Op::PutWifi {
                base: u32::MAX,
                wifi: Some(Wifi {
                    ssid: &[0xff; 32],
                    psk: "0123456789012345678901234567890123456789012345678901234567890ab",
                }),
            },
        };
        let len = encode(&wifi, &mut out).unwrap();
        assert_eq!(decode::<ClientMessage<'_>>(&out[..len]), Ok(wifi));

        // The largest offer: a version 2 manifest and a downgrade statement,
        // each well past the size the feeder accepts.
        let offer = ClientMessage::Request {
            id: [0xff; 16],
            op: Op::Update(UpdateOp::Offer {
                manifest: &[0xff; 200],
                downgrade: Some(&[0xff; 200]),
            }),
        };
        let len = encode(&offer, &mut out).unwrap();
        assert_eq!(decode::<ClientMessage<'_>>(&out[..len]), Ok(offer));

        let chunk = ClientMessage::Request {
            id: [0xff; 16],
            op: Op::Update(UpdateOp::Chunk {
                offset: u32::MAX,
                data: &[0xff; UPDATE_CHUNK_MAX],
            }),
        };
        let len = encode(&chunk, &mut out).unwrap();
        assert_eq!(decode::<ClientMessage<'_>>(&out[..len]), Ok(chunk));

        let view = DeviceMessage::Response {
            id: [0xff; 16],
            result: Outcome::Update(UpdateView {
                phase: UpdatePhase::Receiving,
                bank: 0,
                min_version: u32::MAX,
                digest: Some([0xff; 32]),
                version: u32::MAX,
                image_len: u32::MAX,
                written: u32::MAX,
                wanted: [Some(u32::MAX); WANTED_MAX],
                stopped: Some(UpdateStop::Store),
                bank0: Bank0Check::Blocked {
                    reason: Bank0Blocked::Read,
                },
            }),
        };
        let len = encode(&view, &mut out).unwrap();
        assert_eq!(decode::<DeviceMessage<'_>>(&out[..len]), Ok(view));

        let text = [b'x'; LOG_TEXT_MAX];
        let log = DeviceMessage::Log {
            seq: u32::MAX,
            dropped: u32::MAX,
            text: core::str::from_utf8(&text).unwrap(),
        };
        let len = encode(&log, &mut out).unwrap();
        assert_eq!(decode::<DeviceMessage<'_>>(&out[..len]), Ok(log));

        // The fullest batch: whole bytes, whatever the events.
        let bytes = [0xff; EVENTS_BYTES_MAX];
        let events = DeviceMessage::Events(EventBatch { encoded: &bytes });
        let len = encode(&events, &mut out).unwrap();
        assert_eq!(decode::<DeviceMessage<'_>>(&out[..len]), Ok(events));
        for stats in largest_stats() {
            let message = DeviceMessage::Response {
                id: [0xff; 16],
                result: Outcome::Stats(stats),
            };
            let len = encode(&message, &mut out).unwrap();
            assert_eq!(decode::<DeviceMessage<'_>>(&out[..len]), Ok(message));
        }

        let page = DeviceMessage::Response {
            id: [0xff; 16],
            result: Outcome::Feeder(FeederOutcome::FeedTrace(TracePage {
                summary: largest_summary(),
                stored: true,
                offset: u16::MAX,
                total: u16::MAX,
                bytes: &[0xff; crate::trace::PAGE_MAX],
            })),
        };
        let len = encode(&page, &mut out).unwrap();
        assert_eq!(decode::<DeviceMessage<'_>>(&out[..len]), Ok(page));

        let bank1 = DeviceMessage::Response {
            id: [0xff; 16],
            result: Outcome::Bank1(Bank1Page {
                offset: u32::MAX,
                total: u32::MAX,
                bytes: &[0xff; BANK1_PAGE_MAX],
            }),
        };
        let len = encode(&bank1, &mut out).unwrap();
        assert_eq!(decode::<DeviceMessage<'_>>(&out[..len]), Ok(bank1));
        // The bank is a whole number of pages.
        assert_eq!(0x15_0000 % BANK1_PAGE_MAX, 0);

        let plan = Some(Plan {
            minute: u16::MAX,
            portions: u8::MAX,
            days: u8::MAX,
            enabled: true,
            call: u8::MAX,
        });
        let put = ClientMessage::Request {
            id: [0xff; 16],
            op: Op::Feeder(FeederOp::PutPlans {
                base: u32::MAX,
                plans: [plan; PLANS_MAX],
            }),
        };
        let len = encode(&put, &mut out).unwrap();
        assert_eq!(decode::<ClientMessage<'_>>(&out[..len]), Ok(put));
        let plans = DeviceMessage::Response {
            id: [0xff; 16],
            result: Outcome::Feeder(FeederOutcome::Plans(Plans {
                generation: u32::MAX,
                writable: true,
                plans: [plan; PLANS_MAX],
                skipped: [Some(u32::MAX); PLANS_MAX],
            })),
        };
        let len = encode(&plans, &mut out).unwrap();
        assert_eq!(decode::<DeviceMessage<'_>>(&out[..len]), Ok(plans));

        let chunk = ClientMessage::Request {
            id: [0xff; 16],
            op: Op::Feeder(FeederOp::CallChunk {
                offset: u32::MAX,
                data: &[0xff; CALL_CHUNK_MAX],
            }),
        };
        let len = encode(&chunk, &mut out).unwrap();
        assert_eq!(decode::<ClientMessage<'_>>(&out[..len]), Ok(chunk));
        let call = DeviceMessage::Response {
            id: [0xff; 16],
            result: Outcome::Feeder(FeederOutcome::Call(CallView {
                writable: true,
                clip: CallClip::Receiving {
                    bytes: u32::MAX,
                    crc: u32::MAX,
                    received: u32::MAX,
                },
            })),
        };
        let len = encode(&call, &mut out).unwrap();
        assert_eq!(decode::<DeviceMessage<'_>>(&out[..len]), Ok(call));
    }

    /// A summary with every field at its largest encoding.
    fn largest_summary() -> TraceSummary {
        TraceSummary {
            feed: u32::MAX,
            requested: u8::MAX,
            completed: u8::MAX,
            outcome: FeedOutcome::Uncertain,
            reason: FeedReason::Output,
            drive_ms: u32::MAX,
            samples: u32::MAX,
            unit: CurrentUnit::MilliAmps,
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

    /// A summary fits the storage slot's header with room to spare.
    #[test]
    fn a_summary_fits_its_storage_header() {
        let mut out = [0; 96];
        let len = encode(&largest_summary(), &mut out).unwrap();
        assert!(len <= 80, "{len}");
    }

    /// Every section at its widest: each field at its largest encoding.
    fn largest_stats() -> [Stats<'static>; 7] {
        let counts = Counts {
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
        let resets = Resets {
            power_on: u32::MAX,
            watchdog: u32::MAX,
            software: u32::MAX,
            brownout: u32::MAX,
            other: u32::MAX,
        };
        let erases = Erases {
            storage: [u32::MAX; FLASH_SECTORS],
            update: u32::MAX,
        };
        let task = TaskStack {
            name: "0123456789abcdef",
            free_min: u32::MAX,
        };

        [
            Stats::Counters(Counters {
                boot: counts,
                reset: ResetCause::Brownout,
                lifetime: Some(counts),
                resets: Some(resets),
                handshake_max_ms: u32::MAX,
                handshake_mean_ms: u32::MAX,
            }),
            Stats::Memory(Memory {
                heap_free: u32::MAX,
                heap_min: u32::MAX,
                tasks: [Some(task); TASKS_MAX],
                tasks_total: u32::MAX,
            }),
            Stats::Flash(Flash {
                boot: erases,
                lifetime: Some(erases),
                events_kept: u32::MAX,
                events_capacity: u32::MAX,
            }),
            Stats::Power(PowerStats {
                adapter: Some(true),
                adapter_changes: u32::MAX,
                outages: u32::MAX,
                battery_s: u32::MAX,
                battery_min_mv: Some(u32::MAX),
                battery_max_mv: Some(u32::MAX),
                battery_mv: [Some(u16::MAX); POWER_HISTORY],
                period_s: u32::MAX,
            }),
            Stats::Food(Food {
                present: Some(true),
                ran_low: u32::MAX,
                refilled: u32::MAX,
                feeds_since_refill: Some(u32::MAX),
            }),
            Stats::Feeds(FeedStats {
                traced: u32::MAX,
                drive_ms: u32::MAX,
                samples: u32::MAX,
                unit: CurrentUnit::MilliAmps,
                current_peak: u32::MAX,
                current_mean: Some(u32::MAX),
                rising: u32::MAX,
                falling: u32::MAX,
                beam_breaks: u32::MAX,
                reversals: u32::MAX,
                kept: u32::MAX,
                capacity: u32::MAX,
                unsaved: u32::MAX,
                last: Some(largest_summary()),
            }),
            Stats::Portions(Portions {
                portions: u32::MAX,
                homed: u32::MAX,
                outlet_breaks: u32::MAX,
                portion_mg: Some(u32::MAX),
                delivered_mg: Some(u64::MAX),
            }),
        ]
    }

    /// A kind's op says whether it writes through its module.
    #[test]
    fn a_feeder_op_writes_as_its_module_says() {
        assert!(Op::Feeder(FeederOp::Feed { portions: 1 }).writes());
        assert!(!Op::Feeder(FeederOp::GetPlans).writes());
        assert!(!Op::Status.writes());
        assert!(Op::Reboot.writes());
    }

    /// A writer filled with the largest events until it has no room takes
    /// every one whole, its message fits a frame, and they read back in
    /// order.
    #[test]
    fn a_full_event_batch_fits_a_frame_and_reads_back() {
        let event = |seq| Event {
            seq,
            at: Some(u32::MAX),
            body: EventBody::MealSoon {
                plan: u8::MAX,
                due: u32::MAX,
                minute: u16::MAX,
                portions: u8::MAX,
            },
        };
        let mut buf = [0; FRAME_MAX];
        let mut writer = EventBatchWriter::new(&mut buf);
        let mut seq = u64::MAX - 100;
        while writer.has_room() {
            writer.push(&event(seq)).unwrap();
            seq += 1;
        }
        let written = seq - (u64::MAX - 100);
        assert!(written >= 30, "{written} events");

        let message = DeviceMessage::Events(writer.batch());
        let mut out = [0; MESSAGE_MAX];
        let len = encode(&message, &mut out).unwrap();
        let Ok(DeviceMessage::Events(batch)) = decode::<DeviceMessage<'_>>(&out[..len]) else {
            panic!("not a batch");
        };
        let read: Vec<_> = batch.iter().map(Result::unwrap).collect();
        let expected: Vec<_> = (u64::MAX - 100..seq).map(event).collect();
        assert_eq!(read, expected);
    }

    /// Bytes that are not events yield one error, then nothing.
    #[test]
    fn a_garbled_batch_fails_once() {
        let mut buf = [0; 64];
        let mut writer = EventBatchWriter::new(&mut buf);
        writer
            .push(&Event {
                seq: 1,
                at: None,
                body: EventBody::Lost { count: 1 },
            })
            .unwrap();
        let mut bytes = writer.batch().as_bytes().to_vec();
        bytes.push(0xff);
        let batch = EventBatch { encoded: &bytes };
        let read: Vec<_> = batch.iter().collect();
        assert_eq!(read.len(), 2);
        assert!(read[0].is_ok());
        assert_eq!(read[1], Err(crate::Error::Malformed));
    }

    /// The event ring stores a body in 16 bytes.
    #[test]
    fn every_event_body_fits_sixteen_bytes() {
        let client = [0xff; 4];
        for body in [
            EventBody::Boot {
                bank: 1,
                reset_flags: u32::MAX,
            },
            EventBody::Lock {
                locked: true,
                by: Source::Client(client),
            },
            EventBody::Settings {
                generation: u32::MAX,
                by: Source::Client(client),
            },
            EventBody::Wifi {
                generation: u32::MAX,
                by: Source::Client(client),
            },
            EventBody::Enrolled {
                client,
                role: Role::UpdateSigner,
                path: Path::Recovery,
            },
            EventBody::Revoked {
                client,
                by: Source::Client(client),
            },
            EventBody::ClockSync {
                by: Source::Client(client),
                jump_ms: Some(i32::MIN),
            },
            EventBody::Lost { count: u16::MAX },
            EventBody::Fed {
                feed: u32::MAX,
                requested: u8::MAX,
                completed: u8::MAX,
                outcome: FeedOutcome::Uncertain,
                reason: FeedReason::Output,
                hopper: FoodLevel::Unread,
                by: Source::Client(client),
            },
            EventBody::FeedingBlocked {
                reason: Some(Blocked::Unserved),
            },
            EventBody::Supply(Supply {
                adapter: Some(false),
                battery: Battery::Stale { age_ms: u32::MAX },
                below_floor: Some(true),
            }),
            EventBody::Supply(Supply {
                adapter: Some(true),
                battery: Battery::Implausible { mv: u16::MAX },
                below_floor: Some(false),
            }),
            EventBody::Supply(Supply {
                adapter: Some(false),
                battery: Battery::Reading {
                    mv: u16::MAX,
                    percent: u8::MAX,
                },
                below_floor: Some(true),
            }),
            EventBody::Fed {
                feed: u32::MAX,
                requested: u8::MAX,
                completed: u8::MAX,
                outcome: FeedOutcome::Uncertain,
                reason: FeedReason::Output,
                hopper: FoodLevel::Low,
                by: Source::Schedule(u8::MAX),
            },
            EventBody::Plans {
                generation: u32::MAX,
                by: Source::Client(client),
            },
            EventBody::Missed {
                plan: u8::MAX,
                due: u32::MAX,
                reason: MissedReason::Blocked(Blocked::Unserved),
            },
            EventBody::DispenseRefused {
                reason: Blocked::Battery,
            },
            EventBody::MealSoon {
                plan: u8::MAX,
                due: u32::MAX,
                minute: u16::MAX,
                portions: u8::MAX,
            },
            EventBody::Food {
                level: FoodLevel::Low,
            },
            EventBody::Skipped {
                plan: u8::MAX,
                due: u32::MAX,
                by: Source::Client(client),
            },
            EventBody::Unskipped {
                plan: u8::MAX,
                due: u32::MAX,
                by: Source::Client(client),
            },
            EventBody::UpdateStop {
                stop: UpdateStop::Store,
            },
            EventBody::UpdateAtBoot {
                candidate: UpdateCandidate::Activated,
            },
            EventBody::Restarting,
        ] {
            let mut out = [0; EVENT_BODY_MAX];
            let len = encode(&body, &mut out).unwrap();
            assert_eq!(decode::<EventBody>(&out[..len]), Ok(body));

            // The largest event around the largest body.
            let event = Event {
                seq: u64::MAX,
                at: Some(u32::MAX),
                body,
            };
            let mut out = [0; EVENT_BYTES_MAX];
            let len = encode(&event, &mut out).unwrap();
            assert_eq!(decode::<Event>(&out[..len]), Ok(event));
        }
    }
}
