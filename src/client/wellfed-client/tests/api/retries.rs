//! Retries of writes the device finishes later: a retry under the same id,
//! on a new session after the first one dropped, waits for the write the
//! first queued and hears its answer, never running it again; and a write
//! whose outcome the device lost ends every session waiting on it
//! unanswered.

use wellfed_device_api::clients::Role;
use wellfed_device_api::mailbox::KEYS_FINISHED;
use wellfed_device_api::{KeyOutcome, Reason, UpdateAnswer};
use wellfed_model as model;
use wellfed_model::{Answer, Generation, Op};

use crate::support::{
    Client_, MEMBER, OWNER, Rig, answered_unwritten, open, poll_answer, public, receiving,
    send_unwritten, view,
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

fn put_time() -> Op {
    Op::PutTime {
        base: Generation(0),
        changes: model::TimeChanges {
            zone: Some("MST7MDT,M3.2.0,M11.1.0".to_owned()),
            ntp: Some(model::NtpServer::Gateway { port: 1123 }),
        },
    }
}

fn offer() -> Op {
    Op::OfferUpdate {
        manifest: vec![0xa5; 160],
        downgrade: None,
    }
}

/// A revocation and a time settings write, each queued by a session that
/// dropped before its answer: the retry waits while the writer holds the
/// change, then hears it landed, and the change runs once.
#[test]
fn a_retried_revocation_or_time_write_waits_for_the_first() {
    for time in [false, true] {
        let mut rig = Rig::new();
        // Opening a session runs the writer, so both open first.
        let mut owner = open(&mut rig, OWNER);
        let mut again = open(&mut rig, OWNER);
        let op = if time { put_time() } else { revoke_member() };

        send_unwritten(&mut rig, &mut owner, 1, op.clone());
        drop_connection(&mut rig, &owner);
        send_unwritten(&mut rig, &mut again, 1, op);
        assert!(!answered_unwritten(&mut rig, &mut again, 1), "time={time}");

        let answer = poll_answer(&mut rig, &mut again, 1, 5);
        let generation = time.then_some(Generation(1));
        assert_eq!(answer, Some(Answer::Done { generation }), "time={time}");
        if time {
            assert_eq!(rig.device.time.generation, 1, "saved once");
        } else {
            assert!(!rig.device.clients.iter().any(|c| c.key == public(MEMBER)));
        }
    }
}

/// An update offer queued by a session that dropped: its retry waits for
/// the update stage's answer instead of offering again, and hears the
/// install it started.
#[test]
fn a_retried_update_offer_waits_for_the_first() {
    let mut rig = Rig::new();
    rig.device.update = true;
    let mut owner = open(&mut rig, OWNER);
    let mut again = open(&mut rig, OWNER);
    rig.device.hold = true;

    send_unwritten(&mut rig, &mut owner, 1, offer());
    drop_connection(&mut rig, &owner);
    send_unwritten(&mut rig, &mut again, 1, offer());
    assert!(!answered_unwritten(&mut rig, &mut again, 1));

    rig.device.hold = false;
    rig.device.answer_held();
    let answer = poll_answer(&mut rig, &mut again, 1, 5);
    assert_eq!(answer, Some(view(receiving())));
    assert_eq!(rig.device.updates.len(), 1, "offered once");
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

/// A client's memory holding a write under way is the last given to a new
/// key: four other clients connecting while it waits leave it be, and the
/// retry still finds the write.
#[test]
fn a_memory_with_a_write_under_way_outlasts_other_clients() {
    let mut rig = Rig::new();
    rig.device.update = true;
    let others = [MEMBER, [0x50; 32], [0x60; 32], [0x70; 32]];
    for secret in &others[1..] {
        rig.device.enroll(public(*secret), Role::Member);
    }
    rig.publish();
    let mut owner = open(&mut rig, OWNER);
    rig.device.hold = true;
    send_unwritten(&mut rig, &mut owner, 1, offer());
    drop_connection(&mut rig, &owner);

    for secret in others {
        let other = open(&mut rig, secret);
        drop_connection(&mut rig, &other);
    }

    let mut again = open(&mut rig, OWNER);
    send_unwritten(&mut rig, &mut again, 1, offer());
    assert!(!answered_unwritten(&mut rig, &mut again, 1));
    rig.device.hold = false;
    rig.device.answer_held();
    let answer = poll_answer(&mut rig, &mut again, 1, 5);
    assert_eq!(answer, Some(view(receiving())));
    assert_eq!(rig.device.updates.len(), 1, "offered once");
}
