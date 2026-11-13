//! The rig that steps the server one pass at a time against the device
//! API's fake device ([`wellfed_device_api::fake`]), and the helpers that
//! pump bytes between it and a session or a connection.

use std::time::{Duration, Instant};

use wellfed_client::api::{Error, Incoming, Session};
use wellfed_client::connection::{Connection, Event, Timing};
pub(crate) use wellfed_device_api::fake::{EPOCH, FEEDER, Fake, MEMBER, OWNER, public, receiving};
use wellfed_device_api::pairing::{Config, Pairing};
use wellfed_device_api::{Pass, Reason, Record, Server};
use wellfed_model as model;
use wellfed_model::{Answer, Hello, Op};
use wellfed_proto::api::{self as proto, UpdateView};
use wellfed_session::noise::Keypair;

pub(crate) const STRANGER: [u8; 32] = [0x40; 32];

/// The server, its device and pairing, stepped one pass at a time.
pub(crate) struct Rig {
    pub(crate) server: Box<Server>,
    pub(crate) device: Fake,
    pub(crate) pairing: Pairing,
    pub(crate) identity: Keypair,
    pub(crate) now: u32,
    pub(crate) feeding: bool,
    /// Pairing's decision on a recovery offer, for the next pass.
    pub(crate) recovery: Option<wellfed_device_api::pairing::Event>,
    /// Pairing's decision on an update-key offer, for the next pass.
    pub(crate) signer: Option<wellfed_device_api::pairing::Event>,
    pub(crate) records: Vec<Record>,
    pub(crate) clients_generation: u32,
}

impl Rig {
    pub(crate) fn new() -> Self {
        let mut rig = Self {
            server: Box::new(Server::new()),
            device: Fake::new(),
            pairing: Pairing::new(Config::DEFAULT),
            identity: Keypair::from_secret(FEEDER),
            now: 1_000,
            feeding: false,
            recovery: None,
            signer: None,
            records: Vec::new(),
            clients_generation: 0,
        };
        rig.publish();
        rig
    }

    /// The writer publishes the client table, as `keys::clients_since`.
    pub(crate) fn publish(&mut self) {
        self.clients_generation += 1;
        let table = self.device.table();
        *self.server.clients_table().0 = table;
        let records = &mut self.records;
        self.server
            .clients_changed(self.clients_generation, |r| records.push(r));
    }

    /// One pass; whether the server has work waiting.
    pub(crate) fn pass(&mut self) -> bool {
        self.now += 20;
        let records = &mut self.records;
        self.server.step(
            Pass {
                now_ms: self.now,
                device: &mut self.device,
                pairing: &mut self.pairing,
                identity: Some(&self.identity),
                feeding: self.feeding,
                recovery: self.recovery.take(),
                signer: self.signer.take(),
            },
            |r| records.push(r),
        )
    }

    pub(crate) fn accept(&mut self) -> Option<usize> {
        let records = &mut self.records;
        self.server.accept(self.now, |r| records.push(r))
    }

    /// Client bytes into `slot`, as far as its buffer has room.
    pub(crate) fn deliver(&mut self, slot: usize, bytes: &mut Vec<u8>) {
        let room = self.server.rx(slot);
        let n = room.len().min(bytes.len());
        room[..n].copy_from_slice(&bytes[..n]);
        self.server.received(slot, n);
        bytes.drain(..n);
    }

    /// Everything `slot` has to send.
    pub(crate) fn collect(&mut self, slot: usize) -> Vec<u8> {
        let out = self.server.tx(slot).to_vec();
        self.server.sent(slot, out.len());
        out
    }

    pub(crate) fn ended(&self, reason: Reason) -> bool {
        self.records
            .iter()
            .any(|r| matches!(r, Record::Ended { reason: got, .. } if *got == reason))
    }
}

/// One client on one slot.
pub(crate) struct Client_ {
    pub(crate) slot: usize,
    pub(crate) session: Session,
    pub(crate) outbox: Vec<u8>,
}

/// Run passes, moving bytes both ways, until `done` or a bound.
pub(crate) fn run(rig: &mut Rig, client: &mut Client_, mut done: impl FnMut(&mut Session) -> bool) {
    for _ in 0..200 {
        rig.deliver(client.slot, &mut client.outbox);
        rig.pass();
        let bytes = rig.collect(client.slot);
        if !bytes.is_empty() {
            client.session.receive(&bytes).unwrap();
        }
        rig.device.write();
        if done(&mut client.session) {
            return;
        }
    }
    panic!("no progress");
}

pub(crate) fn connect(rig: &mut Rig, secret: [u8; 32]) -> (Client_, Option<Incoming>) {
    let slot = rig.accept().unwrap();
    let (session, outbox) =
        Session::connect(&Keypair::from_secret(secret), public(FEEDER), [0x77; 32]).unwrap();
    let mut client = Client_ {
        slot,
        session,
        outbox,
    };
    let mut welcome = None;
    run(rig, &mut client, |session| {
        if let Some(incoming @ (Incoming::Opened(_) | Incoming::Refused(_))) =
            session.next_incoming().unwrap()
        {
            welcome = Some(incoming);
            return true;
        }
        false
    });
    (client, welcome)
}

pub(crate) fn open(rig: &mut Rig, secret: [u8; 32]) -> Client_ {
    let (client, welcome) = connect(rig, secret);
    assert!(matches!(welcome, Some(Incoming::Opened(_))), "{welcome:?}");
    client
}

/// Send one request and run until its answer.
pub(crate) fn ask(rig: &mut Rig, client: &mut Client_, id: u8, op: Op) -> Answer {
    let bytes = client.session.request([id; 16], &op).unwrap();
    client.outbox.extend(bytes);
    let mut answer = None;
    run(rig, client, |session| {
        while let Some(incoming) = session.next_incoming().unwrap() {
            if let Incoming::Answer {
                id: got,
                answer: got_answer,
            } = incoming
                && got == [id; 16]
            {
                answer = Some(got_answer);
                return true;
            }
        }
        false
    });
    answer.unwrap()
}

/// Send a request and run one pass without the writer's turn, so a write it
/// queues stays queued.
pub(crate) fn send_unwritten(rig: &mut Rig, client: &mut Client_, id: u8, op: Op) {
    let bytes = client.session.request([id; 16], &op).unwrap();
    client.outbox.extend(bytes);
    rig.deliver(client.slot, &mut client.outbox);
    rig.pass();
}

/// Passes with the writer held off, collecting what the session is sent:
/// whether it heard an answer to `id`.
pub(crate) fn answered_unwritten(rig: &mut Rig, client: &mut Client_, id: u8) -> bool {
    let mut answered = false;
    for _ in 0..5 {
        rig.deliver(client.slot, &mut client.outbox);
        rig.pass();
        let bytes = rig.collect(client.slot);
        if !bytes.is_empty() {
            client.session.receive(&bytes).unwrap();
        }
        while let Some(incoming) = client.session.next_incoming().unwrap() {
            answered |= matches!(incoming, Incoming::Answer { id: got, .. } if got == [id; 16]);
        }
    }
    answered
}

pub(crate) fn fingerprint(secret: [u8; 32]) -> proto::Fingerprint {
    let key = public(secret);
    [key[0], key[1], key[2], key[3]]
}

/// One sans-IO connection on one slot, with the client's clock tied to the
/// rig's passes.
pub(crate) struct Live {
    pub(crate) slot: usize,
    pub(crate) connection: Connection,
    pub(crate) base: Instant,
    pub(crate) outbox: Vec<u8>,
    pub(crate) events: Vec<Event>,
}

impl Live {
    pub(crate) fn now(&self, rig: &Rig) -> Instant {
        self.base + Duration::from_millis(u64::from(rig.now))
    }
}

pub(crate) fn live_with(rig: &mut Rig, secret: [u8; 32], hello: Hello) -> Live {
    let slot = rig.accept().unwrap();
    let base = Instant::now();
    let now = base + Duration::from_millis(u64::from(rig.now));
    let keys = Keypair::from_secret(secret);
    let connection = Connection::with(
        &keys,
        public(FEEDER),
        [0x77; 32],
        hello,
        Timing::DEFAULT,
        now,
    )
    .unwrap();
    Live {
        slot,
        connection,
        base,
        outbox: Vec::new(),
        events: Vec::new(),
    }
}

/// Passes, moving bytes both ways and collecting events, until `done` or
/// the connection fails.
pub(crate) fn drive(
    rig: &mut Rig,
    live: &mut Live,
    mut done: impl FnMut(&Live) -> bool,
) -> Result<(), Error> {
    for _ in 0..200 {
        while let Some(bytes) = live.connection.poll_transmit() {
            live.outbox.extend(bytes);
        }
        rig.deliver(live.slot, &mut live.outbox);
        rig.pass();
        let bytes = rig.collect(live.slot);
        rig.device.write();
        if !bytes.is_empty() {
            let now = live.now(rig);
            live.connection.handle_input(now, &bytes)?;
        }
        while let Some(event) = live.connection.poll_event() {
            live.events.push(event);
        }
        if done(live) {
            return Ok(());
        }
    }
    panic!("no progress");
}

pub(crate) fn live(rig: &mut Rig, secret: [u8; 32]) -> Live {
    let hello = Hello { major: 1 };
    let mut live = live_with(rig, secret, hello);
    drive(rig, &mut live, |live| live.connection.opened().is_some()).unwrap();
    live
}

pub(crate) fn pushed(live: &Live) -> Vec<model::Event> {
    live.events
        .iter()
        .filter_map(|event| match event {
            Event::Pushed(model::Pushed::Event(event)) => Some(*event),
            _ => None,
        })
        .collect()
}

/// The feeder's view as a client reads it.
pub(crate) fn view(view: UpdateView) -> Answer {
    Answer::Update(view.into())
}

pub(crate) fn poll_answer(
    rig: &mut Rig,
    client: &mut Client_,
    id: u8,
    passes: usize,
) -> Option<Answer> {
    for _ in 0..passes {
        rig.deliver(client.slot, &mut client.outbox);
        rig.pass();
        let bytes = rig.collect(client.slot);
        if !bytes.is_empty() {
            client.session.receive(&bytes).unwrap();
        }
        rig.device.write();
        while let Some(incoming) = client.session.next_incoming().unwrap() {
            if let Incoming::Answer { id: got, answer } = incoming
                && got == [id; 16]
            {
                return Some(answer);
            }
        }
    }
    None
}
