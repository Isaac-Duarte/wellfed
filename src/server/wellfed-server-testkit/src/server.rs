//! The HTTP server in process, on a localhost port and its own HTTPS on
//! another, in front of a [`TestService`].

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use wellfed_api::FeederId;
use wellfed_server::identity::Identity;
use wellfed_server::{Beside, Jobs, Listeners, Serving, Tls, TrustedProxies};
use wellfed_server_client::{Client, Options, Trust, Url};
use wellfed_service::{Clock, Config, Service};

use crate::service::{TempStore, TestService, config};

/// How often an open stream checks its caller here: a test sees a stream
/// end soon after its caller lost it.
pub const STREAM_CHECK: Duration = Duration::from_millis(100);

/// The server, serving until [`Self::stop`], as `wellfed-server` runs it
/// ([`wellfed_server::serve`]), with the jobs off unless a test asks.
#[derive(Debug)]
pub struct TestServer {
    service: TestService,
    address: SocketAddr,
    tls_address: SocketAddr,
    identity: [u8; 32],
    shutdown: CancellationToken,
    served: JoinHandle<std::io::Result<()>>,
}

impl TestServer {
    /// The server with [`config`] on a fresh store.
    pub async fn start() -> Self {
        Self::start_with(config()).await
    }

    pub async fn start_with(config: Config) -> Self {
        Self::serve(TestService::start_with(config).await).await
    }

    /// The server with [`config`] on a fresh store, reading `clock`.
    pub async fn start_with_clock(clock: Arc<dyn Clock>) -> Self {
        Self::serve(TestService::start_with_clock(clock).await).await
    }

    /// [`Self::start_with_clock`], with the background jobs running on
    /// `clock`, as the server runs them.
    pub async fn start_with_jobs(clock: Arc<dyn Clock>) -> Self {
        let service = TestService::start_with_clock(clock).await;
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        Self::serve_at(service, TrustedProxies::default(), Jobs::Run, address).await
    }

    /// The server with `config` on a fresh store, reading `clock`.
    pub async fn start_with_config_and_clock(config: Config, clock: Arc<dyn Clock>) -> Self {
        Self::serve(TestService::start_with_config_and_clock(config, clock).await).await
    }

    /// The server with [`config`] on a fresh store, reading `clock` and
    /// believing the forwarded headers of `proxies`.
    pub async fn start_behind(clock: Arc<dyn Clock>, proxies: &[&str]) -> Self {
        let proxies = proxies
            .iter()
            .map(|range| range.parse().expect("a test's proxy range parses"));
        let service = TestService::start_with_clock(clock).await;
        Self::serve_behind(service, TrustedProxies::new(proxies)).await
    }

    async fn serve(service: TestService) -> Self {
        Self::serve_behind(service, TrustedProxies::default()).await
    }

    /// The server again on a stopped one's database
    /// ([`Self::stop_keeping`]), with `config`, at `address`: a server that
    /// comes back where its clients knew it.
    pub async fn reopen_at(store: TempStore, config: Config, address: SocketAddr) -> Self {
        let service = TestService::open(store, config).await;
        Self::serve_at(service, TrustedProxies::default(), Jobs::Off, address).await
    }

    async fn serve_behind(service: TestService, proxies: TrustedProxies) -> Self {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        Self::serve_at(service, proxies, Jobs::Off, address).await
    }

    async fn serve_at(
        service: TestService,
        proxies: TrustedProxies,
        jobs: Jobs,
        address: SocketAddr,
    ) -> Self {
        let listener = tokio::net::TcpListener::bind(address)
            .await
            .expect("bind the server's port");
        let address = listener
            .local_addr()
            .expect("a bound listener has an address");
        // The identity a server on [`config`]'s key has.
        let identity = Identity::derive(&config().key);
        let https = tokio::net::TcpListener::bind((address.ip(), 0))
            .await
            .expect("bind the server's HTTPS port");
        let tls_address = https.local_addr().expect("a bound listener has an address");
        let listeners = Listeners {
            http: listener,
            https: Some(Tls {
                listener: https,
                config: identity.tls().expect("the identity makes its certificate"),
            }),
        };
        let serving = Serving {
            proxies,
            stream_check: STREAM_CHECK,
            ..Serving::new(identity.public())
        };
        let beside = Beside {
            jobs,
            announce: None,
        };
        let shutdown = CancellationToken::new();
        let stop = shutdown.clone();
        let served = tokio::spawn(wellfed_server::serve(
            service.service().clone(),
            listeners,
            serving,
            beside,
            async move { stop.cancelled().await },
        ));

        Self {
            service,
            address,
            tls_address,
            identity: identity.public(),
            shutdown,
            served,
        }
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Where the server's own HTTPS listens.
    pub fn tls_address(&self) -> SocketAddr {
        self.tls_address
    }

    /// The server's identity key, which its HTTPS proves.
    pub fn identity(&self) -> [u8; 32] {
        self.identity
    }

    /// A client of this server with no credentials.
    pub fn client(&self) -> Client {
        let base =
            Url::parse(&format!("http://{}", self.address)).expect("a socket address makes a URL");
        Client::new(&base).expect("an http URL makes a client")
    }

    /// A client of this server's own HTTPS with no credentials, checking
    /// it as `trust` says.
    pub fn https_client(&self, trust: Trust) -> Client {
        let base = self.https_base();
        let options = Options {
            trust,
            ..Options::default()
        };
        Client::with_options(&base, options).expect("an https URL makes a client")
    }

    /// The base URL of this server's own HTTPS.
    pub fn https_base(&self) -> Url {
        Url::parse(&format!("https://{}", self.tls_address)).expect("a socket address makes a URL")
    }

    pub fn service(&self) -> &Service {
        self.service.service()
    }

    /// The service's data directory.
    pub fn store(&self) -> &TempStore {
        self.service.store()
    }

    /// Wait until the server holds the feeder's backlog, as
    /// [`TestService::caught_up`] does: a stream position taken before it
    /// may precede the backlog's events.
    pub async fn caught_up(&self, id: FeederId) {
        self.service.caught_up(id).await;
    }

    /// End the open streams, stop serving, and stop the service, keeping
    /// its database for [`Self::reopen_at`].
    pub async fn stop_keeping(mut self) -> TempStore {
        self.end().await;
        self.service.stop().await
    }

    /// End the open streams, stop serving, and stop the service.
    pub async fn stop(mut self) {
        self.end().await;
    }

    /// Stop as the server does at a signal: serving ends, then what runs
    /// beside it, then the service.
    async fn end(&mut self) {
        self.shutdown.cancel();
        let served = (&mut self.served).await;
        served
            .expect("the server's task ends cleanly")
            .expect("the server serves until it is stopped");
    }
}
