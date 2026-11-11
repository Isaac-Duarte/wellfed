//! Bank 0's install of one admitted offer, sans-IO: which chunks it wants,
//! what each chunk does, and when it stops.
//!
//! The client pushes chunks over the device API, so a chunk may come twice
//! (a retry after a lost answer, or a state read just before the chunk
//! landed). A chunk the install already holds is answered and dropped; a
//! chunk it does not want, or one of the wrong length, stops the install,
//! as an out-of-order answer did when the updater pulled. Every other rule
//! is the [`Writer`]'s: sectors in order, each read back, the whole image's
//! SHA-256 checked, and the marker handed out last.
//!
//! The port runs each [`Program`] inside a flash lease and reports its
//! readback; on [`Install::ready`] it records the activation in the update
//! store, then programs the marker and reads it back.

use super::{MAX_CHUNK, Program, WriteError, Writer};
use crate::store::update::UpdateError;
use wellfed_signed::SECTOR_BYTES;
use wellfed_signed::offer::{Admitted, KeyStatement, OfferError, StatementError};

/// A minute without a chunk ends the install with bank 1 invalid.
pub const IDLE_MS: u32 = 60_000;
/// Chunks wanted at once: one sector's.
pub const WANTED: usize = (SECTOR_BYTES / MAX_CHUNK) as usize;

/// Why an install stopped before activation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    Timeout,
    Cancelled,
    /// A chunk the install did not want, or of the wrong length.
    Order,
    /// The image does not start with the loader's marker.
    Marker,
    /// A sector did not read back.
    Write,
    /// The whole image's SHA-256 differs from the manifest's.
    Hash,
    /// The marker was never programmed: its write could not start (the
    /// flash was not armed). A marker programmed but not read back
    /// activates ([`Marking`]).
    Activate,
    /// The activation could not be recorded.
    Store,
}

crate::named_enum! {
    /// The `stage=` word of an install's stop record (`PLOTA1 error`). Not
    /// one word per [`Stop`]: the words predate the variants, and the bench
    /// matches the words.
    pub enum StopStage {
        Timeout = "timeout",
        Cancelled = "cancelled",
        Order = "order",
        /// [`Stop::Marker`]: the image lacks the loader's marker.
        Image = "image",
        Write = "write",
        Hash = "hash",
        /// [`Stop::Activate`]: the marker was never programmed.
        Marker = "marker",
        Store = "store",
    }
}

impl Stop {
    pub const fn stage(self) -> StopStage {
        match self {
            Self::Timeout => StopStage::Timeout,
            Self::Cancelled => StopStage::Cancelled,
            Self::Order => StopStage::Order,
            Self::Marker => StopStage::Image,
            Self::Write => StopStage::Write,
            Self::Hash => StopStage::Hash,
            Self::Activate => StopStage::Marker,
            Self::Store => StopStage::Store,
        }
    }

    fn of(error: WriteError) -> Self {
        match error {
            WriteError::Order | WriteError::State => Self::Order,
            WriteError::Marker => Self::Marker,
            WriteError::Readback => Self::Write,
            WriteError::Hash => Self::Hash,
        }
    }
}

/// Where an update stands, as the feeder publishes it to its clients.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    /// An offer's signatures or bank 0's image are being checked.
    Checking,
    /// Bank 1 recorded the offer and restarts into bank 0.
    GivingWay,
    Receiving,
    /// Bank 1's marker is written; the feeder restarts into it.
    Activated,
}

/// The update's state, published after every step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub phase: Phase,
    pub bank: u8,
    pub min_version: u32,
    pub digest: Option<[u8; 32]>,
    pub version: u32,
    pub image_len: u32,
    pub written: u32,
    pub wanted: [Option<u32>; WANTED],
    /// Why the last install stopped, if it did.
    pub stopped: Option<Stop>,
    /// Bank 0 as bank 1 last checked it; always [`Bank0Check::Unchecked`]
    /// on bank 0, which records itself instead.
    pub bank0: Bank0Check,
    /// What the update store says about a candidate.
    pub candidate: CandidateState,
}

/// What the update store holds, which outlives a restart. Both states are
/// written before the restart that needs them: a give-way's `Pending` before
/// bank 1's marker is cleared, an activation's `Activated` before its marker
/// is programmed. So the next boot can tell how it got there. A store that
/// can't be read shows `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CandidateState {
    #[default]
    None,
    /// Bank 1 gave way for it, or bank 0 accepted it.
    Pending,
    /// Bank 0 activated it, and no build has confirmed it.
    Activated,
}

impl Progress {
    pub const fn idle(bank: u8, min_version: u32) -> Self {
        Self {
            phase: Phase::Idle,
            bank,
            min_version,
            digest: None,
            version: 0,
            image_len: 0,
            written: 0,
            wanted: [None; WANTED],
            stopped: None,
            bank0: Bank0Check::Unchecked,
            candidate: CandidateState::None,
        }
    }

    /// The same, naming the manifest `admitted`.
    pub const fn of(self, phase: Phase, admitted: &Admitted) -> Self {
        Self {
            phase,
            digest: Some(admitted.digest),
            version: admitted.signed.manifest.version,
            image_len: admitted.signed.manifest.image_len,
            ..self
        }
    }
}

crate::named_enum! {
    /// Why bank 1 does not make way for bank 0: the check that failed, in
    /// the order bank 1 runs them. Bank 1 gives way only to a bank 0 that
    /// booted, recorded its own image in the update store and still holds
    /// exactly that image, so the build it hands over to can install. The
    /// words are the `reason=` of `PLWCH1 refused give_way` and the
    /// `check=` of `PLOTA1 bank0`.
    pub enum Blocked {
        /// Bank 0's loader marker is not valid.
        Marker = "marker",
        /// Bank 0 never recorded itself: it was written over UART (or
        /// the store was reset) and has not booted since.
        Unrecorded = "unrecorded",
        /// Bank 0's built-in update key is not the one the offer leans on.
        Key = "key",
        /// Bank 0's image headers do not give an image to hash.
        Extent = "extent",
        /// Bank 0's image differs from what it recorded, in length or hash.
        Changed = "changed",
        /// Bank 0 could not be read while hashing it.
        Read = "read",
    }
}

crate::named_enum! {
    /// Why bank 0 recorded no image at boot (`PLOTA1 bank0 recorded=0
    /// error=…`).
    pub enum Bank0Error {
        /// Its headers do not give an image to hash.
        Extent = "extent",
        /// It could not be read while hashing.
        Read = "read",
        /// The update store could not be written.
        Store = "store",
    }
}

/// Bank 0 as bank 1 last checked it: once per boot, and at every offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bank0Check {
    /// Not checked yet this boot (or the store could not be read).
    Unchecked,
    /// Bank 0 holds the image it recorded.
    Match,
    /// Bank 1 would not give way to it.
    Blocked(Blocked),
}

impl Bank0Check {
    /// The `check=` word of `PLOTA1 bank0`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unchecked => "unchecked",
            Self::Match => "match",
            Self::Blocked(blocked) => blocked.as_str(),
        }
    }

    /// The check that prints as `text`, as a host reads a record back.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "unchecked" => Some(Self::Unchecked),
            "match" => Some(Self::Match),
            _ => Blocked::parse(text).map(Self::Blocked),
        }
    }
}

/// Why an update request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    Offer(OfferError),
    Statement(StatementError),
    Store(UpdateError),
    /// The store cannot be read: no floor to check against.
    Unreadable,
    /// The mailbox or the flash is not free yet.
    Busy,
    /// Bank 1 does not make way for bank 0, for this reason.
    Fallback(Blocked),
    /// Bank 0 installs only the manifest bank 1 gave way for.
    Pending,
    /// This manifest was activated and failed its trial.
    Failed,
    /// An install reset the feeder this power cycle.
    Attempt,
    /// Another update is in progress.
    Installing,
    NotInstalling,
}

crate::named_enum! {
    /// The `reason=` word of a refusal record (`PLOTA1 refused`). Several
    /// refusals share a word: the bench tells them apart by what it sent.
    pub enum RefusedReason {
        Downgrade = "downgrade",
        Same = "same",
        Key = "key",
        Signature = "signature",
        Statement = "statement",
        Manifest = "manifest",
        Store = "store",
        Busy = "busy",
        Fallback = "fallback",
        Pending = "pending",
        Failed = "failed",
        Attempt = "attempt",
        Installing = "installing",
        NotInstalling = "not-installing",
        /// Signed for another device model.
        Model = "model",
    }
}

impl Refused {
    pub const fn reason(self) -> RefusedReason {
        match self {
            Self::Offer(OfferError::Downgrade) => RefusedReason::Downgrade,
            Self::Offer(OfferError::Same) => RefusedReason::Same,
            Self::Offer(OfferError::Key(_)) | Self::Statement(_) => RefusedReason::Key,
            Self::Offer(OfferError::Signature) => RefusedReason::Signature,
            Self::Offer(OfferError::Statement(_)) => RefusedReason::Statement,
            Self::Offer(OfferError::Model) => RefusedReason::Model,
            Self::Offer(_) => RefusedReason::Manifest,
            Self::Store(_) | Self::Unreadable => RefusedReason::Store,
            Self::Busy => RefusedReason::Busy,
            Self::Fallback(_) => RefusedReason::Fallback,
            Self::Pending => RefusedReason::Pending,
            Self::Failed => RefusedReason::Failed,
            Self::Attempt => RefusedReason::Attempt,
            Self::Installing => RefusedReason::Installing,
            Self::NotInstalling => RefusedReason::NotInstalling,
        }
    }
}

/// What an update request came to, as the update stage answers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Progress(Progress),
    /// A key statement took effect.
    Key(KeyStatement),
}

/// What became of bank 1's marker, as the activation left it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marking {
    /// Programmed and read back: bank 1 is valid.
    ReadBack,
    /// Programmed, but the program reported a failure or the readback
    /// failed or differed. The cells may hold the marker all the same (a
    /// stale read, a program that wrote and then reported a fault), so bank
    /// 1 may be valid: the activation stands, and the restart's boot
    /// decides it. Never treated as never activated, which would leave a
    /// build that boots from that marker below the floor.
    Uncertain,
    /// Never programmed (the flash was not armed for it): bank 1 stays
    /// invalid.
    Unwritten,
}

/// What one chunk did.
#[derive(Debug, PartialEq, Eq)]
pub enum Taken<'a> {
    /// Staged; more of the sector is wanted.
    Staged,
    /// Already held: answered, changed nothing.
    Held,
    /// The sector is complete: program it, then call [`Install::programmed`].
    Program(Program<'a>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Receiving,
    /// Every sector is written and the hash matched: the marker is next.
    Ready,
    Activated,
    Stopped(Stop),
}

pub struct Install {
    admitted: Admitted,
    writer: Writer,
    phase: State,
    /// When the last chunk came (or the install started).
    heard_ms: u32,
}

impl Install {
    /// Start installing `admitted` at `now_ms`. `None` only if the writer
    /// cannot take [`MAX_CHUNK`] chunks, which a constant rules out.
    pub fn new(admitted: Admitted, now_ms: u32) -> Option<Self> {
        Some(Self {
            writer: Writer::new(admitted.signed.manifest, MAX_CHUNK as usize)?,
            admitted,
            phase: State::Receiving,
            heard_ms: now_ms,
        })
    }

    pub fn admitted(&self) -> &Admitted {
        &self.admitted
    }

    /// Image bytes written and read back.
    pub fn written(&self) -> u32 {
        self.writer.written()
    }

    /// Why the install stopped: the port's reason, or the writer's.
    pub fn stopped(&self) -> Option<Stop> {
        match self.phase {
            State::Stopped(stop) => Some(stop),
            _ => self.writer.failed().map(Stop::of),
        }
    }

    /// Every byte is written and verified: record the activation, then
    /// program the marker.
    pub fn ready(&self) -> bool {
        self.phase == State::Ready
    }

    pub fn activated(&self) -> bool {
        self.phase == State::Activated
    }

    /// Still taking chunks.
    pub fn receiving(&self) -> bool {
        self.phase == State::Receiving && self.writer.failed().is_none()
    }

    /// The state, as published.
    pub fn progress(&self, bank: u8, min_version: u32) -> Progress {
        let phase = if self.activated() {
            Phase::Activated
        } else if self.stopped().is_some() {
            Phase::Idle
        } else {
            Phase::Receiving
        };
        Progress {
            written: self.written(),
            wanted: self.wanted(),
            stopped: self.stopped(),
            ..Progress::idle(bank, min_version).of(phase, &self.admitted)
        }
    }

    /// The chunk offsets wanted now, in order.
    pub fn wanted(&self) -> [Option<u32>; WANTED] {
        let mut out = [None; WANTED];
        if self.receiving() {
            for (slot, (offset, _)) in out.iter_mut().zip(self.writer.missing()) {
                *slot = Some(offset);
            }
        }
        out
    }

    fn stop(&mut self, stop: Stop) -> Stop {
        if let Some(earlier) = self.stopped() {
            return earlier;
        }
        if self.phase != State::Activated {
            self.phase = State::Stopped(stop);
        }
        stop
    }

    /// Stop now: cancelled by the owner, or the store failed. Bank 1's
    /// marker is never written after this.
    pub fn abort(&mut self, stop: Stop) {
        self.stop(stop);
    }

    /// End a quiet install once [`IDLE_MS`] passed without a chunk.
    pub fn tick(&mut self, now_ms: u32) -> Option<Stop> {
        let quiet = now_ms.wrapping_sub(self.heard_ms) >= IDLE_MS;
        (self.receiving() && quiet).then(|| self.stop(Stop::Timeout))
    }

    /// One chunk at `offset`.
    pub fn chunk(&mut self, offset: u32, data: &[u8], now_ms: u32) -> Result<Taken<'_>, Stop> {
        if let Some(stop) = self.stopped() {
            return Err(stop);
        }
        self.heard_ms = now_ms;
        if self.phase != State::Receiving || self.writer.has(offset) {
            return Ok(Taken::Held);
        }

        // The writer keeps its own failure, which `stopped` reports.
        match self.writer.accept(offset, data).map_err(Stop::of)? {
            None => Ok(Taken::Staged),
            Some(program) => Ok(Taken::Program(program)),
        }
    }

    /// The staged sector's readback. Once the last one passes the whole
    /// image's hash, the install is [`Self::ready`].
    pub fn programmed(&mut self, readback_ok: bool) -> Result<(), Stop> {
        if let Err(error) = self.writer.programmed(readback_ok) {
            return Err(self.stop(Stop::of(error)));
        }
        if self.writer.next_request().is_none() {
            self.phase = State::Ready;
        }
        Ok(())
    }

    /// The marker to program last, as a bank-1 offset, once the install is
    /// ready and its activation recorded.
    pub fn commit(&mut self) -> Result<(u32, [u8; 8]), Stop> {
        if self.phase != State::Ready {
            return Err(self.stop(Stop::Order));
        }
        self.writer
            .commit()
            .map_err(|error| self.stop(Stop::of(error)))
    }

    /// What the marker's program came to ([`Marking`]): activated once it
    /// was programmed, whether or not it read back, and stopped only when
    /// it never was.
    pub fn marked(&mut self, marking: Marking) -> Result<(), Stop> {
        if self.phase != State::Ready {
            return Err(self.stop(Stop::Order));
        }
        if marking == Marking::Unwritten {
            return Err(self.stop(Stop::Activate));
        }
        self.phase = State::Activated;
        Ok(())
    }
}
