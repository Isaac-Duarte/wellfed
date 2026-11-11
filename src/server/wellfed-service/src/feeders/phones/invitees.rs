//! Attributing the keys a phone key invited.
//!
//! A phone key may ask the feeder for plain invites, so whoever holds it
//! can enroll keys the server never invited. The feeder reports each join
//! through an invite with the first bytes of both keys, and the store keeps
//! it until the server attributes it here: the key that joined is recorded
//! under the account and token of the phone key that invited it, so it is
//! revoked with that key, the account or the token ([`wellfed_store`]'s
//! `phones::invitees`).
//!
//! A join is attributed once the feeder's clients can be read, at the next
//! pass after its event lands. Only a join a phone key made is read for:
//! every adoption is a join through an invite too, and a pass that asked
//! the feeder for its clients each time would load every session. One the feeder's history lost (a ring that
//! overwrote it while the server was away, a gap) is never reported, and
//! the key it enrolled shows as unowned until an owner revokes or keeps it.

use jiff::Timestamp;
use tracing::{info, warn};
use wellfed_api::FeederId;
use wellfed_model::Client;
use wellfed_session::hex;
use wellfed_store::Invitation;

use crate::{Result, Service};

impl Service {
    /// Attribute every join through an invite the feeder reported and the
    /// server has not attributed yet. Whether a key so recorded is owed a
    /// revoke, its inviter being revoked or gone.
    pub(super) async fn attribute_invitees(&self, id: FeederId) -> Result<bool> {
        let store = &self.shared.store;
        let mut pending = store.invitations(id).await?;
        if pending.is_empty() {
            return Ok(false);
        }

        // A key recorded here makes the joins it invited readable in turn,
        // so read again until no more are; each read is settled, never read
        // twice.
        let listed = self.list_clients(id).await?;
        let mut owed = false;
        while !pending.is_empty() {
            owed |= self.attribute(&listed, pending).await?;
            pending = store.invitations(id).await?;
        }
        Ok(owed)
    }

    async fn attribute(&self, listed: &[Client], pending: Vec<Invitation>) -> Result<bool> {
        let store = &self.shared.store;
        let mut owed = false;
        for invitation in pending {
            let joined: Vec<_> = listed
                .iter()
                .filter(|client| !client.you && client.key.0.starts_with(&invitation.invitee))
                .map(|client| (client.key.0, client.role))
                .collect();
            let now: Timestamp = self.shared.clock.now();
            owed |= store
                .attribute_invitation(&invitation, &joined, now)
                .await?;
            let invitee = hex::Lower(&invitation.invitee).to_string();
            let inviter = hex::Lower(&invitation.inviter).to_string();
            info!(%invitee, %inviter, joined = joined.len(), "a join through an invite is settled");
        }
        Ok(owed)
    }

    /// [`Self::attribute_invitees`], logging a failure: the next pass tries
    /// again.
    pub(super) async fn try_attribute_invitees(&self, id: FeederId) {
        if let Err(error) = self.attribute_invitees(id).await {
            warn!(%error, "the joins through an invite wait for the next pass");
        }
    }
}
