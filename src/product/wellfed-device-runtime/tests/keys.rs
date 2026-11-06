//! The key mailbox: one queued change at a time for the settings writer,
//! and the identity published back with a new generation each time.

use std::sync::{Mutex, MutexGuard};

use wellfed_device_api::clients::{Label, Role};
use wellfed_device_runtime::keys::{self, Change, KEYS_FINISHED, KeyResult, Requested};

/// The module's state is process-wide.
static SERIAL: Mutex<()> = Mutex::new(());

fn start() -> MutexGuard<'static, ()> {
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    keys::reset();
    guard
}

#[test]
fn one_change_waits_for_the_writer() {
    let _guard = start();
    assert_eq!(keys::pending(), None);

    assert_eq!(keys::request(Change::Reset), Requested::Queued);
    let enroll = Change::Enroll {
        key: [7; 32],
        label: Label::new(b"phone").unwrap(),
        role: Role::Member,
    };
    assert_eq!(
        keys::request(enroll),
        Requested::Busy,
        "never queued behind"
    );
    assert_eq!(keys::pending(), Some(Change::Reset));
    assert_eq!(keys::pending(), Some(Change::Reset), "held until finish");

    let ticket = keys::ticket();
    assert_eq!(keys::outcome(ticket), None, "not finished yet");
    keys::finish(KeyResult::Done);
    assert_eq!(keys::outcome(ticket), Some(KeyResult::Done));
    assert_eq!(keys::pending(), None);
    assert_eq!(keys::request(enroll), Requested::Queued);
    assert_eq!(keys::pending(), Some(enroll));
}

/// A waiter that has not looked yet still finds its result after newer
/// changes finish behind it, up to as many as the mailbox remembers.
#[test]
fn earlier_results_outlive_later_changes() {
    let _guard = start();
    let revoke = |key| Change::Revoke { key: [key; 32] };

    assert_eq!(keys::request(revoke(1)), Requested::Queued);
    let first = keys::ticket();
    keys::finish(KeyResult::Done);
    assert_eq!(keys::request(revoke(2)), Requested::Queued);
    let second = keys::ticket();
    keys::finish(KeyResult::NotFound);
    assert_eq!(keys::outcome(first), Some(KeyResult::Done));
    assert_eq!(keys::outcome(second), Some(KeyResult::NotFound));

    for key in 3..=u8::try_from(KEYS_FINISHED).unwrap() {
        assert_eq!(keys::request(revoke(key)), Requested::Queued);
        keys::finish(KeyResult::Full);
    }
    assert_eq!(
        keys::outcome(first),
        Some(KeyResult::Done),
        "the oldest kept"
    );
    assert_eq!(keys::request(revoke(0)), Requested::Queued);
    keys::finish(KeyResult::Full);
    assert_eq!(keys::outcome(first), None, "one more pushes it out");
    assert_eq!(keys::outcome(second), Some(KeyResult::NotFound));
}

#[test]
fn each_publish_is_a_new_generation() {
    let _guard = start();
    assert_eq!(keys::identity_since(0), None, "nothing published yet");

    keys::publish_identity(&[1; 32]);
    let (first, secret) = keys::identity_since(0).unwrap();
    assert_eq!(secret, [1; 32]);
    assert_eq!(keys::identity_since(first), None, "already seen");

    keys::publish_identity(&[2; 32]);
    let (second, secret) = keys::identity_since(first).unwrap();
    assert_ne!(second, first);
    assert_eq!(secret, [2; 32]);
}
