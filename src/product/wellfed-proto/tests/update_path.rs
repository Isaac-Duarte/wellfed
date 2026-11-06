//! The frozen update path's bytes: what every build of major 1 must keep
//! decoding and answering alike, for good.
//!
//! Every update passes through bank 0, which keeps the build it was written
//! with: bank 1 checks an offer and gives way, and bank 0 receives the image.
//! So a client of any later build still has to open a session with a bank 0
//! of any earlier build, offer, push chunks and read the answers. A line
//! here that changes strands every deployed bank 0: its feeder could never
//! update again without a UART cable.
//!
//! The lines are only ever appended. A new variant of a frozen enum goes at
//! its end and gets a new line at the end of the file
//! (`WELLFED_BLESS=1 cargo nextest run -p wellfed-proto` appends it); the
//! exhaustive matches in [`every_frozen_variant_has_a_line`] make adding one
//! without its line fail to compile. Only a new major changes the frozen
//! part ([device API](../../../../docs/firmware/device-api.md#evolution)).

use core::fmt::Debug;

use serde::{Deserialize, Serialize};
use wellfed_proto::api::*;
use wellfed_proto::device::Model;
use wellfed_proto::{decode, encode, setup};
use wellfed_session::frame;
use wellfed_session::hex::Lower;
use wellfed_session::noise::{Handshake, Keypair, Pattern, Role as Side, Transport};
use wellfed_session::setup_code::{Button, Code, Exchange, PRESSES, Round, Side as CodeSide};

const PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/update-v1.txt");

const STRANDS: &str = "changing a frozen update-path line strands every deployed bank 0: \
                       its feeder could never update again without a UART cable. Only a new \
                       major changes the frozen part; append new variants instead";

const ID: RequestId = [0x11; 16];

/// A pinned line: its name, its bytes, and whether they decode back to the
/// value they were made from.
struct Line {
    name: &'static str,
    bytes: Vec<u8>,
    decodes: bool,
}

/// `value`'s line. The decoded message may borrow its bytes, so they are
/// kept for the test's lifetime.
fn line<T>(name: &'static str, value: &T) -> Line
where
    T: Serialize + Deserialize<'static> + PartialEq + Debug,
{
    let mut out = vec![0; FRAME_MAX];
    let len = encode(value, &mut out).unwrap();
    out.truncate(len);
    let kept: &'static [u8] = Vec::leak(out.clone());
    Line {
        name,
        bytes: out,
        decodes: decode::<T>(kept).as_ref() == Ok(value),
    }
}

fn request(name: &'static str, op: Op<'static>) -> Line {
    line(name, &ClientMessage::Request { id: ID, op })
}

fn response(name: &'static str, result: Outcome<'static>) -> Line {
    line(name, &DeviceMessage::Response { id: ID, result })
}

fn welcome(name: &'static str, welcome: Welcome<'static>) -> Line {
    line(name, &welcome)
}

fn raw(name: &'static str, bytes: &[u8]) -> Line {
    Line {
        name,
        bytes: bytes.to_vec(),
        decodes: true,
    }
}

/// A view with every field set, which each line varies in one place.
const VIEW: UpdateView = UpdateView {
    phase: UpdatePhase::Receiving,
    bank: 0,
    min_version: 300,
    digest: Some([0x56; 32]),
    version: 301,
    image_len: 1_015_808,
    written: 8_192,
    wanted: [
        Some(8_192),
        Some(8_704),
        Some(9_216),
        Some(9_728),
        None,
        None,
        None,
        None,
    ],
    stopped: None,
    bank0: Bank0Check::Unchecked,
};

const fn view(
    phase: UpdatePhase,
    stopped: Option<UpdateStop>,
    bank0: Bank0Check,
) -> Outcome<'static> {
    Outcome::Update(UpdateView {
        phase,
        stopped,
        bank0,
        ..VIEW
    })
}

const fn refused(refusal: UpdateRefusal) -> Outcome<'static> {
    Outcome::Refused(Refusal::Update(refusal))
}

const fn welcome_open(model: Model, role: Role) -> Welcome<'static> {
    Welcome::Open {
        major: 1,
        model,
        build: "8d67a407",
        role,
        epoch: 0x1234_5678,
        schema: 0x9abc_def0,
    }
}

/// A whole session's opening and one exchange, from fixed keys: the clear
/// `Open`, the IK handshake under the prologue with its hello and welcome,
/// then a sealed request and its sealed answer, each framed. It pins the
/// handshake pattern, the cipher suite and the framing, beside the bytes
/// the messages encode to.
fn transcript() -> Vec<Line> {
    let client = Keypair::from_secret([0x01; 32]);
    let device = Keypair::from_secret([0x02; 32]);
    let mut initiator = Handshake::new(
        Pattern::Ik,
        Side::Initiator,
        PROLOGUE,
        &client,
        Some(*device.public()),
        [0x03; 32],
    )
    .unwrap();
    let mut responder = Handshake::new(
        Pattern::Ik,
        Side::Responder,
        PROLOGUE,
        &device,
        None,
        [0x04; 32],
    )
    .unwrap();
    let framed = |message: &[u8]| {
        let mut out = frame::prefix(message.len(), FRAME_MAX).unwrap().to_vec();
        out.extend_from_slice(message);
        out
    };
    let encoded = |value: &dyn erased::Encode| {
        let mut out = vec![0; FRAME_MAX];
        let len = value.encode_into(&mut out);
        out.truncate(len);
        out
    };

    let mut buffer = [0; FRAME_MAX];
    let mut payload = [0; FRAME_MAX];
    let open = framed(&encoded(&Open::Session));

    let hello = encoded(&Hello { major: 1 });
    let len = initiator.write_message(&hello, &mut buffer).unwrap();
    let first = framed(&buffer[..len]);
    let read = responder
        .read_message(&buffer[..len], &mut payload)
        .unwrap();
    let hello_read = payload[..read] == hello[..];

    let welcome = encoded(&welcome_open(Model::Plaf103, Role::Owner));
    let len = responder.write_message(&welcome, &mut buffer).unwrap();
    let second = framed(&buffer[..len]);
    let read = initiator
        .read_message(&buffer[..len], &mut payload)
        .unwrap();
    let welcome_read = payload[..read] == welcome[..];

    let mut client_side = initiator.into_transport().unwrap();
    let mut device_side = responder.into_transport().unwrap();
    let ask = encoded(&ClientMessage::Request {
        id: ID,
        op: Op::Update(UpdateOp::State),
    });
    let len = client_side.seal(&ask, &mut buffer).unwrap();
    let sealed_request = framed(&buffer[..len]);
    let read = device_side.open(&buffer[..len], &mut payload).unwrap();
    let ask_read = payload[..read] == ask[..];

    let answer = encoded(&DeviceMessage::Response {
        id: ID,
        result: Outcome::Update(VIEW),
    });
    let len = device_side.seal(&answer, &mut buffer).unwrap();
    let sealed_answer = framed(&buffer[..len]);
    let read = client_side.open(&buffer[..len], &mut payload).unwrap();
    let answer_read = payload[..read] == answer[..];

    vec![
        raw("transcript-open", &open),
        Line {
            name: "transcript-ik-hello",
            bytes: first,
            decodes: hello_read,
        },
        Line {
            name: "transcript-ik-welcome",
            bytes: second,
            decodes: welcome_read,
        },
        Line {
            name: "transcript-request",
            bytes: sealed_request,
            decodes: ask_read,
        },
        Line {
            name: "transcript-answer",
            bytes: sealed_answer,
            decodes: answer_read,
        },
    ]
}

/// Encoding behind a trait object, so the transcript encodes messages of
/// several types with one closure.
mod erased {
    pub(crate) trait Encode {
        fn encode_into(&self, out: &mut [u8]) -> usize;
    }

    impl<T: serde::Serialize> Encode for T {
        fn encode_into(&self, out: &mut [u8]) -> usize {
            wellfed_proto::encode(self, out).unwrap()
        }
    }
}

/// Every frozen line, in the order the file holds them. New lines go at
/// the end only.
fn lines() -> Vec<Line> {
    let mut lines = vec![
        // What finds the device and frames its bytes.
        raw("prologue", PROLOGUE),
        line("protocol-major", &PROTOCOL_MAJOR),
        line("frame-max", &u16::try_from(FRAME_MAX).unwrap()),
        raw(
            "frame-prefix-max",
            &frame::prefix(FRAME_MAX, FRAME_MAX).unwrap(),
        ),
        raw("mdns-service", SERVICE.as_bytes()),
        line("port", &PORT),
        line(
            "update-chunk-max",
            &u16::try_from(UPDATE_CHUNK_MAX).unwrap(),
        ),
        line("wanted-max", &u8::try_from(WANTED_MAX).unwrap()),
        // Opening a session.
        line("open-session", &Open::Session),
        line("hello", &Hello { major: 1 }),
        welcome(
            "welcome-open-plaf103-owner",
            welcome_open(Model::Plaf103, Role::Owner),
        ),
        welcome(
            "welcome-open-plwf106-owner",
            welcome_open(Model::Plwf106, Role::Owner),
        ),
        welcome(
            "welcome-open-member",
            welcome_open(Model::Plaf103, Role::Member),
        ),
        welcome(
            "welcome-open-server",
            welcome_open(Model::Plaf103, Role::Server),
        ),
        welcome(
            "welcome-open-update-signer",
            welcome_open(Model::Plaf103, Role::UpdateSigner),
        ),
        welcome(
            "welcome-refused-unknown",
            Welcome::Refused(OpenRefusal::Unknown),
        ),
        welcome(
            "welcome-refused-version",
            Welcome::Refused(OpenRefusal::Version { supported: 1 }),
        ),
        welcome("welcome-refused-full", Welcome::Refused(OpenRefusal::Full)),
        // The ops.
        request(
            "op-update-offer",
            Op::Update(UpdateOp::Offer {
                manifest: &[0x51; 4],
                downgrade: None,
            }),
        ),
        request(
            "op-update-offer-downgrade",
            Op::Update(UpdateOp::Offer {
                manifest: &[0x51; 4],
                downgrade: Some(&[0x52; 3]),
            }),
        ),
        request(
            "op-update-chunk",
            Op::Update(UpdateOp::Chunk {
                offset: 8_192,
                data: &[0x53; 5],
            }),
        ),
        request("op-update-state", Op::Update(UpdateOp::State)),
        request("op-update-cancel", Op::Update(UpdateOp::Cancel)),
        request(
            "op-update-enroll-key",
            Op::Update(UpdateOp::EnrollKey {
                key: [0x54; 32],
                label: "laptop",
            }),
        ),
        request(
            "op-update-key-statement",
            Op::Update(UpdateOp::KeyStatement {
                statement: &[0x55; 6],
            }),
        ),
        request("op-alive", Op::Alive),
        // The answers.
        response("outcome-done", Outcome::Done { generation: None }),
        response(
            "outcome-done-generation",
            Outcome::Done {
                generation: Some(10),
            },
        ),
        response("outcome-update", Outcome::Update(VIEW)),
        response(
            "outcome-update-idle",
            view(UpdatePhase::Idle, None, Bank0Check::Unchecked),
        ),
        response(
            "outcome-update-checking",
            view(UpdatePhase::Checking, None, Bank0Check::Unchecked),
        ),
        response(
            "outcome-update-giving-way",
            view(UpdatePhase::GivingWay, None, Bank0Check::Match),
        ),
        response(
            "outcome-update-activated",
            view(UpdatePhase::Activated, None, Bank0Check::Unchecked),
        ),
    ];

    let stops = [
        ("outcome-update-stopped-timeout", UpdateStop::Timeout),
        ("outcome-update-stopped-cancelled", UpdateStop::Cancelled),
        ("outcome-update-stopped-order", UpdateStop::Order),
        ("outcome-update-stopped-marker", UpdateStop::Marker),
        ("outcome-update-stopped-write", UpdateStop::Write),
        ("outcome-update-stopped-hash", UpdateStop::Hash),
        ("outcome-update-stopped-activate", UpdateStop::Activate),
        ("outcome-update-stopped-store", UpdateStop::Store),
    ];
    for (name, stop) in stops {
        lines.push(response(
            name,
            view(UpdatePhase::Idle, Some(stop), Bank0Check::Unchecked),
        ));
    }

    let blocked = [
        ("outcome-update-bank0-marker", Bank0Blocked::Marker),
        ("outcome-update-bank0-unrecorded", Bank0Blocked::Unrecorded),
        ("outcome-update-bank0-key", Bank0Blocked::Key),
        ("outcome-update-bank0-extent", Bank0Blocked::Extent),
        ("outcome-update-bank0-changed", Bank0Blocked::Changed),
        ("outcome-update-bank0-read", Bank0Blocked::Read),
    ];
    for (name, reason) in blocked {
        lines.push(response(
            name,
            view(UpdatePhase::Idle, None, Bank0Check::Blocked { reason }),
        ));
    }

    let refusals = [
        ("refused-forbidden", Refusal::Forbidden),
        ("refused-busy", Refusal::Busy),
        ("refused-unavailable", Refusal::Unavailable),
        ("refused-invalid", Refusal::Invalid),
        ("refused-not-writable", Refusal::NotWritable),
        ("refused-full", Refusal::Full),
    ];
    for (name, refusal) in refusals {
        lines.push(response(name, Outcome::Refused(refusal)));
    }

    let update_refusals = [
        ("refused-update-manifest", UpdateRefusal::Manifest),
        ("refused-update-key", UpdateRefusal::Key),
        ("refused-update-revoked", UpdateRefusal::Revoked),
        ("refused-update-signature", UpdateRefusal::Signature),
        ("refused-update-target", UpdateRefusal::Target),
        ("refused-update-same", UpdateRefusal::Same),
        ("refused-update-downgrade", UpdateRefusal::Downgrade),
        ("refused-update-statement", UpdateRefusal::Statement),
        ("refused-update-pending", UpdateRefusal::Pending),
        ("refused-update-failed", UpdateRefusal::Failed),
        (
            "refused-update-fallback-marker",
            UpdateRefusal::Fallback {
                reason: Bank0Blocked::Marker,
            },
        ),
        (
            "refused-update-fallback-unrecorded",
            UpdateRefusal::Fallback {
                reason: Bank0Blocked::Unrecorded,
            },
        ),
        (
            "refused-update-fallback-key",
            UpdateRefusal::Fallback {
                reason: Bank0Blocked::Key,
            },
        ),
        (
            "refused-update-fallback-extent",
            UpdateRefusal::Fallback {
                reason: Bank0Blocked::Extent,
            },
        ),
        (
            "refused-update-fallback-changed",
            UpdateRefusal::Fallback {
                reason: Bank0Blocked::Changed,
            },
        ),
        (
            "refused-update-fallback-read",
            UpdateRefusal::Fallback {
                reason: Bank0Blocked::Read,
            },
        ),
        ("refused-update-attempt", UpdateRefusal::Attempt),
        ("refused-update-installing", UpdateRefusal::Installing),
        (
            "refused-update-not-installing",
            UpdateRefusal::NotInstalling,
        ),
        ("refused-update-store", UpdateRefusal::Store),
        ("refused-update-unconfirmed", UpdateRefusal::Unconfirmed),
        ("refused-update-battery", UpdateRefusal::Battery),
    ];
    for (name, refusal) in update_refusals {
        lines.push(response(name, refused(refusal)));
    }

    lines.extend(transcript());
    lines.extend(invites());
    let (recovery, session) = recovery();
    lines.extend(recovery);
    lines.extend(code_proof(session));
    lines.push(response(
        "refused-update-held-as-session-key",
        refused(UpdateRefusal::HeldAsSessionKey),
    ));
    lines
}

/// Invites: a new client still joins a feeder of an older build, then
/// updates it. Appended after the update path's first lines.
fn invites() -> Vec<Line> {
    let invite = Invite {
        id: 7,
        secret: [0x61; 32],
        seconds: 600,
    };
    let mut lines = vec![
        line("open-invite", &Open::Invite { id: 7 }),
        line("open-recovery", &Open::Recovery),
        request("op-create-invite", Op::CreateInvite { role: Role::Member }),
        response("outcome-invite", Outcome::Invite(invite)),
        response("refused-not-found", Outcome::Refused(Refusal::NotFound)),
        line("join", &Join { label: "phone" }),
    ];
    let roles = [
        ("joined-owner", Role::Owner),
        ("joined-member", Role::Member),
        ("joined-server", Role::Server),
        ("joined-update-signer", Role::UpdateSigner),
    ];
    for (name, role) in roles {
        let joined = Joined::Enrolled {
            role,
            model: Model::Plaf103,
        };
        lines.push(line(name, &joined));
    }
    lines.push(line(
        "joined-plwf106",
        &Joined::Enrolled {
            role: Role::Owner,
            model: Model::Plwf106,
        },
    ));
    let refusals = [
        ("joined-refused-full", Refusal::Full),
        ("joined-refused-not-writable", Refusal::NotWritable),
    ];
    for (name, refusal) in refusals {
        lines.push(line(name, &Joined::Refused(refusal)));
    }
    lines.extend(join_transcript(&invite));
    lines
}

/// A whole redemption from fixed keys: the clear `Open::Invite`, XXpsk3
/// under the prologue with the invite's secret, the client's join in its
/// third message and the device's sealed answer.
fn join_transcript(invite: &Invite) -> Vec<Line> {
    let client = Keypair::from_secret([0x05; 32]);
    let device = Keypair::from_secret([0x02; 32]);
    let mut initiator = Handshake::new(
        Pattern::XxPsk3,
        Side::Initiator,
        PROLOGUE,
        &client,
        None,
        [0x06; 32],
    )
    .unwrap();
    let mut responder = Handshake::new(
        Pattern::XxPsk3,
        Side::Responder,
        PROLOGUE,
        &device,
        None,
        [0x07; 32],
    )
    .unwrap();
    initiator.set_psk(invite.secret);
    responder.set_psk(invite.secret);
    let mut buffer = [0; FRAME_MAX];
    let mut payload = [0; FRAME_MAX];
    let open = framed(&encoded(&Open::Invite { id: invite.id }));

    let len = initiator.write_message(&[], &mut buffer).unwrap();
    let first = framed(&buffer[..len]);
    responder
        .read_message(&buffer[..len], &mut payload)
        .unwrap();
    let len = responder.write_message(&[], &mut buffer).unwrap();
    let second = framed(&buffer[..len]);
    initiator
        .read_message(&buffer[..len], &mut payload)
        .unwrap();
    let join = encoded(&Join { label: "phone" });
    let len = initiator.write_message(&join, &mut buffer).unwrap();
    let third = framed(&buffer[..len]);
    let read = responder
        .read_message(&buffer[..len], &mut payload)
        .unwrap();
    let join_read = payload[..read] == join[..];

    let mut client_side = initiator.into_transport().unwrap();
    let mut device_side = responder.into_transport().unwrap();
    let joined = encoded(&Joined::Enrolled {
        role: Role::Member,
        model: Model::Plaf103,
    });
    let len = device_side.seal(&joined, &mut buffer).unwrap();
    let answer = framed(&buffer[..len]);
    let read = client_side.open(&buffer[..len], &mut payload).unwrap();
    let joined_read = payload[..read] == joined[..];

    vec![
        raw("join-transcript-open", &open),
        raw("join-transcript-xxpsk3-1", &first),
        raw("join-transcript-xxpsk3-2", &second),
        Line {
            name: "join-transcript-xxpsk3-join",
            bytes: third,
            decodes: join_read,
        },
        Line {
            name: "join-transcript-joined",
            bytes: answer,
            decodes: joined_read,
        },
    ]
}

/// The recovery window's setup exchange, which BLE setup runs too: an
/// owner who lost every key recovers through it on a feeder of any build.
fn recovery() -> (Vec<Line>, Recovered) {
    let mut lines = vec![
        raw("setup-prologue", setup::PROLOGUE),
        line("setup-request", &setup::Request { label: "phone" }),
        line(
            "setup-answer-round",
            &setup::Answer::Round {
                open: Some([0x71; 32]),
                commit: Some([0x72; 32]),
            },
        ),
        line(
            "setup-answer-last",
            &setup::Answer::Last {
                open: [0x73; 32],
                wifi: None,
            },
        ),
        line(
            "setup-answer-last-wifi",
            &setup::Answer::Last {
                open: [0x73; 32],
                wifi: Some(setup::Wifi {
                    ssid: b"lab",
                    psk: "passphrase",
                }),
            },
        ),
        line(
            "setup-reply-enter-code",
            &setup::Reply::EnterCode { seconds: 60 },
        ),
        line(
            "setup-reply-round",
            &setup::Reply::Round {
                open: None,
                commit: Some([0x74; 32]),
            },
        ),
    ];
    let done = [
        (
            "setup-reply-done-new-saved",
            setup::Enrolled::New,
            setup::WifiSaved::Saved,
        ),
        (
            "setup-reply-done-already-unchanged",
            setup::Enrolled::Already,
            setup::WifiSaved::Unchanged,
        ),
    ];
    for (name, enrolled, wifi) in done {
        lines.push(line(name, &setup::Reply::Done { enrolled, wifi }));
    }
    let refusals = [
        ("setup-refused-busy", setup::Refusal::Busy),
        ("setup-refused-unconfirmed", setup::Refusal::Unconfirmed),
        ("setup-refused-wrong-code", setup::Refusal::WrongCode),
        ("setup-refused-full", setup::Refusal::Full),
        ("setup-refused-not-writable", setup::Refusal::NotWritable),
        ("setup-refused-invalid", setup::Refusal::Invalid),
    ];
    for (name, refusal) in refusals {
        lines.push(line(name, &setup::Reply::Refused(refusal)));
    }
    let (transcript, session) = recovery_transcript();
    lines.extend(transcript);
    (lines, session)
}

/// The recovery session's two ends after its opening, for the code proof.
struct Recovered {
    client: Transport,
    device: Transport,
}

/// The recovery window's opening from fixed keys: the clear
/// `Open::Recovery`, then XX under the setup prologue, its third message
/// carrying the client's request, and the device's first sealed reply.
fn recovery_transcript() -> (Vec<Line>, Recovered) {
    let client = Keypair::from_secret([0x08; 32]);
    let device = Keypair::from_secret([0x02; 32]);
    let mut initiator = Handshake::new(
        Pattern::Xx,
        Side::Initiator,
        setup::PROLOGUE,
        &client,
        None,
        [0x09; 32],
    )
    .unwrap();
    let mut responder = Handshake::new(
        Pattern::Xx,
        Side::Responder,
        setup::PROLOGUE,
        &device,
        None,
        [0x0a; 32],
    )
    .unwrap();
    let mut buffer = [0; FRAME_MAX];
    let mut payload = [0; FRAME_MAX];
    let open = framed(&encoded(&Open::Recovery));

    let len = initiator.write_message(&[], &mut buffer).unwrap();
    let first = framed(&buffer[..len]);
    responder
        .read_message(&buffer[..len], &mut payload)
        .unwrap();
    let len = responder.write_message(&[], &mut buffer).unwrap();
    let second = framed(&buffer[..len]);
    initiator
        .read_message(&buffer[..len], &mut payload)
        .unwrap();
    let request = encoded(&setup::Request { label: "phone" });
    let len = initiator.write_message(&request, &mut buffer).unwrap();
    let third = framed(&buffer[..len]);
    let read = responder
        .read_message(&buffer[..len], &mut payload)
        .unwrap();
    let request_read = payload[..read] == request[..];

    let mut client_side = initiator.into_transport().unwrap();
    let mut device_side = responder.into_transport().unwrap();
    let reply = encoded(&setup::Reply::EnterCode { seconds: 60 });
    let len = device_side.seal(&reply, &mut buffer).unwrap();
    let sealed = framed(&buffer[..len]);
    let read = client_side.open(&buffer[..len], &mut payload).unwrap();
    let reply_read = payload[..read] == reply[..];

    let lines = vec![
        raw("recovery-transcript-open", &open),
        raw("recovery-transcript-xx-1", &first),
        raw("recovery-transcript-xx-2", &second),
        Line {
            name: "recovery-transcript-xx-request",
            bytes: third,
            decodes: request_read,
        },
        Line {
            name: "recovery-transcript-enter-code",
            bytes: sealed,
            decodes: reply_read,
        },
    ];
    let session = Recovered {
        client: client_side,
        device: device_side,
    };
    (lines, session)
}

/// The code proof that follows, from a fixed code and fixed seeds, through
/// every round to `Reply::Done`: each message opens the sender's last
/// commitment and commits to its next press, bound to the handshake hash
/// under `setup_code`'s tag. Changing the commitment's construction, the
/// tag or the turn order changes these bytes, and a client of a later
/// build could no longer recover a feeder of this one.
fn code_proof(mut session: Recovered) -> Vec<Line> {
    let code = Code::new([
        Button::Reset,
        Button::Mic,
        Button::Dispense,
        Button::Lock,
        Button::Dispense,
        Button::Mic,
    ]);
    let mut feeder = Exchange::new(
        CodeSide::Feeder,
        *session.device.handshake_hash(),
        code,
        [0x0b; 32],
    );
    let mut client = Exchange::new(
        CodeSide::Client,
        *session.client.handshake_hash(),
        code,
        [0x0c; 32],
    );

    let mut lines = Vec::new();
    for round in 0..=PRESSES {
        let Round { open, commit } = feeder.write().unwrap();
        let reply = encoded(&setup::Reply::Round { open, commit });
        let (bytes, read) = carry(&mut session.device, &mut session.client, &reply);
        let decodes = read == reply && client.read(Round { open, commit }).is_ok();
        lines.push(Line {
            name: Box::leak(format!("recovery-proof-feeder-{round}").into_boxed_str()),
            bytes,
            decodes,
        });

        let Round { open, commit } = client.write().unwrap();
        let answer = match (client.peer_proven(), open) {
            (true, Some(open)) => encoded(&setup::Answer::Last {
                open,
                wifi: Some(setup::Wifi {
                    ssid: b"lab",
                    psk: "passphrase",
                }),
            }),
            _ => encoded(&setup::Answer::Round { open, commit }),
        };
        let (bytes, read) = carry(&mut session.client, &mut session.device, &answer);
        let decodes = read == answer && feeder.read(Round { open, commit }).is_ok();
        lines.push(Line {
            name: Box::leak(format!("recovery-proof-client-{round}").into_boxed_str()),
            bytes,
            decodes,
        });
    }
    assert!(feeder.peer_proven(), "the client proved the code");

    let done = encoded(&setup::Reply::Done {
        enrolled: setup::Enrolled::New,
        wifi: setup::WifiSaved::Saved,
    });
    let (bytes, read) = carry(&mut session.device, &mut session.client, &done);
    lines.push(Line {
        name: "recovery-proof-done",
        bytes,
        decodes: read == done,
    });
    lines
}

/// `plain` sealed by `from` as framed bytes, and what `to` opens it to.
fn carry(from: &mut Transport, to: &mut Transport, plain: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut sealed = [0; FRAME_MAX];
    let len = from.seal(plain, &mut sealed).unwrap();
    let mut opened = [0; FRAME_MAX];
    let read = to.open(&sealed[..len], &mut opened).unwrap();
    (framed(&sealed[..len]), opened[..read].to_vec())
}

fn framed(message: &[u8]) -> Vec<u8> {
    let mut out = frame::prefix(message.len(), FRAME_MAX).unwrap().to_vec();
    out.extend_from_slice(message);
    out
}

fn encoded(value: &dyn erased::Encode) -> Vec<u8> {
    let mut out = vec![0; FRAME_MAX];
    let len = value.encode_into(&mut out);
    out.truncate(len);
    out
}

fn render(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|line| format!("{} {}\n", line.name, Lower(&line.bytes)))
        .collect()
}

/// The pin: every line already in the file keeps its bytes. Blessing only
/// appends lines for new variants, and refuses to touch a pinned one.
#[test]
fn the_update_path_keeps_its_bytes() {
    let lines = lines();
    let current = render(&lines);
    if std::env::var_os("WELLFED_BLESS").is_some() {
        let fixed = std::fs::read_to_string(PATH).unwrap_or_default();
        assert!(current.starts_with(&fixed), "{STRANDS}");
        std::fs::write(PATH, &current).unwrap();
    }

    let fixed = std::fs::read_to_string(PATH).unwrap();
    let mut pinned = 0;
    for (fixed, now) in fixed.lines().zip(current.lines()) {
        assert_eq!(fixed, now, "{STRANDS}");
        pinned += 1;
    }
    assert_eq!(
        pinned,
        fixed.lines().count(),
        "a pinned line is gone: {STRANDS}"
    );
    assert_eq!(fixed, current, "a frozen line was added: bless it");

    for line in &lines {
        assert!(line.decodes, "{} no longer decodes: {STRANDS}", line.name);
    }
}

/// Every name is used once, so a line can be told apart in a failure.
#[test]
fn every_line_has_its_own_name() {
    let lines = lines();
    let mut names: Vec<_> = lines.iter().map(|line| line.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), lines.len());
}

/// Each frozen enum's variants, matched with no wildcard, so a new variant
/// fails to compile here until it has its line (at the end of the file).
#[test]
fn every_frozen_variant_has_a_line() {
    fn op(op: UpdateOp<'_>) -> &'static str {
        match op {
            UpdateOp::Offer { .. } => "op-update-offer",
            UpdateOp::Chunk { .. } => "op-update-chunk",
            UpdateOp::State => "op-update-state",
            UpdateOp::Cancel => "op-update-cancel",
            UpdateOp::EnrollKey { .. } => "op-update-enroll-key",
            UpdateOp::KeyStatement { .. } => "op-update-key-statement",
        }
    }
    fn model(model: Model) -> &'static str {
        match model {
            Model::Plaf103 => "welcome-open-plaf103-owner",
            Model::Plwf106 => "welcome-open-plwf106-owner",
        }
    }
    fn role(role: Role) -> &'static str {
        match role {
            Role::Owner => "welcome-open-plaf103-owner",
            Role::Member => "welcome-open-member",
            Role::Server => "welcome-open-server",
            Role::UpdateSigner => "welcome-open-update-signer",
        }
    }
    fn open_refusal(refusal: OpenRefusal) -> &'static str {
        match refusal {
            OpenRefusal::Unknown => "welcome-refused-unknown",
            OpenRefusal::Version { .. } => "welcome-refused-version",
            OpenRefusal::Full => "welcome-refused-full",
        }
    }
    fn phase(phase: UpdatePhase) -> &'static str {
        match phase {
            UpdatePhase::Idle => "outcome-update-idle",
            UpdatePhase::Checking => "outcome-update-checking",
            UpdatePhase::GivingWay => "outcome-update-giving-way",
            UpdatePhase::Receiving => "outcome-update",
            UpdatePhase::Activated => "outcome-update-activated",
        }
    }
    fn stop(stop: UpdateStop) -> &'static str {
        match stop {
            UpdateStop::Timeout => "outcome-update-stopped-timeout",
            UpdateStop::Cancelled => "outcome-update-stopped-cancelled",
            UpdateStop::Order => "outcome-update-stopped-order",
            UpdateStop::Marker => "outcome-update-stopped-marker",
            UpdateStop::Write => "outcome-update-stopped-write",
            UpdateStop::Hash => "outcome-update-stopped-hash",
            UpdateStop::Activate => "outcome-update-stopped-activate",
            UpdateStop::Store => "outcome-update-stopped-store",
        }
    }
    fn blocked(reason: Bank0Blocked) -> [&'static str; 2] {
        match reason {
            Bank0Blocked::Marker => [
                "outcome-update-bank0-marker",
                "refused-update-fallback-marker",
            ],
            Bank0Blocked::Unrecorded => [
                "outcome-update-bank0-unrecorded",
                "refused-update-fallback-unrecorded",
            ],
            Bank0Blocked::Key => ["outcome-update-bank0-key", "refused-update-fallback-key"],
            Bank0Blocked::Extent => [
                "outcome-update-bank0-extent",
                "refused-update-fallback-extent",
            ],
            Bank0Blocked::Changed => [
                "outcome-update-bank0-changed",
                "refused-update-fallback-changed",
            ],
            Bank0Blocked::Read => ["outcome-update-bank0-read", "refused-update-fallback-read"],
        }
    }
    fn check(check: Bank0Check) -> &'static str {
        match check {
            Bank0Check::Unchecked => "outcome-update",
            Bank0Check::Match => "outcome-update-giving-way",
            Bank0Check::Blocked { reason } => blocked(reason)[0],
        }
    }
    fn update_refusal(refusal: UpdateRefusal) -> &'static str {
        match refusal {
            UpdateRefusal::Manifest => "refused-update-manifest",
            UpdateRefusal::Key => "refused-update-key",
            UpdateRefusal::Revoked => "refused-update-revoked",
            UpdateRefusal::Signature => "refused-update-signature",
            UpdateRefusal::Target => "refused-update-target",
            UpdateRefusal::Same => "refused-update-same",
            UpdateRefusal::Downgrade => "refused-update-downgrade",
            UpdateRefusal::Statement => "refused-update-statement",
            UpdateRefusal::Pending => "refused-update-pending",
            UpdateRefusal::Failed => "refused-update-failed",
            UpdateRefusal::Fallback { reason } => blocked(reason)[1],
            UpdateRefusal::Attempt => "refused-update-attempt",
            UpdateRefusal::Installing => "refused-update-installing",
            UpdateRefusal::NotInstalling => "refused-update-not-installing",
            UpdateRefusal::Store => "refused-update-store",
            UpdateRefusal::Unconfirmed => "refused-update-unconfirmed",
            UpdateRefusal::Battery => "refused-update-battery",
            UpdateRefusal::HeldAsSessionKey => "refused-update-held-as-session-key",
        }
    }
    /// The frozen refusals; the rest are not frozen and have no line.
    fn refusal(refusal: Refusal) -> Option<&'static str> {
        Some(match refusal {
            Refusal::Forbidden => "refused-forbidden",
            Refusal::Busy => "refused-busy",
            Refusal::Unavailable => "refused-unavailable",
            Refusal::Invalid => "refused-invalid",
            Refusal::NotWritable => "refused-not-writable",
            Refusal::Full => "refused-full",
            Refusal::Update(refusal) => update_refusal(refusal),
            Refusal::NotFound => "refused-not-found",
            Refusal::Stale { .. }
            | Refusal::Muted
            | Refusal::Time
            | Refusal::Fault
            | Refusal::Unverified
            | Refusal::Portions { .. }
            | Refusal::NoClock => return None,
        })
    }

    fn open(open: Open) -> &'static str {
        match open {
            Open::Session => "open-session",
            Open::Invite { .. } => "open-invite",
            Open::Recovery => "open-recovery",
        }
    }
    fn joined(joined: Joined) -> &'static str {
        match joined {
            Joined::Enrolled { role, .. } => match role {
                Role::Owner => "joined-owner",
                Role::Member => "joined-member",
                Role::Server => "joined-server",
                Role::UpdateSigner => "joined-update-signer",
            },
            Joined::Refused(Refusal::NotWritable) => "joined-refused-not-writable",
            Joined::Refused(_) => "joined-refused-full",
        }
    }
    fn answer(answer: setup::Answer<'_>) -> &'static str {
        match answer {
            setup::Answer::Round { .. } => "setup-answer-round",
            setup::Answer::Last { .. } => "setup-answer-last",
        }
    }
    fn reply(reply: setup::Reply) -> &'static str {
        match reply {
            setup::Reply::EnterCode { .. } => "setup-reply-enter-code",
            setup::Reply::Round { .. } => "setup-reply-round",
            setup::Reply::Done { enrolled, wifi } => match (enrolled, wifi) {
                (setup::Enrolled::New, _) => "setup-reply-done-new-saved",
                (
                    setup::Enrolled::Already,
                    setup::WifiSaved::Saved | setup::WifiSaved::Unchanged,
                ) => "setup-reply-done-already-unchanged",
            },
            setup::Reply::Refused(refusal) => match refusal {
                setup::Refusal::Busy => "setup-refused-busy",
                setup::Refusal::Unconfirmed => "setup-refused-unconfirmed",
                setup::Refusal::WrongCode => "setup-refused-wrong-code",
                setup::Refusal::Full => "setup-refused-full",
                setup::Refusal::NotWritable => "setup-refused-not-writable",
                setup::Refusal::Invalid => "setup-refused-invalid",
            },
        }
    }

    let lines = lines();
    let names: Vec<_> = lines.iter().map(|line| line.name).collect();
    let has = |name: &str| assert!(names.contains(&name), "{name} has no frozen line");
    has(open(Open::Recovery));
    has(joined(Joined::Refused(Refusal::Full)));
    has(answer(setup::Answer::Last {
        open: [0; 32],
        wifi: None,
    }));
    has(reply(setup::Reply::Refused(setup::Refusal::Invalid)));
    has(reply(setup::Reply::Done {
        enrolled: setup::Enrolled::Already,
        wifi: setup::WifiSaved::Unchanged,
    }));

    // Every line's own value names the line it is on, so the matches
    // above cover what the file holds.
    has(op(UpdateOp::State));
    has(model(Model::Plwf106));
    has(role(Role::UpdateSigner));
    has(open_refusal(OpenRefusal::Full));
    has(phase(UpdatePhase::GivingWay));
    has(stop(UpdateStop::Store));
    has(check(Bank0Check::Match));
    has(refusal(Refusal::Full).unwrap());
    for line in &lines {
        let Ok(DeviceMessage::Response { result, .. }) = decode::<DeviceMessage<'_>>(&line.bytes)
        else {
            continue;
        };
        if !line.name.starts_with("outcome-") && !line.name.starts_with("refused-") {
            continue;
        }
        match result {
            Outcome::Update(view) => {
                has(phase(view.phase));
                has(check(view.bank0));
                if let Some(stopped) = view.stopped {
                    has(stop(stopped));
                }
            }
            Outcome::Refused(refused) => has(refusal(refused).expect("a frozen refusal")),
            Outcome::Done { .. } | Outcome::Invite(_) => {}
            other => panic!(
                "{} holds an answer that is not frozen: {other:?}",
                line.name
            ),
        }
    }
    for model_ in Model::ALL {
        has(model(model_));
    }
}

/// The frozen ops and answers are the first variants, and only they are
/// frozen: the boundary the rest of the protocol changes behind.
#[test]
fn the_frozen_variants_come_first() {
    let byte = |value: &dyn erased::Encode| {
        let mut out = [0; FRAME_MAX];
        value.encode_into(&mut out);
        out[0]
    };
    assert_eq!(
        byte(&ClientMessage::Request {
            id: ID,
            op: Op::Alive
        }),
        0
    );
    assert_eq!(
        byte(&DeviceMessage::Response {
            id: ID,
            result: Outcome::Done { generation: None }
        }),
        0
    );
    assert_eq!(byte(&Op::Update(UpdateOp::State)), 0);
    assert_eq!(byte(&Op::Alive), 1);
    assert_eq!(byte(&Outcome::Done { generation: None }), 0);
    assert_eq!(byte(&Outcome::Refused(Refusal::Busy)), 1);
    assert_eq!(byte(&Outcome::Update(VIEW)), 2);
    assert!(Op::Alive.is_frozen() && !Op::Status.is_frozen());
    assert!(!Outcome::Refused(Refusal::Stale { current: 1 }).is_frozen());
    assert!(Outcome::Refused(Refusal::Update(UpdateRefusal::Battery)).is_frozen());
    // The last frozen refusal sits just before the first free one.
    assert_eq!(byte(&Refusal::Update(UpdateRefusal::Manifest)), 6);
    assert_eq!(byte(&Refusal::NotFound), 7);
    assert_eq!(byte(&Refusal::Stale { current: 1 }), 8);
    assert_eq!(byte(&Op::CreateInvite { role: Role::Owner }), 2);
    assert_eq!(
        byte(&Outcome::Invite(Invite {
            id: 1,
            secret: [0; 32],
            seconds: 1
        })),
        3
    );
}

/// A frozen refusal is the same bytes as the refusal it becomes, so the
/// device's typed answer and the wire agree.
#[test]
fn every_frozen_refusal_encodes_as_its_refusal() {
    let frozen = [
        FrozenRefusal::Forbidden,
        FrozenRefusal::Busy,
        FrozenRefusal::Unavailable,
        FrozenRefusal::Invalid,
        FrozenRefusal::NotWritable,
        FrozenRefusal::Full,
        FrozenRefusal::Update(UpdateRefusal::Fallback {
            reason: Bank0Blocked::Read,
        }),
        FrozenRefusal::NotFound,
    ];
    for refusal in frozen {
        let mut as_frozen = [0; 8];
        let mut as_refusal = [0; 8];
        let a = encode(&refusal, &mut as_frozen).unwrap();
        let b = encode(&Refusal::from(refusal), &mut as_refusal).unwrap();
        assert_eq!(as_frozen[..a], as_refusal[..b], "{refusal:?}");
        assert_eq!(Refusal::from(refusal).frozen(), Some(refusal));
    }
}

/// Signed manifests name their model by this code, and every deployed bank
/// 0 refuses an offer whose code is not its own (`Target`). The signed
/// formats' fixtures in wellfed-core sign with the literal codes below, so a
/// code changed here would strand those feeders while their fixtures still
/// pass: changing one is never allowed.
#[test]
fn the_model_codes_signed_manifests_carry_never_change() {
    let codes: Vec<(Model, u8)> = Model::ALL.iter().map(|m| (*m, m.code())).collect();
    assert_eq!(
        codes,
        [(Model::Plaf103, 1), (Model::Plwf106, 2)],
        "a model's code is in every signed manifest; changing it strands every deployed bank 0"
    );
}
