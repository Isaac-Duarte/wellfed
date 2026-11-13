//! Opening a session, the handshake limits (one operation a pass, only
//! enrolled keys during a feed, floods and malformed bytes), and how a
//! session ends: idle and stall timeouts, the keepalive, and requests left
//! unanswered.

use std::time::Duration;

use wellfed_client::api::{Error, Incoming, Invite, Join, Session};
use wellfed_client::connection::{Event, KEEPALIVE_ID, Timing};
use wellfed_device_api::clients::Role;
use wellfed_device_api::{Reason, Record, SESSIONS};
use wellfed_model as model;
use wellfed_model::{Answer, Generation, Op, OpenRefusal};
use wellfed_session::frame;
use wellfed_session::noise::Keypair;

use crate::support::{
    EPOCH, FEEDER, MEMBER, OWNER, Rig, STRANGER, ask, connect, drive, live, open, public,
    receiving, send_unwritten, view,
};

#[test]
fn an_owner_opens_a_session_and_reads_status_and_settings() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    assert!(rig.records.iter().any(|r| matches!(
        r,
        Record::Opened {
            role: Role::Owner,
            ..
        }
    )));

    let status = ask(&mut rig, &mut owner, 1, Op::Status);
    let Answer::Status(status) = status else {
        panic!("{status:?}");
    };
    assert_eq!(
        status.feeder().unwrap().settings_generation,
        Some(Generation(7))
    );
    assert_eq!(status.sessions, 1);

    let settings = ask(&mut rig, &mut owner, 2, Op::GetSettings);
    assert!(matches!(
        settings,
        Answer::Settings(model::Settings::Feeder(model::FeederSettings {
            generation: Some(Generation(7)),
            volume: 40,
            ..
        }))
    ));
}

#[test]
fn an_unknown_key_and_the_wrong_feeder_are_refused() {
    let mut rig = Rig::new();
    let (_, welcome) = connect(&mut rig, STRANGER);
    assert_eq!(welcome, Some(Incoming::Refused(OpenRefusal::Unknown)));
    assert!(rig.ended(Reason::Unknown));

    // A client holding another feeder's key: the feeder cannot read its
    // first message and drops it.
    let mut rig = Rig::new();
    let slot = rig.accept().unwrap();
    let (_, mut outbox) =
        Session::connect(&Keypair::from_secret(OWNER), public(STRANGER), [1; 32]).unwrap();
    for _ in 0..5 {
        rig.deliver(slot, &mut outbox);
        rig.pass();
    }
    assert!(rig.ended(Reason::Noise));
    assert!(rig.server.closing(slot));
}

/// During a feed a stranger's handshake is refused without its reply, and
/// holds every other handshake until the feed ends; an enrolled client
/// that connected first still opens a session and cancels the feed.
#[test]
fn during_a_feed_a_client_cancels_and_a_stranger_costs_one_handshake() {
    let mut rig = Rig::new();
    let mut member = open(&mut rig, MEMBER);
    let feed = ask(&mut rig, &mut member, 1, Op::Feed { portions: 3 });
    assert_eq!(feed, Answer::Feeding { feed: 1 });

    rig.feeding = true;
    let mut owner = open(&mut rig, OWNER);
    let cancel = ask(&mut rig, &mut owner, 2, Op::CancelFeed { feed: 1 });
    assert_eq!(cancel, Answer::Done { generation: None });
    assert_eq!(rig.device.cancelled, [1]);

    let stranger = rig.accept().unwrap();
    let (_, mut outbox) =
        Session::connect(&Keypair::from_secret(STRANGER), public(FEEDER), [9; 32]).unwrap();
    rig.deliver(stranger, &mut outbox);
    for _ in 0..10 {
        rig.pass();
    }
    assert!(rig.ended(Reason::Unknown));
    assert!(rig.server.closing(stranger), "closed, ...");
    assert!(rig.server.tx(stranger).is_empty(), "... without the reply");
    let records = &mut rig.records;
    rig.server.closed(stranger, |r| records.push(r));

    // The next connection waits for the feed's end, then opens.
    let late = rig.accept().unwrap();
    let (mut session, mut outbox) =
        Session::connect(&Keypair::from_secret(OWNER), public(FEEDER), [8; 32]).unwrap();
    rig.deliver(late, &mut outbox);
    for _ in 0..10 {
        rig.pass();
        assert!(!rig.server.handshook(), "held for the feed");
    }
    rig.feeding = false;
    let mut opened = false;
    for _ in 0..10 {
        rig.pass();
        let bytes = rig.collect(late);
        if !bytes.is_empty() {
            session.receive(&bytes).unwrap();
        }
        opened |= matches!(session.next_incoming(), Ok(Some(Incoming::Opened(_))));
    }
    assert!(opened);
}

#[test]
fn one_handshake_operation_per_pass_and_only_enrolled_ones_while_feeding() {
    let mut rig = Rig::new();
    let mut clients: Vec<(usize, Vec<u8>)> = (0..SESSIONS - 1)
        .map(|_| {
            let slot = rig.accept().unwrap();
            let (_, outbox) =
                Session::connect(&Keypair::from_secret(OWNER), public(FEEDER), [9; 32]).unwrap();
            (slot, outbox)
        })
        .collect();
    // An invite's connection: its join would enroll a key, a store write
    // that waits out a feed, so its handshake waits for the feed's end.
    let invite = Invite {
        feeder: public(FEEDER),
        id: 77,
        secret: [3; 32],
    };
    let invited = rig.accept().unwrap();
    let (_, mut invite_outbox) =
        Join::connect(&Keypair::from_secret(STRANGER), invite, "phone", [0x55; 32]).unwrap();
    assert_eq!(rig.accept(), None, "a fifth connection is refused");
    assert!(rig.ended(Reason::Full));

    for (slot, outbox) in &mut clients {
        rig.deliver(*slot, outbox);
    }
    rig.deliver(invited, &mut invite_outbox);
    // Each slot needs its open frame read, then three operations: the
    // handshake built, the first message read, the answer written. A feed
    // holds the outputs meanwhile: the enrolled clients still open, one
    // operation a pass, and the loop is told after each.
    rig.pass();
    rig.feeding = true;
    let mut answered = 0;
    for pass in 1..=(3 * (SESSIONS - 1)) {
        rig.pass();
        assert!(rig.server.handshook(), "pass {pass} ran an operation");
        let now = clients
            .iter()
            .filter(|(slot, _)| !rig.server.tx(*slot).is_empty())
            .count();
        assert!(now <= pass / 3 + 1);
        answered = now;
    }
    assert_eq!(answered, SESSIONS - 1);
    rig.pass();
    assert!(!rig.server.handshook(), "the invite waits for the feed");
    assert!(rig.server.tx(invited).is_empty());

    rig.feeding = false;
    rig.pass();
    assert!(rig.server.handshook(), "the invite goes on once it ends");
}

#[test]
fn a_handshake_waiting_on_its_client_is_not_busy() {
    let mut rig = Rig::new();
    let slot = rig.accept().unwrap();
    let (_, mut outbox) =
        Session::connect(&Keypair::from_secret(OWNER), public(FEEDER), [9; 32]).unwrap();
    let open = frame::PREFIX + frame::parse_prefix([outbox[0], outbox[1]], usize::MAX).unwrap();
    let mut first = outbox.drain(..open).collect();

    // The open frame read and the handshake built are work; then the
    // feeder waits for the client's first message, which is not.
    rig.deliver(slot, &mut first);
    assert!(rig.pass(), "the open frame");
    assert!(rig.pass(), "the handshake built");
    assert!(!rig.pass());

    rig.deliver(slot, &mut outbox);
    assert!(rig.pass(), "the client's message is work again");
}

#[test]
fn malformed_bytes_and_floods_cost_only_their_own_session() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);

    // A frame announcing more than the limit.
    let bad = rig.accept().unwrap();
    rig.deliver(bad, &mut vec![0xff, 0xff, 1, 2, 3]);
    rig.pass();
    assert!(rig.ended(Reason::Malformed));

    // Garbage after a valid open frame never finishes a handshake; another
    // client floods requests. The owner still gets answers.
    let flood = open(&mut rig, MEMBER);
    let mut flood = flood;
    for id in 0..50u8 {
        let bytes = flood.session.request([id; 16], &Op::Status).unwrap();
        flood.outbox.extend(bytes);
    }
    for _ in 0..3 {
        rig.deliver(flood.slot, &mut flood.outbox);
        rig.pass();
        let _ = rig.collect(flood.slot);
    }
    let status = ask(&mut rig, &mut owner, 1, Op::Status);
    assert!(matches!(status, Answer::Status(_)));
}

#[test]
fn an_idle_session_and_a_stalled_handshake_time_out() {
    let mut rig = Rig::new();
    let owner = open(&mut rig, OWNER);
    let stalled = rig.accept().unwrap();

    rig.now += wellfed_device_api::HANDSHAKE_MS;
    rig.pass();
    assert!(rig.server.closing(stalled));
    rig.now += wellfed_device_api::IDLE_MS;
    rig.pass();
    assert!(rig.server.closing(owner.slot));
}

#[test]
fn a_connection_names_the_epoch_and_answers_each_request_once() {
    let mut rig = Rig::new();
    let mut owner = live(&mut rig, OWNER);
    let opened = owner.connection.opened().unwrap().clone();
    assert_eq!(
        (opened.role, opened.kind, opened.model, opened.epoch),
        (
            model::ClientRole::Owner,
            model::DeviceKind::Feeder,
            model::Model::Plaf103,
            EPOCH
        )
    );

    let now = owner.now(&rig);
    owner
        .connection
        .request(now, [1; 16], &model::Op::Status)
        .unwrap();
    owner
        .connection
        .request(now, [2; 16], &model::Op::GetSettings)
        .unwrap();
    drive(&mut rig, &mut owner, |live| live.events.len() == 2).unwrap();
    let answered: Vec<_> = owner
        .events
        .iter()
        .map(|event| match event {
            Event::Answer { id, answer } => (id[0], matches!(answer, model::Answer::Status(_))),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(answered, [(1, true), (2, false)]);
    assert!(matches!(
        owner
            .connection
            .request(now, KEEPALIVE_ID, &model::Op::Status),
        Err(Error::ReservedId)
    ));
}

#[test]
fn an_unanswered_request_is_reported_once_and_its_late_answer_dropped() {
    let mut rig = Rig::new();
    let mut owner = live(&mut rig, OWNER);
    let sent = owner.now(&rig);
    owner
        .connection
        .request(sent, [3; 16], &model::Op::Status)
        .unwrap();

    // The answer time passes before the bytes reach the feeder.
    let overdue = sent + Timing::DEFAULT.answer;
    assert!(owner.connection.poll_timeout().unwrap() <= overdue);
    owner.connection.handle_timeout(overdue).unwrap();
    assert_eq!(
        owner.connection.poll_event(),
        Some(Event::Unanswered { id: [3; 16] })
    );

    let mut passes = 0;
    drive(&mut rig, &mut owner, |_| {
        passes += 1;
        passes == 20
    })
    .unwrap();
    assert!(owner.events.is_empty(), "{:?}", owner.events);
}

#[test]
fn the_keepalive_goes_out_when_quiet_and_silence_ends_the_session() {
    let mut rig = Rig::new();
    let mut owner = live(&mut rig, OWNER);
    while owner.connection.poll_transmit().is_some() {}
    let heard = owner.now(&rig);

    let due = owner.connection.poll_timeout().unwrap();
    assert!(due <= heard + Timing::DEFAULT.keepalive, "{due:?}");
    owner.connection.handle_timeout(due).unwrap();
    let keepalive = owner.connection.poll_transmit().expect("a keepalive");
    owner.outbox.extend(keepalive);
    let mut passes = 0;
    drive(&mut rig, &mut owner, |_| {
        passes += 1;
        passes == 20
    })
    .unwrap();
    assert!(
        owner.events.is_empty(),
        "the keepalive's answer is swallowed"
    );

    // Nothing heard for the silence limit: the feeder is gone.
    let silent = owner.now(&rig) + Timing::DEFAULT.silence;
    assert!(matches!(
        owner.connection.handle_timeout(silent),
        Err(Error::Silent)
    ));
    assert!(matches!(
        owner
            .connection
            .request(silent, [4; 16], &model::Op::Status),
        Err(Error::Silent)
    ));
    assert_eq!(owner.connection.poll_timeout(), None);
}

#[test]
fn a_shorter_silence_ends_the_session_sooner_and_the_usual_one_comes_back() {
    let mut rig = Rig::new();
    let mut owner = live(&mut rig, OWNER);
    while owner.connection.poll_transmit().is_some() {}
    let short = Duration::from_secs(20);
    let heard = owner.now(&rig);

    owner.connection.set_silence(short);
    let due = owner.connection.poll_timeout().unwrap();
    assert!(due <= heard + short, "{due:?}");

    // Back to the usual limit, 20 s of silence is nothing.
    owner.connection.set_silence(Timing::DEFAULT.silence);
    owner.connection.handle_timeout(heard + short).unwrap();

    owner.connection.set_silence(short);
    assert!(matches!(
        owner.connection.handle_timeout(heard + short),
        Err(Error::Silent)
    ));
}

#[test]
fn a_request_pending_when_the_session_ends_is_reported_unanswered() {
    let mut rig = Rig::new();
    let mut owner = live(&mut rig, OWNER);
    let sent = owner.now(&rig);
    owner
        .connection
        .request(sent, [5; 16], &model::Op::Status)
        .unwrap();

    // The feeder goes silent before the bytes reach it.
    let silent = sent + Timing::DEFAULT.silence;
    assert!(matches!(
        owner.connection.handle_timeout(silent),
        Err(Error::Silent)
    ));
    assert_eq!(
        owner.connection.poll_event(),
        Some(Event::Unanswered { id: [5; 16] })
    );
    assert_eq!(owner.connection.poll_event(), None);
}

/// The slot each `Ended` record with `reason` names, in order.
fn ended_slots(rig: &Rig, reason: Reason) -> Vec<u8> {
    rig.records
        .iter()
        .filter_map(|r| match r {
            Record::Ended { slot, reason: got } if *got == reason => Some(*slot),
            _ => None,
        })
        .collect()
}

/// A feeder with every slot taken lets a new client in by ending the
/// session that has gone longest without a frame, and that alone.
#[test]
fn a_full_feeder_ends_its_longest_idle_session_for_a_new_client() {
    let mut rig = Rig::new();
    let mut sessions: Vec<_> = (0..SESSIONS).map(|_| open(&mut rig, OWNER)).collect();
    // The first opened, but it has spoken since: the second is idlest.
    let status = ask(&mut rig, &mut sessions[0], 1, Op::Status);
    assert!(matches!(status, Answer::Status(_)), "{status:?}");
    assert!(ended_slots(&rig, Reason::Evicted).is_empty());

    let newcomer = open(&mut rig, MEMBER);
    assert_eq!(newcomer.slot, sessions[1].slot);
    let evicted = u8::try_from(sessions[1].slot).unwrap();
    assert_eq!(ended_slots(&rig, Reason::Evicted), vec![evicted]);
    assert!(!rig.ended(Reason::Full));

    for (index, session) in sessions.iter_mut().enumerate() {
        if index == 1 {
            continue;
        }
        let status = ask(&mut rig, session, 2, Op::Status);
        assert!(matches!(status, Answer::Status(_)), "session {index}");
    }
}

/// An eviction passes over a session with a write under way, one whose
/// feed holds the outputs and one an install answered from; with only
/// those left, a new connection is closed as before.
#[test]
fn an_eviction_passes_over_sessions_a_write_a_feed_or_an_install_holds() {
    let mut rig = Rig::new();
    rig.device.update = true;
    let mut sessions: Vec<_> = (0..SESSIONS).map(|_| open(&mut rig, OWNER)).collect();

    rig.feeding = true;
    let feed = ask(&mut rig, &mut sessions[0], 1, Op::Feed { portions: 1 });
    assert!(matches!(feed, Answer::Feeding { .. }), "{feed:?}");
    let install = ask(&mut rig, &mut sessions[1], 2, Op::UpdateState);
    assert_eq!(install, view(receiving()));
    let put = Op::PutSettings {
        base: Generation(7),
        changes: model::FeederSettingsPatch {
            volume: Some(50),
            ..model::FeederSettingsPatch::default()
        },
    };
    send_unwritten(&mut rig, &mut sessions[2], 3, put);
    assert!(rig.device.settings_pending.is_some(), "the write is queued");

    // The fourth went quiet before the three: it alone may go.
    let first = rig.accept();
    assert_eq!(first, Some(sessions[3].slot));
    assert_eq!(rig.accept(), None, "the rest are busy, and so is a new one");
    assert!(rig.ended(Reason::Full));

    // Once the feed has let the outputs go, its session is idle again.
    rig.feeding = false;
    rig.pass();
    assert_eq!(rig.accept(), Some(sessions[0].slot));
    let evicted = [sessions[3].slot, sessions[0].slot].map(|slot| u8::try_from(slot).unwrap());
    assert_eq!(ended_slots(&rig, Reason::Evicted), evicted);
}

/// A session whose write's answer is sealed but not yet all sent keeps
/// its slot even as the quietest: ending it would leave its client unsure
/// whether the write ran.
#[test]
fn an_eviction_passes_over_a_session_whose_answer_is_still_going_out() {
    let mut rig = Rig::new();
    let mut sessions: Vec<_> = (0..SESSIONS).map(|_| open(&mut rig, OWNER)).collect();

    // The write is answered, and its socket has taken none of the answer.
    let put = Op::PutSettings {
        base: Generation(7),
        changes: model::FeederSettingsPatch {
            volume: Some(50),
            ..model::FeederSettingsPatch::default()
        },
    };
    let writer = &mut sessions[0];
    let request = writer.session.request([1; 16], &put).unwrap();
    writer.outbox.extend(request);
    rig.deliver(writer.slot, &mut writer.outbox);
    for _ in 0..5 {
        rig.pass();
        rig.device.write();
    }
    assert!(
        !rig.server.tx(writer.slot).is_empty(),
        "the answer waits to go out"
    );

    // The others speak after it, so it is the quietest.
    for (index, session) in sessions.iter_mut().enumerate().skip(1) {
        let status = ask(&mut rig, session, 2, Op::Status);
        assert!(matches!(status, Answer::Status(_)), "session {index}");
    }
    assert_eq!(rig.accept(), Some(sessions[1].slot));

    let writer = &mut sessions[0];
    let answer = rig.collect(writer.slot);
    writer.session.receive(&answer).unwrap();
    let mut answered = None;
    while let Some(incoming) = writer.session.next_incoming().unwrap() {
        if let Incoming::Answer { id, answer } = incoming
            && id == [1; 16]
        {
            answered = Some(answer);
        }
    }
    assert_eq!(
        answered,
        Some(Answer::Done {
            generation: Some(Generation(8))
        })
    );
}
