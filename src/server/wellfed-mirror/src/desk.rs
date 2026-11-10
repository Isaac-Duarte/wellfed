//! What each feeder's alert reader knows beyond its kept row: when it last
//! checked the table, and whether a removal holds it. In memory only.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

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
        self.with(id, |desked| desked.checked)
    }

    pub(crate) fn hold(&self, id: FeederId, hold: AlertHold) {
        self.with(id, |desked| desked.held = hold == AlertHold::Held);
    }

    pub(crate) fn held(&self, id: FeederId) -> bool {
        self.with(id, |desked| desked.held)
    }

    pub(crate) fn check(&self, id: FeederId, at: Timestamp) {
        self.with(id, |desked| desked.checked = Some(at));
    }

    fn with<R>(&self, id: FeederId, act: impl FnOnce(&mut Desked) -> R) -> R {
        // Each change is one field: a poisoned map is still whole.
        let mut feeders = self.feeders.lock().unwrap_or_else(PoisonError::into_inner);
        act(feeders.entry(id).or_default())
    }
}
