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
//! round, which the app starts on every connect and list. The claims go
//! out together, so a slow or failing feeder holds up neither the others
//! nor the login.

use futures_util::future::join_all;
use wellfed_api::Hex;

use crate::api::server_error::{ServerError, ServerErrorKind};
use crate::route::io::Router;

impl Router {
    /// Claim `phone`, this phone's own key, on every feeder the server
    /// listed that it is not claimed on yet this session.
    pub(crate) async fn claim_own(&self, phone: [u8; 32]) {
        let Some((client, session, due)) = self.claims_due() else {
            return;
        };

        let client = &client;
        let claims = due.into_iter().map(|id| async move {
            let claimed = client.claim_phone(id, Hex(phone)).await;
            (id, claimed.err().map(ServerError::from))
        });
        for (id, error) in join_all(claims).await {
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
