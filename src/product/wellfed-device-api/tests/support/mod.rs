//! The feeder's network-facing state machines on the host, driven with
//! whatever bytes a test or a fuzzer supplies: the device API's [`Server`]
//! behind the crate's fake device ([`wellfed_device_api::fake`]), and BLE
//! setup's [`Responder`].
//!
//! The property tests (`tests/properties.rs`) and the cargo-fuzz targets
//! (`src/fuzz/`, which includes this file by path) share these drivers, so both
//! check the same invariants: nothing panics, every frame the feeder writes
//! fits its limit, and every message it seals decodes as the protocol says.
//! This file therefore uses only this crate's own dependencies and its `fake`
//! feature.

#![allow(dead_code, reason = "each consumer drives a subset")]

use wellfed_core::buttons::{Button, Buttons, Gesture};
use wellfed_device_api::clients::{CLIENTS, Client, Label, Role};
use wellfed_device_api::fake::Fake;
pub use wellfed_device_api::fake::{FEEDER, Faked, Fountain, MEMBER, OWNER};
use wellfed_device_api::mailbox::Lock;
use wellfed_device_api::pairing::{Config, Event as PairingEvent, Pairing};
use wellfed_device_api::setup::{Received, Responder};
use wellfed_device_api::wifi::Credentials;
use wellfed_device_api::{Pass, Reason, Record, SESSIONS, Server};
use wellfed_proto::api::{
    ClientMessage, DeviceMessage, EventBody, FRAME_MAX, Hello, MESSAGE_MAX, Op, Open, Outcome,
    PROLOGUE, PROTOCOL_MAJOR, Refusal, RequestId, Welcome,
};
use wellfed_proto::setup::{self, Answer, Reply};
use wellfed_session::frame;
use wellfed_session::noise::{Handshake, Keypair, Pattern, Role as NoiseRole, Transport};
use wellfed_session::setup_code::{Code, Exchange, Round, Side};

/// The live invite every rig holds: its id and its secret.
pub const INVITE_ID: u32 = 1;
pub const INVITE_SECRET: [u8; 32] = [0x40; 32];
/// Passes a driver runs after its input is spent, for answers to leave.
const SETTLE: usize = 8;

/// The server with one enrolled owner, the recovery window open and a
/// member's invite live, so a peer's bytes reach every handshake. The
/// device serves everything at its widest ([`Fake::widest`]), and its writer
/// lands every queued write at the start of the next pass unless held.
pub struct Rig<D = Fake> {
    server: Box<Server>,
    device: D,
    pairing: Pairing,
    identity: Keypair,
    now: u32,
    records: Vec<Record>,
    /// The writer has not reached the queued writes yet.
    writes_held: bool,
}

impl Default for Rig {
    fn default() -> Self {
        Self::new()
    }
}

impl Rig {
    pub fn new() -> Self {
        Self::with(Fake::widest())
    }
}

impl Rig<Fountain> {
    /// A rig whose device is of another kind, with no feeder module.
    pub fn fountain() -> Self {
        Self::with(Fountain(Fake::widest()))
    }
}

impl<D: Faked> Rig<D> {
    fn with(device: D) -> Self {
        let mut rig = Self {
            server: Box::new(Server::new()),
            device,
            pairing: Pairing::new(Config::DEFAULT),
            identity: Keypair::from_secret(FEEDER),
            now: 1_000,
            records: Vec::new(),
            writes_held: false,
        };
        // As the device opens it: the recovery chord held, unlocked.
        let hold = Gesture::Hold {
            chord: Buttons::of(Button::Lock).with(Button::Dispense),
            level: Config::DEFAULT.hold_level,
        };
        let opened = rig.pairing.gesture(rig.now, hold, Lock::Unlocked);
        assert_eq!(opened, Some(PairingEvent::RecoveryOpened));
        let invited = rig.pairing.invite(
            rig.now,
            Role::Owner,
            *Keypair::from_secret(OWNER).public(),
            Role::Member,
            None,
            INVITE_SECRET,
        );
        assert_eq!(invited, PairingEvent::Invited { id: INVITE_ID });

        let mut table = [None; CLIENTS];
        table[0] = Some(Client {
            key: *Keypair::from_secret(OWNER).public(),
            label: Label::new(b"owner").expect("a valid label"),
            role: Role::Owner,
            enrolled: 1,
        });
        *rig.server.clients_table().0 = table;
        let records = &mut rig.records;
        rig.server.clients_changed(1, |r| records.push(r));
        rig
    }

    pub fn accept(&mut self) -> Option<usize> {
        let records = &mut self.records;
        self.server.accept(self.now, |r| records.push(r))
    }

    /// One pass, 20 ms after the last, after the writer's turn: whether it
    /// reports work waiting.
    pub fn pass(&mut self) -> bool {
        self.now += 20;
        if !self.writes_held {
            self.device.fake().write();
        }
        let line = format!(
            "PLTST1 pass now={} {}\r\n",
            self.now,
            "x".repeat(self.now as usize % 120)
        );
        self.device.fake().tail.push(line.as_bytes());
        let records = &mut self.records;
        self.server.step(
            Pass {
                now_ms: self.now,
                device: &mut self.device,
                pairing: &mut self.pairing,
                identity: Some(&self.identity),
                feeding: false,
                recovery: None,
                signer: None,
            },
            |r| records.push(r),
        )
    }

    /// Hold each settings write as the writer not having reached it yet, or
    /// let it finish.
    pub fn hold_writes(&mut self, held: bool) {
        self.writes_held = held;
    }

    /// Who the clock's last accepted fix came from.
    /// The key each sync handed to the clock named.
    pub fn sync_keys(&mut self) -> Vec<[u8; 4]> {
        self.device.fake().sync_keys.clone()
    }

    pub fn clock_origin(&mut self) -> Option<wellfed_core::clock::wall::Origin> {
        self.device
            .fake()
            .clock
            .state(1)
            .last
            .map(|(origin, _)| origin)
    }

    /// Have the writer fail the next settings write, or save it.
    pub fn fail_settings(&mut self, fail: bool) {
        self.device.fake().settings_fails = fail;
    }

    /// Empty the client table, as the writer would publish it: every session
    /// ends.
    pub fn revoke_everyone(&mut self) {
        *self.server.clients_table().0 = [None; CLIENTS];
        let records = &mut self.records;
        self.server.clients_changed(2, |r| records.push(r));
    }

    /// The slots not free.
    pub fn sessions(&self) -> usize {
        self.server.sessions()
    }

    /// The bytes waiting to go out on `slot`.
    pub fn queued(&self, slot: usize) -> usize {
        self.server.tx(slot).len()
    }

    /// Send `len` of the bytes waiting on `slot`, to no one.
    pub fn send_some(&mut self, slot: usize, len: usize) {
        self.server.sent(slot, len);
    }

    /// The reasons recorded for connections ended so far.
    pub fn ended_for(&self) -> Vec<Reason> {
        self.records
            .iter()
            .filter_map(|record| match record {
                Record::Ended { reason, .. } => Some(*reason),
                _ => None,
            })
            .collect()
    }

    /// Hold each queued feed before its reservation, or let them through.
    /// Either way the device takes feeds from now on.
    pub fn hold_reservations(&mut self, held: bool) {
        self.device.fake().blocked = None;
        self.device.fake().hold_feeds = held;
    }

    /// Requests answered from a remembered write id so far.
    pub fn retried(&self) -> usize {
        self.records
            .iter()
            .filter(|record| matches!(record, Record::Retried { .. }))
            .count()
    }

    /// The device refused the feed it numbered `feed`.
    pub fn feed_refused(&mut self, feed: u32, refusal: Refusal) {
        self.server.feed_refused(feed, refusal);
    }

    /// The feeds the device was asked for.
    pub fn feeds(&self) -> usize {
        self.device.fake_ref().feeds.len()
    }

    /// The skips and undos the device was asked for.
    pub fn skips(&self) -> u32 {
        self.device.fake_ref().skips
    }

    /// The device refuses every skip with `refusal`, or takes them.
    pub fn refuse_skips(&mut self, refusal: Option<Refusal>) {
        self.device.fake().skip_refusal = refusal;
    }

    /// The writer drops every skip it is handed, or saves them.
    pub fn drop_skips(&mut self, dropped: bool) {
        self.device.fake().skip_dropped = dropped;
    }

    /// Slot `slot`'s mark.
    pub fn mark(&self, slot: usize) -> u32 {
        self.device.fake_ref().plans.marks[slot]
    }

    /// Every event the device recorded, oldest first.
    pub fn events(&self) -> Vec<EventBody> {
        self.device
            .fake_ref()
            .events
            .iter()
            .map(|e| e.body)
            .collect()
    }

    /// As much of `bytes` as the slot has room for; how much that was.
    pub fn deliver(&mut self, slot: usize, bytes: &[u8]) -> usize {
        let room = self.server.rx(slot);
        let n = room.len().min(bytes.len());
        room[..n].copy_from_slice(&bytes[..n]);
        self.server.received(slot, n);
        n
    }

    /// The frames `slot` sends, each checked against the frame limit.
    pub fn collect(&mut self, slot: usize) -> Vec<Vec<u8>> {
        let out = self.server.tx(slot).to_vec();
        self.server.sent(slot, out.len());

        let mut frames = Vec::new();
        let mut rest = out.as_slice();
        while !rest.is_empty() {
            let prefix = [rest[0], *rest.get(1).expect("a whole length prefix")];
            let len = frame::parse_prefix(prefix, FRAME_MAX).expect("a frame within the limit");
            let body = rest
                .get(frame::PREFIX..frame::PREFIX + len)
                .expect("the whole frame the prefix announces");
            frames.push(body.to_vec());
            rest = &rest[frame::PREFIX + len..];
        }
        frames
    }

    /// Whether the slot ended; it is freed once its last bytes are out.
    pub fn ended(&mut self, slot: usize) -> bool {
        if !self.server.closing(slot) {
            return false;
        }
        let records = &mut self.records;
        self.server.closed(slot, |r| records.push(r));
        true
    }

    /// Enroll a member's key (`MEMBER`) beside the owner.
    pub fn enroll_member(&mut self) {
        let (table, _) = self.server.clients_table();
        table[1] = Some(Client {
            key: *Keypair::from_secret(MEMBER).public(),
            label: Label::new(b"member").expect("a valid label"),
            role: Role::Member,
            enrolled: 2,
        });
        let records = &mut self.records;
        self.server.clients_changed(2, |r| records.push(r));
    }

    /// Open the owner's IK session on a new slot: the slot, and the
    /// client's transport.
    pub fn open(&mut self) -> (usize, Transport) {
        self.open_as(OWNER)
    }

    /// Open an IK session with the enrolled key of secret `secret`.
    pub fn open_as(&mut self, secret: [u8; 32]) -> (usize, Transport) {
        let slot = self.accept().expect("a free slot");
        let mut handshake = Handshake::new(
            Pattern::Ik,
            NoiseRole::Initiator,
            PROLOGUE,
            &Keypair::from_secret(secret),
            Some(*self.identity.public()),
            [0x77; 32],
        )
        .expect("IK with the feeder's key");
        let hello = Hello {
            major: PROTOCOL_MAJOR,
        };
        let mut payload = [0; 16];
        let len = wellfed_proto::encode(&hello, &mut payload).expect("a hello fits");
        let mut message = [0; FRAME_MAX];
        let written = handshake
            .write_message(&payload[..len], &mut message)
            .expect("the first message");

        let mut open = [0; 8];
        let len = wellfed_proto::encode(&Open::Session, &mut open).expect("an open fits");
        let mut bytes = framed(&open[..len]);
        bytes.extend(framed(&message[..written]));
        let delivered = self.deliver(slot, &bytes);
        assert_eq!(delivered, bytes.len());

        for _ in 0..SETTLE {
            self.pass();
            if let Some(reply) = self.collect(slot).first() {
                let mut plain = [0; FRAME_MAX];
                let read = handshake
                    .read_message(reply, &mut plain)
                    .expect("the feeder's answer");
                let welcome: Welcome<'_> =
                    wellfed_proto::decode(&plain[..read]).expect("a welcome");
                assert!(matches!(welcome, Welcome::Open { .. }), "{welcome:?}");
                return (slot, handshake.into_transport().expect("a finished IK"));
            }
        }
        panic!("the feeder never answered the handshake");
    }
}

/// Seal `plain` to the slot; the answers that come within a few passes,
/// as their plaintext.
pub fn send<D: Faked>(
    rig: &mut Rig<D>,
    slot: usize,
    transport: &mut Transport,
    plain: &[u8],
) -> Vec<Vec<u8>> {
    let mut sealed = [0; FRAME_MAX];
    let len = transport.seal(plain, &mut sealed).unwrap();
    let bytes = framed(&sealed[..len]);
    assert_eq!(rig.deliver(slot, &bytes), bytes.len());

    let mut answers = Vec::new();
    for _ in 0..SETTLE {
        rig.pass();
        for reply in rig.collect(slot) {
            let mut out = [0; FRAME_MAX];
            let read = transport.open(&reply, &mut out).unwrap();
            answers.push(out[..read].to_vec());
        }
    }
    answers
}

/// A request's plaintext.
pub fn request(id: RequestId, op: Op<'_>) -> Vec<u8> {
    let mut out = vec![0; FRAME_MAX];
    let len = wellfed_proto::encode(&ClientMessage::Request { id, op }, &mut out).unwrap();
    out.truncate(len);
    out
}

/// A response's id and outcome.
pub fn answer(plain: &[u8]) -> (RequestId, Outcome<'_>) {
    match wellfed_proto::decode::<DeviceMessage<'_>>(plain) {
        Ok(DeviceMessage::Response { id, result }) => (id, result),
        other => panic!("not a response: {other:?}"),
    }
}

pub fn framed(bytes: &[u8]) -> Vec<u8> {
    let mut out = frame::prefix(bytes.len(), usize::from(u16::MAX))
        .expect("a frame under 64 KiB")
        .to_vec();
    out.extend_from_slice(bytes);
    out
}

/// Split fuzzer input into messages: each starts with its length byte.
pub fn split(mut data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    while let Some((&len, rest)) = data.split_first() {
        let len = usize::from(len).min(rest.len());
        out.push(&rest[..len]);
        data = &rest[len..];
    }
    out
}

/// Split fuzzer input into session requests: each starts with a two-byte
/// little-endian length, capped at [`MESSAGE_MAX`], so a request reaches
/// the sealed frame's limit (a whole update chunk, an offer carrying a
/// downgrade statement).
pub fn split_requests(mut data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    while let Some((len, rest)) = data.split_first_chunk::<2>() {
        let len = usize::from(u16::from_le_bytes(*len))
            .min(MESSAGE_MAX)
            .min(rest.len());
        out.push(&rest[..len]);
        data = &rest[len..];
    }
    out
}

/// A peer that has not authenticated: any bytes on a new connection, as
/// they arrive, until the feeder closes it.
pub fn connection(bytes: &[u8]) {
    let mut rig = Rig::new();
    let slot = rig.accept().expect("a free slot");
    let mut rest = bytes;
    let mut idle = 0;

    while idle < SETTLE {
        let n = rig.deliver(slot, rest);
        rest = &rest[n..];
        rig.pass();
        let _ = rig.collect(slot);
        if rig.ended(slot) {
            return;
        }
        if rest.is_empty() {
            idle += 1;
        }
    }
}

/// The owner's session, then one sealed frame per message: every answer
/// opens under the session and decodes as a feeder message. The number of
/// answers.
pub fn session(messages: &[&[u8]]) -> usize {
    let mut rig = Rig::new();
    let (slot, mut transport) = rig.open();
    let mut sealed = [0; FRAME_MAX];
    let mut plain = [0; FRAME_MAX];
    let mut answers = 0;

    for message in messages {
        let message = &message[..message.len().min(MESSAGE_MAX)];
        let len = transport
            .seal(message, &mut sealed)
            .expect("a message within the limit");
        let bytes = framed(&sealed[..len]);
        let mut rest = bytes.as_slice();

        for _ in 0..SETTLE {
            let n = rig.deliver(slot, rest);
            rest = &rest[n..];
            rig.pass();
            for reply in rig.collect(slot) {
                let read = transport
                    .open(&reply, &mut plain)
                    .expect("the feeder seals under the session");
                let decoded = wellfed_proto::decode::<DeviceMessage<'_>>(&plain[..read]);
                assert!(decoded.is_ok(), "{:?}", &plain[..read]);
                answers += 1;
            }
            if rig.ended(slot) {
                return answers;
            }
        }
    }
    answers
}

/// Every slot taken by a session that sends `bytes`, then one more
/// connection: the table stays bounded.
pub fn crowd(bytes: &[u8]) {
    let mut rig = Rig::new();
    let slots: Vec<usize> = (0..SESSIONS).filter_map(|_| rig.accept()).collect();
    assert_eq!(slots.len(), SESSIONS);
    assert_eq!(rig.accept(), None);
    for _ in 0..SETTLE {
        for &slot in &slots {
            rig.deliver(slot, bytes);
        }
        rig.pass();
        for &slot in &slots {
            let _ = rig.collect(slot);
            rig.ended(slot);
        }
    }
}

/// BLE setup from a peer that has not authenticated: any messages, in
/// order. Once one fails, every later one is refused.
pub fn setup_messages(messages: &[&[u8]]) {
    let identity = Keypair::from_secret(FEEDER);
    let mut responder = Responder::new(&identity, [0x55; 32]).expect("a responder");
    let mut out = [0; setup::REPLY_MAX + 64];
    let mut failed = false;

    for message in messages {
        let result = responder.receive(message, &mut out);
        if failed {
            assert!(result.is_err(), "{result:?} after a failure");
        }
        failed |= result.is_err();
    }
}

/// How a genuine client's BLE setup ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Setup {
    /// The feeder refused the request (its label) or a step of the proof.
    Refused,
    /// The proof completed and the feeder took this Wi-Fi.
    Proven(Option<Credentials>),
}

/// How a genuine client's setup must end, by the stores' rules: offered
/// exactly when its label is one the client store takes, proven exactly
/// when both sides hold the same code, and its Wi-Fi saved exactly when
/// the Wi-Fi store takes it. The property test and the fuzz target both
/// hold [`setup_client`] to this.
pub fn expected_setup(
    request: &[u8],
    feeder_code: u16,
    client_code: u16,
    wifi: Option<setup::Wifi<'_>>,
) -> Setup {
    let offered = wellfed_proto::decode::<setup::Request<'_>>(request)
        .is_ok_and(|request| Label::new(request.label.as_bytes()).is_some());
    let codes_match = Code::from_bits(feeder_code) == Code::from_bits(client_code);

    match wifi.map(|wifi| Credentials::new(wifi.ssid, wifi.psk.as_bytes())) {
        _ if !offered || !codes_match => Setup::Refused,
        Some(None) => Setup::Refused,
        Some(Some(credentials)) => Setup::Proven(Some(credentials)),
        None => Setup::Proven(None),
    }
}

/// A genuine client: an XX handshake whose third message carries `request`
/// (any bytes), the owner's `code` keyed in, then the code proof with the
/// client's own `code`, its last step carrying `wifi`. The client's buffers
/// grow with its inputs, so any request or Wi-Fi is sent and the feeder,
/// not this driver, decides what it takes.
pub fn setup_client(
    request: &[u8],
    feeder_code: u16,
    client_code: u16,
    wifi: Option<setup::Wifi<'_>>,
) -> Setup {
    let identity = Keypair::from_secret(FEEDER);
    let mut responder = Responder::new(&identity, [0x55; 32]).expect("a responder");
    let mut client = Handshake::new(
        Pattern::Xx,
        NoiseRole::Initiator,
        setup::PROLOGUE,
        &Keypair::from_secret(OWNER),
        None,
        [0x66; 32],
    )
    .expect("XX needs no remote key");
    // The handshake's static key, the answer's fixed fields and the
    // length prefixes fit in the slack.
    let room = 256 + request.len() + wifi.map_or(0, |w| w.ssid.len() + w.psk.len());
    let (mut message, mut plain) = (vec![0; room], vec![0; room]);
    let mut reply = [0; 512];

    let len = client
        .write_message(&[], &mut message)
        .expect("message one");
    let Ok(Received::Send(n)) = responder.receive(&message[..len], &mut reply) else {
        panic!("the feeder answers the first message");
    };
    client
        .read_message(&reply[..n], &mut plain)
        .expect("message two");
    let Ok(len) = client.write_message(request, &mut message) else {
        return Setup::Refused;
    };
    let offer = match responder.receive(&message[..len], &mut reply) {
        Ok(Received::Offer(offer)) => offer,
        Ok(other) => panic!("{other:?} for the third message"),
        Err(_) => return Setup::Refused,
    };
    assert_eq!(offer.key, *Keypair::from_secret(OWNER).public());

    let hash = *client.handshake_hash();
    let mut transport = client.into_transport().expect("a finished XX");
    let mut exchange = Exchange::new(Side::Client, hash, Code::from_bits(client_code), [0x99; 32]);
    let mut n = responder
        .prove(Code::from_bits(feeder_code), [0x88; 32], &mut reply)
        .expect("the first step of the proof");

    loop {
        let read = transport
            .open(&reply[..n], &mut plain)
            .expect("the feeder seals under the session");
        let Ok(Reply::Round { open, commit }) = wellfed_proto::decode(&plain[..read]) else {
            panic!("a round of the proof");
        };
        if exchange.read(Round { open, commit }).is_err() {
            return Setup::Refused;
        }
        let round = exchange.write().expect("our turn");
        let answer = if exchange.peer_proven() {
            Answer::Last {
                open: round.open.expect("the last step opens"),
                wifi,
            }
        } else {
            Answer::Round {
                open: round.open,
                commit: round.commit,
            }
        };
        let len = wellfed_proto::encode(&answer, &mut plain).expect("an answer fits");
        let len = transport
            .seal(&plain[..len], &mut message)
            .expect("a sealed answer");
        match responder.receive(&message[..len], &mut reply) {
            Ok(Received::Send(next)) => n = next,
            Ok(Received::Proven { wifi }) => return Setup::Proven(wifi),
            Ok(other) => panic!("{other:?} during the proof"),
            Err(_) => return Setup::Refused,
        }
    }
}
