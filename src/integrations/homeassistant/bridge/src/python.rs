//! The `wellfed_homeassistant` Python module. Every value the feeder
//! defines crosses as one of [`crate::api`]'s typed views, so Python reads
//! classes the stub describes rather than a copy of the model's spellings.
//! Each blocking call releases the interpreter while it waits.

use std::time::Duration;

use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

use crate::Error;
use crate::api::{Role, Update, Written};
use crate::feeder::{self, Target};
use crate::key::{self, ClientKey, bytes32};

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

/// Redeem `invite` as the key `secret`, listed as `label`; the role the
/// feeder enrolled it with. Runs once: an invite is single-use.
#[pyfunction]
#[pyo3(signature = (secret, invite, label, address=None))]
fn join(
    py: Python<'_>,
    secret: &[u8],
    invite: &str,
    label: &str,
    address: Option<&str>,
) -> PyResult<Role> {
    let key = ClientKey::restore(secret)?;
    let role = py.detach(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(Error::Thread)?;
        runtime.block_on(key::join(&key, invite, label, address))
    })?;
    Ok(role.into())
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

    /// Lock or unlock the panel; how it ended.
    fn set_lock(&self, py: Python<'_>, locked: bool) -> Written {
        py.detach(|| self.0.set_lock(locked))
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
        By, Event, EventKind, Feed, FeedFault, FeedOutcome, FeedReason, FoodLevel, Readiness,
        RefusalKind, Role, Status, Update, UpdateKind, WriteResult, Written,
    };
}
