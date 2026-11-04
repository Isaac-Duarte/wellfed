//! The household: its name, units, time format and zone.

use axum::Json;
use axum::extract::State;
use axum_extra::extract::WithRejection;
use wellfed_api::{Home, HomePatch, Problem};

use super::{ApiResult, JsonBody, TAG_HOME};
use crate::AppState;
use crate::auth::{Allowed, need};

/// The household, as every user's pages show it.
#[utoipa::path(
    get,
    path = "/home",
    tag = TAG_HOME,
    responses((status = 200, body = Home), (status = 401, body = Problem)),
)]
pub(crate) async fn home(
    State(state): State<AppState>,
    _: Allowed<need::Read>,
) -> ApiResult<Json<Home>> {
    Ok(Json(state.service.home().await?))
}

/// Change the household. A new zone goes to every feeder as its POSIX rule:
/// the online ones now, the others when they reconnect.
#[utoipa::path(
    patch,
    path = "/home",
    tag = TAG_HOME,
    request_body = HomePatch,
    responses(
        (status = 200, body = Home),
        (status = 400, body = Problem, description = "invalid: a name, or an unknown zone"),
        (status = 403, body = Problem, description = "forbidden: owners only"),
        (status = 422, body = Problem, description = "zone_unsupported: no POSIX rule describes the zone"),
    ),
)]
pub(crate) async fn update_home(
    State(state): State<AppState>,
    Allowed(caller, _): Allowed<need::Home>,
    WithRejection(Json(patch), _): JsonBody<HomePatch>,
) -> ApiResult<Json<Home>> {
    Ok(Json(state.service.update_home(&caller, patch).await?))
}
