//! A feed the device accepted and numbered can still be refused before it is
//! reserved, and the next feed then takes the same number. Whoever asked hears
//! the refusal, whether its session still waits or a new one retries the id,
//! and never that later feed's reservation as its own.

mod support;

use support::Rig;
use wellfed_proto::api::{ClientMessage, FRAME_MAX, FeederMessage, Op, Outcome, Refusal};
use wellfed_session::noise::Transport;

const FEED_ID: [u8; 16] = [1; 16];
const REFUSED: Outcome<'static> = Outcome::Refused(Refusal::Busy);

/// Seal `op` under `id` to the slot.
fn ask(rig: &mut Rig, slot: usize, transport: &mut Transport, id: [u8; 16], op: Op) {
    let mut plain = [0; 64];
    let len = wellfed_proto::encode(&ClientMessage::Request { id, op }, &mut plain).unwrap();
    let mut sealed = [0; FRAME_MAX];
    let len = transport.seal(&plain[..len], &mut sealed).unwrap();
    let bytes = support::framed(&sealed[..len]);
    assert_eq!(rig.deliver(slot, &bytes), bytes.len());
}

/// A pass, and whether the answer to `id` came out of it as `expected`.
fn answered(rig: &mut Rig, slot: usize, transport: &mut Transport, expected: &Outcome<'_>) -> bool {
    rig.pass();
    let mut plain = [0; FRAME_MAX];
    rig.collect(slot).iter().any(|reply| {
        let read = transport.open(reply, &mut plain).unwrap();
        match wellfed_proto::decode::<FeederMessage<'_>>(&plain[..read]) {
            Ok(FeederMessage::Response { id, result }) if id == FEED_ID => {
                assert_eq!(&result, expected);
                true
            }
            _ => false,
        }
    })
}

#[test]
fn a_session_waiting_for_a_refused_feed_hears_the_refusal_not_the_next_feed() {
    let mut rig = Rig::new();
    let (slot, mut transport) = rig.open();
    rig.hold_reservations(true);

    ask(
        &mut rig,
        slot,
        &mut transport,
        FEED_ID,
        Op::Feed { portions: 1 },
    );
    for _ in 0..4 {
        assert!(!answered(&mut rig, slot, &mut transport, &REFUSED));
    }

    // The next feed, under the same number, is reserved at once.
    rig.feed_refused(1, Refusal::Busy);
    rig.hold_reservations(false);
    assert!(answered(&mut rig, slot, &mut transport, &REFUSED));
}

#[test]
fn a_retry_after_a_refused_feed_hears_the_refusal_on_any_session() {
    let mut rig = Rig::new();
    let (slot, mut transport) = rig.open();
    rig.hold_reservations(true);
    ask(
        &mut rig,
        slot,
        &mut transport,
        FEED_ID,
        Op::Feed { portions: 1 },
    );
    rig.pass();
    rig.feed_refused(1, Refusal::Busy);
    rig.hold_reservations(false);
    assert!(answered(&mut rig, slot, &mut transport, &REFUSED));

    ask(
        &mut rig,
        slot,
        &mut transport,
        FEED_ID,
        Op::Feed { portions: 1 },
    );
    assert!(answered(&mut rig, slot, &mut transport, &REFUSED));
    assert_eq!(rig.feeds(), 1, "the retry queued nothing");
}

/// Only the feed named: another's waiting request goes on.
#[test]
fn a_refusal_of_another_number_leaves_a_waiting_feed_alone() {
    let mut rig = Rig::new();
    let (slot, mut transport) = rig.open();
    rig.hold_reservations(true);
    ask(
        &mut rig,
        slot,
        &mut transport,
        FEED_ID,
        Op::Feed { portions: 1 },
    );
    rig.pass();
    rig.feed_refused(9, Refusal::Busy);
    let feeding = Outcome::Feeding { feed: 1 };
    assert!(!answered(&mut rig, slot, &mut transport, &feeding));

    rig.hold_reservations(false);
    assert!(answered(&mut rig, slot, &mut transport, &feeding));
}
