//! The `[webhook]` settings: where each alert's opening and clearing is
//! POSTed. None by default.

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
    pub fn target(&self) -> Result<Option<WebhookTarget>, WebhookTargetError> {
        let plain = if self.allow_plain_http {
            WebhookPlainHttp::Allowed
        } else {
            WebhookPlainHttp::Refused
        };
        self.url
            .as_ref()
            .map(|url| WebhookTarget::new(url.0.clone(), plain))
            .transpose()
    }
}

/// The webhook flags, each with its environment variable.
#[derive(Debug, Default, clap::Args)]
pub struct WebhookOverrides {
    /// Where each alert's opening and clearing is POSTed as JSON
    /// (`webhook.url`): an https:// URL, or http:// to this computer or
    /// the home network.
    #[arg(long, env = "WELLFED_SERVER_WEBHOOK_URL", value_name = "URL")]
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

/// A URL as written. It never prints whole, since its path may be a
/// secret.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct WebhookUrl(Url);

impl std::fmt::Debug for WebhookUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("WebhookUrl")
            .field(&self.0.host_str().unwrap_or_default())
            .finish()
    }
}

impl TryFrom<String> for WebhookUrl {
    type Error = super::InvalidValue;

    fn try_from(text: String) -> Result<Self, super::InvalidValue> {
        Url::parse(&text)
            .map(Self)
            .map_err(|_| super::InvalidValue::WebhookUrl)
    }
}

impl std::str::FromStr for WebhookUrl {
    type Err = super::InvalidValue;

    fn from_str(text: &str) -> Result<Self, super::InvalidValue> {
        Self::try_from(text.to_owned())
    }
}
