//! The router's calls to the server: each write's one send under its key,
//! the reads the server serves, and how a failed call reads to the rules in
//! [`super::core`].

use uuid::Uuid;
use wellfed_api::{FeederId, Generation, MealRequest, PlansChange, Settings};
use wellfed_server_client::{Client, Error, Lock};

use crate::api::feeder::{FeederRefusal, FeederReply, FeederRequest};
use crate::api::session::FeedRefusal;
use crate::requests::{calibration_of, call_of, plans_view, settings_view};
use crate::route::shape::SentWrite;

const fn done(generation: Option<u32>) -> FeederReply {
    FeederReply::Done { generation }
}

/// Send `write` to feeder `id` under `key`, its one send; `stream` is a
/// meal call upload's clip. `None` for a write no server route carries. A
/// feed is not one of these: it goes by [`Client::feed`] for its journal entry.
pub(crate) async fn send(
    client: &Client,
    id: FeederId,
    write: &SentWrite,
    stream: Option<Vec<u8>>,
    key: Uuid,
) -> Option<Result<FeederReply, Error>> {
    let sent = match *write {
        SentWrite::Cancel { feed } => client.cancel_feed(id, feed, key).await.map(|()| done(None)),
        SentWrite::Settings { base, changes } => client
            .put_settings(id, Generation(base), &changes, key)
            .await
            .map(|settings| done(settings.generation().map(|generation| generation.0))),
        SentWrite::Lock { locked } => {
            let lock = if locked { Lock::Locked } else { Lock::Unlocked };
            client.set_lock(id, lock, key).await.map(|()| done(None))
        }
        SentWrite::Desiccant => client.reset_desiccant(id, key).await.map(|()| done(None)),
        SentWrite::Skip { slot, due } => client
            .skip_meal(id, MealRequest { slot, due }, key)
            .await
            .map(|()| done(None)),
        SentWrite::Unskip { slot, due } => client
            .unskip_meal(id, MealRequest { slot, due }, key)
            .await
            .map(|()| done(None)),
        SentWrite::Plans { base, plans } => client
            .put_plans(id, Generation(base), &PlansChange { plans }, key)
            .await
            .map(|plans| done(Some(plans.generation.0))),
        SentWrite::Calibration { base, calibration } => client
            .put_calibration(id, Generation(base), &calibration, key)
            .await
            .map(|view| FeederReply::Calibration {
                calibration: calibration_of(view),
            }),
        SentWrite::CallUpload => {
            client
                .upload_call(id, stream?, key)
                .await
                .map(|view| FeederReply::Call {
                    call: call_of(view),
                })
        }
        SentWrite::CallDelete => client.delete_call(id, key).await.map(|()| done(None)),
        SentWrite::CallPlay => client.play_call(id, key).await.map(|()| done(None)),
        SentWrite::Other { .. } => return None,
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
        FeederRequest::GetSettings => client.settings(id).await.map(|settings| match settings {
            Settings::Feeder(settings) => FeederReply::Settings {
                settings: settings_view(&settings),
            },
            // A kind of device a newer server added: nothing here reads it.
            Settings::Unknown => FeederReply::Unreadable,
        }),
        FeederRequest::GetPlans => client.plans(id).await.map(|plans| FeederReply::Plans {
            plans: plans_view(&plans),
        }),
        FeederRequest::GetCalibration => {
            client
                .calibration(id)
                .await
                .map(|view| FeederReply::Calibration {
                    calibration: calibration_of(view),
                })
        }
        FeederRequest::GetCall => client.call(id).await.map(|view| FeederReply::Call {
            call: call_of(view),
        }),
        _ => return None,
    };
    Some(read)
}

/// The feeder's refusal a failed write carries, as Dart's reply names it.
pub(crate) fn refusal(error: &Error) -> Option<FeederRefusal> {
    use wellfed_api::FeederRefusal as Api;
    match error {
        Error::Stale(settings) => Some(FeederRefusal::Stale {
            current: settings.generation().map(|generation| generation.0),
        }),
        Error::StalePlans(plans) => Some(FeederRefusal::Stale {
            current: Some(plans.generation.0),
        }),
        Error::StaleCalibration(calibration) => Some(FeederRefusal::Stale {
            current: Some(calibration.generation.0),
        }),
        Error::Problem { problem, .. } => problem.refusal.map(|refusal| match refusal {
            Api::Forbidden => FeederRefusal::Forbidden,
            Api::Stale => FeederRefusal::Stale { current: None },
            Api::Busy => FeederRefusal::Busy,
            Api::Unavailable => FeederRefusal::Unavailable,
            Api::Invalid => FeederRefusal::Invalid,
            Api::NotWritable => FeederRefusal::NotWritable,
            Api::Full => FeederRefusal::Full,
            Api::NotFound => FeederRefusal::NotFound,
            Api::Muted => FeederRefusal::Muted,
            Api::Time => FeederRefusal::Time,
            Api::Fault => FeederRefusal::Fault,
            Api::Portions => FeederRefusal::Portions { asked: None },
            Api::NoClock => FeederRefusal::NoClock,
            Api::Unverified => FeederRefusal::Unverified,
            Api::Unknown => FeederRefusal::Unknown,
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
