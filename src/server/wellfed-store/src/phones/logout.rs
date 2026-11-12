//! A phone's logout, which leaves its own keys the account's with no
//! token, and a claim that puts such a key under the phone's new token.

use jiff::Timestamp;
use uuid::Uuid;
use wellfed_api::{FeederId, TokenId, UserId};

use super::invitees;
use crate::convert::millis;
use crate::{Error, Result, Store};

impl Store {
    /// Log a phone out, in one transaction: take `token` off the phone's
    /// own keys under it (those in `own`) and the keys they invited, in
    /// turn, which stay the account's; mark every other key under it for
    /// revocation, as the token's revocation would; and delete the token.
    /// The phone names its own keys because the token alone does not prove
    /// which keys under it are the phone's: a key someone else got through
    /// the token is revoked with it. A named key not under the token is
    /// passed over. A key already on its way out still goes. `None` if the
    /// token was not there; else the feeders that hold a key now marked.
    pub async fn log_out_token(
        &self,
        token: TokenId,
        own: &[[u8; 32]],
        at: Timestamp,
    ) -> Result<Option<Vec<FeederId>>> {
        let at = millis(at);
        let mut tx = self.writer.begin().await?;
        let deleted = sqlx::query!("DELETE FROM tokens WHERE id = ?", token.0)
            .execute(&mut *tx)
            .await?;
        if deleted.rows_affected() == 0 {
            return Ok(None);
        }

        // The phone's own keys, then the keys each invited, until none.
        let mut kept: Vec<(Uuid, Vec<u8>)> = Vec::new();
        for key in own {
            let key = key.as_slice();
            let rows = sqlx::query!(
                r#"UPDATE phone_keys SET token_id = NULL
                   WHERE token_id = ? AND key = ? AND revoke IS NULL
                   RETURNING feeder_id AS "feeder_id: Uuid", key"#,
                token.0,
                key,
            )
            .fetch_all(&mut *tx)
            .await?;
            kept.extend(rows.into_iter().map(|row| (row.feeder_id, row.key)));
        }
        while let Some((feeder, key)) = kept.pop() {
            let prefix = key
                .get(..invitees::PREFIX)
                .ok_or(Error::Corrupt("phone key"))?;
            let rows = sqlx::query!(
                r#"UPDATE phone_keys SET token_id = NULL
                   WHERE feeder_id = ? AND token_id = ? AND invited_by = ? AND revoke IS NULL
                   RETURNING feeder_id AS "feeder_id: Uuid", key"#,
                feeder,
                token.0,
                prefix,
            )
            .fetch_all(&mut *tx)
            .await?;
            kept.extend(rows.into_iter().map(|row| (row.feeder_id, row.key)));
        }

        let marked = sqlx::query!(
            r#"UPDATE phone_keys SET revoke = 'token_revoked', revoke_ms = ?
               WHERE token_id = ? AND revoke IS NULL
               RETURNING feeder_id AS "feeder_id: Uuid""#,
            at,
            token.0,
        )
        .fetch_all(&mut *tx)
        .await?;
        let invited = invitees::cascade(&mut tx, at).await?;
        tx.commit().await?;

        let mut feeders: Vec<FeederId> = marked
            .into_iter()
            .map(|row| row.feeder_id)
            .chain(invited)
            .map(FeederId)
            .collect();
        feeders.sort_unstable();
        feeders.dedup();
        Ok(Some(feeders))
    }

    /// Put the account's key with no token (one a logout left it) under
    /// `token`: the account's phone claimed it again. Whether it moved.
    pub async fn attach_token(
        &self,
        feeder: FeederId,
        key: &[u8; 32],
        user: UserId,
        token: TokenId,
    ) -> Result<bool> {
        let key = key.as_slice();
        let done = sqlx::query!(
            "UPDATE phone_keys SET token_id = ?
             WHERE feeder_id = ? AND key = ? AND user_id = ? AND token_id IS NULL
               AND revoke IS NULL",
            token.0,
            feeder.0,
            key,
            user.0,
        )
        .execute(&self.writer)
        .await?;
        Ok(done.rows_affected() == 1)
    }
}
