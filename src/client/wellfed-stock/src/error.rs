//! What the stock server's library refuses or fails on. Each refusal is a
//! variant carrying the values its message names, so a caller (the
//! installer, the app, the bench binary) matches it rather than its text.

use std::{fmt, net::SocketAddr, path::PathBuf};

use crate::config::ConfigError;
use crate::ota::image::ImageError;
use crate::protocol::MessageError;
use crate::server::NoFeederEnrolled;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("write {path}")]
    Write {
        path: PathBuf,
        #[source]
        source: wellfed_formats::Error,
    },
    #[error("create {path}")]
    Create {
        path: PathBuf,
        #[source]
        source: wellfed_formats::Error,
    },
    #[error("{path} has no parent directory")]
    NoParent { path: PathBuf },
    #[error("another server already owns the stock ledger at {path}")]
    LedgerHeld { path: PathBuf },
    #[error("invalid stock ledger {path}")]
    Ledger {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("the stock ledger {path} belongs to another device")]
    LedgerDevice { path: PathBuf },
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Image(#[from] ImageError),
    #[error(transparent)]
    Message(#[from] MessageError),
    #[error(transparent)]
    Connect(#[from] ConnectError),
    #[error(transparent)]
    Frame(#[from] wellfed_client::install::FrameError),
    #[error("open a UDP socket")]
    UdpSocket(#[source] std::io::Error),
    #[error("send the setup frame to {to}")]
    FrameSend {
        to: SocketAddr,
        #[source]
        source: std::io::Error,
    },
    #[error("read the feeder's answer")]
    FrameAnswer(#[source] std::io::Error),
    #[error("no feeder is enrolled in {path}; pass --enroll-minutes during an install")]
    NotEnrolled { path: PathBuf },
    #[error(transparent)]
    NoFeederEnrolled(#[from] NoFeederEnrolled),
    #[error(transparent)]
    Unreviewed(#[from] crate::build_info::Unreviewed),
    #[error("the broker ended while waiting for a feeder")]
    BrokerEnded,
    #[error("the stock server stopped unexpectedly")]
    StoppedUnexpectedly,
    #[error("the {task} panicked")]
    Panicked {
        task: Task,
        #[source]
        source: tokio::task::JoinError,
    },
    #[error("bind the {listener} on {address}")]
    Bind {
        listener: Listener,
        address: SocketAddr,
        #[source]
        source: std::io::Error,
    },
    /// One of the server's tasks ended with this error.
    #[error("the {task}")]
    Ended {
        task: Task,
        #[source]
        source: Box<Error>,
    },
    #[error("the broker refused the server's subscription")]
    SubscriptionRefused,
    #[error("the broker has no loopback listener")]
    NoLoopbackListener,
    #[error(transparent)]
    Mqtt(#[from] rumqttc::ClientError),
    #[error(transparent)]
    Broker(#[from] mqtt5::error::MqttError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Formats(#[from] wellfed_formats::Error),
}

impl Error {
    /// `self` as the end of `task`.
    pub(crate) fn of(self, task: Task) -> Self {
        Self::Ended {
            task,
            source: Box::new(self),
        }
    }
}

/// A task the server runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    StockServer,
    ImageServer,
    Broker,
    MqttWorker,
    Api,
}

impl fmt::Display for Task {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::StockServer => "stock server",
            Self::ImageServer => "image server",
            Self::Broker => "broker",
            Self::MqttWorker => "MQTT worker",
            Self::Api => "loopback API",
        })
    }
}

/// A listener the server binds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Listener {
    ImageServer,
    Api,
}

impl fmt::Display for Listener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ImageServer => "OTA image server",
            Self::Api => "loopback HTTP API",
        })
    }
}

/// A CONNECT the broker front could not normalize. Its bytes carry
/// credentials, so no message names them.
#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("expected CONNECT")]
    NotConnect,
    #[error("CONNECT exceeds {limit} bytes")]
    TooLong { limit: usize },
    #[error("malformed remaining length")]
    Length,
    #[error("truncated CONNECT")]
    Truncated,
    #[error("truncated MQTT 3.1 header")]
    TruncatedLegacy,
    #[error("unsupported CONNECT protocol")]
    Protocol,
    #[error("no CONNECT within its wait")]
    TimedOut,
}
