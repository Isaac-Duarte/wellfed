//! What a meal call write recorded of the feeder as it was sent: the clip
//! it found first and, for an upload, whether the feeder took its start.
//! A retry reads them so it never undoes a change that came after.

use crate::{Error, Result, Store};

/// The meal call a call write found on the feeder before its first send.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "held")]
pub enum CallBefore {
    /// No clip, and no upload under way.
    None,
    /// A clip of `bytes` stream bytes with CRC `crc`, stored or arriving.
    Clip { bytes: u32, crc: u32 },
}

impl Store {
    /// Keep what the feeder held of the meal call as `key`'s write was
    /// first sent.
    pub async fn record_call_before(&self, key: &[u8; 16], before: CallBefore) -> Result<()> {
        let key = key.as_slice();
        let json = serde_json::to_string(&before).map_err(|e| Error::Json("call before", e))?;
        sqlx::query!(
            "UPDATE commands SET call_before = ? WHERE key = ?",
            json,
            key
        )
        .execute(&self.writer)
        .await?;
        Ok(())
    }

    /// Keep that the feeder took `key`'s upload's start, which erased the
    /// clip it held.
    pub async fn record_call_started(&self, key: &[u8; 16]) -> Result<()> {
        let key = key.as_slice();
        sqlx::query!("UPDATE commands SET call_started = 1 WHERE key = ?", key)
            .execute(&self.writer)
            .await?;
        Ok(())
    }

    /// What `key`'s call write recorded of the feeder; `None` with no
    /// entry under `key`.
    pub async fn call_write(&self, key: &[u8; 16]) -> Result<Option<CallWrite>> {
        let key = key.as_slice();
        let row = sqlx::query!(
            "SELECT call_before, call_started FROM commands WHERE key = ?",
            key
        )
        .fetch_optional(&self.reader)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };

        let before = row
            .call_before
            .map(|json| serde_json::from_str(&json).map_err(|_| Error::Corrupt("call before")))
            .transpose()?;
        Ok(Some(CallWrite {
            before,
            started: row.call_started != 0,
        }))
    }
}

/// What a call write recorded of the feeder as it was sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallWrite {
    /// The meal call the first send found; `None` when none was recorded.
    pub before: Option<CallBefore>,
    /// Whether the feeder took the upload's start, erasing what it held.
    pub started: bool,
}
