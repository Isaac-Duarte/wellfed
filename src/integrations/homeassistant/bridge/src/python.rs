//! The `wellfed_homeassistant` Python module. Every value the feeder
//! defines crosses as the device API model's own JSON, so Python reads
//! `wellfed-model`'s spellings rather than a copy of them. Each blocking
//! call releases the interpreter while it waits.

use std::time::Duration;

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;

use crate::Error;
use crate::feeder::{self, Target};
use crate::key::{self, ClientKey, bytes32};

create_exception!(
    wellfed_homeassistant,
    WellfedError,
    PyException,
    "A call into the Wellfed bridge failed."
);

impl From<Error> for PyErr {
    fn from(error: Error) -> Self {
        WellfedError::new_err(error.to_string())
    }
}

fn json(value: &impl serde::Serialize) -> PyResult<String> {
    serde_json::to_string(value).map_err(|error| WellfedError::new_err(error.to_string()))
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
) -> PyResult<String> {
    let key = ClientKey::restore(secret)?;
    let role = py.detach(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(Error::Thread)?;
        runtime.block_on(key::join(&key, invite, label, address))
    })?;
    Ok(role.code().to_owned())
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

    /// The next update as JSON within `timeout` seconds, or `None`.
    fn next_update(&self, py: Python<'_>, timeout: f64) -> PyResult<Option<String>> {
        let wait = Duration::try_from_secs_f64(timeout.max(0.0)).unwrap_or(Duration::ZERO);
        py.detach(|| self.0.next_update(wait))
            .map(|update| json(&update))
            .transpose()
    }

    /// Start a feed; how it ended, as JSON. Never retry an `uncertain` one.
    fn feed(&self, py: Python<'_>, portions: u8) -> PyResult<String> {
        json(&py.detach(|| self.0.feed(portions)))
    }

    /// Lock or unlock the panel; how it ended, as JSON.
    fn set_lock(&self, py: Python<'_>, locked: bool) -> PyResult<String> {
        json(&py.detach(|| self.0.set_lock(locked)))
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
}
