//! The shared core serves a device of any kind: one with no feeder module
//! refuses the feeder's ops `Unavailable`, after the same retry and role
//! checks as any write, and still answers the shared ones.

mod support;

use support::{Faked, Rig};
use wellfed_proto::api::{
    ClientMessage, DeviceMessage, FRAME_MAX, FeederOp, Op, Outcome, Refusal, RequestId,
};
use wellfed_session::noise::Transport;

/// Seal `op` under `id` to the slot, and the answer to `id` once it comes.
fn ask<D: Faked>(
    rig: &mut Rig<D>,
    slot: usize,
    transport: &mut Transport,
    id: RequestId,
    op: Op,
) -> String {
    let mut plain = [0; 64];
    let len = wellfed_proto::encode(&ClientMessage::Request { id, op }, &mut plain).unwrap();
    let mut sealed = [0; FRAME_MAX];
    let len = transport.seal(&plain[..len], &mut sealed).unwrap();
    let bytes = support::framed(&sealed[..len]);
    assert_eq!(rig.deliver(slot, &bytes), bytes.len());

    for _ in 0..8 {
        rig.pass();
        let mut plain = [0; FRAME_MAX];
        for reply in rig.collect(slot) {
            let read = transport.open(&reply, &mut plain).unwrap();
            if let Ok(DeviceMessage::Response {
                id: answered,
                result,
            }) = wellfed_proto::decode::<DeviceMessage<'_>>(&plain[..read])
                && answered == id
            {
                return format!("{result:?}");
            }
        }
    }
    panic!("no answer to {id:?}");
}

#[test]
fn a_device_without_a_feeder_module_refuses_its_ops_and_remembers_the_refusal() {
    // A device of another kind: its feeder module is `NoFeeder`.
    let mut rig = Rig::fountain();
    rig.hold_reservations(false);
    let (slot, mut transport) = rig.open();
    let unavailable = format!("{:?}", Outcome::Refused(Refusal::Unavailable));

    let feed = Op::Feeder(FeederOp::Feed { portions: 1 });
    let id = [7; 16];
    assert_eq!(ask(&mut rig, slot, &mut transport, id, feed), unavailable);
    assert_eq!(rig.retried(), 0);
    // A retry of the write id is answered from memory, as for any write.
    assert_eq!(ask(&mut rig, slot, &mut transport, id, feed), unavailable);
    assert_eq!(rig.retried(), 1);
    assert_eq!(rig.feeds(), 0, "nothing reached a feeding owner");

    // A read is refused too, and the shared core still answers.
    let plans = Op::Feeder(FeederOp::GetPlans);
    assert_eq!(
        ask(&mut rig, slot, &mut transport, [8; 16], plans),
        unavailable
    );
    let status = ask(&mut rig, slot, &mut transport, [9; 16], Op::Status);
    assert!(status.starts_with("Status("), "{status}");
}
