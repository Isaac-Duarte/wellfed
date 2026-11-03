//! Adopting from the app over HTTP: the phone's token names the key the
//! phone already holds on the feeder, which `/phones` and the feeder's
//! clients then show as the caller's; a key the feeder does not hold is
//! refused `409 phone_key_not_listed`, the feeder adopted all the same.

use wellfed_api::{AdoptFeeder, ClientOwner, ClientRole, ErrorCode, Hex, NewToken};
use wellfed_server_client::{Auth, Client, StatusCode};
use wellfed_server_testkit::feeder::{self, Running};
use wellfed_server_testkit::{TestServer, ada_setup, local, refused};

/// The owner's phone: a client that sends only a token of the owner's.
async fn owners_phone(server: &TestServer) -> Client {
    let mut login = server.client();
    login.setup(&ada_setup()).await.unwrap();
    let new = NewToken {
        name: "phone".into(),
        expires: None,
    };
    let created = login.create_token(&new).await.unwrap();
    server.client().with_auth(Some(Auth::Token(created.secret)))
}

/// The adoption the app sends: an owner's invite it made on the feeder
/// with its own key, and that key.
async fn adoption(sim: &Running, phone_key: [u8; 32]) -> AdoptFeeder {
    AdoptFeeder {
        invite: feeder::invite(sim, ClientRole::Owner).await,
        name: "kitchen".into(),
        address: Some(sim.address.to_string()),
        phone_key: Some(Hex(phone_key)),
    }
}

#[test]
fn the_app_adopts_and_its_key_is_listed_as_the_callers_phone() {
    local(async {
        let sim = feeder::start().await;
        let server = TestServer::start().await;
        let phone = owners_phone(&server).await;
        let invite = feeder::invite(&sim, ClientRole::Owner).await;
        feeder::join_phone(&sim, &invite, [5; 32]).await;
        let key = feeder::phone_key([5; 32]);

        let kitchen = phone.adopt(&adoption(&sim, key).await).await.unwrap();

        let phones = phone.phones().await.unwrap();
        assert_eq!(phones.len(), 1);
        assert_eq!((phones[0].feeder, phones[0].key.0), (kitchen.id, key));
        let clients = phone.clients(kitchen.id).await.unwrap();
        let client = clients.iter().find(|client| client.key.0 == key).unwrap();
        assert!(matches!(
            client.owner,
            ClientOwner::Phone { revoking: None, .. }
        ));
    });
}

#[test]
fn a_key_the_feeder_does_not_hold_is_refused_after_the_adoption() {
    local(async {
        let sim = feeder::start().await;
        let server = TestServer::start().await;
        let phone = owners_phone(&server).await;

        let adopted = phone
            .adopt(&adoption(&sim, feeder::phone_key([6; 32])).await)
            .await;

        refused(adopted, StatusCode::CONFLICT, ErrorCode::PhoneKeyNotListed);
        assert_eq!(phone.feeders().await.unwrap().len(), 1);
        assert!(phone.phones().await.unwrap().is_empty());
    });
}
