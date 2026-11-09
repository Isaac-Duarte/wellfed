//! A plan that carries a later build's extension tail is refused whole: the
//! device cannot store what the tail asks, and a whole-replace write that
//! dropped it would lose what the client set. Simulation, against the fake
//! device.

mod support;

use support::Rig;
use wellfed_proto::api::{
    ClientMessage, DeviceMessage, FRAME_MAX, FeederOp, Op, Outcome, PLANS_MAX, Plan, Refusal, Tail,
};
use wellfed_session::noise::Transport;

/// The plaintext of a `PutPlans` whose last slot holds a plan; with `tail`,
/// that plan's extension carries those bytes, as a later build writes them.
fn put_plans(id: [u8; 16], tail: &[u8]) -> Vec<u8> {
    let mut plans = [None; PLANS_MAX];
    plans[PLANS_MAX - 1] = Some(Plan {
        minute: 480,
        portions: 2,
        days: 0x7f,
        enabled: true,
        call: 0,
        ext: Tail::EMPTY,
    });
    let message = ClientMessage::Request {
        id,
        op: Op::Feeder(FeederOp::PutPlans { base: 0, plans }),
    };
    let mut plain = [0; 256];
    let len = wellfed_proto::encode(&message, &mut plain).unwrap();

    // The last slot's plan ends the message, and its empty tail is the
    // final byte: a length of zero.
    let mut plain = plain[..len].to_vec();
    assert_eq!(plain.pop(), Some(0));
    plain.push(u8::try_from(tail.len()).unwrap());
    plain.extend_from_slice(tail);
    plain
}

/// The answer to `plain`'s request `id`, within a few passes.
fn answer(rig: &mut Rig, plain: &[u8], id: [u8; 16]) -> Outcome<'static> {
    let (slot, mut transport): (usize, Transport) = rig.open();
    let mut sealed = [0; FRAME_MAX];
    let len = transport.seal(plain, &mut sealed).unwrap();
    let bytes = support::framed(&sealed[..len]);
    assert_eq!(rig.deliver(slot, &bytes), bytes.len());

    let mut read = [0; FRAME_MAX];
    for _ in 0..8 {
        rig.pass();
        for reply in rig.collect(slot) {
            let len = transport.open(&reply, &mut read).unwrap();
            if let Ok(DeviceMessage::Response { id: to, result }) =
                wellfed_proto::decode::<DeviceMessage<'_>>(&read[..len])
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

#[test]
fn a_plan_with_a_tail_this_build_cannot_read_is_refused_invalid() {
    let mut rig = Rig::new();

    let tailed = answer(&mut rig, &put_plans([1; 16], &[1, 2, 3]), [1; 16]);
    let plain = answer(&mut rig, &put_plans([2; 16], &[]), [2; 16]);

    assert_eq!(tailed, Outcome::Refused(Refusal::Invalid));
    // The same plan without its tail passes the table's checks (the fake's
    // writer never lands plans, so it ends `Busy`).
    assert_ne!(plain, Outcome::Refused(Refusal::Invalid));
}
