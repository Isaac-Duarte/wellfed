//! A feeder's open alerts, a feed's trace, the latest statistics, and
//! following a feeder's debug log (`GET /feeders/{id}/log`, Server-Sent
//! Events).

use reqwest::header::{self, HeaderValue};
use reqwest::{Method, StatusCode};
use wellfed_api::{FeedTrace, FeederAlerts, FeederId, FeederStats, LogEvent, TraceQuery};

use crate::stream::Blocks;
use crate::{Client, Error, decode, decode_page, decode_page_counted, expect_status};

impl Client {
    /// The feeder's open alerts, as the server last read its table. An
    /// alert this client cannot read (a kind a newer feeder added) is left
    /// out and counted in the read's `unread`, as a device API session
    /// counts one.
    pub async fn feeder_alerts(&self, id: FeederId) -> Result<FeederAlerts, Error> {
        let response = self
            .api(Method::GET, &format!("/feeders/{id}/alerts"))
            .send()
            .await?;
        let (mut alerts, skipped): (FeederAlerts, usize) =
            decode_page_counted(response, StatusCode::OK, "alerts").await?;
        if let Some(read) = &mut alerts.read {
            let skipped = u8::try_from(skipped).unwrap_or(u8::MAX);
            read.unread = read.unread.saturating_add(skipped);
        }
        Ok(alerts)
    }

    /// Feed `feed`'s trace, as the server fetched it after the feed.
    pub async fn feed_trace(
        &self,
        id: FeederId,
        feed: u32,
        query: &TraceQuery,
    ) -> Result<FeedTrace, Error> {
        let response = self
            .api(Method::GET, &format!("/feeders/{id}/feeds/{feed}/trace"))
            .query(query)
            .send()
            .await?;
        decode(response, StatusCode::OK).await
    }

    /// The latest read of each statistics section. A section this client
    /// cannot read (one a newer feeder added) is left out.
    pub async fn feeder_stats(&self, id: FeederId) -> Result<FeederStats, Error> {
        let response = self
            .api(Method::GET, &format!("/feeders/{id}/stats"))
            .send()
            .await?;
        decode_page(response, StatusCode::OK, "sections").await
    }

    /// Follow the feeder's debug log. Owners only.
    pub async fn follow_log(&self, id: FeederId) -> Result<LogBatches, Error> {
        let request = self
            .api_stream(Method::GET, &format!("/feeders/{id}/log"))
            .header(
                header::ACCEPT,
                HeaderValue::from_static("text/event-stream"),
            );

        let response = expect_status(request.send().await?, StatusCode::OK).await?;
        Ok(LogBatches {
            blocks: Blocks::new(response),
        })
    }
}

/// A followed debug log. It ends when the feeder's tail does, at the
/// server's shutdown, or once the session no longer holds.
pub struct LogBatches {
    blocks: Blocks,
}

impl std::fmt::Debug for LogBatches {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogBatches")
            .field("ready", &self.blocks.queued())
            .finish_non_exhaustive()
    }
}

impl LogBatches {
    /// The next message, or `None` once the log has ended. Cancel-safe: a
    /// block read but not yet returned stays queued.
    pub async fn next(&mut self) -> Option<Result<LogEvent, Error>> {
        Some(self.blocks.next().await?.and_then(|block| block.json()))
    }
}
