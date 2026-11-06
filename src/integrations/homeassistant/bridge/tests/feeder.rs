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
//!   revoked key stops the session.

use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use wellfed_client::api::Invite;
use wellfed_connect::{Connection, Timing};
use wellfed_feeder_sim::harness::{Relay, Threaded};
use wellfed_feeder_sim::serve::Control;
use wellfed_homeassistant::api::{
    EventKind, FeedOutcome, NotSent, RefusalKind, Role, State, Update, UpdateKind, WriteResult,
    Written,
};
use wellfed_homeassistant::feeder::{Clocks, Feeder, Target};
use wellfed_homeassistant::key::{ClientKey, invite_feeder, join, join_member};
use wellfed_model::{
    Answer, ClientRole, FeederSettingsPatch, Generation, Op, PLANS, Plan, Pushed, Weekdays,
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
    Feeder::start(key, target, clocks).unwrap()
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
    assert_eq!(feeder.feed(1), Written::uncertain());

    // The feed reached the feeder once and ran there; the bridge sent it
    // once, and its `Fed` settles it.
    relay.resume();
    let settled = until(&feeder, |update| {
        (update.kind == UpdateKind::Settled).then(|| update.feed.clone())
    });
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
    assert_eq!(fed.join().unwrap(), Written::uncertain());

    // The next session's backlog holds no `Fed` for it, and the feeder is
    // not feeding: it is reported unknown, with no count.
    let portions = until(&feeder, |update| {
        assert_ne!(update.kind, UpdateKind::Settled, "nothing ran to settle it");
        (update.kind == UpdateKind::StillUnknown).then_some(update.portions)
    });
    assert_eq!(portions, Some(1));
}
