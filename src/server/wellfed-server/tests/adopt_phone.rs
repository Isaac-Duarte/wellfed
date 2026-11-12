//! Adopting and claiming from the app over HTTP: the phone's token names
//! the key the phone already holds on the feeder, which `/phones` and the
//! feeder's clients then show as the caller's, with the role the feeder
//! lists; a key the feeder does not hold is refused
//! `409 phone_key_not_listed`, the feeder adopted all the same, and a
//! claim of the right key afterwards settles it.

use wellfed_api::{
    AdoptFeeder, ClientOwner, ClientRole, ErrorCode, Hex, NewToken, PhoneClaim, TokenId, User,
};
use wellfed_server_client::{Auth, Client, StatusCode};
use wellfed_server_testkit::feeder::{self, Running};
use wellfed_server_testkit::{TestServer, ada_setup, local, refused};

/// The owner's phone: a client that sends only a token of the owner's,
/// the owner and the token's id.
async fn owners_phone(server: &TestServer) -> (Client, User, TokenId) {
    let mut login = server.client();
    let owner = login.setup(&ada_setup()).await.unwrap();
    let new = NewToken {
        name: "phone".into(),
        expires: None,
    };
    let created = login.create_token(&new).await.unwrap();
    let phone = server.client().with_auth(Some(Auth::Token(created.secret)));
    (phone, owner, created.token.id)
}

/// Phone `n` paired with `sim` before any server, as its owner.
async fn paired(sim: &Running, n: u8) -> [u8; 32] {
    let invite = feeder::invite(sim, ClientRole::Owner).await;
    feeder::join_phone(sim, &invite, [n; 32]).await;
    feeder::phone_key([n; 32])
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
fn the_app_adopts_and_its_key_is_recorded_as_the_callers_phone() {
    local(async {
        let sim = feeder::start().await;
        let server = TestServer::start().await;
        let (phone, owner, token) = owners_phone(&server).await;
        let key = paired(&sim, 5).await;

        let kitchen = phone.adopt(&adoption(&sim, key).await).await.unwrap();

        let phones = phone.phones().await.unwrap();
        let [recorded] = phones.as_slice() else {
            panic!("one phone key, got {phones:?}");
        };
        assert_eq!((recorded.feeder, recorded.key.0), (kitchen.id, key));
        assert_eq!(recorded.user, owner.id);
        assert_eq!(recorded.token, Some(token));
        assert_eq!(recorded.role, ClientRole::Owner);
        let clients = phone.clients(kitchen.id).await.unwrap();
        let client = clients.iter().find(|client| client.key.0 == key).unwrap();
        let owners = ClientOwner::Phone {
            user: owner.id,
            token: Some(token),
            revoking: None,
        };
        assert_eq!(client.owner, owners);
    });
}

#[test]
fn a_key_the_feeder_does_not_hold_is_refused_and_a_claim_settles_it() {
    local(async {
        let sim = feeder::start().await;
        let server = TestServer::start().await;
        let (phone, owner, token) = owners_phone(&server).await;
        let key = paired(&sim, 6).await;

        let adopted = phone
            .adopt(&adoption(&sim, feeder::phone_key([7; 32])).await)
            .await;

        refused(adopted, StatusCode::CONFLICT, ErrorCode::PhoneKeyNotListed);
        let feeders = phone.feeders().await.unwrap();
        assert_eq!(feeders.len(), 1, "the adoption stands");
        assert!(phone.phones().await.unwrap().is_empty());

        let kitchen = feeders[0].id;
        let claimed = phone
            .claim_phone(
                kitchen,
                &PhoneClaim {
                    key: Hex(key),
                    proof: None,
                },
            )
            .await
            .unwrap();
        let again = phone
            .claim_phone(
                kitchen,
                &PhoneClaim {
                    key: Hex(key),
                    proof: None,
                },
            )
            .await
            .unwrap();

        assert_eq!((claimed.user, claimed.token), (owner.id, Some(token)));
        assert_eq!(claimed.role, ClientRole::Owner);
        assert_eq!(again, claimed);
        assert_eq!(phone.phones().await.unwrap(), [claimed]);
    });
}
