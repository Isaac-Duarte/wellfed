//! Liveness and the server's identity for anyone, readiness for an owner.

use axum::Json;
use axum::extract::State;
use wellfed_api::{Health, Hex, Problem, ServerIdentity, ServerStatus};

use super::TAG_HEALTH;
use crate::AppState;
use crate::auth::{Allowed, need};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Liveness, version and API revision, without authentication: the
/// process is up and serving. It reads nothing, so a probe never waits on the database.
#[utoipa::path(
    get,
    path = "/health",
    tag = TAG_HEALTH,
    responses((status = 200, body = Health)),
)]
pub(crate) async fn health() -> Json<Health> {
    Json(Health {
        version: VERSION.into(),
        revision: wellfed_api::REVISION,
    })
}

/// The server's identity key, without authentication: what the app pins
/// when it reaches the server by a name with a public certificate, as its
/// own HTTPS on the home network proves the key in the handshake. The key
/// is public, and the same for every caller.
#[utoipa::path(
    get,
    path = "/identity",
    tag = TAG_HEALTH,
    responses((status = 200, body = ServerIdentity)),
)]
pub(crate) async fn identity(State(state): State<AppState>) -> Json<ServerIdentity> {
    Json(ServerIdentity {
        key: Hex(state.identity),
    })
}

/// Readiness: whether the database answers, each feeder's link and the
/// background jobs' last runs. A failing database is reported in the body,
/// not as an error.
#[utoipa::path(
    get,
    path = "/status",
    tag = TAG_HEALTH,
    responses(
        (status = 200, body = ServerStatus),
        (status = 401, body = Problem),
        (status = 403, body = Problem, description = "forbidden: owners only"),
    ),
)]
pub(crate) async fn status(
    State(state): State<AppState>,
    _: Allowed<need::ServerData>,
) -> Json<ServerStatus> {
    Json(state.service.status(VERSION).await)
}
