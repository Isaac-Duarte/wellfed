//! The client table over the API: roles, invites and joins, revocations,
//! and LAN recovery with its code, each change in the history once even when
//! its session left before reading the answer.

use wellfed_client::api::{Error, Invite, Join, JoinStep};
use wellfed_client::connection::Recovery;
use wellfed_core::buttons::{Button, Buttons, Gesture};
use wellfed_device_api::clients::Role;
use wellfed_device_api::mailbox::Lock;
use wellfed_device_api::pairing::Event as PairingEvent;
use wellfed_device_api::{Device, KeyChange, KeyOutcome, Reason};
use wellfed_model as model;
use wellfed_model::{Answer, Generation, Joined, Op};
use wellfed_proto::api::{self as proto, EventBody};
use wellfed_session::noise::Keypair;
use wellfed_session::setup_code::Code;

use crate::support::{
    Client_, FEEDER, MEMBER, OWNER, Rig, STRANGER, ask, fingerprint, open, poll_answer, public,
    send_unwritten,
};

#[test]
fn a_member_may_feed_and_change_settings_but_not_manage_clients() {
    let mut rig = Rig::new();
    let mut member = open(&mut rig, MEMBER);

    let wifi = ask(
        &mut rig,
        &mut member,
        1,
        Op::PutWifi {
            base: Generation(3),
            wifi: None,
        },
    );
    assert_eq!(wifi, Answer::Refused(model::Refusal::Forbidden));
    let revoke = ask(
        &mut rig,
        &mut member,
        2,
        Op::RevokeClient { key: public(OWNER) },
    );
    assert_eq!(revoke, Answer::Refused(model::Refusal::Forbidden));
    let reboot = ask(&mut rig, &mut member, 3, Op::Reboot);
    assert_eq!(reboot, Answer::Refused(model::Refusal::Forbidden));
    // The NTP server decides which fixes the clock hears: an owner's.
    let time = ask(
        &mut rig,
        &mut member,
        7,
        Op::PutTime {
            base: Generation(0),
            changes: model::TimeChanges::default(),
        },
    );
    assert_eq!(time, Answer::Refused(model::Refusal::Forbidden));

    let lock = ask(&mut rig, &mut member, 4, Op::SetLock { locked: true });
    assert_eq!(lock, Answer::Done { generation: None });
    let feed = ask(&mut rig, &mut member, 5, Op::Feed { portions: 1 });
    assert_eq!(feed, Answer::Feeding { feed: 1 });
    let cancel = ask(&mut rig, &mut member, 6, Op::CancelFeed { feed: 1 });
    assert_eq!(cancel, Answer::Done { generation: None });
}

#[test]
fn revoking_a_client_closes_its_open_session() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    let member = open(&mut rig, MEMBER);

    let listed = ask(&mut rig, &mut owner, 1, Op::ListClients);
    let Answer::Clients(clients) = listed else {
        panic!("{listed:?}");
    };
    assert_eq!(clients.len(), 2);
    assert!(
        clients
            .iter()
            .any(|c| c.you && c.role == model::ClientRole::Owner)
    );

    let revoke = ask(
        &mut rig,
        &mut owner,
        2,
        Op::RevokeClient {
            key: public(MEMBER),
        },
    );
    assert_eq!(revoke, Answer::Done { generation: None });
    rig.publish();
    assert!(rig.ended(Reason::Revoked));
    assert!(rig.server.closing(member.slot));
}

/// The history's revocations: the client revoked and who asked.
fn revocations(rig: &Rig) -> Vec<(proto::Fingerprint, proto::Source)> {
    rig.device
        .events
        .iter()
        .filter_map(|e| match e.body {
            EventBody::Revoked { client, by } => Some((client, by)),
            _ => None,
        })
        .collect()
}

/// The session goes once its revocation is queued: the writer still applies
/// it, so the history must still say so, once and naming who asked.
#[test]
fn a_revocation_whose_session_left_is_still_in_the_history() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    let revoke = Op::RevokeClient {
        key: public(MEMBER),
    };
    send_unwritten(&mut rig, &mut owner, 1, revoke);
    assert!(
        rig.device.keys_pending.is_some(),
        "the revocation is queued"
    );

    let records = &mut rig.records;
    rig.server.closed(owner.slot, |r| records.push(r));
    rig.pass();
    assert_eq!(revocations(&rig), [], "not before the writer has finished");

    rig.device.write();
    rig.pass();
    rig.pass();
    assert!(!rig.device.clients.iter().any(|c| c.key == public(MEMBER)));
    let by = proto::Source::Client(fingerprint(OWNER));
    assert_eq!(revocations(&rig), [(fingerprint(MEMBER), by)]);
}

/// A revocation its session read before it ended is in the history once:
/// the session's end leaves nothing to settle.
#[test]
fn a_revocation_read_before_its_session_left_is_in_the_history_once() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    let revoke = Op::RevokeClient {
        key: public(MEMBER),
    };
    send_unwritten(&mut rig, &mut owner, 1, revoke);
    rig.device.write();
    rig.pass();
    let by = proto::Source::Client(fingerprint(OWNER));
    assert_eq!(revocations(&rig), [(fingerprint(MEMBER), by)]);

    let records = &mut rig.records;
    rig.server.closed(owner.slot, |r| records.push(r));
    rig.pass();
    rig.pass();
    assert_eq!(revocations(&rig), [(fingerprint(MEMBER), by)]);
}

/// An owner revoking its own key: the new table ends its session before it
/// reads the result, and the history still has the revocation.
#[test]
fn an_owner_revoking_itself_is_in_the_history() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    let revoke = Op::RevokeClient { key: public(OWNER) };
    send_unwritten(&mut rig, &mut owner, 1, revoke);

    rig.device.write();
    rig.publish();
    assert!(rig.ended(Reason::Revoked));
    rig.pass();
    let by = proto::Source::Client(fingerprint(OWNER));
    assert_eq!(revocations(&rig), [(fingerprint(OWNER), by)]);
}

/// A left revocation the writer could not apply changed nothing: no event.
#[test]
fn a_left_revocation_that_found_nothing_is_not_in_the_history() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    let revoke = Op::RevokeClient {
        key: public(MEMBER),
    };
    send_unwritten(&mut rig, &mut owner, 1, revoke);
    let records = &mut rig.records;
    rig.server.closed(owner.slot, |r| records.push(r));

    // Gone before the writer reaches the change: it finds nothing.
    rig.device.clients.retain(|c| c.key != public(MEMBER));
    rig.device.write();
    assert_eq!(rig.device.keys_outcome(1), Some(KeyOutcome::NotFound));
    rig.pass();
    rig.pass();
    assert_eq!(revocations(&rig), []);
}

/// Another change queued and finished before the session's next pass (a
/// BLE setup's enrollment, say) does not hide the session's own result.
#[test]
fn a_change_finished_behind_another_still_answers_its_session() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    let revoke = Op::RevokeClient {
        key: public(MEMBER),
    };
    send_unwritten(&mut rig, &mut owner, 1, revoke);
    rig.device.write();

    let other = rig.device.keys(KeyChange::Revoke {
        key: [0x99; 32],
        by: proto::Source::Client([0x99; 4]),
    });
    rig.device.write();
    assert_eq!(
        rig.device.keys_outcome(other.unwrap()),
        Some(KeyOutcome::NotFound)
    );

    let answer = poll_answer(&mut rig, &mut owner, 1, 5);
    assert_eq!(answer, Some(Answer::Done { generation: None }));
    let by = proto::Source::Client(fingerprint(OWNER));
    assert_eq!(revocations(&rig), [(fingerprint(MEMBER), by)]);
}

#[test]
fn an_invite_enrolls_a_new_client_once() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    let invite = ask(
        &mut rig,
        &mut owner,
        1,
        Op::CreateInvite {
            role: model::ClientRole::Member,
        },
    );
    let Answer::Invite(invite) = invite else {
        panic!("{invite:?}");
    };
    let invite = Invite {
        feeder: public(FEEDER),
        id: invite.id,
        secret: invite.secret.0,
    };

    let joined = redeem(&mut rig, invite, STRANGER);
    assert_eq!(
        joined,
        Some(Joined::Enrolled {
            role: model::ClientRole::Member,
            // The redemption names the model, as the welcome does.
            model: model::Model::Plaf103,
        })
    );
    rig.publish();
    let _stranger = open(&mut rig, STRANGER);

    // The invite was single-use, and an open naming none still live ends
    // before the generator is drawn on, so a stranger cannot spend it.
    let drawn = rig.device.counter;
    assert_eq!(redeem(&mut rig, invite, [0x50; 32]), None);
    assert!(rig.ended(Reason::Invite));
    assert_eq!(rig.device.counter, drawn);
}

/// Revoking a client takes back the invites it made: one a member minted
/// before an owner revoked it admits nobody, though it has not expired.
#[test]
fn a_revoked_clients_invite_admits_nobody() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    let mut member = open(&mut rig, MEMBER);
    let invite = member_invite(&mut rig, &mut member, 1);

    let revoke = Op::RevokeClient {
        key: public(MEMBER),
    };
    assert_eq!(
        ask(&mut rig, &mut owner, 2, revoke),
        Answer::Done { generation: None }
    );
    rig.publish();

    assert_eq!(redeem(&mut rig, invite, STRANGER), None);
    assert!(rig.ended(Reason::Invite));
    rig.publish();
    assert!(!rig.device.clients.iter().any(|c| c.key == public(STRANGER)));
}

/// The history's enrollments: the client, its role and how it came.
fn enrollments(rig: &Rig) -> Vec<(proto::Fingerprint, proto::Role, proto::Path)> {
    rig.device
        .events
        .iter()
        .filter_map(|e| match e.body {
            EventBody::Enrolled { client, role, path } => Some((client, role, path)),
            _ => None,
        })
        .collect()
}

/// A member's invite, asked for as request `id` of the owner's session.
fn member_invite(rig: &mut Rig, owner: &mut Client_, id: u8) -> Invite {
    let invite = ask(
        rig,
        owner,
        id,
        Op::CreateInvite {
            role: model::ClientRole::Member,
        },
    );
    let Answer::Invite(invite) = invite else {
        panic!("{invite:?}");
    };
    Invite {
        feeder: public(FEEDER),
        id: invite.id,
        secret: invite.secret.0,
    }
}

/// Redeem `invite` with `secret` until the joining session has queued its
/// enrollment, then drop the connection before the writer takes it; the
/// enrollment's ticket.
fn redeem_and_drop(rig: &mut Rig, invite: Invite, secret: [u8; 32]) -> u32 {
    let slot = rig.accept().unwrap();
    let (mut join, mut outbox) =
        Join::connect(&Keypair::from_secret(secret), invite, "phone", [0x55; 32]).unwrap();
    for _ in 0..200 {
        rig.deliver(slot, &mut outbox);
        rig.pass();
        if let Some((ticket, KeyChange::Enroll { key, .. })) = rig.device.keys_pending {
            assert_eq!(key, public(secret));
            let records = &mut rig.records;
            rig.server.closed(slot, |r| records.push(r));
            return ticket;
        }
        let bytes = rig.collect(slot);
        if !bytes.is_empty()
            && let Some(JoinStep::Send(bytes)) = join.receive(&bytes).unwrap()
        {
            outbox.extend(bytes);
        }
    }
    panic!("no enrollment queued");
}

/// The connection drops while its enrollment is queued: the writer still
/// enrolls the key, so the history must still say so, once, as a join that
/// stayed would have.
#[test]
fn a_join_whose_connection_dropped_is_still_in_the_history() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    let invite = member_invite(&mut rig, &mut owner, 1);
    redeem_and_drop(&mut rig, invite, STRANGER);
    rig.pass();
    assert_eq!(enrollments(&rig), [], "not before the writer has finished");

    rig.device.write();
    rig.pass();
    rig.pass();
    assert!(rig.device.clients.iter().any(|c| c.key == public(STRANGER)));
    let enrolled = (
        fingerprint(STRANGER),
        proto::Role::Member,
        proto::Path::Invite,
    );
    assert_eq!(enrollments(&rig), [enrolled]);
}

/// A dropped join the writer refused enrolled nothing new: no event, for a
/// key already enrolled or a full table.
#[test]
fn a_dropped_join_that_was_refused_is_not_in_the_history() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    let invite = member_invite(&mut rig, &mut owner, 1);
    let ticket = redeem_and_drop(&mut rig, invite, STRANGER);
    rig.device.enroll(public(STRANGER), Role::Member);
    rig.device.write();
    assert_eq!(rig.device.keys_outcome(ticket), Some(KeyOutcome::Duplicate));
    rig.pass();
    rig.pass();
    assert_eq!(enrollments(&rig), []);

    let invite = member_invite(&mut rig, &mut owner, 2);
    let ticket = redeem_and_drop(&mut rig, invite, [0x50; 32]);
    // The fake's table never fills: finish the change as a full one would.
    rig.device.keys_pending = None;
    rig.device.keys_done.push((ticket, KeyOutcome::Full));
    rig.pass();
    rig.pass();
    assert_eq!(enrollments(&rig), []);
}

/// A join that stays to read its answer is in the history once: its end
/// leaves nothing to settle.
#[test]
fn a_join_that_completes_is_in_the_history_once() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    let invite = member_invite(&mut rig, &mut owner, 1);
    let joined = redeem(&mut rig, invite, STRANGER);
    assert!(
        matches!(joined, Some(Joined::Enrolled { .. })),
        "{joined:?}"
    );
    rig.pass();
    rig.pass();
    let enrolled = (
        fingerprint(STRANGER),
        proto::Role::Member,
        proto::Path::Invite,
    );
    assert_eq!(enrollments(&rig), [enrolled]);
}

/// Redeem `invite` with a new key; the feeder's answer, or `None` if the
/// connection was dropped.
fn redeem(rig: &mut Rig, invite: Invite, secret: [u8; 32]) -> Option<Joined> {
    let slot = rig.accept().unwrap();
    let (mut join, mut outbox) =
        Join::connect(&Keypair::from_secret(secret), invite, "phone", [0x55; 32]).unwrap();
    for _ in 0..200 {
        rig.deliver(slot, &mut outbox);
        rig.pass();
        let bytes = rig.collect(slot);
        if !bytes.is_empty() {
            match join.receive(&bytes).unwrap() {
                Some(JoinStep::Send(bytes)) => outbox.extend(bytes),
                Some(JoinStep::Done(joined)) => return Some(joined),
                None => {}
            }
        }
        rig.device.write();
        if rig.server.closing(slot) && rig.server.tx(slot).is_empty() {
            let records = &mut rig.records;
            rig.server.closed(slot, |r| records.push(r));
            return None;
        }
    }
    panic!("no progress");
}

/// Run a LAN recovery session to its last reply: the setup exchange over
/// the device API, with the owner keying `keyed` in on the panel when the
/// feeder asks. `None` when the feeder dropped the connection.
fn recover(
    rig: &mut Rig,
    secret: [u8; 32],
    code: Code,
    keyed: Code,
) -> Option<wellfed_model::setup::Reply> {
    recover_until(rig, secret, code, keyed, Leave::Never)
}

/// When a recovery client hangs up.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Leave {
    /// It stays for the last reply.
    Never,
    /// As soon as its enrollment is queued, before the writer takes it.
    OnceQueued,
}

/// [`recover`], with the client hanging up as `leave` says.
fn recover_until(
    rig: &mut Rig,
    secret: [u8; 32],
    code: Code,
    keyed: Code,
    leave: Leave,
) -> Option<wellfed_model::setup::Reply> {
    use wellfed_client::setup::{Ask, Session as Setup};
    use wellfed_model::setup::Reply;

    let slot = rig.accept().unwrap();
    let ask = Ask {
        label: "laptop".into(),
        wifi: None,
    };
    let keys = Keypair::from_secret(secret);
    let setup = Setup::new(&keys, [0x61; 32], ask, code, [0x62; 32]).unwrap();
    let mut recovery = Recovery::new(setup).unwrap();
    let mut outbox = Vec::new();

    for _ in 0..400 {
        while let Some(bytes) = recovery.poll_transmit() {
            outbox.extend(bytes);
        }
        rig.deliver(slot, &mut outbox);
        rig.pass();
        if leave == Leave::OnceQueued && rig.device.keys_pending.is_some() {
            let records = &mut rig.records;
            rig.server.closed(slot, |r| records.push(r));
            return None;
        }
        let received = rig.collect(slot);
        rig.device.write();
        match recovery.handle_input(&received) {
            Ok(()) => {}
            // A mismatched code fails at whichever side opens a press
            // first; with the feeder writing first that is the client.
            Err(Error::Setup(wellfed_client::setup::Error::WrongCode)) => {
                return Some(Reply::Refused(wellfed_model::setup::Refusal::WrongCode));
            }
            Err(error) => panic!("{error}"),
        }
        while let Some(reply) = recovery.poll_reply() {
            let Reply::EnterCode { .. } = reply else {
                return Some(reply);
            };
            // Each press goes down and comes up alone, as the button
            // service reports a short press; it counts as it comes up.
            for &button in keyed.presses() {
                let chord = Buttons::of(button);
                let up = Gesture::Up {
                    chord,
                    level: 0,
                    held_ms: 100,
                };
                for gesture in [Gesture::Down { chord }, up] {
                    let event = rig.pairing.gesture(rig.now, gesture, Lock::Unlocked);
                    if matches!(event, Some(PairingEvent::Entered { .. })) {
                        rig.recovery = event;
                    }
                }
            }
        }
        if rig.server.closing(slot) && rig.server.tx(slot).is_empty() {
            let records = &mut rig.records;
            rig.server.closed(slot, |r| records.push(r));
            return None;
        }
    }
    panic!("no progress");
}

fn open_recovery(rig: &mut Rig) {
    let hold = Gesture::Hold {
        chord: Buttons::of(Button::Lock).with(Button::Dispense),
        level: 1,
    };
    assert_eq!(
        rig.pairing.gesture(rig.now, hold, Lock::Unlocked),
        Some(PairingEvent::RecoveryOpened)
    );
}

/// A recovery client that stops reading its `EnterCode` loses the offer with
/// its slot, as one that hangs up does: the owner's next presses work the
/// panel again, rather than keying a code for no one.
#[test]
fn a_recovery_client_that_stops_reading_gives_up_its_offer() {
    use wellfed_client::setup::{Ask, Session as Setup};
    use wellfed_device_api::STALL_MS;
    use wellfed_model::setup::Reply;

    let mut rig = Rig::new();
    open_recovery(&mut rig);
    let slot = rig.accept().unwrap();
    let ask = Ask {
        label: "laptop".into(),
        wifi: None,
    };
    let keys = Keypair::from_secret(STRANGER);
    let code = Code::from_bits(0x0abc);
    let setup = Setup::new(&keys, [0x61; 32], ask, code, [0x62; 32]).unwrap();
    let mut recovery = Recovery::new(setup).unwrap();
    let mut outbox = Vec::new();

    // Up to the offer, stopping as soon as the feeder has answered it.
    while rig.pairing.awaiting().is_none() {
        while let Some(bytes) = recovery.poll_transmit() {
            outbox.extend(bytes);
        }
        rig.deliver(slot, &mut outbox);
        rig.pass();
        if rig.pairing.awaiting().is_none() {
            let received = rig.collect(slot);
            recovery.handle_input(&received).unwrap();
            assert!(!matches!(recovery.poll_reply(), Some(Reply::Done { .. })));
        }
    }
    assert!(!rig.server.tx(slot).is_empty(), "the EnterCode, unread");

    for _ in 0..STALL_MS / 20 + 2 {
        rig.pass();
    }
    assert!(rig.ended(Reason::Stalled));
    assert!(rig.server.closing(slot));
    let records = &mut rig.records;
    rig.server.closed(slot, |r| records.push(r));
    assert_eq!(rig.server.sessions(), 0);

    rig.pass();
    assert_eq!(rig.pairing.awaiting(), None, "the offer went with it");
}

#[test]
fn the_recovery_window_admits_one_owner_who_proves_the_code() {
    use wellfed_model::setup::{Enrolled, Reply, WifiSaved};

    let mut rig = Rig::new();
    // Closed until a Lock and Dispense hold opens it.
    assert_eq!(
        recover(&mut rig, STRANGER, Code::from_bits(7), Code::from_bits(7)),
        None
    );
    assert!(rig.ended(Reason::Recovery));

    open_recovery(&mut rig);
    let code = Code::from_bits(0x0abc);
    assert_eq!(
        recover(&mut rig, STRANGER, code, code),
        Some(Reply::Done {
            enrolled: Enrolled::New,
            wifi: WifiSaved::Unchanged
        })
    );
    assert!(
        rig.device
            .clients
            .iter()
            .any(|c| c.key == public(STRANGER) && c.role == Role::Owner)
    );
    // Once: the writer's event, and nothing from the session that read it.
    let enrolled = (
        fingerprint(STRANGER),
        proto::Role::Owner,
        proto::Path::Recovery,
    );
    assert_eq!(enrollments(&rig), [enrolled]);

    // The offer closed the window: the next client is turned away.
    assert_eq!(
        recover(&mut rig, [0x70; 32], code, code),
        None,
        "one owner per window"
    );
    rig.publish();
    let _owner = open(&mut rig, STRANGER);
}

/// A key already enrolled that redeems an invite joins only if the role it
/// holds covers the invited one, and is then told the role it holds: a
/// member's key redeeming an owner's invite stays a member and is told no,
/// and an owner's key redeeming a member's invite stays an owner.
#[test]
fn an_enrolled_key_joins_only_if_its_role_covers_the_invite() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    let invite = ask(
        &mut rig,
        &mut owner,
        1,
        Op::CreateInvite {
            role: model::ClientRole::Owner,
        },
    );
    let Answer::Invite(invite) = invite else {
        panic!("{invite:?}");
    };
    let invite = Invite {
        feeder: public(FEEDER),
        id: invite.id,
        secret: invite.secret.0,
    };
    assert_eq!(
        redeem(&mut rig, invite, MEMBER),
        Some(Joined::Refused(model::Refusal::Invalid))
    );
    let role_of = |rig: &Rig, secret| {
        rig.device
            .clients
            .iter()
            .find(|c| c.key == public(secret))
            .map(|c| c.role)
    };
    assert_eq!(role_of(&rig, MEMBER), Some(Role::Member));

    let invite = member_invite(&mut rig, &mut owner, 2);
    assert_eq!(
        redeem(&mut rig, invite, OWNER),
        Some(Joined::Enrolled {
            role: model::ClientRole::Owner,
            model: model::Model::Plaf103,
        })
    );
    assert_eq!(role_of(&rig, OWNER), Some(Role::Owner));
}

/// Recovery with a key already enrolled grants nothing, so it succeeds only
/// for an owner's key: a member's is refused and stays a member.
#[test]
fn recovery_with_a_members_key_is_refused_and_an_owners_is_already_done() {
    use wellfed_model::setup::{Enrolled, Refusal, Reply, WifiSaved};

    let mut rig = Rig::new();
    let code = Code::from_bits(0x0abc);
    open_recovery(&mut rig);
    assert_eq!(
        recover(&mut rig, MEMBER, code, code),
        Some(Reply::Refused(Refusal::Invalid))
    );
    assert!(
        rig.device
            .clients
            .iter()
            .any(|c| c.key == public(MEMBER) && c.role == Role::Member)
    );

    rig.publish();
    open_recovery(&mut rig);
    assert_eq!(
        recover(&mut rig, OWNER, code, code),
        Some(Reply::Done {
            enrolled: Enrolled::Already,
            wifi: WifiSaved::Unchanged
        })
    );
}

/// A recovery client that hangs up once its enrollment is queued is still
/// enrolled, and the history says so once: the writer records it with the
/// store write, not the session that has gone.
#[test]
fn a_recovery_whose_client_left_is_still_in_the_history() {
    let mut rig = Rig::new();
    open_recovery(&mut rig);
    let code = Code::from_bits(0x0abc);
    let reply = recover_until(&mut rig, STRANGER, code, code, Leave::OnceQueued);
    assert_eq!(reply, None, "gone before its answer");
    assert!(rig.ended(Reason::Closed));
    assert_eq!(enrollments(&rig), [], "not before the writer has made it");

    rig.device.write();
    rig.pass();
    rig.pass();
    assert!(
        rig.device
            .clients
            .iter()
            .any(|c| c.key == public(STRANGER) && c.role == Role::Owner)
    );
    let enrolled = (
        fingerprint(STRANGER),
        proto::Role::Owner,
        proto::Path::Recovery,
    );
    assert_eq!(enrollments(&rig), [enrolled]);
}

#[test]
fn a_wrong_code_is_refused_and_enrolls_nobody() {
    use wellfed_model::setup::{Refusal as SetupRefusal, Reply};

    let mut rig = Rig::new();
    open_recovery(&mut rig);
    let reply = recover(
        &mut rig,
        STRANGER,
        Code::from_bits(0x0abc),
        Code::from_bits(0x0abd),
    );
    assert_eq!(reply, Some(Reply::Refused(SetupRefusal::WrongCode)));
    assert!(!rig.device.clients.iter().any(|c| c.key == public(STRANGER)));
}
