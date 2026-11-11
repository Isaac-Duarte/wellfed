//! Marking phone keys for revocation, the attempts on them, and
//! forgetting a key the feeder dropped.

use jiff::Timestamp;
use uuid::Uuid;
use wellfed_api::{FeederId, PhoneKey, RevokeAttempt, RevokeReason};

use super::{Whose, invitees};
use crate::convert::millis;
use crate::{Error, Result, Store};

impl Store {
    /// Mark `whose` phone keys for revocation, those not already marked,
    /// and the keys they invited; the feeders that hold any key so marked.
    pub async fn revoke_phones(
        &self,
        whose: Whose,
        reason: RevokeReason,
        at: Timestamp,
    ) -> Result<Vec<FeederId>> {
        let reason = reason.code();
        let at = millis(at);
        let (user, token, feeder, keep) = match whose {
            Whose::User(user) => (Some(user.0), None, None, None),
            Whose::Token(token) => (None, Some(token.0), None, None),
            Whose::Replaced {
                feeder,
                token,
                keep,
            } => (None, Some(token.0), Some(feeder.0), Some(keep.to_vec())),
        };
        let mut tx = self.writer.begin().await?;
        let rows = sqlx::query!(
            r#"UPDATE phone_keys SET revoke = ?1, revoke_ms = ?2
               WHERE revoke IS NULL
                 AND (?3 IS NULL OR user_id = ?3)
                 AND (?4 IS NULL OR token_id = ?4)
                 AND (?5 IS NULL OR feeder_id = ?5)
                 AND (?6 IS NULL OR key != ?6)
                 AND (?3 IS NOT NULL OR ?4 IS NOT NULL)
               RETURNING feeder_id AS "feeder_id: Uuid""#,
            reason,
            at,
            user,
            token,
            feeder,
            keep,
        )
        .fetch_all(&mut *tx)
        .await?;
        let invited = invitees::cascade(&mut tx, at).await?;
        tx.commit().await?;

        let mut feeders: Vec<FeederId> = rows
            .into_iter()
            .map(|row| row.feeder_id)
            .chain(invited)
            .map(FeederId)
            .collect();
        feeders.sort_unstable();
        feeders.dedup();
        Ok(feeders)
    }

    /// The phone keys on a feeder still to be revoked.
    pub async fn revoking(&self, feeder: FeederId) -> Result<Vec<PhoneKey>> {
        let phones = self.feeder_phones(feeder).await?;
        Ok(phones
            .into_iter()
            .filter(|phone| phone.revoking.is_some())
            .collect())
    }

    /// Record what an attempt to revoke a phone key came to.
    pub async fn revoke_attempt(
        &self,
        feeder: FeederId,
        key: &[u8; 32],
        attempt: &RevokeAttempt,
    ) -> Result<()> {
        let key = key.as_slice();
        let json = serde_json::to_string(attempt).map_err(|e| Error::Json("revoke attempt", e))?;
        sqlx::query!(
            "UPDATE phone_keys SET attempt = ? WHERE feeder_id = ? AND key = ?",
            json,
            feeder.0,
            key,
        )
        .execute(&self.writer)
        .await?;
        Ok(())
    }

    /// The feeder no longer holds the key: forget it as a phone's and as a
    /// kept client, and mark the keys a phone key invited, which go with
    /// it. Whether any key is so marked.
    pub async fn forget_client(
        &self,
        feeder: FeederId,
        key: &[u8; 32],
        at: Timestamp,
    ) -> Result<bool> {
        let at = millis(at);
        let mut tx = self.writer.begin().await?;
        invitees::dropped(&mut tx, feeder, key, at).await?;
        let key = key.as_slice();
        sqlx::query!(
            "DELETE FROM phone_keys WHERE feeder_id = ? AND key = ?",
            feeder.0,
            key
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "DELETE FROM kept_clients WHERE feeder_id = ? AND key = ?",
            feeder.0,
            key
        )
        .execute(&mut *tx)
        .await?;
        let invited = invitees::cascade(&mut tx, at).await?;
        let owed = sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM phone_keys WHERE feeder_id = ? AND revoke IS NOT NULL)
               AS "owed!: bool""#,
            feeder.0,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(owed || !invited.is_empty())
    }
}
