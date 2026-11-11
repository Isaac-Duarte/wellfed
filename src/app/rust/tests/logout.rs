//! A phone's logout against the server in process, in front of the
//! simulated feeder (simulation, not hardware): a phone's own key asks the
//! feeder for an invite and a pet sitter's phone joins with it, which the
//! server records under the phone's token. Logging out with the password
//! keeps both keys on the feeder, as kept; a wrong password changes
//! nothing; revoking the token from elsewhere revokes both.

use wellfed_api::{AdoptFeeder, ClientOwner, ClientRole, FeederId, Hex};
use wellfed_app_bridge::api::server::ServerSession;
use wellfed_app_bridge::api::server_error::ServerErrorKind;
use wellfed_server_client::{Auth, Client};
use wellfed_server_testkit::{TestServer, ada_login, ada_setup, feeder, local, until};

const OWN: [u8; 32] = [1; 32];
const SITTER: [u8; 32] = [2; 32];

/// The server with a feeder its owner adopted, the owner's client, and
/// the phone logged in, holding its own key and the sitter's it invited.
async fn phone_with_a_sitter(
    sim: &feeder::Running,
) -> (TestServer, Client, FeederId, ServerSession) {
    let server = TestServer::start().await;
    let mut owner = server.client();
    owner.setup(&ada_setup()).await.unwrap();
    let adopt = AdoptFeeder {
        invite: feeder::invite(sim, ClientRole::Owner).await,
        name: "kitchen".into(),
        address: Some(sim.address.to_string()),
        phone_key: None,
    };
    let kitchen = owner.adopt(&adopt).await.unwrap().id;
    server.caught_up(kitchen).await;

    let credentials = ada_login();
    let url = format!("http://{}", server.address());
    let session = ServerSession::login(url, credentials.name, credentials.password, "pixel".into())
        .await
        .unwrap();
    let token = server
        .client()
        .with_auth(Some(Auth::Token(session.secret())));
    let invite = token
        .invite_phone(kitchen, Hex(feeder::phone_key(OWN)))
        .await
        .unwrap();
    feeder::join_phone(sim, &invite.invite, OWN).await;
    let sitter = feeder::phone_invite(sim, OWN, ClientRole::Member).await;
    feeder::join_phone(sim, &sitter, SITTER).await;
    until("the sitter's phone is the token's", || async {
        let phones = owner.phones().await.unwrap();
        phones
            .iter()
            .any(|phone| phone.key.0 == feeder::phone_key(SITTER))
    })
    .await;
    (server, owner, kitchen, session)
}

#[test]
fn a_logout_keeps_the_phones_key_and_its_sitters() {
    local(async {
        let sim = feeder::start().await;
        let (server, owner, kitchen, session) = phone_with_a_sitter(&sim).await;

        let wrong = session.logout("not the password".into()).await.unwrap_err();
        assert_eq!(wrong.kind, ServerErrorKind::BadCredentials);
        assert!(session.account().await.is_ok(), "the token still holds");

        session.logout(ada_login().password).await.unwrap();
        let clients = owner.clients(kitchen).await.unwrap();
        for secret in [OWN, SITTER] {
            let key = feeder::phone_key(secret);
            let client = clients.iter().find(|client| client.key.0 == key);
            assert_eq!(client.map(|client| &client.owner), Some(&ClientOwner::Kept));
        }
        assert!(owner.phones().await.unwrap().is_empty());
        server.stop().await;
        sim.stop().await;
    });
}

#[test]
fn revoking_the_token_from_elsewhere_revokes_both_keys() {
    local(async {
        let sim = feeder::start().await;
        let (server, mut owner, _kitchen, session) = phone_with_a_sitter(&sim).await;

        // Another session of the same account, as on a lost phone.
        owner.login(&ada_login()).await.unwrap();
        let id = session.link().token_id.parse().unwrap();
        owner.revoke_token(wellfed_api::TokenId(id)).await.unwrap();
        until("both keys are revoked", || async {
            let held = feeder::client_keys(&sim).await;
            [OWN, SITTER]
                .iter()
                .all(|&secret| !held.contains(&feeder::phone_key(secret)))
        })
        .await;
        server.stop().await;
        sim.stop().await;
    });
}
