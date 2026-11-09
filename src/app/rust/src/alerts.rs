//! What the phone keeps of its feeders' alerts, in a file of its own
//! (`alerts.json` beside `wellfed.json`): each feeder's [`Watch`] (the cursor
//! and epoch its table was read to, the notices open and those still to post
//! or take off, how the phone last reached it), and this phone's settings for
//! alerts: the mutes and "instant while home". Its own file, so one that does
//! not read is set aside on its own ([`crate::stored`]) and can never keep the
//! app from its feeders.
//!
//! Every engine in the process (the app's, the periodic catch-up's and the
//! instant service's) shares this library, so [`FILE`] is the one writer: each
//! change loads, changes and saves the file under it. A catch-up holds it only
//! for those steps, never across the network; [`SITTING`] keeps two catch-ups
//! off one feeder, and a second is skipped rather than doubled.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex as Blocking, PoisonError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use wellfed_client::alerts::{Mutes, Watch};

use crate::api::alerts::AlertHold;
use crate::api::error::BridgeError;
use crate::api::store::{SetAside, StoreKind};
use crate::stored::{self, Opened, Versioned};

/// The file's name in the app's directory.
pub(crate) const FILE_NAME: &str = "alerts.json";

/// The alert state the phone keeps.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AlertState {
    pub(crate) settings: AlertSettings,
    /// One entry per feeder this phone has caught up with.
    pub(crate) feeders: Vec<Watched>,
    /// The file this one replaced when that did not read, kept until the
    /// owner has read the notice: a catch-up in the background may be the
    /// first to open it, long before the app shows anything.
    pub(crate) set_aside: Option<SetAside>,
}

impl Versioned for AlertState {
    const VERSION: u32 = 1;
}

/// How this phone takes alerts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AlertSettings {
    /// "Instant while home": a foreground service catches up every minute.
    pub(crate) instant: bool,
    pub(crate) mutes: Mutes,
}

/// One feeder's watch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Watched {
    /// The feeder's identity key, lowercase hex, as `wellfed.json` names it.
    pub(crate) identity: String,
    /// Where a catch-up last reached it (`ip:port`). The app's own file keeps
    /// where the app last did; only the app writes that one.
    pub(crate) address: Option<String>,
    pub(crate) watch: Watch,
}

impl AlertState {
    pub(crate) fn watched(&self, identity: &str) -> Option<&Watched> {
        self.feeders
            .iter()
            .find(|watched| watched.identity == identity)
    }

    /// The feeder's entry, made empty if it has none.
    pub(crate) fn watched_mut(&mut self, identity: &str) -> &mut Watched {
        let at = match self
            .feeders
            .iter()
            .position(|watched| watched.identity == identity)
        {
            Some(at) => at,
            None => {
                self.feeders.push(Watched {
                    identity: identity.to_owned(),
                    address: None,
                    watch: Watch::default(),
                });
                self.feeders.len() - 1
            }
        };
        &mut self.feeders[at]
    }
}

/// The one writer of `alerts.json` in this process.
static FILE: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

pub(crate) fn path(dir: &Path) -> PathBuf {
    dir.join(FILE_NAME)
}

/// The state at `path` as any engine's first open reads it, with a file that
/// did not read set aside and named in the fresh one.
fn opened(path: &Path) -> Result<AlertState, BridgeError> {
    let fresh = |set_aside: &SetAside| AlertState {
        set_aside: Some(set_aside.clone()),
        ..AlertState::default()
    };
    Ok(match stored::open(path, StoreKind::Alerts, fresh)? {
        Opened::Absent => AlertState::default(),
        Opened::Read(state) | Opened::SetAside { value: state, .. } => state,
    })
}

/// Read the state in `dir`, change it with `change`, and save it if it
/// changed: one at a time across the process.
pub(crate) async fn change<T>(
    dir: &Path,
    change: impl FnOnce(&mut AlertState) -> T,
) -> Result<T, BridgeError> {
    let _one = FILE.lock().await;
    let path = path(dir);
    let mut state = opened(&path)?;
    let before = state.clone();
    let result = change(&mut state);
    if state != before {
        stored::write(&path, &state)?;
    }
    Ok(result)
}

/// The state in `dir`, read under the same lock.
pub(crate) async fn read(dir: &Path) -> Result<AlertState, BridgeError> {
    change(dir, |state| state.clone()).await
}

/// Feeders with a catch-up running now.
static SITTING: LazyLock<Blocking<HashSet<[u8; 32]>>> =
    LazyLock::new(|| Blocking::new(HashSet::new()));

/// A catch-up's claim on its feeder, let go when dropped.
pub(crate) struct Sitting([u8; 32]);

impl Sitting {
    /// The feeder's claim, or `None` while another catch-up holds it.
    pub(crate) fn take(feeder: [u8; 32]) -> Option<Self> {
        // A set of plain values is whole even if a holder panicked.
        let mut sitting = SITTING.lock().unwrap_or_else(PoisonError::into_inner);
        // Made only when the claim is taken: a claim dropped here would
        // free the running catch-up's, and its drop takes this same lock.
        sitting.insert(feeder).then(|| Self(feeder))
    }
}

impl Drop for Sitting {
    fn drop(&mut self) {
        let mut sitting = SITTING.lock().unwrap_or_else(PoisonError::into_inner);
        sitting.remove(&self.0);
    }
}

/// How long a run's hold lasts unrenewed. A run ends its hold when it ends,
/// but an app swiped away mid-run says nothing while this process lives on,
/// so a hold lapses by itself. The longest run is a stock install with its
/// 30-minute listener.
const HOLD_LIMIT: Duration = Duration::from_secs(2 * 60 * 60);

static HELD: Blocking<BTreeMap<AlertHold, Instant>> = Blocking::new(BTreeMap::new());

fn holds() -> std::sync::MutexGuard<'static, BTreeMap<AlertHold, Instant>> {
    let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
    held.retain(|_, at| at.elapsed() < HOLD_LIMIT);
    held
}

/// Keep catch-ups off for `run`, or let go.
pub(crate) fn hold(run: AlertHold, held: bool) {
    let mut holds = holds();
    if held {
        holds.insert(run, Instant::now());
    } else {
        holds.remove(&run);
    }
}

/// Whether a run keeps catch-ups off.
pub(crate) fn held() -> bool {
    !holds().is_empty()
}

#[cfg(test)]
mod tests {
    use wellfed_client::alerts::NoticeKind;

    use super::*;

    #[test]
    fn a_damaged_alerts_file_is_set_aside_on_its_own_and_named() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            assert_eq!(read(dir.path()).await.unwrap(), AlertState::default());
            std::fs::write(
                path(dir.path()),
                b"{\"version\": 1, \"data\": {\"mutes\": 3}}",
            )
            .unwrap();

            let state = read(dir.path()).await.unwrap();
            let set_aside = state.set_aside.unwrap();
            assert_eq!(set_aside.kind, StoreKind::Alerts);
            assert!(dir.path().join("alerts.json.damaged-1").exists());
            assert_eq!(state.feeders, []);

            // Kept in the file, so the app says so even if a background
            // catch-up opened it first.
            assert!(read(dir.path()).await.unwrap().set_aside.is_some());
        });
    }

    #[test]
    fn a_change_saves_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            change(dir.path(), |state| {
                state.settings.instant = true;
                state.settings.mutes.kinds.push(NoticeKind::FoodLow);
                state.watched_mut("ab").address = Some("10.0.0.2:4433".into());
            })
            .await
            .unwrap();
            let state = read(dir.path()).await.unwrap();
            assert!(state.settings.instant);
            assert_eq!(state.settings.mutes.kinds, [NoticeKind::FoodLow]);
            assert_eq!(
                state
                    .watched("ab")
                    .and_then(|watched| watched.address.as_deref()),
                Some("10.0.0.2:4433")
            );
        });
    }

    #[test]
    fn one_catch_up_at_a_time_per_feeder() {
        let first = Sitting::take([1; 32]).unwrap();
        assert!(Sitting::take([1; 32]).is_none());
        assert!(Sitting::take([2; 32]).is_some());
        drop(first);
        assert!(Sitting::take([1; 32]).is_some());
    }
}
