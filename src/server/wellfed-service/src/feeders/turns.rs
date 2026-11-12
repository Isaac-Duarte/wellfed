//! Each feeder's turn to be adopted or removed, by its identity. Two
//! adoptions racing for one feeder would both enroll a key, the loser's
//! holding a client slot on the feeder with nothing on the server to use
//! it, and two removals would both revoke. Different feeders never wait
//! on each other: a search for a lost join's key holds its feeder's turn
//! for up to its whole window.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::{Mutex as TurnLock, OwnedMutexGuard};

use crate::lock;

/// Each feeder's lock, by its identity, while anyone holds or waits for it.
type ByIdentity = HashMap<[u8; 32], Arc<TurnLock<()>>>;

#[derive(Clone, Default)]
pub(crate) struct Turns(Arc<Mutex<ByIdentity>>);

/// A feeder's turn, held until it drops.
pub(crate) struct Turn {
    turns: Turns,
    identity: [u8; 32],
    held: Option<OwnedMutexGuard<()>>,
}

impl Turns {
    fn entry(&self, identity: [u8; 32]) -> Arc<TurnLock<()>> {
        Arc::clone(lock(&self.0).entry(identity).or_default())
    }

    fn turn(&self, identity: [u8; 32], held: OwnedMutexGuard<()>) -> Turn {
        Turn {
            turns: self.clone(),
            identity,
            held: Some(held),
        }
    }

    /// The feeder's turn now, or `None` while another holds it: an
    /// adoption is never queued behind another's search.
    pub(crate) fn try_take(&self, identity: [u8; 32]) -> Option<Turn> {
        let held = self.entry(identity).try_lock_owned().ok()?;
        Some(self.turn(identity, held))
    }

    /// The feeder's turn, once whoever holds it lets go.
    pub(crate) async fn take(&self, identity: [u8; 32]) -> Turn {
        let held = self.entry(identity).lock_owned().await;
        self.turn(identity, held)
    }

    /// Drop the feeder's entry once nobody holds or waits for it, so a
    /// feeder tried once and never adopted leaves nothing behind. Entries
    /// are only cloned under the map's lock, so the count cannot rise
    /// while it is read.
    fn forget(&self, identity: [u8; 32]) {
        let mut map = lock(&self.0);
        if map
            .get(&identity)
            .is_some_and(|entry| Arc::strong_count(entry) == 1)
        {
            map.remove(&identity);
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        lock(&self.0).len()
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        // The guard holds a count of its own, so it goes first.
        drop(self.held.take());
        self.turns.forget(self.identity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_turn_refuses_its_feeder_only_and_leaves_nothing_once_dropped() {
        let turns = Turns::default();
        let held = turns.try_take([1; 32]).expect("free");

        assert!(turns.try_take([1; 32]).is_none(), "held");
        let other = turns.try_take([2; 32]).expect("another feeder's is free");
        drop(other);
        assert_eq!(turns.len(), 1);
        drop(held);
        assert_eq!(turns.len(), 0);
        assert!(turns.try_take([1; 32]).is_some(), "free again");
        assert_eq!(turns.len(), 0);
    }
}
