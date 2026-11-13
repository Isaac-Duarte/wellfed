//! What holds a new feed back besides the session's own lost feeds, the
//! owner's releases, and the store's word that it kept a feed before it goes
//! out.
//!
//! A feed whose answer was lost may have run, and the feeder's history names
//! this Home Assistant, not the request, so the next feed waits until the
//! lost one settles or the owner releases it, as the app's feeds wait on
//! every path. Nothing lifts a hold on its own.
//!
//! The hold outlives a restart only through the integration's store, so a
//! feed goes out only once the store says it saved the feed's record
//! ([`Holds::kept_first`]), as the app writes a feed down before it sends
//! it: a crash between the two then leaves a record that holds, never a feed
//! that ran with none.

use std::collections::HashMap;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::sync::oneshot;

use crate::api::{Pending, Update};

/// How long a feed waits for the store to say it saved the feed's record.
/// A save takes milliseconds; one still out after this is a store that is
/// stuck, and the owner who pressed has been waiting the while.
pub(crate) const SAVE_WAIT: Duration = Duration::from_secs(5);

/// What the owner released through a repair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Release {
    /// The lost feed of this id: feeds go again though it may have run.
    Feed(String),
    /// The bowls are checked after the integration's store did not read.
    BowlsChecked,
}

/// What the integration's store said of a feed's record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Saved {
    /// The record is on disk: the feed may go.
    Kept,
    /// The record did not land: the feed never goes.
    Failed,
}

/// What the handle hands the session thread outside its commands. Shared
/// rather than sent as a command: a release made between sessions, when
/// every command is refused, is applied at the next session rather than
/// dropped, and a save's answer reaches a session waiting on it in the
/// middle of a write, when it reads no command.
#[derive(Debug, Default)]
pub(crate) struct Inner {
    /// The owner's releases, until the session thread takes them.
    releases: Mutex<Vec<Release>>,
    /// The feeds waiting for the store's word, by their pending's id.
    saves: Mutex<HashMap<String, oneshot::Sender<Saved>>>,
}

pub(crate) type Shared = Arc<Inner>;

/// The releases waiting. A thread that panicked holding the lock left the
/// list whole, so it is used as it is.
pub(crate) fn releases(shared: &Shared) -> MutexGuard<'_, Vec<Release>> {
    shared
        .releases
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

fn saves(shared: &Shared) -> MutexGuard<'_, HashMap<String, oneshot::Sender<Saved>>> {
    shared.saves.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The store's word on the feed `id` names: the feed waiting on it goes,
/// or never does. A word for no feed waiting (one that gave up waiting) is
/// dropped.
pub(crate) fn saved(shared: &Shared, id: &str, saved: Saved) {
    if let Some(waiting) = saves(shared).remove(id) {
        let _ = waiting.send(saved);
    }
}

/// The holds a session keeps besides its own lost feeds.
#[derive(Debug, Default)]
pub(crate) struct Holds {
    shared: Shared,
    /// The integration's store before this one did not read, so a lost
    /// feed it kept, and which feeder it was on, are lost with it: every
    /// feed waits until the owner says the bowls are checked.
    unread: bool,
}

impl Holds {
    pub(crate) fn new(shared: Shared, unread: bool) -> Self {
        Self { shared, unread }
    }

    /// Whether a new feed waits on what these hold.
    pub(crate) const fn holding(&self) -> bool {
        self.unread
    }

    /// Take the owner's releases, applying the bowl check: the ids of the
    /// lost feeds released, for the session to apply, and the update that
    /// confirms the check.
    pub(crate) fn take(&mut self) -> (Vec<String>, Vec<Update>) {
        let taken = std::mem::take(&mut *releases(&self.shared));
        let mut ids = Vec::new();
        let mut updates = Vec::new();
        for release in taken {
            match release {
                Release::BowlsChecked if self.unread => {
                    self.unread = false;
                    updates.push(Update::bowls_checked());
                }
                Release::BowlsChecked => {}
                Release::Feed(id) => ids.push(id),
            }
        }
        (ids, updates)
    }

    /// Announce `pending` about to go out ([`Update::sending`]) and wait
    /// for the integration's store to say it saved the record: whether the
    /// feed may go. No word within [`SAVE_WAIT`], or no one left to give
    /// it, is a record that did not land.
    pub(crate) async fn kept_first(
        &self,
        updates: &std_mpsc::Sender<Update>,
        pending: &Pending,
    ) -> Saved {
        let (waiting, word) = oneshot::channel();
        saves(&self.shared).insert(pending.id.clone(), waiting);

        let saved = if updates.send(Update::sending(pending.clone())).is_ok() {
            match tokio::time::timeout(SAVE_WAIT, word).await {
                Ok(Ok(saved)) => saved,
                Ok(Err(_)) | Err(_) => Saved::Failed,
            }
        } else {
            Saved::Failed
        };

        saves(&self.shared).remove(&pending.id);
        saved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An unread store holds every feed until the owner checks the bowls,
    /// confirmed once.
    #[test]
    fn an_unread_store_holds_until_the_bowls_are_checked() {
        let shared = Shared::default();
        let mut holds = Holds::new(Arc::clone(&shared), true);
        assert!(holds.holding());
        releases(&shared).push(Release::BowlsChecked);
        releases(&shared).push(Release::BowlsChecked);
        let (ids, updates) = holds.take();
        assert!(ids.is_empty());
        assert_eq!(updates.len(), 1);
        assert!(!holds.holding());
    }
}
