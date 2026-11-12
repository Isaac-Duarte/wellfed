//! Reading across revisions: a major 2 hello's refusal, a newer build's
//! event read as unknown, and every event body reaching every subscriber.

use wellfed_client::api::Error;
use wellfed_client::connection::{Cursor, Event};
use wellfed_device_api::Device;
use wellfed_model as model;
use wellfed_model::{Hello, OpenRefusal};
use wellfed_proto::api::{self as proto, EventBody, FoodLevel};

use crate::support::{EPOCH, MEMBER, OWNER, Rig, drive, live, live_with, pushed};

#[test]
fn a_major_2_hello_gets_the_typed_version_refusal() {
    let mut rig = Rig::new();
    let hello = Hello { major: 2 };
    let mut client = live_with(&mut rig, OWNER, hello);
    let ended = drive(&mut rig, &mut client, |_| false);
    assert!(
        matches!(
            ended,
            Err(Error::Refused(OpenRefusal::Version { supported: 1 }))
        ),
        "{ended:?}"
    );
}

/// A newer build's body comes out unknown, numbered and kept whole, between
/// the events around it, and the cursor moves past it: a mirror never
/// asks for it again.
#[test]
fn a_newer_build_s_event_comes_out_unknown_and_moves_the_cursor() {
    let mut rig = Rig::new();
    for count in 1..=3 {
        rig.device.record(0, EventBody::Lost { count });
    }
    let mut unknown = [0; proto::EVENT_BODY_MAX];
    unknown[..3].copy_from_slice(&[0xf0, 0x01, 0x07]);
    rig.device.newer.push((2, unknown));

    let mut owner = live(&mut rig, OWNER);
    let now = owner.now(&rig);
    owner.connection.subscribe(now, None, false).unwrap();
    drive(&mut rig, &mut owner, |live| live.events.len() >= 3).unwrap();

    let seqs: Vec<(u64, bool)> = owner
        .events
        .iter()
        .filter_map(|event| match event {
            Event::Pushed(model::Pushed::Event(event)) => Some((event.seq, true)),
            Event::Pushed(model::Pushed::Unknown(event)) => {
                assert_eq!(event.raw, model::Hex(unknown));
                assert_eq!(event.epoch, EPOCH);
                Some((event.seq, false))
            }
            _ => None,
        })
        .collect();
    assert_eq!(seqs, [(1, true), (2, false), (3, true)]);
    let newest = Cursor {
        epoch: EPOCH,
        seq: 3,
    };
    assert_eq!(owner.connection.cursor(), Some(newest));

    // Its number is the cursor while it is the newest.
    rig.device.newer.push((3, unknown));
    let mut mirror = live(&mut rig, MEMBER);
    let now = mirror.now(&rig);
    let from = Cursor {
        epoch: EPOCH,
        seq: 2,
    };
    mirror.connection.subscribe(now, Some(from), false).unwrap();
    drive(&mut rig, &mut mirror, |live| !live.events.is_empty()).unwrap();
    assert_eq!(mirror.connection.cursor(), Some(newest));
}

/// Every event body, one of each. `variant` has no wildcard, so a new
/// body fails to compile here until it joins the list.
fn every_body() -> Vec<EventBody> {
    fn variant(body: &EventBody) -> usize {
        match body {
            EventBody::Boot { .. } => 0,
            EventBody::Lock { .. } => 1,
            EventBody::Settings { .. } => 2,
            EventBody::Wifi { .. } => 3,
            EventBody::Enrolled { .. } => 4,
            EventBody::Revoked { .. } => 5,
            EventBody::ClockSync { .. } => 6,
            EventBody::Lost { .. } => 7,
            EventBody::Fed { .. } => 8,
            EventBody::FeedingBlocked { .. } => 9,
            EventBody::Supply(_) => 10,
            EventBody::Plans { .. } => 11,
            EventBody::Missed { .. } => 12,
            EventBody::DispenseRefused { .. } => 13,
            EventBody::MealSoon { .. } => 14,
            EventBody::Food { .. } => 15,
            EventBody::Skipped { .. } => 16,
            EventBody::Unskipped { .. } => 17,
            EventBody::UpdateStop { .. } => 18,
            EventBody::UpdateAtBoot { .. } => 19,
            EventBody::Restarting => 20,
            EventBody::UpdateConfirmed => 21,
            EventBody::Invited { .. } => 22,
        }
    }
    let by = proto::Source::Panel;
    let bodies = vec![
        EventBody::Boot {
            bank: 1,
            reset_flags: 0,
        },
        EventBody::Lock { locked: true, by },
        EventBody::Settings { generation: 1, by },
        EventBody::Wifi { generation: 1, by },
        EventBody::Enrolled {
            client: [1; 4],
            role: proto::Role::Member.into(),
            path: proto::Path::Invite.into(),
        },
        EventBody::Revoked { client: [1; 4], by },
        EventBody::ClockSync { by, jump_ms: None },
        EventBody::Lost { count: 1 },
        EventBody::Fed {
            feed: 1,
            requested: 1,
            completed: 1,
            outcome: proto::FeedOutcome::Done.into(),
            reason: proto::FeedReason::Done.into(),
            hopper: FoodLevel::Present.into(),
            by,
        },
        EventBody::FeedingBlocked {
            reason: Some(proto::Blocked::Fault.into()),
        },
        EventBody::Supply(proto::Supply {
            adapter: Some(false),
            battery: proto::Battery::Reading {
                mv: 4757,
                percent: 100,
            },
            below_floor: Some(false),
        }),
        EventBody::Plans { generation: 1, by },
        EventBody::Missed {
            plan: 0,
            due: 1_793_200_000,
            reason: proto::MissedReason::Late,
        },
        EventBody::DispenseRefused {
            reason: proto::Blocked::Feeding.into(),
        },
        EventBody::MealSoon {
            plan: 0,
            due: 1_793_200_000,
            minute: 450,
            portions: 2,
        },
        EventBody::Food {
            level: proto::FoodLevel::Low.into(),
        },
        EventBody::Skipped {
            plan: 0,
            due: 1_793_200_000,
            by,
        },
        EventBody::Unskipped {
            plan: 0,
            due: 1_793_200_000,
            by,
        },
        EventBody::UpdateStop {
            stop: proto::UpdateStop::Hash.into(),
        },
        EventBody::UpdateAtBoot {
            candidate: proto::UpdateCandidate::None.into(),
        },
        EventBody::Restarting,
        EventBody::UpdateConfirmed,
        EventBody::Invited {
            client: [1; 7],
            by: [2; 7],
        },
    ];
    let mut seen: Vec<usize> = bodies.iter().map(variant).collect();
    seen.dedup();
    assert_eq!(seen, (0..bodies.len()).collect::<Vec<_>>());
    bodies
}

/// Every body reaches every subscriber: what a session is sent does not
/// depend on what its hello says.
#[test]
fn every_subscriber_receives_every_body() {
    let mut rig = Rig::new();
    let bodies = every_body();
    for body in &bodies {
        rig.device.record(0, *body);
    }
    // A last event, so the session knows it has seen everything.
    rig.device.record(0, EventBody::Lost { count: 9 });
    let all: Vec<EventBody> = bodies
        .iter()
        .copied()
        .chain([EventBody::Lost { count: 9 }])
        .collect();

    let mut member = live(&mut rig, MEMBER);
    let now = member.now(&rig);
    member.connection.subscribe(now, None, false).unwrap();
    drive(&mut rig, &mut member, |live| {
        pushed(live).last().map(|e| EventBody::try_from(e.body))
            == Some(Ok(EventBody::Lost { count: 9 }))
    })
    .unwrap();

    let received: Vec<EventBody> = pushed(&member)
        .iter()
        .map(|e| e.body.try_into().unwrap())
        .collect();
    assert_eq!(received, all);
}
