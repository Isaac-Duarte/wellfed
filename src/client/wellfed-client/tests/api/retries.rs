//! Writes the device finishes later whose outcome it lost: every session
//! waiting on one ends unanswered.

use wellfed_device_api::mailbox::KEYS_FINISHED;
use wellfed_device_api::{KeyOutcome, Reason, UpdateAnswer};
use wellfed_model::Op;

use crate::support::{
    Client_, MEMBER, OWNER, Rig, answered_unwritten, open, public, receiving, send_unwritten,
};

/// The connection under `client` drops.
fn drop_connection(rig: &mut Rig, client: &Client_) {
    let records = &mut rig.records;
    rig.server.closed(client.slot, |r| records.push(r));
    rig.pass();
}

/// Later client changes finish, as many as the mailbox keeps, so the
/// results finished before them leave it.
fn age_out_key_results(rig: &mut Rig) {
    for _ in 0..KEYS_FINISHED {
        rig.device.ticket += 1;
        if rig.device.keys_done.len() == KEYS_FINISHED {
            rig.device.keys_done.remove(0);
        }
        let ticket = rig.device.ticket;
        rig.device.keys_done.push((ticket, KeyOutcome::Done));
    }
}

fn revoke_member() -> Op {
    Op::RevokeClient {
        key: public(MEMBER),
    }
}

fn offer() -> Op {
    Op::OfferUpdate {
        manifest: vec![0xa5; 160],
        downgrade: None,
    }
}

/// A revocation whose result left the mailbox before anyone read it: the
/// session that asked ends unanswered (`Forgotten`), and so does a retry,
/// rather than either waiting for good or hearing a guess.
#[test]
fn a_write_whose_outcome_is_lost_ends_unanswered() {
    // The session that asked is still there.
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    send_unwritten(&mut rig, &mut owner, 1, revoke_member());
    rig.device.write();
    age_out_key_results(&mut rig);
    assert!(!answered_unwritten(&mut rig, &mut owner, 1));
    assert!(rig.ended(Reason::Forgotten));

    // It dropped first; the pass settles the write as lost, and the retry
    // hears nothing either.
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    let mut again = open(&mut rig, OWNER);
    send_unwritten(&mut rig, &mut owner, 1, revoke_member());
    drop_connection(&mut rig, &owner);
    rig.device.write();
    age_out_key_results(&mut rig);
    rig.pass();
    assert!(!rig.ended(Reason::Forgotten), "nobody was waiting yet");
    send_unwritten(&mut rig, &mut again, 1, revoke_member());
    assert!(!answered_unwritten(&mut rig, &mut again, 1));
    assert!(rig.ended(Reason::Forgotten));
}

/// An update request whose answer a later request's replaced: its session
/// ends unanswered.
#[test]
fn an_update_whose_outcome_is_lost_ends_unanswered() {
    let mut rig = Rig::new();
    rig.device.update = true;
    let mut owner = open(&mut rig, OWNER);
    rig.device.hold = true;
    send_unwritten(&mut rig, &mut owner, 1, offer());

    let (ticket, _) = rig.device.held.take().unwrap();
    rig.device.update_done = Some((ticket + 1, Ok(UpdateAnswer::View(receiving()))));
    assert!(!answered_unwritten(&mut rig, &mut owner, 1));
    assert!(rig.ended(Reason::Forgotten));
}
