//! A feeder whose welcome names a newer API revision is served as any
//! other: the API only grows, so the bridge reads and writes what both
//! know, and its connect update says the integration is the side to update.
//! A pre-revision build is sent nothing, and named for an update.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use wellfed_connect::Timing;
use wellfed_homeassistant::api::{Skew, Update, UpdateKind};
use wellfed_homeassistant::feeder::{Clocks, Feeder, Target};
use wellfed_homeassistant::key::ClientKey;
use wellfed_proto::api::{FRAME_MAX, Open, PROLOGUE, PROTOCOL_MAJOR, REVISION, Role, Welcome};
use wellfed_session::frame::{self, PREFIX};
use wellfed_session::noise::{Handshake, Keypair, Pattern, Role as Side};

const FEEDER: [u8; 32] = [0x10; 32];
const WAIT: Duration = Duration::from_secs(30);

fn read_frame(stream: &mut TcpStream) -> Option<Vec<u8>> {
    let mut prefix = [0; PREFIX];
    stream.read_exact(&mut prefix).ok()?;
    let len = frame::parse_prefix(prefix, FRAME_MAX)?;
    let mut body = vec![0; len];
    stream.read_exact(&mut body).ok()?;
    Some(body)
}

/// Open one session as a build of a newer revision would, then count
/// every message that arrives after the welcome, answering none.
fn serve(mut stream: TcpStream, schema: u32, sent: &Mutex<usize>) {
    let Some(open) = read_frame(&mut stream) else {
        return;
    };
    assert_eq!(wellfed_proto::decode::<Open>(&open), Ok(Open::Session));
    let identity = Keypair::from_secret(FEEDER);
    let mut handshake = Handshake::new(
        Pattern::Ik,
        Side::Responder,
        PROLOGUE,
        &identity,
        None,
        [9; 32],
    )
    .unwrap();
    let mut plain = [0; FRAME_MAX];
    let mut sealed = [0; FRAME_MAX];
    handshake
        .read_message(&read_frame(&mut stream).unwrap(), &mut plain)
        .unwrap();

    let welcome = Welcome::Open {
        major: PROTOCOL_MAJOR,
        model: wellfed_proto::device::Model::Plaf103,
        build: if schema == REVISION + 1 { "new" } else { "old" },
        role: Role::Member,
        epoch: 1,
        schema,
    };
    let len = wellfed_proto::encode(&welcome, &mut plain).unwrap();
    let written = handshake.write_message(&plain[..len], &mut sealed).unwrap();
    stream
        .write_all(&frame::prefix(written, FRAME_MAX).unwrap())
        .unwrap();
    stream.write_all(&sealed[..written]).unwrap();

    while read_frame(&mut stream).is_some() {
        *sent.lock().unwrap() += 1;
    }
}

/// A session with a feeder whose welcome carries `schema`; its connect
/// update, and how many messages the bridge had sent it a moment later.
fn connect(schema: u32) -> (Update, usize) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let sent = Arc::new(Mutex::new(0));
    {
        let sent = Arc::clone(&sent);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let sent = Arc::clone(&sent);
                std::thread::spawn(move || serve(stream.unwrap(), schema, &sent));
            }
        });
    }

    let clocks = Clocks {
        session: Timing {
            answer: Duration::from_secs(2),
            ..Timing::DEFAULT
        },
        retry: Duration::from_millis(100),
        settle: Clocks::DEFAULT.settle,
    };
    let target = Target {
        identity: *Keypair::from_secret(FEEDER).public(),
        address: Some(address.to_string()),
        zone: jiff::tz::TimeZone::UTC,
    };
    let feeder = Feeder::start(
        ClientKey::generate().unwrap(),
        target,
        clocks,
        Default::default(),
    )
    .unwrap();

    let deadline = Instant::now() + WAIT;
    let connected = loop {
        assert!(Instant::now() < deadline, "no connected update");
        let Some(update) = feeder.next_update(Duration::from_secs(1)) else {
            continue;
        };
        if update.kind == UpdateKind::Connected {
            break update;
        }
    };
    // Long enough for a status read to go out, were one sent.
    std::thread::sleep(Duration::from_millis(500));
    let sent = *sent.lock().unwrap();
    (connected, sent)
}

#[test]
fn a_feeder_of_a_newer_revision_is_served_and_named_ahead() {
    let (connected, sent) = connect(REVISION + 1);

    assert_eq!(connected.build.as_deref(), Some("new"));
    assert_eq!(connected.skew, Some(Skew::FeederAhead));
    assert!(
        sent > 0,
        "the status read goes out, as to a feeder of this revision"
    );
}

/// A pre-revision build sends the schema hash it was built with where the
/// revision now goes (master's last): every byte outside the frozen part
/// moved since, so nothing is sent to it, and the connect update says to
/// update the feeder.
#[test]
fn a_pre_revision_feeder_is_sent_nothing_and_named_legacy() {
    let (connected, sent) = connect(0x2af2_8cd8);

    assert_eq!(connected.build.as_deref(), Some("old"));
    assert_eq!(connected.skew, Some(Skew::Legacy));
    assert_eq!(sent, 0, "no status read or subscription went to it");
}
