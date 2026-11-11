//! A logout names the phones this phone invited, against the server in
//! process in front of the simulated feeder (simulation, not hardware): a
//! phone's own key asks the feeder for an invite, a pet sitter's phone
//! joins with it, and the server records it under this phone's token.
//! Revoking the token on logout revokes both keys, this phone's own too.

use wellfed_api::{AdoptFeeder, ClientRole, Hex};
use wellfed_app_bridge::api::keys::ClientKey;
use wellfed_app_bridge::api::server::ServerSession;
use wellfed_server_client::Auth;
use wellfed_server_testkit::{TestServer, ada_login, ada_setup, feeder, local, until};

const OWN: [u8; 32] = [1; 32];
const SITTER: [u8; 32] = [2; 32];

#[test]
fn a_logout_can_name_the_phones_this_phone_invited() {
    local(async {
        let sim = feeder::start().await;
        let server = TestServer::start().await;
        let mut owner = server.client();
        owner.setup(&ada_setup()).await.unwrap();
        let adopt = AdoptFeeder {
            invite: feeder::invite(&sim, ClientRole::Owner).await,
            name: "kitchen".into(),
            address: Some(sim.address.to_string()),
            phone_key: None,
        };
        let kitchen = owner.adopt(&adopt).await.unwrap().id;
        server.caught_up(kitchen).await;

        let credentials = ada_login();
        let url = format!("http://{}", server.address());
        let session =
            ServerSession::login(url, credentials.name, credentials.password, "pixel".into())
                .await
                .unwrap();
        let own = ClientKey::restore(OWN.to_vec()).unwrap();
        let token = server
            .client()
            .with_auth(Some(Auth::Token(session.secret())));
        let invite = token
            .invite_phone(kitchen, Hex(feeder::phone_key(OWN)))
            .await
            .unwrap();
        feeder::join_phone(&sim, &invite.invite, OWN).await;
        assert!(session.invited_phones(&own).await.unwrap().is_empty());

        let sitter = feeder::phone_invite(&sim, OWN, ClientRole::Member).await;
        feeder::join_phone(&sim, &sitter, SITTER).await;
        until("the sitter's phone is this phone's", || async {
            session.invited_phones(&own).await.unwrap() == [kitchen.to_string()]
        })
        .await;

        session.logout().await.unwrap();
        until("both keys are revoked", || async {
            let held = feeder::client_keys(&sim).await;
            let gone = |secret| !held.contains(&feeder::phone_key(secret));
            gone(OWN) && gone(SITTER)
        })
        .await;
        server.stop().await;
        sim.stop().await;
    });
}
