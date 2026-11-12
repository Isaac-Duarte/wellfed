//! A session handle that gives each feed's record the store's word as Home
//! Assistant's event loop does, so a test drives the bridge as the
//! integration would: a feed goes out only once its `Sending` update is
//! answered, and the test may have the store refuse it, or say nothing.
#![allow(dead_code, reason = "each test crate uses its own part")]

use std::ops::Deref;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use wellfed_homeassistant::api::{Update, UpdateKind};
use wellfed_homeassistant::feeder::Feeder;

/// What the store says of a feed's record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Store {
    /// Saved: the feed goes.
    Saves,
    /// Could not save it: the feed never goes.
    Refuses,
    /// No word at all, as a store that is stuck.
    Silent,
}

impl Store {
    fn from_byte(byte: u8) -> Self {
        match byte {
            0 => Self::Saves,
            1 => Self::Refuses,
            _ => Self::Silent,
        }
    }
}

/// A [`Feeder`] whose updates pass through a relay thread that answers
/// each `Sending` update, as the store said, before the test hears it.
pub struct Saving {
    feeder: Arc<Feeder>,
    heard: Mutex<mpsc::Receiver<Update>>,
    store: Arc<AtomicU8>,
    stop: Arc<AtomicBool>,
    relay: Option<JoinHandle<()>>,
}

impl Saving {
    pub fn new(feeder: Feeder) -> Self {
        let feeder = Arc::new(feeder);
        let store = Arc::new(AtomicU8::new(Store::Saves as u8));
        let stop = Arc::new(AtomicBool::new(false));
        let (tell, heard) = mpsc::channel();

        let relay = {
            let feeder = Arc::clone(&feeder);
            let store = Arc::clone(&store);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let Some(update) = feeder.next_update(Duration::from_millis(20)) else {
                        continue;
                    };
                    if update.kind == UpdateKind::Sending
                        && let Some(pending) = &update.pending
                    {
                        match Store::from_byte(store.load(Ordering::SeqCst)) {
                            Store::Saves => feeder.saved(&pending.id),
                            Store::Refuses => feeder.unsaved(&pending.id),
                            Store::Silent => {}
                        }
                    }
                    if tell.send(update).is_err() {
                        return;
                    }
                }
            })
        };

        Self {
            feeder,
            heard: Mutex::new(heard),
            store,
            stop,
            relay: Some(relay),
        }
    }

    /// What the store says of the records of feeds from now on.
    pub fn store(&self, store: Store) {
        self.store.store(store as u8, Ordering::SeqCst);
    }

    /// The next update within `wait`, or `None`, as [`Feeder::next_update`].
    pub fn next_update(&self, wait: Duration) -> Option<Update> {
        let heard = self.heard.lock().ok()?;
        heard.recv_timeout(wait).ok()
    }
}

impl Deref for Saving {
    type Target = Feeder;

    fn deref(&self) -> &Feeder {
        &self.feeder
    }
}

/// Dropping the handle ends the session, as dropping a [`Feeder`] does:
/// the relay lets go of it first.
impl Drop for Saving {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(relay) = self.relay.take() {
            let _ = relay.join();
        }
    }
}
