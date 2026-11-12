//! A feeder of a newer build than this one, as a client meets one once the
//! public beta has shipped: its welcome names a later revision, and it
//! answers in values this build cannot read. Each feeder write is answered
//! with an outcome tagged past every one this build knows, every other
//! request is refused `Unavailable`, and a session that subscribes is sent
//! a live sample of a metric this build does not know. It records the
//! feeder writes it was sent, so a test sees what a client sent again.

use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex, PoisonError};

use wellfed_proto::api::{
    ClientMessage, DeviceMessage, FRAME_MAX, Op, Outcome, REVISION, Refusal, RequestId,
};
use wellfed_session::noise::Keypair;

use super::peer::Session;

const SECRET: [u8; 32] = [0x11; 32];

/// Postcard's index of `DeviceMessage::Response`, frozen.
const RESPONSE: u8 = 0;
/// Postcard's index of `DeviceMessage::Sample`.
const SAMPLE: u8 = 3;
/// An index past every variant of `Outcome` and of `Metric` this build
/// knows, as a later build's would be.
const LATER: u8 = 0x7f;

/// The newer build, serving each connection on a thread of its own.
pub struct Newer {
    pub address: SocketAddr,
    pub identity: [u8; 32],
    writes: Arc<Mutex<Vec<RequestId>>>,
}

impl Newer {
    /// Start it; any client may open a session, as its owner.
    pub fn start() -> Self {
        let listener =
            TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a loopback listener binds");
        let address = listener.local_addr().expect("a bound listener's address");
        let writes = Arc::new(Mutex::new(Vec::new()));
        let served = Arc::clone(&writes);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let served = Arc::clone(&served);
                std::thread::spawn(move || serve(stream, &served));
            }
        });
        Self {
            address,
            identity: *Keypair::from_secret(SECRET).public(),
            writes,
        }
    }

    /// The request id of every feeder write clients sent so far, in order:
    /// a write sent again under its id shows twice.
    pub fn writes(&self) -> Vec<RequestId> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// One session, until the client closes it.
fn serve(stream: TcpStream, writes: &Mutex<Vec<RequestId>>) -> Option<()> {
    let mut session = Session::accept(stream, SECRET, "newer", REVISION + 1)?;
    let mut plain = [0; FRAME_MAX];
    loop {
        let len = session.receive(&mut plain)?;
        match wellfed_proto::decode::<ClientMessage<'_>>(&plain[..len]) {
            Ok(ClientMessage::Request {
                id,
                op: Op::Feeder(op),
            }) if op.writes() => {
                writes
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(id);
                let mut answer = vec![RESPONSE];
                answer.extend_from_slice(&id);
                answer.push(LATER);
                session.send(&answer)?;
            }
            Ok(ClientMessage::Request { id, .. }) => {
                let refused = DeviceMessage::Response {
                    id,
                    result: Outcome::Refused(Refusal::Unavailable),
                };
                let mut out = [0; FRAME_MAX];
                let len = wellfed_proto::encode(&refused, &mut out).ok()?;
                session.send(&out[..len])?;
            }
            // Its uptime, then the later metric and a value of its own.
            Ok(ClientMessage::Subscribe { .. }) => session.send(&[SAMPLE, 5, LATER, 1, 2])?,
            Err(_) => {}
        }
    }
}
