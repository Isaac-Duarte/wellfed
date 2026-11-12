//! The server's own HTTPS, on the home network: the same API as the plain
//! listener, over TLS 1.3 with the certificate the identity key signs
//! ([`crate::identity`]). It is what the app reaches the server by on the
//! home network, pinning that key, so a password or a token never crosses
//! the LAN in the clear and a host that announces itself as the server
//! cannot answer for it.
//!
//! Each connection gets its handshake, bounded so a client that opens one
//! and says nothing does not hold a task for good, and then HTTP/1.1. The
//! API sees the connection's address, as on the plain listener, and its
//! scheme as `https`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::ConnectInfo;
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use rustls::ServerConfig;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tower::ServiceExt as _;

/// The longest a TLS handshake may take.
const HANDSHAKE: Duration = Duration::from_secs(10);

/// Marks a request that came over the server's own TLS.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DirectTls;

/// The server's HTTPS listener and its TLS.
#[derive(Debug)]
pub struct Tls {
    pub listener: TcpListener,
    pub config: Arc<ServerConfig>,
}

/// Serve `app` over `tls` until `shutdown`, then finish the requests in
/// progress; resolves once every connection has ended.
pub(crate) async fn serve(tls: Tls, app: Router, shutdown: CancellationToken) {
    let acceptor = TlsAcceptor::from(tls.config);
    let connections = TaskTracker::new();
    loop {
        let accepted = tokio::select! {
            () = shutdown.cancelled() => break,
            accepted = tls.listener.accept() => accepted,
        };
        let (stream, peer) = match accepted {
            Ok(accepted) => accepted,
            Err(error) => {
                // Out of file descriptors, say: the listener itself holds.
                tracing::warn!("accept an HTTPS connection: {error}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let (acceptor, app, shutdown) = (acceptor.clone(), app.clone(), shutdown.clone());
        connections.spawn(async move {
            let Ok(Ok(stream)) = tokio::time::timeout(HANDSHAKE, acceptor.accept(stream)).await
            else {
                tracing::debug!("an HTTPS handshake from {peer} did not finish");
                return;
            };
            connection(TokioIo::new(stream), peer, app, shutdown).await;
        });
    }
    connections.close();
    connections.wait().await;
}

/// One connection's requests, each carrying `peer` and the TLS mark.
async fn connection<I>(io: I, peer: SocketAddr, app: Router, shutdown: CancellationToken)
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let service = app.map_request(move |request: hyper::Request<hyper::body::Incoming>| {
        let mut request = request.map(axum::body::Body::new);
        request.extensions_mut().insert(ConnectInfo(peer));
        request.extensions_mut().insert(DirectTls);
        request
    });
    let service = TowerToHyperService::new(service);
    let served = hyper::server::conn::http1::Builder::new()
        .serve_connection(io, service)
        .with_upgrades();
    tokio::pin!(served);
    let ended = tokio::select! {
        ended = served.as_mut() => ended,
        () = shutdown.cancelled() => {
            served.as_mut().graceful_shutdown();
            served.await
        }
    };
    if let Err(error) = ended {
        tracing::debug!("an HTTPS connection from {peer} ended: {error}");
    }
}
