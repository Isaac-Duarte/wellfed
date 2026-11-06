//! The gateway probe's echo: the request's bytes, which datagrams answer it,
//! and the result codes the host tools read.

use wellfed_core::echo::{self, ID, Outcome, PAYLOAD, RECEIVE_BYTES, REQUEST_BYTES};
use wellfed_core::liveness::Probe;

const GATEWAY: u32 = 0xc0a8_0101;

/// The reply `gateway` sends to request `seq`, behind an IPv4 header of
/// `header_words` words.
fn reply(gateway: u32, seq: u16, header_words: u8) -> Vec<u8> {
    let mut datagram = vec![0; usize::from(header_words) * 4];
    datagram[0] = 0x40 | header_words;
    datagram[9] = 1;
    datagram[12..16].copy_from_slice(&gateway.to_be_bytes());

    let mut message = echo::request(seq).to_vec();
    message[0] = 0;
    datagram.extend_from_slice(&message);
    datagram
}

/// The Internet checksum's defining property: summed with it, a message
/// sums to all ones.
fn sums_to_ones(message: &[u8]) -> bool {
    let mut sum: u32 = message
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&word| u32::from(u16::from_be_bytes(word)))
        .sum();
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    sum == 0xffff
}

#[test]
fn the_request_is_an_echo_with_its_id_sequence_and_payload() {
    let request = echo::request(1);

    assert_eq!(request.len(), REQUEST_BYTES);
    assert_eq!(
        request,
        [
            8, 0, 0xf9, 0x7a, 0x50, 0x4c, 0, 1, b'w', b'e', b'l', b'l', b'f', b'e', b'd', 0
        ],
        "worked by hand: the words sum to 0x20683, folded 0x0685"
    );
    assert_eq!(&request[8..], &PAYLOAD);
    assert_eq!(&request[4..6], &ID.to_be_bytes());

    for seq in [0, 1, 0x00ff, 0x0100, 0x7fff, 0xafb3, 0xffff] {
        let request = echo::request(seq);
        assert_eq!(&request[6..8], &seq.to_be_bytes());
        assert!(sums_to_ones(&request), "seq {seq:#06x}");
    }
}

#[test]
fn only_the_gateways_reply_to_this_request_answers() {
    assert!(echo::answers(&reply(GATEWAY, 7, 5), GATEWAY, 7));
    // A header with options: the echo starts where the header says.
    assert!(echo::answers(&reply(GATEWAY, 7, 6), GATEWAY, 7));

    assert!(
        !echo::answers(&reply(GATEWAY, 7, 5), GATEWAY, 8),
        "another sequence"
    );
    assert!(
        !echo::answers(&reply(0xc0a8_0102, 7, 5), GATEWAY, 7),
        "another host"
    );

    let mut other_id = reply(GATEWAY, 7, 5);
    other_id[24] ^= 1;
    assert!(!echo::answers(&other_id, GATEWAY, 7), "another identifier");

    // lwIP answers echo requests itself; the raw socket sees the request
    // a host on the LAN sent us, which is no reply.
    let mut request = reply(GATEWAY, 7, 5);
    request[20] = 8;
    assert!(!echo::answers(&request, GATEWAY, 7));
}

#[test]
fn a_short_or_malformed_datagram_is_skipped() {
    // The IPv4 header and the echo's 8-byte header are all a reply needs;
    // the payload may be cut.
    let whole = reply(GATEWAY, 7, 5);
    for len in 0..28 {
        assert!(!echo::answers(&whole[..len], GATEWAY, 7), "{len} bytes");
    }
    assert!(echo::answers(&whole[..28], GATEWAY, 7));

    // A header length below the least IPv4 header's.
    for words in 0..5 {
        let mut datagram = whole.clone();
        datagram[0] = 0x40 | words;
        assert!(!echo::answers(&datagram, GATEWAY, 7), "{words} words");
    }

    // The most options a header can carry put the echo past what the probe
    // reads.
    let longest = reply(GATEWAY, 7, 15);
    assert!(echo::answers(&longest, GATEWAY, 7));
    assert!(!echo::answers(&longest[..RECEIVE_BYTES], GATEWAY, 7));
}

#[test]
fn outcomes_keep_their_event_codes_and_watch_meanings() {
    let all = [
        (Outcome::Answered, 0, Probe::Answered),
        (Outcome::NoGateway, -1, Probe::NoGateway),
        (Outcome::NoSocket, -2, Probe::Unsent),
        (Outcome::NotSent, -3, Probe::Unsent),
        (Outcome::Silent, -4, Probe::Silent),
    ];
    for (outcome, code, probe) in all {
        assert_eq!(outcome.code(), code, "{outcome:?}");
        assert_eq!(outcome.probe(), probe, "{outcome:?}");
    }
}
