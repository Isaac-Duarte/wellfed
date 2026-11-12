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

use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;
use wellfed_client::alerts::{Change, Mutes, Watch};
use wellfed_formats::kept;

use crate::api::alerts::{AlertHold, PostedNotice};
use crate::api::error::BridgeError;
use crate::api::store::{self, SetAside, StoreKind};
use crate::stored::{self, Opened, Versioned};

/// The file's name in the app's directory.
pub(crate) const FILE_NAME: &str = "alerts.json";

/// The alert state the phone keeps.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AlertState {
    pub(crate) settings: AlertSettings,
    /// One entry per feeder this phone has caught up with.
    pub(crate) feeders: Vec<Watched>,
    /// The file this one replaced when that did not read, kept until the
    /// owner has read the notice: a catch-up in the background may be the
    /// first to open it, long before the app shows anything.
    pub(crate) set_aside: Option<SetAside>,
    /// The file was set aside, so notifications its watches posted may still
    /// be up with nothing that knows their ids: the app takes those off,
    /// then [`AlertState::sweep`] goes back to `false`.
    pub(crate) sweep: bool,
    /// The ids a sweep chose to take off, until it says they are off or
    /// [`SWEEP_HOLD`] has passed. A post of one meanwhile isn't confirmed,
    /// so the next catch-up hands it over again after the sweep took it
    /// down.
    pub(crate) sweeping: Vec<SweepHold>,
}

/// How long a sweep's choice holds a post back. A sweep takes a few
/// notifications off in well under a second; one killed before it said so
/// lets go by itself, so a post is never held back for good.
pub(crate) const SWEEP_HOLD: SignedDuration = SignedDuration::from_mins(1);

/// One notification a sweep is taking off, since `since` by the phone's
/// clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SweepHold {
    pub(crate) id: i32,
    pub(crate) since: Timestamp,
}

impl Versioned for AlertState {
    const VERSION: u32 = 1;

    /// What a newer minor added: the settings and their mutes, each
    /// feeder's entry by identity with its watch and reach, each sweep hold
    /// by id, and the notice while it names the same file. A watch's
    /// notices hold tagged enums and are not carried into.
    fn carry(old: &Value, new: &mut Value) {
        kept::carry(old, new);
        if let Some((old, new)) = kept::both(old, new, "/settings") {
            kept::carry(old, new);
            if let Some((old, new)) = kept::both(old, new, "/mutes") {
                kept::carry(old, new);
            }
        }
        for (old, new) in kept::pairs(old, new, "/feeders", &["/identity"]) {
            kept::carry(old, new);
            if let Some((old, new)) = kept::both(old, new, "/watch") {
                kept::carry(old, new);
                if let Some((old, new)) = kept::both(old, new, "/reach") {
                    kept::carry(old, new);
                }
            }
        }
        for (old, new) in kept::pairs(old, new, "/sweeping", &["/id"]) {
            kept::carry(old, new);
        }
        store::carry_set_aside(old, new);
    }
}

/// How this phone takes alerts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AlertSettings {
    /// "Instant while home": a foreground service catches up every minute.
    pub(crate) instant: bool,
    pub(crate) mutes: Mutes,
    /// The app has asked Android once, unprompted, to let it post: it asks
    /// again only when the owner taps to.
    pub(crate) asked: bool,
    /// Android refused a request to post at least once on this install, so
    /// the alert screens open its notification settings rather than ask:
    /// past a refusal, its dialog may never show again.
    pub(crate) refused: bool,
}

/// One feeder's watch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

    /// Hold posts of `ids` back while a sweep takes them off, beside any
    /// other engine's sweep under way.
    pub(crate) fn hold_sweep(&mut self, ids: &[i32], now: Timestamp) {
        self.lapse_sweeps(now);
        for &id in ids {
            if !self.sweeping.iter().any(|held| held.id == id) {
                self.sweeping.push(SweepHold { id, since: now });
            }
        }
    }

    /// A sweep took `ids` off: their posts go through again.
    pub(crate) fn release_sweep(&mut self, ids: &[i32]) {
        self.sweeping.retain(|held| !ids.contains(&held.id));
    }

    /// Let go of each hold older than [`SWEEP_HOLD`], and of one that starts
    /// further ahead than that: the phone's clock stepped back since, and the
    /// hold would otherwise last as long as the step.
    fn lapse_sweeps(&mut self, now: Timestamp) {
        self.sweeping.retain(|held| {
            let held_for = now.duration_since(held.since);
            -SWEEP_HOLD <= held_for && held_for <= SWEEP_HOLD
        });
    }

    /// The changes Dart made for the feeder `identity`: each notice posted
    /// and each removed one's id. The ids to take off again: everything
    /// posted for a feeder forgotten while its changes were on the way. A
    /// post a sweep is taking down stays unconfirmed, and so does one that
    /// is not the posting Dart made: the notice opened again, or changed
    /// kind, after the catch-up handed it over.
    pub(crate) fn confirm(
        &mut self,
        identity: &str,
        feeder: &[u8; 32],
        made: (&[PostedNotice], &[i32]),
        now: Timestamp,
    ) -> Vec<i32> {
        let (posted, removed) = made;
        self.lapse_sweeps(now);
        let sweeping: Vec<i32> = self.sweeping.iter().map(|held| held.id).collect();
        let Some(watched) = self.watched_mut(identity) else {
            return posted.iter().map(|notice| notice.id).collect();
        };
        let watch = &mut watched.watch;
        for change in watch.changes() {
            let done = match &change {
                Change::Post(tracked) => {
                    let id = tracked.key.notification_id(feeder);
                    let made = posted.iter().any(|notice| notice.posts(feeder, tracked));
                    made && !sweeping.contains(&id)
                }
                Change::Remove(key) => removed.contains(&key.notification_id(feeder)),
            };
            if done {
                watch.confirm(&change);
            }
        }
        Vec::new()
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

/// How long a hold lasts. A run lets go when it ends, so this is only for a
/// run that never does: an engine swiped away mid-run whose task ended with
/// nothing said, or a thread that hangs. The longest run is a stock install
/// with its 30-minute listener.
const HOLD_LIMIT: Duration = Duration::from_secs(2 * 60 * 60);

/// Two kinds of hold keep catch-ups off. The engine's, keyed by run, cover
/// what a run does in Dart (joining the feeder's network, the screens before
/// its thread starts) and go with the engine. A run's thread holds its own
/// ([`RunThread`]) for as long as it lives: an update's chunks, the
/// give-way and the bank-0 restart, or the install listener's wait, all of
/// which outlive an engine swiped away.
struct Holds {
    engine: BTreeMap<AlertHold, Instant>,
    threads: BTreeMap<u64, Instant>,
    next: u64,
}

impl Holds {
    fn lapse(&mut self) {
        self.engine.retain(|_, at| at.elapsed() < HOLD_LIMIT);
        self.threads.retain(|_, at| at.elapsed() < HOLD_LIMIT);
    }
}

static HELD: Blocking<Holds> = Blocking::new(Holds {
    engine: BTreeMap::new(),
    threads: BTreeMap::new(),
    next: 0,
});

fn holds() -> std::sync::MutexGuard<'static, Holds> {
    // Plain maps are whole even if a holder panicked.
    let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
    held.lapse();
    held
}

/// Keep catch-ups off for the engine's `run`, or let go.
pub(crate) fn hold(run: AlertHold, held: bool) {
    let mut holds = holds();
    if held {
        holds.engine.insert(run, Instant::now());
    } else {
        holds.engine.remove(&run);
    }
}

/// A run's thread keeping catch-ups off while it lives. Taken before the
/// thread is spawned and moved into it, so it drops when the thread ends,
/// by its end, an error or a panic, and with the closure if no thread
/// started.
pub(crate) struct RunThread(u64);

impl RunThread {
    pub(crate) fn hold() -> Self {
        let mut holds = holds();
        let id = holds.next;
        holds.next = holds.next.wrapping_add(1);
        holds.threads.insert(id, Instant::now());
        Self(id)
    }
}

impl Drop for RunThread {
    fn drop(&mut self) {
        holds().threads.remove(&self.0);
    }
}

/// Let go of the engine's holds: the app's task was swiped away, and the
/// engine went with it, so none of its runs is left to let go. A run's
/// thread still alive keeps its own.
#[cfg_attr(
    not(any(target_os = "android", test)),
    expect(dead_code, reason = "only Android's keep-alive service calls it")
)]
pub(crate) fn clear_holds() {
    holds().engine.clear();
}

/// Whether a run keeps catch-ups off.
pub(crate) fn held() -> bool {
    let holds = holds();
    !holds.engine.is_empty() || !holds.threads.is_empty()
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use std::collections::BTreeSet;
    use wellfed_client::alerts::{
        Absence, Context, Cursor, Delivery, FeederClock, Notice, NoticeKey, NoticeKind, Reach,
        Tracked,
    };

    use serde::Serialize;
    use wellfed_model::{
        Alert, AlertKey, AlertKind, AlertLevel, AlertsView, Blocked, ClientRole, EnrollPath,
        FeedOutcome, FeedReason, Generation, Hex, MissedReason, OpenAlert, Source, UpdateStop,
    };

    use super::*;
    use crate::api::store::{AppStore, KnownFeeder, swap_store};

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
        let path = dir.join(store::FILE_NAME);
        let path = path.to_string_lossy();
        assert!(swap_store(&path, &AppStore::default(), &store).unwrap());
    }

    fn at(second: i64) -> Timestamp {
        Timestamp::from_second(1_800_000_000 + second).unwrap()
    }

    /// Every field of the file, with a value in each.
    /// One open alert per `Alert` variant, and enough more that every value
    /// of each enum inside them (outcome, reason, missed reason, blocked
    /// reason, update stop, role, enrollment, source) is written, and every
    /// alert key. Unknown values are never written, so none is here.
    fn every_alert() -> Vec<OpenAlert> {
        let open = |second: i64, alert| OpenAlert {
            at: Some(at(second)),
            alert,
        };
        let mut alerts = Vec::new();
        let mut feed = 0;
        for (n, outcome) in FeedOutcome::ALL.into_iter().enumerate() {
            feed += 1;
            alerts.push(open(
                feed,
                Alert::MealFailed {
                    feed: u32::try_from(feed).unwrap(),
                    plan: (n % 2 == 0).then_some(1),
                    requested: 2,
                    completed: 1,
                    outcome,
                    reason: FeedReason::Jammed,
                },
            ));
        }
        for reason in FeedReason::ALL {
            feed += 1;
            alerts.push(open(
                feed,
                Alert::MealUncertain {
                    feed: u32::try_from(feed).unwrap(),
                    plan: None,
                    requested: 3,
                    completed: 0,
                    reason,
                },
            ));
        }
        let missed = [MissedReason::Late, MissedReason::Interrupted]
            .into_iter()
            .chain(Blocked::ALL.map(|reason| MissedReason::Blocked { reason }));
        for (plan, reason) in missed.enumerate() {
            let plan = u8::try_from(plan).unwrap();
            alerts.push(open(
                100,
                Alert::MealMissed {
                    plan,
                    due: at(-3600 - i64::from(plan)),
                    reason,
                },
            ));
        }
        alerts.push(open(
            101,
            Alert::MealNoRecord {
                plan: 20,
                due: at(-60),
            },
        ));
        alerts.push(open(102, Alert::PowerWasOut));
        alerts.push(open(103, Alert::RolledBack));
        let stops = [
            UpdateStop::Timeout,
            UpdateStop::Cancelled,
            UpdateStop::Order,
            UpdateStop::Marker,
            UpdateStop::Write,
            UpdateStop::Hash,
            UpdateStop::Activate,
            UpdateStop::Store,
        ];
        for (n, stop) in (0..).zip(stops) {
            alerts.push(open(200 + n, Alert::UpdateFailed { stop }));
        }
        let joins = [
            (ClientRole::Owner, EnrollPath::Setup),
            (ClientRole::Member, EnrollPath::Invite),
            (ClientRole::Server, EnrollPath::Recovery),
            (ClientRole::UpdateSigner, EnrollPath::Press),
        ];
        for (n, (role, path)) in (0..).zip(joins) {
            alerts.push(OpenAlert {
                // A join on a feeder with no clock has no time.
                at: (n > 0).then(|| at(300 + i64::from(n))),
                alert: Alert::ClientJoined {
                    client: Hex([0xde, 0xad, 0xbe, n]),
                    role,
                    path,
                },
            });
        }
        let sources = [
            Source::Panel,
            Source::Client {
                client: Hex([1, 2, 3, 4]),
            },
            Source::Feeder,
            Source::Schedule { plan: 2 },
        ];
        for (n, by) in (0..).zip(sources) {
            alerts.push(open(
                400,
                Alert::ClientRevoked {
                    client: Hex([0xbe, 0xef, 0, n]),
                    by,
                },
            ));
        }
        for alert in [
            Alert::RepeatedRestarts,
            Alert::NoClock,
            Alert::CannotFeed {
                reason: Blocked::Fault,
            },
            Alert::FoodLow,
            Alert::FoodLikelyOut { confirmed: 3 },
            Alert::DesiccantDue { days: -2 },
            Alert::BatteryBelowFloor,
            Alert::PlansStuck,
        ] {
            alerts.push(open(-60, alert));
        }
        alerts
    }

    /// Every field of the file, with a value in each, and every value of
    /// each enum it holds.
    static MUTES: Mutes = Mutes {
        kinds: Vec::new(),
        levels: Vec::new(),
    };

    /// The posting Dart hands back for `tracked`, as its notice view says.
    fn posting(feeder: &[u8; 32], tracked: &Tracked) -> PostedNotice {
        PostedNotice {
            id: tracked.key.notification_id(feeder),
            seen_ms: tracked.seen.as_millisecond(),
            kind: tracked.notice.kind(),
        }
    }

    fn watch(state: &mut AlertState) -> &mut Watch {
        &mut state.feeders[0].watch
    }

    /// Dart posts a notice while another engine's catch-up finds it ended
    /// and open again: the late confirmation is of the old posting, and the
    /// new one is still posted.
    #[test]
    fn a_late_confirmation_leaves_a_notice_opened_again_meanwhile_due() {
        let feeder =
            wellfed_session::hex::decode_array_either_case::<32>(KITCHEN.as_bytes()).unwrap();
        let context = |now| Context {
            now,
            mutes: &MUTES,
            own: Hex([0; 4]),
            clock: FeederClock::Known,
        };
        let table = |generation, alerts: &[OpenAlert]| {
            let cursor = Cursor {
                epoch: 3,
                boot: 1,
                generation: Generation(generation),
            };
            let view = AlertsView {
                epoch: 3,
                generation: Generation(generation),
                more: false,
                settling: false,
                alerts: alerts.to_vec(),
                unread: 0,
            };
            (cursor, view)
        };
        let food = OpenAlert {
            at: None,
            alert: Alert::FoodLow,
        };
        let mut state = AlertState::default();
        state.feeders.push(Watched {
            identity: KITCHEN.into(),
            address: None,
            watch: Watch::default(),
        });

        let (cursor, view) = table(1, &[food]);
        watch(&mut state).apply(cursor, &view, &context(at(10)));
        let Change::Post(on_the_way) = watch(&mut state).changes()[0] else {
            panic!("a post");
        };
        let posted = [posting(&feeder, &on_the_way)];

        let (cursor, view) = table(2, &[]);
        watch(&mut state).apply(cursor, &view, &context(at(20)));
        let (cursor, view) = table(3, &[food]);
        watch(&mut state).apply(cursor, &view, &context(at(30)));
        state.confirm(KITCHEN, &feeder, (&posted, &[]), at(40));

        let changes = watch(&mut state).changes();
        assert!(
            matches!(changes[..], [Change::Remove(_), Change::Post(again)] if again.seen == at(30)),
            "the new opening is still to post: {changes:?}"
        );
    }

    fn every_field() -> AlertState {
        let deliveries = [
            Delivery::Due,
            Delivery::Posted,
            Delivery::Quiet,
            Delivery::Dismissed,
        ];
        let mut open: Vec<Tracked> = every_alert()
            .into_iter()
            .zip((0..).zip(deliveries.into_iter().cycle()))
            .map(|(open, (n, delivery))| Tracked {
                key: NoticeKey::Alert {
                    epoch: 3,
                    key: open.key(),
                },
                notice: Notice::Alert { open },
                seen: at(5 + n),
                delivery,
                absent: match n % 3 {
                    0 => Some(Absence::Since { since: at(30 + n) }),
                    1 => Some(Absence::NoClock),
                    _ => None,
                },
            })
            .collect();
        open.push(Tracked {
            key: NoticeKey::Unreachable,
            notice: Notice::Unreachable { since: at(40) },
            seen: at(50),
            delivery: Delivery::Due,
            absent: None,
        });
        let removing = vec![open[0].key, NoticeKey::Unreachable];

        AlertState {
            settings: AlertSettings {
                instant: true,
                mutes: Mutes {
                    kinds: NoticeKind::all().collect(),
                    levels: vec![AlertLevel::Urgent, AlertLevel::Soon, AlertLevel::Security],
                },
                asked: true,
                refused: true,
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
                    open,
                    removing,
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
            sweeping: vec![SweepHold {
                id: 0x4000_0abc,
                since: at(70),
            }],
        }
    }

    /// Every value `T`'s schema names, as written: the model derives the
    /// schema from the enum itself, so a new variant is named here as soon
    /// as it exists. `Unknown` stands for a newer build's value and is never
    /// written.
    fn every_name<T: utoipa::PartialSchema>() -> BTreeSet<String> {
        fn walk(value: &serde_json::Value, names: &mut BTreeSet<String>) {
            match value {
                serde_json::Value::Object(map) => {
                    if let Some(serde_json::Value::Array(values)) = map.get("enum") {
                        names.extend(values.iter().filter_map(|v| v.as_str().map(str::to_owned)));
                    }
                    map.values().for_each(|value| walk(value, names));
                }
                serde_json::Value::Array(values) => {
                    values.iter().for_each(|value| walk(value, names))
                }
                _ => {}
            }
        }
        let mut names = BTreeSet::new();
        walk(&serde_json::to_value(T::schema()).unwrap(), &mut names);
        names.remove("unknown");
        names
    }

    /// The name `value` is written under: a string, or a tagged object's
    /// `kind`.
    fn name_of(value: impl Serialize) -> String {
        tag_of(value, "kind")
    }

    /// The name `value` is written under: a string, or the tagged object's
    /// `tag`.
    fn tag_of(value: impl Serialize, tag: &str) -> String {
        match serde_json::to_value(value).unwrap() {
            serde_json::Value::String(name) => name,
            serde_json::Value::Object(map) => map[tag].as_str().unwrap().to_owned(),
            other => panic!("{other}"),
        }
    }

    /// Every alert kind and key, and every value of each enum inside an
    /// alert, is in the sample, so a change to how any of them is written
    /// fails [`version_1_reads_and_writes_as_its_sample`].
    #[test]
    fn the_sample_has_every_value() {
        let alerts = every_alert();
        let kinds: BTreeSet<_> = alerts.iter().map(|open| open.alert.kind()).collect();
        assert_eq!(kinds, AlertKind::ALL.into_iter().collect());
        let keys: BTreeSet<_> = alerts
            .iter()
            .map(|open| tag_of(open.key(), "key"))
            .collect();
        assert_eq!(keys, every_name::<AlertKey>());

        let mut found: [BTreeSet<String>; 8] = Default::default();
        for open in &alerts {
            match open.alert {
                Alert::MealFailed {
                    outcome, reason, ..
                } => {
                    found[0].insert(name_of(outcome));
                    found[1].insert(name_of(reason));
                }
                Alert::MealUncertain { reason, .. } => {
                    found[1].insert(name_of(reason));
                }
                Alert::MealMissed { reason, .. } => {
                    if let MissedReason::Blocked { reason } = reason {
                        found[2].insert(name_of(reason));
                    }
                    found[3].insert(name_of(reason));
                }
                Alert::UpdateFailed { stop } => {
                    found[4].insert(name_of(stop));
                }
                Alert::ClientJoined { role, path, .. } => {
                    found[5].insert(name_of(role));
                    found[6].insert(name_of(path));
                }
                Alert::ClientRevoked { by, .. } => {
                    found[7].insert(name_of(by));
                }
                Alert::CannotFeed { reason } => {
                    found[2].insert(name_of(reason));
                }
                _ => {}
            }
        }
        let every = [
            every_name::<FeedOutcome>(),
            every_name::<FeedReason>(),
            every_name::<Blocked>(),
            every_name::<MissedReason>(),
            every_name::<UpdateStop>(),
            every_name::<ClientRole>(),
            every_name::<EnrollPath>(),
            every_name::<Source>(),
        ];
        for (found, every) in found.iter().zip(&every) {
            assert!(!every.is_empty());
            assert_eq!(found, every);
        }
    }

    #[test]
    fn version_1_reads_and_writes_as_its_sample() {
        let sample = include_str!("../tests/fixtures/alerts-v1.json");
        stored::assert_every_field_written::<AlertState>(sample);
        let read = stored::read::<AlertState>(sample.as_bytes()).unwrap();
        assert_eq!(read, stored::Found::Read(every_field()));

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
        let watch = &every_field().feeders[0].watch;
        assert_eq!(
            changes.len(),
            watch.open.len() + watch.removing.len(),
            "everything open and everything being removed"
        );
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
                vec![PostedNotice {
                    id: 0x4000_0001,
                    seen_ms: 0,
                    kind: NoticeKind::FoodLow,
                }],
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
    fn a_sweep_keeps_what_a_watch_knows() {
        let dir = tempfile::tempdir().unwrap();
        let dir_name = dir.path().display().to_string();
        run(async {
            let state = every_field();
            let feeder =
                wellfed_session::hex::decode_array_either_case::<32>(KITCHEN.as_bytes()).unwrap();
            let known = state.feeders[0].watch.open[0].key.notification_id(&feeder);
            change(dir.path(), |stored| *stored = state).await.unwrap();

            let sweep =
                || crate::api::alerts::alerts_to_sweep(dir_name.clone(), vec![known, 0x4000_0abc]);
            assert_eq!(
                sweep().await.unwrap(),
                [0x4000_0abc],
                "only the unknown one"
            );
            crate::api::alerts::alerts_swept(dir_name.clone(), vec![0x4000_0abc])
                .await
                .unwrap();
            assert!(sweep().await.unwrap().is_empty(), "done");
            assert!(read(dir.path()).await.unwrap().sweeping.is_empty());
        });
    }

    /// Two engines sweeping at once: the first to finish lets go only of
    /// what it took off, and the other's ids stay held until it does.
    #[test]
    fn overlapping_sweeps_hold_each_others_ids() {
        let dir = tempfile::tempdir().unwrap();
        let dir_name = dir.path().display().to_string();
        run(async {
            let mut state = every_field();
            state.sweeping.clear();
            change(dir.path(), |stored| *stored = state).await.unwrap();
            let api =
                |showing: Vec<i32>| crate::api::alerts::alerts_to_sweep(dir_name.clone(), showing);

            let first = api(vec![0x4000_0001]).await.unwrap();
            let second = api(vec![0x4000_0001, 0x4000_0002]).await.unwrap();
            assert_eq!(second, [0x4000_0001, 0x4000_0002]);

            crate::api::alerts::alerts_swept(dir_name.clone(), first)
                .await
                .unwrap();
            let held: Vec<i32> = read(dir.path())
                .await
                .unwrap()
                .sweeping
                .iter()
                .map(|held| held.id)
                .collect();
            assert_eq!(held, [0x4000_0002]);
            crate::api::alerts::alerts_swept(dir_name.clone(), second)
                .await
                .unwrap();
            assert!(read(dir.path()).await.unwrap().sweeping.is_empty());
        });
    }

    /// A sweep killed before it said its notifications were off holds their
    /// posts back for a minute, then lets them be confirmed as any other.
    #[test]
    fn a_killed_sweep_lets_go_by_itself() {
        let mut state = every_field();
        state.sweeping.clear();
        let feeder =
            wellfed_session::hex::decode_array_either_case::<32>(KITCHEN.as_bytes()).unwrap();
        let due = *state.feeders[0]
            .watch
            .open
            .iter()
            .find(|tracked| tracked.delivery == Delivery::Due)
            .unwrap();
        let posted = [posting(&feeder, &due)];
        let id = posted[0].id;
        let due = due.key;
        let delivery = |state: &AlertState| {
            state.feeders[0]
                .watch
                .open
                .iter()
                .find(|tracked| tracked.key == due)
                .unwrap()
                .delivery
        };

        // Chosen, then the engine dies: no `alerts_swept`.
        state.hold_sweep(&[id], at(0));
        state.confirm(KITCHEN, &feeder, (&posted, &[]), at(30));
        assert_eq!(
            delivery(&state),
            Delivery::Due,
            "held while it may still go"
        );

        state.confirm(
            KITCHEN,
            &feeder,
            (&posted, &[]),
            at(0) + SWEEP_HOLD + SignedDuration::from_secs(1),
        );
        assert_eq!(delivery(&state), Delivery::Posted);
        assert!(state.sweeping.is_empty());

        // The phone's clock steps back an hour after the choice: the hold
        // lets go at once rather than an hour on.
        state.feeders[0]
            .watch
            .open
            .iter_mut()
            .find(|tracked| tracked.key == due)
            .unwrap()
            .delivery = Delivery::Due;
        state.hold_sweep(&[id], at(0));
        state.confirm(
            KITCHEN,
            &feeder,
            (&posted, &[]),
            at(0) - SignedDuration::from_hours(1),
        );
        assert_eq!(delivery(&state), Delivery::Posted, "a step back lets go");
        assert!(state.sweeping.is_empty());
    }

    /// A catch-up that tracks a notice the sweep is about to take off: its
    /// post isn't confirmed until the sweep is done, so it posts again.
    #[test]
    fn a_post_the_sweep_takes_down_is_handed_over_again() {
        let dir = tempfile::tempdir().unwrap();
        let dir_name = dir.path().display().to_string();
        run(async {
            let mut state = every_field();
            let feeder =
                wellfed_session::hex::decode_array_either_case::<32>(KITCHEN.as_bytes()).unwrap();
            let watch = &mut state.feeders[0].watch;
            let due = *watch
                .open
                .iter()
                .find(|tracked| tracked.delivery == Delivery::Due)
                .unwrap();
            let posted = posting(&feeder, &due);
            let id = posted.id;
            let due = due.key;
            // Chosen while the file did not know it yet.
            state.sweeping = vec![SweepHold {
                id,
                since: Timestamp::now(),
            }];
            change(dir.path(), |stored| *stored = state).await.unwrap();

            crate::api::alerts::confirm_notices(
                dir_name.clone(),
                KITCHEN.into(),
                vec![posted],
                vec![],
            )
            .await
            .unwrap();
            let still = read(dir.path()).await.unwrap();
            let tracked = still.feeders[0]
                .watch
                .open
                .iter()
                .find(|t| t.key == due)
                .unwrap();
            assert_eq!(
                tracked.delivery,
                Delivery::Due,
                "posted again after the sweep"
            );

            crate::api::alerts::alerts_swept(dir_name.clone(), vec![id])
                .await
                .unwrap();
            crate::api::alerts::confirm_notices(dir_name, KITCHEN.into(), vec![posted], vec![])
                .await
                .unwrap();
            let after = read(dir.path()).await.unwrap();
            let tracked = after.feeders[0]
                .watch
                .open
                .iter()
                .find(|t| t.key == due)
                .unwrap();
            assert_eq!(tracked.delivery, Delivery::Posted);
        });
    }

    #[test]
    fn the_app_asks_for_notifications_once_and_posts_what_is_still_due() {
        let dir = tempfile::tempdir().unwrap();
        let dir_name = dir.path().display().to_string();
        run(async {
            let due = || crate::api::alerts::notifications_offer_due(dir_name.clone());
            assert!(due().await.unwrap());
            assert!(due().await.unwrap(), "not until the request has run");
            let refused = || crate::api::alerts::notifications_refused(dir_name.clone());
            assert!(!refused().await.unwrap());
            crate::api::alerts::note_notifications_refused(dir_name.clone())
                .await
                .unwrap();
            assert!(refused().await.unwrap(), "kept for good on this install");

            crate::api::alerts::notifications_offered(dir_name.clone())
                .await
                .unwrap();
            assert!(!due().await.unwrap(), "the owner asks after that");

            let mut state = every_field();
            state.set_aside = None;
            change(dir.path(), |stored| *stored = state).await.unwrap();
            let pending = crate::api::alerts::pending_notices(dir_name.clone())
                .await
                .unwrap();
            let [kitchen] = pending.as_slice() else {
                panic!("{pending:?}");
            };
            // Only the due ones post; the posted, quiet and dismissed ones
            // are not posted again, and the removals stay.
            let watch = &every_field().feeders[0].watch;
            let due = watch
                .open
                .iter()
                .filter(|tracked| tracked.delivery == Delivery::Due)
                .count();
            let posts = kitchen
                .changes
                .iter()
                .filter(|change| matches!(change, crate::api::alerts::NoticeChange::Post { .. }))
                .count();
            assert_eq!(posts, due);
            assert_eq!(kitchen.changes.len(), due + watch.removing.len());
        });
    }

    /// One test, as the holds are the process's: a swipe takes the engine's
    /// and leaves a live run thread's, which goes when its thread ends,
    /// a panic included.
    #[test]
    fn a_swipe_leaves_a_live_run_threads_hold() {
        hold(AlertHold::Install, true);
        hold(AlertHold::Restore, true);
        clear_holds();
        assert!(!held(), "the engine's holds went with it");

        let (finish, finished) = std::sync::mpsc::channel::<()>();
        let guard = RunThread::hold();
        let thread = std::thread::spawn(move || {
            let _guard = guard;
            finished.recv().unwrap();
        });
        clear_holds();
        assert!(held(), "the update's thread still runs");
        finish.send(()).unwrap();
        thread.join().unwrap();
        assert!(!held(), "its thread ended");

        let guard = RunThread::hold();
        let panicked = std::thread::spawn(move || {
            let _guard = guard;
            panic!("a run that fails");
        });
        assert!(panicked.join().is_err());
        assert!(!held(), "a panic lets go too");
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

    /// A newer minor's fields, at every struct the file nests, outlast a
    /// change and its save.
    #[test]
    fn a_save_carries_what_a_newer_minor_added() {
        let dir = tempfile::tempdir().unwrap();
        let mut grown: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/alerts-v1.json")).unwrap();
        let places = [
            "",
            "/data",
            "/data/settings",
            "/data/settings/mutes",
            "/data/feeders/0",
            "/data/feeders/0/watch",
            "/data/feeders/0/watch/reach",
            "/data/sweeping/0",
            "/data/set_aside",
        ];
        for place in places {
            grown.pointer_mut(place).unwrap()["added"] = serde_json::json!(place);
        }
        std::fs::write(path(dir.path()), grown.to_string()).unwrap();

        run(change(dir.path(), |state| state.sweep = !state.sweep)).unwrap();

        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path(dir.path())).unwrap()).unwrap();
        assert_ne!(saved["data"]["sweep"], grown["data"]["sweep"], "saved");
        for place in places {
            assert_eq!(saved.pointer(place).unwrap()["added"], place, "{place}");
        }
    }
}
