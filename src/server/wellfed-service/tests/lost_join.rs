//! An adoption whose join answer is lost. The feeder may have enrolled
//! the server's key, or enroll it once a running feed ends, with the
//! connection gone. The service answers the caller at once that the
//! outcome is unknown, and goes on looking for the key in a task of its
//! own: found, the feeder is adopted; a caller that gave up, or a server
//! that stops, never leaves the key's secret dropped unlogged.

use std::time::Duration;

use wellfed_api::{AdoptFeeder, ClientRole, Connection as Link, Model};
use wellfed_mirror::Invite;
use wellfed_model::{Answer, Op};
use wellfed_server_testkit::feeder::{self, Relay, Running, invite, open_owner};
use wellfed_server_testkit::{TestService, WAIT, ada, config, local, until};
use wellfed_service::{Caller, Error, FeederError, Service};

fn owner() -> Caller {
    ada().into()
}

fn adopt(invite: String, address: String) -> AdoptFeeder {
    AdoptFeeder {
        invite,
        name: "kitchen".into(),
        address: Some(address),
        phone_key: None,
    }
}

fn uncertain<T: std::fmt::Debug>(result: &Result<T, Error>) -> bool {
    matches!(result, Err(Error::Feeder(FeederError::JoinUncertain)))
}

/// A real invite, and the relay budget that lets its join through whole
/// and holds the answer to it. An invite the feeder does not know is
/// dropped once its opening arrives, which measures the opening.
async fn measured(service: &Service, sim: &Running, relay: &Relay) -> (String, usize) {
    let real = invite(sim, ClientRole::Member).await;
    let parsed = Invite::parse(&real).unwrap();
    let unknown = Invite {
        id: parsed.id ^ 1,
        ..parsed
    };
    let address = relay.address.to_string();

    let before = relay.passed();
    let dropped = service
        .adopt(&owner(), adopt(unknown.text(), address))
        .await;
    assert!(
        matches!(dropped, Err(Error::Feeder(FeederError::InviteExpired))),
        "{dropped:?}"
    );
    (real, relay.passed() - before + 1)
}

/// The feeder lists one key besides its owner's: the server's.
async fn one_key(sim: &Running) {
    let mut owner = open_owner(sim).await;
    let Answer::Clients(clients) = owner.ask([3; 16], &Op::ListClients).await.unwrap() else {
        panic!("the clients were not listed");
    };
    let others = clients.iter().filter(|c| c.role != ClientRole::Owner);
    assert_eq!(others.count(), 1, "{clients:?}");
}

/// The caller is answered as soon as the answer is lost: the outcome is
/// unknown. The key the feeder enrolled is then found, and the feeder
/// adopted with the role and model a session names.
#[test]
fn a_lost_join_answer_is_unknown_and_the_feeder_is_adopted_once_found() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        server.ensure_home().await;
        let relay = Relay::start(sim.address).await;
        let service = server.service();
        let (real, budget) = measured(service, &sim, &relay).await;

        relay.hold_answers_after(budget);
        let lose = async {
            until("the join's answer is held", || async {
                relay.holding_answers()
            })
            .await;
            relay.cut();
            relay.resume();
        };
        let caller = owner();
        let adopting = service.adopt(&caller, adopt(real, relay.address.to_string()));
        let (adopted, ()) = tokio::join!(adopting, lose);
        assert!(uncertain(&adopted), "{adopted:?}");

        until("the feeder is adopted", || async {
            !service.feeders().await.unwrap().is_empty()
        })
        .await;
        let feeder = service.feeders().await.unwrap().remove(0);
        assert_eq!(feeder.role, ClientRole::Member);
        assert_eq!(feeder.model, Model::Plaf103.into());
        server.link(feeder.id, Link::Online).await;
        one_key(&sim).await;
        server.shutdown().await;
        sim.stop().await;
    });
}

/// A caller that gives up while the join waits for its answer leaves the
/// adoption running: the answer is lost, the key found, the feeder stored.
#[test]
fn an_adoption_its_caller_dropped_still_stores_the_feeder() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        server.ensure_home().await;
        let relay = Relay::start(sim.address).await;
        let service = server.service();
        let (real, budget) = measured(service, &sim, &relay).await;

        relay.hold_answers_after(budget);
        let caller = owner();
        let adopting = service.adopt(&caller, adopt(real, relay.address.to_string()));
        let held = async {
            until("the join's answer is held", || async {
                relay.holding_answers()
            })
            .await;
        };
        tokio::select! {
            result = adopting => panic!("the adoption ended while its answer was held: {result:?}"),
            () = held => {}
        }
        relay.cut();
        relay.resume();

        until("the feeder is adopted", || async {
            !service.feeders().await.unwrap().is_empty()
        })
        .await;
        let feeder = service.feeders().await.unwrap().remove(0);
        server.link(feeder.id, Link::Online).await;
        one_key(&sim).await;
        server.shutdown().await;
        sim.stop().await;
    });
}

/// A wrong invite secret looks like a lost answer to a client: the feeder
/// takes the handshake, drops the join unanswered and never enrolls the
/// key. The caller hears the outcome is unknown at once, nothing is
/// stored, and a server stopping during the long search ends it.
#[test]
fn a_search_for_a_key_never_enrolled_ends_with_the_server() {
    local(async {
        let sim = feeder::start().await;
        let mut long = config();
        long.timing.join_settle = Duration::from_secs(600);
        let server = TestService::start_with(long).await;
        server.ensure_home().await;
        let real = Invite::parse(&invite(&sim, ClientRole::Member).await).unwrap();
        let wrong = Invite {
            secret: [0x77; 32],
            ..real
        };

        let service = server.service();
        let caller = owner();
        let answered = tokio::time::timeout(
            WAIT,
            service.adopt(&caller, adopt(wrong.text(), sim.address.to_string())),
        );
        let adopted = answered.await.expect("answered within the clients' wait");
        assert!(uncertain(&adopted), "{adopted:?}");
        assert!(service.feeders().await.unwrap().is_empty());

        let stopped = tokio::time::timeout(WAIT, server.shutdown()).await;
        assert!(stopped.is_ok(), "the search ended with the server");
        sim.stop().await;
    });
}
