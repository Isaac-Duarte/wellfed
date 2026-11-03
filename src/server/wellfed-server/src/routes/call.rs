//! The meal call: the clip a plan plays before its meal. Anyone reads its
//! description; an owner or a member uploads, erases or plays it, as they
//! change the settings and play a tune.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum_extra::extract::WithRejection;
use wellfed_api::{CallView, FeederId, Problem};

use super::{ApiResult, IdPath, TAG_FEEDERS};
use crate::AppState;
use crate::auth::{Allowed, need};
use crate::error::ApiError;
use crate::extract::OptionalIdempotencyKey;

/// A clip in the feeder's own stream: 4-bit IMA ADPCM at 8 kHz, mono, as
/// `wellfed_client::call::Clip::from_wav` converts a recording.
#[derive(utoipa::ToSchema)]
#[schema(value_type = String, format = Binary)]
#[expect(
    dead_code,
    reason = "only its schema is used: the body travels as bytes"
)]
pub(crate) struct CallStream(Vec<u8>);

/// The stored clip, or the upload under way, read from the feeder.
#[utoipa::path(
    get,
    path = "/feeders/{id}/call",
    tag = TAG_FEEDERS,
    params(("id" = FeederId, Path)),
    responses(
        (status = 200, body = CallView),
        (status = 404, body = Problem),
        (status = 409, body = Problem, description = "feeder_refused: `unavailable` from a build without the meal call, `busy`"),
        (status = 503, body = Problem, description = "feeder_offline"),
    ),
)]
pub(crate) async fn call(
    State(state): State<AppState>,
    _: Allowed<need::Read>,
    WithRejection(Path(id), _): IdPath,
) -> ApiResult<Json<CallView>> {
    Ok(Json(state.service.call(id).await?))
}

/// Upload a clip as the meal call, in place of the one the feeder holds.
/// The body is the feeder's own stream, converted by the client; the
/// server converts nothing and checks its length. It is sent a chunk at a
/// time beside feeds and status reads, and answered once the feeder has
/// read it back and stored it. A retry under the same key after
/// `outcome_unknown` sends only what the feeder lacks.
#[utoipa::path(
    put,
    path = "/feeders/{id}/call",
    tag = TAG_FEEDERS,
    params(("id" = FeederId, Path), OptionalIdempotencyKey),
    request_body(content = CallStream, content_type = "application/octet-stream"),
    responses(
        (status = 200, body = CallView, description = "the feeder stored the clip"),
        (status = 400, body = Problem, description = "invalid: an empty clip, one past the feeder's length, or a malformed Idempotency-Key"),
        (status = 403, body = Problem, description = "forbidden: a viewer"),
        (status = 404, body = Problem),
        (status = 409, body = Problem, description = "feeder_archived; install_running; or feeder_refused (`forbidden`: the server's key is not an owner's; `not_writable`, `busy`): final for this key, and the clip the feeder held may be gone"),
        (status = 413, body = Problem, description = "too_large"),
        (status = 422, body = Problem, description = "key_reused: the key names another clip"),
        (status = 503, body = Problem, description = "feeder_offline: nothing was sent"),
        (status = 504, body = Problem, description = "outcome_unknown: retry with the same key"),
    ),
)]
pub(crate) async fn upload_call(
    State(state): State<AppState>,
    Allowed(caller, _): Allowed<need::Control>,
    WithRejection(Path(id), _): IdPath,
    OptionalIdempotencyKey(key): OptionalIdempotencyKey,
    WithRejection(stream, _): WithRejection<Bytes, ApiError>,
) -> ApiResult<Json<CallView>> {
    let call = state
        .service
        .upload_call(&caller, id, stream.into(), key)
        .await?;
    Ok(Json(call))
}

/// Erase the meal call; plans then play none.
#[utoipa::path(
    delete,
    path = "/feeders/{id}/call",
    tag = TAG_FEEDERS,
    params(("id" = FeederId, Path), OptionalIdempotencyKey),
    responses(
        (status = 204),
        (status = 403, body = Problem, description = "forbidden: a viewer"),
        (status = 404, body = Problem),
        (status = 409, body = Problem, description = "feeder_archived, or feeder_refused (`forbidden`: the server's key is not an owner's): final for this key"),
        (status = 503, body = Problem, description = "feeder_offline"),
        (status = 504, body = Problem, description = "outcome_unknown: retry with the same key"),
    ),
)]
pub(crate) async fn delete_call(
    State(state): State<AppState>,
    Allowed(caller, _): Allowed<need::Control>,
    WithRejection(Path(id), _): IdPath,
    OptionalIdempotencyKey(key): OptionalIdempotencyKey,
) -> ApiResult<StatusCode> {
    state.service.delete_call(&caller, id, key).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Play the stored clip once on the feeder, to hear it.
#[utoipa::path(
    post,
    path = "/feeders/{id}/call/play",
    tag = TAG_FEEDERS,
    params(("id" = FeederId, Path), OptionalIdempotencyKey),
    responses(
        (status = 204),
        (status = 403, body = Problem, description = "forbidden: a viewer"),
        (status = 404, body = Problem),
        (status = 409, body = Problem, description = "feeder_archived, or feeder_refused (`muted` while sound is off, `busy` while the speaker plays, `not_found` with no clip): final for this key"),
        (status = 503, body = Problem, description = "feeder_offline"),
        (status = 504, body = Problem, description = "outcome_unknown: retry with the same key"),
    ),
)]
pub(crate) async fn play_call(
    State(state): State<AppState>,
    Allowed(caller, _): Allowed<need::Control>,
    WithRejection(Path(id), _): IdPath,
    OptionalIdempotencyKey(key): OptionalIdempotencyKey,
) -> ApiResult<StatusCode> {
    state.service.play_call(&caller, id, key).await?;
    Ok(StatusCode::NO_CONTENT)
}
