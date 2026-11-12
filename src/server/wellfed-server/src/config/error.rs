//! Why `server.toml` or a flag was refused. An error never quotes the
//! webhook's URL, a secret, or the file's text around it.

use std::path::PathBuf;

use super::WebhookError;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("read {}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The file is not TOML.
    #[error("{} is not valid TOML", path.display())]
    Syntax {
        path: PathBuf,
        #[source]
        source: Refused,
    },
    /// A key is unknown, or its value is the wrong type or out of range.
    /// `field` is the key's dotted path (`retention.raw_sample_days`,
    /// `hosts[1]`), an unknown key's included; `source` has its line.
    #[error("{}: `{field}` is invalid", path.display())]
    Invalid {
        path: PathBuf,
        field: String,
        #[source]
        source: Refused,
    },
    /// The webhook's URL is no URL, or breaks its rule ([`Webhook::target`](super::Webhook::target)).
    #[error("{}: `webhook.url`", path.display())]
    Webhook {
        path: PathBuf,
        #[source]
        source: WebhookError,
    },
}

/// Where the TOML reader refused the file, and why. It never carries the
/// file's text, which the reader's own error quotes: the webhook's URL is
/// a secret ([`Webhook`](super::Webhook)), and a value under `webhook`, or a line in its
/// table, may be it, so the reason there is left out too.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("line {line}, column {column}: {}", reason.as_deref().unwrap_or(HIDDEN))]
pub struct Refused {
    /// From 1.
    pub line: usize,
    /// From 1, in bytes.
    pub column: usize,
    pub reason: Option<String>,
}

/// What stands for a reason that may quote the webhook's URL.
const HIDDEN: &str = "not shown, since it may quote the webhook's URL";

impl Refused {
    /// `error`, refused in `text`; its reason kept unless `secret`.
    pub(super) fn new(text: &str, error: &toml::de::Error, secret: bool) -> Self {
        let at = error.span().map_or(0, |span| span.start).min(text.len());
        let before = text.get(..at).unwrap_or_default();
        let line_start = before.rfind('\n').map_or(0, |newline| newline + 1);
        Self {
            line: before.matches('\n').count() + 1,
            column: at - line_start + 1,
            reason: (!secret).then(|| error.message().to_owned()),
        }
    }

    /// Whether the line it names lies in the `[webhook]` table or sets a
    /// `webhook` key: a reason there may quote the URL.
    pub(super) fn in_webhook(&self, text: &str) -> bool {
        let lines: Vec<&str> = text.lines().take(self.line).collect();
        let here = lines.last().copied().unwrap_or_default();
        let table = lines
            .iter()
            .rev()
            .map(|line| line.trim_start())
            .find(|line| line.starts_with('['));
        here.contains("webhook") || table.is_some_and(|table| table.contains("webhook"))
    }
}

/// A value refused by its own type, whether it came from the file or a flag.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum InvalidValue {
    #[error("a label is 1-16 bytes with no NUL; {len} bytes were given")]
    Label { len: usize },
    #[error(
        "`{name}` is not a host name (dot-separated labels of 1-63 letters, digits and inner `-`, up to 253 in all)"
    )]
    HostName { name: String },
    #[error("`{text}` is not an update key (64 lowercase hex digits)")]
    UpdateKey { text: String },
}
