//! This phone's own key under the token its login minted. A logout leaves
//! the phone's key on its feeders with the account and no token, and
//! nothing on the server ties a key with no token to the next token: had
//! the phone logged in again and then been lost, revoking that token would
//! leave its key on the feeders. So once the phone is logged in, it claims
//! its own key on each feeder the server lists. The server puts a key of
//! the account's that has no token under the claiming one, answers a key
//! already under it with its record, and records a key an owner's phone
//! paired before the server, once the feeder lists it.
//!
//! Each feeder is claimed once a session. A refusal (another phone holds
//! the key, the feeder does not list it, only an owner may claim it) holds
//! for the session. A claim the server could not settle (the server
//! unreachable, the feeder offline) is logged and tried again on the next
//! round, which the app starts on every connect and list. A few claims go
//! out at once ([`CLAIMS_AT_ONCE`]), so a slow or failing feeder holds up
//! neither the others nor the login. Each claim proves the phone holds the
//! key ([`claim_with_proof`]), which the server asks of a key with no token.

use futures_util::{StreamExt, stream};
use uuid::Uuid;
use wellfed_api::{FeederId, Hex, KeyProof, PhoneClaim, PhoneKey};
use wellfed_server_client::Client;
use wellfed_session::noise::Keypair;
use wellfed_session::proof;

use crate::api::server_error::{ServerError, ServerErrorKind};
use crate::route::io::{ClaimsDue, Router};

/// How many claims a round has out at once. Each asks for a challenge of
/// its own, and the server holds at most 8 unanswered per token
/// (`PER_TOKEN` in `wellfed-service`'s phone proofs), dropping the oldest
/// for a new one: a round with every feeder's claim out at once would have
/// its earliest proofs refused past 8 feeders. Half that leaves room for a
/// claim the owner starts meanwhile (an adoption's).
const CLAIMS_AT_ONCE: usize = 4;

impl Router {
    /// Claim `phone`, this phone's own key, on every feeder the server
    /// listed that it is not claimed on yet this session.
    pub(crate) async fn claim_own(&self, phone: &Keypair) {
        let Some(ClaimsDue {
            client,
            token,
            session,
            due,
        }) = self.claims_due()
        else {
            return;
        };

        let client = &client;
        let claims = due.into_iter().map(|id| async move {
            let claimed = claim_with_proof(client, token, id, phone).await;
            (id, claimed.err())
        });
        let answered: Vec<_> = stream::iter(claims)
            .buffer_unordered(CLAIMS_AT_ONCE)
            .collect()
            .await;
        for (id, error) in answered {
            let Some(error) = error else {
                continue;
            };
            if holds(error.kind) {
                log::info!("this phone's key on {id} stays unclaimed: {error}");
            } else {
                log::warn!("this phone's key on {id} was not claimed, tried again later: {error}");
                self.claim_again(session, id);
            }
        }
    }
}

/// A refusal the same claim would meet again this session.
const fn holds(kind: ServerErrorKind) -> bool {
    matches!(
        kind,
        ServerErrorKind::PhoneKeyRefused
            | ServerErrorKind::Forbidden
            | ServerErrorKind::NotFound
            | ServerErrorKind::Refused
    )
}

/// Claim `phone`, this phone's own key, on `id` as `token`'s, proving the
/// phone holds its secret: the server puts a key a logout left with no
/// token under the claiming token only with that proof. Each claim answers
/// a challenge of its own, which serves once. With no token id to bind a
/// proof to (a stored id that does not parse), the claim goes without one.
pub(crate) async fn claim_with_proof(
    client: &Client,
    token: Option<Uuid>,
    id: FeederId,
    phone: &Keypair,
) -> Result<PhoneKey, ServerError> {
    let key = Hex(*phone.public());
    let proof = match token {
        Some(token) => {
            let challenge = client.phone_challenge().await?;
            let nonce = &challenge.nonce.0;
            let Ok(mac) = proof::prove(phone, &challenge.key.0, nonce, token.as_bytes()) else {
                return Err(ServerError::new(
                    ServerErrorKind::Protocol,
                    "the server's challenge is not a key a proof can answer",
                ));
            };
            Some(KeyProof {
                challenge: challenge.id,
                mac: Hex(mac),
            })
        }
        None => None,
    };
    let claim = PhoneClaim { key, proof };
    Ok(client.claim_phone(id, &claim).await?)
}
