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
    /// The address answered with a redirect, which the phone never follows
    /// (it would carry the password to another host): `message` names
    /// where, for the owner to enter that address instead.
    Redirected,
    /// Nothing answered at the address, or not in time to connect.
    Unreachable,
    /// The request broke after it was sent, its answer was cut off, or the
    /// server went quiet past the phone's timeouts: it may have run.
    Interrupted,
    /// The server answered that it failed inside, or lost the feeder part
    /// way (a `5xx` problem): it may have run.
    ServerFailed,
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
    /// A full bag on a feeder with no food set: set its food first.
    NoCurrentFood,
    /// The feeder is already on this server.
    AlreadyAdopted,
    /// The feeder dropped the invite unanswered: used or expired. A new
    /// adoption makes a new one.
    InviteExpired,
    /// The server could not read the feeder's clients to confirm this
    /// phone's key (the feeder offline, or slow to connect), or took no
    /// proof that the phone holds it (its challenge ran out), so the key is
    /// not this account's yet. Claiming it again records it.
    PhoneKeyUnconfirmed,
    /// The server will not record this phone's key as the account's: the
    /// feeder does not list it, another phone or client holds it, an owner
    /// kept it, only an owner may claim it, or it asks a proof the phone
    /// did not give. `message` says which.
    PhoneKeyRefused,
    /// The server has no session with the feeder right now
    /// (`503 feeder_offline`): its feeder task never took the request, so
    /// nothing ran. A server that lost the feeder part way
    /// (`unreachable`) may have sent it, and reads as [`Self::Interrupted`].
    FeederOffline,
    /// No answer from the feeder in time (`504 outcome_unknown`): the
    /// write may have run. Retried under the same key, or read from the
    /// server's journal; never sent again any other way.
    OutcomeUnknown,
    /// The idempotency key already names another request: nothing was
    /// sent.
    KeyReused,
    /// The server refused for another reason; `message` is its title.
    Refused,
    /// The server answered something this app does not read: another
    /// version, or not a Wellfed server.
    Protocol,
    /// A session's stream already ended with [`close`].
    ///
    /// [`close`]: crate::api::server::ServerSession::close
    Closed,
    /// The address is plain `http://` to a host off the home network, which
    /// would carry the password and the phone's token unencrypted. Nothing
    /// was sent; the server's `https://` address works.
    PlainHttp,
    /// Another adoption or a removal of the feeder is running on the server
    /// (`409 feeder_busy`), such as an adoption whose join answer was lost,
    /// which the server looks for up to twelve minutes: nothing was done.
    /// The server's feeder list shows the feeder once it is found.
    FeederBusy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerError {
    pub kind: ServerErrorKind,
    pub message: String,
    /// The wait the server named in `Retry-After`, as on a rate-limited
    /// login.
    pub retry_after_seconds: Option<u32>,
    /// A change that ended so may have run all the same: its answer was
    /// lost, could not be read, or the server failed partway. The kind
    /// names the words; this, from the server client's one reading of a
    /// failure, says whether a change took, so a screen never resends one
    /// that may have.
    pub may_have_run: bool,
}

impl ServerError {
    /// An error the app built itself, which may have run only when its kind
    /// says the answer was lost.
    pub(crate) fn new(kind: ServerErrorKind, message: impl fmt::Display) -> Self {
        Self {
            may_have_run: matches!(
                kind,
                ServerErrorKind::Interrupted | ServerErrorKind::OutcomeUnknown
            ),
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
