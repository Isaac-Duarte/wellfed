//! The `wellfed_homeassistant` Python module. Every value the feeder
//! defines crosses as one of [`crate::api`]'s typed views, so Python reads
//! classes the stub describes rather than a copy of the model's spellings.
//! Each blocking call releases the interpreter while it waits.

use std::time::Duration;

use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

use crate::Error;
use wellfed_model::FeederSettingsPatch;

use crate::api::{Tune, Update, Written};
use crate::feeder::{self, Clocks, Target};
use crate::key::{self, ClientKey, Joined, bytes32};

/// A call into the Wellfed bridge failed.
///
/// A class rather than `create_exception!`, so the stub describes it.
#[pyclass(extends = PyException, subclass, module = "wellfed_homeassistant")]
struct WellfedError;

#[pymethods]
impl WellfedError {
    #[new]
    #[pyo3(signature = (*_args))]
    fn new(_args: &Bound<'_, PyTuple>) -> Self {
        Self
    }
}

impl From<Error> for PyErr {
    fn from(error: Error) -> Self {
        PyErr::new::<WellfedError, _>(error.to_string())
    }
}

/// A new client key's secret, for the config entry.
#[pyfunction]
fn generate_secret() -> PyResult<Vec<u8>> {
    Ok(ClientKey::generate()?.secret().to_vec())
}

/// The public key of `secret`, as the feeder lists this client.
#[pyfunction]
fn public_key(secret: &[u8]) -> PyResult<Vec<u8>> {
    Ok(ClientKey::restore(secret)?.public().to_vec())
}

/// The identity key of the feeder `text` invites to.
#[pyfunction]
fn invite_feeder(text: &str) -> PyResult<Vec<u8>> {
    Ok(key::invite_feeder(text)?.to_vec())
}

/// Redeem `invite` as the key `secret`, listed as `label`, keeping the key
/// only as a member: any other role is revoked at once. Runs once: an invite
/// is single-use.
#[pyfunction]
#[pyo3(signature = (secret, invite, label, address=None))]
fn join(
    py: Python<'_>,
    secret: &[u8],
    invite: &str,
    label: &str,
    address: Option<&str>,
) -> PyResult<Joined> {
    let key = ClientKey::restore(secret)?;
    let joined = py.detach(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(Error::Thread)?;
        runtime.block_on(key::join_member(&key, invite, label, address))
    })?;
    Ok(joined)
}

/// One feeder's session, on a thread of its own until this is dropped.
#[pyclass(frozen)]
struct Feeder(feeder::Feeder);

#[pymethods]
impl Feeder {
    #[new]
    #[pyo3(signature = (secret, identity, address=None))]
    fn new(secret: &[u8], identity: &[u8], address: Option<String>) -> PyResult<Self> {
        let key = ClientKey::restore(secret)?;
        let identity = bytes32(identity, Error::IdentityLength)?;
        Ok(Self(feeder::Feeder::start(
            key,
            Target { identity, address },
            Clocks::DEFAULT,
        )?))
    }

    /// The next update within `timeout` seconds, or `None`.
    fn next_update(&self, py: Python<'_>, timeout: f64) -> Option<Update> {
        let wait = Duration::try_from_secs_f64(timeout.max(0.0)).unwrap_or(Duration::ZERO);
        py.detach(|| self.0.next_update(wait))
    }

    /// Start a feed; how it ended. Never retry an uncertain one.
    fn feed(&self, py: Python<'_>, portions: u8) -> Written {
        py.detach(|| self.0.feed(portions))
    }

    /// Stop the feed the feeder numbered `feed`, if it still runs.
    fn cancel_feed(&self, py: Python<'_>, feed: u32) -> Written {
        py.detach(|| self.0.cancel_feed(feed))
    }

    /// Lock or unlock the panel; how it ended.
    fn set_lock(&self, py: Python<'_>, locked: bool) -> Written {
        py.detach(|| self.0.set_lock(locked))
    }

    /// Record a fresh desiccant pack today.
    fn reset_desiccant(&self, py: Python<'_>) -> Written {
        py.detach(|| self.0.reset_desiccant())
    }

    fn play(&self, py: Python<'_>, tune: &Tune) -> Written {
        let tune = *tune;
        py.detach(|| self.0.play(tune))
    }

    /// Change the settings named, on the settings last read; the rest
    /// stay as they are. One settings change at a time: another while it
    /// waits is not sent.
    #[pyo3(signature = (*, sound=None, lights=None, volume=None, desiccant_days=None))]
    fn put_settings(
        &self,
        py: Python<'_>,
        sound: Option<bool>,
        lights: Option<bool>,
        volume: Option<u8>,
        desiccant_days: Option<u8>,
    ) -> Written {
        let changes = FeederSettingsPatch {
            sound,
            lights,
            volume,
            desiccant_days,
            ..FeederSettingsPatch::default()
        };
        py.detach(|| self.0.put_settings(changes))
    }

    /// Turn the plan in `slot` on or off, on the plans last read.
    fn set_plan_enabled(&self, py: Python<'_>, slot: u8, enabled: bool) -> Written {
        py.detach(|| self.0.set_plan_enabled(slot, enabled))
    }

    /// Store the weight of one portion in milligrams, 1,000-50,000.
    fn set_portion_weight(&self, py: Python<'_>, mg: u32) -> Written {
        py.detach(|| self.0.set_portion_weight(mg))
    }

    /// Read the status now.
    fn refresh(&self) {
        self.0.refresh();
    }
}

#[pymodule]
mod wellfed_homeassistant {
    #[pymodule_export]
    use super::{Feeder, WellfedError, generate_secret, invite_feeder, join, public_key};
    #[pymodule_export]
    use crate::api::{
        By, Capabilities, Event, EventKind, Feed, FeedFault, FeedOutcome, FeedReason, FoodLevel,
        NextMeal, NotSent, Plan, PortionsToday, Readiness, RefusalKind, Role, Settings, State,
        Status, Tune, Update, UpdateKind, WriteResult, Written,
    };
    #[pymodule_export]
    use crate::key::Joined;
}
