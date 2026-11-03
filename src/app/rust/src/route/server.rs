//! The router's calls to the server: each pinned write under its key, the
//! reads the server serves, and how a failed call reads to the rules in
//! [`super::core`].

use uuid::Uuid;
use wellfed_api::{Command, ErrorCode, FeederId, Generation, PlansChange};
use wellfed_server_client::{Client, Error, Lock};

use crate::api::feeder::{FeederRefusal, FeederReply, FeederRequest};
use crate::api::session::FeedRefusal;
use crate::requests::{plans_view, settings_view};
use crate::route::core::ServerFailure;
use crate::route::pins::PinnedWrite;

/// What the server answered a write.
pub(crate) enum Answered {
    /// A feed's journal entry: reserved (`feeding`), or, for a retry, what
    /// it came to since.
    Feed(Command),
    /// A write that took effect; `generation` is the store's after it.
    Done { generation: Option<u32> },
}

/// Send `write` to feeder `id` under `key`; `None` for a write no server
/// route carries.
pub(crate) async fn send(
    client: &Client,
    id: FeederId,
    write: &PinnedWrite,
    key: Uuid,
) -> Option<Result<Answered, Error>> {
    let sent = match *write {
        PinnedWrite::Feed { portions } => client.feed(id, portions, key).await.map(Answered::Feed),
        PinnedWrite::Cancel { feed } => client
            .cancel_feed(id, feed, key)
            .await
            .map(|()| Answered::Done { generation: None }),
        PinnedWrite::Settings { base, changes } => client
            .put_settings(id, Generation(base), &changes, key)
            .await
            .map(|settings| Answered::Done {
                generation: Some(settings.generation.0),
            }),
        PinnedWrite::Lock { locked } => {
            let lock = if locked { Lock::Locked } else { Lock::Unlocked };
            client
                .set_lock(id, lock, key)
                .await
                .map(|()| Answered::Done { generation: None })
        }
        PinnedWrite::Desiccant => client
            .reset_desiccant(id, key)
            .await
            .map(|()| Answered::Done { generation: None }),
        PinnedWrite::Plans { base, plans } => client
            .put_plans(id, Generation(base), &PlansChange { plans }, key)
            .await
            .map(|plans| Answered::Done {
                generation: Some(plans.generation.0),
            }),
        PinnedWrite::Other { .. } => return None,
    };
    Some(sent)
}

/// Run a read the server serves; `None` for one it has no route for.
pub(crate) async fn read(
    client: &Client,
    id: FeederId,
    request: &FeederRequest,
) -> Option<Result<FeederReply, Error>> {
    let read = match request {
        FeederRequest::GetSettings => {
            client
                .settings(id)
                .await
                .map(|settings| FeederReply::Settings {
                    settings: settings_view(&settings),
                })
        }
        FeederRequest::GetPlans => client.plans(id).await.map(|plans| FeederReply::Plans {
            plans: plans_view(&plans),
        }),
        _ => return None,
    };
    Some(read)
}

/// How a failed call reads to the router's rules. Only a failure to
/// connect counts as nothing sent: reqwest's connect errors are the
/// resolver, the TCP connect and the TLS handshake, all before a byte of
/// the request is written; a connection that breaks after is a request
/// error, which may have run.
pub(crate) fn failure(error: &Error) -> ServerFailure {
    match error {
        Error::Http(http) if http.is_connect() => ServerFailure::Connect,
        Error::Http(_) | Error::Decode(_) | Error::Unexpected { .. } => ServerFailure::Interrupted,
        Error::Problem {
            status, problem, ..
        } => match problem.code {
            ErrorCode::OutcomeUnknown => ServerFailure::OutcomeUnknown,
            ErrorCode::FeederOffline | ErrorCode::Unreachable => ServerFailure::FeederOffline,
            ErrorCode::FeederRefused => ServerFailure::FeederRefused,
            ErrorCode::KeyReused => ServerFailure::KeyReused,
            // The server failed inside, which may be after it sent.
            _ if status.is_server_error() => ServerFailure::Interrupted,
            _ => ServerFailure::Rejected,
        },
        Error::Stale(_) | Error::StalePlans(_) => ServerFailure::Stale,
        Error::Base(_) | Error::Build(_) | Error::Redirect { .. } | Error::NoSession => {
            ServerFailure::Rejected
        }
    }
}

/// The feeder's refusal a failed write carries, as Dart's reply names it.
pub(crate) fn refusal(error: &Error) -> Option<FeederRefusal> {
    use wellfed_api::FeederRefusal as Api;
    match error {
        Error::Stale(settings) => Some(FeederRefusal::Stale {
            current: settings.generation.0,
        }),
        Error::StalePlans(plans) => Some(FeederRefusal::Stale {
            current: plans.generation.0,
        }),
        Error::Problem { problem, .. } => problem.refusal.map(|refusal| match refusal {
            Api::Forbidden => FeederRefusal::Forbidden,
            // The server's refusal does not carry the generation.
            Api::Stale => FeederRefusal::Stale { current: 0 },
            Api::Busy => FeederRefusal::Busy,
            Api::Unavailable => FeederRefusal::Unavailable,
            Api::Invalid => FeederRefusal::Invalid,
            Api::NotWritable => FeederRefusal::NotWritable,
            Api::Full => FeederRefusal::Full,
            Api::NotFound => FeederRefusal::NotFound,
            Api::Muted => FeederRefusal::Muted,
            Api::Time => FeederRefusal::Time,
            Api::Fault => FeederRefusal::Fault,
            Api::Portions => FeederRefusal::Portions { asked: 0 },
            Api::NoClock => FeederRefusal::NoClock,
        }),
        _ => None,
    }
}

/// A feed's refusal, as the feed sheet names it.
pub(crate) const fn feed_refusal(refusal: FeederRefusal) -> FeedRefusal {
    match refusal {
        FeederRefusal::Forbidden => FeedRefusal::Forbidden,
        FeederRefusal::Busy => FeedRefusal::Busy,
        FeederRefusal::Fault => FeedRefusal::Fault,
        FeederRefusal::Portions { .. } => FeedRefusal::Portions,
        FeederRefusal::NotFound => FeedRefusal::NotFound,
        FeederRefusal::Unavailable => FeedRefusal::Unavailable,
        _ => FeedRefusal::Other,
    }
}
