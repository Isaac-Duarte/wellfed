use proptest::prelude::*;
use wellfed_api::FeederRefusal;
use wellfed_model::{FeedOutcome, FeedReason};

use super::*;
use crate::api::server::ServerRefusal;

const NOW: i64 = 1_000_000;

fn reach(server: ServerReach, online: Option<bool>, direct_open: bool) -> Reach {
    Reach {
        server,
        online_on_server: online,
        direct_open,
    }
}

fn feed_pin(key: u128, feeder: &str, path: RoutePath) -> PinRecord {
    PinRecord {
        key: Uuid::from_u128(key),
        feeder: feeder.into(),
        server_feeder: None,
        path,
        write: PinnedWrite::Feed { portions: 1 },
        state: PinState::Sending,
        sent_ms: NOW,
        tried_ms: NOW,
        mark: Some(10),
        excluded: Vec::new(),
        acknowledged: false,
    }
}

fn seen(seq: u64, feed: u32) -> Seen {
    Seen {
        seq,
        feed,
        requested: 1,
        completed: 1,
        outcome: FeedOutcome::Done,
        reason: FeedReason::Done,
    }
}

#[test]
fn the_server_goes_first_while_it_answers_and_holds_the_feeder() {
    let up = ServerReach::Up;
    assert_eq!(
        choose(Access::Both, reach(up, Some(true), true), NOW),
        RoutePath::Server
    );
    // Before the server has said anything of its link: still the server.
    assert_eq!(
        choose(Access::Both, reach(up, None, true), NOW),
        RoutePath::Server
    );
}

#[test]
fn a_down_server_or_a_feeder_it_reports_offline_goes_direct_when_the_phone_reaches_it() {
    let down = ServerReach::Down { since_ms: NOW };
    assert_eq!(
        choose(Access::Both, reach(down, Some(true), true), NOW),
        RoutePath::Direct
    );
    assert_eq!(
        choose(Access::Both, reach(ServerReach::Up, Some(false), true), NOW),
        RoutePath::Direct
    );
    // Neither works: the server's failure says why.
    assert_eq!(
        choose(Access::Both, reach(down, Some(false), false), NOW),
        RoutePath::Server
    );
}

#[test]
fn a_down_server_is_tried_again_after_a_while() {
    let down = ServerReach::Down { since_ms: NOW };
    assert_eq!(
        choose(Access::Both, reach(down, None, true), NOW + RECHECK_MS - 1),
        RoutePath::Direct
    );
    assert_eq!(
        choose(Access::Both, reach(down, None, true), NOW + RECHECK_MS),
        RoutePath::Server
    );
}

#[test]
fn a_viewer_or_an_expiring_account_goes_through_the_server_only() {
    assert_eq!(
        Access::of(true, true, Keys::ServerOnly),
        Some(Access::ServerOnly)
    );
    let down = ServerReach::Down { since_ms: NOW };
    assert_eq!(
        choose(Access::ServerOnly, reach(down, Some(false), true), NOW),
        RoutePath::Server
    );
    assert_eq!(Access::of(true, true, Keys::Own), Some(Access::Both));
    assert_eq!(Access::of(false, true, Keys::Own), Some(Access::DirectOnly));
    assert_eq!(Access::of(false, false, Keys::Own), None);
}

#[test]
fn only_a_definite_non_send_on_the_server_goes_direct_and_only_once() {
    assert_eq!(
        fallback(RoutePath::Server, Ended::Unsent, Access::Both, true, false),
        Some(RoutePath::Direct)
    );
    for ended in [
        Ended::Uncertain,
        Ended::Answered,
        Ended::Refused,
        Ended::Superseded,
    ] {
        assert_eq!(
            fallback(RoutePath::Server, ended, Access::Both, true, false),
            None
        );
    }
    assert_eq!(
        fallback(RoutePath::Server, Ended::Unsent, Access::Both, true, true),
        None
    );
    assert_eq!(
        fallback(RoutePath::Server, Ended::Unsent, Access::Both, false, false),
        None
    );
    assert_eq!(
        fallback(
            RoutePath::Server,
            Ended::Unsent,
            Access::ServerOnly,
            true,
            false
        ),
        None
    );
    // The direct path never falls back to the server.
    assert_eq!(
        fallback(RoutePath::Direct, Ended::Unsent, Access::Both, true, false),
        None
    );
}

#[test]
fn a_first_attempt_the_server_never_sent_is_unsent_and_a_retry_s_says_nothing() {
    use ServerFailure as F;
    assert_eq!(server_ended(F::Connect, Attempt::First), Ended::Unsent);
    assert_eq!(
        server_ended(F::FeederOffline, Attempt::First),
        Ended::Unsent
    );
    assert_eq!(server_ended(F::Rejected, Attempt::First), Ended::Refused);
    assert_eq!(server_ended(F::KeyReused, Attempt::First), Ended::Refused);
    assert_eq!(server_ended(F::Stale, Attempt::First), Ended::Refused);
    for failure in [F::Interrupted, F::OutcomeUnknown] {
        assert_eq!(server_ended(failure, Attempt::First), Ended::Uncertain);
        assert_eq!(server_ended(failure, Attempt::Retry), Ended::Uncertain);
    }
    for failure in [F::Connect, F::FeederOffline, F::Rejected, F::KeyReused] {
        assert_eq!(server_ended(failure, Attempt::Retry), Ended::Uncertain);
    }
    // A refusal is final for the key, and the server replays it.
    assert_eq!(
        server_ended(F::FeederRefused, Attempt::Retry),
        Ended::Refused
    );
    assert_eq!(server_ended(F::Stale, Attempt::Retry), Ended::Superseded);
    assert!(server_down(F::Connect));
    assert!(!server_down(F::FeederOffline));
}

#[test]
fn the_journal_settles_a_write_once_it_is_answered_or_resolved() {
    assert_eq!(settled_by_command(CommandState::Unknown, None), None);
    assert_eq!(
        settled_by_command(CommandState::Sent, Some(&CommandOutcome::Done)),
        None
    );
    assert_eq!(
        settled_by_command(
            CommandState::Answered,
            Some(&CommandOutcome::Feeding { feed: 4 })
        ),
        Some(Settlement::Started { feed: 4 })
    );
    let fed = CommandOutcome::Fed {
        feed: 4,
        requested: 2,
        completed: 1,
        outcome: FeedOutcome::Uncertain,
        reason: FeedReason::Interrupted,
    };
    assert_eq!(
        settled_by_command(CommandState::Resolved, Some(&fed)),
        Some(Settlement::Fed {
            feed: 4,
            requested: 2,
            completed: 1,
            outcome: FeedOutcome::Uncertain,
            reason: FeedReason::Interrupted,
        })
    );
    assert_eq!(
        settled_by_command(CommandState::Resolved, Some(&CommandOutcome::Unrecorded)),
        Some(Settlement::Unrecorded)
    );
    assert_eq!(
        settled_by_command(
            CommandState::Answered,
            Some(&CommandOutcome::Refused {
                refusal: FeederRefusal::Busy
            })
        ),
        Some(Settlement::Refused {
            refusal: ServerRefusal::Busy
        })
    );
}

#[test]
fn a_server_write_the_journal_lacks_is_unsent_only_once_its_attempt_is_old() {
    let pin = feed_pin(1, "a", RoutePath::Server);
    assert_eq!(settled_unfound(&pin, NOW + UNFOUND_GRACE_MS - 1), None);
    assert_eq!(
        settled_unfound(&pin, NOW + UNFOUND_GRACE_MS),
        Some(Settlement::NotSent)
    );
}

#[test]
fn a_direct_feed_settles_by_this_phone_s_one_fed_after_its_mark() {
    let pin = feed_pin(1, "a", RoutePath::Direct);
    // Only events after the mark count.
    assert_eq!(settled_by_history(&pin, &[], &[seen(9, 1)]), None);
    assert_eq!(
        settled_by_history(&pin, &[], &[seen(9, 1), seen(12, 2)]),
        Some(Settlement::Fed {
            feed: 2,
            requested: 1,
            completed: 1,
            outcome: FeedOutcome::Done,
            reason: FeedReason::Done,
        })
    );
    // Two candidates: neither is taken for it.
    assert_eq!(
        settled_by_history(&pin, &[], &[seen(12, 2), seen(14, 3)]),
        None
    );
    // One this phone heard answered since is another feed's end.
    let excluded = PinRecord {
        excluded: vec![3],
        ..pin.clone()
    };
    assert!(settled_by_history(&excluded, &[], &[seen(12, 2), seen(14, 3)]).is_some());
    // No mark: the session knew nothing when it went, so nothing settles it.
    let unmarked = PinRecord {
        mark: None,
        ..pin.clone()
    };
    assert_eq!(settled_by_history(&unmarked, &[], &[seen(12, 2)]), None);
    // A server pin is not the feeder history's to settle.
    let settings = PinRecord {
        write: PinnedWrite::Lock { locked: true },
        ..pin
    };
    assert_eq!(settled_by_history(&settings, &[], &[seen(12, 2)]), None);
}

#[test]
fn an_end_another_uncertain_direct_feed_could_claim_settles_neither() {
    let first = feed_pin(1, "a", RoutePath::Direct);
    let second = PinRecord {
        mark: Some(13),
        ..feed_pin(2, "a", RoutePath::Direct)
    };
    let others = [&first, &second];

    // Between the two marks: only the first's.
    assert!(settled_by_history(&first, &others, &[seen(12, 2)]).is_some());
    // After both: either's.
    assert_eq!(settled_by_history(&first, &others, &[seen(15, 2)]), None);
    assert_eq!(settled_by_history(&second, &others, &[seen(15, 2)]), None);
    // Another feeder's pin claims nothing here.
    let elsewhere = PinRecord {
        mark: Some(13),
        ..feed_pin(3, "b", RoutePath::Direct)
    };
    assert!(settled_by_history(&first, &[&first, &elsewhere], &[seen(15, 2)]).is_some());
}

#[test]
fn a_feed_waits_for_one_unanswered_or_uncertain_until_the_owner_puts_it_away() {
    let mut book = Book::default();
    book.admit(feed_pin(1, "a", RoutePath::Server)).unwrap();

    // Still sending, on either path.
    let held = book.admit(feed_pin(2, "a", RoutePath::Direct)).unwrap_err();
    assert_eq!(held, Uuid::from_u128(1));
    // Another feeder's feed goes.
    book.admit(feed_pin(3, "b", RoutePath::Direct)).unwrap();

    assert!(book.ended(feed_pin(1, "a", RoutePath::Server), Ended::Uncertain));
    assert!(book.admit(feed_pin(4, "a", RoutePath::Direct)).is_err());
    assert!(book.acknowledge(Uuid::from_u128(1)));
    assert!(!book.acknowledge(Uuid::from_u128(1)), "once");
    book.admit(feed_pin(4, "a", RoutePath::Direct)).unwrap();
    assert_eq!(
        book.get(Uuid::from_u128(1)).map(|pin| pin.path),
        Some(RoutePath::Server),
        "put away, it stays pinned"
    );
}

#[test]
fn an_uncertain_pin_keeps_its_path_and_the_owner_s_acknowledgement() {
    let mut book = Book::default();
    book.admit(feed_pin(1, "a", RoutePath::Server)).unwrap();
    assert!(book.ended(feed_pin(1, "a", RoutePath::Server), Ended::Uncertain));
    assert!(book.acknowledge(Uuid::from_u128(1)));

    // A retry that ends uncertain again, as told from any path, changes
    // only its last attempt.
    let retried = PinRecord {
        tried_ms: NOW + 5,
        ..feed_pin(1, "a", RoutePath::Direct)
    };
    assert!(book.ended(retried, Ended::Uncertain));
    let pin = book.get(Uuid::from_u128(1)).unwrap();
    assert_eq!(
        (pin.path, pin.acknowledged, pin.tried_ms),
        (RoutePath::Server, true, NOW + 5)
    );
    // And an uncertain pin never moves to the other path.
    assert!(!book.moved(Uuid::from_u128(1), Some(3), NOW));
}

#[test]
fn a_feed_the_server_never_sent_moves_while_still_sending() {
    let mut book = Book::default();
    book.admit(feed_pin(1, "a", RoutePath::Server)).unwrap();
    assert!(book.moved(Uuid::from_u128(1), Some(21), NOW + 1));
    let pin = book.get(Uuid::from_u128(1)).unwrap();
    assert_eq!((pin.path, pin.mark), (RoutePath::Direct, Some(21)));
    assert!(!book.moved(Uuid::from_u128(1), Some(22), NOW + 2), "once");
}

#[test]
fn any_end_but_uncertain_takes_the_pin_out() {
    for ended in [
        Ended::Answered,
        Ended::Refused,
        Ended::Unsent,
        Ended::Superseded,
    ] {
        let mut book = Book::default();
        book.admit(feed_pin(1, "a", RoutePath::Server)).unwrap();
        assert!(book.ended(feed_pin(1, "a", RoutePath::Server), ended));
        assert!(book.pins.is_empty());
        book.admit(feed_pin(2, "a", RoutePath::Server)).unwrap();
    }
}

#[test]
fn putting_away_a_write_nothing_can_settle_takes_it_out() {
    let mut book = Book::default();
    let lock = PinRecord {
        write: PinnedWrite::Lock { locked: true },
        state: PinState::Uncertain,
        ..feed_pin(1, "a", RoutePath::Direct)
    };
    let unmarked = PinRecord {
        state: PinState::Uncertain,
        mark: None,
        ..feed_pin(2, "b", RoutePath::Direct)
    };
    let server_lock = PinRecord {
        path: RoutePath::Server,
        ..lock.clone()
    }
    .with_key(Uuid::from_u128(3));
    book.pins = vec![lock, unmarked, server_lock];

    assert!(book.acknowledge(Uuid::from_u128(1)));
    assert!(book.acknowledge(Uuid::from_u128(2)));
    assert!(book.acknowledge(Uuid::from_u128(3)));
    let left: Vec<Uuid> = book.pins.iter().map(|pin| pin.key).collect();
    assert_eq!(
        left,
        [Uuid::from_u128(3)],
        "the server's journal may settle it"
    );

    assert!(!book.expire(NOW + KEEP_MS - 1));
    assert!(book.expire(NOW + KEEP_MS));
    assert!(book.pins.is_empty());
}

#[test]
fn a_direct_feed_answered_is_no_uncertain_one_s_end() {
    let mut book = Book::default();
    book.admit(feed_pin(1, "a", RoutePath::Direct)).unwrap();
    book.admit(feed_pin(2, "b", RoutePath::Direct)).unwrap();
    book.admit(feed_pin(3, "a", RoutePath::Server)).ok();
    assert!(book.answered_direct("a", 7));
    assert_eq!(book.get(Uuid::from_u128(1)).unwrap().excluded, [7]);
    assert!(book.get(Uuid::from_u128(2)).unwrap().excluded.is_empty());
    assert!(!book.answered_direct("a", 7), "once");
}

/// One step a phone can take with one feeder.
#[derive(Clone, Debug)]
enum Step {
    /// A new feed, with the paths as they stand.
    Feed {
        server: bool,
        online: Option<bool>,
        direct_open: bool,
        keys_own: bool,
        /// How the first attempt ends; the fallback's, when it falls back.
        first: Ended,
        second: Ended,
    },
    /// A retry of the oldest pending write, ending so on its own path.
    Retry(Ended),
    Acknowledge,
    /// The app stops and starts again.
    Restart,
}

fn ended() -> impl Strategy<Value = Ended> {
    prop_oneof![
        Just(Ended::Answered),
        Just(Ended::Refused),
        Just(Ended::Unsent),
        Just(Ended::Uncertain),
        Just(Ended::Superseded),
    ]
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => (
            any::<bool>(),
            prop::option::of(any::<bool>()),
            any::<bool>(),
            any::<bool>(),
            ended(),
            ended()
        )
            .prop_map(|(server, online, direct_open, keys_own, first, second)| Step::Feed {
                server,
                online,
                direct_open,
                keys_own,
                first,
                second,
            }),
        2 => ended().prop_map(Step::Retry),
        1 => Just(Step::Acknowledge),
        1 => Just(Step::Restart),
    ]
}

proptest! {
    /// Whatever the paths do, a write never goes out on a second path
    /// once it may have run on its first, and no feed starts while
    /// another on the feeder is unanswered, or uncertain and not put away.
    #[test]
    fn an_uncertain_write_never_crosses_paths_and_never_doubles(steps in prop::collection::vec(step(), 1..40)) {
        let mut book = Book::default();
        // Every path a key went out on, past a definite non-send.
        let mut sent_on: std::collections::HashMap<Uuid, Vec<RoutePath>> = Default::default();
        let mut next_key = 1u128;

        for step in steps {
            match step {
                Step::Feed { server, online, direct_open, keys_own, first, second } => {
                    let key = Uuid::from_u128(next_key);
                    next_key += 1;
                    let keys = if keys_own { Keys::Own } else { Keys::ServerOnly };
                    let Some(access) = Access::of(server, direct_open, keys) else { continue };
                    let reach = reach(ServerReach::Up, online, direct_open);
                    let path = choose(access, reach, NOW);
                    let holding = book.holding_feeds("a").is_some();
                    let admitted = book.admit(feed_pin(next_key, "a", path).with_key(key));
                    prop_assert_eq!(admitted.is_err(), holding);
                    if admitted.is_err() {
                        continue;
                    }

                    let mut path = path;
                    let mut ending = first;
                    let mut fell_back = false;
                    loop {
                        if ending != Ended::Unsent {
                            sent_on.entry(key).or_default().push(path);
                        }
                        match fallback(path, ending, access, direct_open, fell_back) {
                            Some(next) => {
                                prop_assert!(book.moved(key, Some(1), NOW));
                                path = next;
                                fell_back = true;
                                ending = second;
                            }
                            None => {
                                let pin = book.get(key).cloned().unwrap_or_else(|| feed_pin(0, "a", path).with_key(key));
                                book.ended(PinRecord { path, ..pin }, ending);
                                break;
                            }
                        }
                    }
                }
                Step::Retry(ending) => {
                    let Some(pin) = book.pins.first().cloned() else { continue };
                    // A retry goes on the pin's own path, and only there.
                    if ending != Ended::Unsent {
                        sent_on.entry(pin.key).or_default().push(pin.path);
                    }
                    let ending = match ending {
                        // A retry's non-send says nothing of the first.
                        Ended::Unsent => Ended::Uncertain,
                        other => other,
                    };
                    book.ended(pin, ending);
                }
                Step::Acknowledge => {
                    if let Some(key) = book.pins.first().map(|pin| pin.key) {
                        book.acknowledge(key);
                    }
                }
                Step::Restart => {
                    book.pins = book.pins.iter().cloned().map(|pin| PinRecord { state: PinState::Uncertain, ..pin }).collect();
                }
            }

            for (key, paths) in &sent_on {
                let first = paths.first().copied();
                prop_assert!(paths.iter().all(|path| Some(*path) == first), "{key} went on {paths:?}");
            }
            for pin in &book.pins {
                prop_assert_eq!(sent_on.get(&pin.key).and_then(|paths| paths.first().copied()).unwrap_or(pin.path), pin.path);
            }
        }
    }
}

impl PinRecord {
    fn with_key(self, key: Uuid) -> Self {
        Self { key, ..self }
    }
}
