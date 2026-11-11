//! Keys that joined through a phone key's invite.
//!
//! A phone key may ask the feeder for invites of its own, so whoever holds
//! it can enroll more keys that the server never invited. The feeder
//! reports each join through an invite with the first [`PREFIX`] bytes of
//! the key that joined and of the key that asked for the invite (an
//! `invited` event). The server records such a key under the account and
//! token of the phone key that invited it, and remembers that key, so
//! revoking the inviter however it is revoked (the account deleted, demoted
//! or given an expiry, the token revoked, or the key revoked by an owner)
//! revokes the keys it invited, and theirs in turn. A token's key replaced
//! by a new one hands its invitees to the new key instead, and an owner who
//! keeps an invitee takes it out of all of this. Keeping one key vouches
//! for that key alone: the keys it invited stay its inviter's account's,
//! and a join it makes once kept is attributed to no one.
//!
//! The prefixes are long enough that no one can grind a key sharing one,
//! so a join is never pinned on another phone; were two keys ever to share
//! one, the cascade matches by prefix and revokes too much, never too
//! little. A phone key the feeder already dropped leaves a row in
//! `dropped_phones`, so a join it made that the server reads only later is
//! revoked as soon as it is recorded. So does an invitee the feeder dropped
//! before the server attributed it, so a chain stays whole however long
//! the server was away.

use jiff::Timestamp;
use sqlx::SqliteConnection;
use uuid::Uuid;
use wellfed_api::{ClientRole, FeederId};

use crate::convert::{millis, timestamp};
use crate::projections::Place;
use crate::{Error, Result, Store};

/// How many of a key's first bytes an `invited` event names it by.
pub(super) const PREFIX: usize = 7;

/// A join through an invite that a feeder reported and the server has not
/// attributed yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Invitation {
    pub feeder: FeederId,
    pub epoch: u32,
    pub seq: u64,
    /// The first bytes of the key that joined.
    pub invitee: [u8; PREFIX],
    /// The first bytes of the key whose invite it redeemed.
    pub inviter: [u8; PREFIX],
    /// When the server read the join.
    pub received: Timestamp,
    /// Its place in the order the server recorded joins in.
    pub(crate) id: i64,
}

/// Whether the feeder lists any client whose key starts with a join's
/// invitee prefix, the server's own key included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Listed {
    /// Some client still holds a key with the prefix.
    Yes,
    /// None does: the key that joined left before the join was read.
    No,
}

/// The account and token a joined key is recorded under, and whether the
/// key that invited it is on its way out.
struct Inviter {
    user: Uuid,
    token: Option<Uuid>,
    revoking: bool,
}

/// Record an `invited` event's join, in its event's transaction. A replay
/// or a rebuild of the projections finds it there and changes nothing.
pub(crate) async fn project(
    conn: &mut SqliteConnection,
    id: FeederId,
    place: Place,
    invitee: [u8; PREFIX],
    inviter: [u8; PREFIX],
) -> Result<()> {
    let (invitee, inviter) = (invitee.as_slice(), inviter.as_slice());
    sqlx::query!(
        "INSERT INTO invitations (feeder_id, epoch, seq, invitee, inviter, received_ms)
         VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING",
        id.0,
        place.epoch,
        place.seq,
        invitee,
        inviter,
        place.received_ms,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Mark for revocation every key whose inviter is marked, until no more
/// are: a chain of invites goes whole. The feeders that hold one so marked.
pub(crate) async fn cascade(conn: &mut SqliteConnection, at: i64) -> Result<Vec<Uuid>> {
    let mut feeders = Vec::new();
    loop {
        let marked = sqlx::query!(
            r#"UPDATE phone_keys SET revoke = 'inviter_revoked', revoke_ms = ?1
               WHERE revoke IS NULL AND invited_by IS NOT NULL AND EXISTS (
                   SELECT 1 FROM phone_keys AS inviter
                   WHERE inviter.feeder_id = phone_keys.feeder_id
                     AND inviter.revoke IS NOT NULL
                     AND substr(inviter.key, 1, 7) = phone_keys.invited_by)
               RETURNING feeder_id AS "feeder_id: Uuid""#,
            at,
        )
        .fetch_all(&mut *conn)
        .await?;
        if marked.is_empty() {
            return Ok(feeders);
        }
        feeders.extend(marked.into_iter().map(|row| row.feeder_id));
    }
}

/// The feeder dropped phone key `key`: keep whose it was, and mark the
/// keys it invited, which go with it.
pub(crate) async fn dropped(
    conn: &mut SqliteConnection,
    feeder: FeederId,
    key: &[u8; 32],
    at: i64,
) -> Result<()> {
    let slice = key.as_slice();
    let Some(phone) = sqlx::query!(
        r#"SELECT user_id AS "user_id: Uuid", token_id AS "token_id: Uuid", last_invitation
           FROM phone_keys WHERE feeder_id = ? AND key = ?"#,
        feeder.0,
        slice,
    )
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(());
    };

    // A prefix dropped again keeps its earliest mark: a join the first key
    // made, read between the two, is still its.
    let prefix = prefix(key);
    let prefix = prefix.as_slice();
    sqlx::query!(
        "INSERT INTO dropped_phones
             (feeder_id, prefix, user_id, token_id, last_invitation, dropped_ms)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT (feeder_id, prefix) DO UPDATE SET
             user_id = excluded.user_id, token_id = excluded.token_id,
             last_invitation = MIN(last_invitation, excluded.last_invitation),
             dropped_ms = excluded.dropped_ms",
        feeder.0,
        prefix,
        phone.user_id,
        phone.token_id,
        phone.last_invitation,
        at,
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!(
        "UPDATE phone_keys SET revoke = 'inviter_revoked', revoke_ms = ?
         WHERE feeder_id = ? AND revoke IS NULL AND invited_by = ?",
        at,
        feeder.0,
        prefix,
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

fn prefix(key: &[u8; 32]) -> [u8; PREFIX] {
    let mut prefix = [0; PREFIX];
    prefix.copy_from_slice(&key[..PREFIX]);
    prefix
}

impl Store {
    /// The joins through an invite a feeder reported that the server has
    /// not attributed yet and can: those whose inviter is a phone key it
    /// recorded, or saw dropped, before it recorded the join (by the order
    /// it recorded them in, not by its clock). Oldest first, so
    /// a key's own invitees come after it. Any other join (the server's own
    /// adoption, a key the owner's CLI invited, or one a phone made before
    /// the server knew it) asks the feeder nothing and is never attributed:
    /// claiming a phone does not sweep in the invites it made before.
    pub async fn invitations(&self, feeder: FeederId) -> Result<Vec<Invitation>> {
        let rows = sqlx::query!(
            r#"SELECT id AS "id!", epoch, seq, invitee, inviter, received_ms FROM invitations AS i
               WHERE feeder_id = ?1 AND settled_ms IS NULL
                 AND (EXISTS (SELECT 1 FROM phone_keys AS p
                              WHERE p.feeder_id = ?1 AND substr(p.key, 1, 7) = i.inviter
                                AND p.last_invitation < i.id)
                      OR EXISTS (SELECT 1 FROM dropped_phones AS d
                                 WHERE d.feeder_id = ?1 AND d.prefix = i.inviter
                                   AND d.last_invitation < i.id))
               ORDER BY id"#,
            feeder.0,
        )
        .fetch_all(&self.reader)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(Invitation {
                    feeder,
                    epoch: u32::try_from(row.epoch)
                        .map_err(|_| Error::Corrupt("invitation epoch"))?,
                    seq: u64::try_from(row.seq).map_err(|_| Error::Corrupt("invitation seq"))?,
                    invitee: row
                        .invitee
                        .try_into()
                        .map_err(|_| Error::Corrupt("invitee prefix"))?,
                    inviter: row
                        .inviter
                        .try_into()
                        .map_err(|_| Error::Corrupt("inviter prefix"))?,
                    received: timestamp(row.received_ms)?,
                    id: row.id,
                })
            })
            .collect()
    }

    /// Settle `invitation`. Each of `joined` (the feeder's clients whose
    /// key starts with the invitee's prefix, with the role it lists) is
    /// recorded as a phone key of the account and token behind the
    /// inviter, a phone key recorded or one the feeder dropped before the
    /// join was, and marked for revocation at once unless that key still
    /// stands. It counts as recorded from the join on, so the joins it made
    /// in turn are its. A key recorded or kept already
    /// stays as it is, and a join no such phone key invited records nothing.
    /// With no client listed under the invitee's prefix (`listed`; the
    /// feeder dropped it first), the prefix is recorded as a dropped phone
    /// key's, so the keys it invited go too. A prefix still listed but not
    /// in `joined` (the server's own key, rejoined through a phone's invite)
    /// records nothing. Whether a key it recorded is owed a revoke.
    pub async fn attribute_invitation(
        &self,
        invitation: &Invitation,
        joined: &[([u8; 32], ClientRole)],
        listed: Listed,
        at: Timestamp,
    ) -> Result<bool> {
        let feeder = invitation.feeder;
        let inviter_prefix = invitation.inviter.as_slice();
        let received = millis(invitation.received);
        let order = invitation.id;
        let at = millis(at);
        let mut tx = self.writer.begin().await?;

        let recorded = sqlx::query!(
            r#"SELECT user_id AS "user_id: Uuid", token_id AS "token_id: Uuid",
                      revoke IS NOT NULL AS "revoking!: bool"
               FROM phone_keys
               WHERE feeder_id = ? AND substr(key, 1, 7) = ? AND last_invitation < ?
               ORDER BY last_invitation, key LIMIT 1"#,
            feeder.0,
            inviter_prefix,
            order,
        )
        .fetch_optional(&mut *tx)
        .await?
        .map(|row| Inviter {
            user: row.user_id,
            token: row.token_id,
            revoking: row.revoking,
        });
        let inviter = match recorded {
            Some(inviter) => Some(inviter),
            None => sqlx::query!(
                r#"SELECT user_id AS "user_id: Uuid", token_id AS "token_id: Uuid"
                   FROM dropped_phones
                   WHERE feeder_id = ? AND prefix = ? AND last_invitation < ?"#,
                feeder.0,
                inviter_prefix,
                order,
            )
            .fetch_optional(&mut *tx)
            .await?
            .map(|row| Inviter {
                user: row.user_id,
                token: row.token_id,
                revoking: true,
            }),
        };

        let mut owed = false;
        if let Some(inviter) = inviter {
            let revoke = inviter.revoking.then_some("inviter_revoked");
            let revoke_ms = inviter.revoking.then_some(at);
            for (key, role) in joined {
                let key = key.as_slice();
                let role = role.code();
                let added = sqlx::query!(
                    "INSERT INTO phone_keys (feeder_id, key, user_id, token_id, role, invited_ms,
                                             invite_until_ms, invited_by, revoke, revoke_ms,
                                             last_invitation)
                     SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8, ?9, ?10
                     WHERE NOT EXISTS (SELECT 1 FROM phone_keys WHERE feeder_id = ?1 AND key = ?2)
                       AND NOT EXISTS (SELECT 1 FROM kept_clients WHERE feeder_id = ?1 AND key = ?2)",
                    feeder.0,
                    key,
                    inviter.user,
                    inviter.token,
                    role,
                    received,
                    inviter_prefix,
                    revoke,
                    revoke_ms,
                    order,
                )
                .execute(&mut *tx)
                .await?;
                owed |= inviter.revoking && added.rows_affected() == 1;
            }
            // The feeder no longer lists the key that joined: it left before
            // the server read its join. Its own joins are still the
            // inviter's, and they go as they are recorded, as a dropped
            // phone key's do.
            if listed == Listed::No {
                let invitee = invitation.invitee.as_slice();
                sqlx::query!(
                    "INSERT INTO dropped_phones
                         (feeder_id, prefix, user_id, token_id, last_invitation, dropped_ms)
                     VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING",
                    feeder.0,
                    invitee,
                    inviter.user,
                    inviter.token,
                    order,
                    at,
                )
                .execute(&mut *tx)
                .await?;
            }
        }

        let (epoch, seq) = (
            i64::from(invitation.epoch),
            i64::try_from(invitation.seq).map_err(|_| Error::Corrupt("invitation seq"))?,
        );
        sqlx::query!(
            "UPDATE invitations SET settled_ms = ? WHERE feeder_id = ? AND epoch = ? AND seq = ?",
            at,
            feeder.0,
            epoch,
            seq,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(owed)
    }
}
