//! The server's one install slot: the running install, held by a
//! [`Claim`] that frees it when dropped, and the ends the store did not
//! take.

use std::sync::{Arc, Mutex};

use wellfed_api::Install;

use super::InstallError;
use crate::{Result, lock};

/// The server's one install slot, with the ends the store did not take.
pub(crate) type Slot = Arc<Mutex<Installs>>;

/// What the service keeps of installs in memory.
#[derive(Debug, Default)]
pub(crate) struct Installs {
    /// The running install, as it stands.
    pub(super) running: Option<Install>,
    /// Installs that ended while the store's row still says `running`:
    /// their end was not stored.
    pub(super) unsaved: Vec<Install>,
}

/// The slot, held by the one running install; dropping it frees the slot.
pub(crate) struct Claim {
    slot: Slot,
    pub(super) install: Install,
}

impl Claim {
    pub(super) fn take(slot: &Slot, install: Install) -> Result<Self> {
        let mut held = lock(slot);
        if held.running.is_some() {
            return Err(InstallError::Running.into());
        }
        held.running = Some(install);
        Ok(Self {
            slot: Arc::clone(slot),
            install,
        })
    }

    /// Change the running install and return it as it now stands.
    pub(super) fn update(&mut self, change: impl FnOnce(&mut Install)) -> Install {
        change(&mut self.install);
        lock(&self.slot).running = Some(self.install);
        self.install
    }

    /// Keep the ended `install`, whose end the store did not take.
    pub(super) fn unsaved(&self, install: Install) {
        lock(&self.slot).unsaved.push(install);
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        lock(&self.slot).running = None;
    }
}
