//! Session persistence, expiry, and the password used to open one.

use jiff::Timestamp;
use uuid::Uuid;
use wellfed_api::UserId;

use super::{SessionRow, user};
use crate::convert::{millis, timestamp};
use crate::{Result, Store};

impl Store {
    pub async fn create_session(
        &self,
        token_hash: &[u8; 32],
        user: UserId,
        now: Timestamp,
        password_hash: &str,
    ) -> Result<bool> {
        let hash = token_hash.as_slice();
        let now = millis(now);
        // The comparison and insert are one statement: a password reset
        // between verification and this write cannot create a new session.
        let done = sqlx::query!(
            "INSERT INTO sessions (token_hash, user_id, created_ms, last_used_ms)
             SELECT ?, id, ?, ? FROM users WHERE id = ? AND password_hash = ?",
            hash,
            now,
            now,
            user.0,
            password_hash,
        )
        .execute(&self.writer)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// The session with this token, and its user, however long ago it was
    /// last used: the caller decides whether it still holds.
    pub async fn session(&self, token_hash: &[u8; 32]) -> Result<Option<SessionRow>> {
        let hash = token_hash.as_slice();
        let row = sqlx::query!(
            r#"SELECT users.id AS "id: Uuid", users.name, users.role, users.expires_ms,
                      sessions.last_used_ms
               FROM sessions JOIN users ON users.id = sessions.user_id
               WHERE sessions.token_hash = ?"#,
            hash,
        )
        .fetch_optional(&self.reader)
        .await?;
        row.map(|row| {
            Ok(SessionRow {
                user: user(row.id, row.name, &row.role, row.expires_ms)?,
                last_used: timestamp(row.last_used_ms)?,
            })
        })
        .transpose()
    }

    /// Record that the session was used at `now`.
    pub async fn touch_session(&self, token_hash: &[u8; 32], now: Timestamp) -> Result<()> {
        let hash = token_hash.as_slice();
        let now = millis(now);
        sqlx::query!(
            "UPDATE sessions SET last_used_ms = ? WHERE token_hash = ? AND last_used_ms < ?",
            now,
            hash,
            now,
        )
        .execute(&self.writer)
        .await?;
        Ok(())
    }

    pub async fn delete_session(&self, token_hash: &[u8; 32]) -> Result<()> {
        let hash = token_hash.as_slice();
        sqlx::query!("DELETE FROM sessions WHERE token_hash = ?", hash)
            .execute(&self.writer)
            .await?;
        Ok(())
    }

    /// End every session of `user` but `except`; how many there were.
    pub async fn delete_sessions_of(&self, user: UserId, except: Option<&[u8; 32]>) -> Result<u64> {
        let keep = except.map(<[u8; 32]>::as_slice);
        let done = sqlx::query!(
            "DELETE FROM sessions WHERE user_id = ? AND token_hash IS NOT ?",
            user.0,
            keep,
        )
        .execute(&self.writer)
        .await?;
        Ok(done.rows_affected())
    }

    /// The hash a user's password is checked against.
    pub async fn password_hash(&self, id: UserId) -> Result<Option<String>> {
        let row = sqlx::query!("SELECT password_hash FROM users WHERE id = ?", id.0)
            .fetch_optional(&self.reader)
            .await?;
        Ok(row.map(|row| row.password_hash))
    }

    /// End every session last used at or before `idle`, and every session
    /// of an account expired at `now`; how many there were.
    pub async fn delete_ended_sessions(&self, idle: Timestamp, now: Timestamp) -> Result<u64> {
        let idle = millis(idle);
        let now = millis(now);
        let done = sqlx::query!(
            "DELETE FROM sessions WHERE last_used_ms <= ? OR user_id IN
                 (SELECT id FROM users WHERE expires_ms IS NOT NULL AND expires_ms <= ?)",
            idle,
            now,
        )
        .execute(&self.writer)
        .await?;
        Ok(done.rows_affected())
    }
}
