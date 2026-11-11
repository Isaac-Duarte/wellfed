//! The frozen update path as the server serves it: a request from a client
//! of another build whose op this build cannot decode is refused
//! `Unavailable` and the session goes on, and every answer to a frozen op
//! is one the frozen contract holds, so a client of any later build reads it.

mod support;

use proptest::prelude::*;
use support::{Rig, answer, request, send};
use wellfed_device_api::Reason;
use wellfed_proto::api::{Op, Outcome, Refusal, Role, UPDATE_CHUNK_MAX, UpdateOp};

/// An op past every one this build knows, as a newer client would send it:
/// refused `Unavailable` under its id, and the session still serves.
#[test]
fn an_op_this_build_does_not_know_is_refused_unavailable() {
    let mut rig = Rig::new();
    let (slot, mut transport) = rig.open();

    let mut unknown = vec![0];
    unknown.extend([0x42; 16]);
    unknown.extend([0x7f, 1, 2, 3]);
    let answers = send(&mut rig, slot, &mut transport, &unknown);
    assert_eq!(answers.len(), 1);
    assert_eq!(
        answer(&answers[0]),
        ([0x42; 16], Outcome::Refused(Refusal::Unavailable))
    );
    assert!(!rig.ended(slot));

    // A known op with fields this build cannot read is the same to it.
    let mut garbled = request([0x43; 16], Op::Update(UpdateOp::State));
    garbled.push(0xff);
    let answers = send(&mut rig, slot, &mut transport, &garbled);
    assert_eq!(
        answer(&answers[0]),
        ([0x43; 16], Outcome::Refused(Refusal::Unavailable))
    );

    let answers = send(
        &mut rig,
        slot,
        &mut transport,
        &request([0x44; 16], Op::Alive),
    );
    assert_eq!(
        answer(&answers[0]),
        ([0x44; 16], Outcome::Done { generation: None })
    );
    assert!(!rig.ended(slot));
}

/// Bytes with no readable request id are no request: the session ends as
/// before.
#[test]
fn bytes_with_no_request_id_still_end_the_session() {
    for garbage in [&[0x05, 1, 2][..], &[0, 1, 2][..], &[][..]] {
        let mut rig = Rig::new();
        let (slot, mut transport) = rig.open();
        let answers = send(&mut rig, slot, &mut transport, garbage);
        assert!(answers.is_empty(), "{garbage:?}");
        assert!(rig.ended(slot), "{garbage:?}");
        assert_eq!(rig.ended_for(), [Reason::Malformed], "{garbage:?}");
    }
}

fn frozen_op() -> impl Strategy<Value = (Vec<u8>, Vec<u8>, Vec<u8>, String, u8, u32)> {
    (
        proptest::collection::vec(any::<u8>(), 0..120),
        proptest::collection::vec(any::<u8>(), 0..120),
        proptest::collection::vec(any::<u8>(), 0..=UPDATE_CHUNK_MAX + 1),
        "[a-z]{0,20}",
        0u8..9,
        any::<u32>(),
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Every answer to a frozen op, whatever its fields, is frozen.
    #[test]
    fn every_answer_to_a_frozen_op_is_frozen(
        (manifest, downgrade, data, label, which, offset) in frozen_op(),
    ) {
        let op = match which {
            0 => Op::Update(UpdateOp::Offer { manifest: &manifest, downgrade: Some(&downgrade) }),
            1 => Op::Update(UpdateOp::Offer { manifest: &manifest, downgrade: None }),
            2 => Op::Update(UpdateOp::Chunk { offset, data: &data }),
            3 => Op::Update(UpdateOp::State),
            4 => Op::Update(UpdateOp::Cancel),
            5 => Op::Update(UpdateOp::EnrollKey { key: [7; 32], label: &label }),
            6 => Op::Update(UpdateOp::KeyStatement { statement: &manifest }),
            7 => Op::Alive,
            _ => Op::CreateInvite { role: Role::Member },
        };
        let mut rig = Rig::new();
        let (slot, mut transport) = rig.open();
        for plain in send(&mut rig, slot, &mut transport, &request([1; 16], op)) {
            let (_, result) = answer(&plain);
            prop_assert!(result.is_frozen(), "{op:?} got {result:?}");
        }
    }
}
