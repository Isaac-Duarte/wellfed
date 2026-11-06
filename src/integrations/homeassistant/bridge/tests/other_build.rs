//! A feeder whose welcome names another schema revision (an older bank 0
//! after a failed trial, say) takes only the frozen update path, which the
//! app sends. The bridge sends it nothing, says so once, and stops for good
//! rather than reconnecting to the same build.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use wellfed_connect::Timing;
use wellfed_homeassistant::api::{NotSent, UpdateKind, Written};
use wellfed_homeassistant::feeder::{Clocks, Feeder, Target};
use wellfed_homeassistant::key::ClientKey;
use wellfed_proto::api::{FRAME_MAX, Open, PROLOGUE, PROTOCOL_MAJOR, Role, SCHEMA, Welcome};
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

/// Open one session as a build of another schema revision would, then
/// count every message that arrives after the welcome.
fn serve(mut stream: TcpStream, sent: &Mutex<usize>) {
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
        build: "old",
        role: Role::Member,
        epoch: 1,
        schema: SCHEMA ^ 1,
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

#[test]
fn a_feeder_of_another_build_stops_the_session_for_good() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let sent = Arc::new(Mutex::new(0));
    let connections = Arc::new(Mutex::new(0));
    {
        let (sent, connections) = (Arc::clone(&sent), Arc::clone(&connections));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                *connections.lock().unwrap() += 1;
                let sent = Arc::clone(&sent);
                std::thread::spawn(move || serve(stream.unwrap(), &sent));
            }
        });
    }

    let clocks = Clocks {
        session: Timing {
            answer: Duration::from_secs(2),
            ..Timing::DEFAULT
        },
        retry: Duration::from_millis(100),
    };
    let target = Target {
        identity: *Keypair::from_secret(FEEDER).public(),
        address: Some(address.to_string()),
    };
    let feeder = Feeder::start(ClientKey::generate().unwrap(), target, clocks).unwrap();

    let deadline = Instant::now() + WAIT;
    let stopped = loop {
        assert!(Instant::now() < deadline, "no other_build update");
        let Some(update) = feeder.next_update(Duration::from_secs(1)) else {
            continue;
        };
        assert_ne!(update.kind, UpdateKind::Connected);
        assert_ne!(update.kind, UpdateKind::Disconnected);
        if update.kind == UpdateKind::OtherBuild {
            break update;
        }
    };
    assert_eq!(stopped.build.as_deref(), Some("old"));
    assert_eq!(feeder.feed(1), Written::not_sent(NotSent::Stopped));

    // Well past the retry clock: no second connection, and nothing sent.
    std::thread::sleep(Duration::from_millis(500));
    assert!(feeder.next_update(Duration::from_millis(100)).is_none());
    assert_eq!(*connections.lock().unwrap(), 1);
    assert_eq!(*sent.lock().unwrap(), 0);
}
