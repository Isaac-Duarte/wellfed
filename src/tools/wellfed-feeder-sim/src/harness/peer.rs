//! A device's side of a session, written by hand for the builds a test
//! plays that this one is not (an older build, a newer one): the
//! handshake, the welcome, and sealed frames each way.

use std::io::{Read, Write};
use std::net::TcpStream;

use wellfed_proto::api::{FRAME_MAX, Open, PROLOGUE, PROTOCOL_MAJOR, Role, Welcome};
use wellfed_session::frame::{self, PREFIX};
use wellfed_session::noise::{Handshake, Keypair, Pattern, Role as Side, Transport};

/// An open session, its welcome sent.
pub(super) struct Session {
    stream: TcpStream,
    transport: Transport,
}

impl Session {
    /// Take a client's session on `stream` as the device whose identity
    /// is `secret`, welcoming it as an owner with `schema` where the
    /// revision goes. `None` for a client that opened anything else, or
    /// went away.
    pub(super) fn accept(
        mut stream: TcpStream,
        secret: [u8; 32],
        build: &str,
        schema: u32,
    ) -> Option<Self> {
        let identity = Keypair::from_secret(secret);
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
            build,
            role: Role::Owner,
            epoch: 1,
            schema,
        };
        let len = wellfed_proto::encode(&welcome, &mut plain).ok()?;
        let written = handshake.write_message(&plain[..len], &mut sealed).ok()?;
        write_frame(&mut stream, &sealed[..written])?;
        let transport = handshake.into_transport().ok()?;
        Some(Self { stream, transport })
    }

    /// The client's next message, opened into `plain`: its length.
    pub(super) fn receive(&mut self, plain: &mut [u8]) -> Option<usize> {
        let message = read_frame(&mut self.stream)?;
        self.transport.open(&message, plain).ok()
    }

    /// Seal `plain` as a message of its own and send it.
    pub(super) fn send(&mut self, plain: &[u8]) -> Option<()> {
        let mut sealed = [0; FRAME_MAX];
        let written = self.transport.seal(plain, &mut sealed).ok()?;
        write_frame(&mut self.stream, &sealed[..written])
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
