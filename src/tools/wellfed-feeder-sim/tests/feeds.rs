//! The simulator's feeding owner over the device API: a feed's number is
//! its journal id, its result is a `Fed` event, a cancel ends it, and a
//! restart in the middle of one resolves it as uncertain under its own
//! number, never as a new feed. A feed asks for 1 to 48 portions, and a
//! Dispense press takes the same owner. The feeding history reads every
//! feed back after a restart.

mod common;

use common::paused;
use std::assert_matches;
use std::rc::Rc;
use std::time::Duration;

use tokio::time::sleep;
use wellfed_client::history::{Delivered, Entry};
use wellfed_connect::Connection;
use wellfed_core_feeder::feed::food_read::{FOOD_READ_EVERY_MS, FOOD_READ_FIRST_MS};
use wellfed_feeder_sim::feeding;
use wellfed_feeder_sim::flash::Flash;
use wellfed_feeder_sim::harness::{Running, local};
use wellfed_feeder_sim::serve::Control;
use wellfed_hal::interlock::WRITE_WAIT_MS;
use wellfed_model::{
    Answer, Blocked, EventBody, FeedFault, FeedOutcome, FeedReason, FeederEvent,
    FeederSettingsPatch, FoodLevel, Generation, Op, Pushed, Readiness, Refusal, Source,
};
use wellfed_session::noise::Keypair;

const OWNER: [u8; 32] = [0x21; 32];

/// A simulator on fresh flash, and its owner subscribed to the events.
async fn subscribed() -> (Rc<Flash>, Running, Connection) {
    let flash = Flash::memory();
    let sim = start(&flash).await;
    let mut owner = open(&sim).await;
    owner.subscribe(None, false).await.unwrap();
    (flash, sim, owner)
}

async fn start(flash: &Rc<Flash>) -> Running {
    Running::start(flash, Some(*Keypair::from_secret(OWNER).public())).await
}

async fn open(sim: &Running) -> Connection {
    Connection::open(
        &Keypair::from_secret(OWNER),
        sim.identity,
        sim.address,
        [0xa1; 32],
    )
    .await
    .unwrap()
}

/// The next `Fed` event the subscription pushes.
async fn next_fed(connection: &mut Connection) -> EventBody {
    loop {
        let pushed = connection.next(Duration::from_secs(5)).await.unwrap();
        if let Pushed::Event(event) = pushed
            && matches!(event.body, EventBody::Feeder(FeederEvent::Fed { .. }))
        {
            return event.body;
        }
    }
}

async fn readiness(connection: &mut Connection, id: u8) -> Readiness {
    match connection.ask([id; 16], &Op::Status).await.unwrap() {
        Answer::Status(status) => status.feeder().unwrap().readiness,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_feed_is_numbered_by_the_journal_and_ends_in_its_fed_event() {
    local(async {
        let (flash, sim, mut owner) = subscribed().await;

        let answer = owner.ask([1; 16], &Op::Feed { portions: 2 }).await.unwrap();
        assert_eq!(answer, Answer::Feeding { feed: 1 });
        let again = owner.ask([1; 16], &Op::Feed { portions: 2 }).await.unwrap();
        assert_eq!(again, answer, "a repeated id is the same feed");
        let busy = owner.ask([2; 16], &Op::Feed { portions: 1 }).await.unwrap();
        assert_eq!(busy, Answer::Refused(Refusal::Busy), "one feed at a time");

        let EventBody::Feeder(FeederEvent::Fed {
            feed,
            requested,
            completed,
            outcome,
            reason,
            hopper,
            by: Source::Client { .. },
        }) = next_fed(&mut owner).await
        else {
            panic!("a feed by the client");
        };
        assert_eq!((feed, requested, completed), (1, 2, 2));
        assert_eq!((outcome, reason), (FeedOutcome::Done, FeedReason::Done));
        assert_eq!(
            hopper,
            FoodLevel::Present,
            "the simulator's hopper starts full"
        );
        assert_eq!(readiness(&mut owner, 3).await, Readiness::Ready);

        let next = owner.ask([4; 16], &Op::Feed { portions: 1 }).await.unwrap();
        assert_eq!(next, Answer::Feeding { feed: 2 });
        next_fed(&mut owner).await;
        assert_eq!(feeding::reserved(&flash), Some(2));
        sim.stop().await;
    });
}

/// Every feed asks for 1 to 48 portions; anything else is refused by
/// name, with the value, before readiness is checked, and reserves no
/// number.
#[test]
fn a_feed_outside_one_to_48_portions_is_refused_by_name() {
    local(async {
        let flash = Flash::memory();
        let sim = start(&flash).await;
        sim.control(Control::StallFeeds(true));
        let mut owner = open(&sim).await;
        owner.subscribe(None, false).await.unwrap();

        for (id, asked) in [(1, 0), (2, 49), (3, u8::MAX)] {
            let answer = owner.ask([id; 16], &Op::Feed { portions: asked }).await;
            let refused = Answer::Refused(Refusal::Portions { asked });
            assert_eq!(answer.unwrap(), refused, "{asked} portions");
        }
        let most = owner
            .ask([4; 16], &Op::Feed { portions: 48 })
            .await
            .unwrap();
        assert_eq!(most, Answer::Feeding { feed: 1 }, "48 is allowed");

        // Named even while a feed runs: what to change, not when to retry.
        let during = owner.ask([5; 16], &Op::Feed { portions: 0 }).await.unwrap();
        assert_eq!(during, Answer::Refused(Refusal::Portions { asked: 0 }));
        let cancel = owner.ask([6; 16], &Op::CancelFeed { feed: 1 }).await;
        assert_matches!(cancel, Ok(Answer::Done { .. }));
        let EventBody::Feeder(FeederEvent::Fed {
            feed, requested, ..
        }) = next_fed(&mut owner).await
        else {
            unreachable!("next_fed returns a feed")
        };
        assert_eq!((feed, requested), (1, 48));
        sim.stop().await;
    });
}

/// Dispense goes through the owner a client's feed takes: one portion, by
/// the panel. A press while a feed runs starts nothing, queues nothing and
/// is recorded once for its reason; the next feed takes the next number.
#[test]
fn a_dispense_press_feeds_one_portion_and_one_during_a_feed_is_recorded_not_queued() {
    local(async {
        let (_flash, sim, mut owner) = subscribed().await;
        // The panel starts locked; a locked press would do nothing.
        sim.type_line("hold lock").unwrap();
        loop {
            let pushed = owner.next(Duration::from_secs(10)).await.unwrap();
            if let Pushed::Event(event) = pushed
                && matches!(
                    event.body,
                    EventBody::Feeder(FeederEvent::Lock { locked: false, .. })
                )
            {
                break;
            }
        }

        sim.type_line("press dispense").unwrap();
        let EventBody::Feeder(FeederEvent::Fed {
            feed,
            requested,
            completed,
            outcome,
            by,
            ..
        }) = next_fed(&mut owner).await
        else {
            unreachable!("next_fed returns a feed")
        };
        assert_eq!((feed, requested, completed), (1, 1, 1));
        assert_eq!((outcome, by), (FeedOutcome::Done, Source::Panel));

        sim.control(Control::StallFeeds(true));
        let answer = owner.ask([1; 16], &Op::Feed { portions: 2 }).await.unwrap();
        assert_eq!(answer, Answer::Feeding { feed: 2 });
        sim.type_line("press dispense").unwrap();
        sim.type_line("press dispense").unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        let cancel = owner.ask([2; 16], &Op::CancelFeed { feed: 2 }).await;
        assert_matches!(cancel, Ok(Answer::Done { .. }));

        // Event appends wait for the outputs, so the refused press's event
        // may follow the feed's result.
        let mut bodies = Vec::new();
        while let Ok(Pushed::Event(event)) = owner.next(Duration::from_millis(500)).await {
            bodies.push(event.body);
        }
        let fed: Vec<u32> = bodies
            .iter()
            .filter_map(|body| match body {
                EventBody::Feeder(FeederEvent::Fed { feed, .. }) => Some(*feed),
                _ => None,
            })
            .collect();
        assert_eq!(fed, [2], "the presses queued no feed");
        let refused: Vec<&EventBody> = bodies
            .iter()
            .filter(|body| matches!(body, EventBody::Feeder(FeederEvent::DispenseRefused { .. })))
            .collect();
        assert_eq!(
            refused,
            [&EventBody::Feeder(FeederEvent::DispenseRefused {
                reason: Blocked::Feeding
            })],
            "one event for the reason"
        );

        sim.control(Control::StallFeeds(false));
        let next = owner.ask([3; 16], &Op::Feed { portions: 1 }).await.unwrap();
        assert_eq!(next, Answer::Feeding { feed: 3 });
        next_fed(&mut owner).await;
        sim.stop().await;
    });
}

#[test]
fn a_cancel_ends_the_feed_and_finds_nothing_once_it_has_ended() {
    local(async {
        let flash = Flash::memory();
        let sim = start(&flash).await;
        sim.control(Control::StallFeeds(true));
        let mut owner = open(&sim).await;
        owner.subscribe(None, false).await.unwrap();

        let answer = owner.ask([1; 16], &Op::Feed { portions: 3 }).await.unwrap();
        assert_eq!(answer, Answer::Feeding { feed: 1 });
        let cancel = owner.ask([2; 16], &Op::CancelFeed { feed: 1 }).await;
        assert_matches!(cancel, Ok(Answer::Done { .. }));
        let EventBody::Feeder(FeederEvent::Fed { feed, outcome, .. }) = next_fed(&mut owner).await
        else {
            unreachable!("next_fed returns a feed")
        };
        assert_eq!((feed, outcome), (1, FeedOutcome::Cancelled));

        let late = owner
            .ask([3; 16], &Op::CancelFeed { feed: 1 })
            .await
            .unwrap();
        assert_eq!(late, Answer::Refused(Refusal::NotFound));
        sim.stop().await;
    });
}

#[test]
fn a_restart_mid_feed_resolves_it_as_uncertain_under_its_own_number() {
    local(async {
        let flash = Flash::memory();
        let sim = start(&flash).await;
        sim.control(Control::StallFeeds(true));
        let mut owner = open(&sim).await;
        let answer = owner.ask([1; 16], &Op::Feed { portions: 2 }).await.unwrap();
        assert_eq!(answer, Answer::Feeding { feed: 1 });
        assert_eq!(
            readiness(&mut owner, 2).await,
            Readiness::Blocked(Blocked::Feeding)
        );

        sim.control(Control::Reboot);
        drop(owner);
        tokio::time::sleep(Duration::from_millis(200)).await;
        let mut owner = open(&sim).await;
        owner.subscribe(None, false).await.unwrap();
        let fed = next_fed(&mut owner).await;
        assert_eq!(
            fed,
            EventBody::Feeder(FeederEvent::Fed {
                feed: 1,
                requested: 2,
                completed: 0,
                outcome: FeedOutcome::Uncertain,
                reason: FeedReason::Interrupted,
                hopper: FoodLevel::Unread,
                by: Source::Feeder,
            })
        );

        // The id the feed went out under is forgotten with the boot, so it
        // is a new feed now, under the next number: never the same one.
        let resent = owner.ask([1; 16], &Op::Feed { portions: 2 }).await.unwrap();
        assert_eq!(resent, Answer::Feeding { feed: 2 });
        assert_eq!(feeding::reserved(&flash), Some(2));
        sim.stop().await;
    });
}

/// The next `Food` event the subscription pushes.
async fn next_food(connection: &mut Connection) -> FoodLevel {
    loop {
        let pushed = connection.next(Duration::from_secs(5)).await.unwrap();
        if let Pushed::Event(event) = pushed
            && let EventBody::Feeder(FeederEvent::Food { level }) = event.body
        {
            return level;
        }
    }
}

async fn food(connection: &mut Connection, id: u8) -> FoodLevel {
    match connection.ask([id; 16], &Op::Status).await.unwrap() {
        Answer::Status(status) => status.feeder().unwrap().food,
        other => panic!("{other:?}"),
    }
}

/// With no feed at all, the boot's food reading shows an empty hopper in the
/// status, and the next, ten minutes on, shows a refill.
#[test]
fn the_food_beam_reads_at_rest_after_a_boot_and_a_refill() {
    paused(async {
        let flash = Flash::memory();
        let sim = start(&flash).await;
        sim.type_line("empty").unwrap();
        let mut owner = open(&sim).await;
        owner.subscribe(None, false).await.unwrap();
        assert_eq!(food(&mut owner, 1).await, FoodLevel::Unread, "just booted");

        sleep(Duration::from_millis(u64::from(FOOD_READ_FIRST_MS) + 100)).await;
        assert_eq!(food(&mut owner, 2).await, FoodLevel::Low);

        // A session lasts 90 s without a request; the reading does not care.
        sim.type_line("fill").unwrap();
        let mut owner = open(&sim).await;
        assert_eq!(food(&mut owner, 3).await, FoodLevel::Low, "not read yet");
        drop(owner);
        sleep(Duration::from_millis(u64::from(FOOD_READ_EVERY_MS))).await;
        let mut owner = open(&sim).await;
        assert_eq!(food(&mut owner, 4).await, FoodLevel::Present);

        sim.stop().await;
    });
}

/// A feed runs whatever the food beam reads, as stock's does: on an empty
/// hopper it still delivers, and its `Fed` event says the hopper read
/// empty. The level shows in the status and changes as one `Food` event
/// after the feed that read it.
#[test]
fn a_feed_on_an_empty_hopper_runs_and_records_that_it_read_empty() {
    local(async {
        let (_flash, sim, mut owner) = subscribed().await;
        assert_eq!(food(&mut owner, 1).await, FoodLevel::Unread, "no feed yet");

        let mut id = 2;
        for (line, level) in [
            (None, FoodLevel::Present),
            (Some("empty"), FoodLevel::Low),
            (Some("fill"), FoodLevel::Present),
        ] {
            if let Some(line) = line {
                sim.type_line(line).unwrap();
            }
            let answer = owner.ask([id; 16], &Op::Feed { portions: 2 }).await;
            assert_matches!(answer, Ok(Answer::Feeding { .. }));
            let EventBody::Feeder(FeederEvent::Fed {
                completed,
                outcome,
                hopper,
                ..
            }) = next_fed(&mut owner).await
            else {
                unreachable!("next_fed returns a feed")
            };
            assert_eq!((completed, outcome, hopper), (2, FeedOutcome::Done, level));
            assert_eq!(next_food(&mut owner).await, level);
            assert_eq!(food(&mut owner, id + 1).await, level);
            id += 2;
        }
        sim.stop().await;
    });
}

async fn feed_fault(connection: &mut Connection, id: u8) -> Option<FeedFault> {
    match connection.ask([id; 16], &Op::Status).await.unwrap() {
        Answer::Status(status) => status.feeder().unwrap().feed_fault,
        other => panic!("{other:?}"),
    }
}

/// An outlet that blocks mid-feed and a jam no reversal frees each end
/// the feed with the portion delivered so far, by name, and never run it
/// again; the status names the fault until a later feed completes.
#[test]
fn a_blocked_outlet_or_a_jam_ends_the_feed_by_name_with_its_portions_so_far() {
    local(async {
        let (flash, sim, mut owner) = subscribed().await;
        assert_eq!(feed_fault(&mut owner, 1).await, None);

        let mut id = 2;
        for (line, reason) in [
            ("block", FeedReason::OutletBlocked),
            ("jam", FeedReason::Jammed),
        ] {
            sim.type_line(line).unwrap();
            let answer = owner.ask([id; 16], &Op::Feed { portions: 3 }).await;
            let Ok(Answer::Feeding { feed }) = answer else {
                panic!("{answer:?}");
            };
            let EventBody::Feeder(FeederEvent::Fed {
                feed: ended,
                requested,
                completed,
                outcome,
                reason: ended_by,
                ..
            }) = next_fed(&mut owner).await
            else {
                unreachable!("next_fed returns a feed")
            };
            assert_eq!((ended, requested, completed), (feed, 3, 1));
            assert_eq!((outcome, ended_by), (FeedOutcome::Failed, reason));
            let fault = FeedFault {
                feed,
                outcome: FeedOutcome::Failed,
                reason,
            };
            assert_eq!(feed_fault(&mut owner, id + 1).await, Some(fault));
            id += 2;
        }

        // Never re-run: the next feed is a new number, and completing it
        // clears the fault.
        let answer = owner.ask([id; 16], &Op::Feed { portions: 1 }).await;
        assert_eq!(answer.unwrap(), Answer::Feeding { feed: 3 });
        let EventBody::Feeder(FeederEvent::Fed { outcome, .. }) = next_fed(&mut owner).await else {
            unreachable!("next_fed returns a feed")
        };
        assert_eq!(outcome, FeedOutcome::Done);
        assert_eq!(feed_fault(&mut owner, id + 1).await, None);
        assert_eq!(feeding::reserved(&flash), Some(3));
        sim.stop().await;
    });
}

/// The feeding history survives a restart: a fresh session after it reads
/// every feed from the event ring, each with who asked and what it
/// delivered, and the feed the restart interrupted as a range, never a
/// count.
#[test]
fn the_feeding_history_reads_every_feed_after_a_restart() {
    local(async {
        let (_flash, sim, mut owner) = subscribed().await;
        let answer = owner.ask([1; 16], &Op::Feed { portions: 2 }).await.unwrap();
        assert_eq!(answer, Answer::Feeding { feed: 1 });
        next_fed(&mut owner).await;

        sim.control(Control::StallFeeds(true));
        let answer = owner.ask([2; 16], &Op::Feed { portions: 3 }).await.unwrap();
        assert_eq!(answer, Answer::Feeding { feed: 2 });
        sim.control(Control::Reboot);
        drop(owner);
        tokio::time::sleep(Duration::from_millis(200)).await;

        let mut owner = open(&sim).await;
        owner.subscribe(None, false).await.unwrap();
        let mut feeds = Vec::new();
        while let Ok(pushed) = owner.next(Duration::from_millis(500)).await {
            match Entry::from_pushed(&pushed) {
                Some(Entry::Feed(feed)) => feeds.push(feed),
                Some(lost @ (Entry::Gap(_) | Entry::Lost { .. })) => {
                    panic!("nothing left the ring or the queue: {lost:?}")
                }
                None => {}
            }
        }

        let read: Vec<_> = feeds
            .iter()
            .map(|feed| (feed.feed, feed.requested, feed.delivered, feed.outcome))
            .collect();
        let interrupted = Delivered::Uncertain {
            confirmed: 0,
            requested: 3,
        };
        assert_eq!(
            read,
            [
                (1, 2, Delivered::Counted(2), FeedOutcome::Done),
                (2, 3, interrupted, FeedOutcome::Uncertain),
            ]
        );
        assert!(matches!(feeds[0].by, Some(Source::Client { .. })));
        // The journal does not keep who asked: unknown, and interrupted.
        assert_eq!(
            (feeds[1].by, feeds[1].reason),
            (None, FeedReason::Interrupted)
        );
        assert!(feeds[0].seq < feeds[1].seq, "oldest first");
        sim.stop().await;
    });
}

/// A feed behind a storage write: the request waits, unanswered, while the
/// loop serves everyone else honestly, and it is answered `Feeding` only once
/// its claim is taken and it is reserved. One feed runs, under one number.
#[test]
fn a_feed_behind_a_write_is_answered_once_it_runs_and_the_loop_serves_meanwhile() {
    local(async {
        let flash = Flash::memory();
        let sim = start(&flash).await;
        let mut owner = open(&sim).await;
        let mut other = open(&sim).await;
        owner.subscribe(None, false).await.unwrap();
        sim.control(Control::HoldWrite(true));

        let feed = owner.ask([1; 16], &Op::Feed { portions: 1 });
        let meanwhile = async {
            sleep(Duration::from_millis(200)).await;

            // Readiness says so, a second feed and a restart are refused, and
            // a change that would queue a write behind it is refused too.
            assert_eq!(
                readiness(&mut other, 2).await,
                Readiness::Blocked(Blocked::Feeding)
            );
            let second = other.ask([3; 16], &Op::Feed { portions: 1 }).await.unwrap();
            assert_eq!(second, Answer::Refused(Refusal::Busy));
            let reboot = other.ask([4; 16], &Op::Reboot).await.unwrap();
            assert_eq!(reboot, Answer::Refused(Refusal::Busy));
            let change = Op::PutSettings {
                base: Generation(0),
                changes: FeederSettingsPatch {
                    volume: Some(21),
                    ..FeederSettingsPatch::default()
                },
            };
            let change = other.ask([5; 16], &change).await.unwrap();
            assert_eq!(change, Answer::Refused(Refusal::Busy));
            assert_eq!(feeding::reserved(&flash), Some(0), "nothing reserved yet");

            sim.control(Control::HoldWrite(false));
        };
        let (answer, ()) = tokio::join!(feed, meanwhile);

        assert_eq!(answer.unwrap(), Answer::Feeding { feed: 1 });
        assert_eq!(feeding::reserved(&flash), Some(1));
        let again = owner.ask([1; 16], &Op::Feed { portions: 1 }).await.unwrap();
        assert_eq!(again, Answer::Feeding { feed: 1 }, "the same feed");
        let EventBody::Feeder(FeederEvent::Fed {
            feed, requested, ..
        }) = next_fed(&mut owner).await
        else {
            panic!("a feed");
        };
        assert_eq!((feed, requested), (1, 1));
        assert_eq!(feeding::reserved(&flash), Some(1), "and no other");
        sim.stop().await;
    });
}

/// A write that outlasts the wait refuses the feed `Busy`, once. Nothing moved,
/// the retry of that id still hears `Busy`, though the next feed has the number
/// it was promised, and the feeder is ready again.
#[test]
fn a_feed_past_its_claim_s_deadline_is_refused_and_its_id_stays_refused() {
    local(async {
        let (flash, sim, mut owner) = subscribed().await;
        sim.control(Control::HoldWrite(true));

        let begun = std::time::Instant::now();
        let answer = owner.ask([1; 16], &Op::Feed { portions: 1 }).await.unwrap();
        assert_eq!(answer, Answer::Refused(Refusal::Busy));
        assert!(begun.elapsed() >= Duration::from_millis(2_900), "its wait");
        assert_eq!(feeding::reserved(&flash), Some(0), "nothing reserved");
        assert_eq!(readiness(&mut owner, 2).await, Readiness::Ready);
        sim.control(Control::HoldWrite(false));

        // The next feed takes the number the refused one was given.
        let next = owner.ask([3; 16], &Op::Feed { portions: 1 }).await.unwrap();
        assert_eq!(next, Answer::Feeding { feed: 1 });
        let again = owner.ask([1; 16], &Op::Feed { portions: 1 }).await.unwrap();
        assert_eq!(again, Answer::Refused(Refusal::Busy), "the first answer");
        next_fed(&mut owner).await;
        assert_eq!(feeding::reserved(&flash), Some(1));
        sim.stop().await;
    });
}

/// A client whose session ended while its feed waited hears the truth on a new
/// session: the refusal, even though the number now names another feed that
/// is running, and never that feed's `Feeding` as its own.
#[test]
fn a_client_gone_while_its_claim_was_refused_hears_the_refusal_on_a_retry() {
    local(async {
        let flash = Flash::memory();
        let sim = start(&flash).await;
        sim.control(Control::HoldWrite(true));
        let mut gone = open(&sim).await;
        gone.request([1; 16], &Op::Feed { portions: 1 }).unwrap();
        let unanswered = gone.receive(Duration::from_millis(100)).await;
        assert!(unanswered.is_err(), "{unanswered:?}");
        drop(gone);

        // Past the deadline, with the number free again.
        sleep(Duration::from_millis(u64::from(WRITE_WAIT_MS) + 100)).await;
        sim.control(Control::HoldWrite(false));
        sim.control(Control::StallFeeds(true));
        let mut owner = open(&sim).await;
        let next = owner.ask([2; 16], &Op::Feed { portions: 1 }).await.unwrap();
        assert_eq!(next, Answer::Feeding { feed: 1 });

        let retry = owner.ask([1; 16], &Op::Feed { portions: 1 }).await.unwrap();
        assert_eq!(retry, Answer::Refused(Refusal::Busy));
        sim.control(Control::StallFeeds(false));
        sim.stop().await;
    });
}

/// A client whose session ended while its feed waited, and whose feed then ran,
/// hears `Feeding` on a new session, as for a feed queued before its reservation.
#[test]
fn a_client_gone_while_its_claim_waited_hears_its_feed_on_a_retry() {
    local(async {
        let flash = Flash::memory();
        let sim = start(&flash).await;
        sim.control(Control::HoldWrite(true));
        let mut gone = open(&sim).await;
        gone.request([1; 16], &Op::Feed { portions: 1 }).unwrap();
        let unanswered = gone.receive(Duration::from_millis(100)).await;
        assert!(unanswered.is_err(), "{unanswered:?}");
        drop(gone);
        sim.control(Control::HoldWrite(false));

        let mut owner = open(&sim).await;
        let retry = owner.ask([1; 16], &Op::Feed { portions: 1 }).await.unwrap();
        assert_eq!(retry, Answer::Feeding { feed: 1 });
        assert_eq!(feeding::reserved(&flash), Some(1), "one feed");
        sim.stop().await;
    });
}

/// A cancel naming a feed still waiting for its claim finds nothing running:
/// `NotFound`, and the feed goes on to run as if nobody had asked, once.
#[test]
fn a_cancel_of_a_feed_waiting_for_its_claim_is_not_found_and_the_feed_runs() {
    local(async {
        let flash = Flash::memory();
        let sim = start(&flash).await;
        let mut owner = open(&sim).await;
        let mut other = open(&sim).await;
        owner.subscribe(None, false).await.unwrap();
        sim.control(Control::HoldWrite(true));

        let feed = owner.ask([1; 16], &Op::Feed { portions: 2 });
        let cancel = async {
            sleep(Duration::from_millis(100)).await;
            let refused = other
                .ask([2; 16], &Op::CancelFeed { feed: 1 })
                .await
                .unwrap();
            assert_eq!(refused, Answer::Refused(Refusal::NotFound));
            sim.control(Control::HoldWrite(false));
        };
        let (answer, ()) = tokio::join!(feed, cancel);
        assert_eq!(answer.unwrap(), Answer::Feeding { feed: 1 });

        let EventBody::Feeder(FeederEvent::Fed {
            feed,
            completed,
            outcome,
            ..
        }) = next_fed(&mut owner).await
        else {
            panic!("a feed");
        };
        assert_eq!((feed, completed, outcome), (1, 2, FeedOutcome::Done));
        assert_eq!(feeding::reserved(&flash), Some(1), "one feed");
        assert_eq!(readiness(&mut other, 3).await, Readiness::Ready);
        sim.stop().await;
    });
}
