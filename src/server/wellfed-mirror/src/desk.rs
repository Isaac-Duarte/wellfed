//! What each feeder's alert reader knows beyond its kept row: when it last
//! checked the table, and whether a removal holds it. In memory only.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use jiff::Timestamp;
use wellfed_api::FeederId;

/// Whether a feeder's alerts are judged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertHold {
    /// Not while its removal runs: nothing read or judged.
    Held,
    Released,
}

/// What each feeder's alert reader knows beyond the kept row, in memory.
#[derive(Debug, Default)]
pub(crate) struct Desk {
    feeders: Mutex<HashMap<FeederId, Desked>>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Desked {
    /// When the table was last read or found unchanged. A poll that finds
    /// it unchanged writes nothing to the row, so this is kept here; after
    /// a restart the first poll sets it again, within a minute.
    checked: Option<Timestamp>,
    held: bool,
}

impl Desk {
    pub(crate) fn checked(&self, id: FeederId) -> Option<Timestamp> {
        self.feeders().get(&id).and_then(|desked| desked.checked)
    }

    pub(crate) fn hold(&self, id: FeederId, hold: AlertHold) {
        self.change(id, |desked| desked.held = hold == AlertHold::Held);
    }

    pub(crate) fn held(&self, id: FeederId) -> bool {
        self.feeders().get(&id).is_some_and(|desked| desked.held)
    }

    pub(crate) fn check(&self, id: FeederId, at: Timestamp) {
        self.change(id, |desked| desked.checked = Some(at));
    }

    /// The feeder's reader stopped: its last check says nothing of the
    /// next run's. A hold stays, since a removal stops the reader while
    /// it holds the alerts.
    pub(crate) fn forget(&self, id: FeederId) {
        self.change(id, |desked| desked.checked = None);
    }

    /// Change one feeder's entry, keeping one only while it holds
    /// something, so a feeder stopped or removed leaves none behind.
    fn change(&self, id: FeederId, act: impl FnOnce(&mut Desked)) {
        let mut feeders = self.feeders();
        let desked = feeders.entry(id).or_default();
        act(desked);
        if !desked.held && desked.checked.is_none() {
            feeders.remove(&id);
        }
    }

    fn feeders(&self) -> MutexGuard<'_, HashMap<FeederId, Desked>> {
        // Each change is one field: a poisoned map is still whole.
        self.feeders.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    const ID: FeederId = FeederId(Uuid::from_u128(7));

    #[test]
    fn a_feeder_leaves_no_entry_once_stopped_and_released() {
        let desk = Desk::default();
        assert!(!desk.held(ID));
        assert_eq!(desk.checked(ID), None);
        assert!(desk.feeders().is_empty(), "a read adds nothing");

        desk.check(ID, Timestamp::UNIX_EPOCH);
        desk.hold(ID, AlertHold::Held);
        desk.forget(ID);
        assert!(desk.held(ID), "a removal's hold outlasts the reader");
        assert_eq!(desk.checked(ID), None, "the stopped run's check is gone");
        desk.hold(ID, AlertHold::Released);
        assert!(desk.feeders().is_empty());

        desk.check(ID, Timestamp::UNIX_EPOCH);
        desk.forget(ID);
        assert!(desk.feeders().is_empty());
    }
}
