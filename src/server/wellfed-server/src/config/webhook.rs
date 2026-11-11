//! The `[webhook]` settings: where each alert's opening and clearing is
//! POSTed. None by default.
//!
//! The URL is often a secret of its own (Home Assistant's is), so it is
//! kept as written and parsed only once the settings are read: a TOML or a
//! flag parser's error quotes the value it refused, and this one must never
//! reach a log or a terminal whole.

use serde::Deserialize;
use wellfed_service::{Url, WebhookPlainHttp, WebhookTarget, WebhookTargetError};

/// The webhook as set, before its URL's rule is checked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Webhook {
    pub url: Option<WebhookUrl>,
    /// Whether a plain `http://` URL off this computer and the home network
    /// is taken.
    pub allow_plain_http: bool,
}

impl Webhook {
    pub(super) fn merge(file: WebhookFile, overrides: WebhookOverrides) -> Self {
        Self {
            url: overrides.webhook_url.or(file.url),
            allow_plain_http: overrides
                .webhook_allow_plain_http
                .or(file.allow_plain_http)
                .unwrap_or(false),
        }
    }

    /// The checked target, or `None` with no URL set.
    pub fn target(&self) -> Result<Option<WebhookTarget>, WebhookError> {
        let plain = if self.allow_plain_http {
            WebhookPlainHttp::Allowed
        } else {
            WebhookPlainHttp::Refused
        };
        let Some(url) = &self.url else {
            return Ok(None);
        };
        let url = Url::parse(&url.0).map_err(|_| WebhookError::NotUrl)?;
        Ok(Some(WebhookTarget::new(url, plain)?))
    }
}

/// Why the webhook's URL was refused. Neither names the URL.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum WebhookError {
    #[error("the webhook URL is not a URL")]
    NotUrl,
    #[error(transparent)]
    Rule(#[from] WebhookTargetError),
}

/// The webhook flags, each with its environment variable.
#[derive(Debug, Default, clap::Args)]
pub struct WebhookOverrides {
    /// Where each alert's opening and clearing is POSTed as JSON
    /// (`webhook.url`): an https:// URL, or http:// to this computer or
    /// the home network.
    #[arg(
        long,
        env = "WELLFED_SERVER_WEBHOOK_URL",
        hide_env_values = true,
        value_name = "URL"
    )]
    pub webhook_url: Option<WebhookUrl>,
    /// Take a plain http:// webhook URL off the home network
    /// (`webhook.allow_plain_http`; default false).
    #[arg(
        long,
        env = "WELLFED_SERVER_WEBHOOK_ALLOW_PLAIN_HTTP",
        value_name = "BOOL"
    )]
    pub webhook_allow_plain_http: Option<bool>,
}

/// `[webhook]` as written.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WebhookFile {
    url: Option<WebhookUrl>,
    allow_plain_http: Option<bool>,
}

/// A URL as written, checked by [`Webhook::target`]. It never prints,
/// since its path may be a secret.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct WebhookUrl(String);

impl std::fmt::Debug for WebhookUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WebhookUrl(..)")
    }
}

impl std::str::FromStr for WebhookUrl {
    type Err = std::convert::Infallible;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ok(Self(text.to_owned()))
    }
}
