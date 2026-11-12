//! Idempotency-Keys a journal read found no write under. A client takes
//! that answer as never sent, so the journal takes no write under the key
//! after it ([`Journaled::Retired`](crate::Journaled::Retired)): a request
//! stalled in front of the journal never runs after its client gave it up.

use jiff::Timestamp;

use crate::convert::millis;
use crate::{Result, Store};

impl Store {
    /// Retire `key` unless the journal holds a write under it: whether it
    /// is retired now. One statement, so a write journaled at the same
    /// moment either is found, or finds the key retired.
    pub async fn retire_key(&self, key: &[u8; 16], at: Timestamp) -> Result<bool> {
        let key = key.as_slice();
        let at = millis(at);
        sqlx::query!(
            "INSERT INTO retired_keys (key, retired_ms)
             SELECT ?1, ?2 WHERE NOT EXISTS (SELECT 1 FROM commands WHERE key = ?1)
             ON CONFLICT (key) DO NOTHING",
            key,
            at,
        )
        .execute(&self.writer)
        .await?;
        let retired = sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM retired_keys WHERE key = ?) AS "retired: bool""#,
            key
        )
        .fetch_one(&self.writer)
        .await?;
        Ok(retired)
    }

    /// Drop the keys retired before `before`: no request outlasts that.
    pub async fn forget_retired_keys(&self, before: Timestamp) -> Result<u64> {
        let before = millis(before);
        let done = sqlx::query!("DELETE FROM retired_keys WHERE retired_ms < ?", before)
            .execute(&self.writer)
            .await?;
        Ok(done.rows_affected())
    }
}
