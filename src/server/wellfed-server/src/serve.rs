//! The server as one process runs it, which `main` and the test kit's
//! server share: the API on its bound listeners (plain HTTP, and HTTPS
//! when set), the background jobs and the mDNS announcement beside it, and
//! the order they stop in.
//!
//! Shutdown runs in this order once `stop` resolves:
//! 1. The open event streams, exports and imports end, both listeners stop
//!    taking connections, and the requests in progress finish.
//! 2. The jobs' supervisor stops. A job still running is cut off there
//!    (not recorded, so it runs again at its next time), but its task has
//!    ended before the service stops, so the store never closes under it.
//! 3. The mDNS record is withdrawn.
//! 4. The service stops: the feeder connections close, then the database.

use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use wellfed_service::Service;

use crate::config::Advertise;
use crate::server_id::ServerId;
use crate::tls::Tls;
use crate::{Hosts, TrustedProxies, app};

/// How the API answers, besides the service it serves.
#[derive(Clone, Debug)]
pub struct Serving {
    /// The server's identity key ([`crate::identity`]), which
    /// `GET /identity` answers and the announcement names.
    pub identity: [u8; 32],
    /// The names it answers to besides IP addresses and `localhost`.
    pub hosts: Hosts,
    /// Whose forwarded headers name the client and its scheme.
    pub proxies: TrustedProxies,
    /// How often an open event or log stream checks that its caller may
    /// still follow it: that the session or token still holds, with the
    /// role it opened with.
    pub stream_check: Duration,
}

impl Serving {
    /// Serving as the server's identity `identity`, under the default
    /// settings: no names but addresses and `localhost`, no trusted
    /// proxies, and streams checked each minute.
    pub fn new(identity: [u8; 32]) -> Self {
        Self {
            identity,
            hosts: Hosts::default(),
            proxies: TrustedProxies::default(),
            stream_check: Duration::from_secs(60),
        }
    }
}

/// Where the API listens.
#[derive(Debug)]
pub struct Listeners {
    /// Plain HTTP: this computer, or a reverse proxy in front.
    pub http: TcpListener,
    /// The server's own HTTPS, which the app reaches it by on the home
    /// network and which is all mDNS announces.
    pub https: Option<Tls>,
}

/// Whether the background jobs run beside the API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Jobs {
    Run,
    /// A test's server, whose jobs a test runs itself.
    Off,
}

/// The mDNS announcement of the server.
#[derive(Clone, Debug)]
pub struct Announce {
    pub advertise: Advertise,
    pub label: String,
    pub id: ServerId,
}

/// What runs beside the API.
#[derive(Clone, Debug)]
pub struct Beside {
    pub jobs: Jobs,
    /// `None` announces nothing.
    pub announce: Option<Announce>,
}

/// Serve `service` on `listeners` with what runs `beside` it until `stop`
/// resolves, then stop everything in the module's order. The service is
/// stopped whatever happened, so a caller never shuts it down itself.
pub async fn serve(
    service: Service,
    listeners: Listeners,
    serving: Serving,
    beside: Beside,
    stop: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let shutdown = CancellationToken::new();
    let identity = serving.identity;
    let app = app(service.clone(), shutdown.clone(), serving);
    let jobs = match beside.jobs {
        Jobs::Run => Some(crate::jobs::spawn(
            service.clone(),
            service.clock(),
            shutdown.clone(),
        )),
        Jobs::Off => None,
    };
    let Listeners { http, https } = listeners;
    let announced = beside.announce.and_then(|announce| {
        crate::advertise::spawn(
            announce.advertise,
            https.as_ref().map(|https| &https.listener),
            &announce.label,
            announce.id,
            identity,
            shutdown.clone(),
        )
    });
    let secure =
        https.map(|https| tokio::spawn(crate::tls::serve(https, app.clone(), shutdown.clone())));

    let ending = shutdown.clone();
    // Each request knows its connection's address: logins are limited by
    // the client's.
    let served = axum::serve(
        http,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        stop.await;
        ending.cancel();
    })
    .await;

    // Serving can also end on its own error, with nothing cancelled yet.
    shutdown.cancel();
    if let Some(secure) = secure
        && let Err(error) = secure.await
    {
        tracing::error!("the HTTPS listener's task: {error}");
    }
    if let Some(jobs) = jobs
        && let Err(error) = jobs.await
    {
        tracing::error!("the jobs' task: {error}");
    }
    if let Some(announced) = announced
        && let Err(error) = announced.await
    {
        tracing::error!("the mDNS announcement's task: {error}");
    }
    service.shutdown().await;
    served
}
