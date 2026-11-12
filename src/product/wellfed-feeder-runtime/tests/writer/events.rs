//! The event ring: queued events appended and read back across a reboot,
//! readers catching up a batch at a time and taking turns, and the slot
//! kept for a feed's outcome.

use std::sync::MutexGuard;

use wellfed_device::store::events::Read;
use wellfed_device_api::Reader;
use wellfed_device_runtime::events;
use wellfed_device_runtime::writer::{self as writer};
use wellfed_feeder_runtime::writer::{Record, Writer};
use wellfed_hal::flash_lock::provider::Provider;
use wellfed_hal::interlock::INTERLOCK;
use wellfed_proto::api::EventBody;

use crate::support::{A, Armed, B, EVENTS_PROGRAMS_FAIL, Unreached, start, step};

/// The loop's queued events reach the ring, survive a reboot, and
/// come back through the read mailbox in order.
#[test]
fn queued_events_are_appended_and_read_back_after_a_reboot() {
    let (_guard, mut writer, mut provider) = start();
    for _ in 0..7 {
        step(&mut writer, &mut provider); // open, net open, time open, keys, events, stats, its save
    }
    assert!(events::record(Some(1_700_000_000), [1; 16]));
    assert!(events::record(None, [2; 16]));
    step(&mut writer, &mut provider);
    step(&mut writer, &mut provider);
    assert_eq!(events::last_seq(), Some(2));

    // The next boot finds both; a reader asks, the writer answers.
    events::reset();
    let mut writer = Writer::default();
    for _ in 0..4 {
        step(&mut writer, &mut provider);
    }
    assert_eq!(
        step(&mut writer, &mut provider),
        Some(Record::Events {
            open: true,
            last: Some(2),
            oldest: Some(1),
            epoch: events::epoch().unwrap(),
        })
    );
    step(&mut writer, &mut provider); // stats
    step(&mut writer, &mut provider); // the boot's stats save
    assert_eq!(events::read_result(A, None), None, "asked");
    step(&mut writer, &mut provider);
    let Some(Read::Record(first)) = events::read_result(A, None) else {
        panic!("the first event");
    };
    assert_eq!(
        (first.seq, first.at, first.body),
        (1, Some(1_700_000_000), [1; 16])
    );
    // One answer is a batch: the next record needs no other read.
    assert!(matches!(
        events::read_result(A, Some(1)),
        Some(Read::Record(r)) if r.seq == 2
    ));

    // At the newest record there is nothing to read, until the next append.
    assert_eq!(events::read_result(A, Some(2)), Some(Read::End));
    assert!(events::record(None, [3; 16]));
    step(&mut writer, &mut provider);
    assert_eq!(
        events::read_result(A, Some(2)),
        None,
        "stale once a record lands"
    );
    step(&mut writer, &mut provider);
    assert!(matches!(
        events::read_result(A, Some(2)),
        Some(Read::Record(r)) if r.seq == 3
    ));

    // A subscriber caught up does not take the slot from one catching up:
    // the answer for the first waits however often the second polls.
    assert_eq!(events::read_result(A, Some(3)), Some(Read::End));
    assert_eq!(events::read_result(B, None), None, "asked");
    step(&mut writer, &mut provider);
    for _ in 0..3 {
        assert_eq!(events::read_result(A, Some(3)), Some(Read::End));
    }
    assert!(matches!(
        events::read_result(B, None),
        Some(Read::Record(r)) if r.seq == 1
    ));
}

/// A reader catching up gets a batch a read: it walks through it without
/// asking again, a second reader inside it shares it, and the writer keeps
/// the read pace between batches rather than falling back to its idle poll.
#[test]
fn a_reader_catching_up_takes_a_batch_a_read() {
    let (_guard, mut writer, mut provider) = start();
    for _ in 0..7 {
        step(&mut writer, &mut provider); // open, net open, time open, keys, events, stats, its save
    }
    let total = events::READ_BATCH + 4;
    for n in 0..total {
        assert!(events::record(None, [u8::try_from(n).unwrap(); 16]));
        step(&mut writer, &mut provider);
    }
    assert_eq!(events::last_seq(), Some(total as u64));

    assert_eq!(events::read_result(A, None), None, "asked");
    // SAFETY: the provider outlives the run.
    let answered = unsafe { writer.step::<Unreached, Armed>(&mut provider) };
    assert_eq!(
        answered.delay_ms,
        writer::EVENTS_POLL_MS,
        "a reader is about"
    );

    // The whole batch, one record a call, with no read between.
    let mut after = None;
    for seq in 1..=events::READ_BATCH as u64 {
        let Some(Read::Record(record)) = events::read_result(A, after) else {
            panic!("record {seq} from the batch");
        };
        assert_eq!(record.seq, seq);
        after = Some(seq);
    }
    // A second reader inside the batch shares it.
    assert!(matches!(
        events::read_result(B, Some(3)),
        Some(Read::Record(r)) if r.seq == 4
    ));

    // Past the batch the reader asks again; the writer, still at the read
    // pace, answers on its next poll.
    // SAFETY: as above.
    let idle = unsafe { writer.step::<Unreached, Armed>(&mut provider) };
    assert_eq!(
        idle.delay_ms,
        writer::EVENTS_POLL_MS,
        "lingers after a read"
    );
    assert_eq!(events::read_result(A, after), None, "asked");
    step(&mut writer, &mut provider);
    for seq in events::READ_BATCH as u64 + 1..=total as u64 {
        let Some(Read::Record(record)) = events::read_result(A, after) else {
            panic!("record {seq} from the next batch");
        };
        assert_eq!(record.seq, seq);
        after = Some(seq);
    }
    assert_eq!(events::read_result(A, after), Some(Read::End));

    // With no reader the writer goes back to its idle poll.
    let delay = (0..=writer::READ_LINGER_POLLS)
        // SAFETY: as above.
        .map(|_| unsafe { writer.step::<Unreached, Armed>(&mut provider) }.delay_ms)
        .last();
    assert_eq!(delay, Some(writer::POLL_MS));
}

/// One loop pass's reads for the subscriber in `reader`'s slot at `after`:
/// everything the slot answers it now; true once it is at the newest
/// record.
fn drain(reader: usize, after: &mut Option<u64>, got: &mut Vec<u64>) -> bool {
    loop {
        match events::read_result(Reader::slot(reader), *after) {
            Some(Read::Record(record)) => {
                got.push(record.seq);
                *after = Some(record.seq);
            }
            Some(Read::Gap { to, .. }) => *after = Some(to),
            Some(Read::End) => return true,
            None => return false,
        }
    }
}

/// A writer with `total` records appended, ready for reads.
fn with_records(total: u8) -> (MutexGuard<'static, ()>, Writer<Armed>, Provider) {
    let (guard, mut writer, mut provider) = start();
    for _ in 0..7 {
        step(&mut writer, &mut provider); // open, net open, time open, keys, events, stats, its save
    }
    for n in 0..total {
        assert!(events::record(None, [n; 16]));
        step(&mut writer, &mut provider);
    }
    assert_eq!(events::last_seq(), Some(u64::from(total)));
    (guard, writer, provider)
}

/// Subscribers in slots 0.. at `start`, visited in `order` each pass with
/// a writer round trip between passes, until each has every record after
/// its cursor: the most passes in a row each went without a record.
fn catch_up(total: u8, start: &[Option<u64>], order: &[usize]) -> Vec<usize> {
    let (_guard, mut writer, mut provider) = with_records(total);
    let n = start.len();
    let mut cursors = start.to_vec();
    let mut got = vec![Vec::new(); n];
    let mut done = vec![false; n];
    let mut idle = vec![0; n];
    let mut worst = vec![0; n];

    let mut passes = 0;
    while done.contains(&false) {
        for &reader in order {
            let before = got[reader].len();
            done[reader] = drain(reader, &mut cursors[reader], &mut got[reader]);
            idle[reader] = if done[reader] || got[reader].len() > before {
                0
            } else {
                idle[reader] + 1
            };
            worst[reader] = worst[reader].max(idle[reader]);
        }
        step(&mut writer, &mut provider);
        passes += 1;
        assert!(passes < 200, "no progress: {start:?} in {order:?}");
    }

    for (reader, cursor) in start.iter().enumerate() {
        let from = cursor.map_or(1, |c| c + 1);
        assert_eq!(got[reader], (from..=u64::from(total)).collect::<Vec<_>>());
    }
    worst
}

/// Each subscriber goes at most one pass more than there are subscribers
/// without a record: a round of the others' batches, the pass that fetches
/// its own, and one more when its turn came round just before it asked.
fn assert_turns(total: u8, start: &[Option<u64>]) {
    let forward: Vec<usize> = (0..start.len()).collect();
    let backward: Vec<usize> = forward.iter().rev().copied().collect();
    for order in [forward, backward] {
        let worst = catch_up(total, start, &order);
        let bound = start.len() + 1;
        assert!(
            worst.iter().all(|&w| w <= bound),
            "{start:?} in {order:?}: idle {worst:?}, bound {bound}"
        );
    }
}

/// Two subscribers catching up from different units take turns, whichever
/// the loop visits first.
#[test]
fn two_readers_in_different_units_take_turns() {
    assert_turns(200, &[None, Some(150)]);
}

/// Two subscribers inside one unit, near each other or a batch apart,
/// share what they can and take turns for the rest, across the unit's
/// end, in either order.
#[test]
fn overlapping_readers_in_one_unit_take_turns() {
    for second in [5, 10, 20, 31, 40] {
        assert_turns(255, &[None, Some(second)]);
    }
    assert_turns(255, &[Some(100), Some(120)]);
    assert_turns(120, &[Some(10), None]);
}

/// Three and four subscribers at spread cursors each get a turn a round.
#[test]
fn three_and_four_readers_each_get_a_turn() {
    // Orders a random search found at the bound.
    for (start, order) in [
        (
            vec![Some(250), None, Some(127), Some(128)],
            vec![2, 1, 3, 0],
        ),
        (vec![Some(93), Some(229), Some(42)], vec![0, 2, 1]),
    ] {
        let worst = catch_up(255, &start, &order);
        assert!(
            worst.iter().all(|&w| w <= start.len() + 1),
            "{start:?} in {order:?}: idle {worst:?}"
        );
    }

    assert_turns(200, &[None, Some(150), Some(60)]);
    assert_turns(200, &[None, Some(150), Some(60), Some(100)]);
    assert_turns(255, &[Some(200), Some(10), Some(130), None]);
}

/// Once caught up, each new record reaches every subscriber within a few
/// passes.
#[test]
fn caught_up_readers_each_get_a_new_record() {
    let (_guard, mut writer, mut provider) = with_records(120);
    let mut cursors = [None, Some(10), Some(60)];
    let mut got = [Vec::new(), Vec::new(), Vec::new()];
    for _ in 0..100 {
        for reader in 0..3 {
            drain(reader, &mut cursors[reader], &mut got[reader]);
        }
        step(&mut writer, &mut provider);
    }
    assert_eq!(cursors, [Some(120); 3]);

    for round in 0..3 {
        assert!(events::record(None, [round; 16]));
        step(&mut writer, &mut provider);
        let want = Some(121 + u64::from(round));
        let mut passes = 0;
        while cursors != [want; 3] {
            for reader in 0..3 {
                drain(reader, &mut cursors[reader], &mut got[reader]);
            }
            step(&mut writer, &mut provider);
            passes += 1;
            assert!(passes <= 4, "round {round}: {cursors:?}");
        }
    }
}

/// A batch fetched for a subscriber that ended is nobody's: the next ask
/// replaces it at once.
#[test]
fn a_departed_reader_s_batch_gives_way_at_once() {
    let (_guard, mut writer, mut provider) = with_records(200);
    assert_eq!(events::read_result(Reader::slot(0), None), None, "asked");
    step(&mut writer, &mut provider);

    events::reader_left(Reader::slot(0));
    assert_eq!(
        events::read_result(Reader::slot(1), Some(150)),
        None,
        "asked"
    );
    step(&mut writer, &mut provider);
    assert!(matches!(
        events::read_result(Reader::slot(1), Some(150)),
        Some(Read::Record(r)) if r.seq == 151
    ));
}

/// A batch whose subscriber stopped asking without ending (its last frame
/// still going out) holds the slot for a bounded number of other asks;
/// then another subscriber's read replaces it.
#[test]
fn a_stalled_reader_s_batch_gives_way_after_its_bound() {
    let (_guard, mut writer, mut provider) = with_records(200);
    assert_eq!(events::read_result(Reader::slot(0), None), None, "asked");
    step(&mut writer, &mut provider);

    for _ in 0..events::MISSES_MAX {
        assert_eq!(
            events::read_result(Reader::slot(1), Some(150)),
            None,
            "held for its subscriber"
        );
        step(&mut writer, &mut provider);
    }
    assert_eq!(
        events::read_result(Reader::slot(1), Some(150)),
        None,
        "asked"
    );
    step(&mut writer, &mut provider);
    assert!(matches!(
        events::read_result(Reader::slot(1), Some(150)),
        Some(Read::Record(r)) if r.seq == 151
    ));
}

/// Events pile up while a feed holds the outputs; the last slot is kept for
/// the feed's own outcome, which is appended after the release with the rest.
#[test]
fn a_feeds_outcome_has_a_slot_however_many_events_pile_up() {
    let (_guard, mut writer, mut provider) = start();
    for _ in 0..7 {
        step(&mut writer, &mut provider); // open, net open, time open, keys, events, stats, its save
    }
    INTERLOCK.claim_feed(u32::MAX).unwrap();

    let mut queued = 0_u8;
    while events::record(None, [queued; 16]) {
        queued += 1;
    }
    assert_eq!(usize::from(queued), events::QUEUE - events::RESERVED);
    assert!(
        events::record_outcome(None, [0xfe; 16]),
        "the outcome's slot"
    );
    assert!(!events::record_outcome(None, [0xff; 16]), "one slot");
    step(&mut writer, &mut provider);
    assert_eq!(events::last_seq(), None, "held off by the feed");
    assert_eq!(
        events::take_lost(),
        0,
        "the count waits for a slot its Lost fits"
    );

    INTERLOCK.release_feed();
    for _ in 0..events::QUEUE {
        step(&mut writer, &mut provider);
    }
    assert_eq!(events::last_seq(), Some(u64::from(queued) + 1));
    assert_eq!(
        events::take_lost(),
        2,
        "every record dropped while the queue was full"
    );
    assert_eq!(
        events::read_result(A, Some(u64::from(queued))),
        None,
        "asked"
    );
    step(&mut writer, &mut provider);
    let Some(Read::Record(last)) = events::read_result(A, Some(u64::from(queued))) else {
        panic!("the outcome");
    };
    assert_eq!(last.body, [0xfe; 16]);
}

/// A queued event the ring refused (a failed program) is counted lost, so a
/// `Lost` follows it into the history. A refused `Lost` keeps its count
/// until a record lands, then hands it back: not at once, or a store whose
/// programs keep failing would queue one a pass, each costing a slot.
#[test]
fn an_event_the_ring_refused_is_counted_lost_and_a_refused_lost_waits() {
    let (_guard, mut writer, mut provider) = start();
    for _ in 0..7 {
        step(&mut writer, &mut provider); // open, net open, time open, keys, events, stats, its save
    }
    EVENTS_PROGRAMS_FAIL.with(|f| f.set(true));

    assert!(events::record(None, [1; 16]));
    assert_eq!(step(&mut writer, &mut provider), Some(Record::EventError));
    assert_eq!(events::take_lost(), 1, "the refused event");

    let mut lost = [0; 16];
    wellfed_proto::encode(&EventBody::Lost { count: 3 }, &mut lost).unwrap();
    assert!(events::record(None, lost));
    assert_eq!(step(&mut writer, &mut provider), Some(Record::EventError));
    assert_eq!(events::take_lost(), 0, "held while the ring refuses");

    EVENTS_PROGRAMS_FAIL.with(|f| f.set(false));
    assert!(events::record(None, [2; 16]));
    step(&mut writer, &mut provider);
    assert_eq!(
        events::take_lost(),
        3,
        "the refused Lost's count, once one lands"
    );
}
