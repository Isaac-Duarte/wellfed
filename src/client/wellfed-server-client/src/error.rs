//! Every way a call to the server fails.

use std::time::Duration;
use wellfed_api::{CalibrationView, ErrorCode, Plans, Problem, Settings};

use reqwest::{StatusCode, Url};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the server's address must be an http or https URL with a host, not {0}")]
    Base(Url),
    /// Plain `http://` to a host off this computer and the home network,
    /// which would carry the password and a token that can feed across the
    /// network in the clear. Nothing was sent.
    #[error(
        "plain http:// to {host} is off this computer and the home network, so the password \
         and the token would cross it unencrypted: use the server's https:// address"
    )]
    PlainHttp { host: String },
    #[error("could not build the HTTP client")]
    Build(#[source] reqwest::Error),
    #[error("could not set up the client's TLS")]
    Tls(#[source] rustls::Error),
    /// The server proved another identity key than the pinned one: another
    /// server, or this one reset with a new key. Nothing was sent.
    #[error(
        "the server's identity key is not the one this client pinned{}",
        offered.map(|key| format!(": it offers {}", wellfed_api::fingerprint(&key))).unwrap_or_default()
    )]
    KeyChanged { offered: Option<[u8; 32]> },
    /// The server proved no identity key in its handshake: not a Wellfed
    /// server's own HTTPS. Nothing was sent.
    #[error("the server proves no identity key: is this its own https:// address?")]
    NoIdentity,
    /// The request failed on the way, or its answer was cut off. A keyed
    /// write may have run: retry it under the same key, never a new one.
    #[error("the request to the server failed")]
    Http(#[source] reqwest::Error),
    /// The server refused, with its typed reason. `retry_after` is the
    /// wait its `Retry-After` named, as on `429 login_limited`.
    #[error("the server answered {status}: {}", problem.title)]
    Problem {
        status: StatusCode,
        problem: Box<Problem>,
        retry_after: Option<Duration>,
    },
    /// An edit named a generation the settings have moved past; these are
    /// the current settings.
    #[error("the settings moved on to generation {}", .0.generation().map_or(0, |generation| generation.0))]
    Stale(Box<Settings>),
    /// A plans change named a generation the plans have moved past; these
    /// are the current plans.
    #[error("the plans moved on to generation {}", .0.generation)]
    StalePlans(Box<Plans>),
    /// A calibration change named a generation the calibration has moved
    /// past; this is the current calibration.
    #[error("the calibration moved on to generation {}", .0.generation)]
    StaleCalibration(Box<CalibrationView>),
    /// The address answered with a redirect, which is never followed: it
    /// would carry a password or a write's body to another host. The
    /// caller is to use the final URL (`location`, when it names one).
    #[error("the server redirects to {}: use that address instead", location.as_deref().unwrap_or("another address"))]
    Redirect {
        status: StatusCode,
        location: Option<String>,
    },
    #[error("the server answered {status} where {expected} was expected")]
    Unexpected {
        status: StatusCode,
        expected: StatusCode,
    },
    #[error("the server's answer does not decode")]
    Decode(#[from] serde_json::Error),
    #[error("the server's login answer set no session cookie")]
    NoSession,
}

impl From<reqwest::Error> for Error {
    /// A handshake refused for its key is its own error, whatever reqwest
    /// wrapped it in.
    fn from(error: reqwest::Error) -> Self {
        if let Some(mismatch) = crate::trust::carried::<crate::trust::KeyMismatch>(&error) {
            return Self::KeyChanged {
                offered: mismatch.offered,
            };
        }
        if crate::trust::carried::<crate::trust::NoIdentity>(&error).is_some() {
            return Self::NoIdentity;
        }
        Self::Http(error)
    }
}

impl Error {
    /// How long the server asked to wait before trying again.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Problem { retry_after, .. } => *retry_after,
            _ => None,
        }
    }

    /// The problem's code, when the server refused with one.
    pub fn code(&self) -> Option<ErrorCode> {
        match self {
            Self::Problem { problem, .. } => Some(problem.code),
            _ => None,
        }
    }

    /// What the failure says of the write the call carried, the one
    /// reading every client of the server shares. Of reqwest's errors, only
    /// a failure to connect or to build the request counts as nothing
    /// sent: its connect errors are the resolver, the TCP connect and the
    /// TLS handshake, all before a byte of the request is written; a
    /// connection that breaks after is a request error, which may have run. Of the server's answers, only
    /// `feeder_offline` says its feeder task never took the write.
    pub fn failure(&self) -> Failure {
        match self {
            Self::Http(http) if http.is_connect() => Failure::Connect,
            // A request reqwest could not build never left this client.
            Self::Http(http) if http.is_builder() => Failure::Rejected,
            Self::Http(_) | Self::Decode(_) | Self::Unexpected { .. } => Failure::Interrupted,
            Self::Problem {
                status, problem, ..
            } => match problem.code {
                ErrorCode::OutcomeUnknown => Failure::OutcomeUnknown,
                ErrorCode::FeederOffline => Failure::FeederOffline,
                ErrorCode::FeederRefused => Failure::FeederRefused,
                ErrorCode::KeyReused => Failure::KeyReused,
                ErrorCode::Unauthenticated | ErrorCode::AccountDisabled => Failure::Unauthorized,
                // The server failed inside, or lost its feeder part way
                // (`unreachable`): either may be after it sent. A code a
                // newer server added reads by its status the same way.
                _ if status.is_server_error() => Failure::Interrupted,
                _ => Failure::Rejected,
            },
            Self::Stale(_) | Self::StalePlans(_) | Self::StaleCalibration(_) => Failure::Stale,
            // Refused in the handshake, before the request was written.
            Self::KeyChanged { .. } | Self::NoIdentity => Failure::Connect,
            Self::Base(_)
            | Self::PlainHttp { .. }
            | Self::Build(_)
            | Self::Tls(_)
            | Self::Redirect { .. }
            | Self::NoSession => Failure::Rejected,
        }
    }
}

/// How one call to the server failed, as a client deciding what became of
/// the write it carried tells them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// No connection to the server: nothing left this client.
    Connect,
    /// Sent, and the answer was cut off, timed out or could not be read,
    /// or the server failed inside: it may have run.
    Interrupted,
    /// `504 outcome_unknown`: the journal keeps it unknown.
    OutcomeUnknown,
    /// `503 feeder_offline`: the server's feeder task never took it, and a
    /// first attempt's entry is dropped. Nothing ran.
    FeederOffline,
    /// The feeder refused it (`409 feeder_refused`): final for the key.
    FeederRefused,
    /// A settings, plans or calibration write whose base the store moved
    /// past: the feeder refused it, and nothing ran.
    Stale,
    /// The key already names another request: nothing was sent.
    KeyReused,
    /// The client's token or session no longer holds (`401`), or its
    /// account is disabled: nothing was sent, and the server is closed to
    /// this client.
    Unauthorized,
    /// The server refused before sending anything: forbidden, invalid, a
    /// redirect.
    Rejected,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn problem(status: StatusCode, code: ErrorCode) -> Error {
        Error::Problem {
            status,
            problem: Box::new(Problem {
                kind: "about:blank".into(),
                title: "t".into(),
                status: status.as_u16(),
                code,
                detail: None,
                refusal: None,
                release: None,
            }),
            retry_after: None,
        }
    }

    /// Every client decides a write's fate from this table, so each row is
    /// pinned: only a refusal the server answered with a code that says so,
    /// or a request that never left, is sure not to have run.
    #[test]
    fn a_failure_reads_one_way_for_every_client() {
        let rows = [
            (
                StatusCode::GATEWAY_TIMEOUT,
                ErrorCode::OutcomeUnknown,
                Failure::OutcomeUnknown,
            ),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::FeederOffline,
                Failure::FeederOffline,
            ),
            (
                StatusCode::CONFLICT,
                ErrorCode::FeederRefused,
                Failure::FeederRefused,
            ),
            (
                StatusCode::CONFLICT,
                ErrorCode::KeyReused,
                Failure::KeyReused,
            ),
            (
                StatusCode::UNAUTHORIZED,
                ErrorCode::Unauthenticated,
                Failure::Unauthorized,
            ),
            (
                StatusCode::FORBIDDEN,
                ErrorCode::AccountDisabled,
                Failure::Unauthorized,
            ),
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                Failure::Interrupted,
            ),
            (
                StatusCode::BAD_GATEWAY,
                ErrorCode::Unreachable,
                Failure::Interrupted,
            ),
            // A code a newer server added reads by its status.
            (
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::Unknown,
                Failure::Interrupted,
            ),
            (
                StatusCode::BAD_REQUEST,
                ErrorCode::Unknown,
                Failure::Rejected,
            ),
            (
                StatusCode::FORBIDDEN,
                ErrorCode::Forbidden,
                Failure::Rejected,
            ),
            (
                StatusCode::BAD_REQUEST,
                ErrorCode::Invalid,
                Failure::Rejected,
            ),
        ];
        for (status, code, failure) in rows {
            assert_eq!(
                problem(status, code).failure(),
                failure,
                "{status} {code:?}"
            );
        }

        let unexpected = Error::Unexpected {
            status: StatusCode::BAD_GATEWAY,
            expected: StatusCode::ACCEPTED,
        };
        assert_eq!(unexpected.failure(), Failure::Interrupted);
        let garbled = serde_json::from_str::<u8>("x").unwrap_err();
        assert_eq!(Error::Decode(garbled).failure(), Failure::Interrupted);
        assert_eq!(Error::NoSession.failure(), Failure::Rejected);

        let unbuilt = reqwest::Client::new()
            .get("http://[::1")
            .build()
            .unwrap_err();
        assert!(unbuilt.is_builder(), "{unbuilt}");
        assert_eq!(Error::Http(unbuilt).failure(), Failure::Rejected);
    }
}
