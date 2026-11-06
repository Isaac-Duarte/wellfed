//! A client's skip of a scheduled meal and its undo through the device API:
//! the device checks and queues it, the answer waits until the store holds
//! it, the event names who asked, and a retry of the id changes nothing.
//! Simulation, against a fake device whose writer lands each skip at once.

mod support;

use support::Rig;
use wellfed_proto::api::{
    ClientMessage, DeviceMessage, EventBody, FRAME_MAX, FeederOp, Op, Outcome, Refusal, Source,
};
use wellfed_session::noise::Transport;

const DUE_S: u32 = 1_793_811_600;
/// A skip's answer: it names no generation.
const DONE: Outcome<'static> = Outcome::Done { generation: None };

/// Seal `op` under `id` to the slot.
fn ask(rig: &mut Rig, slot: usize, transport: &mut Transport, id: [u8; 16], op: FeederOp<'_>) {
    let mut plain = [0; 64];
    let message = ClientMessage::Request {
        id,
        op: Op::Feeder(op),
    };
    let len = wellfed_proto::encode(&message, &mut plain).unwrap();
    let mut sealed = [0; FRAME_MAX];
    let len = transport.seal(&plain[..len], &mut sealed).unwrap();
    let bytes = support::framed(&sealed[..len]);
    assert_eq!(rig.deliver(slot, &bytes), bytes.len());
}

/// The answer to `id`, within a few passes.
fn answer(rig: &mut Rig, slot: usize, transport: &mut Transport, id: [u8; 16]) -> Outcome<'static> {
    let mut plain = [0; FRAME_MAX];
    for _ in 0..8 {
        rig.pass();
        for reply in rig.collect(slot) {
            let read = transport.open(&reply, &mut plain).unwrap();
            if let Ok(DeviceMessage::Response { id: to, result }) =
                wellfed_proto::decode::<DeviceMessage<'_>>(&plain[..read])
                && to == id
            {
                return match result {
                    Outcome::Done { generation } => Outcome::Done { generation },
                    Outcome::Refused(refusal) => Outcome::Refused(refusal),
                    other => panic!("not a write's answer: {other:?}"),
                };
            }
        }
    }
    panic!("no answer to {id:?}");
}

fn skip(slot: u8) -> FeederOp<'static> {
    FeederOp::SkipMeal { slot, due_s: DUE_S }
}

fn unskip(slot: u8) -> FeederOp<'static> {
    FeederOp::UnskipMeal { slot, due_s: DUE_S }
}

#[test]
fn a_skip_and_its_undo_are_done_once_the_store_holds_them_and_recorded_by_client() {
    let mut rig = Rig::new();
    let (slot, mut transport) = rig.open();

    ask(&mut rig, slot, &mut transport, [1; 16], skip(2));
    assert_eq!(answer(&mut rig, slot, &mut transport, [1; 16]), DONE);
    assert_eq!(rig.mark(2), DUE_S);

    ask(&mut rig, slot, &mut transport, [2; 16], unskip(2));
    assert_eq!(answer(&mut rig, slot, &mut transport, [2; 16]), DONE);
    assert!(rig.mark(2) < DUE_S);

    let recorded: Vec<_> = rig
        .events()
        .into_iter()
        .filter(|body| {
            matches!(
                body,
                EventBody::Skipped { .. } | EventBody::Unskipped { .. }
            )
        })
        .collect();
    let [
        EventBody::Skipped {
            plan: 2,
            due: DUE_S,
            by: Source::Client(skipper),
        },
        EventBody::Unskipped {
            plan: 2,
            due: DUE_S,
            by: Source::Client(undoer),
        },
    ] = recorded[..]
    else {
        panic!("{recorded:?}");
    };
    assert_eq!(skipper, undoer, "the one owner asked both");
}

#[test]
fn a_retried_skip_is_answered_from_its_id_and_asks_the_device_nothing() {
    let mut rig = Rig::new();
    let (slot, mut transport) = rig.open();
    ask(&mut rig, slot, &mut transport, [3; 16], skip(0));
    assert_eq!(answer(&mut rig, slot, &mut transport, [3; 16]), DONE);

    ask(&mut rig, slot, &mut transport, [3; 16], skip(0));
    assert_eq!(answer(&mut rig, slot, &mut transport, [3; 16]), DONE);
    assert_eq!(rig.skips(), 1);
    assert_eq!(rig.retried(), 1);
}

#[test]
fn the_device_s_refusal_is_the_answer_and_nothing_is_recorded() {
    let mut rig = Rig::new();
    let (slot, mut transport) = rig.open();
    for (id, refusal) in [
        (4, Refusal::NoClock),
        (5, Refusal::Invalid),
        (6, Refusal::NotFound),
        (7, Refusal::Busy),
    ] {
        rig.refuse_skips(Some(refusal));
        ask(&mut rig, slot, &mut transport, [id; 16], skip(1));
        let refused = Outcome::Refused(refusal);
        assert_eq!(answer(&mut rig, slot, &mut transport, [id; 16]), refused);
    }
    assert!(
        !rig.events()
            .iter()
            .any(|body| matches!(body, EventBody::Skipped { .. }))
    );
}

#[test]
fn a_skip_the_writer_drops_is_refused_not_writable_and_not_recorded() {
    let mut rig = Rig::new();
    let (slot, mut transport) = rig.open();
    rig.drop_skips(true);

    ask(&mut rig, slot, &mut transport, [8; 16], skip(3));
    let refused = Outcome::Refused(Refusal::NotWritable);
    assert_eq!(answer(&mut rig, slot, &mut transport, [8; 16]), refused);
    assert_eq!(rig.mark(3), 0);
    assert!(
        !rig.events()
            .iter()
            .any(|body| matches!(body, EventBody::Skipped { .. }))
    );
}
