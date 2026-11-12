//! Following `GET /api/v1/stream`, the server's Server-Sent Events.
//!
//! [`Parser`] reads the event-stream format without IO, and [`Updates`]
//! drives it over a response body. Each update carries the stream's last
//! event ID as of that update, as a browser's `EventSource` keeps it: a
//! client that reconnects sends it back as `Last-Event-ID` and gets what it
//! missed, or a `resync` whose empty id clears it.
//!
//! An update this client cannot read, one a newer server added, is
//! skipped: the stream goes on, and [`Updates::last_event_id`] moves past
//! it, as the device API's sessions skip a kind of message they don't know.

use std::collections::VecDeque;

use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use reqwest::header::{self, HeaderValue};
use reqwest::{Method, Response, StatusCode};
use tracing::warn;
use wellfed_api::{StreamEvent, StreamQuery};

use crate::{Client, Error, expect_status};

/// One update from the stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    /// The last event ID: the position to resume after, from this block's
    /// `id` field or the latest before it. Empty when the client must start
    /// afresh, or before the server has named one.
    pub id: String,
    pub event: StreamEvent,
}

impl Client {
    /// Follow the event stream, with the sample metrics `query` names,
    /// resuming after `last_event_id` when given. An empty id, the one a
    /// `resync` leaves, resumes from nothing, as a browser's does: no
    /// `Last-Event-ID` goes, so the server does not answer with another
    /// resync.
    pub async fn stream(
        &self,
        query: &StreamQuery,
        last_event_id: Option<&str>,
    ) -> Result<Updates, Error> {
        let last_event_id = last_event_id.filter(|id| !id.is_empty());
        let mut request = self.api_stream(Method::GET, "/stream").query(query).header(
            header::ACCEPT,
            HeaderValue::from_static("text/event-stream"),
        );
        if let Some(id) = last_event_id {
            request = request.header("Last-Event-ID", id);
        }

        let response = expect_status(request.send().await?, StatusCode::OK).await?;
        Ok(Updates {
            blocks: Blocks::new(response),
            last_event_id: last_event_id.unwrap_or_default().to_owned(),
            skipped: 0,
        })
    }
}

/// An open event stream. It ends when the server ends it: at shutdown, or
/// once the session no longer holds.
pub struct Updates {
    blocks: Blocks,
    last_event_id: String,
    skipped: u64,
}

impl std::fmt::Debug for Updates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Updates")
            .field("ready", &self.blocks.queued())
            .field("last_event_id", &self.last_event_id)
            .field("skipped", &self.skipped)
            .finish_non_exhaustive()
    }
}

impl Updates {
    /// The next update, or `None` once the stream has ended. Cancel-safe:
    /// a block read but not yet returned stays queued. An update this
    /// client cannot read is skipped, logged and counted ([`Self::skipped`]):
    /// one a newer server added, or a decoding regression between this
    /// client and its server, which the log is the only sign of.
    pub async fn next(&mut self) -> Option<Result<Update, Error>> {
        loop {
            let block = match self.blocks.next().await? {
                Ok(block) => block,
                Err(error) => return Some(Err(error)),
            };
            block.id.clone_into(&mut self.last_event_id);
            match block.update() {
                Ok(update) => return Some(Ok(update)),
                Err(error) => {
                    self.skipped += 1;
                    warn!("skipped a stream update this client cannot read: {error}");
                }
            }
        }
    }

    /// The position to resume after: the last update's id, read or
    /// skipped. Send it back as `Last-Event-ID`.
    pub fn last_event_id(&self) -> &str {
        &self.last_event_id
    }

    /// How many updates this stream skipped, unread: ones a newer server
    /// added.
    pub const fn skipped(&self) -> u64 {
        self.skipped
    }
}

/// A response body read as event-stream blocks: the event stream's and the
/// debug log's. Cancel-safe: a block read but not yet taken stays queued.
pub(crate) struct Blocks {
    body: BoxStream<'static, reqwest::Result<Vec<u8>>>,
    parser: Parser,
    ready: VecDeque<Block>,
}

impl Blocks {
    pub(crate) fn new(response: Response) -> Self {
        let body = response
            .bytes_stream()
            .map(|chunk| chunk.map(|bytes| bytes.to_vec()))
            .boxed();
        Self::of(body)
    }

    fn of(body: BoxStream<'static, reqwest::Result<Vec<u8>>>) -> Self {
        Self {
            body,
            parser: Parser::default(),
            ready: VecDeque::new(),
        }
    }

    /// How many blocks are read and not yet taken.
    pub(crate) fn queued(&self) -> usize {
        self.ready.len()
    }

    /// The next block, or `None` once the body has ended. A line or a
    /// block past the parser's bounds ends the body with its error.
    pub(crate) async fn next(&mut self) -> Option<Result<Block, Error>> {
        loop {
            if let Some(block) = self.ready.pop_front() {
                return Some(Ok(block));
            }

            match self.body.next().await? {
                Ok(chunk) => match self.parser.push(&chunk) {
                    Ok(blocks) => self.ready.extend(blocks),
                    Err(error) => {
                        self.body = futures_util::stream::empty().boxed();
                        return Some(Err(error));
                    }
                },
                Err(error) => return Some(Err(error.into())),
            }
        }
    }
}

/// One dispatched block: the last event ID and its joined `data` lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Block {
    id: String,
    data: String,
}

impl Block {
    fn update(self) -> Result<Update, Error> {
        Ok(Update {
            id: self.id,
            event: serde_json::from_str(&self.data)?,
        })
    }

    /// The block's data, read as JSON.
    pub(crate) fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, Error> {
        Ok(serde_json::from_str(&self.data)?)
    }
}

/// The `text/event-stream` format, read incrementally: bytes in, blocks
/// out. A leading byte-order mark is dropped, and lines end in LF or CRLF
/// (the server never sends a bare CR). The last event ID persists across
/// blocks, so a block with only an `id` moves it; comments, `event` and
/// `retry` fields are skipped, and a block whose data is empty dispatches
/// nothing.
#[derive(Debug, Default)]
pub(crate) struct Parser {
    /// Whether the first line has begun, past any byte-order mark.
    started: bool,
    line: Vec<u8>,
    last_id: String,
    data: Option<String>,
}

impl Parser {
    /// Read `bytes`, returning the blocks they complete; a line or a
    /// block's data past [`LINE_MAX`] is refused, so a server (or anyone
    /// on a plain `http` path) that never ends one cannot grow this
    /// client's memory without bound.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<Vec<Block>, Error> {
        let mut blocks = Vec::new();

        for &byte in bytes {
            if byte != b'\n' {
                if self.line.len() >= LINE_MAX {
                    return Err(too_long("line"));
                }
                self.line.push(byte);
                continue;
            }
            let mut line = std::mem::take(&mut self.line);
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if !self.started {
                self.started = true;
                if line.starts_with(BOM) {
                    line.drain(..BOM.len());
                }
            }
            if let Some(block) = self.line(&String::from_utf8_lossy(&line))? {
                blocks.push(block);
            }
        }

        Ok(blocks)
    }

    fn line(&mut self, line: &str) -> Result<Option<Block>, Error> {
        if line.is_empty() {
            let Some(data) = self.data.take().filter(|data| !data.is_empty()) else {
                return Ok(None);
            };
            return Ok(Some(Block {
                id: self.last_id.clone(),
                data,
            }));
        }

        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "data" => match &mut self.data {
                Some(data) if data.len() + 1 + value.len() > LINE_MAX => {
                    return Err(too_long("block"));
                }
                Some(data) => {
                    data.push('\n');
                    data.push_str(value);
                }
                None => self.data = Some(value.to_owned()),
            },
            "id" if !value.contains('\0') => value.clone_into(&mut self.last_id),
            // A comment (empty field), `event`, `retry` or unknown.
            _ => {}
        }
        Ok(None)
    }
}

/// The longest line, and the longest block's data, the parser holds:
/// far past the largest the server sends (a batch of log lines, or an
/// event), and a bound on what a stream that never ends one can take.
const LINE_MAX: usize = 1 << 20;

/// A stream past [`LINE_MAX`]: its answer does not decode, and the
/// stream ends.
fn too_long(what: &str) -> Error {
    let error = <serde_json::Error as serde::de::Error>::custom(format!(
        "an event-stream {what} past {LINE_MAX} bytes"
    ));
    Error::Decode(error)
}

/// UTF-8's byte-order mark, which may open a stream.
const BOM: &[u8] = b"\xef\xbb\xbf";

#[cfg(test)]
mod tests {
    use futures_util::FutureExt;
    use futures_util::stream;

    use super::*;

    fn block(id: &str, data: &str) -> Block {
        Block {
            id: id.into(),
            data: data.into(),
        }
    }

    #[test]
    fn blocks_dispatch_on_a_blank_line_with_their_fields_in_any_order() {
        let mut parser = Parser::default();
        let text = "\u{feff}data: {\"type\":\"resync\"}\nid:\n\n: keep-alive\n\nid: 7-1\r\ndata:a\r\ndata: b\r\n\r\n";
        assert_eq!(
            parser.push(text.as_bytes()).unwrap(),
            [block("", "{\"type\":\"resync\"}"), block("7-1", "a\nb"),]
        );
    }

    #[test]
    fn a_block_split_across_chunks_waits_for_its_end() {
        let mut parser = Parser::default();
        assert_eq!(parser.push(b"id: 1-2\nda").unwrap(), []);
        assert_eq!(parser.push(b"ta: x\n").unwrap(), []);
        assert_eq!(parser.push(b"\n").unwrap(), [block("1-2", "x")]);
    }

    #[test]
    fn the_last_event_id_persists_and_empty_data_dispatches_nothing() {
        let mut parser = Parser::default();
        assert_eq!(
            parser.push(b"data: y\n\n").unwrap(),
            [block("", "y")],
            "none yet"
        );
        assert_eq!(parser.push(b"id: 3\n\n").unwrap(), [], "no data, no block");
        assert_eq!(
            parser.push(b"data:\n\n").unwrap(),
            [],
            "empty data, no block"
        );
        assert_eq!(parser.push(b"data: z\n\n").unwrap(), [block("3", "z")]);
        assert_eq!(parser.push(b"id\ndata: w\n\n").unwrap(), [block("", "w")]);
    }

    #[test]
    fn an_update_this_client_cannot_read_is_skipped_and_the_position_moves() {
        let text = "id: 1-2\ndata: {\"type\":\"newer_kind\"}\n\nid: 1-3\ndata: {\"type\":\"resync\"}\n\nid: 1-4\ndata: {\"type\":\"newer_kind\"}\n\n";
        let mut updates = Updates {
            blocks: Blocks::of(stream::iter([Ok(text.as_bytes().to_vec())]).boxed()),
            last_event_id: "1-1".into(),
            skipped: 0,
        };

        assert_eq!(
            updates.next().now_or_never().flatten().map(Result::ok),
            Some(Some(Update {
                id: "1-3".into(),
                event: StreamEvent::Resync,
            }))
        );
        assert!(updates.next().now_or_never().flatten().is_none(), "ended");
        assert_eq!(updates.last_event_id(), "1-4", "past the last, unread");
        assert_eq!(updates.skipped(), 2);
    }

    /// A line or a block that never ends is refused at the bound, rather
    /// than held as it grows.
    #[test]
    fn a_line_or_a_block_past_the_bound_is_refused() {
        let mut parser = Parser::default();
        let endless = vec![b'x'; LINE_MAX + 1];
        assert!(matches!(parser.push(&endless), Err(Error::Decode(_))));

        let mut parser = Parser::default();
        let line = format!("data: {}\n", "y".repeat(LINE_MAX / 2));
        assert_eq!(parser.push(line.as_bytes()).unwrap(), []);
        assert!(matches!(
            parser.push(line.as_bytes()),
            Err(Error::Decode(_))
        ));
    }

    /// A body past the bound ends the stream with the error.
    #[test]
    fn a_body_past_the_bound_ends_with_its_error() {
        let endless = vec![b'x'; LINE_MAX + 1];
        let mut blocks = Blocks::of(stream::iter([Ok(endless), Ok(b"\n\n".to_vec())]).boxed());
        assert!(matches!(
            blocks.next().now_or_never().flatten(),
            Some(Err(Error::Decode(_)))
        ));
        assert!(blocks.next().now_or_never().flatten().is_none());
    }

    #[test]
    fn a_block_decodes_to_its_update() {
        let resync = block("", "{\"type\":\"resync\"}").update();
        assert_eq!(
            resync.ok(),
            Some(Update {
                id: String::new(),
                event: StreamEvent::Resync,
            })
        );
        assert!(matches!(
            block("", "nonsense").update(),
            Err(Error::Decode(_))
        ));
    }
}
