//! A feeder's clients and the phones' own keys: listing whose each client
//! is, keeping or revoking one, an invite for the caller's own phone, and
//! claiming a key the phone already holds.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum_extra::extract::WithRejection;
use uuid::Uuid;
use wellfed_api::{
    FeederClient, FeederId, Hex, NewPhoneInvite, PhoneClaim, PhoneInvite, PhoneKey, Problem,
};

use super::{ApiResult, IdPath, JsonBody, TAG_FEEDERS};
use crate::AppState;
use crate::auth::{Allowed, need};
use crate::error::ApiError;
use crate::extract::OptionalIdempotencyKey;

type ClientPath = WithRejection<Path<(Uuid, Hex<32>)>, ApiError>;

/// The feeder's clients, read from it, each with whose it is to the
/// server: its own key, a phone it invited, one an owner kept, or unowned.
#[utoipa::path(
    get,
    path = "/feeders/{id}/clients",
    tag = TAG_FEEDERS,
    params(("id" = FeederId, Path)),
    responses(
        (status = 200, body = Vec<FeederClient>),
        (status = 403, body = Problem, description = "forbidden: owners only"),
        (status = 404, body = Problem),
        (status = 409, body = Problem, description = "feeder_archived, or feeder_refused"),
        (status = 503, body = Problem, description = "feeder_offline"),
        (status = 504, body = Problem, description = "outcome_unknown"),
    ),
)]
pub(crate) async fn clients(
    State(state): State<AppState>,
    _: Allowed<need::FeederAdmin>,
    WithRejection(Path(id), _): IdPath,
) -> ApiResult<Json<Vec<FeederClient>>> {
    Ok(Json(state.service.clients(id).await?))
}

/// Revoke one of the feeder's clients. A phone's key is forgotten with it.
/// The feeder lets only an owner's key revoke: a server adopted with the
/// `server` role is refused `forbidden`.
#[utoipa::path(
    delete,
    path = "/feeders/{id}/clients/{key}",
    tag = TAG_FEEDERS,
    params(("id" = FeederId, Path), ("key" = String, Path), OptionalIdempotencyKey),
    responses(
        (status = 204),
        (status = 400, body = Problem, description = "invalid: the server's own key (remove the feeder instead)"),
        (status = 403, body = Problem, description = "forbidden: owners only"),
        (status = 404, body = Problem),
        (status = 409, body = Problem, description = "feeder_archived, or feeder_refused (not_found: no such client)"),
        (status = 503, body = Problem, description = "feeder_offline"),
        (status = 504, body = Problem, description = "outcome_unknown: retry with the same key"),
    ),
)]
pub(crate) async fn revoke_client(
    State(state): State<AppState>,
    Allowed(caller, _): Allowed<need::FeederAdmin>,
    WithRejection(Path((id, key)), _): ClientPath,
    OptionalIdempotencyKey(idempotency): OptionalIdempotencyKey,
) -> ApiResult<StatusCode> {
    state
        .service
        .revoke_client(&caller, FeederId(id), key.0, idempotency)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Keep a client the server did not invite, so it is no longer listed as
/// unowned. Nothing is sent to the feeder.
#[utoipa::path(
    post,
    path = "/feeders/{id}/clients/{key}/keep",
    tag = TAG_FEEDERS,
    params(("id" = FeederId, Path), ("key" = String, Path)),
    responses(
        (status = 204),
        (status = 403, body = Problem, description = "forbidden: owners only; session_required: not from a token"),
        (status = 404, body = Problem),
        (status = 409, body = Problem, description = "feeder_archived"),
    ),
)]
pub(crate) async fn keep_client(
    State(state): State<AppState>,
    Allowed(caller, _): Allowed<need::FeederAdmin>,
    WithRejection(Path((id, key)), _): ClientPath,
) -> ApiResult<StatusCode> {
    state
        .service
        .keep_client(&caller, FeederId(id), key.0)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// An invite for the caller's own phone, which joins the feeder with the
/// key it names, on the LAN, with its account's role. Only a phone's
/// token gets one, for an owner or a member with no expiry on the account
/// or the token. One key per token: the token's other key on the feeder
/// is revoked. The invite holds a secret, is shown once and is never
/// resent: a caller that lost it asks again.
#[utoipa::path(
    post,
    path = "/feeders/{id}/invites",
    tag = TAG_FEEDERS,
    params(("id" = FeederId, Path)),
    request_body = NewPhoneInvite,
    responses(
        (status = 201, body = PhoneInvite),
        (status = 403, body = Problem, description = "forbidden (a viewer), token_required or direct_key_refused"),
        (status = 404, body = Problem),
        (status = 409, body = Problem, description = "client_key_taken, feeder_archived, or feeder_refused (forbidden: the server's key is not an owner's)"),
        (status = 503, body = Problem, description = "feeder_offline"),
        (status = 504, body = Problem, description = "outcome_unknown: ask for a new invite"),
    ),
)]
pub(crate) async fn invite_phone(
    State(state): State<AppState>,
    Allowed(caller, _): Allowed<need::InviteOwnPhone>,
    WithRejection(Path(id), _): IdPath,
    WithRejection(Json(request), _): JsonBody<NewPhoneInvite>,
) -> ApiResult<(StatusCode, Json<PhoneInvite>)> {
    let invite = state.service.invite_phone(&caller, id, request).await?;
    Ok((StatusCode::CREATED, Json(invite)))
}

/// Claim a key the caller's phone already holds on the feeder (paired
/// before the server, or named by an adoption whose claim did not settle)
/// as the caller's phone key, once the feeder lists it. A phone's token
/// with no expiry on it or the account, as for an invite; and only an
/// owner claims a key the server has not recorded, since the feeder cannot
/// prove whose a key is. A member's phone that paired before the server
/// asks for an invite (`POST /feeders/{id}/invites`) and joins with a new
/// key instead. The token that recorded a key asking again, whatever its
/// role, is answered that record, so a lost answer or
/// `phone_key_unconfirmed` is settled by asking again.
#[utoipa::path(
    post,
    path = "/feeders/{id}/phones",
    tag = TAG_FEEDERS,
    params(("id" = FeederId, Path)),
    request_body = PhoneClaim,
    responses(
        (status = 200, body = PhoneKey),
        (status = 400, body = Problem, description = "invalid: the server's own key"),
        (status = 403, body = Problem, description = "forbidden (a viewer), token_required, direct_key_refused, or phone_claim_owners_only (a member's claim of a key not recorded)"),
        (status = 404, body = Problem),
        (status = 409, body = Problem, description = "phone_key_not_listed, phone_key_unconfirmed (claim again), phone_key_kept, phone_key_not_a_phone, client_key_taken or feeder_archived"),
    ),
)]
pub(crate) async fn claim_phone(
    State(state): State<AppState>,
    Allowed(caller, _): Allowed<need::InviteOwnPhone>,
    WithRejection(Path(id), _): IdPath,
    WithRejection(Json(request), _): JsonBody<PhoneClaim>,
) -> ApiResult<Json<PhoneKey>> {
    Ok(Json(state.service.claim_phone(&caller, id, request).await?))
}

/// The phone keys the server recorded: everyone's for an owner, one's own
/// for anyone else. A key being revoked shows why and what its last
/// attempt came to, so a feeder that still holds it is reported.
#[utoipa::path(
    get,
    path = "/phones",
    tag = TAG_FEEDERS,
    responses((status = 200, body = Vec<PhoneKey>), (status = 401, body = Problem)),
)]
pub(crate) async fn phones(
    State(state): State<AppState>,
    Allowed(caller, _): Allowed<need::OwnAccount>,
) -> ApiResult<Json<Vec<PhoneKey>>> {
    Ok(Json(state.service.phones(&caller.user).await?))
}
