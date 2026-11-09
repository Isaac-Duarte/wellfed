//! A feeder whose welcome carries a pre-revision schema hash, as the bench
//! unit's bank 0 and every feeder updated before API revisions send it. It
//! answers the frozen update path alone (an idle update state, the
//! keepalive) and refuses anything else, and it records what each client
//! sent, so a test sees that nothing past the frozen part went out.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex, PoisonError};

use wellfed_proto::api::{
    Bank0Check, ClientMessage, DeviceMessage, FRAME_MAX, Op, Open, Outcome, PROLOGUE,
    PROTOCOL_MAJOR, Refusal, Role, UpdateOp, UpdatePhase, UpdateView, WANTED_MAX, Welcome,
};
use wellfed_session::frame::{self, PREFIX};
use wellfed_session::noise::{Handshake, Keypair, Pattern, Role as Side};

/// Master's schema hash before API revisions, the last a pre-revision
/// build sent.
pub const SCHEMA: u32 = 0x2af2_8cd8;

const SECRET: [u8; 32] = [0x10; 32];

/// What the old build read from a client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seen {
    /// A frozen update op.
    Update,
    Alive,
    /// A request outside the frozen part, which its build would misread.
    Other,
    Subscribe,
}

/// The old build, serving each connection on a thread of its own.
pub struct PreRevision {
    pub address: SocketAddr,
    pub identity: [u8; 32],
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl PreRevision {
    /// Start it; any client may open a session, as its owner.
    pub fn start() -> Self {
        let listener =
            TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a loopback listener binds");
        let address = listener.local_addr().expect("a bound listener's address");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let served = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let served = Arc::clone(&served);
                std::thread::spawn(move || serve(stream, &served));
            }
        });
        Self {
            address,
            identity: *Keypair::from_secret(SECRET).public(),
            seen,
        }
    }

    /// Everything clients sent so far, in order.
    pub fn seen(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn read_frame(stream: &mut TcpStream) -> Option<Vec<u8>> {
    let mut prefix = [0; PREFIX];
    stream.read_exact(&mut prefix).ok()?;
    let len = frame::parse_prefix(prefix, FRAME_MAX)?;
    let mut body = vec![0; len];
    stream.read_exact(&mut body).ok()?;
    Some(body)
}

fn write_frame(stream: &mut TcpStream, body: &[u8]) -> Option<()> {
    stream
        .write_all(&frame::prefix(body.len(), FRAME_MAX)?)
        .ok()?;
    stream.write_all(body).ok()
}

fn idle() -> UpdateView {
    UpdateView {
        phase: UpdatePhase::Idle,
        bank: 0,
        min_version: 1,
        digest: None,
        version: 0,
        image_len: 0,
        written: 0,
        wanted: [None; WANTED_MAX],
        stopped: None,
        bank0: Bank0Check::Unchecked,
    }
}

/// One session, until the client closes it. The frozen ops decode the same
/// under this build's types, so they stand in for the old build's.
fn serve(mut stream: TcpStream, seen: &Mutex<Vec<Seen>>) -> Option<()> {
    let identity = Keypair::from_secret(SECRET);
    let open = read_frame(&mut stream)?;
    if wellfed_proto::decode::<Open>(&open) != Ok(Open::Session) {
        return None;
    }
    let mut handshake = Handshake::new(
        Pattern::Ik,
        Side::Responder,
        PROLOGUE,
        &identity,
        None,
        [9; 32],
    )
    .ok()?;
    let mut plain = [0; FRAME_MAX];
    let mut sealed = [0; FRAME_MAX];
    handshake
        .read_message(&read_frame(&mut stream)?, &mut plain)
        .ok()?;
    let welcome = Welcome::Open {
        major: PROTOCOL_MAJOR,
        model: wellfed_proto::device::Model::Plaf103,
        build: "pre-revision",
        role: Role::Owner,
        epoch: 1,
        schema: SCHEMA,
    };
    let len = wellfed_proto::encode(&welcome, &mut plain).ok()?;
    let written = handshake.write_message(&plain[..len], &mut sealed).ok()?;
    write_frame(&mut stream, &sealed[..written])?;
    let mut transport = handshake.into_transport().ok()?;

    loop {
        let message = read_frame(&mut stream)?;
        let len = transport.open(&message, &mut plain).ok()?;
        let (kind, answer) = match wellfed_proto::decode::<ClientMessage<'_>>(&plain[..len]) {
            Ok(ClientMessage::Request { id, op }) => match op {
                Op::Update(UpdateOp::State) => (Seen::Update, Some((id, Outcome::Update(idle())))),
                Op::Update(_) => (Seen::Update, Some((id, Outcome::Refused(Refusal::Busy)))),
                Op::Alive => (Seen::Alive, Some((id, Outcome::Done { generation: None }))),
                _ => (
                    Seen::Other,
                    Some((id, Outcome::Refused(Refusal::Unavailable))),
                ),
            },
            Ok(ClientMessage::Subscribe { .. }) => (Seen::Subscribe, None),
            Err(_) => (Seen::Other, None),
        };
        seen.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(kind);

        let Some((id, result)) = answer else {
            continue;
        };
        let len =
            wellfed_proto::encode(&DeviceMessage::Response { id, result }, &mut plain).ok()?;
        let written = transport.seal(&plain[..len], &mut sealed).ok()?;
        write_frame(&mut stream, &sealed[..written])?;
    }
}
