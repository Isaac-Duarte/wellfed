//! A typed client of the owner's server HTTP API ([API design]), over
//! `wellfed-api`'s bodies: the CLI's `wellfed server` commands and the
//! server's own tests go through it, so neither can disagree with the
//! routes.
//!
//! A [`Client`] holds the server's base URL and how it authenticates
//! ([`Auth`]): the session cookie a login sets, or a bearer token. Each
//! typed method sends one request and accepts only its route's success
//! status; anything else comes back as an [`Error`], a refusal as the
//! server's typed [`Problem`]. [`Client::stream`] follows the server's
//! event stream.
//!
//! Writes carry the caller's `Idempotency-Key`, and nothing here retries:
//! a write that ends in [`Error::Http`] or `outcome_unknown` may have run,
//! so the caller retries it under the same key or not at all.
//!
//! Nothing follows a redirect either: following one would send a login's
//! password, or a write's body, to whatever host the answer names. A
//! redirect is [`Error::Redirect`], and the caller names the final URL.
//! Nor does a client speak plain `http://` beyond this computer and the
//! home network unless its caller asked the owner ([`PlainHttp`]): a login
//! carries the password and every call a token that can feed.
//! How long a client waits on a server that goes quiet is its
//! [`Timeouts`]; by default only the connection's opening is bounded.
//!
//! [`Client::request`] is the escape hatch for a request the typed methods
//! cannot make (a malformed one, in a test), with the same base, origin
//! and credentials.
//!
//! [API design]: ../../../../docs/server/README.md#the-http-api
#![forbid(unsafe_code)]

mod audit;
mod auth;
mod backups;
mod call;
mod clients;
mod commands;
mod diagnostics;
mod error;
mod export;
mod feeders;
mod feeds;
mod foods;
mod health;
mod home;
mod pets;
mod releases;
mod stock;
mod stream;
mod tokens;
mod users;

use std::fmt;
use std::time::Duration;

use reqwest::header::{self, HeaderValue};
use reqwest::{RequestBuilder, Response};
use serde::de::DeserializeOwned;
use wellfed_api::{PREFIX, Problem, SESSION_COOKIE, at_home};

pub use commands::{Settled, settled};
pub use diagnostics::LogBatches;
pub use error::{Error, Failure};
pub use feeders::Lock;
pub use releases::Uploaded;
pub use reqwest::{self, Method, StatusCode, Url};
pub use stock::Kept;
pub use stream::{Update, Updates};
/// The API's bodies, for a caller that may not depend on `wellfed-api`
/// itself: the CLI reaches the server through this crate alone.
pub use wellfed_api as api;
pub use wellfed_api::PlainHttp;

/// How long a connection to the server may take to open. A server on the
/// LAN answers in milliseconds; a request's own time is the caller's to
/// bound ([`Timeouts`]), since an event stream stays open for good.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a client waits on a server that stops answering. `None` waits
/// for good, as a CLI watching an export or a long stream wants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timeouts {
    /// The longest silence while reading any answer, streams included: a
    /// half-open connection ends after it instead of never. The server's
    /// event streams send a keep-alive every 15 s.
    pub read: Option<Duration>,
    /// The longest a whole request may take, from sending to the end of
    /// its answer. Event streams are exempt, since they stay open.
    pub request: Option<Duration>,
}

/// How a client proves who it is.
#[derive(Clone, PartialEq, Eq)]
pub enum Auth {
    /// The value of the session cookie a login set.
    Session(String),
    /// A bearer token.
    Token(String),
}

impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Either one is a secret.
        match self {
            Self::Session(_) => f.write_str("Session(..)"),
            Self::Token(_) => f.write_str("Token(..)"),
        }
    }
}

/// A client of one server. Cloning it is cheap and shares the connection
/// pool; each clone keeps its own credentials.
#[derive(Clone, Debug)]
pub struct Client {
    http: reqwest::Client,
    /// The base URL, without a trailing slash.
    base: String,
    /// The base URL's origin, sent with every request.
    origin: HeaderValue,
    auth: Option<Auth>,
    /// Bounds every request but the event streams.
    request_timeout: Option<Duration>,
}

impl Client {
    /// A client of the server at `base`, an `https` URL, or an `http` one
    /// on this computer or the home network such as
    /// `http://192.168.1.20:8080` (a path is kept as a prefix, for a server
    /// behind a reverse proxy), with no credentials yet and no
    /// [`Timeouts`].
    pub fn new(base: &Url) -> Result<Self, Error> {
        Self::with_timeouts(base, Timeouts::default(), PlainHttp::Refused)
    }

    /// [`Self::new`], waiting on a quiet server only as long as `timeouts`
    /// say, and taking plain `http://` anywhere if `plain` is
    /// [`PlainHttp::Allowed`]: only a caller that asked the owner passes
    /// that.
    pub fn with_timeouts(base: &Url, timeouts: Timeouts, plain: PlainHttp) -> Result<Self, Error> {
        let http_url = matches!(base.scheme(), "http" | "https") && base.host().is_some();
        if !http_url || base.query().is_some() || base.fragment().is_some() {
            return Err(Error::Base(base.clone()));
        }
        let host = base.host_str().unwrap_or_default();
        if base.scheme() == "http" && plain == PlainHttp::Refused && !at_home(host) {
            return Err(Error::PlainHttp {
                host: host.to_owned(),
            });
        }
        let origin = HeaderValue::from_str(&base.origin().ascii_serialization())
            .map_err(|_| Error::Base(base.clone()))?;

        let mut http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none());
        if let Some(read) = timeouts.read {
            http = http.read_timeout(read);
        }
        let http = http.build().map_err(Error::Build)?;

        Ok(Self {
            http,
            base: base.as_str().trim_end_matches('/').to_owned(),
            origin,
            auth: None,
            request_timeout: timeouts.request,
        })
    }

    /// This client with `auth` in place of its credentials.
    #[must_use]
    pub fn with_auth(mut self, auth: Option<Auth>) -> Self {
        self.auth = auth;
        self
    }

    pub fn auth(&self) -> Option<&Auth> {
        self.auth.as_ref()
    }

    /// The URL of `path`, from the server's root (`/health`, `/api/v1/…`).
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// The underlying HTTP client, for a request that must not carry this
    /// client's origin or credentials.
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// A request for `path` from the server's root, carrying the origin
    /// and the credentials every typed method sends.
    ///
    /// The server refuses a cookie-authenticated write whose `Origin` is
    /// not its own, to stop other sites' pages; this client is not a
    /// page, so it names the server's origin as the server's own pages do.
    /// A token-authenticated write needs no `Origin`, and carries it all
    /// the same.
    pub fn request(&self, method: Method, path: &str) -> RequestBuilder {
        let request = self.open_request(method, path);
        match self.request_timeout {
            Some(timeout) => request.timeout(timeout),
            None => request,
        }
    }

    /// [`Self::request`] with no whole-request timeout, for an event stream
    /// that stays open: only the read timeout ends a quiet one.
    fn open_request(&self, method: Method, path: &str) -> RequestBuilder {
        let request = self
            .http
            .request(method, self.url(path))
            .header(header::ORIGIN, self.origin.clone());

        match &self.auth {
            Some(Auth::Session(value)) => {
                request.header(header::COOKIE, format!("{SESSION_COOKIE}={value}"))
            }
            Some(Auth::Token(token)) => request.bearer_auth(token),
            None => request,
        }
    }

    /// A request for `path` under the API's prefix.
    fn api(&self, method: Method, path: &str) -> RequestBuilder {
        self.request(method, &format!("{PREFIX}{path}"))
    }

    /// An event stream under the API's prefix.
    fn api_stream(&self, method: Method, path: &str) -> RequestBuilder {
        self.open_request(method, &format!("{PREFIX}{path}"))
    }
}

/// `response` if its status is `expected`. Any other status is the
/// server's [`Error::Problem`] when it sent one, a redirect
/// [`Error::Redirect`], else [`Error::Unexpected`].
pub async fn expect_status(response: Response, expected: StatusCode) -> Result<Response, Error> {
    let status = response.status();
    if status == expected {
        return Ok(response);
    }
    if status.is_redirection() {
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        return Err(Error::Redirect { status, location });
    }

    if is_problem(&response) {
        let retry_after = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|seconds| seconds.trim().parse().ok())
            .map(Duration::from_secs);
        let problem: Problem = serde_json::from_slice(&response.bytes().await?)?;
        return Err(Error::Problem {
            status,
            problem: Box::new(problem),
            retry_after,
        });
    }
    Err(Error::Unexpected { status, expected })
}

/// The JSON body of `response`, if its status is `expected`.
pub async fn decode<T: DeserializeOwned>(
    response: Response,
    expected: StatusCode,
) -> Result<T, Error> {
    let response = expect_status(response, expected).await?;
    Ok(serde_json::from_slice(&response.bytes().await?)?)
}

/// The JSON page `response` carries, if its status is `expected`, with
/// each entry of its `list` field decoded on its own: an entry this client
/// cannot read, one a newer server added (a new kind of event, say), is
/// left out rather than failing the page. A page's `next` is the server's,
/// so paging still moves past it.
pub(crate) async fn decode_page<T: DeserializeOwned>(
    response: Response,
    expected: StatusCode,
    list: &str,
) -> Result<T, Error> {
    let response = expect_status(response, expected).await?;
    let page: serde_json::Value = serde_json::from_slice(&response.bytes().await?)?;
    lenient_page(page, list)
}

/// `page` decoded with only the entries of its `list` field that decode.
fn lenient_page<T: DeserializeOwned>(page: serde_json::Value, list: &str) -> Result<T, Error> {
    Ok(lenient_counted(page, list)?.0)
}

/// [`decode_page`], with how many entries were left out.
pub(crate) async fn decode_page_counted<T: DeserializeOwned>(
    response: Response,
    expected: StatusCode,
    list: &str,
) -> Result<(T, usize), Error> {
    let response = expect_status(response, expected).await?;
    let page: serde_json::Value = serde_json::from_slice(&response.bytes().await?)?;
    lenient_counted(page, list)
}

/// [`lenient_page`], with how many entries were left out.
fn lenient_counted<T: DeserializeOwned>(
    mut page: serde_json::Value,
    list: &str,
) -> Result<(T, usize), Error> {
    let Some(slot) = page.get_mut(list) else {
        // No list to pick from: the page fails or decodes as it is.
        return Ok((serde_json::from_value(page)?, 0));
    };
    let entries = match slot.take() {
        serde_json::Value::Array(entries) => entries,
        // Not a list: the page decodes, or fails, as it is.
        other => {
            *slot = other;
            return Ok((serde_json::from_value(page)?, 0));
        }
    };

    // An entry is readable when the page holding it alone decodes. The
    // page is otherwise small (the list emptied), so each try is cheap.
    let all = entries.len();
    let readable: Vec<serde_json::Value> = entries
        .into_iter()
        .filter(|entry| {
            let mut alone = page.clone();
            alone[list] = serde_json::Value::Array(vec![entry.clone()]);
            serde_json::from_value::<T>(alone).is_ok()
        })
        .collect();

    let skipped = all - readable.len();
    page[list] = serde_json::Value::Array(readable);
    Ok((serde_json::from_value(page)?, skipped))
}

/// Whether `response` carries an RFC 9457 problem, whatever parameters
/// (`; charset=utf-8`) its media type has.
fn is_problem(response: &Response) -> bool {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|kind| kind.to_str().ok())
        .is_some_and(is_problem_type)
}

fn is_problem_type(content_type: &str) -> bool {
    let media = content_type.split(';').next().unwrap_or_default();
    media
        .trim()
        .eq_ignore_ascii_case("application/problem+json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(base: &str) -> Result<Client, Error> {
        Client::new(&Url::parse(base).expect("a URL"))
    }

    #[test]
    fn the_base_is_an_http_url_and_its_origin_goes_with_every_request() {
        let client = open("http://192.168.1.20:8080/").expect("an http base");
        assert_eq!(client.url("/health"), "http://192.168.1.20:8080/health");

        let request = client
            .with_auth(Some(Auth::Session("abc".into())))
            .api(Method::POST, "/setup")
            .build()
            .expect("a request");
        assert_eq!(
            request.url().as_str(),
            "http://192.168.1.20:8080/api/v1/setup"
        );
        assert_eq!(
            request.headers()[header::ORIGIN],
            "http://192.168.1.20:8080"
        );
        assert_eq!(request.headers()[header::COOKIE], "wellfed_session=abc");

        for base in [
            "ftp://192.0.2.7",
            "unix:/run/server.sock",
            "http://192.0.2.7/?a=1",
            "http://192.0.2.7/#top",
        ] {
            assert!(matches!(open(base), Err(Error::Base(_))), "{base}");
        }
    }

    #[test]
    fn plain_http_stays_home_unless_the_caller_asked_the_owner() {
        for base in [
            "http://192.168.1.20:8080",
            "http://127.0.0.1:8080",
            "http://[fd00::4]:8080",
            "http://wellfed.local:8080",
            "https://feeder.example",
            "https://203.0.113.4/wellfed",
        ] {
            assert!(open(base).is_ok(), "{base}");
        }

        for base in [
            "http://feeder.example",
            "http://203.0.113.4:8080",
            "http://nas:8080",
        ] {
            assert!(matches!(open(base), Err(Error::PlainHttp { .. })), "{base}");
            let allowed = Client::with_timeouts(
                &Url::parse(base).expect("a URL"),
                Timeouts::default(),
                PlainHttp::Allowed,
            );
            assert!(allowed.is_ok(), "{base}");
        }
    }

    #[test]
    fn a_problem_is_its_media_type_with_any_parameters() {
        for kind in [
            "application/problem+json",
            "application/problem+json; charset=utf-8",
            "Application/Problem+JSON ;charset=UTF-8",
        ] {
            assert!(is_problem_type(kind), "{kind}");
        }
        for kind in ["application/json", "application/problem+jsonx", ""] {
            assert!(!is_problem_type(kind), "{kind}");
        }
    }

    #[test]
    fn a_page_leaves_out_the_entries_this_client_cannot_read() {
        let event = |seq: u64, kind: &str| {
            serde_json::json!({
                "epoch": 1,
                "seq": seq,
                "at": null,
                "received": "2026-11-08T00:00:00Z",
                "body": { "kind": kind },
            })
        };
        let page = serde_json::json!({
            "epoch": 1,
            "events": [event(1, "restarting"), event(2, "newer_kind"), event(3, "update_confirmed")],
            "gaps": [],
            "next": 3,
        });

        let read: wellfed_api::EventPage = lenient_page(page, "events").expect("the page reads");
        assert_eq!(
            read.events
                .iter()
                .map(|event| event.seq)
                .collect::<Vec<_>>(),
            [1, 3]
        );
        assert_eq!(read.next, Some(3), "paging moves past the entry left out");

        let broken =
            serde_json::json!({ "epoch": 1, "events": "no list", "gaps": [], "next": null });
        assert!(lenient_page::<wellfed_api::EventPage>(broken, "events").is_err());
    }

    #[test]
    fn an_alert_this_client_cannot_read_is_left_out_and_counted() {
        let alerts = serde_json::json!({
            "read": null,
            "alerts": [
                { "at": null, "alert": { "kind": "food_low" } },
                { "at": null, "alert": { "kind": "a_newer_kind" } },
                { "at": null, "alert": { "kind": "no_clock" } },
            ],
            "unreachable": null,
        });

        let (read, skipped): (wellfed_api::FeederAlerts, usize) =
            lenient_counted(alerts, "alerts").expect("the alerts read");
        assert_eq!(skipped, 1);
        assert_eq!(read.alerts.len(), 2);
    }

    #[test]
    fn a_token_is_a_bearer_and_neither_secret_is_printed() {
        let client = open("https://feeder.example")
            .expect("an https base")
            .with_auth(Some(Auth::Token("secret".into())));
        let request = client
            .api(Method::GET, "/feeders")
            .build()
            .expect("a request");
        assert_eq!(request.headers()[header::AUTHORIZATION], "Bearer secret");
        assert!(request.headers().get(header::COOKIE).is_none());
        assert!(!format!("{client:?}").contains("secret"));
    }
}
