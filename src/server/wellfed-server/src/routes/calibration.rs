//! The portion calibration: what one portion weighs and the bowl mode.
//! Anyone reads it; an owner or a member changes it, as the settings,
//! since it decides how the food views count rather than what is fed.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::WithRejection;
use wellfed_api::{Calibration, CalibrationView, FeederId, Problem};

use super::feeders::etag;
use super::{ApiResult, IdPath, JsonBody, TAG_FEEDERS, insert};
use crate::AppState;
use crate::auth::{Allowed, need};
use crate::extract::{IfMatch, OptionalIdempotencyKey};

/// The feeder's portion calibration, read from it. The `ETag` is the
/// generation a change sends back in `If-Match`.
#[utoipa::path(
    get,
    path = "/feeders/{id}/calibration",
    tag = TAG_FEEDERS,
    params(("id" = FeederId, Path)),
    responses(
        (status = 200, body = CalibrationView, headers(("ETag" = String))),
        (status = 404, body = Problem),
        (status = 409, body = Problem, description = "feeder_refused: `unavailable` from a build that keeps no calibration"),
        (status = 503, body = Problem, description = "feeder_offline"),
    ),
)]
pub(crate) async fn calibration(
    State(state): State<AppState>,
    _: Allowed<need::Read>,
    WithRejection(Path(id), _): IdPath,
) -> ApiResult<Response> {
    let calibration = state.service.calibration(id).await?;
    Ok(tagged(calibration))
}

/// Store the calibration, if it is still at the generation in `If-Match`.
/// `portion_mg` is the mean weight of one portion (1-50 g), from a scale's
/// reading of counted portions, or `null` to clear it.
#[utoipa::path(
    put,
    path = "/feeders/{id}/calibration",
    tag = TAG_FEEDERS,
    params(("id" = FeederId, Path), IfMatch, OptionalIdempotencyKey),
    request_body = Calibration,
    responses(
        (status = 200, body = CalibrationView, headers(("ETag" = String))),
        (status = 400, body = Problem, description = "invalid: a malformed body, If-Match or Idempotency-Key, or a portion weight out of range"),
        (status = 403, body = Problem, description = "forbidden: a viewer"),
        (status = 404, body = Problem),
        (status = 409, body = Problem, description = "feeder_archived, or feeder_refused (`unverified`: a bowl mode the feeder has not verified; `forbidden`: the server's key is not an owner's): final for this key"),
        (status = 412, body = CalibrationView, description = "Stale: the current calibration"),
        (status = 428, body = Problem, description = "precondition_required"),
        (status = 503, body = Problem, description = "feeder_offline"),
        (status = 504, body = Problem, description = "outcome_unknown: retry with the same key"),
    ),
)]
pub(crate) async fn put_calibration(
    State(state): State<AppState>,
    Allowed(caller, _): Allowed<need::Control>,
    WithRejection(Path(id), _): IdPath,
    IfMatch(base): IfMatch,
    OptionalIdempotencyKey(key): OptionalIdempotencyKey,
    WithRejection(Json(calibration), _): JsonBody<Calibration>,
) -> ApiResult<Response> {
    let calibration = state
        .service
        .put_calibration(&caller, id, base, calibration, key)
        .await?;
    Ok(tagged(calibration))
}

fn tagged(calibration: CalibrationView) -> Response {
    let mut response = Json(calibration).into_response();
    insert(&mut response, header::ETAG, &etag(calibration.generation));
    response
}
