//! Device API, the client's side ([`wellfed_proto::api`]).
//!
//! The caller owns the TCP connection and moves bytes; this module frames,
//! seals and checks them:
//! - [`Session::connect`] opens an enrolled client's session (Noise IK
//!   against the feeder's known identity). Write its bytes, then feed what
//!   arrives to [`Session::receive`] and drain [`Session::next_incoming`]: the first
//!   message is the feeder's welcome ([`Incoming::Opened`] or
//!   [`Incoming::Refused`]), then answers, events, gaps and samples, each
//!   already `wellfed-model`'s owned value. The feeder sends events in
//!   batches; they come out one [`Pushed::Event`] at a time, in order.
//! - [`Session::request`] seals one request under the caller's id. The
//!   feeder remembers a key's last 32 write ids, with their answers, in RAM
//!   across reconnects: a write resent under the same id within that window
//!   and the same boot gets its first answer rather than running again.
//!   Past the window or across a restart, the same id is a new request. So
//!   a write whose answer never arrived is uncertain: a feed (or any write
//!   that must not run twice) is never resent, but settled from the history
//!   ([`crate::settle`]).
//! - [`Join::connect`] redeems an invite (Noise XXpsk3), checking that the
//!   feeder that answers holds the identity the invite named.
//!
//! [`crate::connection::Connection`] runs a whole session over a
//! [`Session`]: the clients use it, and the bench and tests use
//! [`Session`] directly to send what a well-behaved client never would.

pub use wellfed_model::Skew;
use wellfed_model::{
    Answer, ClientRole, DeviceKind, Gap, Hello, Joined, Model, Op, OpenRefusal, Pushed, Refusal,
    convert,
};
pub use wellfed_proto::api::Samples;
use wellfed_proto::api::{
    ClientMessage, DEVICE_MESSAGES, DeviceMessage, FRAME_MAX, Join as JoinRequest, MESSAGE_MAX,
    Op as WireOp, Open, PROLOGUE, REVISION, RequestId, Welcome,
};
use wellfed_session::frame;

use crate::frames::{Frames, TooLong};
use wellfed_session::hex::{self, Lower};
use wellfed_session::noise::{self, Handshake, Keypair, Pattern, Role, Transport};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the handshake failed ({0:?}): the wrong feeder, or a different protocol version")]
    Noise(noise::Error),
    #[error("the feeder sent a message the session did not expect")]
    Unexpected,
    #[error("the feeder's message does not decode")]
    Malformed(Garbled),
    #[error("a frame announces {0} bytes, more than the device API carries")]
    TooLong(usize),
    #[error("the message does not fit one frame")]
    TooLarge,
    #[error("the feeder that answered is not the one the invite names")]
    WrongFeeder,
    #[error("the label must be 1-16 bytes with no NUL")]
    Label,
    #[error("the feeder refused the session: {0:?}")]
    Refused(OpenRefusal),
    #[error("the feeder did not answer in time")]
    Timeout,
    #[error("the feeder stopped answering")]
    Silent,
    #[error("that request id is the keepalive's")]
    ReservedId,
    #[error("the session is not open yet")]
    NotOpen,
    /// The feeder runs a build from before the API revision
    /// ([`Skew::Legacy`]): only frozen ops go to it, so nothing was sent.
    #[error(
        "the feeder runs a build from before API revisions, which takes only an update; \
         nothing was sent: update the feeder"
    )]
    Legacy,
    /// The op holds a value only a newer build names (a role or a window
    /// read from a newer server), which has no wire form: nothing was sent.
    #[error("the request holds a value this build cannot name; nothing was sent")]
    NoWireForm(#[from] wellfed_model::convert::NoWireForm),
    #[error(transparent)]
    Setup(#[from] crate::setup::Error),
}

impl From<noise::Error> for Error {
    fn from(error: noise::Error) -> Self {
        Self::Noise(error)
    }
}

/// Bytes of an undecodable message kept for the evidence.
pub const GARBLED_PREFIX: usize = 32;

/// What a message that authenticated but did not decode held: its length
/// and first bytes. It authenticated, so the feeder sealed exactly these
/// bytes; they tell a feeder that sealed wrong bytes from a client that
/// reads right ones wrong.
///
/// It is the message's plaintext, which never holds session keys, but an
/// invite's answer carries the invite's secret: so neither `Debug` nor any
/// error text shows the bytes, and only [`Self::prefix`] reads them, for a
/// caller that knows no such answer can be outstanding.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Garbled {
    /// The whole plaintext's length.
    pub len: usize,
    prefix: [u8; GARBLED_PREFIX],
}

impl Garbled {
    fn of(plain: &[u8]) -> Self {
        let mut prefix = [0; GARBLED_PREFIX];
        let kept = plain.len().min(GARBLED_PREFIX);
        prefix[..kept].copy_from_slice(&plain[..kept]);
        Self {
            len: plain.len(),
            prefix,
        }
    }

    /// The first bytes, up to [`GARBLED_PREFIX`].
    pub fn prefix(&self) -> &[u8] {
        &self.prefix[..self.len.min(GARBLED_PREFIX)]
    }
}

impl std::fmt::Debug for Garbled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Garbled")
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

/// One frame around `message`.
pub fn frame(message: &[u8]) -> Result<Vec<u8>, Error> {
    let prefix = frame::prefix(message.len(), FRAME_MAX).ok_or(Error::TooLarge)?;
    let mut out = prefix.to_vec();
    out.extend_from_slice(message);
    Ok(out)
}

/// The clear frame that opens a connection.
pub(crate) fn open_frame(open: Open) -> Result<Vec<u8>, Error> {
    let mut out = [0; 16];
    let len = wellfed_proto::encode(&open, &mut out).map_err(|_| Error::TooLarge)?;
    frame(&out[..len])
}

enum State {
    Handshake(Box<Handshake>),
    Open(Box<Transport>),
    Ended,
}

/// The device's welcome, owned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opened {
    /// Which module of the device API the device serves: its model's
    /// kind, which the welcome does not repeat.
    pub kind: DeviceKind,
    /// The device's model.
    pub model: Model,
    /// The running image's build identity.
    pub build: String,
    /// The role the device gave this key.
    pub role: ClientRole,
    /// The epoch the feeder's event numbers count in.
    pub epoch: u32,
    /// The feeder's build's API revision ([`REVISION`]), which the
    /// welcome's frozen `schema` field carries.
    pub schema: u32,
}

impl Opened {
    /// Which side is the older build. The API only grows, so either way
    /// every op this client knows may go: a feeder behind it refuses what
    /// its build lacks `Unavailable`, and one ahead of it has more than this
    /// client can show.
    pub fn skew(&self) -> Skew {
        Skew::of(self.schema, REVISION)
    }
}

/// What [`Session::next_incoming`] yields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Incoming {
    /// The feeder opened the session.
    Opened(Opened),
    /// The feeder refused the session, which has ended.
    Refused(OpenRefusal),
    /// The feeder's answer to the request under `id`.
    Answer { id: RequestId, answer: Answer },
    /// An event, gap or sample the feeder pushed.
    Pushed(Pushed),
}

/// An enrolled client's session.
pub struct Session {
    state: State,
    frames: Frames,
    /// Opened messages waiting for [`Self::next_incoming`], oldest first.
    inbox: std::collections::VecDeque<Vec<u8>>,
    /// The rest of a batch of events, oldest first: they come before
    /// anything still in `inbox`.
    events: std::collections::VecDeque<Pushed>,
    /// The welcome's epoch, once it has arrived: what pushed events count
    /// in.
    epoch: Option<u32>,
    /// The welcome named a pre-revision build ([`Skew::Legacy`]): every op
    /// outside the frozen part, and a subscription, is refused unsent.
    legacy: bool,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("open", &matches!(self.state, State::Open(_)))
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Open a session to the feeder whose identity is `feeder`; the bytes to
    /// write first. `ephemeral` comes from the OS generator, fresh.
    pub fn connect(
        keys: &Keypair,
        feeder: [u8; 32],
        ephemeral: [u8; 32],
    ) -> Result<(Self, Vec<u8>), Error> {
        Self::connect_with(keys, feeder, ephemeral, Hello::CURRENT)
    }

    /// As [`Self::connect`] with another hello: what a client built for
    /// another major version looks like to the device.
    pub fn connect_with(
        keys: &Keypair,
        feeder: [u8; 32],
        ephemeral: [u8; 32],
        hello: Hello,
    ) -> Result<(Self, Vec<u8>), Error> {
        let mut handshake = Handshake::new(
            Pattern::Ik,
            Role::Initiator,
            PROLOGUE,
            keys,
            Some(feeder),
            ephemeral,
        )?;
        let mut payload = [0; 16];
        let wire = wellfed_proto::api::Hello::from(hello);
        let len = wellfed_proto::encode(&wire, &mut payload).map_err(|_| Error::TooLarge)?;
        let mut message = [0; FRAME_MAX];
        let written = handshake.write_message(&payload[..len], &mut message)?;

        let mut out = open_frame(Open::Session)?;
        out.extend(frame(&message[..written])?);
        let session = Self {
            state: State::Handshake(Box::new(handshake)),
            frames: Frames::new(FRAME_MAX),
            inbox: std::collections::VecDeque::new(),
            events: std::collections::VecDeque::new(),
            epoch: None,
            legacy: false,
        };
        Ok((session, out))
    }

    pub fn is_open(&self) -> bool {
        matches!(self.state, State::Open(_))
    }

    /// Bytes from the connection. Any error ends the session.
    pub fn receive(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let result = self.take(bytes);
        if result.is_err() {
            self.state = State::Ended;
        }
        result
    }

    fn take(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.frames.push(bytes);
        while let Some(message) = self
            .frames
            .next()
            .map_err(|TooLong(announced)| Error::TooLong(announced))?
        {
            let mut plain = vec![0; FRAME_MAX];
            let len = match &mut self.state {
                State::Handshake(handshake) => {
                    let len = handshake.read_message(&message, &mut plain)?;
                    let State::Handshake(handshake) =
                        std::mem::replace(&mut self.state, State::Ended)
                    else {
                        return Err(Error::Unexpected);
                    };
                    self.state = State::Open(Box::new(handshake.into_transport()?));
                    len
                }
                State::Open(transport) => transport.open(&message, &mut plain)?,
                State::Ended => return Err(Error::Unexpected),
            };
            plain.truncate(len);
            self.inbox.push_back(plain);
        }
        Ok(())
    }

    /// The next message, decoded into owned values.
    pub fn next_incoming(&mut self) -> Result<Option<Incoming>, Error> {
        // A refused session is over: whatever else arrived with the
        // refusal is never read, so it cannot pass for a second welcome.
        if matches!(self.state, State::Ended) {
            self.inbox.clear();
            self.events.clear();
            return Ok(None);
        }
        if let Some(event) = self.events.pop_front() {
            return Ok(Some(Incoming::Pushed(event)));
        }
        // A message kind a newer build added comes in a frame of its own,
        // so it is skipped whole and the next one read. Its tag is past
        // every variant this build decodes, so no message it knows is lost.
        // So is a live sample this build cannot decode (a metric, or a value
        // inside one, that a newer build added): samples are never stored
        // and the next of a kind replaces the last, so it is no loss, and
        // ending the session over it would leave every write in flight
        // uncertain.
        loop {
            let Some(plain) = self.inbox.pop_front() else {
                return Ok(None);
            };
            let Some(epoch) = self.epoch else {
                let welcome: Welcome<'_> = wellfed_proto::decode(&plain)
                    .map_err(|_| Error::Malformed(Garbled::of(&plain)))?;
                return Ok(Some(match welcome {
                    Welcome::Open {
                        major: _,
                        model,
                        build,
                        role,
                        epoch,
                        schema,
                    } => {
                        self.epoch = Some(epoch);
                        self.legacy = Skew::of(schema, REVISION) == Skew::Legacy;
                        Incoming::Opened(Opened {
                            kind: model.kind(),
                            model,
                            build: build.to_owned(),
                            role: role.into(),
                            epoch,
                            schema,
                        })
                    }
                    Welcome::Refused(refusal) => {
                        self.state = State::Ended;
                        self.inbox.clear();
                        Incoming::Refused(refusal.into())
                    }
                }));
            };
            if newer_message(&plain) {
                continue;
            }

            let message = match wellfed_proto::decode::<DeviceMessage<'_>>(&plain) {
                Ok(message) => message,
                Err(_) if plain.first() == Some(&SAMPLE) => continue,
                Err(_) => {
                    return unreadable(&plain)
                        .map(Some)
                        .ok_or_else(|| Error::Malformed(Garbled::of(&plain)));
                }
            };
            return Ok(Some(match message {
                DeviceMessage::Response { id, result } => Incoming::Answer {
                    id,
                    answer: result.into(),
                },
                // One batch, one event at a time: the whole batch frames
                // before any of it is handed on, so a garbled one is refused
                // whole, as any other malformed message is. A body this build
                // does not know is no fault: it comes out unknown, numbered, so
                // the cursor moves past it.
                DeviceMessage::Events(batch) => {
                    let events = batch
                        .iter()
                        .map(|event| event.map(|event| convert::raw_event(epoch, event)))
                        .collect::<Result<std::collections::VecDeque<_>, _>>()
                        .map_err(|_| Error::Malformed(Garbled::of(&plain)))?;
                    self.events = events;
                    let Some(first) = self.events.pop_front() else {
                        return Err(Error::Malformed(Garbled::of(&plain)));
                    };
                    Incoming::Pushed(first)
                }
                DeviceMessage::Gap { from, to } => {
                    Incoming::Pushed(Pushed::Gap(Gap { epoch, from, to }))
                }
                DeviceMessage::Sample(sample) => Incoming::Pushed(Pushed::Sample(sample.into())),
                DeviceMessage::Log { seq, dropped, text } => {
                    Incoming::Pushed(Pushed::Log(convert::log(seq, dropped, text)))
                }
            }));
        }
    }

    /// One request under `id`, sealed and framed. It goes to a feeder of
    /// any revision: the API only grows, so an op means the same to every
    /// build that knows it, and one that does not answers `Unavailable`. A
    /// pre-revision build takes frozen ops alone: any other is
    /// [`Error::Legacy`], unsent, since its bytes mean something else there.
    pub fn request(&mut self, id: RequestId, op: &Op) -> Result<Vec<u8>, Error> {
        if self.legacy && !op.is_frozen() {
            return Err(Error::Legacy);
        }
        self.send(&ClientMessage::Request {
            id,
            op: op.to_proto()?,
        })
    }

    /// The keepalive under `id`: frozen, so it goes to a feeder of any
    /// build, and its answer is always one this client reads.
    pub fn alive(&mut self, id: RequestId) -> Result<Vec<u8>, Error> {
        self.send(&ClientMessage::Request {
            id,
            op: WireOp::Alive,
        })
    }

    /// Ask for every event after `after` and then each new one, and the
    /// live samples if `samples` is on. A newer feeder's bodies this client
    /// does not know come out as [`Pushed::Unknown`].
    pub fn subscribe(&mut self, after: Option<u64>, samples: Samples) -> Result<Vec<u8>, Error> {
        // A pre-revision build frames its events without lengths: they
        // would read as junk.
        if self.legacy {
            return Err(Error::Legacy);
        }
        self.send(&ClientMessage::Subscribe { after, samples })
    }

    /// Nothing goes before the welcome is read: the handshake's reply
    /// opens the transport, but only the welcome says whether the feeder
    /// is a pre-revision build that must get frozen ops alone. A message
    /// sealed into a frame must leave room for the seal's tag, so a larger
    /// one is [`Error::TooLarge`] before any nonce is spent.
    fn send(&mut self, message: &ClientMessage<'_>) -> Result<Vec<u8>, Error> {
        let State::Open(transport) = &mut self.state else {
            return Err(Error::Unexpected);
        };
        if self.epoch.is_none() {
            return Err(Error::NotOpen);
        }

        let mut plain = [0; MESSAGE_MAX];
        let len = wellfed_proto::encode(message, &mut plain).map_err(|_| Error::TooLarge)?;
        let mut sealed = [0; FRAME_MAX];
        let sealed_len = transport.seal(&plain[..len], &mut sealed)?;
        frame(&sealed[..sealed_len])
    }
}

/// Whether `plain` is a device message of a kind a newer build added: its
/// tag, postcard's leading `u32` varint, is at or past [`DEVICE_MESSAGES`].
/// Bytes whose tag is no `u32` varint at all are garbled, not newer, and
/// end the session like any other message that does not decode.
fn newer_message(plain: &[u8]) -> bool {
    // A `u32` varint takes at most five bytes, seven bits each.
    let mut tag: u64 = 0;
    for (index, &byte) in plain.iter().take(5).enumerate() {
        tag |= u64::from(byte & 0x7f) << (7 * index);
        if byte & 0x80 == 0 {
            return u32::try_from(tag).is_ok_and(|tag| tag >= DEVICE_MESSAGES);
        }
    }
    false
}

/// Postcard's one-byte index of [`DeviceMessage::Sample`], which keeps its
/// index as every message kind does once a build ships: a sample that does
/// not decode is a newer build's.
const SAMPLE: u8 = 3;

/// A response this build cannot decode, which a newer build sent: its id
/// sits right after the message's tag, so the request it answers is still
/// known, and the session goes on. An outcome tagged as a refusal is one this
/// build cannot name: a device refuses before it acts, so the request did
/// not run. Anything else is unreadable: the request may have run, so its
/// caller treats a write answered so as uncertain and never resends it.
/// `None` for bytes that are no response at all, which end the session.
fn unreadable(plain: &[u8]) -> Option<Incoming> {
    // Postcard writes `DeviceMessage::Response` and `Outcome::Refused`, both
    // frozen, as their one-byte indices.
    const RESPONSE: u8 = 0;
    const REFUSED: u8 = 1;

    let (&RESPONSE, rest) = plain.split_first()? else {
        return None;
    };
    let (id, outcome) = rest.split_first_chunk::<16>()?;
    let answer = match outcome.first() {
        Some(&REFUSED) => Answer::Refused(Refusal::Unknown),
        _ => Answer::Unreadable,
    };
    Some(Incoming::Answer { id: *id, answer })
}

/// An invite as the issuing client shows it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Invite {
    /// The feeder's identity key.
    pub feeder: [u8; 32],
    pub id: u32,
    pub secret: [u8; 32],
}

impl std::fmt::Debug for Invite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Invite")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// The text form's start; the digit is the form's version.
pub const INVITE_PREFIX: &str = "wellfed-invite:1:";

/// Why a text is not an invite.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InviteText {
    #[error("not a wellfed invite")]
    Prefix,
    #[error("an invite has a feeder, an id and a secret")]
    Parts,
    #[error("the invite's feeder key is not 32 bytes of lowercase hex")]
    Feeder,
    #[error("the invite's id is not a number")]
    Id,
    #[error("the invite's secret is not 32 bytes of lowercase hex")]
    Secret,
}

impl Invite {
    /// The invite as one line of text (`wellfed-invite:1:<feeder>:<id>:<secret>`),
    /// as the issuing client shows it and a redeeming one takes it. It holds
    /// the secret, so it is shown once and never logged.
    pub fn text(&self) -> String {
        format!(
            "{INVITE_PREFIX}{}:{}:{}",
            Lower(&self.feeder),
            self.id,
            Lower(&self.secret)
        )
    }

    pub fn parse(text: &str) -> Result<Self, InviteText> {
        let rest = text
            .trim()
            .strip_prefix(INVITE_PREFIX)
            .ok_or(InviteText::Prefix)?;
        let mut parts = rest.split(':');
        let (Some(feeder), Some(id), Some(secret), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(InviteText::Parts);
        };

        Ok(Self {
            feeder: hex::decode_array(feeder.as_bytes()).ok_or(InviteText::Feeder)?,
            id: id.parse().map_err(|_| InviteText::Id)?,
            secret: hex::decode_array(secret.as_bytes()).ok_or(InviteText::Secret)?,
        })
    }
}

/// What a redemption step asks of the caller.
#[derive(Debug, PartialEq, Eq)]
pub enum JoinStep {
    /// Write these bytes and wait for more.
    Send(Vec<u8>),
    /// The feeder's answer; the connection is done.
    Done(Joined),
}

/// A new client redeeming an invite.
pub struct Join {
    handshake: Option<Box<Handshake>>,
    transport: Option<Box<Transport>>,
    frames: Frames,
    invite: Invite,
    label: String,
}

impl std::fmt::Debug for Join {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Join").finish_non_exhaustive()
    }
}

impl Join {
    /// Start redeeming `invite` as `label`; the bytes to write first.
    pub fn connect(
        keys: &Keypair,
        invite: Invite,
        label: &str,
        ephemeral: [u8; 32],
    ) -> Result<(Self, Vec<u8>), Error> {
        if label.is_empty() || label.len() > 16 || label.contains('\0') {
            return Err(Error::Label);
        }
        let mut handshake = Handshake::new(
            Pattern::XxPsk3,
            Role::Initiator,
            PROLOGUE,
            keys,
            None,
            ephemeral,
        )?;
        handshake.set_psk(invite.secret);
        let mut message = [0; 64];
        let written = handshake.write_message(&[], &mut message)?;

        let mut out = open_frame(Open::Invite { id: invite.id })?;
        out.extend(frame(&message[..written])?);
        let join = Self {
            handshake: Some(Box::new(handshake)),
            transport: None,
            frames: Frames::new(FRAME_MAX),
            invite,
            label: label.to_owned(),
        };
        Ok((join, out))
    }

    /// Bytes from the connection: what to write next, or the answer.
    pub fn receive(&mut self, bytes: &[u8]) -> Result<Option<JoinStep>, Error> {
        self.frames.push(bytes);
        let Some(message) = self
            .frames
            .next()
            .map_err(|TooLong(announced)| Error::TooLong(announced))?
        else {
            return Ok(None);
        };
        let mut plain = [0; FRAME_MAX];

        if let Some(mut handshake) = self.handshake.take() {
            handshake.read_message(&message, &mut plain)?;
            if handshake.remote_static() != Some(&self.invite.feeder) {
                return Err(Error::WrongFeeder);
            }
            let join = JoinRequest { label: &self.label };
            let mut payload = [0; 32];
            let len = wellfed_proto::encode(&join, &mut payload).map_err(|_| Error::TooLarge)?;
            let mut out = [0; 256];
            let written = handshake.write_message(&payload[..len], &mut out)?;
            self.transport = Some(Box::new(handshake.into_transport()?));
            return Ok(Some(JoinStep::Send(frame(&out[..written])?)));
        }

        let transport = self.transport.as_mut().ok_or(Error::Unexpected)?;
        let len = transport.open(&message, &mut plain)?;
        let joined: wellfed_proto::api::Joined = wellfed_proto::decode(&plain[..len])
            .map_err(|_| Error::Malformed(Garbled::of(&plain[..len])))?;
        Ok(Some(JoinStep::Done(joined.into())))
    }
}

#[cfg(test)]
mod tests {
    use wellfed_proto::api::{OpenRefusal as WireRefusal, Role as WireRole};
    use wellfed_proto::device::Model;

    use super::*;

    /// The feeder's side of an IK session: answer the client's first
    /// message with `welcome`, then seal each of `after` as a transport
    /// message, all in one buffer.
    fn feeder_says(hello: &[u8], welcome: &Welcome<'_>, after: &[Welcome<'_>]) -> Vec<u8> {
        let feeder = Keypair::from_secret([0x10; 32]);
        let mut handshake = Handshake::new(
            Pattern::Ik,
            Role::Responder,
            PROLOGUE,
            &feeder,
            None,
            [0x22; 32],
        )
        .unwrap();
        let open = frame::PREFIX + frame::parse_prefix([hello[0], hello[1]], FRAME_MAX).unwrap();
        let mut frames = Frames::new(FRAME_MAX);
        frames.push(&hello[open..]);
        let first = frames.next().unwrap().unwrap();
        let mut payload = [0; FRAME_MAX];
        handshake.read_message(&first, &mut payload).unwrap();

        let mut plain = [0; FRAME_MAX];
        let mut message = [0; FRAME_MAX];
        let len = wellfed_proto::encode(welcome, &mut plain).unwrap();
        let written = handshake
            .write_message(&plain[..len], &mut message)
            .unwrap();
        let mut out = frame(&message[..written]).unwrap();
        let mut transport = handshake.into_transport().unwrap();
        for next in after {
            let len = wellfed_proto::encode(next, &mut plain).unwrap();
            let sealed = transport.seal(&plain[..len], &mut message).unwrap();
            out.extend(frame(&message[..sealed]).unwrap());
        }
        out
    }

    /// [`SAMPLE`] is the index a sample is written under.
    #[test]
    fn a_sample_is_written_under_its_index() {
        let sample = DeviceMessage::Sample(wellfed_proto::api::Sample {
            at_ms: 5,
            metric: wellfed_proto::api::Metric::Rssi(-60),
        });
        let mut plain = [0; FRAME_MAX];
        wellfed_proto::encode(&sample, &mut plain).unwrap();

        assert_eq!(plain[0], SAMPLE);
    }

    /// A garbled invite answer may hold the invite's secret: the error's
    /// `Debug` and text name only the length.
    #[test]
    fn a_garbled_message_s_bytes_stay_out_of_the_error_s_debug_and_text() {
        let secret = [0xa7; 40];
        let error = Error::Malformed(Garbled::of(&secret));

        let shown = format!("{error:?} {error}");

        assert!(shown.contains("len: 40"), "{shown}");
        for byte in ["a7", "A7", "167"] {
            assert!(!shown.contains(byte), "{shown}");
        }
        let Error::Malformed(garbled) = error else {
            unreachable!()
        };
        assert_eq!(garbled.prefix(), &secret[..GARBLED_PREFIX]);
    }

    #[test]
    fn nothing_after_a_refusal_reads_as_a_welcome() {
        let client = Keypair::from_secret([0x20; 32]);
        let feeder = *Keypair::from_secret([0x10; 32]).public();
        let (mut session, hello) = Session::connect(&client, feeder, [0x33; 32]).unwrap();
        let open = Welcome::Open {
            major: 1,
            model: Model::Plaf103,
            build: "b",
            role: WireRole::Owner,
            epoch: 1,
            schema: REVISION,
        };
        let bytes = feeder_says(&hello, &Welcome::Refused(WireRefusal::Unknown), &[open]);

        // A second welcome arrives in the same read as the refusal.
        session.receive(&bytes).unwrap();
        assert_eq!(
            session.next_incoming().unwrap(),
            Some(Incoming::Refused(OpenRefusal::Unknown))
        );
        assert_eq!(session.next_incoming().unwrap(), None);
        assert!(!session.is_open());
    }

    fn opened_by(welcome: &Welcome<'_>) -> Session {
        let client = Keypair::from_secret([0x20; 32]);
        let feeder = *Keypair::from_secret([0x10; 32]).public();
        let (mut session, hello) = Session::connect(&client, feeder, [0x33; 32]).unwrap();
        session.receive(&feeder_says(&hello, welcome, &[])).unwrap();
        session
    }

    fn welcome(schema: u32) -> Welcome<'static> {
        Welcome::Open {
            major: 1,
            model: Model::Plaf103,
            build: "b",
            role: WireRole::Owner,
            epoch: 1,
            schema,
        }
    }

    /// A request whose message fills a frame but leaves no room for the
    /// seal's tag is too large, refused before a nonce is spent, so the
    /// next request still seals in step.
    #[test]
    fn a_request_past_a_sealed_frame_is_too_large() {
        let mut session = opened_by(&welcome(REVISION));
        assert!(matches!(
            session.next_incoming(),
            Ok(Some(Incoming::Opened(_)))
        ));
        let chunk = |len| Op::UpdateChunk {
            offset: 0,
            data: vec![0; len],
        };
        let encoded = |op: &Op| {
            let mut out = [0; 2 * FRAME_MAX];
            wellfed_proto::encode(
                &ClientMessage::Request {
                    id: [8; 16],
                    op: op.to_proto().unwrap(),
                },
                &mut out,
            )
            .unwrap()
        };
        let fits = (900..FRAME_MAX)
            .map(chunk)
            .find(|op| encoded(op) == MESSAGE_MAX)
            .unwrap();

        for len in [MESSAGE_MAX + 1, FRAME_MAX - 1, FRAME_MAX] {
            let op = (900..FRAME_MAX)
                .map(chunk)
                .find(|op| encoded(op) == len)
                .unwrap();
            assert!(matches!(
                session.request([8; 16], &op),
                Err(Error::TooLarge)
            ));
        }
        let sealed = session.request([8; 16], &fits).unwrap();
        assert_eq!(sealed.len(), frame::PREFIX + FRAME_MAX);
        assert!(session.request([9; 16], &Op::Status).is_ok());
    }

    /// The handshake's reply opens the transport, but nothing goes until
    /// the welcome is read: only it says whether the feeder takes frozen
    /// ops alone.
    #[test]
    fn nothing_goes_before_the_welcome_is_read() {
        let mut session = opened_by(&welcome(wellfed_model::LEGACY_SCHEMA_FROM));
        assert!(session.is_open());

        assert!(matches!(
            session.request([1; 16], &Op::Status),
            Err(Error::NotOpen)
        ));
        assert!(matches!(session.alive([2; 16]), Err(Error::NotOpen)));
        assert!(matches!(
            session.next_incoming(),
            Ok(Some(Incoming::Opened(_)))
        ));
        assert!(matches!(
            session.request([1; 16], &Op::Status),
            Err(Error::Legacy)
        ));
        assert!(session.alive([2; 16]).is_ok());
    }

    /// The feeder's side of an IK session up to its welcome: the bytes to
    /// hand the client, and the transport to seal more with.
    fn welcomed(hello: &[u8]) -> (Vec<u8>, Transport) {
        let feeder = Keypair::from_secret([0x10; 32]);
        let mut handshake = Handshake::new(
            Pattern::Ik,
            Role::Responder,
            PROLOGUE,
            &feeder,
            None,
            [0x22; 32],
        )
        .unwrap();
        let open = frame::PREFIX + frame::parse_prefix([hello[0], hello[1]], FRAME_MAX).unwrap();
        let mut frames = Frames::new(FRAME_MAX);
        frames.push(&hello[open..]);
        let first = frames.next().unwrap().unwrap();
        let mut payload = [0; FRAME_MAX];
        handshake.read_message(&first, &mut payload).unwrap();

        let mut plain = [0; FRAME_MAX];
        let mut message = [0; FRAME_MAX];
        let len = wellfed_proto::encode(&welcome(REVISION), &mut plain).unwrap();
        let written = handshake
            .write_message(&plain[..len], &mut message)
            .unwrap();
        (
            frame(&message[..written]).unwrap(),
            handshake.into_transport().unwrap(),
        )
    }

    fn sealed(transport: &mut Transport, plain: &[u8]) -> Vec<u8> {
        let mut message = [0; FRAME_MAX];
        let len = transport.seal(plain, &mut message).unwrap();
        frame(&message[..len]).unwrap()
    }

    /// An answer a newer feeder sent that this build can't decode reaches
    /// the pending write through the whole connection: unreadable (it may
    /// have run) or, tagged as a refusal, an unknown refusal (it did not),
    /// and the session stays open.
    #[test]
    fn an_undecodable_answer_reaches_its_write_and_the_session_stays() {
        use crate::connection::{Connection, Event};

        let start = std::time::Instant::now();
        let client = Keypair::from_secret([0x20; 32]);
        let feeder = *Keypair::from_secret([0x10; 32]).public();
        let mut connection = Connection::new(&client, feeder, [0x33; 32], start).unwrap();
        let hello = connection.poll_transmit().unwrap();
        let (welcome, mut transport) = welcomed(&hello);
        connection.handle_input(start, &welcome).unwrap();
        assert!(connection.opened().is_some());

        let lock = Op::SetLock { locked: true };
        connection.request(start, [1; 16], &lock).unwrap();
        connection.request(start, [2; 16], &lock).unwrap();
        let mut unreadable = vec![0];
        unreadable.extend_from_slice(&[1; 16]);
        unreadable.extend_from_slice(&[0x70, 0x01]);
        let mut refused = vec![0];
        refused.extend_from_slice(&[2; 16]);
        refused.extend_from_slice(&[1, 0x70]);
        let mut bytes = sealed(&mut transport, &unreadable);
        bytes.extend(sealed(&mut transport, &refused));
        connection.handle_input(start, &bytes).unwrap();

        assert_eq!(
            connection.poll_event(),
            Some(Event::Answer {
                id: [1; 16],
                answer: Answer::Unreadable
            })
        );
        assert_eq!(
            connection.poll_event(),
            Some(Event::Answer {
                id: [2; 16],
                answer: Answer::Refused(Refusal::Unknown)
            })
        );
        assert!(connection.request(start, [3; 16], &Op::Status).is_ok());
        assert!(connection.opened().is_some());
    }

    /// An invite's redemption checks the feeder that answers holds the key
    /// the invite named: one that knows the secret but holds another key
    /// gets no `Join`.
    #[test]
    fn an_impostor_with_the_invite_s_secret_gets_no_join() {
        let client = Keypair::from_secret([0x20; 32]);
        let invite = Invite {
            feeder: *Keypair::from_secret([0x10; 32]).public(),
            id: 7,
            secret: [0x44; 32],
        };
        let (mut join, hello) = Join::connect(&client, invite, "phone", [0x33; 32]).unwrap();

        let impostor = Keypair::from_secret([0x66; 32]);
        let mut handshake = Handshake::new(
            Pattern::XxPsk3,
            Role::Responder,
            PROLOGUE,
            &impostor,
            None,
            [0x22; 32],
        )
        .unwrap();
        handshake.set_psk(invite.secret);
        let mut frames = Frames::new(FRAME_MAX);
        frames.push(&hello);
        let _open = frames.next().unwrap().unwrap();
        let first = frames.next().unwrap().unwrap();
        let mut payload = [0; FRAME_MAX];
        handshake.read_message(&first, &mut payload).unwrap();
        let mut message = [0; FRAME_MAX];
        let written = handshake.write_message(&[], &mut message).unwrap();

        let answer = join.receive(&frame(&message[..written]).unwrap());

        assert!(matches!(answer, Err(Error::WrongFeeder)), "{answer:?}");
    }

    /// A label is 1 to 16 bytes with no NUL, checked before anything goes.
    #[test]
    fn a_join_label_is_bounded() {
        let client = Keypair::from_secret([0x20; 32]);
        let invite = Invite {
            feeder: [0x10; 32],
            id: 7,
            secret: [0x44; 32],
        };
        for label in ["", "seventeen bytes!!", "a\0b"] {
            assert!(
                matches!(
                    Join::connect(&client, invite, label, [0x33; 32]),
                    Err(Error::Label)
                ),
                "{label:?}"
            );
        }
        assert!(Join::connect(&client, invite, "sixteen bytes!!!", [0x33; 32]).is_ok());
        assert!(Join::connect(&client, invite, "a", [0x33; 32]).is_ok());
    }

    /// A response a newer build sent that this one cannot decode still
    /// answers its request: an unknown refusal by its tag, anything else
    /// unreadable. Bytes that are no response at all are no answer, and
    /// end the session as malformed.
    #[test]
    fn an_undecodable_response_answers_its_request() {
        let id = [0x42; 16];
        let mut refusal = vec![0];
        refusal.extend_from_slice(&id);
        refusal.extend_from_slice(&[1, 0x70]);
        let mut outcome = vec![0];
        outcome.extend_from_slice(&id);
        outcome.extend_from_slice(&[0x70, 0x01, 0x02]);
        assert_eq!(
            unreadable(&refusal),
            Some(Incoming::Answer {
                id,
                answer: Answer::Refused(Refusal::Unknown)
            })
        );
        assert_eq!(
            unreadable(&outcome),
            Some(Incoming::Answer {
                id,
                answer: Answer::Unreadable
            })
        );
        assert_eq!(unreadable(&[0x7f, 1, 2]), None);
        assert_eq!(unreadable(&[0, 1, 2]), None);
    }

    #[test]
    fn an_invite_survives_its_text_form() {
        let invite = Invite {
            feeder: [0xab; 32],
            id: 7,
            secret: [0x01; 32],
        };
        let text = invite.text();
        assert!(text.starts_with("wellfed-invite:1:abab"));
        assert_eq!(Invite::parse(&text), Ok(invite));
        assert_eq!(Invite::parse(&format!(" {text}\n")), Ok(invite));
    }

    #[test]
    fn a_malformed_invite_says_which_part() {
        let feeder = "ab".repeat(32);
        let secret = "01".repeat(32);
        let upper = "AB".repeat(32);
        let short = "01".repeat(31);
        for (text, error) in [
            (format!("invite:{feeder}:1:{secret}"), InviteText::Prefix),
            (format!("{INVITE_PREFIX}{feeder}:1"), InviteText::Parts),
            (
                format!("{INVITE_PREFIX}{feeder}:1:{secret}:x"),
                InviteText::Parts,
            ),
            (
                format!("{INVITE_PREFIX}{upper}:1:{secret}"),
                InviteText::Feeder,
            ),
            (
                format!("{INVITE_PREFIX}{feeder}:x:{secret}"),
                InviteText::Id,
            ),
            (
                format!("{INVITE_PREFIX}{feeder}:1:{short}"),
                InviteText::Secret,
            ),
        ] {
            assert_eq!(Invite::parse(&text), Err(error), "{text}");
        }
    }

    /// A tag past the known kinds is a newer build's message, whatever its
    /// varint's length; bytes that are no `u32` varint are garbled.
    #[test]
    fn a_newer_tag_is_told_from_garbled_bytes() {
        let past = u8::try_from(DEVICE_MESSAGES).expect("a one-byte tag");

        assert!(!newer_message(&[0, 1, 2]));
        assert!(!newer_message(&[past - 1]));
        assert!(newer_message(&[past, 0]));
        assert!(newer_message(&[0x80, 0x01]));

        assert!(!newer_message(&[0xff; 24]));
        assert!(!newer_message(&[0xff, 0xff]));
        assert!(!newer_message(&[]));
    }
}
