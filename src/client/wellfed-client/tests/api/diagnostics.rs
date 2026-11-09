//! What an owner reads to diagnose a feeder: statistics and their metrics,
//! feed traces, bank 1, and the log tail.

use wellfed_client::api::Incoming;
use wellfed_client::connection::Event;
use wellfed_model as model;
use wellfed_model::{Answer, Op};
use wellfed_proto::api::{self as proto, Metric, Sample};

use crate::support::{Client_, Live, MEMBER, OWNER, Rig, ask, drive, live, open, run};

/// A sealed trace of feed `feed`: a reading every poll of a 400-poll drive,
/// two pages of body.
fn trace(feed: u32) -> (proto::TraceSummary, Vec<u8>, bool) {
    use wellfed_proto::trace::{Entry, Writer};

    let mut writer = Writer::<{ wellfed_proto::trace::BODY_MAX }>::new();
    for poll in 0..400 {
        writer.push(Entry::Current {
            at_ms: 30 * poll,
            value: 60 + poll % 7,
        });
    }
    writer.push(Entry::Mark {
        at_ms: 7_840,
        edges: 1,
        high: false,
    });
    let summary = proto::TraceSummary {
        feed,
        requested: 1,
        completed: 0,
        outcome: proto::FeedOutcome::Failed.into(),
        reason: proto::FeedReason::NoFeedback.into(),
        drive_ms: 12_000,
        samples: 400,
        unit: proto::CurrentUnit::Scaled.into(),
        current_peak: 66,
        current_mean: Some(63),
        rising: 0,
        falling: 1,
        first_edge_ms: Some(7_840),
        beam_breaks: 0,
        reversals: 0,
        battery_rest_mv: None,
        battery_min_mv: None,
        stride: 1,
        truncated: false,
    };
    (summary, writer.bytes().to_vec(), true)
}

#[test]
fn every_statistics_section_answers_an_owner() {
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    for (id, section) in (1..).zip(model::StatsSection::ALL) {
        let answer = ask(&mut rig, &mut owner, id, Op::Stats { section });
        let Answer::Stats(stats) = answer else {
            panic!("{section:?}: {answer:?}");
        };
        let answered = match stats {
            model::Stats::Counters(_) => model::StatsSection::Counters,
            model::Stats::Memory(memory) => {
                assert_eq!(memory.tasks[0].name, "wellfed-loop");
                model::StatsSection::Memory
            }
            model::Stats::Flash(flash) => {
                assert_eq!(flash.boot.storage.len(), proto::FLASH_SECTORS);
                model::StatsSection::Flash
            }
            model::Stats::Power(power) => {
                assert_eq!(power.battery_mv.len(), proto::POWER_HISTORY);
                model::StatsSection::Power
            }
            model::Stats::Food(_) => model::StatsSection::Food,
            model::Stats::Feeds(feeds) => {
                assert_eq!((feeds.kept, feeds.capacity), (1, 8));
                model::StatsSection::Feeds
            }
            model::Stats::Portions(portions) => {
                assert_eq!(portions.delivered_mg, Some(29_400));
                model::StatsSection::Portions
            }
        };
        assert_eq!(answered, section);
    }

    // A member reads them too.
    let mut member = open(&mut rig, MEMBER);
    let section = model::StatsSection::Counters;
    let answer = ask(&mut rig, &mut member, 9, Op::Stats { section });
    assert!(matches!(answer, Answer::Stats(_)), "{answer:?}");

    // The stack walk pauses the feeder's scheduler: owners only, and never
    // during a feed.
    let memory = model::StatsSection::Memory;
    let answer = ask(&mut rig, &mut member, 10, Op::Stats { section: memory });
    assert_eq!(answer, Answer::Refused(model::Refusal::Forbidden));
    rig.feeding = true;
    let answer = ask(&mut rig, &mut owner, 11, Op::Stats { section: memory });
    assert_eq!(answer, Answer::Refused(model::Refusal::Busy));
    rig.feeding = false;
}

/// A feed's trace, read a page at a time by any role: the two pages make
/// the body back, `Busy` while that feed is still recorded, and `NotFound`
/// for a feed with no trace.
#[test]
fn a_trace_is_read_a_page_at_a_time() {
    use wellfed_client::trace::{Pages, TraceError};

    let mut rig = Rig::new();
    let (summary, body, _) = trace(5);
    rig.device.trace = Some((summary, body.clone(), true));
    let mut member = open(&mut rig, MEMBER);

    let mut pages = Pages::new(5);
    let mut id = 0;
    let read = loop {
        id += 1;
        let answer = ask(&mut rig, &mut member, id, pages.next_op());
        if let Some(read) = pages.take(answer).unwrap() {
            break read;
        }
    };
    assert_eq!(id, 2, "two pages");
    assert_eq!(read.summary, summary.into());
    assert!(read.stored);
    let expected = model::Trace::decode(summary.into(), true, &body).unwrap();
    assert_eq!(read, expected);
    assert_eq!(read.entries.len(), 401);

    rig.device.tracing = Some(6);
    let mut pages = Pages::new(6);
    let answer = ask(&mut rig, &mut member, 10, pages.next_op());
    assert_eq!(
        pages.take(answer),
        Err(TraceError::Refused(model::Refusal::Busy))
    );
    let answer = ask(
        &mut rig,
        &mut member,
        11,
        Op::FeedTrace { feed: 7, offset: 0 },
    );
    assert_eq!(answer, Answer::Refused(model::Refusal::NotFound));
}

/// Bank 1 read whole a page at a time, by an owner only: a member is
/// forbidden, a feeder that does not serve it refuses it, and so is a read
/// while an install holds the flash or past the bank's end.
#[test]
fn bank_1_is_read_whole_by_an_owner() {
    use wellfed_client::install::{Bank1Reader, ReadError, STOCK_PAYLOAD_LEN};

    let mut rig = Rig::new();
    let bank: Vec<u8> = (0..STOCK_PAYLOAD_LEN)
        .map(|i| i.to_le_bytes()[0] ^ i.to_le_bytes()[2])
        .collect();
    rig.device.bank1 = Some(bank.clone());
    let mut owner = open(&mut rig, OWNER);

    let mut reader = Bank1Reader::new();
    let mut id: u32 = 0;
    let read = loop {
        id += 1;
        let request = u8::try_from(id % 251).unwrap();
        let answer = ask(&mut rig, &mut owner, request, reader.next_op());
        if let Some(read) = reader.take(answer).unwrap() {
            break read;
        }
    };
    assert_eq!(read, bank);
    assert_eq!(id, STOCK_PAYLOAD_LEN / 768, "a page of 768 bytes each");

    let past = Op::ReadBank1 {
        offset: STOCK_PAYLOAD_LEN,
    };
    let answer = ask(&mut rig, &mut owner, 252, past);
    assert_eq!(answer, Answer::Refused(model::Refusal::Invalid));
    rig.device.installing = true;
    let mut reader = Bank1Reader::new();
    let answer = ask(&mut rig, &mut owner, 253, reader.next_op());
    assert_eq!(
        reader.take(answer),
        Err(ReadError::Refused(model::Refusal::Busy))
    );
    rig.device.installing = false;

    let mut member = open(&mut rig, MEMBER);
    let answer = ask(&mut rig, &mut member, 1, Op::ReadBank1 { offset: 0 });
    assert_eq!(answer, Answer::Refused(model::Refusal::Forbidden));

    // A feeder that serves no backup.
    let mut rig = Rig::new();
    let mut owner = open(&mut rig, OWNER);
    let answer = ask(&mut rig, &mut owner, 1, Op::ReadBank1 { offset: 0 });
    assert_eq!(answer, Answer::Refused(model::Refusal::Unavailable));
}

/// The statistics' metrics go only to subscribers that asked for samples,
/// and one waiting is replaced by the next of its kind, never by another
/// kind.
#[test]
fn stats_metrics_reach_only_their_subscribers_and_replace_their_own_kind() {
    let mut rig = Rig::new();
    let mut current = live(&mut rig, OWNER);
    let mut old = live(&mut rig, MEMBER);
    for (live, samples) in [(&mut current, true), (&mut old, false)] {
        let now = live.now(&rig);
        live.connection.subscribe(now, None, samples).unwrap();
        // A few passes, so the server holds the subscription.
        for _ in 0..3 {
            drive(&mut rig, live, |_| true).unwrap();
        }
    }

    let heap = |at_ms, free| Sample {
        at_ms,
        metric: Metric::Heap { free, min: 100 },
    };
    rig.server.sample(heap(10, 1));
    rig.server.sample(Sample {
        at_ms: 11,
        metric: Metric::Rssi(-60),
    });
    rig.server.sample(heap(12, 2));

    let samples = |live: &Live| -> Vec<model::SampleValue> {
        live.events
            .iter()
            .filter_map(|event| match event {
                Event::Pushed(model::Pushed::Sample(sample)) => Some(sample.value),
                _ => None,
            })
            .collect()
    };
    drive(&mut rig, &mut current, |live| samples(live).len() == 2).unwrap();
    for _ in 0..10 {
        drive(&mut rig, &mut current, |_| true).unwrap();
        drive(&mut rig, &mut old, |_| true).unwrap();
    }
    assert_eq!(
        samples(&current),
        [
            model::SampleValue::Rssi { dbm: -60 },
            model::SampleValue::Heap { free: 2, min: 100 },
        ]
    );
    assert_eq!(samples(&old), []);
}

fn log_rig() -> Rig {
    let mut rig = Rig::new();
    rig.device.log = true;
    rig
}

fn log_line(n: u32) -> String {
    format!("PLTST1 line={n:04} {}", "x".repeat(80))
}

/// Run until the session has read every log line up to `seq`: the lines
/// read and the dropped counts, each batch checked against the frame.
fn follow_log(rig: &mut Rig, client: &mut Client_, seq: u32) -> (Vec<String>, u32) {
    let (mut lines, mut dropped, mut last) = (Vec::new(), 0, 0);
    run(rig, client, |session| {
        while let Some(incoming) = session.next_incoming().unwrap() {
            if let Incoming::Pushed(model::Pushed::Log(batch)) = incoming {
                let text: usize = batch.lines.iter().map(|line| line.len() + 1).sum();
                assert!(text <= proto::LOG_TEXT_MAX, "{text} bytes");
                assert!(batch.seq > last, "batches go forward");
                last = batch.seq;
                dropped += batch.dropped;
                lines.extend(batch.lines);
            }
        }
        last == seq
    });
    (lines, dropped)
}

#[test]
fn an_owner_follows_the_log_in_batches_that_fit_a_frame() {
    let mut rig = log_rig();
    for n in 1..=30 {
        rig.device
            .tail
            .push(format!("{}\r\n", log_line(n)).as_bytes());
    }

    let mut owner = open(&mut rig, OWNER);
    let answer = ask(&mut rig, &mut owner, 1, Op::TailLog { after: None });
    assert_eq!(answer, Answer::Done { generation: None });
    let (lines, dropped) = follow_log(&mut rig, &mut owner, 30);
    let expected: Vec<_> = (1..=30).map(log_line).collect();
    assert_eq!(
        lines, expected,
        "every line, in order, across several frames"
    );
    assert_eq!(dropped, 0);

    // New lines follow live, and a request is still answered between them.
    rig.device.tail.push(log_line(31).as_bytes());
    let (lines, _) = follow_log(&mut rig, &mut owner, 31);
    assert_eq!(lines, [log_line(31)]);
    let answer = ask(&mut rig, &mut owner, 2, Op::Status);
    assert!(matches!(answer, Answer::Status(_)), "{answer:?}");

    // Asking again moves the cursor back.
    ask(&mut rig, &mut owner, 3, Op::TailLog { after: Some(29) });
    let (lines, _) = follow_log(&mut rig, &mut owner, 31);
    assert_eq!(lines, [log_line(30), log_line(31)]);
}

#[test]
fn lines_the_tail_overwrote_before_they_were_sent_are_counted_dropped() {
    let mut rig = log_rig();
    rig.device.tail.push(log_line(1).as_bytes());
    let mut owner = open(&mut rig, OWNER);
    ask(&mut rig, &mut owner, 1, Op::TailLog { after: None });
    follow_log(&mut rig, &mut owner, 1);

    // Far more than the tail holds, between two of the session's turns.
    for n in 2..=200 {
        rig.device.tail.push(log_line(n).as_bytes());
    }
    let (lines, dropped) = follow_log(&mut rig, &mut owner, 200);
    assert!(dropped > 100, "{dropped}");
    assert_eq!(u32::try_from(lines.len()).unwrap() + dropped, 199);
    assert_eq!(lines.last(), Some(&log_line(200)));
}

#[test]
fn only_an_owner_may_tail_and_a_feeder_without_a_tail_pushes_nothing() {
    let tail = Op::TailLog { after: None };
    let mut rig = log_rig();
    rig.device.tail.push(log_line(1).as_bytes());
    let mut member = open(&mut rig, MEMBER);
    let answer = ask(&mut rig, &mut member, 1, tail.clone());
    assert_eq!(answer, Answer::Refused(model::Refusal::Forbidden));

    // A feeder that keeps no log takes the cursor and pushes nothing.
    let mut plain = Rig::new();
    plain.device.tail.push(log_line(1).as_bytes());
    let mut owner = open(&mut plain, OWNER);
    let done = Answer::Done { generation: None };
    assert_eq!(ask(&mut plain, &mut owner, 1, tail), done);

    for _ in 0..20 {
        rig.pass();
        plain.pass();
        let bytes = rig.collect(member.slot);
        member.session.receive(&bytes).unwrap();
        let bytes = plain.collect(owner.slot);
        owner.session.receive(&bytes).unwrap();
    }
    for session in [&mut member.session, &mut owner.session] {
        while let Some(incoming) = session.next_incoming().unwrap() {
            assert!(!matches!(incoming, Incoming::Pushed(model::Pushed::Log(_))));
        }
    }
}
