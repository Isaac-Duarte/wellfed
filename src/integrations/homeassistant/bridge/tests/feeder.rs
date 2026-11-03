//! The bridge against the simulated feeder, which runs the real feeder's
//! pairing and feeding owner, in real time on a thread of its own:
//! - an owner's member invite enrolls the integration's new key;
//! - the session reports its status, runs a feed to its `Fed` event and
//!   changes the lock;
//! - a write while the feeder is gone is refused unsent, never held.

use std::time::{Duration, Instant};

use wellfed_client::api::Invite;
use wellfed_connect::Connection;
use wellfed_feeder_sim::harness::Threaded;
use wellfed_homeassistant::feeder::{Feeder, Target, Update, Written};
use wellfed_homeassistant::key::{ClientKey, invite_feeder, join};
use wellfed_model::{Answer, ClientRole, EventBody, Op};
use wellfed_session::noise::Keypair;

const OWNER: [u8; 32] = [0x21; 32];
const WAIT: Duration = Duration::from_secs(60);

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// A member invite from the simulator's owner, as `wellfed invite` prints it.
fn invite(sim: &Threaded) -> String {
    runtime().block_on(async {
        let owner = Keypair::from_secret(OWNER);
        let mut connection = Connection::open(&owner, sim.identity, sim.address, [0x41; 32])
            .await
            .unwrap();
        let answer = connection
            .ask(
                [0x51; 16],
                &Op::CreateInvite {
                    role: ClientRole::Member,
                },
            )
            .await
            .unwrap();
        let Answer::Invite(invite) = answer else {
            panic!("no invite: {answer:?}");
        };
        Invite {
            feeder: sim.identity,
            id: invite.id,
            secret: invite.secret.0,
        }
        .text()
    })
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

#[test]
fn the_session_feeds_to_its_fed_event_and_sets_the_lock() {
    let (sim, key) = joined();
    let restored = ClientKey::restore(&key.secret()).unwrap();
    assert_eq!(restored.public(), key.public());

    let feeder = Feeder::start(
        restored,
        Target {
            identity: sim.identity,
            address: Some(sim.address.to_string()),
        },
    )
    .unwrap();
    let role = until(&feeder, |update| match update {
        Update::Connected { role, .. } => Some(*role),
        _ => None,
    });
    assert_eq!(role, ClientRole::Member);
    until(&feeder, |update| {
        matches!(update, Update::Status { .. }).then_some(())
    });

    let Written::Feeding { feed } = feeder.feed(1) else {
        panic!("the feed did not start");
    };
    let delivered = until(&feeder, |update| match update {
        Update::Event { event } => match &event.body {
            EventBody::Fed {
                feed: fed,
                completed,
                ..
            } if *fed == feed => Some(*completed),
            _ => None,
        },
        _ => None,
    });
    assert_eq!(delivered, 1);

    assert_eq!(feeder.set_lock(true), Written::Done);
    let locked = until(&feeder, |update| match update {
        Update::Status { status } => status.locked.filter(|locked| *locked),
        _ => None,
    });
    assert!(locked);
}

#[test]
fn a_write_while_the_feeder_is_gone_is_not_sent() {
    let (sim, key) = joined();
    let feeder = Feeder::start(
        key,
        Target {
            identity: sim.identity,
            address: Some(sim.address.to_string()),
        },
    )
    .unwrap();
    until(&feeder, |update| {
        matches!(update, Update::Status { .. }).then_some(())
    });

    drop(sim);
    until(&feeder, |update| {
        matches!(update, Update::Disconnected { .. }).then_some(())
    });
    assert!(matches!(feeder.feed(1), Written::NotSent { .. }));
}

#[test]
fn a_feed_whose_answer_is_lost_is_uncertain_and_not_sent_again() {
    let (sim, key) = joined();
    // The relay's tasks run on this runtime's workers for the whole test.
    let relays = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let relay = relays.block_on(wellfed_feeder_sim::harness::Relay::start(sim.address));
    let feeder = Feeder::start(
        key,
        Target {
            identity: sim.identity,
            address: Some(relay.address.to_string()),
        },
    )
    .unwrap();
    until(&feeder, |update| {
        matches!(update, Update::Status { .. }).then_some(())
    });

    relay.hold_answers();
    let passed = relay.passed();
    assert_eq!(feeder.feed(1), Written::Uncertain);

    // The feed reached the feeder once and ran there; the bridge sent it once.
    relay.resume();
    until(&feeder, |update| match update {
        Update::Event { event } => matches!(event.body, EventBody::Fed { .. }).then_some(()),
        _ => None,
    });
    assert!(relay.passed() > passed);
    let quiet = Instant::now() + Duration::from_secs(3);
    while Instant::now() < quiet {
        if let Some(Update::Event { event }) = feeder.next_update(Duration::from_millis(200)) {
            assert!(
                !matches!(event.body, EventBody::Fed { .. }),
                "a second feed ran"
            );
        }
    }
}
