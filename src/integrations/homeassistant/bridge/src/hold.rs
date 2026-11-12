//! What holds a new feed back on either route besides the route's own lost
//! feeds, the owner's releases, and the store's word that it kept a feed
//! before it goes out.
//!
//! A feed whose answer was lost may have run, and the feeder's history names
//! this Home Assistant, not the request, so the next feed waits until the
//! lost one settles or the owner releases it, as the app's feeds wait on
//! every path. The hold belongs to the feed, not the route: an entry moved to
//! the other route takes its lost feeds along, and the new route holds on
//! them though it cannot settle them, until the owner releases them or the
//! entry moves back. Nothing lifts a hold on its own.
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
use crate::feeder::{forgotten, now_ms};

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

/// The holds a route keeps for what it cannot settle itself.
#[derive(Debug, Default)]
pub(crate) struct Holds {
    shared: Shared,
    /// Lost feeds that went by the other route, carried across a route
    /// change: this route never settles or forgets one it still holds on,
    /// and keeps one released for [`crate::feeder::KEEP_MS`] from its
    /// release, so the integration still has it if the entry moves back.
    elsewhere: Vec<Pending>,
    /// The integration's store before this one did not read, so a lost
    /// feed it kept, and which feeder it was on, are lost with it: every
    /// feed waits until the owner says the bowls are checked.
    unread: bool,
}

impl Holds {
    pub(crate) fn new(shared: Shared, elsewhere: Vec<Pending>, unread: bool) -> Self {
        Self {
            shared,
            elsewhere,
            unread,
        }
    }

    /// What a session announces when it starts: each lost feed from the
    /// other route still held, as reported still unknown, since only the
    /// owner's release ends it here.
    pub(crate) fn announce(&mut self) -> Vec<Update> {
        self.elsewhere
            .iter_mut()
            .filter(|pending| pending.released_ms.is_none())
            .map(|pending| {
                pending.reported = true;
                Update::still_unknown(pending.clone())
            })
            .collect()
    }

    /// Whether a new feed waits on what these hold.
    pub(crate) fn holding(&self) -> bool {
        self.unread
            || self
                .elsewhere
                .iter()
                .any(|pending| pending.released_ms.is_none())
    }

    /// Take the owner's releases, applying those for what these hold: the
    /// ids of the feeds left for the route's own lost feeds, and the updates
    /// that confirm the rest.
    pub(crate) fn take(&mut self) -> (Vec<String>, Vec<Update>) {
        let taken = std::mem::take(&mut *releases(&self.shared));
        let at = now_ms();
        let mut ids = Vec::new();
        let mut updates = Vec::new();
        for release in taken {
            match release {
                Release::BowlsChecked if self.unread => {
                    self.unread = false;
                    updates.push(Update::bowls_checked());
                }
                Release::BowlsChecked => {}
                Release::Feed(id) => {
                    let held = self
                        .elsewhere
                        .iter_mut()
                        .find(|pending| pending.id == id && pending.released_ms.is_none());
                    match held {
                        Some(pending) => {
                            pending.released_ms = Some(at);
                            updates.push(Update::released(pending.clone()));
                        }
                        None => ids.push(id),
                    }
                }
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

    /// Forget the feeds from the other route released a week ago.
    pub(crate) fn past_keep(&mut self) -> Vec<Update> {
        let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.elsewhere)
            .into_iter()
            .partition(|pending| pending.released_ms.is_some_and(forgotten));
        self.elsewhere = kept;
        gone.into_iter().map(Update::forgotten).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feeder::KEEP_MS;

    fn lost(id: &str) -> Pending {
        Pending {
            id: id.into(),
            portions: 1,
            sent_ms: now_ms() - KEEP_MS * 2,
            after: None,
            reported: false,
            released_ms: None,
            token_id: None,
        }
    }

    /// A feed from the other route holds however old it is, is announced
    /// so its repair shows, and once released is kept a week from then.
    #[test]
    fn a_feed_from_the_other_route_holds_until_released() {
        let shared = Shared::default();
        let mut holds = Holds::new(Arc::clone(&shared), vec![lost("other")], false);
        assert!(holds.holding());
        assert!(holds.past_keep().is_empty(), "never forgotten while held");
        let announced = holds.announce();
        assert_eq!(announced.len(), 1);
        assert!(announced[0].pending.as_ref().is_some_and(|p| p.reported));

        releases(&shared).push(Release::Feed("own".into()));
        releases(&shared).push(Release::Feed("other".into()));
        let (ids, updates) = holds.take();
        assert_eq!(ids, ["own"], "left for the route's own feeds");
        assert_eq!(updates.len(), 1);
        assert!(!holds.holding());
        assert!(holds.past_keep().is_empty(), "kept from its release");

        holds.elsewhere[0].released_ms = Some(now_ms() - KEEP_MS);
        assert_eq!(holds.past_keep().len(), 1);
    }

    /// An unread store holds every feed until the owner checks the bowls,
    /// confirmed once.
    #[test]
    fn an_unread_store_holds_until_the_bowls_are_checked() {
        let shared = Shared::default();
        let mut holds = Holds::new(Arc::clone(&shared), Vec::new(), true);
        assert!(holds.holding());
        releases(&shared).push(Release::BowlsChecked);
        releases(&shared).push(Release::BowlsChecked);
        let (ids, updates) = holds.take();
        assert!(ids.is_empty());
        assert_eq!(updates.len(), 1);
        assert!(!holds.holding());
    }
}
