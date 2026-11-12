//! The bridge against the simulated feeder, which runs the real feeder's
//! pairing, stores and feeding owner, in real time on a thread of its own:
//! - an owner's member invite enrolls the integration's new key;
//! - the session reads the backlog into its state, runs a feed to its
//!   `Fed` event, cancels one, and changes the lock;
//! - a store change goes on the generation read, one at a time: a second
//!   is not sent while the first waits, and a stale one is refused;
//! - a feed whose answer is lost is uncertain, sent once, and settled by
//!   its `Fed` event;
//! - a write while the feeder is gone is refused unsent, never held, and a
//!   revoked key stops the session;
//! - a write refused before it went leaves the session, and a feed still
//!   out on it, as they were.

use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use wellfed_client::api::Invite;
use wellfed_connect::{Connection, Timing};
use wellfed_feeder_sim::harness::{Relay, Threaded};
use wellfed_feeder_sim::serve::Control;
use wellfed_homeassistant::api::JoinProblem;
use wellfed_homeassistant::api::{
    EventKind, FeedOutcome, NotSent, Pending, RefusalKind, Role, State, Update, UpdateKind,
    WriteResult, Written,
};
use wellfed_homeassistant::feeder::{Carried, Clocks, Feeder, Target};
use wellfed_homeassistant::key::{
    ClientKey, Unsettled, check_join, invite_feeder, join, join_member, join_problem, redeem,
};
use wellfed_model::{
    Answer, ClientRole, FeederSettingsPatch, Generation, MealReminder, Op, PLANS, Plan, Pushed,
    Weekdays,
};
use wellfed_session::noise::Keypair;

const OWNER: [u8; 32] = [0x21; 32];
const WAIT: Duration = Duration::from_secs(60);

/// The clocks of a test that waits out a lost answer or a lost session:
/// far shorter than Home Assistant's, so it waits seconds rather than
/// tens of them, and still far longer than the simulator takes to answer.
/// A test that holds answers back while it does other work keeps
/// [`Clocks::DEFAULT`], so a slow host never times its write out.
const SHORT: Clocks = Clocks {
    session: Timing {
        answer: Duration::from_secs(2),
        ..Timing::DEFAULT
    },
    retry: Duration::from_millis(200),
    settle: Clocks::DEFAULT.settle,
};

/// [`SHORT`], with a lost feed reported still unknown after seconds.
const QUICK_REPORT: Clocks = Clocks {
    settle: Duration::from_secs(3),
    ..SHORT
};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// `op` from the simulator's owner, on a session of its own, under a
/// request id no earlier write in this process used: the feeder answers a
/// repeated id from memory.
fn owner(sim: &Threaded, op: &Op) -> Answer {
    static IDS: AtomicU8 = AtomicU8::new(1);
    let id = [IDS.fetch_add(1, Ordering::Relaxed); 16];
    runtime().block_on(async {
        let owner = Keypair::from_secret(OWNER);
        let mut connection = Connection::open(&owner, sim.identity, sim.address, [id[0]; 32])
            .await
            .unwrap();
        connection.ask(id, op).await.unwrap()
    })
}

/// A member invite from the simulator's owner, as `wellfed invite` prints it.
fn invite(sim: &Threaded) -> String {
    let role = ClientRole::Member;
    let Answer::Invite(invite) = owner(sim, &Op::CreateInvite { role }) else {
        panic!("no invite");
    };
    Invite {
        feeder: sim.identity,
        id: invite.id,
        secret: invite.secret.0,
    }
    .text()
}

/// A simulator, and the integration's key joined to it as a member.
fn joined() -> (Threaded, ClientKey) {
    let sim = Threaded::start(*Keypair::from_secret(OWNER).public());
    let text = invite(&sim);
    assert_eq!(invite_feeder(&text).unwrap(), sim.identity);

    let key = ClientKey::generate().unwrap();
    let address = sim.address.to_string();
    let role = runtime()
        .block_on(join(&key, &text, "home-assistant", Some(&address)))
        .unwrap();
    assert_eq!(role, ClientRole::Member);
    (sim, key)
}

/// The bridge's session with the feeder at `address`, on `clocks`.
fn session(
    sim: &Threaded,
    key: ClientKey,
    address: std::net::SocketAddr,
    clocks: Clocks,
) -> Feeder {
    let target = Target {
        identity: sim.identity,
        address: Some(address.to_string()),
    };
    Feeder::start(key, target, clocks, Carried::default()).unwrap()
}

/// Updates until one `want` accepts, failing past [`WAIT`].
fn until<T>(feeder: &Feeder, mut want: impl FnMut(&Update) -> Option<T>) -> T {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if let Some(update) = feeder.next_update(Duration::from_secs(1))
            && let Some(found) = want(&update)
        {
            return found;
        }
    }
    panic!("no matching update within {WAIT:?}");
}

/// The first state that `want` accepts.
/// The owner releases the lost feed `pending` names, through its repair:
/// it as the session confirms it.
fn release(feeder: &Feeder, pending: &Pending) -> Pending {
    feeder.release(pending.id.clone());
    let released = until(feeder, |update| {
        (update.kind == UpdateKind::Released).then(|| update.pending.clone())
    })
    .expect("the released feed");
    assert_eq!(released.id, pending.id);
    assert!(released.released_ms.is_some());
    released
}

fn state_where(feeder: &Feeder, want: impl Fn(&State) -> bool) -> State {
    until(feeder, |update| {
        update.state.clone().filter(|state| want(state))
    })
}

/// A relay's tasks, on workers that run for the whole test.
fn relays() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap()
}

/// Wait until `done` holds, failing past [`WAIT`].
fn wait_for(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "{what} within {WAIT:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

const BREAKFAST: Plan = Plan {
    minute: 7 * 60 + 30,
    portions: 2,
    days: Weekdays::DAILY,
    enabled: true,
    call: 0,
};

#[test]
fn the_session_feeds_to_its_fed_event_and_sets_the_lock() {
    let (sim, key) = joined();
    let restored = ClientKey::restore(&key.secret()).unwrap();
    assert_eq!(restored.public(), key.public());

    let feeder = session(&sim, restored, sim.address, Clocks::DEFAULT);
    let role = until(&feeder, |update| update.role);
    assert_eq!(role, Role::Member);
    state_where(&feeder, |state| state.settings.is_some());

    let written = feeder.feed(1);
    assert_eq!(written.result, WriteResult::Feeding, "{written:?}");
    let feed = written.feed;
    let delivered = until(&feeder, |update| {
        let event = update.event.as_ref()?;
        assert!(!update.replayed, "a feed as it happens is news");
        let fed = event.fed.as_ref()?;
        (Some(fed.feed) == feed).then_some(fed.completed)
    });
    assert_eq!(delivered, 1);
    let state = state_where(&feeder, |state| state.last_fed.is_some());
    assert_eq!(state.last_fed.map(|fed| fed.feed), feed);

    assert_eq!(feeder.set_lock(true), Written::done());
    state_where(&feeder, |state| state.status.locked == Some(true));
}

#[test]
fn the_backlog_sets_the_last_feed_and_is_not_news() {
    let (sim, key) = joined();
    let Answer::Feeding { feed } = owner(&sim, &Op::Feed { portions: 1 }) else {
        panic!("the owner's feed did not start");
    };
    // The owner's feed ends in its `Fed` before the bridge connects.
    runtime().block_on(async {
        let mut connection = Connection::open(
            &Keypair::from_secret(OWNER),
            sim.identity,
            sim.address,
            [0x77; 32],
        )
        .await
        .unwrap();
        connection.subscribe(None, false).await.unwrap();
        loop {
            if let Pushed::Event(event) = connection.next(WAIT).await.unwrap()
                && matches!(
                    event.body,
                    wellfed_model::EventBody::Feeder(wellfed_model::FeederEvent::Fed { .. })
                )
            {
                break;
            }
        }
    });

    let feeder = session(&sim, key, sim.address, Clocks::DEFAULT);
    let replayed = until(&feeder, |update| {
        let event = update.event.as_ref()?;
        (event.kind == EventKind::Fed).then_some(update.replayed)
    });
    assert!(
        replayed,
        "a feed from before the session is history, not news"
    );
    let state = until(&feeder, |update| update.state.clone());
    let last = state.last_fed.expect("the backlog's feed");
    assert_eq!((last.feed, last.completed), (feed, 1));
    assert!(state.status.last_event.is_some());
}

#[test]
fn a_running_feed_can_be_cancelled() {
    let (sim, key) = joined();
    let feeder = session(&sim, key, sim.address, Clocks::DEFAULT);
    state_where(&feeder, |state| state.settings.is_some());

    sim.control(Control::StallFeeds(true));
    let written = feeder.feed(3);
    assert_eq!(written.result, WriteResult::Feeding, "{written:?}");
    let feed = written.feed.unwrap();
    assert_eq!(feeder.cancel_feed(feed), Written::done());

    let outcome = until(&feeder, |update| {
        let fed = update.event.as_ref()?.fed.as_ref()?;
        (fed.feed == feed).then_some(fed.outcome)
    });
    assert_eq!(outcome, FeedOutcome::Cancelled);
}

#[test]
fn settings_change_on_the_generation_read_and_one_at_a_time() {
    let (sim, key) = joined();
    let relays = relays();
    let relay = relays.block_on(Relay::start(sim.address));
    let feeder = std::sync::Arc::new(session(&sim, key, relay.address, Clocks::DEFAULT));
    state_where(&feeder, |state| state.settings.is_some());

    // The first change reaches the feeder; its answer is held back.
    let before = relay.passed();
    relay.hold_answers();
    let first = {
        let feeder = std::sync::Arc::clone(&feeder);
        std::thread::spawn(move || feeder.put_settings(volume(20)))
    };
    wait_for("the first change to go out", || relay.passed() > before);

    let second = feeder.put_settings(volume(30));
    assert_eq!(second, Written::not_sent(NotSent::Busy));

    relay.resume();
    assert_eq!(first.join().unwrap(), Written::done());
    // The next change goes on the generation the first landed at, at once,
    // without waiting for the settings to be read again.
    assert_eq!(feeder.put_settings(volume(25)), Written::done());
    let state = state_where(&feeder, |state| {
        state.settings.is_some_and(|settings| settings.volume == 25)
    });
    assert_eq!(state.settings.map(|settings| settings.volume), Some(25));
}

fn volume(volume: u8) -> FeederSettingsPatch {
    FeederSettingsPatch {
        volume: Some(volume),
        ..FeederSettingsPatch::default()
    }
}

#[test]
fn a_stale_plans_change_is_refused_and_the_plans_read_again() {
    let (sim, key) = joined();
    let mut table = [None; PLANS];
    table[0] = Some(BREAKFAST);
    let put = Op::PutPlans {
        base: Generation(0),
        plans: table,
    };
    assert!(matches!(owner(&sim, &put), Answer::Done { .. }));

    let relays = relays();
    let relay = relays.block_on(Relay::start(sim.address));
    let feeder = std::sync::Arc::new(session(&sim, key, relay.address, Clocks::DEFAULT));
    state_where(&feeder, |state| {
        state.plans.as_ref().is_some_and(|plans| plans.len() == 1)
    });

    // The bridge's change waits in the relay while the owner changes the
    // plans, so it reaches the feeder on an older generation.
    relay.pause();
    let switched = {
        let feeder = std::sync::Arc::clone(&feeder);
        std::thread::spawn(move || feeder.set_plan_enabled(0, false))
    };
    wait_for("the change to wait in the relay", || {
        relay.holding_requests()
    });
    let Answer::Plans(plans) = owner(&sim, &Op::GetPlans) else {
        panic!("the owner reads the plans");
    };
    table[1] = Some(Plan {
        minute: 18 * 60,
        ..BREAKFAST
    });
    let moved = Op::PutPlans {
        base: plans.generation,
        plans: table,
    };
    assert!(matches!(owner(&sim, &moved), Answer::Done { .. }));
    relay.resume();

    let written = switched.join().unwrap();
    assert_eq!(written.result, WriteResult::Refused, "{written:?}");
    assert_eq!(written.refusal, Some(RefusalKind::Stale));
    // Read again: the owner's two plans, the first still on.
    let state = state_where(&feeder, |state| {
        state.plans.as_ref().is_some_and(|plans| plans.len() == 2)
    });
    assert!(state.plans.unwrap().iter().all(|plan| plan.enabled));
}

#[test]
fn a_write_while_the_feeder_is_gone_is_not_sent() {
    let (sim, key) = joined();
    let feeder = session(&sim, key, sim.address, Clocks::DEFAULT);
    state_where(&feeder, |_| true);

    drop(sim);
    until(&feeder, |update| {
        (update.kind == UpdateKind::Disconnected).then_some(())
    });
    assert_eq!(feeder.feed(1), Written::not_sent(NotSent::Disconnected));
}

/// A write refused before a byte went (a settings change naming a value
/// this build cannot put on the wire) is not sent, and the session goes
/// on: a feed still out beside it keeps its answer, never made uncertain.
#[test]
fn an_unsendable_write_is_not_sent_and_leaves_the_feed_beside_it() {
    let (sim, key) = joined();
    let relays = relays();
    let relay = relays.block_on(Relay::start(sim.address));
    let feeder = std::sync::Arc::new(session(&sim, key, relay.address, Clocks::DEFAULT));
    state_where(&feeder, |state| state.settings.is_some());

    let before = relay.passed();
    relay.hold_answers();
    let feed = {
        let feeder = std::sync::Arc::clone(&feeder);
        std::thread::spawn(move || feeder.feed(2))
    };
    wait_for("the feed to go out", || relay.passed() > before);

    let passed = relay.passed();
    let unsendable = FeederSettingsPatch {
        meal_reminder: Some(MealReminder::Unknown),
        ..FeederSettingsPatch::default()
    };
    assert_eq!(
        feeder.put_settings(unsendable),
        Written::not_sent(NotSent::Invalid)
    );
    assert_eq!(relay.passed(), passed, "nothing went out");

    relay.resume();
    let written = feed.join().unwrap();
    assert_eq!(written.result, WriteResult::Feeding, "{written:?}");
    let number = written.feed;
    let outcome = until(&feeder, |update| {
        let fed = update.event.as_ref()?.fed.as_ref()?;
        (Some(fed.feed) == number).then_some(fed.outcome)
    });
    assert_eq!(outcome, FeedOutcome::Done);
}

#[test]
fn a_member_sets_the_portion_weight() {
    let (sim, key) = joined();
    let feeder = session(&sim, key, sim.address, Clocks::DEFAULT);
    state_where(&feeder, |_| true);

    assert_eq!(feeder.set_portion_weight(9_500), Written::done());
    state_where(&feeder, |state| state.portion_mg == Some(9_500));
    assert_eq!(
        feeder.set_portion_weight(999),
        Written::not_sent(NotSent::Invalid)
    );
}

#[test]
fn an_owner_invite_s_key_removes_itself() {
    let sim = Threaded::start(*Keypair::from_secret(OWNER).public());
    let role = ClientRole::Owner;
    let Answer::Invite(invite) = owner(&sim, &Op::CreateInvite { role }) else {
        panic!("no invite");
    };
    let text = Invite {
        feeder: sim.identity,
        id: invite.id,
        secret: invite.secret.0,
    }
    .text();

    let key = ClientKey::generate().unwrap();
    let address = sim.address.to_string();
    let joined = runtime()
        .block_on(join_member(&key, &text, "home-assistant", Some(&address)))
        .unwrap();
    assert_eq!(joined.role, Role::Owner);
    assert!(joined.removed);

    let Answer::Clients(clients) = owner(&sim, &Op::ListClients) else {
        panic!("the owner lists the clients");
    };
    assert!(clients.iter().all(|client| client.key.0 != key.public()));
}

#[test]
fn a_feed_whose_answer_is_lost_is_uncertain_sent_once_and_then_settled() {
    let (sim, key) = joined();
    let relays = relays();
    let relay = relays.block_on(Relay::start(sim.address));
    let feeder = session(&sim, key, relay.address, SHORT);
    state_where(&feeder, |_| true);

    relay.hold_answers();
    let passed = relay.passed();
    let written = feeder.feed(1);
    assert_eq!(written.result, WriteResult::Uncertain);
    let pending = written.pending.expect("an uncertain feed is kept pending");

    // The feed reached the feeder once and ran there; the bridge sent it
    // once, and its `Fed` settles it.
    relay.resume();
    // Announced as pending before anything settles it.
    let mut announced = false;
    let (settled, which) = until(&feeder, |update| {
        if update.kind == UpdateKind::Pending {
            assert_eq!(update.pending.as_ref(), Some(&pending));
            announced = true;
        }
        (update.kind == UpdateKind::Settled).then(|| (update.feed.clone(), update.pending.clone()))
    });
    assert!(announced, "the pending update comes before its settlement");
    assert_eq!(which, Some(pending));
    let settled = settled.expect("the settled feed");
    assert_eq!(settled.completed, 1);
    assert!(relay.passed() > passed);

    let quiet = Instant::now() + Duration::from_secs(3);
    while Instant::now() < quiet {
        if let Some(update) = feeder.next_update(Duration::from_millis(200)) {
            let fed = update.event.and_then(|event| event.fed);
            assert!(
                fed.is_none_or(|fed| fed.feed == settled.feed),
                "a second feed ran"
            );
            assert_ne!(update.kind, UpdateKind::StillUnknown);
        }
    }
}

#[test]
fn a_revoked_key_stops_the_session_for_good() {
    let (sim, key) = joined();
    let public = key.public();
    let feeder = session(&sim, key, sim.address, SHORT);
    state_where(&feeder, |_| true);

    let revoke = Op::RevokeClient { key: public };
    assert!(matches!(owner(&sim, &revoke), Answer::Done { .. }));
    until(&feeder, |update| {
        (update.kind == UpdateKind::Revoked).then_some(())
    });
    assert_eq!(feeder.feed(1), Written::not_sent(NotSent::Stopped));
}

#[test]
fn a_feed_that_never_reached_the_feeder_is_still_unknown_after_the_backlog() {
    let (sim, key) = joined();
    let relays = relays();
    let relay = relays.block_on(Relay::start(sim.address));
    let feeder = std::sync::Arc::new(session(&sim, key, relay.address, SHORT));
    state_where(&feeder, |_| true);

    // The request waits in the relay, which then drops it with the
    // connection: it went out, so the bridge cannot know it never arrived.
    relay.pause();
    let fed = {
        let feeder = std::sync::Arc::clone(&feeder);
        std::thread::spawn(move || feeder.feed(1))
    };
    wait_for("the feed to wait in the relay", || relay.holding_requests());
    relay.cut();
    relay.resume();
    let written = fed.join().unwrap();
    assert_eq!(written.result, WriteResult::Uncertain);
    let pending = written.pending.expect("an uncertain feed is kept pending");

    // The next session's backlog holds no `Fed` for it, and the feeder is
    // not feeding: it is reported unknown, with no count.
    let (portions, which) = until(&feeder, |update| {
        assert_ne!(update.kind, UpdateKind::Settled, "nothing ran to settle it");
        (update.kind == UpdateKind::StillUnknown).then(|| (update.portions, update.pending.clone()))
    });
    let reported = Pending {
        reported: true,
        ..pending
    };
    assert_eq!((portions, which), (Some(1), Some(reported)));
}

/// A lost feed reported still unknown and released by the owner holds
/// feeds back no longer, and stays open to its own `Fed`. A later lost
/// feed of other portions that runs is settled by its own `Fed`, never the
/// older one, which never ran.
#[test]
fn a_fed_settles_the_lost_feed_of_its_portions_not_an_older_one() {
    let (sim, key) = joined();
    let relays = relays();
    let relay = relays.block_on(Relay::start(sim.address));
    let feeder = std::sync::Arc::new(session(&sim, key, relay.address, SHORT));
    state_where(&feeder, |_| true);

    // A, of two portions, waits in the relay, which drops it with the
    // connection: it never reaches the feeder.
    relay.pause();
    let fed = {
        let feeder = std::sync::Arc::clone(&feeder);
        std::thread::spawn(move || feeder.feed(2))
    };
    wait_for("the feed to wait in the relay", || relay.holding_requests());
    relay.cut();
    relay.resume();
    let older = fed.join().unwrap().pending.expect("A is kept pending");
    let reported = until(&feeder, |update| {
        (update.kind == UpdateKind::StillUnknown).then(|| update.pending.clone())
    });
    let older = Pending {
        reported: true,
        ..older
    };
    assert_eq!(reported, Some(older.clone()));
    release(&feeder, &older);

    // B, of one portion, goes out though A is still open; its answer is
    // lost, and it runs.
    relay.hold_answers();
    let written = feeder.feed(1);
    assert_eq!(written.result, WriteResult::Uncertain, "{written:?}");
    let newer = written.pending.expect("B is kept pending");
    relay.resume();
    let (which, settled) = until(&feeder, |update| {
        (update.kind == UpdateKind::Settled).then(|| (update.pending.clone(), update.feed.clone()))
    });
    assert_eq!(which, Some(newer));
    assert_eq!(settled.map(|fed| fed.requested), Some(1));

    let quiet = Instant::now() + Duration::from_secs(3);
    while Instant::now() < quiet {
        if let Some(update) = feeder.next_update(Duration::from_millis(200)) {
            assert!(
                !matches!(update.kind, UpdateKind::Settled | UpdateKind::Forgotten),
                "A ran nowhere and is still waited on: {update:?}"
            );
        }
    }
}

/// A feed reported still unknown stays open, and holds new feeds back:
/// its `Fed`, held back past the report, still settles it, and only then
/// does a feed go again.
#[test]
fn a_feed_reported_unknown_is_settled_by_its_late_fed() {
    let (sim, key) = joined();
    let relays = relays();
    let relay = relays.block_on(Relay::start(sim.address));
    let feeder = session(&sim, key, relay.address, QUICK_REPORT);
    state_where(&feeder, |_| true);

    relay.hold_answers();
    let written = feeder.feed(1);
    assert_eq!(written.result, WriteResult::Uncertain, "{written:?}");
    let pending = written.pending.expect("the feed is kept pending");
    let reported = until(&feeder, |update| {
        assert_ne!(update.kind, UpdateKind::Settled, "its `Fed` is held back");
        (update.kind == UpdateKind::StillUnknown).then(|| update.pending.clone())
    });
    assert_eq!(reported.as_ref().map(|lost| &lost.id), Some(&pending.id));
    assert_eq!(feeder.feed(1), Written::not_sent(NotSent::Held));

    relay.resume();
    let (which, fed) = until(&feeder, |update| {
        (update.kind == UpdateKind::Settled).then(|| (update.pending.clone(), update.feed.clone()))
    });
    assert_eq!(which.map(|lost| lost.id), Some(pending.id));
    assert_eq!(fed.map(|fed| fed.requested), Some(1));
    assert_eq!(feeder.feed(1).result, WriteResult::Feeding);
}

/// Home Assistant stops while a feed is out, after a feed of the same
/// portions was reported still unknown. The feed it was sending is in its
/// store, so after the restart that feed's `Fed` could be either's, and
/// settles neither: the older feed, which never ran, is never given it.
#[test]
fn a_feed_sending_when_home_assistant_stops_is_never_taken_for_an_older_one() {
    let (sim, key) = joined();
    let secret = key.secret().to_vec();
    let relays = relays();
    let relay = relays.block_on(Relay::start(sim.address));
    let feeder = std::sync::Arc::new(session(&sim, key, relay.address, SHORT));
    state_where(&feeder, |_| true);

    // U, of two portions, never reaches the feeder, is reported still
    // unknown at the next session's backlog, and the owner releases it.
    relay.pause();
    let fed = {
        let feeder = std::sync::Arc::clone(&feeder);
        std::thread::spawn(move || feeder.feed(2))
    };
    wait_for("the feed to wait in the relay", || relay.holding_requests());
    relay.cut();
    relay.resume();
    assert_eq!(fed.join().unwrap().result, WriteResult::Uncertain);
    let older = until(&feeder, |update| {
        (update.kind == UpdateKind::StillUnknown).then(|| update.pending.clone())
    })
    .expect("U");
    assert!(older.reported);
    let older = release(&feeder, &older);

    // F, of two portions, goes out and runs; Home Assistant stops before
    // its answer, with what the store had by then: F as it went out.
    relay.hold_answers();
    assert_eq!(feeder.feed(2).result, WriteResult::Uncertain);
    let sending = until(&feeder, |update| {
        (update.kind == UpdateKind::Sending).then(|| update.pending.clone())
    })
    .expect("F");
    assert!(!sending.reported);
    drop(feeder);
    std::thread::sleep(Duration::from_secs(3));

    let target = Target {
        identity: sim.identity,
        address: Some(sim.address.to_string()),
    };
    let f = sending.id.clone();
    let carried = Carried {
        pending: vec![older, sending],
        answered: Vec::new(),
        unread: false,
    };
    let restarted =
        Feeder::start(ClientKey::restore(&secret).unwrap(), target, SHORT, carried).unwrap();
    let (mut fed, mut lost) = (false, Vec::new());
    let quiet = Instant::now() + Duration::from_secs(5);
    while Instant::now() < quiet {
        if let Some(update) = restarted.next_update(Duration::from_millis(200)) {
            fed |= update
                .event
                .as_ref()
                .and_then(|event| event.fed.as_ref())
                .is_some_and(|fed| fed.requested == 2);
            assert_ne!(update.kind, UpdateKind::Settled, "F's `Fed` settled a feed");
            if let Some(pending) = &update.pending {
                lost.push((update.kind, pending.id.clone()));
            }
        }
    }
    assert!(fed, "F ran, and its `Fed` was read");
    // F is lost after the restart: announced so, and reported still
    // unknown once the backlog shows the feeder idle, never settled.
    assert_eq!(
        lost,
        [
            (UpdateKind::Pending, f.clone()),
            (UpdateKind::StillUnknown, f)
        ]
    );
}

/// A lost feed carried from the last run, released by the owner a week
/// ago, is forgotten, still unknown.
#[test]
fn a_lost_feed_a_week_old_is_forgotten() {
    let (sim, key) = joined();
    let target = Target {
        identity: sim.identity,
        address: Some(sim.address.to_string()),
    };
    let week = 7 * 24 * 3_600_000;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let old = Pending {
        id: "old".into(),
        portions: 1,
        sent_ms: i64::try_from(now_ms).unwrap() - week - 60_000,
        after: Some(1),
        reported: true,
        released_ms: Some(i64::try_from(now_ms).unwrap() - week - 1000),
        token_id: None,
    };
    let carried = Carried {
        pending: vec![old.clone()],
        answered: Vec::new(),
        unread: false,
    };
    let feeder = Feeder::start(key, target, SHORT, carried).unwrap();
    let which = until(&feeder, |update| {
        assert_ne!(update.kind, UpdateKind::StillUnknown, "reported before");
        (update.kind == UpdateKind::Forgotten).then(|| update.pending.clone())
    });
    assert_eq!(which, Some(old));
}

/// Home Assistant restarts while a feed's answer is lost: the feed it kept
/// pending is handed to the next session, whose backlog holds its `Fed`.
#[test]
fn a_lost_feed_carried_across_a_restart_settles_in_the_next_session() {
    let (sim, key) = joined();
    let secret = key.secret().to_vec();
    let relays = relays();
    let relay = relays.block_on(Relay::start(sim.address));
    let feeder = session(&sim, key, relay.address, SHORT);
    state_where(&feeder, |_| true);

    relay.hold_answers();
    let written = feeder.feed(1);
    assert_eq!(written.result, WriteResult::Uncertain);
    let pending = written.pending.expect("an uncertain feed is kept pending");
    drop(feeder);

    let target = Target {
        identity: sim.identity,
        address: Some(sim.address.to_string()),
    };
    let carried = Carried {
        pending: vec![pending.clone()],
        answered: Vec::new(),
        unread: false,
    };
    let restarted =
        Feeder::start(ClientKey::restore(&secret).unwrap(), target, SHORT, carried).unwrap();
    let (fed, which) = until(&restarted, |update| {
        assert_ne!(
            update.kind,
            UpdateKind::StillUnknown,
            "its `Fed` is in the history"
        );
        (update.kind == UpdateKind::Settled).then(|| (update.feed.clone(), update.pending.clone()))
    });
    assert_eq!(which, Some(pending));
    assert_eq!(fed.map(|fed| fed.completed), Some(1));
}

/// A feed answered before a lost one went out, whose `Fed` comes after it,
/// settles nothing after a restart: the answered numbers are carried, and
/// the lost feed, which the feeder refused while busy, is still unknown.
#[test]
fn an_answered_feeds_fed_does_not_settle_a_lost_one_after_a_restart() {
    let (sim, key) = joined();
    let secret = key.secret().to_vec();
    let relays = relays();
    let relay = relays.block_on(Relay::start(sim.address));
    let feeder = session(&sim, key, relay.address, SHORT);
    state_where(&feeder, |_| true);

    // B runs for two seconds; A goes out while it does, and its answer
    // is lost.
    let answered = feeder.feed(10);
    assert_eq!(answered.result, WriteResult::Feeding);
    let b = answered.feed.expect("the feeder numbered B");
    relay.hold_answers();
    let lost = feeder.feed(1);
    let pending = lost.pending.expect("an uncertain feed is kept pending");
    let after = pending.after.expect("A went out after a known event");
    drop(feeder);
    std::thread::sleep(Duration::from_secs(3));

    let target = Target {
        identity: sim.identity,
        address: Some(sim.address.to_string()),
    };
    let carried = Carried {
        pending: vec![pending.clone()],
        answered: vec![b],
        unread: false,
    };
    let restarted =
        Feeder::start(ClientKey::restore(&secret).unwrap(), target, SHORT, carried).unwrap();
    let mut b_after = false;
    let which = until(&restarted, |update| {
        if let Some(event) = &update.event
            && event.fed.as_ref().is_some_and(|fed| fed.feed == b)
        {
            // B's `Fed` comes after A went out: without the answered
            // numbers it would settle A.
            b_after = event.seq > after;
        }
        assert_ne!(update.kind, UpdateKind::Settled, "B's `Fed` settled A");
        (update.kind == UpdateKind::StillUnknown).then(|| update.pending.clone())
    });
    assert!(b_after, "B's `Fed` was replayed after A went out");
    assert_eq!(which.map(|lost| lost.id), Some(pending.id));
}

/// While a feed's answer is lost, a second feed is not sent: a `Fed` names
/// this client, not the request, so with two unsettled the first `Fed` could
/// settle the wrong one. Once the first settles, a feed goes again.
#[test]
fn a_feed_is_held_while_another_is_uncertain() {
    let (sim, key) = joined();
    let relays = relays();
    let relay = relays.block_on(Relay::start(sim.address));
    let feeder = session(&sim, key, relay.address, SHORT);
    state_where(&feeder, |_| true);

    relay.hold_answers();
    let lost = feeder.feed(1);
    assert_eq!(lost.result, WriteResult::Uncertain);
    let passed = relay.passed();
    assert_eq!(feeder.feed(1), Written::not_sent(NotSent::Held));
    assert_eq!(relay.passed(), passed, "nothing went out");

    relay.resume();
    until(&feeder, |update| {
        (update.kind == UpdateKind::Settled).then_some(())
    });
    assert_eq!(feeder.feed(1).result, WriteResult::Feeding);
}

/// A lost feed settled by its `Fed` hands its number on, so after a restart
/// the replay of that `Fed` settles nothing: a later lost feed, which the
/// feeder refused while busy, is still unknown, never taken for it.
#[test]
fn a_settled_feed_s_number_is_carried_so_its_replay_settles_nothing() {
    let (sim, key) = joined();
    let secret = key.secret().to_vec();
    let relays = relays();
    let relay = relays.block_on(Relay::start(sim.address));
    let feeder = session(&sim, key, relay.address, SHORT);
    state_where(&feeder, |_| true);

    // The first lost feed runs, and its `Fed` settles it.
    relay.hold_answers();
    assert_eq!(feeder.feed(1).result, WriteResult::Uncertain);
    relay.resume();
    let mut answered = Vec::new();
    let settled = until(&feeder, |update| {
        if update.kind == UpdateKind::Answered {
            answered.clone_from(&update.answered);
        }
        (update.kind == UpdateKind::Settled).then(|| update.feed.clone())
    });
    let first = settled.expect("the feed that ran").feed;
    assert_eq!(answered, [first], "announced before its settlement");

    // B runs for two seconds; a second lost feed goes out while it does,
    // and is refused unseen. The integration keeps the list as announced.
    let b = feeder.feed(10).feed.expect("the feeder numbered B");
    let answered = until(&feeder, |update| {
        (update.kind == UpdateKind::Answered && update.answered.contains(&b))
            .then(|| update.answered.clone())
    });
    assert_eq!(answered, [first, b]);
    relay.hold_answers();
    let lost = feeder.feed(1);
    let pending = lost.pending.expect("an uncertain feed is kept pending");
    drop(feeder);
    std::thread::sleep(Duration::from_secs(3));

    // The integration hands on what it was told.
    let target = Target {
        identity: sim.identity,
        address: Some(sim.address.to_string()),
    };
    let carried = Carried {
        pending: vec![pending.clone()],
        answered,
        unread: false,
    };
    let restarted =
        Feeder::start(ClientKey::restore(&secret).unwrap(), target, SHORT, carried).unwrap();
    let which = until(&restarted, |update| {
        assert_ne!(
            update.kind,
            UpdateKind::Settled,
            "a replayed `Fed` settled the refused feed"
        );
        (update.kind == UpdateKind::StillUnknown).then(|| update.pending.clone())
    });
    assert_eq!(which.map(|lost| lost.id), Some(pending.id));
}

/// A feed whose answer is lost mid-session is reported still unknown once
/// a status asked after it reads the feeder idle with no `Fed` for it, not
/// at the end of the whole settle window: the feeder refused it while
/// busy. The report lets the owner release it; until then it holds.
#[test]
fn a_lost_feed_the_feeder_never_ran_is_reported_once_it_reads_idle() {
    let (sim, key) = joined();
    let relays = relays();
    let relay = relays.block_on(Relay::start(sim.address));
    let feeder = session(&sim, key, relay.address, SHORT);
    state_where(&feeder, |_| true);

    // B runs for two seconds; A goes out while it does, and its answer is
    // lost on the way back.
    let b = feeder.feed(10).feed.expect("the feeder numbered B");
    relay.hold_answers();
    let lost = feeder.feed(1);
    assert_eq!(lost.result, WriteResult::Uncertain, "{lost:?}");
    let pending = lost.pending.expect("A is kept pending");
    assert_eq!(feeder.feed(1), Written::not_sent(NotSent::Held));
    relay.resume();

    // Far sooner than the 15 minutes `SHORT` settles by.
    let started = Instant::now();
    let mut b_ended = false;
    let reported = until(&feeder, |update| {
        b_ended |= update
            .event
            .as_ref()
            .and_then(|event| event.fed.as_ref())
            .is_some_and(|fed| fed.feed == b);
        assert_ne!(update.kind, UpdateKind::Settled, "B's `Fed` settled A");
        (update.kind == UpdateKind::StillUnknown).then(|| update.pending.clone())
    });
    assert!(b_ended, "reported only once B, which A met, had ended");
    assert!(started.elapsed() < Duration::from_secs(30));
    let reported = reported.expect("A");
    assert_eq!(reported.id, pending.id);
    assert!(reported.reported);

    assert_eq!(feeder.feed(1), Written::not_sent(NotSent::Held));
    release(&feeder, &reported);
    assert_eq!(feeder.feed(1).result, WriteResult::Feeding);
}

/// A lost feed holds new feeds back however long ago it went: past the
/// 15 minutes it is reported still unknown by, across a restart, until the
/// owner releases it. Its repair is raised again after the restart.
#[test]
fn a_lost_feed_holds_feeds_past_the_report_window_until_released() {
    let (sim, key) = joined();
    let target = Target {
        identity: sim.identity,
        address: Some(sim.address.to_string()),
    };
    let now_ms = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    let lost = Pending {
        id: "lost".into(),
        portions: 1,
        sent_ms: now_ms - 20 * 60_000,
        after: Some(1),
        reported: true,
        released_ms: None,
        token_id: None,
    };
    let carried = Carried {
        pending: vec![lost.clone()],
        answered: Vec::new(),
        unread: false,
    };
    let feeder = Feeder::start(key, target, Clocks::DEFAULT, carried).unwrap();
    let again = until(&feeder, |update| {
        (update.kind == UpdateKind::StillUnknown).then(|| update.pending.clone())
    });
    assert_eq!(again, Some(lost.clone()), "its repair comes back");
    state_where(&feeder, |_| true);

    assert_eq!(feeder.feed(1), Written::not_sent(NotSent::Held));
    release(&feeder, &lost);
    assert_eq!(feeder.feed(1).result, WriteResult::Feeding);
}

/// Whether the simulated feeder lists `key`, as its owner reads it.
fn listed(sim: &Threaded, key: [u8; 32]) -> bool {
    let Answer::Clients(clients) = owner(sim, &Op::ListClients) else {
        panic!("the owner lists the clients");
    };
    clients.iter().any(|client| client.key.0 == key)
}

/// A key an unchecked join left is checked before the next invite for its
/// feeder is spent: enrolled, it is the key, and the invite stays unspent.
/// One held for another feeder says nothing of this one and stays held.
#[test]
fn redeem_checks_a_held_key_before_it_spends_the_invite() {
    let (sim, key) = joined();
    let address = sim.address.to_string();
    let held = Unsettled {
        secret: key.secret(),
        feeder: sim.identity,
    };
    let unspent = invite(&sim);
    let redeemed = runtime()
        .block_on(redeem(
            &unspent,
            "home-assistant",
            Some(&address),
            Some(held),
        ))
        .unwrap();
    assert_eq!(redeemed.key.public(), key.public());
    assert!(redeemed.held.is_none());

    let elsewhere = Unsettled {
        secret: [7; 32],
        feeder: [9; 32],
    };
    let again = runtime()
        .block_on(redeem(
            &unspent,
            "home-assistant",
            Some(&address),
            Some(elsewhere),
        ))
        .unwrap();
    assert_ne!(
        again.key.public(),
        key.public(),
        "the invite enrolled a new key"
    );
    assert!(listed(&sim, again.key.public()));
    assert_eq!(again.held.map(|held| held.feeder), Some([9; 32]));
}

/// How many bytes a join sends the feeder, from one that went through the
/// relay: the same for every key and invite with a label of this length.
fn join_size(sim: &Threaded, relay: &Relay) -> usize {
    let before = relay.passed();
    let other = ClientKey::generate().unwrap();
    let address = relay.address.to_string();
    let joined = runtime().block_on(join(&other, &invite(sim), "home-assistant", Some(&address)));
    assert_eq!(joined.unwrap(), ClientRole::Member);
    relay.passed() - before
}

/// A join whose answer is lost on its way back is settled by a session
/// with the key, never sent again: the feeder enrolled it, so the join
/// ends as a member's.
#[test]
fn a_join_whose_answer_is_lost_is_settled_by_a_session_with_the_key() {
    let sim = Threaded::start(*Keypair::from_secret(OWNER).public());
    let relays = relays();
    let relay = relays.block_on(Relay::start(sim.address));
    let size = join_size(&sim, &relay);

    let text = invite(&sim);
    let key = ClientKey::generate().unwrap();
    let public = key.public();
    let address = relay.address.to_string();
    relay.hold_answers_after(size);
    let joining = std::thread::spawn(move || {
        runtime().block_on(join_member(&key, &text, "home-assistant", Some(&address)))
    });
    wait_for("the feeder to enroll the key", || listed(&sim, public));
    relay.cut();
    relay.resume();

    let joined = joining
        .join()
        .unwrap()
        .expect("the session found the key enrolled");
    assert_eq!(joined.role, Role::Member);
    assert!(!joined.removed);
}

/// A join whose answer is lost, with no session to check it, may have
/// enrolled the key: it ends uncertain, and a later check with the same
/// key finds it enrolled, with no second invite.
#[test]
fn a_join_no_session_could_check_is_checked_again_later() {
    let sim = Threaded::start(*Keypair::from_secret(OWNER).public());
    let relays = relays();
    let relay = relays.block_on(Relay::start(sim.address));
    let size = join_size(&sim, &relay);

    let text = invite(&sim);
    let secret = ClientKey::generate().unwrap().secret();
    let public = ClientKey::from_secret(secret).public();
    let address = relay.address.to_string();
    relay.hold_answers_after(size);
    let joining = {
        let (text, address) = (text.clone(), address.clone());
        std::thread::spawn(move || {
            let key = ClientKey::from_secret(secret);
            runtime().block_on(join_member(&key, &text, "home-assistant", Some(&address)))
        })
    };
    wait_for("the feeder to enroll the key", || listed(&sim, public));
    // The connection drops with the answer held, and the session that
    // checks it gets no answer either.
    relay.cut();
    let error = joining
        .join()
        .unwrap()
        .expect_err("no session could check it");
    assert_eq!(join_problem(&error), JoinProblem::Uncertain, "{error}");

    relay.resume();
    let key = ClientKey::from_secret(secret);
    let checked = runtime().block_on(check_join(&key, &sim.identity, Some(&address)));
    let joined = checked.unwrap().expect("the key was enrolled");
    assert_eq!(joined.role, Role::Member);
}

/// An invite whose secret is wrong ends the join as a lost answer does;
/// the session that checks it finds the key unknown, so the invite was not
/// taken.
#[test]
fn a_join_the_feeder_did_not_take_is_not_accepted() {
    let sim = Threaded::start(*Keypair::from_secret(OWNER).public());
    let Answer::Invite(invite) = owner(
        &sim,
        &Op::CreateInvite {
            role: ClientRole::Member,
        },
    ) else {
        panic!("no invite");
    };
    let wrong = Invite {
        feeder: sim.identity,
        id: invite.id,
        secret: invite.secret.0.map(|byte| !byte),
    }
    .text();

    let key = ClientKey::generate().unwrap();
    let address = sim.address.to_string();
    let error = runtime()
        .block_on(join_member(&key, &wrong, "home-assistant", Some(&address)))
        .expect_err("a wrong secret enrolls nothing");
    assert_eq!(join_problem(&error), JoinProblem::NotAccepted, "{error}");
    assert!(!listed(&sim, key.public()));
    let checked = runtime().block_on(check_join(&key, &sim.identity, Some(&address)));
    assert!(checked.unwrap().is_none());
}
