//! Adopting a feeder by invite, the connect sequence, and a feeder that
//! no longer knows the server.

use wellfed_api::{
    AdoptFeeder, ClientRole, Connection as Link, CoreEvent, DeviceKind, EventBody, EventQuery,
    Model,
};
use wellfed_mirror::Invite;
use wellfed_model::{Answer, Op};
use wellfed_server_testkit::feeder::{self, Relay, invite, open_owner};
use wellfed_server_testkit::fixed::NOW_MS;
use wellfed_server_testkit::{TestService, ada, local, until};
use wellfed_service::{Caller, CommandError, Error, FeederError, Invalid};

/// The owner, adopting with no phone key.
fn owner() -> Caller {
    ada().into()
}

#[test]
fn an_adoption_stores_the_model_the_redemption_names_in_the_home() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        server.ensure_home().await;
        let service = server.service();

        // Adoption opens no session of its own: the row names the model
        // the redemption's answer gave, before the mirror's first session.
        let invite = invite(&sim, ClientRole::Server).await;
        let request = AdoptFeeder {
            invite,
            name: "kitchen".into(),
            address: Some(sim.address.to_string()),
            phone_key: None,
        };
        let adopted = service.adopt(&owner(), request).await.unwrap();

        let store = server.store().open().await;
        let row = store
            .feeder(adopted.id)
            .await
            .unwrap()
            .expect("the device is stored");
        assert_eq!(row.model, Model::Plaf103);
        assert_eq!(row.kind, DeviceKind::Feeder);
        assert_eq!(row.home, service.home().await.unwrap().id);
        assert_eq!(
            (adopted.model, adopted.kind),
            (row.model.into(), row.kind.into())
        );
    });
}

#[test]
fn an_adopted_feeder_connects_syncs_its_clock_and_mirrors_its_history() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        let id = server.adopt(&sim).await;
        server.link(id, Link::Online).await;

        let detail = server.service().feeder(id).await.unwrap();
        let status = detail.status.unwrap();
        assert!(status.live);
        assert_eq!(
            status
                .status
                .clock
                .map(|clock| clock.as_millisecond() / 1000),
            Some(NOW_MS / 1000),
            "the server set the feeder's clock on connect"
        );

        // Boot, the server's enrolment, the clock sync.
        server.synced(id, status.status.last_event.unwrap()).await;
        let history = server.bodies(id).await;
        assert!(matches!(
            history[0],
            EventBody::Core(CoreEvent::Boot { .. })
        ));
        assert!(history.iter().any(|body| matches!(
            body,
            EventBody::Core(CoreEvent::Enrolled {
                role: ClientRole::Server,
                ..
            })
        )));
        assert!(
            history
                .iter()
                .any(|body| matches!(body, EventBody::Core(CoreEvent::ClockSync { .. })))
        );

        let owner = owner();
        let again = server.service().adopt(
            &owner,
            AdoptFeeder {
                invite: invite(&sim, ClientRole::Server).await,
                name: "again".into(),
                address: Some(sim.address.to_string()),
                phone_key: None,
            },
        );
        assert!(matches!(
            again.await,
            Err(Error::Feeder(FeederError::AlreadyAdopted))
        ));
        server.shutdown().await;
        sim.stop().await;
    });
}

#[test]
fn a_revoked_server_key_is_refused() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        let id = server.adopt(&sim).await;
        server.link(id, Link::Online).await;

        let mut owner = open_owner(&sim).await;
        let answer = owner.ask([1; 16], &Op::ListClients).await.unwrap();
        let Answer::Clients(clients) = answer else {
            panic!("{answer:?}");
        };
        let key = clients
            .iter()
            .find(|client| client.role == ClientRole::Server)
            .unwrap()
            .key
            .0;
        let answer = owner.ask([2; 16], &Op::RevokeClient { key }).await.unwrap();
        assert_eq!(answer, Answer::Done { generation: None });

        server.link(id, Link::Refused).await;
        assert!(matches!(
            server
                .service()
                .set_lock(&ada().into(), id, true, Some([3; 16]))
                .await,
            Err(Error::Command(CommandError::Offline))
        ));
        server.shutdown().await;
        sim.stop().await;
    });
}

#[test]
fn an_invite_for_a_member_is_adopted_with_that_role_and_bad_input_is_refused() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        server.ensure_home().await;
        let service = server.service();

        let member = service
            .adopt(
                &owner(),
                AdoptFeeder {
                    invite: invite(&sim, ClientRole::Member).await,
                    name: "hall".into(),
                    address: Some(sim.address.to_string()),
                    phone_key: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(member.role, ClientRole::Member);

        let used = invite(&sim, ClientRole::Server).await;
        let request = |invite: &str, address: Option<&str>| AdoptFeeder {
            invite: invite.into(),
            name: "x".into(),
            address: address.map(Into::into),
            phone_key: None,
        };
        assert!(matches!(
            service
                .adopt(&owner(), request("nonsense", Some("127.0.0.1")))
                .await,
            Err(Error::Feeder(FeederError::Invite(_)))
        ));
        assert!(matches!(
            service.adopt(&owner(), request(&used, None)).await,
            Err(Error::Feeder(FeederError::AlreadyAdopted))
        ));
        let other = feeder::start().await;
        let fresh = invite(&other, ClientRole::Server).await;
        assert!(matches!(
            service.adopt(&owner(), request(&fresh, None)).await,
            Err(Error::Invalid(Invalid::AddressRequired))
        ));
        assert!(matches!(
            service
                .adopt(&owner(), request(&fresh, Some("not an address")))
                .await,
            Err(Error::Invalid(Invalid::Address))
        ));
        let query = EventQuery {
            limit: Some(0),
            ..EventQuery::default()
        };
        assert!(matches!(
            service.events(member.id, query).await,
            Err(Error::Invalid(Invalid::Limit))
        ));
        server.shutdown().await;
        other.stop().await;
        sim.stop().await;
    });
}

/// A join the feeder took whose answer was lost may have enrolled the
/// server's key. The server opens a session with the key to learn that it
/// did, and the adoption stands, holding the one key the feeder enrolled.
#[test]
fn an_adoption_whose_join_answer_was_lost_learns_it_joined() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        server.ensure_home().await;
        let relay = Relay::start(sim.address).await;
        let adopt = |invite: String| AdoptFeeder {
            invite,
            name: "kitchen".into(),
            address: Some(relay.address.to_string()),
            phone_key: None,
        };

        // An invite the feeder does not know is dropped once its opening
        // arrives: that opening's size lets the real one through whole and
        // holds the answer to the join that follows it.
        let real = invite(&sim, ClientRole::Member).await;
        let parsed = Invite::parse(&real).unwrap();
        let unknown = Invite {
            id: parsed.id ^ 1,
            ..parsed
        };
        let before = relay.passed();
        let dropped = server
            .service()
            .adopt(&owner(), adopt(unknown.text()))
            .await;
        assert!(
            matches!(dropped, Err(Error::Feeder(FeederError::InviteExpired))),
            "{dropped:?}"
        );
        let opening = relay.passed() - before;
        relay.hold_answers_after(opening + 1);

        let lose = async {
            until("the join's answer is held", || async {
                relay.holding_answers()
            })
            .await;
            relay.cut();
            relay.resume();
        };
        let caller = owner();
        let adopting = server.service().adopt(&caller, adopt(real));
        let (adopted, ()) = tokio::join!(adopting, lose);
        let feeder = adopted.unwrap();
        assert_eq!(feeder.role, ClientRole::Member);
        assert_eq!(feeder.model, Model::Plaf103.into());
        server.link(feeder.id, Link::Online).await;

        let mut owner = open_owner(&sim).await;
        let Answer::Clients(clients) = owner.ask([1; 16], &Op::ListClients).await.unwrap() else {
            panic!("the clients were not listed");
        };
        let others = clients.iter().filter(|c| c.role != ClientRole::Owner);
        assert_eq!(others.count(), 1, "{clients:?}");
        server.shutdown().await;
        sim.stop().await;
    });
}
