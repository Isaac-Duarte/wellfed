//! The phone keys the server made invites for and the keys they invited
//! ([`invitees`]), the revocations still owed on them, and the clients it
//! did not invite that an owner kept.

use jiff::Timestamp;
use uuid::Uuid;
use wellfed_api::{
    ClientRole, FeederId, Hex, PhoneKey, RevokeAttempt, RevokeReason, Revoking, TokenId, UserId,
};

use crate::convert::{lenient, millis, timestamp};
use crate::{Error, Result, Store};

mod invitees;
mod reconcile;
mod revoke;

pub(crate) use invitees::project as project_invitation;
pub use invitees::{Invitation, Listed};

/// A phone key about to be invited.
pub struct NewPhone {
    pub feeder: FeederId,
    pub key: [u8; 32],
    pub user: UserId,
    pub token: TokenId,
    pub role: ClientRole,
    pub invited: Timestamp,
    /// A bound on the invite's deadline while its answer is pending.
    pub invite_until: Timestamp,
}

/// What [`Store::record_phone`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recorded {
    /// The key is new: recorded.
    New,
    /// The key is already this token's, and not being revoked: asked again
    /// (a lost invite), its deadline raised to the new bound. `until` is
    /// the deadline it had.
    Again { until: Timestamp },
    /// The key is another token's, or being revoked: nothing changed.
    Taken,
}

/// Whose phone keys [`Store::revoke_phones`] marks.
#[derive(Clone, Copy, Debug)]
pub enum Whose {
    User(UserId),
    Token(TokenId),
    /// The token's other keys on the feeder: one key per token.
    Replaced {
        feeder: FeederId,
        token: TokenId,
        keep: [u8; 32],
    },
}

/// One `phone_keys` row as selected.
struct Raw {
    feeder_id: Uuid,
    key: Vec<u8>,
    user_id: Uuid,
    token_id: Option<Uuid>,
    role: String,
    invited_ms: i64,
    invite_until_ms: i64,
    revoke: Option<String>,
    revoke_ms: Option<i64>,
    attempt: Option<String>,
}

impl TryFrom<Raw> for PhoneKey {
    type Error = Error;

    fn try_from(raw: Raw) -> Result<Self> {
        let revoking = match (raw.revoke, raw.revoke_ms) {
            (Some(reason), Some(since)) => Some(Revoking {
                reason: RevokeReason::from_code(&reason).ok_or(Error::Corrupt("revoke reason"))?,
                since: timestamp(since)?,
                last: raw
                    .attempt
                    .map(|json| lenient(&json, RevokeAttempt::Unknown, "revoke attempt"))
                    .transpose()?,
            }),
            (None, None) => None,
            _ => return Err(Error::Corrupt("revocation without its time")),
        };
        Ok(Self {
            feeder: FeederId(raw.feeder_id),
            key: Hex(raw
                .key
                .try_into()
                .map_err(|_| Error::Corrupt("phone key"))?),
            user: UserId(raw.user_id),
            token: raw.token_id.map(TokenId),
            role: ClientRole::from_code(&raw.role).ok_or(Error::Corrupt("phone role"))?,
            invited: timestamp(raw.invited_ms)?,
            invite_until: timestamp(raw.invite_until_ms)?,
            revoking,
        })
    }
}

impl Store {
    /// Record a phone key for `phone.token` before its invite is asked
    /// for, unless another token holds it or it is being revoked.
    pub async fn record_phone(&self, phone: &NewPhone) -> Result<Recorded> {
        let key = phone.key.as_slice();
        let role = phone.role.code();
        let invited = millis(phone.invited);
        let until = millis(phone.invite_until);
        let mut tx = self.writer.begin().await?;
        let existing = sqlx::query!(
            r#"SELECT token_id AS "token_id: Uuid", invite_until_ms, revoke
               FROM phone_keys WHERE feeder_id = ? AND key = ?"#,
            phone.feeder.0,
            key,
        )
        .fetch_optional(&mut *tx)
        .await?;

        let recorded = match existing {
            None => {
                // Only a join recorded from here on can be this key's.
                sqlx::query!(
                    "INSERT INTO phone_keys
                         (feeder_id, key, user_id, token_id, role, invited_ms, invite_until_ms,
                          last_invitation)
                     VALUES (?, ?, ?, ?, ?, ?, ?,
                             (SELECT COALESCE(MAX(id), 0) FROM invitations))",
                    phone.feeder.0,
                    key,
                    phone.user.0,
                    phone.token.0,
                    role,
                    invited,
                    until,
                )
                .execute(&mut *tx)
                .await?;
                Recorded::New
            }
            Some(row) if row.token_id == Some(phone.token.0) && row.revoke.is_none() => {
                sqlx::query!(
                    "UPDATE phone_keys SET invite_until_ms = MAX(invite_until_ms, ?)
                     WHERE feeder_id = ? AND key = ?",
                    until,
                    phone.feeder.0,
                    key,
                )
                .execute(&mut *tx)
                .await?;
                Recorded::Again {
                    until: timestamp(row.invite_until_ms)?,
                }
            }
            Some(_) => Recorded::Taken,
        };
        tx.commit().await?;
        Ok(recorded)
    }

    /// Move the key's invite deadline from `expected`, the bound this
    /// request recorded, to `until`, once the feeder has said it or no
    /// invite was made. Only while the deadline is still `expected`: a
    /// request for the same key that raised it since keeps its own bound,
    /// so a deadline is never lowered below a live invite's. Whether it
    /// moved.
    pub async fn settle_invite_until(
        &self,
        feeder: FeederId,
        key: &[u8; 32],
        expected: Timestamp,
        until: Timestamp,
    ) -> Result<bool> {
        let key = key.as_slice();
        let expected = millis(expected);
        let until = millis(until);
        let done = sqlx::query!(
            "UPDATE phone_keys SET invite_until_ms = ?
             WHERE feeder_id = ? AND key = ? AND invite_until_ms = ?",
            until,
            feeder.0,
            key,
            expected,
        )
        .execute(&self.writer)
        .await?;
        Ok(done.rows_affected() == 1)
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

    /// Forget a key this request recorded new, once no invite was made for
    /// it: only while its deadline is still the bound it recorded, `expected`.
    pub async fn unrecord_phone(
        &self,
        feeder: FeederId,
        key: &[u8; 32],
        expected: Timestamp,
    ) -> Result<()> {
        let key = key.as_slice();
        let expected = millis(expected);
        sqlx::query!(
            "DELETE FROM phone_keys
             WHERE feeder_id = ? AND key = ? AND invite_until_ms = ? AND revoke IS NULL",
            feeder.0,
            key,
            expected,
        )
        .execute(&self.writer)
        .await?;
        Ok(())
    }

    pub async fn phone(&self, feeder: FeederId, key: &[u8; 32]) -> Result<Option<PhoneKey>> {
        let key = key.as_slice();
        let row = sqlx::query_as!(
            Raw,
            r#"SELECT feeder_id AS "feeder_id: Uuid", key, user_id AS "user_id: Uuid",
                      token_id AS "token_id: Uuid", role, invited_ms, invite_until_ms,
                      revoke, revoke_ms, attempt
               FROM phone_keys WHERE feeder_id = ? AND key = ?"#,
            feeder.0,
            key,
        )
        .fetch_optional(&self.reader)
        .await?;
        row.map(TryInto::try_into).transpose()
    }

    /// Every phone key recorded: one user's, or everyone's.
    pub async fn phones(&self, user: Option<UserId>) -> Result<Vec<PhoneKey>> {
        let user = user.map(|user| user.0);
        let rows = sqlx::query_as!(
            Raw,
            r#"SELECT feeder_id AS "feeder_id: Uuid", key, user_id AS "user_id: Uuid",
                      token_id AS "token_id: Uuid", role, invited_ms, invite_until_ms,
                      revoke, revoke_ms, attempt
               FROM phone_keys WHERE ?1 IS NULL OR user_id = ?1
               ORDER BY invited_ms, feeder_id, key"#,
            user,
        )
        .fetch_all(&self.reader)
        .await?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    /// The phone keys recorded on one feeder.
    pub async fn feeder_phones(&self, feeder: FeederId) -> Result<Vec<PhoneKey>> {
        let rows = sqlx::query_as!(
            Raw,
            r#"SELECT feeder_id AS "feeder_id: Uuid", key, user_id AS "user_id: Uuid",
                      token_id AS "token_id: Uuid", role, invited_ms, invite_until_ms,
                      revoke, revoke_ms, attempt
               FROM phone_keys WHERE feeder_id = ? ORDER BY invited_ms, key"#,
            feeder.0,
        )
        .fetch_all(&self.reader)
        .await?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    /// Keep a client the server did not invite.
    /// A key a phone key invited is detached from it as it is kept: it is
    /// no longer that account's phone key, so neither the account, its
    /// token nor the inviting key revokes it, and a revoke owed on it is
    /// owed no more. A key the server invited itself stays its account's.
    pub async fn keep_client(&self, feeder: FeederId, key: &[u8; 32], at: Timestamp) -> Result<()> {
        let key = key.as_slice();
        let at = millis(at);
        let mut tx = self.writer.begin().await?;
        sqlx::query!(
            "DELETE FROM phone_keys WHERE feeder_id = ? AND key = ? AND invited_by IS NOT NULL",
            feeder.0,
            key,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "INSERT INTO kept_clients (feeder_id, key, kept_ms) VALUES (?, ?, ?)
             ON CONFLICT DO NOTHING",
            feeder.0,
            key,
            at,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

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

    /// The clients an owner kept on a feeder.
    pub async fn kept_clients(&self, feeder: FeederId) -> Result<Vec<[u8; 32]>> {
        let rows = sqlx::query!(
            "SELECT key FROM kept_clients WHERE feeder_id = ? ORDER BY kept_ms",
            feeder.0
        )
        .fetch_all(&self.reader)
        .await?;
        rows.into_iter()
            .map(|row| {
                row.key
                    .try_into()
                    .map_err(|_| Error::Corrupt("kept client key"))
            })
            .collect()
    }
}
