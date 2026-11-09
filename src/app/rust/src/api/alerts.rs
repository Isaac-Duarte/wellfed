//! Alerts at home, as Dart reaches them. The feeder decides every alert; this
//! phone catches up with its table, posts each alert once, takes it off when
//! the feeder clears it, and judges only that a feeder is unreachable. The
//! catch-up, the cursor and every choice of what to post or remove are Rust's
//! ([`crate::catch_up`], `wellfed_client::alerts`): Dart schedules the
//! catch-ups, posts and removes notifications, and confirms each change with
//! [`confirm_notices`] once made, so a change lost on the way is handed over
//! again.
//!
//! Nothing here sends a feed or a write: a catch-up reads, and a mute or a
//! dismissal stays on this phone.

use std::path::Path;

use flutter_rust_bridge::frb;
use jiff::Timestamp;
use wellfed_client::alerts::{Change, Delivery, Notice, NoticeKind, Tracked};
use wellfed_formats::canonical::{canonical_of, sha256};
use wellfed_model::{
    Alert, AlertLevel, Blocked, ClientRole, FeedOutcome, FeedReason, MissedReason, UpdateStop,
};

use crate::alerts;
use crate::api::error::{BridgeError, ErrorKind};
use crate::api::keys::ClientKey;
use crate::api::store::SetAside;
use crate::catch_up::{self, Target};

/// How the phone is connected, as Android says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhoneNetwork {
    /// On a Wi-Fi network this app may use now, by its addresses: what tells
    /// one network from another with no location permission, which its name
    /// would need.
    Wifi {
        gateway: String,
        subnet: String,
        dns: Vec<String>,
        domains: Option<String>,
    },
    /// Off Wi-Fi, or Android keeps this app off the network for now: a
    /// catch-up that fails here says nothing about the feeder.
    Elsewhere,
}

impl PhoneNetwork {
    /// A digest of the network's addresses, so the file keeps none of them. A
    /// lookalike with all the same addresses still passes.
    fn fingerprint(&self) -> Result<Option<String>, BridgeError> {
        match self {
            Self::Wifi {
                gateway,
                subnet,
                dns,
                domains,
            } => {
                // Android promises no order of the servers.
                let mut dns = dns.clone();
                dns.sort_unstable();
                Ok(Some(sha256(&canonical_of(&(
                    gateway, subnet, dns, domains,
                ))?)))
            }
            Self::Elsewhere => Ok(None),
        }
    }
}

/// Which try of a round this is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Attempt {
    /// Where the feeder last answered a catch-up, or the app.
    First,
    /// After [`CatchUpEnd::Unanswered`]: where NSD found it now, if it did.
    Last { located: Option<String> },
}

/// How a catch-up ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatchUpEnd {
    /// It read the alert table and applied it.
    Read,
    /// The status showed the table as it was last read.
    Unchanged,
    /// The feeder answered, but gave no table: it turned this phone away,
    /// or its build has none. It is there, so it is not unreachable.
    Answered,
    /// Another catch-up with this feeder was running, or a run keeps them off.
    Skipped,
    /// No address to try, and not on a Wi-Fi network: the phone is away.
    Away,
    /// Nothing answered the first try: look for the feeder and try again.
    Unanswered,
    /// Nothing answered the last try. On the feeder's home network that
    /// counts towards unreachable; anywhere else it counts for nothing.
    Failed,
}

/// What a notice says, for Dart's words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoticeDetail {
    /// A feed that failed (`outcome` set) or may have dispensed up to
    /// `requested` (`outcome` none).
    Feed {
        feed: u32,
        plan: Option<u8>,
        requested: u8,
        completed: u8,
        outcome: Option<FeedOutcome>,
        reason: FeedReason,
    },
    /// A plan's meal that was missed (`missed`) or has no record.
    Meal {
        plan: u8,
        due_ms: i64,
        missed: Option<MissedReason>,
    },
    /// A client joined (`role` set) or was removed.
    Client {
        /// The client's fingerprint, hex.
        client: String,
        role: Option<ClientRole>,
    },
    UpdateFailed {
        stop: UpdateStop,
    },
    CannotFeed {
        reason: Blocked,
    },
    FoodLikelyOut {
        confirmed: u8,
    },
    /// Days left of the desiccant's period: 0 or below.
    DesiccantDue {
        days: i32,
    },
    /// Not reached at home since then.
    Unreachable {
        since_ms: i64,
    },
    /// The kind says it all.
    Plain,
}

/// One notice, as a notification or the app's list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoticeView {
    /// Its Android notification id: the same for the same alert, always.
    pub id: i32,
    pub kind: NoticeKind,
    pub level: AlertLevel,
    /// The feeder's time for it (a report's own, or when a condition began),
    /// or none without a valid clock then.
    pub at_ms: Option<i64>,
    /// When this phone first saw it.
    pub seen_ms: i64,
    pub detail: NoticeDetail,
    /// Listed, never posted: muted, or this phone's own joining.
    pub quiet: bool,
}

/// One change to make on the phone, then confirm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoticeChange {
    Post { notice: NoticeView },
    Remove { id: i32 },
}

/// A feeder's changes to make.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeederNotices {
    /// The feeder's identity, hex.
    pub identity: String,
    pub changes: Vec<NoticeChange>,
}

/// What one catch-up found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatchUp {
    pub end: CatchUpEnd,
    /// Every change still to make for this feeder, older ones included.
    pub changes: Vec<NoticeChange>,
}

/// A feeder's alerts as this phone lists them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeederAlerts {
    /// Open and not dismissed, oldest first.
    pub notices: Vec<NoticeView>,
    /// Alerts the feeder holds that this app is too old to read.
    pub unread: u8,
    /// When a catch-up last reached it.
    pub reached_ms: Option<i64>,
}

/// This phone's choices for alerts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertSettingsView {
    pub instant: bool,
    /// Every kind a notice can be, with its level, in the order the settings
    /// list them.
    pub kinds: Vec<KindLevel>,
    pub muted_kinds: Vec<NoticeKind>,
    pub muted_levels: Vec<AlertLevel>,
}

/// A kind of notice and the level it takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KindLevel {
    pub kind: NoticeKind,
    pub level: AlertLevel,
}

/// A run that keeps catch-ups off every feeder while it goes: a feeder
/// mid-install is meant to go quiet and must not read as unreachable, and the
/// install may hold the feeder's own network.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AlertHold {
    Install,
    Restore,
    Update,
}

/// Keep catch-ups off while `run` goes (`held`), or let go. A hold lapses by
/// itself after two hours unrenewed.
#[frb(sync)]
pub fn hold_alerts(run: AlertHold, held: bool) {
    alerts::hold(run, held);
}

/// One catch-up with the feeder `identity` (hex), as this phone's `key`, with
/// its files in `dir`. `manual` is where the owner said to dial it and `known`
/// where the app last reached it; `attempt` says which try of a round this is.
pub async fn catch_up_with(
    dir: String,
    key: &ClientKey,
    identity: String,
    manual: Option<String>,
    known: Option<String>,
    attempt: Attempt,
    network: PhoneNetwork,
) -> Result<CatchUp, BridgeError> {
    let feeder = identity_of(&identity)?;
    let target = Target {
        keys: key.keypair(),
        identity: feeder,
        manual: manual.as_deref(),
        known: known.as_deref(),
        attempt: &attempt,
        network: network.fingerprint()?,
    };
    let (end, changes) = catch_up::catch_up(Path::new(&dir), &target, Timestamp::now).await?;
    Ok(CatchUp {
        end,
        changes: views(&feeder, &changes),
    })
}

/// The changes Dart made: each posted notification's id and each removed one's.
/// A change that went stale meanwhile is left for the next catch-up. The ids
/// to take off again: everything posted for a feeder forgotten while its
/// changes were on the way.
pub async fn confirm_notices(
    dir: String,
    identity: String,
    posted: Vec<i32>,
    removed: Vec<i32>,
) -> Result<Vec<i32>, BridgeError> {
    let feeder = identity_of(&identity)?;
    alerts::change(Path::new(&dir), |state| {
        let sweeping = state.sweeping.clone();
        let Some(watched) = state.watched_mut(&identity) else {
            return posted;
        };
        let watch = &mut watched.watch;
        for change in watch.changes() {
            let made = match &change {
                // The sweep may take it down after it went up.
                Change::Post(tracked) => {
                    let id = tracked.key.notification_id(&feeder);
                    posted.contains(&id) && !sweeping.contains(&id)
                }
                Change::Remove(key) => removed.contains(&key.notification_id(&feeder)),
            };
            if made {
                watch.confirm(&change);
            }
        }
        Vec::new()
    })
    .await
}

/// The feeder's alerts as this phone lists them.
pub async fn feeder_alerts(dir: String, identity: String) -> Result<FeederAlerts, BridgeError> {
    let feeder = identity_of(&identity)?;
    let state = alerts::read(Path::new(&dir)).await?;
    let Some(watched) = state.watched(&identity) else {
        return Ok(FeederAlerts {
            notices: Vec::new(),
            unread: 0,
            reached_ms: None,
        });
    };
    Ok(FeederAlerts {
        notices: watched
            .watch
            .listed()
            .map(|tracked| view(&feeder, tracked))
            .collect(),
        unread: watched.watch.unread,
        reached_ms: watched.watch.reach.reached.map(Timestamp::as_millisecond),
    })
}

/// The owner dismissed the notice `id` on this phone: hidden here until the
/// feeder clears it. The feeder is never told. The changes to make.
pub async fn dismiss_notice(
    dir: String,
    identity: String,
    id: i32,
) -> Result<Vec<NoticeChange>, BridgeError> {
    let feeder = identity_of(&identity)?;
    let changes = alerts::change(Path::new(&dir), |state| {
        let Some(watched) = state.watched_mut(&identity) else {
            return Vec::new();
        };
        let watch = &mut watched.watch;
        let key = watch
            .listed()
            .map(|tracked| tracked.key)
            .find(|key| key.notification_id(&feeder) == id);
        if let Some(key) = key {
            watch.dismiss(key);
        }
        watch.changes()
    })
    .await?;
    Ok(views(&feeder, &changes))
}

/// This phone's choices for alerts.
pub async fn alert_settings(dir: String) -> Result<AlertSettingsView, BridgeError> {
    let state = alerts::read(Path::new(&dir)).await?;
    Ok(AlertSettingsView {
        instant: state.settings.instant,
        kinds: NoticeKind::all()
            .map(|kind| KindLevel {
                kind,
                level: kind.level(),
            })
            .collect(),
        muted_kinds: state.settings.mutes.kinds,
        muted_levels: state.settings.mutes.levels,
    })
}

/// Whether the app has yet to ask Android, unprompted, to let it post. It
/// asks once, at a moment the owner sees why (Home with a feeder), and says
/// so with [`notifications_offered`] once the request has run. Later asks
/// are the owner's, from the alert screens.
pub async fn notifications_offer_due(dir: String) -> Result<bool, BridgeError> {
    Ok(!alerts::read(Path::new(&dir)).await?.settings.asked)
}

/// The app asked Android, unprompted: it does not again.
pub async fn notifications_offered(dir: String) -> Result<(), BridgeError> {
    alerts::change(Path::new(&dir), |state| state.settings.asked = true).await
}

/// Every feeder's changes still to make, with no catch-up: for when Android
/// starts letting the app post. Only alerts still open are posted; one that
/// ended meanwhile was never kept.
pub async fn pending_notices(dir: String) -> Result<Vec<FeederNotices>, BridgeError> {
    let state = alerts::read(Path::new(&dir)).await?;
    state
        .feeders
        .iter()
        .map(|watched| {
            let feeder = identity_of(&watched.identity)?;
            Ok(FeederNotices {
                identity: watched.identity.clone(),
                changes: views(&feeder, &watched.watch.changes()),
            })
        })
        .collect()
}

/// Save whether "instant while home" is on.
pub async fn save_instant(dir: String, enabled: bool) -> Result<(), BridgeError> {
    alerts::change(Path::new(&dir), |state| state.settings.instant = enabled).await
}

/// Mute or unmute a kind of notice, or a level, on this phone. Notices
/// posted or due that are muted now come off: the changes to make, per
/// feeder.
pub async fn set_mute(
    dir: String,
    mute: MuteTarget,
    muted: bool,
) -> Result<Vec<FeederNotices>, BridgeError> {
    let changed = alerts::change(Path::new(&dir), |state| {
        let mutes = &mut state.settings.mutes;
        match mute {
            MuteTarget::Kind { kind } => toggle(&mut mutes.kinds, kind, muted),
            MuteTarget::Level { level } => toggle(&mut mutes.levels, level, muted),
        }
        let mutes = mutes.clone();
        state
            .feeders
            .iter_mut()
            .map(|watched| {
                watched.watch.remute(&mutes);
                (watched.identity.clone(), watched.watch.changes())
            })
            .collect::<Vec<_>>()
    })
    .await?;
    changed
        .into_iter()
        .map(|(identity, changes)| {
            let feeder = identity_of(&identity)?;
            Ok(FeederNotices {
                changes: views(&feeder, &changes),
                identity,
            })
        })
        .collect()
}

/// What a mute applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MuteTarget {
    Kind { kind: NoticeKind },
    Level { level: AlertLevel },
}

fn toggle<T: PartialEq>(list: &mut Vec<T>, item: T, on: bool) {
    let present = list.contains(&item);
    if on && !present {
        list.push(item);
    } else if !on {
        list.retain(|each| *each != item);
    }
}

/// Forget the feeder `identity`'s alerts, as the phone forgets the feeder:
/// the ids of every notification it may have on the phone, to take off.
pub async fn forget_alerts(dir: String, identity: String) -> Result<Vec<i32>, BridgeError> {
    let feeder = identity_of(&identity)?;
    alerts::change(Path::new(&dir), |state| {
        state
            .forget(&identity)
            .iter()
            .map(|change| match change {
                Change::Post(tracked) => tracked.key.notification_id(&feeder),
                Change::Remove(key) => key.notification_id(&feeder),
            })
            .collect()
    })
    .await
}

/// `alerts.json` as it was set aside, if it was and the owner has not read
/// the notice.
pub async fn alerts_set_aside(dir: String) -> Result<Option<SetAside>, BridgeError> {
    Ok(alerts::read(Path::new(&dir)).await?.set_aside)
}

/// Of the alert notifications `showing` on the phone, those to take off
/// since `alerts.json` was set aside: the ones no watch in the file knows,
/// which nothing would ever take off. None once the sweep is done. Chosen
/// under the file's lock, and a catch-up writes a notice's key before Dart
/// posts it, so a notification another engine posts meanwhile is known and
/// kept. One a catch-up tracks after this is not confirmed posted until
/// [`alerts_swept`] says they are off, so it is handed over again. Taking
/// off one of those ids again is harmless, so the sweep is done only then.
pub async fn alerts_to_sweep(dir: String, showing: Vec<i32>) -> Result<Vec<i32>, BridgeError> {
    alerts::change(Path::new(&dir), |state| {
        if !state.sweep {
            return Ok(Vec::new());
        }
        let mut known = Vec::new();
        for watched in &state.feeders {
            let feeder = identity_of(&watched.identity)?;
            known.extend(
                watched
                    .watch
                    .every_key()
                    .map(|key| key.notification_id(&feeder)),
            );
        }
        let stale: Vec<i32> = showing
            .into_iter()
            .filter(|id| !known.contains(id))
            .collect();
        state.sweeping.clone_from(&stale);
        Ok(stale)
    })
    .await?
}

/// The notifications [`alerts_to_sweep`] chose are off.
pub async fn alerts_swept(dir: String) -> Result<(), BridgeError> {
    alerts::change(Path::new(&dir), |state| {
        state.sweep = false;
        state.sweeping.clear();
    })
    .await
}

/// The owner read the notice of a set-aside `alerts.json`.
pub async fn alerts_notice_read(dir: String) -> Result<(), BridgeError> {
    alerts::change(Path::new(&dir), |state| state.set_aside = None).await
}

fn identity_of(identity: &str) -> Result<[u8; 32], BridgeError> {
    wellfed_session::hex::decode_array_either_case::<32>(identity.as_bytes())
        .ok_or_else(|| BridgeError::new(ErrorKind::Key, "a feeder identity is 64 hex digits"))
}

fn views(feeder: &[u8; 32], changes: &[Change]) -> Vec<NoticeChange> {
    changes
        .iter()
        .map(|change| match change {
            Change::Post(tracked) => NoticeChange::Post {
                notice: view(feeder, tracked),
            },
            Change::Remove(key) => NoticeChange::Remove {
                id: key.notification_id(feeder),
            },
        })
        .collect()
}

fn view(feeder: &[u8; 32], tracked: &Tracked) -> NoticeView {
    let kind = tracked.notice.kind();
    let (at, detail) = match tracked.notice {
        Notice::Alert { open } => (open.at, detail(open.alert)),
        Notice::Unreachable { since } => (
            Some(since),
            NoticeDetail::Unreachable {
                since_ms: since.as_millisecond(),
            },
        ),
    };
    NoticeView {
        id: tracked.key.notification_id(feeder),
        kind,
        level: kind.level(),
        at_ms: at.map(Timestamp::as_millisecond),
        seen_ms: tracked.seen.as_millisecond(),
        detail,
        quiet: tracked.delivery == Delivery::Quiet,
    }
}

fn detail(alert: Alert) -> NoticeDetail {
    match alert {
        Alert::MealFailed {
            feed,
            plan,
            requested,
            completed,
            outcome,
            reason,
        } => NoticeDetail::Feed {
            feed,
            plan,
            requested,
            completed,
            outcome: Some(outcome),
            reason,
        },
        Alert::MealUncertain {
            feed,
            plan,
            requested,
            completed,
            reason,
        } => NoticeDetail::Feed {
            feed,
            plan,
            requested,
            completed,
            outcome: None,
            reason,
        },
        Alert::MealMissed { plan, due, reason } => NoticeDetail::Meal {
            plan,
            due_ms: due.as_millisecond(),
            missed: Some(reason),
        },
        Alert::MealNoRecord { plan, due } => NoticeDetail::Meal {
            plan,
            due_ms: due.as_millisecond(),
            missed: None,
        },
        Alert::ClientJoined { client, role, .. } => NoticeDetail::Client {
            client: client.to_string(),
            role: Some(role),
        },
        Alert::ClientRevoked { client, .. } => NoticeDetail::Client {
            client: client.to_string(),
            role: None,
        },
        Alert::UpdateFailed { stop } => NoticeDetail::UpdateFailed { stop },
        Alert::CannotFeed { reason } => NoticeDetail::CannotFeed { reason },
        Alert::FoodLikelyOut { confirmed } => NoticeDetail::FoodLikelyOut { confirmed },
        Alert::DesiccantDue { days } => NoticeDetail::DesiccantDue { days },
        Alert::PowerWasOut
        | Alert::RolledBack
        | Alert::RepeatedRestarts
        | Alert::NoClock
        | Alert::FoodLow
        | Alert::BatteryBelowFloor
        | Alert::PlansStuck => NoticeDetail::Plain,
    }
}
