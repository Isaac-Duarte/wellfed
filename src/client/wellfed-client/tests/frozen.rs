//! A whole update against a feeder that knows only the frozen update path,
//! as a bank 0 of an older build does: every update passes through bank 0,
//! which keeps the build it was written with, so a client of any later build
//! must still finish an update with it.
//!
//! The device here declares, on its own, only the frozen variants of the
//! messages ([`ClientMessage::Request`], [`Op::Update`], [`Op::Alive`], and
//! the frozen answers), as a build that knew nothing else would. Its welcome
//! names an older revision. The client offers to bank 1, which gives
//! way; reconnects to bank 0, reads its bank, offers again and pushes every
//! chunk until bank 0 activates. Every request it sends must decode as one
//! of the frozen ops: one that does not is answered `Unavailable`, as a
//! device does, and fails the test.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use wellfed_client::api::Skew;
use wellfed_client::connection::{Connection, Event, Timing};
use wellfed_client::update::{Image, Installer, Next, Sent};
use wellfed_model::{Answer, Op as ModelOp, Refusal as ModelRefusal, UpdatePhase as ModelPhase};
use wellfed_proto::api::{
    Bank0Check, FRAME_MAX, Hello, Open, PROLOGUE, PROTOCOL_MAJOR, REVISION, Refusal, RequestId,
    Role, UPDATE_CHUNK_MAX, UpdateOp, UpdatePhase, UpdateView, WANTED_MAX, Welcome,
};
use wellfed_proto::device::Model;
use wellfed_session::frame::{self, PREFIX};
use wellfed_session::noise::{Handshake, Keypair, Pattern, Role as Side, Transport};

const FEEDER: [u8; 32] = [0x10; 32];
const OWNER: [u8; 32] = [0x20; 32];
/// The older build's revision.
const OLD_REVISION: u32 = REVISION - 1;
/// Where the revision now goes, a pre-revision build sent master's schema
/// hash: the last value the bench unit's bank 0 and every feeder updated
/// before this build carry.
const PRE_REVISION: u32 = 0x2af2_8cd8;

/// What an older build decodes: the frozen request alone.
#[derive(Debug, Deserialize)]
enum ClientMessage<'a> {
    Request {
        id: RequestId,
        #[serde(borrow)]
        op: Op<'a>,
    },
}

/// The frozen ops alone, at their frozen indices.
#[derive(Debug, Deserialize)]
enum Op<'a> {
    #[serde(borrow)]
    Update(UpdateOp<'a>),
    Alive,
}

/// The frozen answer alone.
#[derive(Serialize)]
enum DeviceMessage {
    Response { id: RequestId, result: Outcome },
}

/// The frozen answers alone, at their frozen indices.
#[derive(Serialize)]
enum Outcome {
    Done { generation: Option<u32> },
    Refused(Refusal),
    Update(UpdateView),
}

enum Stage {
    Opening,
    Handshake(Box<Handshake>),
    Open(Box<Transport>),
}

/// One connection to the device.
struct Link {
    stage: Stage,
    buffer: Vec<u8>,
    out: VecDeque<Vec<u8>>,
}

/// A feeder of an older build, both banks, with an install that keeps
/// only what the frozen answers name.
struct OldBuild {
    identity: Keypair,
    bank: u8,
    link: Option<Link>,
    /// The image the manifest names, by length: the offer is not checked.
    image_len: u32,
    image: Vec<Option<Vec<u8>>>,
    phase: UpdatePhase,
    /// Once this answer is out the device restarts: into bank 0 after a
    /// give-way, into bank 1 after an activation.
    restart: Option<u8>,
    /// Every op it decoded, by name, and the requests it could not.
    ops: Vec<&'static str>,
    unknown: usize,
    /// What its welcome carries where the revision goes.
    schema: u32,
}

impl OldBuild {
    fn new(image_len: u32) -> Self {
        let chunks = usize::try_from(image_len)
            .unwrap()
            .div_ceil(UPDATE_CHUNK_MAX);
        Self {
            identity: Keypair::from_secret(FEEDER),
            bank: 1,
            link: None,
            image_len,
            image: vec![None; chunks],
            phase: UpdatePhase::Idle,
            restart: None,
            ops: Vec::new(),
            unknown: 0,
            schema: OLD_REVISION,
        }
    }

    /// A pre-revision build: its welcome carries a schema hash.
    fn pre_revision(image_len: u32) -> Self {
        Self {
            schema: PRE_REVISION,
            ..Self::new(image_len)
        }
    }

    fn accept(&mut self) {
        self.link = Some(Link {
            stage: Stage::Opening,
            buffer: Vec::new(),
            out: VecDeque::new(),
        });
    }

    fn view(&self) -> UpdateView {
        let mut wanted = [None; WANTED_MAX];
        let missing = self.image.iter().enumerate().filter(|(_, c)| c.is_none());
        for (slot, (index, _)) in wanted.iter_mut().zip(missing) {
            *slot = Some(u32::try_from(index * UPDATE_CHUNK_MAX).unwrap());
        }
        let written = self.image.iter().flatten().map(Vec::len).sum::<usize>();
        UpdateView {
            phase: self.phase,
            bank: self.bank,
            min_version: 1,
            digest: None,
            version: 2,
            image_len: self.image_len,
            written: u32::try_from(written).unwrap(),
            wanted: if self.phase == UpdatePhase::Receiving {
                wanted
            } else {
                [None; WANTED_MAX]
            },
            stopped: None,
            bank0: Bank0Check::Unchecked,
        }
    }

    /// Bytes from the client.
    fn receive(&mut self, bytes: &[u8]) {
        let Some(link) = &mut self.link else {
            return;
        };
        link.buffer.extend_from_slice(bytes);
        while let Some(prefix) = self
            .link
            .as_ref()
            .and_then(|link| link.buffer.first_chunk::<PREFIX>().copied())
        {
            let len = frame::parse_prefix(prefix, FRAME_MAX).expect("a frame within the limit");
            let link = self.link.as_mut().unwrap();
            if link.buffer.len() < PREFIX + len {
                return;
            }
            let message: Vec<u8> = link.buffer.drain(..PREFIX + len).skip(PREFIX).collect();
            self.message(&message);
        }
    }

    fn message(&mut self, message: &[u8]) {
        let link = self.link.as_mut().unwrap();
        let mut plain = [0; FRAME_MAX];
        match &mut link.stage {
            Stage::Opening => {
                assert_eq!(wellfed_proto::decode::<Open>(message), Ok(Open::Session));
                let handshake = Handshake::new(
                    Pattern::Ik,
                    Side::Responder,
                    PROLOGUE,
                    &self.identity,
                    None,
                    [0x44; 32],
                )
                .unwrap();
                link.stage = Stage::Handshake(Box::new(handshake));
            }
            Stage::Handshake(handshake) => {
                let len = handshake.read_message(message, &mut plain).unwrap();
                let hello: Hello = wellfed_proto::decode(&plain[..len]).unwrap();
                assert_eq!(hello.major, PROTOCOL_MAJOR);
                assert_eq!(
                    handshake.remote_static(),
                    Some(Keypair::from_secret(OWNER).public())
                );
                let welcome = Welcome::Open {
                    major: PROTOCOL_MAJOR,
                    model: Model::Plaf103,
                    build: if self.bank == 0 {
                        "old-bank0"
                    } else {
                        "old-bank1"
                    },
                    role: Role::Owner,
                    epoch: 7,
                    schema: self.schema,
                };
                let len = wellfed_proto::encode(&welcome, &mut plain).unwrap();
                let mut sealed = [0; FRAME_MAX];
                let sealed_len = handshake.write_message(&plain[..len], &mut sealed).unwrap();
                link.out.push_back(framed(&sealed[..sealed_len]));
                let Stage::Handshake(handshake) =
                    std::mem::replace(&mut link.stage, Stage::Opening)
                else {
                    unreachable!()
                };
                link.stage = Stage::Open(Box::new(handshake.into_transport().unwrap()));
            }
            Stage::Open(transport) => {
                let len = transport.open(message, &mut plain).unwrap();
                let (id, result) = self.request(&plain[..len]);
                self.answer(id, &result);
            }
        }
    }

    /// One request: what a device of an older build answers.
    fn request(&mut self, plain: &[u8]) -> (RequestId, Outcome) {
        let request = match wellfed_proto::decode::<ClientMessage<'_>>(plain) {
            Ok(ClientMessage::Request { id, op }) => (id, op),
            Err(_) => {
                // The id is readable behind the frozen request's tag.
                self.unknown += 1;
                let id = plain[1..17].try_into().expect("a request id");
                return (id, Outcome::Refused(Refusal::Unavailable));
            }
        };
        let (id, op) = request;
        let result = match op {
            Op::Alive => {
                self.ops.push("alive");
                Outcome::Done { generation: None }
            }
            Op::Update(UpdateOp::State) => {
                self.ops.push("state");
                Outcome::Update(self.view())
            }
            Op::Update(UpdateOp::Offer { .. }) if self.bank == 1 => {
                self.ops.push("offer");
                self.phase = UpdatePhase::GivingWay;
                self.restart = Some(0);
                Outcome::Update(self.view())
            }
            Op::Update(UpdateOp::Offer { .. }) => {
                self.ops.push("offer");
                self.phase = UpdatePhase::Receiving;
                Outcome::Update(self.view())
            }
            Op::Update(UpdateOp::Chunk { offset, data }) => {
                self.ops.push("chunk");
                let index = usize::try_from(offset).unwrap() / UPDATE_CHUNK_MAX;
                self.image[index] = Some(data.to_vec());
                if self.image.iter().all(Option::is_some) {
                    self.phase = UpdatePhase::Activated;
                    self.restart = Some(1);
                }
                Outcome::Update(self.view())
            }
            Op::Update(UpdateOp::Cancel) => {
                self.ops.push("cancel");
                self.phase = UpdatePhase::Idle;
                Outcome::Update(self.view())
            }
            Op::Update(UpdateOp::EnrollKey { .. } | UpdateOp::KeyStatement { .. }) => {
                self.ops.push("key");
                Outcome::Done { generation: None }
            }
        };
        (id, result)
    }

    fn answer(&mut self, id: RequestId, result: &Outcome) {
        let link = self.link.as_mut().unwrap();
        let Stage::Open(transport) = &mut link.stage else {
            unreachable!()
        };
        let mut plain = [0; FRAME_MAX];
        let message = DeviceMessage::Response {
            id,
            result: match result {
                Outcome::Done { generation } => Outcome::Done {
                    generation: *generation,
                },
                Outcome::Refused(refusal) => Outcome::Refused(*refusal),
                Outcome::Update(view) => Outcome::Update(*view),
            },
        };
        let len = wellfed_proto::encode(&message, &mut plain).unwrap();
        let mut sealed = [0; FRAME_MAX];
        let sealed_len = transport.seal(&plain[..len], &mut sealed).unwrap();
        link.out.push_back(framed(&sealed[..sealed_len]));
    }

    /// Seal `plain` as a message of its own, as a newer build sends one.
    fn send_plain(&mut self, plain: &[u8]) {
        let link = self.link.as_mut().unwrap();
        let Stage::Open(transport) = &mut link.stage else {
            unreachable!()
        };
        let mut sealed = [0; FRAME_MAX];
        let sealed_len = transport.seal(plain, &mut sealed).unwrap();
        link.out.push_back(framed(&sealed[..sealed_len]));
    }

    /// What the device sends; a restart it owes happens once that is out.
    fn transmit(&mut self) -> Vec<Vec<u8>> {
        let out = self
            .link
            .as_mut()
            .map(|link| link.out.drain(..).collect())
            .unwrap_or_default();
        if let Some(bank) = self.restart.take() {
            self.link = None;
            self.bank = bank;
            if bank == 0 {
                self.phase = UpdatePhase::Idle;
            }
        }
        out
    }
}

fn framed(message: &[u8]) -> Vec<u8> {
    let mut out = frame::prefix(message.len(), FRAME_MAX).unwrap().to_vec();
    out.extend_from_slice(message);
    out
}

/// The client's side: one session at a time, and the requests in flight.
struct Client {
    now: Instant,
    connection: Connection,
    next_id: u128,
    sent: Vec<(RequestId, Sent)>,
}

impl Client {
    /// A session with `device`, open once the welcome is in.
    fn connect(device: &mut OldBuild, now: Instant) -> Self {
        device.accept();
        let connection = Connection::new(
            &Keypair::from_secret(OWNER),
            *Keypair::from_secret(FEEDER).public(),
            [0x33; 32],
            now,
        )
        .unwrap();
        let mut client = Self {
            now,
            connection,
            next_id: 1,
            sent: Vec::new(),
        };
        client.pump(device);
        let opened = client.connection.opened().expect("the welcome");
        let expected = if device.schema == PRE_REVISION {
            Skew::Legacy
        } else {
            Skew::FeederBehind
        };
        assert_eq!(opened.skew(), expected, "the device plays an older build");
        client
    }

    fn id(&mut self) -> RequestId {
        self.next_id += 1;
        self.next_id.to_le_bytes()
    }

    /// Move bytes both ways until neither side has more.
    fn pump(&mut self, device: &mut OldBuild) {
        loop {
            let mut moved = false;
            while let Some(bytes) = self.connection.poll_transmit() {
                device.receive(&bytes);
                moved = true;
            }
            for bytes in device.transmit() {
                self.connection.handle_input(self.now, &bytes).unwrap();
                moved = true;
            }
            if !moved {
                return;
            }
        }
    }

    /// Ask `op` and wait for its answer.
    fn ask(&mut self, device: &mut OldBuild, op: &ModelOp) -> Answer {
        let id = self.id();
        self.connection.request(self.now, id, op).unwrap();
        self.pump(device);
        match self.connection.poll_event() {
            Some(Event::Answer {
                id: answered,
                answer,
            }) if answered == id => answer,
            other => panic!("no answer to {op:?}: {other:?}"),
        }
    }

    /// The bank the session answers from, as an update driver reads it:
    /// from the frozen update state, never the status.
    fn bank(&mut self, device: &mut OldBuild) -> u8 {
        match self.ask(device, &ModelOp::UpdateState) {
            Answer::Update(view) => view.bank,
            other => panic!("not the update state: {other:?}"),
        }
    }
}

fn payload() -> Vec<u8> {
    // Three chunks short of three sectors' worth, the last one partial.
    (0..UPDATE_CHUNK_MAX * 20 + 100)
        .map(|i| u8::try_from(i % 251).unwrap())
        .collect()
}

#[test]
fn an_update_goes_through_a_bank_0_of_another_build() {
    update_through(OldBuild::new);
}

/// The update path is all a pre-revision build takes, and all it needs:
/// the bench unit's bank 0 and every feeder updated before this build.
#[test]
fn an_update_goes_through_a_pre_revision_bank_0() {
    update_through(OldBuild::pre_revision);
}

fn update_through(build: fn(u32) -> OldBuild) {
    let payload = payload();
    let manifest = [0x5a; 96];
    let image = Image {
        manifest: &manifest,
        downgrade: None,
        payload: &payload,
    };
    let mut device = build(u32::try_from(payload.len()).unwrap());
    let mut installer = Installer::new(image);
    let mut client = Client::connect(&mut device, Instant::now());
    let bank = client.bank(&mut device);
    assert_eq!(bank, 1);
    installer.opened(bank);

    let mut reconnects = 0;
    let activated = loop {
        match installer.poll() {
            Next::Send(sent, op) => {
                assert!(op.is_frozen(), "{op:?} is not on the frozen update path");
                let id = client.id();
                client.connection.request(client.now, id, &op).unwrap();
                client.sent.push((id, sent));
                client.pump(&mut device);
                while let Some(event) = client.connection.poll_event() {
                    let Event::Answer { id, answer } = event else {
                        panic!("not an answer: {event:?}");
                    };
                    let index = client.sent.iter().position(|(s, _)| *s == id).unwrap();
                    let (_, sent) = client.sent.swap_remove(index);
                    let _ = installer.answer(sent, &answer);
                }
            }
            Next::Wait => client.now += Duration::from_millis(200),
            Next::Reconnect => {
                reconnects += 1;
                assert!(device.link.is_none(), "bank 1 restarted into bank 0");
                installer.lost();
                client = Client::connect(&mut device, client.now + Duration::from_secs(5));
                let bank = client.bank(&mut device);
                assert_eq!(bank, 0);
                installer.opened(bank);
            }
            Next::Activated => break true,
            Next::Failed(error) => panic!("the install failed: {error}"),
            Next::Cancelled => break false,
        }
    };

    assert!(activated);
    assert_eq!(reconnects, 1, "one give-way, then bank 0 installs");
    assert_eq!(
        installer.view().map(|view| view.phase),
        Some(ModelPhase::Activated)
    );
    let written: Vec<u8> = device.image.iter().flatten().flatten().copied().collect();
    assert_eq!(written, payload, "bank 0 received the whole image");
    assert_eq!(device.unknown, 0, "every request was a frozen op");
    assert_eq!(device.ops.iter().filter(|op| **op == "offer").count(), 2);
    assert_eq!(device.bank, 1, "bank 0 activated and restarted into bank 1");
}

/// On a session with an older build every op goes: the API only grows, so
/// an op the build knows means the same there, and one it lacks comes back
/// `Unavailable` with the session kept. The keepalive is the frozen
/// `Alive`, and its answer keeps the session open.
#[test]
fn a_session_with_an_older_build_sends_every_op_and_reads_unavailable() {
    let mut device = OldBuild::new(1_000);
    let now = Instant::now();
    let mut client = Client::connect(&mut device, now);

    for op in [ModelOp::Status, ModelOp::ListClients] {
        let answer = client.ask(&mut device, &op);
        assert_eq!(answer, Answer::Refused(ModelRefusal::Unavailable), "{op:?}");
    }
    assert_eq!(
        device.unknown, 2,
        "both went, and the build could not read them"
    );
    assert!(client.connection.opened().is_some(), "the session is kept");

    // Quiet past the keepalive: `Alive` goes, and its answer is swallowed.
    client.now += Timing::DEFAULT.keepalive;
    client.connection.handle_timeout(client.now).unwrap();
    client.pump(&mut device);
    assert_eq!(device.ops, ["alive"]);
    assert_eq!(client.connection.poll_event(), None);
    // Heard within the silence limit, so the session lives on.
    client.now += Timing::DEFAULT.keepalive;
    client.connection.handle_timeout(client.now).unwrap();

    // The frozen path still goes.
    assert!(matches!(
        client.ask(&mut device, &ModelOp::UpdateState),
        Answer::Update(_)
    ));
}

/// A pre-revision build's bytes outside the frozen part moved since: no
/// status read and no subscription goes to it, each refused before a byte
/// is sent, while the update path and the keepalive still do.
#[test]
fn a_pre_revision_build_is_sent_the_frozen_part_alone() {
    let mut device = OldBuild::pre_revision(1_000);
    let mut client = Client::connect(&mut device, Instant::now());

    for op in [ModelOp::Status, ModelOp::ListClients, ModelOp::GetSettings] {
        let id = client.id();
        let refused = client.connection.request(client.now, id, &op);
        assert!(
            matches!(refused, Err(wellfed_client::api::Error::Legacy)),
            "{op:?}: {refused:?}"
        );
    }
    let refused = client.connection.subscribe(client.now, None, false);
    assert!(
        matches!(refused, Err(wellfed_client::api::Error::Legacy)),
        "{refused:?}"
    );
    assert!(client.connection.poll_transmit().is_none(), "nothing went");
    client.pump(&mut device);
    assert_eq!(device.unknown, 0);
    assert!(device.ops.is_empty(), "{:?}", device.ops);

    assert!(matches!(
        client.ask(&mut device, &ModelOp::UpdateState),
        Answer::Update(_)
    ));
    assert!(client.connection.opened().is_some(), "the session is kept");
}

/// A message kind a later build adds goes at the end of `DeviceMessage`,
/// in a sealed frame of its own: this build skips it whole, by its tag, and
/// reads the next message on the same session.
#[test]
fn a_newer_kind_of_message_is_skipped_and_the_session_kept() {
    let mut device = OldBuild::new(1_000);
    let mut client = Client::connect(&mut device, Instant::now());

    let past = u8::try_from(wellfed_proto::api::DEVICE_MESSAGES).unwrap();
    device.send_plain(&[past, 1, 2, 3]);
    device.send_plain(&[0x80, 0x01, 9]);
    assert!(matches!(
        client.ask(&mut device, &ModelOp::UpdateState),
        Answer::Update(_)
    ));
    assert!(client.connection.opened().is_some(), "the session is kept");
}
