//! Who may send an op, as a session sees it: the role is checked before the
//! op's own fields, so a role that may not send an op is refused
//! `Forbidden` whether or not the op is well formed.

mod support;

use support::{MEMBER, Rig, answer, request, send};
use wellfed_proto::api::{Op, Outcome, Refusal, RequestId, UpdateOp};

/// An update chunk with no bytes is malformed: an owner, who may send
/// chunks, is refused `Invalid`; a member, who may not, `Forbidden`.
#[test]
fn a_malformed_op_is_forbidden_to_a_role_that_may_not_send_it() {
    let mut rig = Rig::new();
    rig.enroll_member();
    let empty = Op::Update(UpdateOp::Chunk {
        offset: 0,
        data: &[],
    });

    for (secret, refusal) in [
        (support::OWNER, Refusal::Invalid),
        (MEMBER, Refusal::Forbidden),
    ] {
        let (slot, mut transport) = rig.open_as(secret);
        let id: RequestId = [7; 16];
        let answers = send(&mut rig, slot, &mut transport, &request(id, empty));
        let [plain] = answers.as_slice() else {
            panic!("one answer, not {}", answers.len());
        };
        assert_eq!(answer(plain), (id, Outcome::Refused(refusal)));
    }
}
