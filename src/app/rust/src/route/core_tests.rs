use std::collections::HashMap;

use proptest::prelude::*;
use wellfed_client::history::Delivered;
use wellfed_model::{FeedOutcome, FeedReason, FoodLevel, Hex, Source};

use super::*;
use crate::api::feeder::FeederRefusal;
use crate::api::router::{NotSentReason, WriteKind};
use crate::api::store::StoreKind;

const NOW: i64 = 1_000_000;

fn feed_pin(key: u128, feeder: &str) -> PinRecord {
    PinRecord {
        key: Uuid::from_u128(key),
        feeder: feeder.into(),
        write: PinnedWrite::Feed { portions: 1 },
        state: PinState::Sending,
        sent_ms: NOW,
        mark: Some(10),
        excluded: Vec::new(),
        acknowledged_ms: None,
    }
}

fn lock_pin(key: u128, feeder: &str) -> PinRecord {
    PinRecord {
        write: PinnedWrite::Other {
            kind: WriteKind::Lock,
        },
        ..feed_pin(key, feeder)
    }
}

/// This phone's key's first 4 bytes, as events name it.
const PHONE: [u8; 4] = [5, 6, 7, 8];

/// This phone's one-portion feed numbered `feed`, ended at event `seq`.
fn seen(seq: u64, feed: u32) -> Feed {
    Feed {
        epoch: 1,
        seq,
        feed,
        at: None,
        by: Some(Source::Client { client: Hex(PHONE) }),
        requested: 1,
        delivered: Delivered::Counted(1),
        outcome: FeedOutcome::Done,
        reason: FeedReason::Done,
        hopper: FoodLevel::Present,
    }
}

/// An uncertain feed's pin.
fn lost_pin(key: u128, feeder: &str) -> PinRecord {
    PinRecord {
        state: PinState::Uncertain,
        ..feed_pin(key, feeder)
    }
}

/// What `seen` settles of `pins` on feeder "a", by key.
fn by_history(pins: &[PinRecord], seen: &[Feed]) -> Vec<(u128, Settlement)> {
    settled_by_history(pins, "a", seen, PHONE)
        .into_iter()
        .map(|(key, settlement)| (key.as_u128(), settlement))
        .collect()
}

/// The feed numbers `seen` settles `pins` with, by key.
fn numbers(pins: &[PinRecord], seen: &[Feed]) -> Vec<(u128, u32)> {
    by_history(pins, seen)
        .into_iter()
        .filter_map(|(key, settlement)| match settlement {
            Settlement::Fed { feed, .. } => Some((key, feed)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_feed_settles_by_this_phone_s_one_fed_after_its_mark() {
    let pin = lost_pin(1, "a");
    // Only events after the mark count.
    assert_eq!(by_history(std::slice::from_ref(&pin), &[seen(9, 1)]), []);
    assert_eq!(
        by_history(std::slice::from_ref(&pin), &[seen(9, 1), seen(12, 2)]),
        [(
            1,
            Settlement::Fed {
                feed: 2,
                requested: 1,
                completed: 1,
                outcome: FeedOutcome::Done,
                reason: FeedReason::Done,
            }
        )]
    );
    // Two candidates: neither is taken for it.
    assert_eq!(
        numbers(std::slice::from_ref(&pin), &[seen(12, 2), seen(14, 3)]),
        []
    );
    // Another feed's portions are another feed's end.
    let two = Feed {
        requested: 2,
        ..seen(12, 2)
    };
    assert_eq!(numbers(std::slice::from_ref(&pin), &[two]), []);
    // Another client's feed is not this phone's.
    let theirs = Feed {
        by: Some(Source::Client {
            client: Hex([1; 4]),
        }),
        ..seen(12, 2)
    };
    assert_eq!(numbers(std::slice::from_ref(&pin), &[theirs]), []);
    // One this phone saw started is another feed's end.
    let excluded = PinRecord {
        excluded: vec![3],
        ..pin.clone()
    };
    assert_eq!(numbers(&[excluded], &[seen(12, 2), seen(14, 3)]), [(1, 2)]);
    // No mark: the session knew nothing when it went, so nothing settles it.
    let unmarked = PinRecord {
        mark: None,
        ..pin.clone()
    };
    assert_eq!(numbers(&[unmarked], &[seen(12, 2)]), []);
    // Only a feed is the feeder history's to settle.
    let lock = PinRecord {
        state: PinState::Uncertain,
        ..lock_pin(1, "a")
    };
    assert_eq!(numbers(&[lock], &[seen(12, 2)]), []);
    // A pin still sending is not settled here: its answer settles it.
    let sending = PinRecord {
        state: PinState::Sending,
        ..pin
    };
    assert_eq!(numbers(&[sending], &[seen(12, 2)]), []);
}

/// An uncertain feed's end given as a range keeps only the portions it
/// confirmed as its count.
#[test]
fn an_uncertain_end_settles_with_the_portions_it_confirmed() {
    let uncertain = Feed {
        requested: 1,
        delivered: Delivered::Uncertain {
            confirmed: 0,
            requested: 1,
        },
        outcome: FeedOutcome::Uncertain,
        reason: FeedReason::Interrupted,
        ..seen(12, 2)
    };
    assert_eq!(
        by_history(&[lost_pin(1, "a")], &[uncertain]),
        [(
            1,
            Settlement::Fed {
                feed: 2,
                requested: 1,
                completed: 0,
                outcome: FeedOutcome::Uncertain,
                reason: FeedReason::Interrupted,
            }
        )]
    );
}

#[test]
fn an_end_another_feed_could_claim_settles_neither() {
    let first = lost_pin(1, "a");
    let second = PinRecord {
        mark: Some(13),
        ..lost_pin(2, "a")
    };
    let both = [first.clone(), second.clone()];

    // Between the two marks: only the first's.
    assert_eq!(numbers(&both, &[seen(12, 2)]), [(1, 2)]);
    // After both: either's.
    assert_eq!(numbers(&both, &[seen(15, 2)]), []);
    // A feed still waiting on its answer may yet be lost: it claims too.
    let sending = PinRecord {
        state: PinState::Sending,
        ..second
    };
    assert_eq!(numbers(&[first.clone(), sending], &[seen(15, 2)]), []);
    // Another feeder's pin claims nothing here.
    let elsewhere = PinRecord {
        mark: Some(13),
        ..lost_pin(3, "b")
    };
    assert_eq!(numbers(&[first, elsewhere], &[seen(15, 2)]), [(1, 2)]);
}

/// Two uncertain feeds of 2 and 1 portions; only the second ran. Its `Fed`
/// settles the feed of its portions, never the older one.
#[test]
fn of_two_uncertain_feeds_a_fed_settles_the_one_of_its_portions() {
    let two = PinRecord {
        write: PinnedWrite::Feed { portions: 2 },
        ..lost_pin(1, "a")
    };
    let one = PinRecord {
        mark: Some(12),
        ..lost_pin(2, "a")
    };
    assert_eq!(numbers(&[two, one], &[seen(15, 7)]), [(2, 7)]);
}

#[test]
fn a_feed_waits_for_one_unanswered_or_uncertain_until_the_owner_puts_it_away() {
    let mut book = Book::default();
    book.admit(feed_pin(1, "a"), NOW).unwrap();

    // Still sending.
    let held = book.admit(feed_pin(2, "a"), NOW).unwrap_err();
    assert_eq!(held, Held::By(Uuid::from_u128(1)));
    // Another feeder's feed goes.
    book.admit(feed_pin(3, "b"), NOW).unwrap();

    // Uncertain.
    assert!(book.ended(Uuid::from_u128(1), Ended::Uncertain));
    assert_eq!(
        book.admit(feed_pin(4, "a"), NOW),
        Err(Held::By(Uuid::from_u128(1)))
    );

    // Put away by the owner: the next goes, and the old one stays pinned.
    assert!(book.acknowledge(Uuid::from_u128(1), NOW));
    assert!(!book.acknowledge(Uuid::from_u128(1), NOW), "once");
    book.admit(feed_pin(4, "a"), NOW).unwrap();
    assert!(
        book.get(Uuid::from_u128(1)).is_some(),
        "put away, it stays pinned"
    );
}

#[test]
fn a_set_aside_pins_file_holds_every_feed_back_until_the_bowls_are_checked() {
    let mut book = Book {
        set_aside: Some(SetAside {
            kind: StoreKind::Pending,
            moved_to: "pending.json.damaged-1".into(),
            reason: "not this file's shape".into(),
            at_ms: NOW,
        }),
        ..Book::default()
    };

    for feeder in ["a", "b"] {
        assert_eq!(book.admit(feed_pin(1, feeder), NOW), Err(Held::SetAside));
    }
    book.admit(lock_pin(2, "a"), NOW).unwrap();

    assert!(book.bowls_checked());
    assert!(!book.bowls_checked(), "once");
    book.admit(feed_pin(3, "a"), NOW).unwrap();
}

#[test]
fn a_write_other_than_a_feed_is_never_held_back_and_holds_no_feed_back() {
    let mut book = Book::default();
    book.admit(feed_pin(1, "a"), NOW).unwrap();
    book.admit(lock_pin(2, "a"), NOW).unwrap();

    book.settle(Uuid::from_u128(1));
    book.admit(feed_pin(3, "a"), NOW).unwrap();
}

#[test]
fn an_uncertain_pin_keeps_the_owner_s_acknowledgement() {
    let mut book = Book::default();
    book.admit(feed_pin(1, "a"), NOW).unwrap();
    assert!(book.ended(Uuid::from_u128(1), Ended::Uncertain));
    assert!(book.acknowledge(Uuid::from_u128(1), NOW));

    // Ending uncertain again keeps it put away.
    assert!(book.ended(Uuid::from_u128(1), Ended::Uncertain));
    let pin = book.get(Uuid::from_u128(1)).unwrap();
    assert_eq!(pin.acknowledged_ms, Some(NOW));
}

#[test]
fn any_end_but_uncertain_takes_the_pin_out() {
    for ended in [Ended::Answered, Ended::Refused, Ended::Unsent] {
        let mut book = Book::default();
        book.admit(feed_pin(1, "a"), NOW).unwrap();
        assert!(book.ended(Uuid::from_u128(1), ended));
        assert!(book.pins.is_empty());
        book.admit(feed_pin(2, "a"), NOW).unwrap();
    }
}

#[test]
fn putting_away_a_write_nothing_can_settle_takes_it_out() {
    let mut book = Book::default();
    let lock = PinRecord {
        state: PinState::Uncertain,
        ..lock_pin(1, "a")
    };
    let unmarked = PinRecord {
        state: PinState::Uncertain,
        mark: None,
        ..feed_pin(2, "b")
    };
    let marked = lost_pin(3, "c");
    book.pins = vec![lock, unmarked, marked];

    let at = NOW + 1_000;
    assert!(book.acknowledge(Uuid::from_u128(1), at));
    assert!(book.acknowledge(Uuid::from_u128(2), at));
    assert!(book.acknowledge(Uuid::from_u128(3), at));
    let left: Vec<Uuid> = book.pins.iter().map(|pin| pin.key).collect();
    assert_eq!(left, [Uuid::from_u128(3)], "the history may settle it");

    // A week from the acknowledgement, not from the send.
    assert!(!book.expire(at + KEEP_MS - 1));
    assert!(book.expire(at + KEEP_MS));
    assert!(book.pins.is_empty());
}

#[test]
fn a_feed_seen_started_is_no_uncertain_one_s_end_then_or_later() {
    let mut book = Book::default();
    book.admit(feed_pin(1, "a"), NOW).unwrap();
    assert!(book.ended(Uuid::from_u128(1), Ended::Uncertain));
    assert!(book.acknowledge(Uuid::from_u128(1), NOW));
    book.admit(feed_pin(2, "b"), NOW).unwrap();

    book.started("a", 7, NOW + 60_000);
    assert_eq!(book.get(Uuid::from_u128(1)).unwrap().excluded, [7]);
    assert!(book.get(Uuid::from_u128(2)).unwrap().excluded.is_empty());

    // A feed sent while 7 runs takes no end from it either.
    book.admit(feed_pin(3, "a"), NOW + 1).unwrap();
    assert_eq!(book.get(Uuid::from_u128(3)).unwrap().excluded, [7]);
    assert!(book.ended_running("a", &[7]));
    assert!(!book.ended_running("a", &[7]), "once");
    assert!(book.running_on("a", NOW + 2).is_empty());

    // A running feed past the time it is kept no longer holds.
    book.started("a", 8, NOW + 10);
    assert_eq!(book.running_on("a", NOW + 9), [8]);
    assert!(book.expire(NOW + 10));
    assert!(book.running.is_empty());
}

/// How one send ends, in the world and as the phone hears it.
#[derive(Clone, Copy, Debug)]
enum Sent {
    /// It ran and the phone heard so.
    Answered,
    /// It ran, and the answer was lost before it reached the phone.
    LostAfterRunning,
    /// It never ran, but the phone heard nothing: the session dropped
    /// before the feeder read it.
    LostBeforeRunning,
    /// It never ran, and the phone knows: the session was closed.
    Unsent,
    Refused,
}

/// One step a phone can take with one feeder.
#[derive(Clone, Copy, Debug)]
enum Step {
    Feed(Sent),
    /// Read the history for the oldest pending write.
    Retry,
    Acknowledge,
    /// The app stops and starts again.
    Restart,
}

fn sent() -> impl Strategy<Value = Sent> {
    prop_oneof![
        Just(Sent::Answered),
        Just(Sent::LostAfterRunning),
        Just(Sent::LostBeforeRunning),
        Just(Sent::Unsent),
        Just(Sent::Refused),
    ]
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => sent().prop_map(Step::Feed),
        3 => Just(Step::Retry),
        2 => Just(Step::Acknowledge),
        1 => Just(Step::Restart),
    ]
}

/// What really happened, of which the phone sees only part.
#[derive(Default)]
struct World {
    /// Feeds the feeder dispensed, by key, under the feed number it gave.
    ran: HashMap<Uuid, u32>,
    /// Every send of each key.
    sends: HashMap<Uuid, u32>,
    /// The history's next event.
    seq: u64,
    /// This phone's feed ends, as the history holds them.
    history: Vec<Feed>,
}

impl World {
    /// Send `key`, ending `how`: what the phone hears.
    fn send(&mut self, key: Uuid, how: Sent) -> Ended {
        let (ran, heard) = match how {
            Sent::Answered => (true, Ended::Answered),
            Sent::LostAfterRunning => (true, Ended::Uncertain),
            Sent::LostBeforeRunning => (false, Ended::Uncertain),
            Sent::Unsent => (false, Ended::Unsent),
            Sent::Refused => (false, Ended::Refused),
        };
        if heard != Ended::Unsent {
            *self.sends.entry(key).or_default() += 1;
        }
        if ran {
            self.seq += 1;
            let feed = u32::try_from(self.seq).unwrap_or(u32::MAX);
            self.ran.insert(key, feed);
            self.history.push(seen(self.seq + 10, feed));
        }
        heard
    }
}

proptest! {
    /// Whatever the session does and whatever the owner does after (put a
    /// feed away, feed again, read the history), each key is sent once, the
    /// feeder dispenses no key twice, a feed is settled only by the end of
    /// the feed it really was, and no feed starts while another on the
    /// feeder is unanswered or uncertain and not put away.
    #[test]
    fn an_uncertain_feed_is_never_sent_again_and_holds_the_next(steps in prop::collection::vec(step(), 1..60)) {
        let mut book = Book::default();
        let mut world = World::default();
        let mut next_key = 1u128;

        for step in steps {
            match step {
                Step::Feed(how) => {
                    let key = next_key;
                    next_key += 1;
                    let holding = book.holding_feeds("a").is_some();
                    let pin = PinRecord {
                        mark: Some(world.seq + 10),
                        ..feed_pin(key, "a")
                    };
                    let admitted = book.admit(pin, NOW);
                    prop_assert_eq!(admitted.is_err(), holding);
                    if admitted.is_err() {
                        continue;
                    }
                    let key = Uuid::from_u128(key);
                    let ended = world.send(key, how);
                    if ended == Ended::Answered
                        && let Some(feed) = world.ran.get(&key)
                    {
                        book.started("a", *feed, NOW + KEEP_MS);
                    }
                    book.ended(key, ended);
                }
                Step::Retry => {
                    for (key, settlement) in settled_by_history(&book.pins, "a", &world.history, PHONE) {
                        let Settlement::Fed { feed, .. } = settlement else {
                            return Err(TestCaseError::fail("only a feed's end settles from history"));
                        };
                        prop_assert_eq!(world.ran.get(&key), Some(&feed), "{} took another's end", key);
                        book.settle(key);
                    }
                }
                Step::Acknowledge => {
                    if let Some(key) = book.pins.first().map(|pin| pin.key) {
                        book.acknowledge(key, NOW);
                    }
                }
                Step::Restart => {
                    for pin in &mut book.pins {
                        pin.state = PinState::Uncertain;
                    }
                }
            }

            for (key, count) in &world.sends {
                prop_assert!(*count <= 1, "{key} sent {count} times");
            }
        }
    }
}

/// A write's pin goes only on an answer that names its end: one this build
/// can't read, or the wrong one, may follow a write that ran, so its pin
/// stays, uncertain, through a restart.
#[test]
fn a_write_stays_uncertain_unless_its_answer_names_its_end() {
    for reply in [
        FeederReply::Unanswered,
        FeederReply::Unreadable,
        FeederReply::Unexpected,
    ] {
        assert_eq!(direct_ended(&reply), Ended::Uncertain, "{reply:?}");

        let mut book = Book::default();
        let pin = PinRecord {
            write: PinnedWrite::Other {
                kind: WriteKind::Settings,
            },
            ..feed_pin(1, "a")
        };
        book.admit(pin, NOW).unwrap();
        book.ended(Uuid::from_u128(1), direct_ended(&reply));
        assert_eq!(book.pins.len(), 1, "{reply:?}");
        assert_eq!(book.pins[0].state, PinState::Uncertain);
    }

    assert_eq!(
        direct_ended(&FeederReply::Done { generation: None }),
        Ended::Answered
    );
    assert_eq!(
        direct_ended(&FeederReply::Refused {
            refusal: FeederRefusal::Busy
        }),
        Ended::Refused
    );
    assert_eq!(
        direct_ended(&FeederReply::NotAsked {
            reason: NotAsked::BadPlan
        }),
        Ended::Refused
    );
    for unsent in [
        FeederReply::NotSent {
            reason: NotSentReason::Session,
        },
        FeederReply::NotAsked {
            reason: NotAsked::Held,
        },
    ] {
        assert_eq!(direct_ended(&unsent), Ended::Unsent);
    }
}
