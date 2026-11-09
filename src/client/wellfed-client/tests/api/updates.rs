//! Over-the-air updates through the API: who may update, how offers,
//! chunks and statements wait for feeds, and update keys that enroll only
//! once Dispense is pressed.

use wellfed_core::buttons::{Button, Buttons, Gesture};
use wellfed_device_api::clients::Role;
use wellfed_device_api::mailbox::Lock;
use wellfed_device_api::pairing::{Config, Event as PairingEvent};
use wellfed_device_api::{Reason, Record, UpdateKeyChange};
use wellfed_model as model;
use wellfed_model::{Answer, FeederSettingsPatch, Generation, Op};
use wellfed_proto::api::{self as proto, EventBody};

use crate::support::{
    Client_, MEMBER, OWNER, Rig, STRANGER, ask, open, poll_answer, public, receiving, view,
};

/// A rig whose feeder serves updates, with a server key enrolled too.
fn update_rig() -> Rig {
    let mut rig = Rig::new();
    rig.device.update = true;
    rig.device.enroll(public(STRANGER), Role::Server);
    rig.publish();
    rig
}

const MANIFEST: [u8; 160] = [0xa5; 160];

fn offer() -> Op {
    Op::OfferUpdate {
        manifest: MANIFEST.to_vec(),
        downgrade: None,
    }
}

fn enroll_laptop() -> Op {
    Op::EnrollUpdateKey {
        key: [0x5e; 32],
        label: "laptop".into(),
    }
}

#[test]
fn only_an_owner_may_update() {
    let mut rig = update_rig();
    let ops = [
        offer(),
        Op::UpdateChunk {
            offset: 0,
            data: vec![1; 512],
        },
        Op::UpdateState,
        Op::CancelUpdate,
        Op::EnrollUpdateKey {
            key: [3; 32],
            label: "laptop".into(),
        },
        Op::UpdateKeyStatement {
            statement: vec![4; 88],
        },
    ];
    for secret in [MEMBER, STRANGER] {
        let mut client = open(&mut rig, secret);
        for (n, op) in ops.iter().enumerate() {
            let answer = ask(
                &mut rig,
                &mut client,
                u8::try_from(n).unwrap() + 1,
                op.clone(),
            );
            assert_eq!(answer, Answer::Refused(model::Refusal::Forbidden), "{op:?}");
        }
    }
    assert!(rig.device.updates.is_empty() && rig.device.chunks.is_empty());

    // A feeder that installs nothing refuses the installer's ops. An update
    // key is its key store's, so it still waits for the press.
    let mut plain = Rig::new();
    let mut owner = open(&mut plain, OWNER);
    for (n, op) in ops.iter().enumerate() {
        if matches!(op, Op::EnrollUpdateKey { .. }) {
            continue;
        }
        let id = u8::try_from(n).unwrap() + 1;
        let unavailable = Answer::Refused(model::Refusal::Unavailable);
        assert_eq!(
            ask(&mut plain, &mut owner, id, op.clone()),
            unavailable,
            "{op:?}"
        );
    }
    assert!(plain.device.updates.is_empty());
}

#[test]
fn an_offer_waits_for_no_feed_and_a_feed_waits_for_the_install() {
    let mut rig = update_rig();
    let mut owner = open(&mut rig, OWNER);
    let busy_refusal = Answer::Refused(model::Refusal::Busy);

    // A feed is prepared or driven: the offer is refused, not queued.
    rig.feeding = true;
    assert_eq!(ask(&mut rig, &mut owner, 1, offer()), busy_refusal);
    rig.feeding = false;

    // A feed queued but not yet reserved holds the offer off too.
    rig.device.feeds.push((1, proto::Source::Panel));
    assert_eq!(ask(&mut rig, &mut owner, 2, offer()), busy_refusal);
    rig.device.reserved = 1;
    assert!(rig.device.updates.is_empty());

    assert_eq!(ask(&mut rig, &mut owner, 3, offer()), view(receiving()));
    assert_eq!(rig.device.updates.len(), 1);
    assert!(rig.device.installing);

    let feed = ask(&mut rig, &mut owner, 4, Op::Feed { portions: 1 });
    assert_eq!(feed, busy_refusal);
    assert_eq!(rig.device.feeds.len(), 1, "no feed during the install");

    // The same offer id again: the state now, not a second offer.
    assert_eq!(ask(&mut rig, &mut owner, 3, offer()), view(receiving()));
    assert_eq!(rig.device.updates.len(), 1);

    let cancel = ask(&mut rig, &mut owner, 5, Op::CancelUpdate);
    assert!(matches!(
        cancel,
        Answer::Update(model::UpdateView {
            phase: model::UpdatePhase::Idle,
            ..
        })
    ));
    let feed = ask(&mut rig, &mut owner, 6, Op::Feed { portions: 1 });
    assert_eq!(feed, Answer::Feeding { feed: 2 });
}

#[test]
fn a_statement_or_a_cancel_waits_for_no_feed_and_a_feed_waits_for_either() {
    let mut rig = update_rig();
    let mut owner = open(&mut rig, OWNER);
    let mut other = open(&mut rig, OWNER);
    let busy = Answer::Refused(model::Refusal::Busy);
    let statement = Op::UpdateKeyStatement {
        statement: vec![4; 88],
    };

    // Both may write the update store: never while a feed is driven or
    // queued.
    rig.feeding = true;
    assert_eq!(ask(&mut rig, &mut owner, 1, statement.clone()), busy);
    assert_eq!(ask(&mut rig, &mut owner, 2, Op::CancelUpdate), busy);
    rig.feeding = false;
    rig.device.feeds.push((1, proto::Source::Panel));
    assert_eq!(ask(&mut rig, &mut owner, 3, statement.clone()), busy);
    rig.device.feeds.push((1, proto::Source::Panel));
    assert_eq!(ask(&mut rig, &mut owner, 4, Op::CancelUpdate), busy);
    assert!(rig.device.updates.is_empty());

    // A statement waiting for the update stage holds the flash: no feed
    // until it is answered.
    rig.device.hold = true;
    assert_eq!(ask_within(&mut rig, &mut owner, 5, statement, 5), None);
    let feed = ask(&mut rig, &mut other, 6, Op::Feed { portions: 1 });
    assert_eq!(feed, busy);
    assert_eq!(rig.device.feeds.len(), 2, "no feed while it waits");

    rig.device.hold = false;
    rig.device.answer_held();
    let done = poll_answer(&mut rig, &mut owner, 5, 50).unwrap();
    assert_eq!(done, Answer::Done { generation: None });
    let feed = ask(&mut rig, &mut other, 7, Op::Feed { portions: 1 });
    assert_eq!(feed, Answer::Feeding { feed: 3 });
}

#[test]
fn a_session_key_is_never_enrolled_as_an_update_signer() {
    let mut rig = update_rig();
    let mut owner = open(&mut rig, OWNER);
    let enroll = Op::EnrollUpdateKey {
        key: public(MEMBER),
        label: "phone".into(),
    };
    assert_eq!(ask_within(&mut rig, &mut owner, 1, enroll, 50), None);

    rig.signer = press_dispense(&mut rig);
    let refused = poll_answer(&mut rig, &mut owner, 1, 50).unwrap();
    assert_eq!(
        refused,
        Answer::Refused(model::Refusal::Update(
            model::UpdateRefusal::HeldAsSessionKey
        ))
    );
    let member = rig.device.clients.iter().find(|c| c.key == public(MEMBER));
    assert_eq!(member.map(|c| c.role), Some(Role::Member));
}

#[test]
fn chunks_are_answered_with_the_state_and_never_push_out_a_remembered_write() {
    let mut rig = update_rig();
    let mut owner = open(&mut rig, OWNER);
    let put = Op::PutSettings {
        base: Generation(7),
        changes: FeederSettingsPatch {
            volume: Some(50),
            ..FeederSettingsPatch::default()
        },
    };
    let first = ask(&mut rig, &mut owner, 200, put.clone());
    assert_eq!(
        first,
        Answer::Done {
            generation: Some(Generation(8))
        }
    );

    let not_yet = ask(
        &mut rig,
        &mut owner,
        1,
        Op::UpdateChunk {
            offset: 0,
            data: vec![1; 512],
        },
    );
    assert_eq!(
        not_yet,
        Answer::Refused(model::Refusal::Update(model::UpdateRefusal::NotInstalling))
    );
    ask(&mut rig, &mut owner, 2, offer());

    // More chunks than the server remembers writes.
    for n in 0..40u8 {
        let offset = u32::from(n) * 512;
        let answer = ask(
            &mut rig,
            &mut owner,
            n + 10,
            Op::UpdateChunk {
                offset,
                data: vec![n; 512],
            },
        );
        assert_eq!(answer, view(receiving()));
    }
    assert_eq!(rig.device.chunks.len(), 40);
    let oversized = ask(
        &mut rig,
        &mut owner,
        60,
        Op::UpdateChunk {
            offset: 0,
            data: vec![0; 513],
        },
    );
    assert_eq!(oversized, Answer::Refused(model::Refusal::Invalid));

    let retry = ask(&mut rig, &mut owner, 200, put);
    assert_eq!(retry, first, "the settings write is still remembered");
    assert_eq!(rig.device.settings.generation, 8);
    let logged = rig
        .records
        .iter()
        .filter(|r| {
            matches!(
                r,
                Record::Request {
                    op: "update-chunk",
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        logged, 1,
        "only the refused chunk is logged, not every one taken"
    );
}

/// A client sends a chunk refused `Busy` again at once, so each would be a
/// line on the UART: about two hundred for one meal call.
#[test]
fn a_chunk_refused_busy_logs_no_line_and_any_other_refusal_does() {
    let mut rig = update_rig();
    rig.device.call = true;
    rig.publish();
    let mut owner = open(&mut rig, OWNER);
    let chunk_lines = |rig: &Rig, op_name: &str| {
        rig.records
            .iter()
            .filter(|r| matches!(r, Record::Request { op, .. } if *op == op_name))
            .count()
    };
    let update = |offset| Op::UpdateChunk {
        offset,
        data: vec![0; 512],
    };
    let call = |offset| Op::CallChunk {
        offset,
        data: vec![0; 512],
    };
    let busy = Answer::Refused(model::Refusal::Busy);

    rig.device.chunks_busy = true;
    assert_eq!(ask(&mut rig, &mut owner, 1, update(0)), busy);
    assert_eq!(ask(&mut rig, &mut owner, 2, call(0)), busy);
    assert_eq!(chunk_lines(&rig, "update-chunk"), 0);
    assert_eq!(chunk_lines(&rig, "call-chunk"), 0);

    rig.device.chunks_busy = false;
    let not_installing = model::Refusal::Update(model::UpdateRefusal::NotInstalling);
    assert_eq!(
        ask(&mut rig, &mut owner, 3, update(0)),
        Answer::Refused(not_installing)
    );
    assert_eq!(
        ask(&mut rig, &mut owner, 4, call(0)),
        Answer::Refused(model::Refusal::Invalid)
    );
    assert_eq!(chunk_lines(&rig, "update-chunk"), 1);
    assert_eq!(chunk_lines(&rig, "call-chunk"), 1);
}

#[test]
fn a_key_statement_is_answered_done_and_logged_by_its_key_id() {
    let mut rig = update_rig();
    let mut owner = open(&mut rig, OWNER);
    let mut statement = [0u8; 88];
    statement[..8].copy_from_slice(&[0xc1; 8]);
    let done = ask(
        &mut rig,
        &mut owner,
        1,
        Op::UpdateKeyStatement {
            statement: statement.to_vec(),
        },
    );
    assert_eq!(done, Answer::Done { generation: None });
    assert!(rig.records.iter().any(|r| matches!(
        r,
        Record::UpdateKey {
            change: UpdateKeyChange::Retired,
            key,
            ..
        } if *key == [0xc1; 8]
    )));
}

/// Send a request and run `passes` passes; its answer, if one came.
fn ask_within(
    rig: &mut Rig,
    client: &mut Client_,
    id: u8,
    op: Op,
    passes: usize,
) -> Option<Answer> {
    let bytes = client.session.request([id; 16], &op).unwrap();
    client.outbox.extend(bytes);
    poll_answer(rig, client, id, passes)
}

fn press_dispense(rig: &mut Rig) -> Option<PairingEvent> {
    let press = Gesture::Down {
        chord: Buttons::of(Button::Dispense),
    };
    rig.pairing.gesture(rig.now, press, Lock::Unlocked)
}

/// An update key waits up to a minute for its press. The owner's session stays
/// open past the stall limit, since nothing is queued for it, and the loop does
/// not spin for it.
#[test]
fn a_dispense_wait_rests_the_loop_and_keeps_its_session() {
    let mut rig = update_rig();
    let mut owner = open(&mut rig, OWNER);
    assert_eq!(
        ask_within(&mut rig, &mut owner, 1, enroll_laptop(), 5),
        None
    );

    for _ in 0..wellfed_device_api::STALL_MS / 20 + 2 {
        assert!(!rig.pass(), "waiting for a press is not work");
    }
    assert!(!rig.ended(Reason::Stalled));

    rig.signer = press_dispense(&mut rig);
    let done = poll_answer(&mut rig, &mut owner, 1, 50).unwrap();
    assert_eq!(done, Answer::Done { generation: None });
}

#[test]
fn an_update_key_enrolls_only_once_dispense_is_pressed() {
    let mut rig = update_rig();
    let mut owner = open(&mut rig, OWNER);
    assert_eq!(
        ask_within(&mut rig, &mut owner, 1, enroll_laptop(), 50),
        None
    );
    assert!(!rig.device.clients.iter().any(|c| c.key == [0x5e; 32]));

    // A second offer while the first waits for its press.
    let mut other = open(&mut rig, OWNER);
    let busy = ask(&mut rig, &mut other, 2, enroll_laptop());
    assert_eq!(busy, Answer::Refused(model::Refusal::Busy));

    rig.signer = press_dispense(&mut rig);
    assert!(matches!(rig.signer, Some(PairingEvent::Admit { .. })));
    let done = poll_answer(&mut rig, &mut owner, 1, 50).unwrap();
    assert_eq!(done, Answer::Done { generation: None });
    let enrolled = rig.device.clients.iter().find(|c| c.key == [0x5e; 32]);
    assert_eq!(enrolled.map(|c| c.role), Some(Role::UpdateSigner));
    assert!(rig.records.iter().any(|r| matches!(
        r,
        Record::UpdateKey {
            change: UpdateKeyChange::Enrolled,
            ..
        }
    )));

    // The enrolment is in the history, with what confirmed it.
    let history = |rig: &Rig| {
        rig.device
            .events
            .iter()
            .filter(|e| {
                matches!(
                    e.body,
                    EventBody::Enrolled {
                        role: proto::Role::UpdateSigner,
                        path: proto::Path::Press,
                        ..
                    }
                )
            })
            .count()
    };
    assert_eq!(history(&rig), 1);

    // Offered and pressed again, the key is already there: nothing changed.
    assert_eq!(
        ask_within(&mut rig, &mut owner, 3, enroll_laptop(), 5),
        None
    );
    rig.signer = press_dispense(&mut rig);
    let again = poll_answer(&mut rig, &mut owner, 3, 50).unwrap();
    assert_eq!(again, Answer::Done { generation: None });
    assert_eq!(history(&rig), 1, "no event for a key already enrolled");
}

/// The session goes after the press, with the key queued: the writer still
/// enrolls it, so the history must still say so.
#[test]
fn a_pressed_key_whose_session_left_is_still_in_the_history() {
    let mut rig = update_rig();
    let mut owner = open(&mut rig, OWNER);
    assert_eq!(
        ask_within(&mut rig, &mut owner, 1, enroll_laptop(), 5),
        None
    );
    rig.signer = press_dispense(&mut rig);
    rig.pass();
    assert!(rig.device.keys_pending.is_some(), "the key is queued");

    let records = &mut rig.records;
    rig.server.closed(owner.slot, |r| records.push(r));
    rig.pass();
    let enrolled = |rig: &Rig| {
        rig.device
            .events
            .iter()
            .filter(|e| {
                matches!(
                    e.body,
                    EventBody::Enrolled {
                        path: proto::Path::Press,
                        ..
                    }
                )
            })
            .count()
    };
    assert_eq!(enrolled(&rig), 0, "not before the writer has finished");

    rig.device.write();
    rig.pass();
    rig.pass();
    assert!(rig.device.clients.iter().any(|c| c.key == [0x5e; 32]));
    assert_eq!(enrolled(&rig), 1);
}

#[test]
fn an_unpressed_update_key_is_refused_and_a_closed_session_cancels_its_offer() {
    let mut rig = update_rig();
    let mut owner = open(&mut rig, OWNER);
    assert_eq!(
        ask_within(&mut rig, &mut owner, 1, enroll_laptop(), 5),
        None
    );
    rig.now += Config::DEFAULT.confirm_ms;
    rig.signer = rig.pairing.tick(rig.now);
    let refused = poll_answer(&mut rig, &mut owner, 1, 50).unwrap();
    assert_eq!(
        refused,
        Answer::Refused(model::Refusal::Update(model::UpdateRefusal::Unconfirmed))
    );

    // The client leaves while its offer waits: the next press feeds again,
    // and a leftover decision never admits a later offer.
    assert_eq!(
        ask_within(&mut rig, &mut owner, 2, enroll_laptop(), 5),
        None
    );
    let records = &mut rig.records;
    rig.server.closed(owner.slot, |r| records.push(r));
    rig.pass();
    assert_eq!(
        press_dispense(&mut rig),
        None,
        "no offer is pending any more"
    );
    assert!(!rig.device.clients.iter().any(|c| c.key == [0x5e; 32]));
}
