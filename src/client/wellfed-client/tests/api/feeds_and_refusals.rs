//! Feeds and restarts: a feed answered once and never repeated by a retry,
//! what the status says about starting one, and a reboot that never runs
//! into a feed.

use wellfed_client::api::Incoming;
use wellfed_device_api::{FeederDevice, Record};
use wellfed_model as model;
use wellfed_model::{Answer, Op};
use wellfed_proto::api::{self as proto};

use crate::support::{MEMBER, OWNER, Rig, ask, open, public, run};

#[test]
fn a_feed_answers_once_reserved_and_a_retry_never_feeds_twice() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);

    let zero = ask(&mut rig, &mut owner, 1, Op::Feed { portions: 0 });
    assert_eq!(zero, Answer::Refused(model::Refusal::Portions { asked: 0 }));
    let feed = ask(&mut rig, &mut owner, 2, Op::Feed { portions: 3 });
    assert_eq!(feed, Answer::Feeding { feed: 1 });
    let by = proto::Source::Client(public(OWNER)[..4].try_into().unwrap());
    assert_eq!(rig.device.feeds, [(3, by)]);

    // The same id after a reconnect: the first answer, and no second feed.
    let mut owner = open(&mut rig, OWNER);
    let again = ask(&mut rig, &mut owner, 2, Op::Feed { portions: 3 });
    assert_eq!(again, Answer::Feeding { feed: 1 });
    assert_eq!(rig.device.feeds.len(), 1);

    let cancel = ask(&mut rig, &mut owner, 3, Op::CancelFeed { feed: 1 });
    assert_eq!(cancel, Answer::Done { generation: None });
    let ended = ask(&mut rig, &mut owner, 4, Op::CancelFeed { feed: 1 });
    assert_eq!(ended, Answer::Refused(model::Refusal::NotFound));
}

/// The first request queued the feed; its session ended before the owner
/// reserved it, so the client never heard. The same id on a new session
/// waits for that feed's reservation and answers with it: no second feed.
#[test]
fn a_feed_s_id_is_remembered_from_the_moment_it_is_queued() {
    let mut rig = Rig::new();
    rig.device.hold_feeds = true;
    let mut owner = open(&mut rig, OWNER);
    let bytes = owner
        .session
        .request([2; 16], &Op::Feed { portions: 2 })
        .unwrap();
    owner.outbox.extend(bytes);
    for _ in 0..5 {
        rig.deliver(owner.slot, &mut owner.outbox);
        rig.pass();
    }
    assert_eq!(rig.device.feeds.len(), 1, "queued");
    assert!(rig.server.tx(owner.slot).is_empty(), "not yet answered");
    let records = &mut rig.records;
    rig.server.closed(owner.slot, |r| records.push(r));

    let mut owner = open(&mut rig, OWNER);
    let bytes = owner
        .session
        .request([2; 16], &Op::Feed { portions: 2 })
        .unwrap();
    owner.outbox.extend(bytes);
    for _ in 0..5 {
        rig.deliver(owner.slot, &mut owner.outbox);
        rig.pass();
    }
    assert!(
        rig.server.tx(owner.slot).is_empty(),
        "the retry waits for the first feed's reservation"
    );
    assert!(
        rig.records
            .iter()
            .any(|r| matches!(r, Record::Retried { .. }))
    );

    rig.device.hold_feeds = false;
    let mut answer = None;
    run(&mut rig, &mut owner, |session| {
        if let Some(Incoming::Answer { answer: got, .. }) = session.next_incoming().unwrap() {
            answer = Some(got);
        }
        answer.is_some()
    });
    assert_eq!(answer, Some(Answer::Feeding { feed: 1 }));
    assert_eq!(rig.device.feeds.len(), 1, "fed once");

    // After the feed, the same id still names it.
    let again = ask(&mut rig, &mut owner, 2, Op::Feed { portions: 2 });
    assert_eq!(again, Answer::Feeding { feed: 1 });
    assert_eq!(rig.device.feeds.len(), 1);
}

#[test]
fn the_status_says_whether_a_feed_can_start_and_a_feed_obeys_it() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    let readiness = |answer: Answer| match answer {
        Answer::Status(status) => status.feeder().unwrap().readiness,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        readiness(ask(&mut rig, &mut owner, 1, Op::Status)),
        model::Readiness::Ready
    );

    rig.device.blocked = Some(proto::Blocked::Fault);
    assert_eq!(
        readiness(ask(&mut rig, &mut owner, 2, Op::Status)),
        model::Readiness::Blocked(model::Blocked::Fault)
    );
    let fed = ask(&mut rig, &mut owner, 3, Op::Feed { portions: 1 });
    assert_eq!(fed, Answer::Refused(model::Refusal::Fault));
    rig.device.blocked = Some(proto::Blocked::Joining);
    let fed = ask(&mut rig, &mut owner, 4, Op::Feed { portions: 1 });
    assert_eq!(fed, Answer::Refused(model::Refusal::Busy));
    assert!(rig.device.feeds.is_empty(), "nothing reached the owner");

    // The battery's reason reaches every session; a feed is refused
    // `Busy`, as it clears by itself.
    rig.device.blocked = Some(proto::Blocked::Battery);
    assert_eq!(
        readiness(ask(&mut rig, &mut owner, 5, Op::Status)),
        model::Readiness::Blocked(model::Blocked::Battery)
    );
    let fed = ask(&mut rig, &mut owner, 6, Op::Feed { portions: 1 });
    assert_eq!(fed, Answer::Refused(model::Refusal::Busy));

    assert!(rig.device.feeds.is_empty(), "nothing reached the owner");
}

#[test]
fn a_reboot_is_refused_while_a_feed_is_in_flight() {
    let mut rig = Rig::new();
    rig.device.hold_feeds = true;
    let mut owner = open(&mut rig, OWNER);
    let mut member = open(&mut rig, MEMBER);
    let bytes = member
        .session
        .request([1; 16], &Op::Feed { portions: 1 })
        .unwrap();
    member.outbox.extend(bytes);
    rig.deliver(member.slot, &mut member.outbox);
    rig.pass();
    assert_eq!(rig.device.feeds.len(), 1);

    let reboot = ask(&mut rig, &mut owner, 2, Op::Reboot);
    assert_eq!(reboot, Answer::Refused(model::Refusal::Busy));
    rig.device.hold_feeds = false;
    rig.device.write();
    let reboot = ask(&mut rig, &mut owner, 3, Op::Reboot);
    assert_eq!(reboot, Answer::Done { generation: None });
}

/// Once a restart is accepted no feed is asked for until it, and a feed
/// queued meanwhile from elsewhere (a Dispense press) holds it until the
/// feed has ended.
#[test]
fn a_feed_never_runs_into_an_accepted_reboot() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    let reboot = ask(&mut rig, &mut owner, 1, Op::Reboot);
    assert_eq!(reboot, Answer::Done { generation: None });
    let fed = ask(&mut rig, &mut owner, 2, Op::Feed { portions: 1 });
    assert_eq!(fed, Answer::Refused(model::Refusal::Busy));
    assert!(rig.device.feeds.is_empty());

    // The panel's feed, which the server never sees, is in flight at the
    // restart's deadline.
    rig.device.hold_feeds = true;
    rig.device.feed(1, proto::Source::Panel).unwrap();
    for _ in 0..(2 * wellfed_device_api::REBOOT_MS / 20) {
        rig.pass();
    }
    assert!(!rig.device.rebooted, "the restart waits for the feed");
    rig.device.hold_feeds = false;
    rig.device.write();
    rig.pass();
    assert!(rig.device.rebooted);
}

#[test]
fn reboot_and_time_sync_reach_the_device() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    let sync = model::TimeSync {
        unix_ms: 1_700_000_000_000,
        offset_s: 0,
        next_change_s: 0,
        next_offset_s: 0,
    };
    let synced = ask(&mut rig, &mut owner, 1, Op::TimeSync(sync));
    assert_eq!(synced, Answer::Done { generation: None });
    assert_eq!(rig.device.syncs, 1);

    let reboot = ask(&mut rig, &mut owner, 2, Op::Reboot);
    assert_eq!(reboot, Answer::Done { generation: None });
    rig.now += wellfed_device_api::REBOOT_MS;
    rig.pass();
    assert!(rig.device.rebooted);
}
