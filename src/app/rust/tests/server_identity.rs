//! The phone pins the server's identity key (simulation, not hardware):
//! a login over the server's own HTTPS learns or checks the key before the
//! password goes, the link keeps it, and a server that later proves
//! another key is refused as `KeyChanged` with nothing sent.

use wellfed_api::{AuditAction, Hex, PageQuery};
use wellfed_app_bridge::api::server::{ServerSession, server_fingerprint};
use wellfed_app_bridge::api::server_error::ServerErrorKind;
use wellfed_server_testkit::{TestServer, ada_login, ada_setup, local};

/// The server with its first owner.
async fn started() -> TestServer {
    let server = TestServer::start().await;
    server.client().setup(&ada_setup()).await.unwrap();
    server
}

async fn login(server: &TestServer, key: Option<String>) -> Result<ServerSession, ServerErrorKind> {
    let credentials = ada_login();
    ServerSession::login(
        server.https_base().to_string(),
        credentials.name,
        credentials.password,
        "pixel".into(),
        key,
    )
    .await
    .map_err(|error| error.kind)
}

#[test]
fn a_typed_address_pins_the_key_its_handshake_proves() {
    local(async {
        let server = started().await;

        let session = login(&server, None).await.unwrap();

        let key = Hex(server.identity()).to_string();
        assert_eq!(session.link().key, key);
        assert_eq!(
            server_fingerprint(key).unwrap(),
            wellfed_api::fingerprint(&server.identity())
        );
        session.account().await.unwrap();
        server.stop().await;
    });
}

#[test]
fn the_announced_key_is_checked_before_the_password_goes() {
    local(async {
        let server = started().await;

        let announced = Hex(server.identity()).to_string();
        login(&server, Some(announced)).await.unwrap();

        let forged = Hex([7; 32]).to_string();
        assert_eq!(
            login(&server, Some(forged)).await.err(),
            Some(ServerErrorKind::KeyChanged)
        );

        let mut owner = server.client();
        owner.login(&ada_login()).await.unwrap();
        let log = owner.audit(&PageQuery::default()).await.unwrap();
        let logins = log
            .items
            .iter()
            .filter(|entry| {
                matches!(
                    entry.action,
                    AuditAction::Login { .. } | AuditAction::LoginFailed { .. }
                )
            })
            .count();
        assert_eq!(logins, 2, "the announced key's login and the owner's own");
        server.stop().await;
    });
}

#[test]
fn a_server_proving_another_key_is_refused_and_never_adopted() {
    local(async {
        let server = started().await;
        let session = login(&server, None).await.unwrap();

        let mut link = session.link();
        link.key = Hex([7; 32]).to_string();
        let moved = ServerSession::restore(link, session.secret()).unwrap();

        let refused = moved.account().await.unwrap_err();
        assert_eq!(refused.kind, ServerErrorKind::KeyChanged);
        assert!(!refused.may_have_run);
        assert_eq!(
            moved.link().key,
            Hex([7; 32]).to_string(),
            "the phone never takes the new key by itself"
        );
        server.stop().await;
    });
}

/// A server's own HTTPS reached as if by a public name is checked by the
/// system's authorities, which never vouch for it: its own words, not that
/// nothing answered.
#[test]
fn an_untrusted_certificate_has_its_own_kind() {
    local(async {
        let server = started().await;
        let platform = server.https_client(wellfed_server_client::Trust::Platform);

        let refused = platform.health().await.unwrap_err();

        let error = wellfed_app_bridge::api::server_error::ServerError::from(refused);
        assert_eq!(error.kind, ServerErrorKind::UntrustedCertificate);
        assert!(!error.may_have_run);
        server.stop().await;
    });
}
