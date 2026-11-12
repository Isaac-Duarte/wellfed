//! The reconcile: phone keys whose account or token no longer may hold
//! one, marked whatever change left them so.

use jiff::Timestamp;
use uuid::Uuid;
use wellfed_api::{ClientRole, FeederId, RevokeReason, Role};

use crate::convert::millis;
use crate::{Result, Store};

impl Store {
    /// Mark every key whose account or token no longer may hold one,
    /// whatever change left it so: a deleted account or token, a viewer, an
    /// owner's key on an account now a member's, an account or token with
    /// an expiry, and every key a marked key invited. The account changes
    /// mark their keys themselves; this catches a key recorded while its
    /// account changed. The feeders that hold any key so marked.
    pub async fn reconcile_phones(&self, at: Timestamp) -> Result<Vec<FeederId>> {
        let at = millis(at);
        // The spellings the enums own, bound rather than written here, so
        // a reader never meets one it does not know.
        let reason = |reason: RevokeReason| reason.code();
        let (viewer, member) = (Role::Viewer.code(), Role::Member.code());
        let owner_key = ClientRole::Owner.code();
        let mut tx = self.writer.begin().await?;
        let deleted = sqlx::query!(
            r#"UPDATE phone_keys SET revoke = ?, revoke_ms = ?
               WHERE revoke IS NULL AND user_id NOT IN (SELECT id FROM users)
               RETURNING feeder_id AS "feeder_id: Uuid""#,
            reason(RevokeReason::Deleted),
            at,
        )
        .fetch_all(&mut *tx)
        .await?;
        let token = sqlx::query!(
            r#"UPDATE phone_keys SET revoke = ?, revoke_ms = ?
               WHERE revoke IS NULL AND token_id IS NOT NULL
                 AND token_id NOT IN (SELECT id FROM tokens)
               RETURNING feeder_id AS "feeder_id: Uuid""#,
            reason(RevokeReason::TokenRevoked),
            at,
        )
        .fetch_all(&mut *tx)
        .await?;
        let demoted = sqlx::query!(
            r#"UPDATE phone_keys SET revoke = ?, revoke_ms = ?
               WHERE revoke IS NULL AND EXISTS (
                   SELECT 1 FROM users WHERE users.id = phone_keys.user_id
                     AND (users.role = ?
                          OR (users.role = ? AND phone_keys.role = ?)))
               RETURNING feeder_id AS "feeder_id: Uuid""#,
            reason(RevokeReason::Demoted),
            at,
            viewer,
            member,
            owner_key,
        )
        .fetch_all(&mut *tx)
        .await?;
        let expiring = sqlx::query!(
            r#"UPDATE phone_keys SET revoke = ?, revoke_ms = ?
               WHERE revoke IS NULL AND (
                   EXISTS (SELECT 1 FROM users WHERE users.id = phone_keys.user_id
                             AND users.expires_ms IS NOT NULL)
                   OR EXISTS (SELECT 1 FROM tokens WHERE tokens.id = phone_keys.token_id
                                AND tokens.expires_ms IS NOT NULL))
               RETURNING feeder_id AS "feeder_id: Uuid""#,
            reason(RevokeReason::Expiring),
            at,
        )
        .fetch_all(&mut *tx)
        .await?;
        let invited = super::invitees::cascade(&mut tx, at).await?;
        tx.commit().await?;

        let mut feeders: Vec<FeederId> = deleted
            .into_iter()
            .map(|row| row.feeder_id)
            .chain(token.into_iter().map(|row| row.feeder_id))
            .chain(demoted.into_iter().map(|row| row.feeder_id))
            .chain(expiring.into_iter().map(|row| row.feeder_id))
            .chain(invited)
            .map(FeederId)
            .collect();
        feeders.sort_unstable();
        feeders.dedup();
        Ok(feeders)
    }
}
