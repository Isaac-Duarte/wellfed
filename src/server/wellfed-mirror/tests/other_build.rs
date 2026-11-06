//! A feeder whose welcome names another schema revision: an older bank 0,
//! say, after a failed trial. Its build may read this server's other ops as
//! something else, so the task neither subscribes nor reads its status or
//! sets its clock: it shows the feeder as `other_build`, refuses every
//! command but the frozen update path before sending it, and keeps the
//! session for an install.

mod support;

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use wellfed_api::{Connection as Link, Model};
use wellfed_mirror::{AskError, Config, Diagnostics, Discovery, Feeder, Mirror, MirrorEvent};
use wellfed_model::{Answer, Op};
use wellfed_proto::api::{
    Bank0Check, FRAME_MAX, Open, PROLOGUE, PROTOCOL_MAJOR, Refusal, RequestId, Role, SCHEMA,
    UpdateOp, UpdatePhase, UpdateView, WANTED_MAX, Welcome,
};
use wellfed_server_testkit::fixed::{FixedClock, SeededRandom};
use wellfed_server_testkit::{TempStore, WAIT, local};
use wellfed_session::frame::{self, PREFIX};
use wellfed_session::noise::{Handshake, Keypair, Pattern, Role as Side};
use wellfed_store::{NewFeeder, SealedKey};

use support::{HOME, ID, SECRET, next, timing};

const FEEDER: [u8; 32] = [0x10; 32];

/// The frozen request alone, as an older build decodes it.
#[derive(Deserialize)]
enum ClientMessage<'a> {
    Request {
        id: RequestId,
        #[serde(borrow)]
        op: FrozenOp<'a>,
    },
}

#[derive(Deserialize)]
enum FrozenOp<'a> {
    #[serde(borrow)]
    Update(UpdateOp<'a>),
    Alive,
}

#[derive(Serialize)]
enum DeviceMessage {
    Response { id: RequestId, result: Outcome },
}

#[derive(Serialize)]
enum Outcome {
    Done { generation: Option<u32> },
    Refused(Refusal),
    Update(UpdateView),
}

fn read_frame(stream: &mut TcpStream) -> Option<Vec<u8>> {
    let mut prefix = [0; PREFIX];
    stream.read_exact(&mut prefix).ok()?;
    let len = frame::parse_prefix(prefix, FRAME_MAX)?;
    let mut body = vec![0; len];
    stream.read_exact(&mut body).ok()?;
    Some(body)
}

fn write_frame(stream: &mut TcpStream, body: &[u8]) {
    stream
        .write_all(&frame::prefix(body.len(), FRAME_MAX).unwrap())
        .unwrap();
    stream.write_all(body).unwrap();
}

/// Serve one connection as an older build would; every message it reads
/// goes into `seen`, by name, or as `unknown` when it does not decode.
fn serve(mut stream: TcpStream, seen: &Mutex<Vec<&'static str>>) {
    let identity = Keypair::from_secret(FEEDER);
    let Some(open) = read_frame(&mut stream) else {
        return;
    };
    assert_eq!(wellfed_proto::decode::<Open>(&open), Ok(Open::Session));
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
        role: Role::Owner,
        epoch: 1,
        schema: SCHEMA ^ 1,
    };
    let len = wellfed_proto::encode(&welcome, &mut plain).unwrap();
    let written = handshake.write_message(&plain[..len], &mut sealed).unwrap();
    write_frame(&mut stream, &sealed[..written]);
    let mut transport = handshake.into_transport().unwrap();

    while let Some(message) = read_frame(&mut stream) {
        let len = transport.open(&message, &mut plain).unwrap();
        let (id, result) = match wellfed_proto::decode::<ClientMessage<'_>>(&plain[..len]) {
            Ok(ClientMessage::Request { id, op }) => {
                let (name, result) = match op {
                    FrozenOp::Alive => ("alive", Outcome::Done { generation: None }),
                    FrozenOp::Update(UpdateOp::State) => (
                        "state",
                        Outcome::Update(UpdateView {
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
                        }),
                    ),
                    FrozenOp::Update(_) => ("update", Outcome::Refused(Refusal::Busy)),
                };
                seen.lock().unwrap().push(name);
                (id, result)
            }
            Err(_) => {
                seen.lock().unwrap().push("unknown");
                let Some(id) = plain.get(1..17).and_then(|id| id.try_into().ok()) else {
                    continue;
                };
                (id, Outcome::Refused(Refusal::Unavailable))
            }
        };
        let len =
            wellfed_proto::encode(&DeviceMessage::Response { id, result }, &mut plain).unwrap();
        let written = transport.seal(&plain[..len], &mut sealed).unwrap();
        write_frame(&mut stream, &sealed[..written]);
    }
}

#[test]
fn a_feeder_of_another_build_is_sent_only_the_update_path() {
    local(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let served = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                serve(stream.unwrap(), &served);
            }
        });

        let temp = TempStore::new();
        let store = temp.open().await;
        support::home(&store).await;
        let identity = *Keypair::from_secret(FEEDER).public();
        store
            .insert_feeder(NewFeeder {
                id: ID,
                home: HOME,
                model: Model::Plaf103,
                identity,
                name: "kitchen",
                role: wellfed_api::ClientRole::Owner,
                address: Some(&address),
                key: SealedKey {
                    nonce: [0; 12],
                    sealed: Vec::new(),
                },
                created: FixedClock.now(),
            })
            .await
            .unwrap();
        let config = Config {
            discovery: Discovery::Off,
            timing: timing(WAIT),
            diagnostics: Diagnostics::Read {
                stats_every: std::time::Duration::from_millis(50),
            },
        };
        let mirror = Mirror::new(
            store.clone(),
            config,
            Arc::new(FixedClock),
            Arc::new(SeededRandom::fresh()),
            CancellationToken::new(),
        );
        let mut updates = mirror
            .start(Feeder {
                id: ID,
                identity,
                secret: SECRET,
                address: Some(address),
            })
            .await;

        let mut links = Vec::new();
        while links.last() != Some(&Link::OtherBuild) {
            if let MirrorEvent::Link(link) = next(&mut updates).await {
                links.push(link);
            }
        }
        assert_eq!(links, [Link::Connecting, Link::OtherBuild]);
        assert!(mirror.session_open(ID));

        for op in [Op::Status, Op::Feed { portions: 1 }, Op::ListClients] {
            let asked = mirror.ask(ID, [1; 16], op.clone()).await;
            assert_eq!(asked, Err(AskError::OtherBuild), "{op:?}");
        }
        let state = mirror.ask(ID, [2; 16], Op::UpdateState).await;
        assert!(matches!(state, Ok(Answer::Update(view)) if view.bank == 0));

        // An install's turn still runs its frozen steps.
        let mut operation = mirror.operation(ID).await.unwrap();
        let step = operation.step([3; 16], Op::UpdateState).await;
        assert!(matches!(step, Ok(Answer::Update(_))));

        assert_eq!(mirror.link(ID), Some(Link::OtherBuild));
        assert_eq!(
            *seen.lock().unwrap(),
            ["state", "state"],
            "no subscription, status, clock or statistics went to it"
        );
    });
}
