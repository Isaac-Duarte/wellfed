//! Claiming a key the caller's phone already holds on a feeder: one it
//! paired before there was a server, or the one it named when it adopted
//! the feeder. Once the feeder lists the key, the server records it as the
//! caller's phone key, as it records a key it invited, so deleting,
//! demoting or putting an expiry on the account revokes it.
//!
//! The feeder cannot prove which phone holds a key, so a claim trusts the
//! account to name its own, and only an owner's is trusted that far: an
//! owner may revoke any client anyway, but a member who claimed another
//! person's key could then revoke it, by claiming another key under the
//! same token or revoking the token. A member's phone that paired before
//! the server gets a key of its own through an invite instead. Asking again
//! for a key already recorded is answered for any role. What an owner's
//! claim cannot take: a key the feeder does not list, the server's own, one
//! an owner kept as a client of its own, and one listed with a role no
//! phone holds (a server's or an update signer's).

use tracing::warn;
use wellfed_api::{ClientRole, FeederId, PhoneClaim, PhoneKey, RevokeReason, Role, TokenId};
use wellfed_store::{NewPhone, Recorded, Whose};

use crate::{Caller, ClientError, Result, Service};

/// Who may hold a key of their own: a phone's token, and the role its key
/// takes.
#[derive(Clone, Copy, Debug)]
pub(in crate::feeders) struct Holder {
    pub(in crate::feeders) token: TokenId,
    pub(in crate::feeders) role: ClientRole,
}

impl Service {
    /// Claim `claim.key`, which the caller's phone already holds on the
    /// feeder, as the caller's phone key. Asked again by the same token, a
    /// claim answers the record it made; so a claim whose answer was lost,
    /// or one refused `phone_key_unconfirmed`, is settled by asking again.
    pub async fn claim_phone(
        &self,
        caller: &Caller,
        id: FeederId,
        claim: PhoneClaim,
    ) -> Result<PhoneKey> {
        let holder = self.direct_key_holder(caller).await?;
        self.live_row(id).await?;
        self.confirm_phone(caller, holder, id, claim.key.0).await
    }

    /// Record `key` as `holder`'s phone key on the feeder once its live
    /// client list shows it may be: what a claim and an adoption naming a
    /// key both run. The key joined long before, so no invite of it is
    /// live.
    pub(in crate::feeders) async fn confirm_phone(
        &self,
        caller: &Caller,
        holder: Holder,
        id: FeederId,
        key: [u8; 32],
    ) -> Result<PhoneKey> {
        let store = &self.shared.store;
        if let Some(phone) = store.phone(id, &key).await? {
            return if phone.token == Some(holder.token) && phone.revoking.is_none() {
                Ok(phone)
            } else {
                Err(ClientError::KeyTaken.into())
            };
        }
        if holder.role != ClientRole::Owner {
            return Err(ClientError::PhoneClaimOwnersOnly.into());
        }
        if store.kept_clients(id).await?.contains(&key) {
            return Err(ClientError::PhoneKeyKept.into());
        }
        let role = self.listed_role(id, &key).await?;
        if matches!(role, ClientRole::Server | ClientRole::UpdateSigner) {
            return Err(ClientError::PhoneKeyNotAPhone.into());
        }

        let now = self.shared.clock.now();
        let phone = NewPhone {
            feeder: id,
            key,
            user: caller.user.id,
            token: holder.token,
            // The role the feeder enforces for the key, not the account's.
            role,
            invited: now,
            invite_until: now,
        };
        match store.record_phone(&phone).await? {
            Recorded::New | Recorded::Again { .. } => {}
            Recorded::Taken => return Err(ClientError::KeyTaken.into()),
        }

        // The account or token may have lost its key since it was checked:
        // the reconcile marks the key, and starts revoking it.
        self.reconcile_phones().await?;
        let recorded = store.phone(id, &key).await?;
        let Some(recorded) = recorded.filter(|phone| phone.revoking.is_none()) else {
            return Err(ClientError::DirectKeyRefused.into());
        };
        let replaced = Whose::Replaced {
            feeder: id,
            token: holder.token,
            keep: key,
        };
        self.revoke_phones(replaced, RevokeReason::Replaced).await?;
        Ok(recorded)
    }

    /// The role the feeder lists `key` with, read over the server's
    /// session once it is up.
    async fn listed_role(&self, id: FeederId, key: &[u8; 32]) -> Result<ClientRole> {
        // A session just started, by an adoption or a reconnect, comes up
        // in the background.
        let within = self.shared.config.timing.answer;
        if !self.mirror.online(id, within).await {
            warn!("the feeder is not online to confirm a phone's key");
            return Err(ClientError::PhoneKeyUnconfirmed.into());
        }
        let listed = match self.list_clients(id).await {
            Ok(listed) => listed,
            Err(error) => {
                warn!(%error, "the feeder's clients could not be read to confirm a phone's key");
                return Err(ClientError::PhoneKeyUnconfirmed.into());
            }
        };
        let Some(client) = listed.iter().find(|client| client.key.0 == *key) else {
            return Err(ClientError::PhoneKeyNotListed.into());
        };
        if client.you {
            return Err(ClientError::OwnKey.into());
        }
        Ok(client.role)
    }

    /// The caller, when it may hold a key of its own: a phone's token, for
    /// an owner's or member's account, with no expiry on either.
    pub(in crate::feeders) async fn direct_key_holder(&self, caller: &Caller) -> Result<Holder> {
        let Some(token) = caller.token else {
            return Err(ClientError::TokenRequired.into());
        };
        let role = match caller.user.role {
            Role::Owner => ClientRole::Owner,
            Role::Member => ClientRole::Member,
            Role::Viewer | Role::Unknown => return Err(ClientError::DirectKeyRefused.into()),
        };
        let token_expires = self
            .shared
            .store
            .token(token)
            .await?
            .and_then(|token| token.expires);
        if caller.user.expires.is_some() || token_expires.is_some() {
            return Err(ClientError::DirectKeyRefused.into());
        }
        Ok(Holder { token, role })
    }
}
