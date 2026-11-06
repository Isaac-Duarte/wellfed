//! The gateway probe's ICMP echo: the request the Wi-Fi task sends to the
//! lease's gateway, which datagrams answer it, and what the probe found.
//!
//! The probe reads one raw ICMP socket, which sees every ICMP datagram to
//! this host, and lwIP still answers echo requests itself. So a reply is
//! matched by its source, type, identifier and sequence, and anything else
//! is skipped until the probe's budget ([`TIMEOUT_MS`]) runs out. The gateway
//! is one hop away, so an answer later than that is as good as none.
//!
//! [`Outcome::code`] is the gateway event's `result`, which the host tools
//! read, and [`Outcome::probe`] what the link's watch
//! ([`crate::liveness`]) makes of it.

use crate::liveness::Probe;

/// The echo identifier, "PL".
pub const ID: u16 = 0x504c;
/// The echo's payload.
pub const PAYLOAD: [u8; 8] = *b"wellfed\0";
/// An echo request: the 8-byte ICMP header, then [`PAYLOAD`].
pub const REQUEST_BYTES: usize = 16;
/// How long one probe waits for its answer, over every datagram it skips.
pub const TIMEOUT_MS: u32 = 1_000;
/// How much of each datagram the probe reads: an IPv4 header without
/// options and the echo header fit with room to spare. A reply whose header
/// carries so many options that its echo header falls past this is skipped.
pub const RECEIVE_BYTES: usize = 64;

/// An IPv4 header's least length, without options.
const IPV4_MIN: usize = 20;
/// Where the IPv4 header holds the source address.
const IPV4_SOURCE: usize = 12;
/// ICMP types.
const ECHO_REPLY: u8 = 0;
const ECHO_REQUEST: u8 = 8;

/// The echo request numbered `seq`, its checksum filled in.
pub fn request(seq: u16) -> [u8; REQUEST_BYTES] {
    let mut message = [0; REQUEST_BYTES];
    message[0] = ECHO_REQUEST;
    message[4..6].copy_from_slice(&ID.to_be_bytes());
    message[6..8].copy_from_slice(&seq.to_be_bytes());
    message[8..].copy_from_slice(&PAYLOAD);

    let sum = checksum(&message);
    message[2..4].copy_from_slice(&sum.to_be_bytes());
    message
}

/// The Internet checksum (RFC 1071) of an even-length message: the ones'
/// complement of the ones'-complement sum of its big-endian words.
fn checksum(message: &[u8]) -> u16 {
    let mut sum = message
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&word| u32::from(u16::from_be_bytes(word)))
        .sum::<u32>();
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    let folded = u16::try_from(sum).unwrap_or(u16::MAX);
    !folded
}

/// Whether `datagram`, as the raw socket hands it over (the IPv4 header
/// first), is `gateway`'s (host order) reply to request `seq`.
pub fn answers(datagram: &[u8], gateway: u32, seq: u16) -> bool {
    let Some(&first) = datagram.first() else {
        return false;
    };
    if datagram.len() < IPV4_MIN {
        return false;
    }

    // The header's length, in words.
    let at = usize::from(first & 0x0f) * 4;
    let Some(echo) = datagram.get(at..at + 8) else {
        return false;
    };
    let Some(source) = datagram.get(IPV4_SOURCE..IPV4_SOURCE + 4) else {
        return false;
    };
    if at < IPV4_MIN || u32::from_be_bytes([source[0], source[1], source[2], source[3]]) != gateway
    {
        return false;
    }

    echo[0] == ECHO_REPLY && echo[4..6] == ID.to_be_bytes() && echo[6..8] == seq.to_be_bytes()
}

/// What one probe of the gateway found: the gateway event's `result`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The gateway answered the echo.
    Answered,
    /// The lease named no gateway, so nothing was sent.
    NoGateway,
    /// No socket: the stack's connections may all be taken by the device
    /// API's sessions and mDNS.
    NoSocket,
    /// The send failed.
    NotSent,
    /// Sent, and no answer within [`TIMEOUT_MS`].
    Silent,
}

impl Outcome {
    /// The gateway event's `result`: 0 answered, or a negative failure.
    pub const fn code(self) -> i32 {
        match self {
            Self::Answered => 0,
            Self::NoGateway => -1,
            Self::NoSocket => -2,
            Self::NotSent => -3,
            Self::Silent => -4,
        }
    }

    /// What the watch makes of it: a probe that never left asked nothing
    /// of the gateway.
    pub const fn probe(self) -> Probe {
        match self {
            Self::Answered => Probe::Answered,
            Self::NoGateway => Probe::NoGateway,
            Self::Silent => Probe::Silent,
            Self::NoSocket | Self::NotSent => Probe::Unsent,
        }
    }
}
