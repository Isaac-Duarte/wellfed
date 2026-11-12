//! Challenges a phone answers to prove it holds its key's secret.
//!
//! A logout leaves the phone's own key with the account and no token, and a
//! claim puts it under the claiming token again. Any token of the account
//! may claim, so a claim that moves a key with no token must prove the
//! phone holds that key's secret: otherwise another token of the account (a
//! stolen one) could take the logged-out phone's key and then have it
//! revoked with itself. The proof is `wellfed_session::proof`'s, answered
//! to a one-use ephemeral key the server makes here.
//!
//! Challenges live in memory: a restart drops them, and the phone asks for
//! another. Each is the asking token's alone, answered once (a claim takes
//! it whether or not its proof holds) within [`LIFETIME`], and a token
//! holds at most [`PER_TOKEN`], its oldest dropped for a new one.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use jiff::{SignedDuration, Timestamp};
use uuid::Uuid;
use wellfed_api::{Hex, KeyProof, PhoneChallenge, TokenId};
use wellfed_session::noise::Keypair;
use wellfed_session::proof;

use super::claim::Holder;
use crate::{Caller, ClientError, Result, Service, lock};

/// How long a challenge may be answered.
const LIFETIME: SignedDuration = SignedDuration::from_secs(60);

/// The most challenges one token holds at once.
const PER_TOKEN: usize = 8;

/// The challenges not yet answered.
#[derive(Clone, Default)]
pub(crate) struct Challenges(Arc<Mutex<Book>>);

#[derive(Default)]
struct Book {
    pending: HashMap<Uuid, Pending>,
    /// Counts the challenges made, so a token's oldest is known even when
    /// the clock has not moved between them.
    made: u64,
}

struct Pending {
    token: TokenId,
    made: u64,
    /// The server's one-use key pair, its secret dropped with the
    /// challenge.
    key: Keypair,
    nonce: [u8; proof::NONCE_LEN],
    expires: Timestamp,
}

/// Whether a claim showed that the phone holds the key's secret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::feeders) enum Possession {
    Proven,
    Unproven,
}

impl Service {
    /// A challenge for the caller's phone to answer in a claim, proving it
    /// holds its key's secret: a phone's token, which may hold a key of its
    /// own.
    pub async fn phone_challenge(&self, caller: &Caller) -> Result<PhoneChallenge> {
        let holder = self.direct_key_holder(caller).await?;
        let id = uuid::Builder::from_random_bytes(self.shared.random()?).into_uuid();
        let key = Keypair::from_secret(self.shared.random()?);
        let nonce = self.shared.random()?;
        let now = self.shared.clock.now();
        let expires = now + LIFETIME;
        let challenge = PhoneChallenge {
            id,
            key: Hex(*key.public()),
            nonce: Hex(nonce),
            expires,
        };

        let mut book = lock(&self.feeders.challenges.0);
        book.pending.retain(|_, held| held.expires > now);
        let mut held: Vec<(u64, Uuid)> = book
            .pending
            .iter()
            .filter(|(_, held)| held.token == holder.token)
            .map(|(id, held)| (held.made, *id))
            .collect();
        held.sort_unstable();
        let over = (held.len() + 1).saturating_sub(PER_TOKEN);
        for (_, oldest) in held.into_iter().take(over) {
            book.pending.remove(&oldest);
        }
        book.made += 1;
        let pending = Pending {
            token: holder.token,
            made: book.made,
            key,
            nonce,
            expires,
        };
        book.pending.insert(id, pending);
        Ok(challenge)
    }

    /// What `proof`, if any, shows of `key`: proven when it answers a
    /// challenge `holder`'s token holds, unexpired, with the MAC only
    /// `key`'s secret makes. The challenge is used up either way.
    pub(in crate::feeders) fn possession(
        &self,
        holder: Holder,
        key: &[u8; 32],
        proof: Option<KeyProof>,
    ) -> Result<Possession> {
        let Some(proof) = proof else {
            return Ok(Possession::Unproven);
        };
        let pending = lock(&self.feeders.challenges.0)
            .pending
            .remove(&proof.challenge);
        let now = self.shared.clock.now();
        let Some(pending) = pending.filter(|held| held.token == holder.token && now < held.expires)
        else {
            return Err(ClientError::PhoneKeyProofRefused.into());
        };
        let token = holder.token.0.as_bytes();
        if !proof::verify(&pending.key, key, &pending.nonce, token, &proof.mac.0) {
            return Err(ClientError::PhoneKeyProofRefused.into());
        }
        Ok(Possession::Proven)
    }
}
