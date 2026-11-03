//! The one error a call to the owner's server gives Dart: a closed
//! [`ServerErrorKind`] for each failure a screen tells apart, and the
//! underlying message for logs. Separate from the feeder's
//! [`BridgeError`](crate::api::error::BridgeError), since a server answers in
//! HTTP problems, not device API refusals.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerErrorKind {
    /// The server's address is not an `http` or `https` URL with a host.
    Address,
    /// Nothing answered at the address, or not in time to connect.
    Unreachable,
    /// The request broke after it was sent, or its answer was cut off: it
    /// may have run on the server.
    Interrupted,
    /// The name or the password is wrong.
    BadCredentials,
    /// Too many failed logins: `retry_after_seconds` says when the server
    /// takes the next.
    RateLimited,
    /// The phone's token no longer holds: revoked, expired, or its account
    /// deleted. Logging in again mints a new one.
    TokenRevoked,
    /// The account is past its expiry, until an owner extends it.
    AccountDisabled,
    /// This account's role may not do that.
    Forbidden,
    /// No such feeder, pet or food on this server.
    NotFound,
    /// A field failed the server's checks: `message` says which.
    Invalid,
    /// A feeder's food history names the food, so it stays.
    FoodInUse,
    /// The server has no session with the feeder right now, or could not
    /// reach it.
    FeederOffline,
    /// The server refused for another reason; `message` is its title.
    Refused,
    /// The server answered something this app does not read: another
    /// version, or not a Wellfed server.
    Protocol,
    /// A session's stream already ended with [`close`].
    ///
    /// [`close`]: crate::api::server::ServerSession::close
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerError {
    pub kind: ServerErrorKind,
    pub message: String,
    /// The wait the server named in `Retry-After`, as on a rate-limited
    /// login.
    pub retry_after_seconds: Option<u32>,
}

impl ServerError {
    pub(crate) fn new(kind: ServerErrorKind, message: impl fmt::Display) -> Self {
        Self {
            kind,
            message: message.to_string(),
            retry_after_seconds: None,
        }
    }
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for ServerError {}
