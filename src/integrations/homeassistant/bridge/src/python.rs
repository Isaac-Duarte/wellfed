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

use crate::api::{JoinProblem, Pending, Tune, Update, Written};
use crate::feeder::{self, Carried, Clocks, Target};
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

/// Redeeming an invite did not enroll Home Assistant's key; `problem` says
/// how, as the setup flow words it, and `held` is the key the next `redeem`
/// must be handed, if any.
#[pyclass(extends = WellfedError, frozen, module = "wellfed_homeassistant")]
struct JoinFailed {
    #[pyo3(get)]
    problem: JoinProblem,
    #[pyo3(get)]
    held: Option<HeldKey>,
}

#[pymethods]
impl JoinFailed {
    #[new]
    #[pyo3(signature = (problem, held, *_args))]
    fn new(
        problem: &JoinProblem,
        held: Option<HeldKey>,
        _args: &Bound<'_, PyTuple>,
    ) -> PyClassInitializer<Self> {
        PyClassInitializer::from(WellfedError).add_subclass(Self {
            problem: *problem,
            held,
        })
    }
}

impl From<Box<key::Unredeemed>> for PyErr {
    fn from(unredeemed: Box<key::Unredeemed>) -> Self {
        let problem = key::join_problem(&unredeemed.error);
        let held = unredeemed.held.map(HeldKey::from);
        PyErr::new::<JoinFailed, _>((problem, held, unredeemed.error.to_string()))
    }
}

/// A key an unchecked join may have enrolled on `feeder`: the setup flow
/// holds it and hands it to the next `redeem`, which checks it before it
/// spends another invite for that feeder.
#[pyclass(frozen, get_all, from_py_object, module = "wellfed_homeassistant")]
#[derive(Clone)]
struct HeldKey {
    secret: Vec<u8>,
    feeder: Vec<u8>,
}

#[pymethods]
impl HeldKey {
    #[new]
    fn new(secret: Vec<u8>, feeder: Vec<u8>) -> Self {
        Self { secret, feeder }
    }

    /// The public key, as the feeder would list it.
    fn public_key(&self) -> PyResult<Vec<u8>> {
        Ok(ClientKey::restore(&self.secret)?.public().to_vec())
    }
}

impl From<key::Unsettled> for HeldKey {
    fn from(held: key::Unsettled) -> Self {
        Self {
            secret: held.secret.to_vec(),
            feeder: held.feeder.to_vec(),
        }
    }
}

impl TryFrom<&HeldKey> for key::Unsettled {
    type Error = Error;

    fn try_from(held: &HeldKey) -> Result<Self, Error> {
        Ok(Self {
            secret: bytes32(&held.secret, Error::SecretLength)?,
            feeder: bytes32(&held.feeder, Error::IdentityLength)?,
        })
    }
}

/// A key an invite enrolled: its secret and its feeder, how it joined, and
/// the key still held for the next `redeem`, if any.
#[pyclass(frozen, get_all, module = "wellfed_homeassistant")]
struct Redeemed {
    secret: Vec<u8>,
    feeder: Vec<u8>,
    joined: Joined,
    held: Option<HeldKey>,
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

/// Join with `invite`, listed as `label`, keeping the key only as a member:
/// any other role is revoked at once. `held` is the key an earlier unchecked
/// join left: for this invite's feeder it is checked first, and the invite,
/// which works once, is spent only when the feeder does not know it. A join
/// that did not enroll a key raises `JoinFailed`, whose `held` is the key
/// to hand the next call.
#[pyfunction]
#[pyo3(signature = (invite, label, address=None, held=None))]
fn redeem(
    py: Python<'_>,
    invite: &str,
    label: &str,
    address: Option<&str>,
    held: Option<&Bound<'_, HeldKey>>,
) -> PyResult<Redeemed> {
    let held = held
        .map(|held| key::Unsettled::try_from(held.get()))
        .transpose()?;
    let redeemed = py.detach(|| block_on(key::redeem(invite, label, address, held)))?;
    Ok(Redeemed {
        secret: redeemed.key.secret().to_vec(),
        feeder: redeemed.feeder.to_vec(),
        joined: redeemed.joined,
        held: redeemed.held.map(HeldKey::from),
    })
}

#[pymethods]
impl Pending {
    /// A pending feed as the integration kept it, to hand back to a new
    /// session.
    #[new]
    #[pyo3(signature = (id, portions, sent_ms, after, reported, released_ms))]
    fn new(
        id: String,
        portions: u8,
        sent_ms: i64,
        after: Option<u64>,
        reported: bool,
        released_ms: Option<i64>,
    ) -> Self {
        Self {
            id,
            portions,
            sent_ms,
            after,
            reported,
            released_ms,
        }
    }
}

/// One feeder's session, on a thread of its own until this is dropped.
#[pyclass(frozen)]
struct Feeder(feeder::Feeder);

#[pymethods]
impl Feeder {
    /// `pending` and `answered` are what the last run left: the feeds still
    /// unsettled, and the feed numbers the last `Answered` update listed.
    /// `unread` says the integration's store did not read, and the owner
    /// has not yet checked the bowls: every feed waits until
    /// `bowls_checked`.
    #[new]
    #[pyo3(signature = (
        secret, identity, address=None, pending=Vec::new(), answered=Vec::new(), unread=false
    ))]
    fn new(
        secret: &[u8],
        identity: &[u8],
        address: Option<String>,
        pending: Vec<Pending>,
        answered: Vec<u32>,
        unread: bool,
    ) -> PyResult<Self> {
        let key = ClientKey::restore(secret)?;
        let identity = bytes32(identity, Error::IdentityLength)?;
        let carried = Carried {
            pending,
            answered,
            unread,
        };
        Ok(Self(feeder::Feeder::start(
            key,
            Target { identity, address },
            Clocks::DEFAULT,
            carried,
        )?))
    }

    /// The next update within `timeout` seconds, or `None`.
    fn next_update(&self, py: Python<'_>, timeout: f64) -> Option<Update> {
        let wait = Duration::try_from_secs_f64(timeout.max(0.0)).unwrap_or(Duration::ZERO);
        py.detach(|| self.0.next_update(wait))
    }

    /// Start a feed; how it ended. Never retry an uncertain one. It goes
    /// out only once `saved` confirms the `Sending` update's record is in
    /// the store.
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

    /// The owner releases the lost feed `id` names, through its repair: new
    /// feeds go again though it may have run. A `Released` update confirms
    /// it.
    fn release(&self, id: String) {
        self.0.release(id);
    }

    /// The owner checked the bowls after the integration's store did not
    /// read, through its repair: feeds go again. A `BowlsChecked` update
    /// confirms it.
    fn bowls_checked(&self) {
        self.0.bowls_checked();
    }

    /// The integration's store saved the record of the feed `id` names, as
    /// a `Sending` update asked: the feed goes out. Called from the event
    /// loop while `feed` blocks an executor thread.
    fn saved(&self, id: &str) {
        self.0.saved(id);
    }

    /// The integration's store could not save the record of the feed `id`
    /// names: it never goes out, and `feed` answers it not sent.
    fn unsaved(&self, id: &str) {
        self.0.unsaved(id);
    }
}

#[pymodule]
mod wellfed_homeassistant {
    #[pymodule_export]
    use super::{
        Feeder, HeldKey, JoinFailed, Redeemed, WellfedError, invite_feeder, public_key, redeem,
    };
    #[pymodule_export]
    use crate::api::{
        Alert, AlertKind, AlertLevel, Alerts, By, Capabilities, Event, EventKind, Feed, FeedFault,
        FeedOutcome, FeedReason, FoodLevel, JoinProblem, NextMeal, NotSent, Pending, Plan,
        PortionsToday, Readiness, RefusalKind, Role, Settings, Skew, State, Status, Tune, Update,
        UpdateKind, WriteResult, Written,
    };
    #[pymodule_export]
    use crate::key::Joined;

    // The feeder's limits, from the model the bridge checks writes against,
    // so the integration's entities and service schema never hold a copy.

    /// The fewest portions one feed asks for.
    #[pymodule_export]
    const PORTIONS_MIN: u8 = wellfed_model::PORTIONS_MIN;
    /// The most portions one feed asks for.
    #[pymodule_export]
    const PORTIONS_MAX: u8 = wellfed_model::PORTIONS_MAX;
    /// The lightest portion weight the feeder stores, in milligrams.
    #[pymodule_export]
    const PORTION_MG_MIN: u32 = wellfed_model::PORTION_MG_MIN;
    /// The heaviest portion weight the feeder stores, in milligrams.
    #[pymodule_export]
    const PORTION_MG_MAX: u32 = wellfed_model::PORTION_MG_MAX;
}
