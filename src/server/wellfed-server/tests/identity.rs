//! The server's own HTTPS: the identity key it proves in each handshake,
//! which a client pins, and refuses when another server answers in its
//! place.

use std::net::SocketAddr;
use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use wellfed_api::{AuditAction, Credentials, ErrorCode, PREFIX, PageQuery};
use wellfed_server::identity::Identity;
use wellfed_server_client::reqwest::header::SET_COOKIE;
use wellfed_server_client::{
    Client, Error, Failure, Method, Options, StatusCode, Trust, Url, expect_status,
};
use wellfed_server_testkit::{TestServer, ada_login, ada_setup, config, local};

/// A machine in front of the real server's plain listener: it ends TLS as
/// `tls` says, then passes the bytes on, so anything a client sent through
/// it would reach the real server, and its audit.
async fn front(server: &TestServer, tls: rustls::ServerConfig) -> Url {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address: SocketAddr = listener.local_addr().unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
    let behind = server.address();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(stream).await else {
                    return;
                };
                let Ok(mut real) = tokio::net::TcpStream::connect(behind).await else {
                    return;
                };
                let _ = tokio::io::copy_bidirectional(&mut tls, &mut real).await;
            });
        }
    });
    Url::parse(&format!("https://{address}")).unwrap()
}

/// Presents one certificate and signs with one key, whether or not they
/// match: rustls's own check that they do is what this gets past.
#[derive(Debug)]
struct Presenting(Arc<CertifiedKey>);

impl ResolvesServerCert for Presenting {
    fn resolve(&self, _: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.0.clone())
    }
}

/// A fresh Ed25519 key, as PKCS#8, and its public half.
fn other_key() -> (PrivatePkcs8KeyDer<'static>, [u8; 32]) {
    let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
    let public: [u8; 32] = pair.public_key_raw().try_into().unwrap();
    (PrivatePkcs8KeyDer::from(pair.serialize_der()), public)
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

/// The logins and failed logins the server's audit holds.
async fn logins(server: &TestServer) -> usize {
    let mut owner = server.client();
    owner.login(&ada_login()).await.unwrap();
    let log = owner.audit(&PageQuery::default()).await.unwrap();
    log.items
        .iter()
        .filter(|entry| {
            matches!(
                entry.action,
                AuditAction::Login { .. } | AuditAction::LoginFailed { .. }
            )
        })
        .count()
}

fn pinned(base: &Url, key: [u8; 32]) -> Client {
    let options = Options {
        trust: Trust::Pinned(key),
        ..Options::default()
    };
    Client::with_options(base, options).unwrap()
}

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
        assert!(refused.untrusted_certificate(), "{refused:?}");
        assert_eq!(refused.failure(), Failure::Connect);
        server.stop().await;
    });
}

/// A machine that presents the real server's own certificate, naming the
/// pinned key, but cannot sign with that key: the handshake fails as
/// `KeyChanged`, and no password reaches the server behind it.
#[test]
fn the_real_certificate_with_another_signer_is_refused_before_the_password() {
    local(async {
        let server = TestServer::start().await;
        server.client().setup(&ada_setup()).await.unwrap();
        let real = Identity::derive(&config().key);
        assert_eq!(real.public(), server.identity());
        let (signer, _) = other_key();
        let signer = rustls::crypto::aws_lc_rs::sign::any_eddsa_type(&signer).unwrap();
        let presenting = CertifiedKey::new(vec![real.certificate().unwrap()], signer);
        let tls = rustls::ServerConfig::builder_with_provider(provider())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(Presenting(Arc::new(presenting))));
        let impostor = front(&server, tls).await;
        let before = logins(&server).await;

        let mut client = pinned(&impostor, server.identity());
        let refused = client.login(&ada_login()).await.unwrap_err();

        assert!(
            matches!(refused, Error::KeyChanged { offered: None }),
            "{refused:?}"
        );
        assert_eq!(refused.failure(), Failure::Connect);
        assert_eq!(
            logins(&server).await,
            before + 1,
            "the audit's own read only"
        );
        server.stop().await;
    });
}

/// A machine that holds the pinned key but speaks only TLS 1.2 is refused,
/// while the same machine over TLS 1.3 is served: the client speaks 1.3
/// alone, as the server's own HTTPS does.
#[test]
fn a_server_speaking_only_tls_1_2_is_refused() {
    local(async {
        let server = TestServer::start().await;
        server.client().setup(&ada_setup()).await.unwrap();
        let (key, public) = other_key();
        let pair = rcgen::KeyPair::try_from(key.secret_pkcs8_der()).unwrap();
        let params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        let certificate: CertificateDer<'static> = params.self_signed(&pair).unwrap().der().clone();
        let config = |version: &'static rustls::SupportedProtocolVersion| {
            rustls::ServerConfig::builder_with_provider(provider())
                .with_protocol_versions(&[version])
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(
                    vec![certificate.clone()],
                    PrivateKeyDer::Pkcs8(key.clone_key()),
                )
                .unwrap()
        };
        let old = front(&server, config(&rustls::version::TLS12)).await;
        let current = front(&server, config(&rustls::version::TLS13)).await;
        let before = logins(&server).await;

        let refused = pinned(&old, public).login(&ada_login()).await.unwrap_err();
        assert!(matches!(refused, Error::Http(_)), "{refused:?}");
        assert_eq!(refused.failure(), Failure::Connect);
        assert_eq!(
            logins(&server).await,
            before + 1,
            "the audit's own read only"
        );

        pinned(&current, public).health().await.unwrap();
        server.stop().await;
    });
}
