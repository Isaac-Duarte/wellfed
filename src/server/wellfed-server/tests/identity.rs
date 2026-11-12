//! The server's own HTTPS: the identity key it proves in each handshake,
//! which a client pins, and refuses when another server answers in its
//! place.

use wellfed_api::{Credentials, ErrorCode, PREFIX};
use wellfed_server_client::reqwest::header::SET_COOKIE;
use wellfed_server_client::{Error, Failure, Method, Options, StatusCode, Trust, expect_status};
use wellfed_server_testkit::{TestServer, ada_login, ada_setup, local};

#[test]
fn a_client_pinning_the_key_is_served_and_its_cookie_is_secure() {
    local(async {
        let server = TestServer::start().await;
        let mut client = server.https_client(Trust::Pinned(server.identity()));

        client.health().await.unwrap();
        assert_eq!(client.identity().await.unwrap(), server.identity());
        assert_eq!(
            server.client().identity().await.unwrap(),
            server.identity(),
            "the plain listener names the same key"
        );
        client.setup(&ada_setup()).await.unwrap();

        let answer = client
            .request(Method::POST, &format!("{PREFIX}/session"))
            .json(&ada_login())
            .send()
            .await
            .unwrap();
        let answer = expect_status(answer, StatusCode::OK).await.unwrap();
        let cookie = answer.headers()[SET_COOKIE].to_str().unwrap();
        assert!(cookie.ends_with("; Secure"), "{cookie}");
        server.stop().await;
    });
}

#[test]
fn a_server_proving_another_key_is_refused_before_anything_is_sent() {
    local(async {
        let server = TestServer::start().await;
        let mut client = server.https_client(Trust::Pinned([9; 32]));

        let refused = client
            .login(&Credentials {
                name: "ada".into(),
                password: "a password that must not leave".into(),
            })
            .await
            .unwrap_err();

        match &refused {
            Error::KeyChanged { offered } => assert_eq!(*offered, Some(server.identity())),
            other => panic!("expected the key refused, got {other:?}"),
        }
        assert_eq!(refused.failure(), Failure::Connect);
        assert_eq!(refused.code(), None::<ErrorCode>);
        server.stop().await;
    });
}

#[test]
fn first_contact_learns_the_key_and_the_systems_authorities_never_vouch_for_it() {
    local(async {
        let server = TestServer::start().await;

        let learned = wellfed_server_client::identify(&server.https_base(), Options::default())
            .await
            .unwrap();
        assert_eq!(learned, server.identity());

        let platform = server.https_client(Trust::Platform);
        let refused = platform.health().await.unwrap_err();
        assert!(matches!(refused, Error::Http(_)), "{refused:?}");
        assert_eq!(refused.failure(), Failure::Connect);
        server.stop().await;
    });
}
