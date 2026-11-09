//! What the worker keeps of each change: its id, and whether the
//! receiver took it.

use reqwest::StatusCode;
use wellfed_api::FeederId;
use wellfed_client::alerts::{Change, Delivery, NoticeKey, Tracked};
use wellfed_formats::canonical::sha256;
use wellfed_mirror::Kept;

use crate::body::Transition;

/// Whether the receiver took a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Told {
    /// It answered 2xx.
    Took,
    /// Refused, given up, or never sent.
    Not,
}

/// Keep `hooked` to the alerts the receiver took the opening of.
pub(crate) fn hook(hooked: &mut Vec<Tracked>, change: &Change, told: Told) {
    match change {
        Change::Post(tracked) => {
            hooked.retain(|each| each.key != tracked.key);
            if told == Told::Took {
                hooked.push(*tracked);
            }
        }
        Change::Remove(key) => hooked.retain(|each| each.key != *key),
    }
}

/// Why a try failed.
pub(crate) enum Failure {
    /// Worth trying again: no answer, or the receiver said so.
    Again(String),
    /// The receiver refused the body: trying again would get the same.
    Final(String),
}

/// What an answer of `status` other than 2xx means for the change.
pub(crate) fn refused(status: StatusCode) -> Failure {
    let again = status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error();
    let reason = format!("answered {status}");
    if again {
        Failure::Again(reason)
    } else {
        Failure::Final(reason)
    }
}

/// A request error's words without its URL, whose path may be a secret.
pub(crate) fn without_url(error: reqwest::Error) -> String {
    let kind = if error.is_timeout() {
        "timed out"
    } else if error.is_connect() {
        "could not connect"
    } else {
        "request failed"
    };
    format!("{kind}: {}", error.without_url())
}

/// The same for every try of one change: the feeder, the transition and
/// the alert, by its key, kind and when the server first saw it, so the
/// same alert opening again later is a new change.
pub(crate) fn delivery_id(feeder: FeederId, transition: Transition, tracked: &Tracked) -> String {
    let key: &NoticeKey = &tracked.key;
    // Each part is printed whole and none holds a `\n`, so no two changes
    // print alike.
    let text = format!(
        "{feeder}\n{}\n{}\n{}\n{}",
        transition.name(),
        key.text(),
        tracked.notice.kind().name(),
        tracked.seen.as_millisecond(),
    );
    let mut digest = sha256(text.as_bytes());
    digest.truncate(32);
    digest
}

/// What `change` would send, or `None` when it sends nothing: a quiet
/// alert, or the clear of one whose opening the receiver never took.
pub(crate) fn sendable(kept: &Kept, change: &Change) -> Option<(Transition, Tracked)> {
    let (transition, tracked) = match change {
        Change::Post(tracked) => (Transition::Opened, *tracked),
        Change::Remove(key) => (
            Transition::Cleared,
            *kept.hooked.iter().find(|hooked| hooked.key == *key)?,
        ),
    };
    (tracked.delivery != Delivery::Quiet).then_some((transition, tracked))
}
