use std::collections::{HashMap, HashSet};

use proptest::prelude::*;
use wellfed_api::FeederRefusal;
use wellfed_model::{FeedOutcome, FeedReason};

use super::*;
use crate::api::server::ServerRefusal;

const NOW: i64 = 1_000_000;
const MARK: ServerMark = ServerMark {
    token: Uuid::from_u128(70),
    user: Uuid::from_u128(80),
};

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
        server: (path == RoutePath::Server).then_some(MARK),
        path,
        write: PinnedWrite::Feed { portions: 1 },
        state: PinState::Sending,
        sent_ms: NOW,
        tried_ms: NOW,
        mark: Some(10),
        excluded: Vec::new(),
        acknowledged_ms: None,
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
    // No key of its own, even to a feeder the server does not list.
    assert_eq!(Access::of(false, true, Keys::ServerOnly), None);
}

#[test]
fn a_token_the_server_no_longer_takes_closes_the_server_path_and_opens_nothing() {
    let refused = ServerReach::Refused;
    // An owner's own key closes with the token: nothing routes around the
    // account on the LAN, to a feeder the server lists or one it does not.
    let keys = Keys::Own.under(Standing::Refused);
    assert_eq!(keys, Keys::ServerOnly);
    assert_eq!(Access::of(true, true, keys), Some(Access::ServerOnly));
    assert_eq!(Access::of(false, true, keys), None);
    // However the server's reach goes after: refused is the standing's,
    // not the reach's.
    for reach_now in [
        refused,
        ServerReach::Down { since_ms: NOW },
        ServerReach::Up,
    ] {
        assert_eq!(
            choose(
                Access::ServerOnly,
                reach(reach_now, Some(false), true),
                NOW + RECHECK_MS
            ),
            RoutePath::Server
        );
    }
    assert_eq!(Keys::Own.under(Standing::Taken), Keys::Own);
    let taken = Standing::Taken;
    assert_eq!(
        standing_after(taken, ServerFailure::Unauthorized),
        Standing::Refused
    );
    for failure in [
        ServerFailure::Connect,
        ServerFailure::Interrupted,
        ServerFailure::Rejected,
    ] {
        assert_eq!(
            standing_after(Standing::Refused, failure),
            Standing::Refused
        );
        assert_eq!(standing_after(taken, failure), taken);
    }
    assert_eq!(reach_after(ServerFailure::Unauthorized, NOW), refused);
    assert_eq!(
        reach_after(ServerFailure::Connect, NOW),
        ServerReach::Down { since_ms: NOW }
    );
    assert_eq!(
        reach_after(ServerFailure::FeederOffline, NOW),
        ServerReach::Up
    );
    assert_eq!(
        reach_after(ServerFailure::Interrupted, NOW),
        ServerReach::Up
    );
}

#[test]
fn only_a_definite_non_send_on_the_server_goes_direct_and_only_once() {
    assert_eq!(
        fallback(RoutePath::Server, Ended::Unsent, Access::Both, true, false),
        Some(RoutePath::Direct)
    );
    for ended in [Ended::Uncertain, Ended::Answered, Ended::Refused] {
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
fn only_a_failure_before_sending_is_unsent() {
    use ServerFailure as F;
    for failure in [F::Connect, F::FeederOffline] {
        assert_eq!(server_ended(failure), Ended::Unsent);
    }
    for failure in [
        F::Rejected,
        F::KeyReused,
        F::Stale,
        F::Unauthorized,
        F::FeederRefused,
    ] {
        assert_eq!(server_ended(failure), Ended::Refused);
    }
    for failure in [F::Interrupted, F::OutcomeUnknown] {
        assert_eq!(server_ended(failure), Ended::Uncertain);
    }
}

#[test]
fn an_uncertain_write_is_only_ever_read_on_its_own_path() {
    assert_eq!(
        retry_plan(&feed_pin(1, "a", RoutePath::Server)),
        RetryPlan::ReadJournal
    );
    assert_eq!(
        retry_plan(&feed_pin(1, "a", RoutePath::Direct)),
        RetryPlan::ReadHistory
    );
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
    // A meal call write another change overtook: final for its key.
    assert_eq!(
        settled_by_command(CommandState::Answered, Some(&CommandOutcome::Superseded)),
        Some(Settlement::Superseded)
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
fn a_server_write_the_journal_lacks_is_unsent_only_from_its_own_journal_once_old() {
    let pin = feed_pin(1, "a", RoutePath::Server);
    let grace = UNFOUND_GRACE_MS;
    assert_eq!(
        settled_unfound(&pin, Some(MARK), NOW + grace - 1, grace),
        None
    );
    assert_eq!(
        settled_unfound(&pin, Some(MARK), NOW + grace, grace),
        Some(Settlement::NotSent)
    );

    // Another server (another token) or another account: its journal says
    // nothing of this write.
    let other_server = ServerMark {
        token: Uuid::from_u128(71),
        ..MARK
    };
    let other_user = ServerMark {
        user: Uuid::from_u128(81),
        ..MARK
    };
    for other in [Some(other_server), Some(other_user), None] {
        assert_eq!(settled_unfound(&pin, other, NOW + grace, grace), None);
    }
    let unmarked = PinRecord {
        server: None,
        ..pin
    };
    assert_eq!(settled_unfound(&unmarked, None, NOW + grace, grace), None);
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
    // Another feed's portions are another feed's end.
    let two = Seen {
        requested: 2,
        ..seen(12, 2)
    };
    assert_eq!(settled_by_history(&pin, &[], &[two]), None);
    // One this phone saw started is another feed's end.
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
    // Only a feed is the feeder history's to settle.
    let lock = PinRecord {
        write: PinnedWrite::Lock { locked: true },
        ..pin
    };
    assert_eq!(settled_by_history(&lock, &[], &[seen(12, 2)]), None);
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
    book.admit(feed_pin(1, "a", RoutePath::Server), NOW)
        .unwrap();

    // Still sending, on either path.
    let held = book
        .admit(feed_pin(2, "a", RoutePath::Direct), NOW)
        .unwrap_err();
    assert_eq!(held, Uuid::from_u128(1));
    // Another feeder's feed goes.
    book.admit(feed_pin(3, "b", RoutePath::Direct), NOW)
        .unwrap();

    assert!(book.ended(Uuid::from_u128(1), Ended::Uncertain, NOW));
    assert!(
        book.admit(feed_pin(4, "a", RoutePath::Direct), NOW)
            .is_err()
    );
    assert!(book.acknowledge(Uuid::from_u128(1), NOW));
    assert!(!book.acknowledge(Uuid::from_u128(1), NOW), "once");
    book.admit(feed_pin(4, "a", RoutePath::Direct), NOW)
        .unwrap();
    assert_eq!(
        book.get(Uuid::from_u128(1)).map(|pin| pin.path),
        Some(RoutePath::Server),
        "put away, it stays pinned"
    );
}

#[test]
fn a_write_other_than_a_feed_is_never_held_back() {
    let mut book = Book::default();
    book.admit(feed_pin(1, "a", RoutePath::Server), NOW)
        .unwrap();
    let lock = PinRecord {
        write: PinnedWrite::Lock { locked: true },
        ..feed_pin(2, "a", RoutePath::Server)
    };
    book.admit(lock, NOW).unwrap();
    // And it holds no feed back.
    book.settle(Uuid::from_u128(1));
    book.admit(feed_pin(3, "a", RoutePath::Server), NOW)
        .unwrap();
}

#[test]
fn an_uncertain_pin_keeps_its_path_and_the_owner_s_acknowledgement() {
    let mut book = Book::default();
    book.admit(feed_pin(1, "a", RoutePath::Server), NOW)
        .unwrap();
    assert!(book.ended(Uuid::from_u128(1), Ended::Uncertain, NOW));
    assert!(book.acknowledge(Uuid::from_u128(1), NOW));

    // Ending uncertain again changes only its last attempt.
    assert!(book.ended(Uuid::from_u128(1), Ended::Uncertain, NOW + 5));
    let pin = book.get(Uuid::from_u128(1)).unwrap();
    assert_eq!(
        (pin.path, pin.acknowledged_ms, pin.tried_ms, pin.server),
        (RoutePath::Server, Some(NOW), NOW + 5, Some(MARK))
    );
    // And an uncertain pin never moves to the other path.
    assert!(!book.moved(Uuid::from_u128(1), Some(3), NOW));
}

#[test]
fn a_feed_the_server_never_sent_moves_while_still_sending() {
    let mut book = Book::default();
    book.started_direct("a", 6, NOW + 60_000);
    book.admit(feed_pin(1, "a", RoutePath::Server), NOW)
        .unwrap();
    assert!(book.moved(Uuid::from_u128(1), Some(21), NOW + 1));
    let pin = book.get(Uuid::from_u128(1)).unwrap();
    assert_eq!(
        (pin.path, pin.mark, pin.server, pin.excluded.as_slice()),
        (RoutePath::Direct, Some(21), None, &[6][..])
    );
    assert!(!book.moved(Uuid::from_u128(1), Some(22), NOW + 2), "once");
}

#[test]
fn any_end_but_uncertain_takes_the_pin_out() {
    for ended in [Ended::Answered, Ended::Refused, Ended::Unsent] {
        let mut book = Book::default();
        book.admit(feed_pin(1, "a", RoutePath::Server), NOW)
            .unwrap();
        assert!(book.ended(Uuid::from_u128(1), ended, NOW));
        assert!(book.pins.is_empty());
        book.admit(feed_pin(2, "a", RoutePath::Server), NOW)
            .unwrap();
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
        key: Uuid::from_u128(3),
        path: RoutePath::Server,
        server: Some(MARK),
        ..lock.clone()
    };
    book.pins = vec![lock, unmarked, server_lock];

    let at = NOW + 1_000;
    assert!(book.acknowledge(Uuid::from_u128(1), at));
    assert!(book.acknowledge(Uuid::from_u128(2), at));
    assert!(book.acknowledge(Uuid::from_u128(3), at));
    let left: Vec<Uuid> = book.pins.iter().map(|pin| pin.key).collect();
    assert_eq!(
        left,
        [Uuid::from_u128(3)],
        "the server's journal may settle it"
    );

    // A week from the acknowledgement, not from the send.
    assert!(!book.expire(at + KEEP_MS - 1));
    assert!(book.expire(at + KEEP_MS));
    assert!(book.pins.is_empty());
}

#[test]
fn a_direct_feed_seen_started_is_no_uncertain_one_s_end_then_or_later() {
    let mut book = Book::default();
    book.admit(feed_pin(1, "a", RoutePath::Direct), NOW)
        .unwrap();
    assert!(book.ended(Uuid::from_u128(1), Ended::Uncertain, NOW));
    assert!(book.acknowledge(Uuid::from_u128(1), NOW));
    book.admit(feed_pin(2, "b", RoutePath::Direct), NOW)
        .unwrap();

    book.started_direct("a", 7, NOW + 60_000);
    assert_eq!(book.get(Uuid::from_u128(1)).unwrap().excluded, [7]);
    assert!(book.get(Uuid::from_u128(2)).unwrap().excluded.is_empty());

    // A feed sent while 7 runs takes no end from it either.
    book.admit(feed_pin(3, "a", RoutePath::Direct), NOW + 1)
        .unwrap();
    assert_eq!(book.get(Uuid::from_u128(3)).unwrap().excluded, [7]);
    assert!(book.ended_direct("a", &[7]));
    assert!(!book.ended_direct("a", &[7]), "once");
    assert!(book.running_on("a", NOW + 2).is_empty());

    // A running feed past the time it is kept no longer holds.
    book.started_direct("a", 8, NOW + 10);
    assert_eq!(book.running_on("a", NOW + 9), [8]);
    assert!(book.expire(NOW + 10));
    assert!(book.running.is_empty());
}

/// How one send ends, in the world and as the phone hears it.
#[derive(Clone, Copy, Debug)]
enum Sent {
    /// It ran and the phone heard so.
    Answered,
    /// It ran and was journaled, and the answer was lost before it reached the
    /// phone.
    LostAfterRunning,
    /// `504`: journaled unknown; it may or may not have run.
    Unknown {
        ran: bool,
    },
    /// It never ran and the server holds no entry, but the phone heard
    /// nothing: a lost `feeder_offline`, a `500` before the insert, a
    /// request that never reached it, a crash before the send.
    LostUnjournaled,
    /// It never ran, and the phone knows: `feeder_offline` or no
    /// connection.
    Unsent,
    Refused,
}

/// One step a phone can take with one feeder.
#[derive(Clone, Debug)]
enum Step {
    /// A new feed, with the paths as they stand.
    Feed {
        online: Option<bool>,
        direct_open: bool,
        keys_own: bool,
        /// How the first attempt ends; the fallback's, when it falls back.
        first: Sent,
        second: Sent,
    },
    /// Ask the oldest pending write's own path what became of it.
    Retry,
    Acknowledge,
    /// The app stops and starts again.
    Restart,
}

fn sent() -> impl Strategy<Value = Sent> {
    prop_oneof![
        Just(Sent::Answered),
        Just(Sent::LostAfterRunning),
        any::<bool>().prop_map(|ran| Sent::Unknown { ran }),
        Just(Sent::LostUnjournaled),
        Just(Sent::Unsent),
        Just(Sent::Refused),
    ]
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => (
            prop::option::of(any::<bool>()),
            any::<bool>(),
            any::<bool>(),
            sent(),
            sent()
        )
            .prop_map(|(online, direct_open, keys_own, first, second)| Step::Feed {
                online,
                direct_open,
                keys_own,
                first,
                second,
            }),
        3 => Just(Step::Retry),
        2 => Just(Step::Acknowledge),
        1 => Just(Step::Restart),
    ]
}

/// What really happened, of which the phone sees only part.
#[derive(Default)]
struct World {
    /// Feeds the feeder dispensed, by key.
    ran: HashMap<Uuid, u32>,
    /// The keys the server's journal holds.
    journal: HashSet<Uuid>,
    /// Every send of each key, by path.
    sent_on: HashMap<Uuid, Vec<RoutePath>>,
}

impl World {
    /// Send `key` on `path`, ending `how`: what the phone hears.
    fn send(&mut self, key: Uuid, path: RoutePath, how: Sent) -> Ended {
        let (ran, journaled, heard) = match how {
            Sent::Answered => (true, true, Ended::Answered),
            Sent::LostAfterRunning => (true, true, Ended::Uncertain),
            Sent::Unknown { ran } => (ran, true, Ended::Uncertain),
            Sent::LostUnjournaled => (false, false, Ended::Uncertain),
            Sent::Unsent => (false, false, Ended::Unsent),
            Sent::Refused => (false, true, Ended::Refused),
        };
        if heard != Ended::Unsent {
            self.sent_on.entry(key).or_default().push(path);
        }
        if ran {
            *self.ran.entry(key).or_default() += 1;
        }
        if journaled && path == RoutePath::Server {
            self.journal.insert(key);
        }
        heard
    }

    /// What reading `pin`'s own path says, by its retry plan: a retry only
    /// reads. A journaled write that ran is answered; one journaled unknown
    /// that did not run stays unknown.
    fn read(&self, pin: &PinRecord) -> Option<Settlement> {
        let ran = self.ran.contains_key(&pin.key);
        match retry_plan(pin) {
            RetryPlan::ReadJournal if self.journal.contains(&pin.key) => {
                ran.then_some(Settlement::Started { feed: 1 })
            }
            RetryPlan::ReadJournal => settled_unfound(
                pin,
                Some(MARK),
                pin.tried_ms + UNFOUND_GRACE_MS,
                UNFOUND_GRACE_MS,
            ),
            RetryPlan::ReadHistory => ran.then_some(Settlement::Started { feed: 1 }),
        }
    }
}

proptest! {
    /// Whatever the paths do and whatever the owner does after (put a write
    /// away, feed again, retry the old one), each key is sent once on one
    /// path, the feeder dispenses no key twice, a write is settled as never
    /// sent only when it never ran, and no feed starts while another on the
    /// feeder is unanswered or uncertain and not put away.
    #[test]
    fn an_uncertain_write_never_crosses_paths_and_never_doubles(steps in prop::collection::vec(step(), 1..60)) {
        let mut book = Book::default();
        let mut world = World::default();
        let mut next_key = 1u128;

        for step in steps {
            match step {
                Step::Feed { online, direct_open, keys_own, first, second } => {
                    let key = next_key;
                    next_key += 1;
                    let keys = if keys_own { Keys::Own } else { Keys::ServerOnly };
                    let Some(access) = Access::of(true, direct_open, keys) else { continue };
                    let path = choose(access, reach(ServerReach::Up, online, direct_open), NOW);
                    let holding = book.holding_feeds("a").is_some();
                    let admitted = book.admit(feed_pin(key, "a", path), NOW);
                    prop_assert_eq!(admitted.is_err(), holding);
                    if admitted.is_err() {
                        continue;
                    }

                    let key = Uuid::from_u128(key);
                    let mut path = path;
                    let mut ending = world.send(key, path, first);
                    if fallback(path, ending, access, direct_open, false).is_some() {
                        prop_assert!(book.moved(key, Some(1), NOW));
                        path = RoutePath::Direct;
                        ending = world.send(key, path, second);
                    }
                    book.ended(key, ending, NOW);
                }
                Step::Retry => {
                    let Some(pin) = book.pins.first().cloned() else { continue };
                    if let Some(settlement) = world.read(&pin) {
                        prop_assert!(
                            settlement != Settlement::NotSent || !world.ran.contains_key(&pin.key),
                            "{} ran, and was settled as never sent", pin.key
                        );
                        book.settle(pin.key);
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

            for (key, paths) in &world.sent_on {
                prop_assert_eq!(paths.len(), 1, "{} went on {:?}", key, paths);
            }
            for (key, count) in &world.ran {
                prop_assert!(*count <= 1, "{key} fed {count} times");
            }
            for pin in &book.pins {
                let first = world.sent_on.get(&pin.key).and_then(|paths| paths.first().copied());
                prop_assert_eq!(first.unwrap_or(pin.path), pin.path);
            }
        }
    }
}
