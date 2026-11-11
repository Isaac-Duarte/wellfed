//! The server's slots, the driver's surface and the pass that steps them.
//!
//! Each kind of work is an `impl Server` block in its own file, so all of
//! them share the slots' private state: opening a session (`handshake`),
//! the recovery window (`recovery`), an open session's turn (`session`),
//! one request (`request`) and the feeder's module it routes to (`feeder`),
//! writes the device finishes later (`pending`)
//! and the write ids a retry is answered from (`memory`).

mod feeder;
pub(crate) mod handshake;
mod memory;
mod pending;
mod recovery;
pub(crate) mod request;
mod samples;
mod session;
mod wire;

use self::feeder::FeederWait;
use self::memory::Memory;
use self::recovery::Rescue;
use self::samples::Samples;
use self::wire::Wire;
use crate::clients::{CLIENTS, Client, Label, Role};
use crate::pairing::{self, Pairing, Path};
use crate::setup::Responder;
use crate::{BUFFER, Device, Reader, Reason, Record, SESSIONS};
use wellfed_core::deadline_passed;
use wellfed_proto::api::{
    self as proto, DeviceMessage, FRAME_MAX, Open, OpenRefusal, Refusal, RequestId, Sample,
};

use wellfed_session::noise::{Handshake, Keypair, Transport};

/// One pass's inputs from the loop.
pub struct Pass<'a, D> {
    pub now_ms: u32,
    pub device: &'a mut D,
    pub pairing: &'a mut Pairing,
    pub identity: Option<&'a Keypair>,
    /// A feed holds the outputs: only an IK session handshake may run this
    /// pass, a refused one closes unanswered, and after one refused or
    /// failed the rest wait for the feed's end (see [`Server::handshook`]).
    pub feeding: bool,
    /// Pairing's decision on a LAN recovery offer this pass: its code keyed
    /// in, or its timeout.
    pub recovery: Option<pairing::Event>,
    /// Pairing's decision on an update-key offer this pass: a Dispense press
    /// admitted it, or nobody pressed in time.
    pub signer: Option<pairing::Event>,
}

/// A write the device is still completing, as its client's memory keeps
/// it from the moment it is queued (`Remembered::Waiting`): polled by the
/// session that asked while it stays, and by the pass once it has gone, so
/// a retry on any session waits for the same write rather than running it
/// again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wait {
    /// A Wi-Fi change, which finishes as change `change` of this boot
    /// (`Saved::next` when it was queued).
    Wifi {
        change: u32,
    },
    /// The digest of the settings asked for: the write succeeded if the
    /// store holds them once nothing is pending.
    Time {
        digest: u32,
    },
    Keys {
        ticket: u32,
    },
    /// A write of the feeder's module.
    Feeder(FeederWait),
    Update {
        ticket: u32,
    },
}

/// What a session's pending request waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Waiting {
    /// The write it queued.
    Write(Wait),
    /// A retry of a write still under way: its answer, once the client's
    /// memory holds it.
    Retry,
    /// An update key waits for its Dispense press. The session's own: the
    /// offer is cancelled when it ends.
    Signer { key: [u8; 32], label: Label },
    /// The pressed key's enrollment is queued.
    SignerKeys { ticket: u32, key: [u8; 32] },
}

#[derive(Clone, Copy)]
struct Pending {
    id: RequestId,
    op: &'static str,
    waiting: Waiting,
}

enum Phase {
    Free,
    /// Connected; the clear [`Open`] frame comes first.
    Opening,
    /// The connection asked for an invite; the handshake starts next turn.
    Starting(Open),
    Handshake {
        handshake: Handshake,
        open: Open,
        /// The client's message is read; ours is due.
        reply: bool,
        /// The IK client's hello, once read.
        refusal: Option<OpenRefusal>,
    },
    Open {
        transport: Transport,
        key: [u8; 32],
        role: Role,
        memory: usize,
        pending: Option<Pending>,
        /// Push events after this cursor (`Some(None)`: from the oldest).
        events: Option<Option<u64>>,
        /// The live samples the session subscribed to.
        samples: proto::Samples,
        /// Samples waiting to go out, one of each kind.
        sample: Samples,
        /// Push log lines after this cursor (`Some(None)`: from the
        /// oldest), once the session asked.
        log: Option<Option<u32>>,
    },
    /// An invite's handshake is done: enroll the key, then answer.
    Joining {
        transport: Transport,
        key: [u8; 32],
        label: Label,
        role: Role,
        ticket: Option<u32>,
    },
    /// The recovery window's session: the setup exchange over this
    /// stream, so the owner keys the client's code in before it is enrolled.
    Recovery(Rescue),
    /// Send what is left, then close.
    Closing,
}

struct Slot {
    phase: Phase,
    since_ms: u32,
    wire: Wire,
}

impl Slot {
    const FREE: Self = Self {
        phase: Phase::Free,
        since_ms: 0,
        wire: Wire::EMPTY,
    };
}

pub struct Server {
    slots: [Slot; SESSIONS],
    memories: [Memory; SESSIONS],
    clients: [Option<Client>; CLIENTS],
    clients_generation: u32,
    /// Where the next handshake turn starts looking, so no slot starves.
    turn: usize,
    pass: u32,
    reboot_at: Option<u32>,
    /// The one recovery session's responder; its buffer would not fit a slot.
    rescue: Responder,
    /// Pairing's decision on the recovery offer this pass.
    decision: Option<pairing::Event>,
    /// A recovery client left with its offer pending: cancel it next pass.
    cancel_rescue: bool,
    /// Pairing's decision on an update-key offer, until its session takes it.
    signer: Option<pairing::Event>,
    /// A session waiting for an update key's press ended: cancel the offer.
    cancel_signer: bool,
    /// The client table changed: drop the invites of every key gone from
    /// it at the next pass, before any handshake can redeem one.
    prune_invites: bool,
    /// Subscribers that ended since the last pass, by slot: the device
    /// hears of each on the next.
    readers_left: [bool; SESSIONS],
    /// A feed is prepared or driven this pass.
    feeding: bool,
    /// The last step ran a handshake operation.
    handshook: bool,
    /// A session handshake was refused or failed during this feed: the rest
    /// wait for its end.
    struck: bool,
    /// The feeder's identity key, for an offer's downgrade statement.
    identity_public: Option<[u8; 32]>,
    /// The length of the request in `plain`, which update ops read again
    /// for their bytes rather than copy them.
    plain_len: usize,
    /// A message's plaintext, and the next one's encoding: here rather than
    /// on the loop's stack.
    plain: [u8; FRAME_MAX],
    out: [u8; FRAME_MAX],
}

impl core::fmt::Debug for Server {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Server")
            .field("sessions", &self.sessions())
            .finish_non_exhaustive()
    }
}

impl Server {
    /// Built at compile time: the table is about 10 KB, more than the loop
    /// task's stack.
    pub const fn new() -> Self {
        Self {
            slots: [Slot::FREE, Slot::FREE, Slot::FREE, Slot::FREE],
            memories: [Memory::EMPTY; SESSIONS],
            clients: [None; CLIENTS],
            clients_generation: 0,
            turn: 0,
            pass: 0,
            reboot_at: None,
            rescue: Responder::idle(),
            decision: None,
            cancel_rescue: false,
            signer: None,
            cancel_signer: false,
            prune_invites: false,
            readers_left: [false; SESSIONS],
            feeding: false,
            handshook: false,
            struck: false,
            identity_public: None,
            plain_len: 0,
            plain: [0; FRAME_MAX],
            out: [0; FRAME_MAX],
        }
    }

    /// The table to refresh from the writer, and its generation; call
    /// [`Self::clients_changed`] after refreshing it.
    pub fn clients_table(&mut self) -> (&mut [Option<Client>; CLIENTS], u32) {
        (&mut self.clients, self.clients_generation)
    }

    /// The table changed: close every session whose key is gone, and
    /// (at the next pass) drop the invites it made.
    pub fn clients_changed(&mut self, generation: u32, mut record: impl FnMut(Record)) {
        self.clients_generation = generation;
        self.prune_invites = true;
        for index in 0..SESSIONS {
            let Phase::Open { key, .. } = self.slots[index].phase else {
                continue;
            };
            if !self.clients.iter().flatten().any(|c| c.key == key) {
                self.end(index, Reason::Revoked, &mut record);
            }
        }
    }

    /// Open sessions and connections still opening.
    pub fn sessions(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| !matches!(slot.phase, Phase::Free))
            .count()
    }

    /// A new connection: its slot, or `None` to refuse it (close at once).
    pub fn accept(&mut self, now_ms: u32, mut record: impl FnMut(Record)) -> Option<usize> {
        let Some(index) = self
            .slots
            .iter()
            .position(|slot| matches!(slot.phase, Phase::Free))
        else {
            record(Record::Ended {
                slot: u8::MAX,
                reason: Reason::Full,
            });
            return None;
        };
        let slot = &mut self.slots[index];
        *slot = Slot::FREE;
        slot.phase = Phase::Opening;
        slot.since_ms = now_ms;
        record(Record::Accepted {
            slot: slot_id(index),
        });
        Some(index)
    }

    /// Room for received bytes in `slot`.
    pub fn rx(&mut self, slot: usize) -> &mut [u8] {
        let slot = &mut self.slots[slot].wire;
        &mut slot.rx[slot.rx_len..]
    }

    pub fn received(&mut self, slot: usize, len: usize) {
        let slot = &mut self.slots[slot].wire;
        slot.rx_len = (slot.rx_len + len).min(BUFFER);
    }

    /// Bytes waiting to be sent on `slot`.
    pub fn tx(&self, slot: usize) -> &[u8] {
        let slot = &self.slots[slot].wire;
        &slot.tx[slot.tx_sent..slot.tx_len]
    }

    pub fn sent(&mut self, slot: usize, len: usize) {
        self.slots[slot].wire.sent(len);
    }

    /// The driver should close `slot` now: it ended and its last bytes are
    /// out. Call [`Self::closed`] once it has.
    pub fn closing(&self, slot: usize) -> bool {
        let slot = &self.slots[slot];
        matches!(slot.phase, Phase::Closing) && slot.wire.tx_idle()
    }

    /// The connection is gone (the client hung up, or the driver closed it).
    pub fn closed(&mut self, slot: usize, mut record: impl FnMut(Record)) {
        match self.slots[slot].phase {
            Phase::Free => {}
            Phase::Closing => self.slots[slot] = Slot::FREE,
            _ => {
                self.end(slot, Reason::Closed, &mut record);
                self.slots[slot] = Slot::FREE;
            }
        }
    }

    /// The feed `feed` was accepted but ended refused before it was reserved:
    /// every request that names it hears `refusal`, the sessions still waiting
    /// and the ids remembered for a client that has gone. Told at once, since
    /// the next feed takes the same number, and asking the device after that
    /// would answer for someone else's feed.
    pub fn feed_refused(&mut self, feed: u32, refusal: Refusal) {
        for slot in &mut self.slots {
            if let Phase::Open {
                pending: Some(pending),
                ..
            } = &mut slot.phase
                && matches!(pending.waiting, Waiting::Write(Wait::Feeder(FeederWait::Feed { feed: waited })) if waited == feed)
            {
                pending.waiting = Waiting::Write(Wait::Feeder(FeederWait::FeedRefused(refusal)));
            }
        }
        for memory in &mut self.memories {
            memory.refuse_queued(feed, refusal);
        }
    }

    /// A session ended with a write pending. One that waits for an update
    /// key's press: the next pass cancels the offer, so the next Dispense
    /// press feeds again. A client store change it queued (a revocation, a
    /// pressed key, its own key from an invite or recovery) lands anyway,
    /// and the settings writer records its event in the same turn as the
    /// store write ([`KeyChange::event`](crate::KeyChange::event)), so
    /// nothing is left to record here.
    fn forget_waiting(&mut self, index: usize) {
        if let Phase::Open {
            pending:
                Some(Pending {
                    waiting: Waiting::Signer { .. },
                    ..
                }),
            ..
        } = self.slots[index].phase
        {
            self.cancel_signer = true;
        }
    }

    /// Tell the device of each subscriber that ended since the last pass.
    /// Out of line so its loop does not grow `step`'s frame, which every
    /// session's deepest path sits under.
    #[inline(never)]
    fn tell_readers_left<D: Device>(&mut self, device: &mut D) {
        for (index, left) in self.readers_left.iter_mut().enumerate() {
            if core::mem::take(left) {
                device.reader_left(Reader::slot(index));
            }
        }
    }

    /// A live sample for subscribers that asked for samples; one not
    /// sent before the next of its kind is replaced by it.
    pub fn sample(&mut self, sample: Sample) {
        for slot in &mut self.slots {
            if let Phase::Open {
                samples,
                sample: waiting,
                ..
            } = &mut slot.phase
                && *samples == proto::Samples::On
            {
                waiting.put(sample);
            }
        }
    }

    /// Whether the last [`Self::step`] ran a handshake operation: the
    /// costly part of a pass (one or two X25519s), after which a loop that
    /// feeds steps its feeding owner again at once.
    pub fn handshook(&self) -> bool {
        self.handshook
    }

    /// One pass. Returns true while the next pass has work to do, so the loop
    /// should step again soon. Waiting is not work: for a client to read, for
    /// a write the writer has not reached, or for a press. The loop's own poll
    /// looks at those, and a loop kept spinning on them would spend its time
    /// on nothing, behind a client that has stopped reading for as long as it
    /// stays connected.
    pub fn step<D: Device>(&mut self, pass: Pass<'_, D>, mut record: impl FnMut(Record)) -> bool {
        let Pass {
            now_ms,
            device,
            pairing,
            identity,
            feeding,
            recovery,
            signer,
        } = pass;
        self.pass = self.pass.wrapping_add(1);
        self.decision = recovery;
        self.signer = self.signer.or(signer);
        self.feeding = feeding;
        self.struck &= feeding;
        self.identity_public = identity.map(|keys| *keys.public());
        if core::mem::take(&mut self.cancel_rescue) {
            pairing.cancel(Path::Recovery);
        }
        if core::mem::take(&mut self.cancel_signer) {
            pairing.cancel(Path::Signer);
            self.signer = None;
        }
        if core::mem::take(&mut self.prune_invites) {
            let clients = &self.clients;
            pairing.issuers_left(|key| clients.iter().flatten().any(|c| c.key == *key));
        }
        self.tell_readers_left(device);
        self.settle_left(device, &mut record);

        // A feed queued before the deadline (a Dispense press: the server
        // refuses its own) holds the restart until it has ended, so the
        // restart never leaves a feed uncertain.
        if let Some(at) = self.reboot_at
            && deadline_passed(now_ms, at)
            && !device.operating()
        {
            self.reboot_at = None;
            device.reboot();
        }

        let mut busy = false;
        self.handshook = false;
        if let Some(identity) = identity {
            self.handshook = self.handshake_turn(now_ms, identity, device, pairing, &mut record);
            busy |= self.handshook;
        }
        for index in 0..SESSIONS {
            busy |= self.session(index, now_ms, device, pairing, &mut record);
        }
        busy
    }

    fn seal_and_send(&mut self, index: usize, message: &DeviceMessage<'_>) -> Result<(), Reason> {
        let len = proto_encode(message, &mut self.plain)?;
        self.seal_plain(index, len)
    }

    /// Seal `self.plain[..len]` into the slot's transmit buffer.
    fn seal_plain(&mut self, index: usize, len: usize) -> Result<(), Reason> {
        let slot = &mut self.slots[index];
        let Phase::Open { transport, .. } = &mut slot.phase else {
            return Ok(());
        };
        let sealed = transport
            .seal(&self.plain[..len], &mut self.out)
            .map_err(|_| Reason::Noise)?;
        slot.wire.send(&self.out[..sealed])
    }

    /// Drop a slot whose client stopped reading: what is queued goes with it,
    /// and the driver closes it. True if it did.
    ///
    /// No answer is lost that a lost connection would have kept. A session
    /// sends only while it waits for nothing (`Self::session`'s pending write
    /// holds back its pushes and any next request), so a write in flight is
    /// never what ends it, and a write it has answered is in the memory under
    /// its id for the retry. Whether a feed ran is in its event either way.
    fn drop_stalled(&mut self, index: usize, now_ms: u32, record: &mut impl FnMut(Record)) -> bool {
        let slot = &mut self.slots[index];
        if !slot.wire.stalled(now_ms) {
            return false;
        }
        slot.wire.discard();

        self.end(index, Reason::Stalled, record);
        true
    }

    /// End `index` with `reason`: anything queued still goes out, then the
    /// driver closes it. A recovery session's offer waiting for its code
    /// goes with it, however it ended (a hang-up, a stall or an error), so
    /// the owner's next presses work the panel again.
    fn end(&mut self, index: usize, reason: Reason, record: &mut impl FnMut(Record)) {
        self.forget_waiting(index);
        let slot = &mut self.slots[index];
        if matches!(slot.phase, Phase::Free | Phase::Closing) {
            return;
        }
        if matches!(slot.phase, Phase::Recovery(_)) {
            self.cancel_rescue = true;
            self.rescue.stop();
        }
        if let Phase::Open {
            events: Some(_), ..
        } = slot.phase
            && let Some(left) = self.readers_left.get_mut(index)
        {
            *left = true;
        }
        slot.phase = Phase::Closing;
        record(Record::Ended {
            slot: slot_id(index),
            reason,
        });
    }
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

fn slot_id(index: usize) -> u8 {
    u8::try_from(index).unwrap_or(u8::MAX)
}

fn proto_encode<T: serde::Serialize>(value: &T, out: &mut [u8]) -> Result<usize, Reason> {
    wellfed_proto::encode(value, out).map_err(|_| Reason::Malformed)
}
