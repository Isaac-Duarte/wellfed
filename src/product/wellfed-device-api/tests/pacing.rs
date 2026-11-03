//! The loop passes every 20 ms unless the next pass has work. A session waiting
//! on something outside the loop (a client to read, the writer, a press) is
//! not work, and a client that stops reading loses its slot rather than keeping
//! it for as long as it stays connected.

mod support;

use support::Rig;
use wellfed_device_api::{Reason, STALL_MS};
use wellfed_proto::api::{
    ClientMessage, DeviceMessage, FRAME_MAX, FeederOp, FeederOutcome, Op, Outcome, RequestId,
    SettingsChanges,
};
use wellfed_session::noise::Transport;

const ID: RequestId = [1; 16];
/// The rig's passes are this far apart, as the loop's are at rest.
const PASS_MS: u32 = 20;
/// Passes that cover the stall limit, with a pass over for the slot's end.
const STALL_PASSES: u32 = STALL_MS / PASS_MS + 2;

/// Seal `op` under `id` to the slot.
fn ask(rig: &mut Rig, slot: usize, transport: &mut Transport, id: RequestId, op: Op) {
    let mut plain = [0; 256];
    let len = wellfed_proto::encode(&ClientMessage::Request { id, op }, &mut plain).unwrap();
    let mut sealed = [0; FRAME_MAX];
    let len = transport.seal(&plain[..len], &mut sealed).unwrap();
    let bytes = support::framed(&sealed[..len]);
    assert_eq!(rig.deliver(slot, &bytes), bytes.len());
}

/// The answer among what `slot` has queued, if any, as its client reads it.
fn answer(rig: &mut Rig, slot: usize, transport: &mut Transport) -> Option<String> {
    let mut plain = [0; FRAME_MAX];
    rig.collect(slot).iter().find_map(|reply| {
        let read = transport.open(reply, &mut plain).unwrap();
        match wellfed_proto::decode::<DeviceMessage<'_>>(&plain[..read]) {
            Ok(DeviceMessage::Response { id, result }) if id == ID => Some(format!("{result:?}")),
            _ => None,
        }
    })
}

/// A session whose client has been sent an answer and is not reading it.
fn unread_answer() -> (Rig, usize) {
    let mut rig = Rig::new();
    let (slot, mut transport) = rig.open();
    ask(&mut rig, slot, &mut transport, ID, Op::Status);
    assert!(rig.pass(), "a request is work");
    assert!(rig.queued(slot) > 0);
    (rig, slot)
}

/// The loop's rest while nothing goes out: not one pass reports work, and the
/// session stays until the limit.
#[test]
fn a_client_that_stops_reading_is_closed_and_the_loop_rests_meanwhile() {
    let (mut rig, slot) = unread_answer();

    for _ in 0..STALL_MS / PASS_MS - 1 {
        assert!(!rig.pass(), "waiting for the client is not work");
        assert!(!rig.ended(slot), "inside the limit");
    }

    let passes = (0..3).take_while(|_| {
        rig.pass();
        !rig.ended(slot)
    });
    assert!(passes.count() < 3, "dropped at the limit");
    assert_eq!(rig.ended_for(), [Reason::Stalled]);
    assert_eq!(rig.sessions(), 0, "its slot is free");
    assert!(!rig.pass(), "back to resting");
    assert!(rig.accept().is_some(), "and the slot takes a new client");
}

#[test]
fn bytes_that_move_restart_the_limit() {
    let (mut rig, slot) = unread_answer();

    for _ in 0..2 {
        for _ in 0..STALL_MS / PASS_MS - 2 {
            assert!(!rig.pass());
            assert!(!rig.ended(slot));
        }
        rig.send_some(slot, 1);
    }
    assert!(rig.queued(slot) > 0);

    for _ in 0..STALL_PASSES {
        rig.pass();
    }
    assert!(rig.ended(slot), "nothing moved since");
    assert_eq!(rig.ended_for(), [Reason::Stalled]);
}

/// A slot already ending stays only until its last bytes are out, and those
/// wait no longer than a session's would.
#[test]
fn a_closing_slot_whose_client_stopped_reading_is_freed() {
    let (mut rig, slot) = unread_answer();
    rig.revoke_everyone();
    assert_eq!(rig.ended_for(), [Reason::Revoked]);
    assert!(rig.queued(slot) > 0);

    for _ in 0..STALL_MS / PASS_MS - 1 {
        assert!(!rig.pass());
        assert!(!rig.ended(slot), "its last bytes may still go");
    }
    for _ in 0..3 {
        rig.pass();
    }
    assert!(rig.ended(slot));
    assert_eq!(rig.sessions(), 0);
    assert_eq!(rig.ended_for(), [Reason::Revoked], "it ended only once");
}

/// A client that reads, closed by nothing: only unsent bytes run the limit.
#[test]
fn a_quiet_session_is_not_stalled() {
    let mut rig = Rig::new();
    let (slot, _transport) = rig.open();
    for _ in 0..STALL_PASSES {
        assert!(!rig.pass());
    }
    assert!(!rig.ended(slot));
    assert!(rig.ended_for().is_empty());
}

/// The answer to a feed that never reached its client: dropping the session
/// loses no more than a lost connection does. The retry hears that first
/// answer on a new session, and no second feed runs.
#[test]
fn a_stalled_feed_answer_is_heard_by_the_retry_and_feeds_once() {
    let mut rig = Rig::new();
    let (slot, mut transport) = rig.open();
    rig.hold_reservations(false);
    ask(
        &mut rig,
        slot,
        &mut transport,
        ID,
        Op::Feeder(FeederOp::Feed { portions: 1 }),
    );
    rig.pass();
    rig.pass();
    assert!(rig.queued(slot) > 0, "the answer waits for its client");

    for _ in 0..STALL_PASSES {
        rig.pass();
    }
    assert!(rig.ended(slot));
    assert_eq!(rig.ended_for(), [Reason::Stalled]);

    let (slot, mut transport) = rig.open();
    ask(
        &mut rig,
        slot,
        &mut transport,
        ID,
        Op::Feeder(FeederOp::Feed { portions: 1 }),
    );
    rig.pass();
    let heard = answer(&mut rig, slot, &mut transport).expect("the retry's answer");
    assert_eq!(
        heard,
        format!("{:?}", Outcome::Feeder(FeederOutcome::Feeding { feed: 1 }))
    );
    assert_eq!(rig.feeds(), 1, "the retry queued nothing");
}

/// A write in flight sends nothing, so however long it takes it is never what
/// a client loses its session over.
#[test]
fn a_feed_waiting_for_its_reservation_rests_the_loop_and_keeps_its_session() {
    let mut rig = Rig::new();
    let (slot, mut transport) = rig.open();
    rig.hold_reservations(true);
    ask(
        &mut rig,
        slot,
        &mut transport,
        ID,
        Op::Feeder(FeederOp::Feed { portions: 1 }),
    );
    assert!(rig.pass(), "the request itself is work");

    for _ in 0..STALL_PASSES {
        assert!(!rig.pass(), "a reservation on its way is not");
    }
    assert!(!rig.ended(slot));

    rig.hold_reservations(false);
    assert!(rig.pass(), "its answer is");
    let heard = answer(&mut rig, slot, &mut transport).expect("the answer");
    assert_eq!(
        heard,
        format!("{:?}", Outcome::Feeder(FeederOutcome::Feeding { feed: 1 }))
    );
}

#[test]
fn a_settings_write_the_writer_has_not_reached_rests_the_loop() {
    let mut rig = Rig::new();
    let (slot, mut transport) = rig.open();
    rig.hold_writes(true);
    let changes = SettingsChanges {
        volume: Some(10),
        ..SettingsChanges::default()
    };
    ask(
        &mut rig,
        slot,
        &mut transport,
        ID,
        Op::Feeder(FeederOp::PutSettings { base: 0, changes }),
    );
    assert!(rig.pass(), "the request itself is work");
    assert_eq!(rig.queued(slot), 0, "held for the writer");

    for _ in 0..200 {
        assert!(!rig.pass(), "the writer's poll is not the loop's work");
    }

    rig.hold_writes(false);
    assert!(rig.pass(), "its answer is");
    let heard = answer(&mut rig, slot, &mut transport).expect("the answer");
    assert!(heard.starts_with("Done"), "{heard}");
}
