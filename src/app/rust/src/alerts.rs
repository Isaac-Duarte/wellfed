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
use wellfed_client::alerts::{Change, Mutes, Watch};

use crate::api::alerts::AlertHold;
use crate::api::error::BridgeError;
use crate::api::store::{self, SetAside, StoreKind};
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
    /// The file was set aside, so notifications its watches posted may still
    /// be up with nothing that knows their ids: the app clears every alert
    /// notification, then [`AlertState::sweep`] goes back to `false`.
    pub(crate) sweep: bool,
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
    /// The app has asked Android once, unprompted, to let it post: it asks
    /// again only when the owner taps to.
    pub(crate) asked: bool,
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

    pub(crate) fn watched_mut(&mut self, identity: &str) -> Option<&mut Watched> {
        self.feeders
            .iter_mut()
            .find(|watched| watched.identity == identity)
    }

    /// The feeder's entry for a catch-up, made empty if it has none, or
    /// `None` when the phone no longer knows the feeder: a catch-up that
    /// outlived its feeder must not bring its alerts back.
    fn entry(&mut self, identity: &str, known: bool) -> Option<&mut Watched> {
        if !known {
            return None;
        }
        if self.watched(identity).is_none() {
            self.feeders.push(Watched {
                identity: identity.to_owned(),
                address: None,
                watch: Watch::default(),
            });
        }
        self.watched_mut(identity)
    }

    /// Drop the feeder's entry: a removal for every notice it may have up.
    pub(crate) fn forget(&mut self, identity: &str) -> Vec<Change> {
        let removals = self
            .watched(identity)
            .map(|watched| watched.watch.every_key().map(Change::Remove).collect())
            .unwrap_or_default();
        self.feeders.retain(|watched| watched.identity != identity);
        removals
    }

    /// Change the feeder `identity`'s watch for a catch-up, if this phone
    /// still knows the feeder (`wellfed.json` in `dir` lists it): the changes
    /// to make. A feeder forgotten meanwhile is dropped here too, and every
    /// notification it may have up comes off.
    pub(crate) fn catch_up(
        &mut self,
        dir: &Path,
        identity: &str,
        apply: impl FnOnce(&mut Watched, &Mutes),
    ) -> Result<Vec<Change>, BridgeError> {
        let known = known(dir, identity)?;
        let mutes = self.settings.mutes.clone();
        let Some(watched) = self.entry(identity, known) else {
            return Ok(self.forget(identity));
        };
        apply(watched, &mutes);
        Ok(watched.watch.changes())
    }
}

/// Whether the app's own file in `dir` lists the feeder `identity`. The app
/// saves a feeder there before it ever catches up with it, and drops it there
/// before it forgets its alerts.
fn known(dir: &Path, identity: &str) -> Result<bool, BridgeError> {
    let path = dir.join(store::FILE_NAME);
    let store = store::load_store(&path.to_string_lossy())?;
    Ok(store
        .feeders
        .iter()
        .any(|feeder| feeder.identity.eq_ignore_ascii_case(identity)))
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
        sweep: true,
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

/// Let go of every run's hold: the app's task was swiped away, so no run of
/// its engine is left to end its own.
#[cfg_attr(
    not(any(target_os = "android", test)),
    expect(dead_code, reason = "only Android's keep-alive service calls it")
)]
pub(crate) fn clear_holds() {
    holds().clear();
}

/// Whether a run keeps catch-ups off.
pub(crate) fn held() -> bool {
    !holds().is_empty()
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use wellfed_client::alerts::{Cursor, Delivery, Notice, NoticeKey, NoticeKind, Reach, Tracked};
    use wellfed_model::{
        Alert, AlertLevel, ClientRole, EnrollPath, FeedOutcome, FeedReason, Generation, Hex,
        OpenAlert,
    };

    use super::*;
    use crate::api::store::{AppStore, KnownFeeder, save_store};

    const KITCHEN: &str = "abababababababababababababababababababababababababababababababab";

    fn run<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn know(dir: &Path, identities: &[&str]) {
        let store = AppStore {
            feeders: identities
                .iter()
                .map(|identity| KnownFeeder {
                    identity: (*identity).to_owned(),
                    name: "Kitchen".into(),
                    address: None,
                    manual_address: None,
                })
                .collect(),
            ..AppStore::default()
        };
        save_store(&dir.join(store::FILE_NAME).to_string_lossy(), &store).unwrap();
    }

    fn at(second: i64) -> Timestamp {
        Timestamp::from_second(1_800_000_000 + second).unwrap()
    }

    /// Every field of the file, with a value in each.
    fn every_field() -> AlertState {
        let failed = OpenAlert {
            at: Some(at(0)),
            alert: Alert::MealFailed {
                feed: 7,
                plan: Some(1),
                requested: 2,
                completed: 1,
                outcome: FeedOutcome::Failed,
                reason: FeedReason::Jammed,
            },
        };
        let joined = OpenAlert {
            at: None,
            alert: Alert::ClientJoined {
                client: Hex([0xde, 0xad, 0xbe, 0xef]),
                role: ClientRole::Member,
                path: EnrollPath::Invite,
            },
        };
        let food = OpenAlert {
            at: Some(at(-60)),
            alert: Alert::FoodLow,
        };
        let tracked = |open: OpenAlert, delivery, absent| Tracked {
            key: NoticeKey::Alert {
                epoch: 3,
                key: open.key(),
            },
            notice: Notice::Alert { open },
            seen: at(5),
            delivery,
            absent,
        };
        AlertState {
            settings: AlertSettings {
                instant: true,
                mutes: Mutes {
                    kinds: vec![NoticeKind::FoodLow],
                    levels: vec![AlertLevel::Security],
                },
                asked: true,
            },
            feeders: vec![Watched {
                identity: KITCHEN.into(),
                address: Some("192.168.1.20:4433".into()),
                watch: Watch {
                    cursor: Some(Cursor {
                        epoch: 3,
                        boot: 12,
                        generation: Generation(9),
                    }),
                    open: vec![
                        tracked(failed, Delivery::Posted, None),
                        tracked(joined, Delivery::Quiet, Some(at(30))),
                        Tracked {
                            key: NoticeKey::Unreachable,
                            notice: Notice::Unreachable { since: at(40) },
                            seen: at(50),
                            delivery: Delivery::Due,
                            absent: None,
                        },
                        tracked(food, Delivery::Dismissed, None),
                    ],
                    removing: vec![NoticeKey::Alert {
                        epoch: 2,
                        key: failed.key(),
                    }],
                    unread: 1,
                    reach: Reach {
                        home: Some("5f3a".into()),
                        failures: 2,
                        first_failure: Some(at(60)),
                        reached: Some(at(20)),
                    },
                },
            }],
            set_aside: Some(SetAside {
                kind: StoreKind::Alerts,
                moved_to: "/data/alerts.json.damaged-1".into(),
                reason: "version 2, where this build reads 1".into(),
                at_ms: 1_800_000_000_000,
            }),
            sweep: true,
        }
    }

    /// Pinned: the beta freezes this file's shape, `wellfed_model`'s alert
    /// values inside it included. A change here is a new version with a
    /// reader for this one ([`Versioned::earlier`]), never an edit to the
    /// sample.
    #[test]
    fn version_1_reads_and_writes_as_its_sample() {
        let sample = include_str!("../tests/fixtures/alerts-v1.json");
        let read: AlertState = stored::read(sample.as_bytes()).unwrap();
        assert_eq!(read, every_field());

        let dir = tempfile::tempdir().unwrap();
        stored::write(&path(dir.path()), &every_field()).unwrap();
        let written: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path(dir.path())).unwrap()).unwrap();
        let sample: serde_json::Value = serde_json::from_str(sample).unwrap();
        assert_eq!(written, sample);
    }

    #[test]
    fn a_catch_up_never_brings_back_a_forgotten_feeder() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = every_field();

        // Forgotten while its catch-up was out: what it found is dropped,
        // and everything it may have up comes off.
        know(dir.path(), &[]);
        let mut applied = false;
        let changes = state
            .catch_up(dir.path(), KITCHEN, |_, _| applied = true)
            .unwrap();
        assert!(!applied);
        assert_eq!(state.feeders, []);
        assert_eq!(changes.len(), 5, "four open and one being removed");
        assert!(
            changes
                .iter()
                .all(|change| matches!(change, Change::Remove(_)))
        );

        // Known: its first catch-up makes its entry.
        know(dir.path(), &[KITCHEN]);
        state
            .catch_up(dir.path(), KITCHEN, |watched, _| {
                watched.address = Some("10.0.0.2:4433".into());
            })
            .unwrap();
        assert!(state.watched(KITCHEN).is_some());
    }

    #[test]
    fn confirming_for_a_forgotten_feeder_takes_its_posts_off() {
        let dir = tempfile::tempdir().unwrap();
        let dir_name = dir.path().display().to_string();
        run(async {
            let again = crate::api::alerts::confirm_notices(
                dir_name.clone(),
                KITCHEN.into(),
                vec![0x4000_0001],
                vec![0x4000_0002],
            )
            .await
            .unwrap();
            assert_eq!(again, [0x4000_0001]);
            assert_eq!(read(dir.path()).await.unwrap().feeders, []);

            let dismissed =
                crate::api::alerts::dismiss_notice(dir_name, KITCHEN.into(), 0x4000_0001)
                    .await
                    .unwrap();
            assert_eq!(dismissed, []);
            assert_eq!(read(dir.path()).await.unwrap().feeders, []);
        });
    }

    #[test]
    fn the_app_asks_for_notifications_once_and_posts_what_is_still_due() {
        let dir = tempfile::tempdir().unwrap();
        let dir_name = dir.path().display().to_string();
        run(async {
            let ask = || crate::api::alerts::ask_for_notifications_once(dir_name.clone());
            assert!(ask().await.unwrap());
            assert!(!ask().await.unwrap(), "the owner asks after that");

            let mut state = every_field();
            state.set_aside = None;
            change(dir.path(), |stored| *stored = state).await.unwrap();
            let pending = crate::api::alerts::pending_notices(dir_name.clone())
                .await
                .unwrap();
            let [kitchen] = pending.as_slice() else {
                panic!("{pending:?}");
            };
            // The unreachable notice is due; the posted, quiet and
            // dismissed ones are not posted again, and the removal stays.
            let posts = kitchen
                .changes
                .iter()
                .filter(|change| matches!(change, crate::api::alerts::NoticeChange::Post { .. }))
                .count();
            assert_eq!((kitchen.changes.len(), posts), (2, 1));
        });
    }

    #[test]
    fn a_swipe_lets_go_of_every_hold() {
        hold(AlertHold::Install, true);
        hold(AlertHold::Restore, true);
        assert!(held());
        clear_holds();
        assert!(!held());
    }

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
            assert!(state.sweep, "what it posted comes off");

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
                state.feeders.push(Watched {
                    identity: "ab".into(),
                    address: Some("10.0.0.2:4433".into()),
                    watch: Watch::default(),
                });
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
