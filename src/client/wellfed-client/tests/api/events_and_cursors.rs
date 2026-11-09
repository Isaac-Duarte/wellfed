//! The event history a subscriber reads: from its cursor, in frames of
//! many events, with gaps and epochs.

use wellfed_client::api::Incoming;
use wellfed_client::connection::{Cursor, Resume};
use wellfed_device_api::Device;
use wellfed_model as model;
use wellfed_model::Hello;
use wellfed_proto::api::{self as proto, EventBody, FoodLevel};

use crate::support::{
    Client_, EPOCH, MEMBER, OWNER, Rig, drive, live, live_with, open, pushed, run,
};

#[test]
fn a_subscriber_reads_every_event_after_its_cursor_and_learns_of_a_gap() {
    let mut rig = Rig::new();
    for generation in 10..15 {
        let body = EventBody::Settings {
            generation,
            by: proto::Source::Panel,
        };
        rig.device.record(0, body);
    }
    // Events 1 and 2 left the ring.
    rig.device.oldest = 3;

    let mut owner = open(&mut rig, OWNER);
    let bytes = owner.session.subscribe(Some(0), false).unwrap();
    owner.outbox.extend(bytes);
    let mut seen = Vec::new();
    run(&mut rig, &mut owner, |session| {
        while let Some(incoming) = session.next_incoming().unwrap() {
            match incoming {
                Incoming::Pushed(model::Pushed::Gap(gap)) => seen.push((gap.from, gap.to)),
                Incoming::Pushed(model::Pushed::Event(event)) => seen.push((event.seq, 0)),
                _ => {}
            }
        }
        seen.len() == 4
    });
    assert_eq!(seen, [(1, 2), (3, 0), (4, 0), (5, 0)]);

    // A new event reaches the subscriber live.
    rig.device.record(0, EventBody::Lost { count: 1 });
    let mut live = None;
    run(&mut rig, &mut owner, |session| {
        if let Some(Incoming::Pushed(model::Pushed::Event(event))) =
            session.next_incoming().unwrap()
        {
            live = Some(event.seq);
        }
        live.is_some()
    });
    assert_eq!(live, Some(6));
}

/// What arrived in each pass's frames, until `count` events and gaps.
fn frames(rig: &mut Rig, client: &mut Client_, count: usize) -> Vec<Vec<(u64, u64)>> {
    let mut frames = Vec::new();
    let mut seen = 0;
    run(rig, client, |session| {
        let mut frame = Vec::new();
        while let Some(incoming) = session.next_incoming().unwrap() {
            match incoming {
                Incoming::Pushed(model::Pushed::Gap(gap)) => frame.push((gap.from, gap.to)),
                Incoming::Pushed(model::Pushed::Event(event)) => frame.push((event.seq, 0)),
                _ => {}
            }
        }
        seen += frame.len();
        if !frame.is_empty() {
            frames.push(frame);
        }
        seen >= count
    });
    frames
}

#[test]
fn a_catch_up_arrives_in_frames_of_many_events() {
    let mut rig = Rig::new();
    for generation in 0..200 {
        let body = EventBody::Settings {
            generation,
            by: proto::Source::Panel,
        };
        rig.device.record(0, body);
    }

    let mut owner = open(&mut rig, OWNER);
    let bytes = owner.session.subscribe(None, false).unwrap();
    owner.outbox.extend(bytes);
    let frames = frames(&mut rig, &mut owner, 200);

    let seqs: Vec<u64> = frames.iter().flatten().map(|&(seq, _)| seq).collect();
    assert_eq!(seqs, (1..=200).collect::<Vec<_>>());
    // One frame a pass, each as full as a pass reads: a few frames, not
    // two hundred.
    assert!(frames.len() <= 4, "{} frames", frames.len());
    assert!(frames.iter().all(|frame| frame.len() >= 8), "{frames:?}");
}

/// Events as large as any encodes fill a frame and stop there: each frame
/// holds as many as fit beside the room the largest needs, and the rest
/// follow, in order.
#[test]
fn the_largest_events_fill_a_frame_and_the_rest_follow() {
    let mut rig = Rig::new();
    let first = u64::MAX - 100;
    for seq in first..first + 70 {
        rig.device.events.push(proto::Event {
            seq,
            at: Some(u32::MAX),
            body: EventBody::Fed {
                feed: u32::MAX,
                requested: u8::MAX,
                completed: u8::MAX,
                outcome: proto::FeedOutcome::Uncertain,
                reason: proto::FeedReason::Output,
                hopper: FoodLevel::Unread,
                by: proto::Source::Client([0xff; 4]),
            },
        });
    }

    let mut owner = open(&mut rig, OWNER);
    let bytes = owner.session.subscribe(None, false).unwrap();
    owner.outbox.extend(bytes);
    let frames = frames(&mut rig, &mut owner, 70);

    // 33 bytes each, the body's length among them: 30 fill all but 15 of
    // the batch's 1,005.
    let per_frame = (proto::EVENTS_BYTES_MAX - proto::EVENT_BYTES_MAX) / proto::EVENT_BYTES_MAX + 1;
    assert_eq!(per_frame, 30);
    let sizes: Vec<usize> = frames.iter().map(Vec::len).collect();
    assert_eq!(sizes, [per_frame, per_frame, 70 - 2 * per_frame]);
    let seqs: Vec<u64> = frames.iter().flatten().map(|&(seq, _)| seq).collect();
    assert_eq!(seqs, (first..first + 70).collect::<Vec<_>>());
}

#[test]
fn a_gap_ends_a_batch_and_arrives_next() {
    let mut rig = Rig::new();
    for count in 1..=10 {
        rig.device.record(0, EventBody::Lost { count });
    }
    rig.device.events.retain(|e| !(4..=6).contains(&e.seq));
    rig.device.lost = Some((4, 6));

    let mut owner = open(&mut rig, OWNER);
    let bytes = owner.session.subscribe(None, false).unwrap();
    owner.outbox.extend(bytes);
    let frames = frames(&mut rig, &mut owner, 8);

    assert_eq!(
        frames,
        [
            vec![(1, 0), (2, 0), (3, 0)],
            vec![(4, 6)],
            vec![(7, 0), (8, 0), (9, 0), (10, 0)],
        ]
    );
}

#[test]
fn a_welcome_waits_for_the_event_ring() {
    let mut rig = Rig::new();
    rig.device.epoch = None;
    let hello = Hello { major: 1 };
    let mut owner = live_with(&mut rig, OWNER, hello);
    let mut passes = 0;
    drive(&mut rig, &mut owner, |_| {
        passes += 1;
        passes == 20
    })
    .unwrap();
    assert!(owner.connection.opened().is_none());

    rig.device.epoch = Some(EPOCH);
    drive(&mut rig, &mut owner, |live| {
        live.connection.opened().is_some()
    })
    .unwrap();
}

#[test]
fn a_cursor_resumes_in_its_epoch_and_restarts_from_another() {
    let mut rig = Rig::new();
    for generation in 1..=3 {
        let by = proto::Source::Panel;
        rig.device.record(0, EventBody::Settings { generation, by });
    }

    let mut owner = live(&mut rig, OWNER);
    let same = Cursor {
        epoch: EPOCH,
        seq: 2,
    };
    let now = owner.now(&rig);
    let resume = owner.connection.subscribe(now, Some(same), false).unwrap();
    assert_eq!(resume, Resume::Continued);
    drive(&mut rig, &mut owner, |live| pushed(live).len() == 1).unwrap();
    assert_eq!(pushed(&owner)[0].seq, 3);
    let newest = Cursor {
        epoch: EPOCH,
        seq: 3,
    };
    assert_eq!(owner.connection.cursor(), Some(newest));

    // A mirror of a ring since re-created: its numbers mean nothing here.
    let mut mirror = live(&mut rig, MEMBER);
    let stale = Cursor {
        epoch: EPOCH ^ 1,
        seq: 2,
    };
    let now = mirror.now(&rig);
    let resume = mirror
        .connection
        .subscribe(now, Some(stale), false)
        .unwrap();
    assert_eq!(resume, Resume::Restarted);
    drive(&mut rig, &mut mirror, |live| pushed(live).len() == 3).unwrap();
    let seqs: Vec<u64> = pushed(&mirror).iter().map(|e| e.seq).collect();
    assert_eq!(seqs, [1, 2, 3]);
    assert_eq!(mirror.connection.cursor(), Some(newest));
}
