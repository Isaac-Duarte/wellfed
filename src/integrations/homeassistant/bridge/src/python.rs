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

use uuid::Uuid;
use wellfed_server_client::api::FeederId;

use crate::api::{ServerProblem, Tune, Update, Written};
use crate::feeder::{self, Clocks, Target};
use crate::key::{self, ClientKey, Joined, bytes32};
use crate::server::{self, Login, ServerFeeder};

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

/// The owner's server refused a setup call, or could not be reached;
/// `problem` says how, as the setup flow words it.
#[pyclass(extends = WellfedError, frozen, module = "wellfed_homeassistant")]
struct ServerRefused {
    #[pyo3(get)]
    problem: ServerProblem,
}

#[pymethods]
impl ServerRefused {
    #[new]
    #[pyo3(signature = (problem, *_args))]
    fn new(problem: &ServerProblem, _args: &Bound<'_, PyTuple>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(WellfedError).add_subclass(Self { problem: *problem })
    }
}

impl From<server::Refused> for PyErr {
    fn from(refused: server::Refused) -> Self {
        PyErr::new::<ServerRefused, _>((refused.problem(), refused.to_string()))
    }
}

/// Run `future` to its end on a runtime of its own.
fn block_on<T, E>(future: impl Future<Output = Result<T, E>>) -> PyResult<T>
where
    PyErr: From<E>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(Error::Thread)?;
    Ok(runtime.block_on(future)?)
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
    py.detach(|| block_on(key::join_member(&key, invite, label, address)))
}

/// Sign in to the owner's server at `url` as `name`, and create a token
/// labelled `label` for Home Assistant. The password is used for this alone
/// and not kept; an account that may change nothing is refused.
#[pyfunction]
fn server_login(
    py: Python<'_>,
    url: &str,
    name: &str,
    password: &str,
    label: &str,
) -> PyResult<Login> {
    py.detach(|| block_on(server::login(url, name, password, label)))
}

/// The feeders the server at `url` holds, as `token` sees them.
#[pyfunction]
fn server_feeders(py: Python<'_>, url: &str, token: &str) -> PyResult<Vec<ServerFeeder>> {
    py.detach(|| block_on(server::feeders(url, token)))
}

/// Revoke the token `token_id` with itself.
#[pyfunction]
fn server_revoke_token(py: Python<'_>, url: &str, token: &str, token_id: &str) -> PyResult<()> {
    py.detach(|| block_on(server::revoke_token(url, token, token_id)))
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

    /// The session to the feeder `feeder_id` on the owner's server at
    /// `url`, signed in with `token`; `identity` is the feeder's, and a
    /// feeder under that id with another one is never served as this one.
    #[staticmethod]
    fn server(url: &str, token: String, feeder_id: &str, identity: &[u8]) -> PyResult<Self> {
        let target = server::Target {
            url: server::address(url)?,
            token,
            feeder: FeederId(
                Uuid::parse_str(feeder_id).map_err(|_| Error::FeederId(feeder_id.to_owned()))?,
            ),
            identity: bytes32(identity, Error::IdentityLength)?,
        };
        Ok(Self(feeder::Feeder::start_server(
            target,
            server::Clocks::DEFAULT,
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
    use super::{
        Feeder, ServerRefused, WellfedError, generate_secret, invite_feeder, join, public_key,
        server_feeders, server_login, server_revoke_token,
    };
    #[pymodule_export]
    use crate::api::{
        AccountRole, By, Capabilities, Event, EventKind, Feed, FeedFault, FeedOutcome, FeedReason,
        FoodLevel, NextMeal, NotSent, Plan, PortionsToday, Readiness, RefusalKind, Role,
        ServerProblem, Settings, Skew, State, Status, Tune, Update, UpdateKind, WriteResult,
        Written,
    };
    #[pymodule_export]
    use crate::key::Joined;
    #[pymodule_export]
    use crate::server::{Login, ServerFeeder};
}
