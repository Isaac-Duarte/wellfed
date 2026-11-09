//! Every way a call to the server fails.

use std::time::Duration;
use wellfed_api::{CalibrationView, ErrorCode, Plans, Problem, Settings};

use reqwest::{StatusCode, Url};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the server's address must be an http or https URL with a host, not {0}")]
    Base(Url),
    #[error("could not build the HTTP client")]
    Build(#[source] reqwest::Error),
    /// The request failed on the way, or its answer was cut off. A keyed
    /// write may have run: retry it under the same key, never a new one.
    #[error("the request to the server failed")]
    Http(#[from] reqwest::Error),
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
    #[error("the settings moved on to generation {}", .0.generation())]
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

    /// What the failure says of the write the call carried. Only a failure
    /// to connect counts as nothing sent: reqwest's connect errors are the
    /// resolver, the TCP connect and the TLS handshake, all before a byte
    /// of the request is written; a connection that breaks after is a
    /// request error, which may have run. Of the server's answers, only
    /// `feeder_offline` says its feeder task never took the write.
    pub fn failure(&self) -> Failure {
        match self {
            Self::Http(http) if http.is_connect() => Failure::Connect,
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
            Self::Base(_) | Self::Build(_) | Self::Redirect { .. } | Self::NoSession => {
                Failure::Rejected
            }
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
