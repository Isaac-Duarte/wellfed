//! What a phone makes of one feeder's alerts between catch-ups: which to
//! post, which to take off, and the one judgement the phone makes itself,
//! that the feeder is unreachable. The feeder decides every other alert
//! ([alerts]); this only keeps track of what the phone has said about them.
//!
//! A catch-up opens a session, reads the status, and reads the alert table
//! only when the status shows it moved since the last one applied: the
//! [`Cursor`] is the table's epoch, the boot and its generation, which counts
//! from 0 at each boot. [`Watch::apply`] then compares the table with what the
//! phone tracks, by each alert's key, so an alert posts once however often it
//! is read, and a notice comes off when the feeder clears the alert. While
//! the feeder says an absent alert may not have ended (`settling`, or `more`
//! for a report), its notice stays.
//!
//! Delivery is at least once. A post or a removal waits in the watch until
//! the caller confirms it ([`Watch::confirm`]), in the same file as the state
//! that made it, so a catch-up stopped between the two loses nothing: the
//! next one hands it over again, under the same notification id.
//!
//! Nothing here feeds or writes: a catch-up reads, and a dismissal or a mute
//! is this phone's alone. The feeder is never told.
//!
//! [alerts]: ../../../../docs/firmware/alerts.md

use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wellfed_model::{AlertKey, AlertKind, AlertLevel, AlertsView, Generation, Hex, OpenAlert};

/// How long a report stays in the feeder's table after its own time, with an
/// hour's room for the two clocks: past this it has lapsed whatever the table
/// says.
pub const REPORT_LIFE: SignedDuration = SignedDuration::from_hours(49);
/// Failures in a row on the home network before the phone says the feeder is
/// unreachable: one more than a lease moving it and a missed discovery.
pub const UNREACHABLE_AFTER: u32 = 3;
/// And for at least this long, so a phone checking every minute does not call
/// a restart or a router's hiccup a lost feeder.
pub const UNREACHABLE_FOR: SignedDuration = SignedDuration::from_mins(10);
/// How long a condition may stay absent from tables the feeder can't vouch
/// for (`settling`) before its notice comes off anyway. `settling` is one
/// flag for the whole table, and a reading that stays unknown keeps it set
/// for good (the desiccant's days with no clock), so without this a cleared
/// condition would stay posted. Twice the longest threshold that starts again
/// at a boot (no clock, 1 hour), so a boot's own thresholds never end one.
///
/// A condition that waits for the clock (the desiccant's days, and plans
/// stuck, whose threshold runs from the clock's arrival) isn't held to this
/// while the feeder's clock is unknown: its hold counts from the first read
/// with a clock, so a reboot with no clock neither ends it nor, when the
/// clock comes, posts it again and loses a dismissal. Held longer than this
/// by another reading still unknown (the food beam before its first reading,
/// a fitted pack's verdict), a condition does come off, and posts again if
/// the feeder reopens it.
pub const SETTLING_HOLD: SignedDuration = SignedDuration::from_hours(2);
/// The most notices a watch tracks. The feeder's table holds 12; reports kept
/// past it while the feeder can't vouch for their end, and a phone's own,
/// stay well under this. Past it the oldest report goes first.
const TRACKED_MAX: usize = 48;

/// What a notice is about: a feeder's alert kind, or the phone's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    MealFailed,
    MealUncertain,
    MealMissed,
    MealNoRecord,
    PowerWasOut,
    UpdateFailed,
    RolledBack,
    ClientJoined,
    ClientRevoked,
    RepeatedRestarts,
    NoClock,
    CannotFeed,
    FoodLow,
    FoodLikelyOut,
    DesiccantDue,
    BatteryBelowFloor,
    PlansStuck,
    /// The phone, at home, has not reached the feeder for a while: power or
    /// Wi-Fi. The one notice the phone decides.
    Unreachable,
}

impl From<AlertKind> for NoticeKind {
    fn from(kind: AlertKind) -> Self {
        match kind {
            AlertKind::MealFailed => Self::MealFailed,
            AlertKind::MealUncertain => Self::MealUncertain,
            AlertKind::MealMissed => Self::MealMissed,
            AlertKind::MealNoRecord => Self::MealNoRecord,
            AlertKind::PowerWasOut => Self::PowerWasOut,
            AlertKind::UpdateFailed => Self::UpdateFailed,
            AlertKind::RolledBack => Self::RolledBack,
            AlertKind::ClientJoined => Self::ClientJoined,
            AlertKind::ClientRevoked => Self::ClientRevoked,
            AlertKind::RepeatedRestarts => Self::RepeatedRestarts,
            AlertKind::NoClock => Self::NoClock,
            AlertKind::CannotFeed => Self::CannotFeed,
            AlertKind::FoodLow => Self::FoodLow,
            AlertKind::FoodLikelyOut => Self::FoodLikelyOut,
            AlertKind::DesiccantDue => Self::DesiccantDue,
            AlertKind::BatteryBelowFloor => Self::BatteryBelowFloor,
            AlertKind::PlansStuck => Self::PlansStuck,
        }
    }
}

impl NoticeKind {
    /// Every kind: the feeder's, then the phone's own.
    pub fn all() -> impl Iterator<Item = Self> {
        AlertKind::ALL
            .into_iter()
            .map(Self::from)
            .chain([Self::Unreachable])
    }

    pub fn level(self) -> AlertLevel {
        match AlertKind::ALL
            .into_iter()
            .find(|kind| Self::from(*kind) == self)
        {
            Some(kind) => kind.level(),
            None => AlertLevel::Urgent,
        }
    }
}

/// What a phone mutes: no notice posts for these, though the app still lists
/// them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mutes {
    pub kinds: Vec<NoticeKind>,
    pub levels: Vec<AlertLevel>,
}

impl Mutes {
    pub fn mutes(&self, kind: NoticeKind) -> bool {
        self.kinds.contains(&kind) || self.levels.contains(&kind.level())
    }
}

/// One notice's identity on a feeder: equal keys post as one notification.
/// A feeder's alert counts in its ring's epoch, as its feed numbers do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "notice", rename_all = "snake_case")]
pub enum NoticeKey {
    Alert { epoch: u32, key: AlertKey },
    Unreachable,
}

impl NoticeKey {
    /// The Android notification id for this notice on the feeder `identity`:
    /// the same across runs, and different between two feeders' copies of one
    /// alert. Positive, as Android's ids are signed, and above the small ids
    /// a foreground service's own notice takes.
    pub fn notification_id(&self, identity: &[u8; 32]) -> i32 {
        let mut hash = Sha256::new();
        hash.update(identity);
        self.hash_into(&mut hash);
        let digest = hash.finalize();
        let head = digest.first_chunk::<4>().copied().unwrap_or_default();
        let id = (u32::from_le_bytes(head) & 0x3fff_ffff) | 0x4000_0000;
        i32::try_from(id).unwrap_or(i32::MAX)
    }

    /// A fixed encoding of the key, so the id never depends on a serializer.
    fn hash_into(&self, hash: &mut Sha256) {
        let time = |at: Option<Timestamp>| at.map_or(0, Timestamp::as_second).to_le_bytes();
        let kind = |kind: AlertKind| {
            let at = AlertKind::ALL.iter().position(|each| *each == kind);
            [at.and_then(|at| u8::try_from(at).ok()).unwrap_or(u8::MAX)]
        };
        match self {
            Self::Unreachable => hash.update([0]),
            Self::Alert { epoch, key } => {
                hash.update([1]);
                hash.update(epoch.to_le_bytes());
                match key {
                    AlertKey::Feed { feed } => {
                        hash.update([0]);
                        hash.update(feed.to_le_bytes());
                    }
                    AlertKey::Meal { plan, due } => {
                        hash.update([1, *plan]);
                        hash.update(time(Some(*due)));
                    }
                    AlertKey::Client {
                        kind: alert,
                        client,
                        at,
                    } => {
                        hash.update([2]);
                        hash.update(kind(*alert));
                        hash.update(client.0);
                        hash.update(time(*at));
                    }
                    AlertKey::Timed { kind: alert, at } => {
                        hash.update([3]);
                        hash.update(kind(*alert));
                        hash.update(time(*at));
                    }
                    AlertKey::Kind { kind: alert } => {
                        hash.update([4]);
                        hash.update(kind(*alert));
                    }
                }
            }
        }
    }
}

/// What a notice says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "notice", rename_all = "snake_case")]
pub enum Notice {
    /// The feeder's alert, as its table last showed it.
    Alert { open: OpenAlert },
    /// Not reached at home since `since`, the first failure in the run.
    Unreachable { since: Timestamp },
}

impl Notice {
    pub fn kind(&self) -> NoticeKind {
        match self {
            Self::Alert { open } => open.alert.kind().into(),
            Self::Unreachable { .. } => NoticeKind::Unreachable,
        }
    }
}

/// What the phone has done with a notice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    /// To post: not yet confirmed shown.
    Due,
    /// Posted. The owner may have swiped it away since, which changes
    /// nothing: it is never posted again for the same thing.
    Posted,
    /// Muted, or this phone's own joining: listed, never posted.
    Quiet,
    /// The owner dismissed it on this phone: hidden here until it ends.
    Dismissed,
}

/// One notice the phone tracks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tracked {
    pub key: NoticeKey,
    pub notice: Notice,
    /// When this phone first saw it, by the phone's clock.
    pub seen: Timestamp,
    pub delivery: Delivery,
    /// Since when tables this phone read have left it out, while the
    /// feeder could not vouch that it ended; `None` while tables list it.
    pub absent: Option<Absence>,
}

/// How long a tracked alert has been left out of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "absence", rename_all = "snake_case")]
pub enum Absence {
    /// Left out since `since`, by the phone's clock.
    Since { since: Timestamp },
    /// Left out while the feeder had no clock, for a condition that waits
    /// for one: its hold starts at the first read with a clock.
    NoClock,
}

/// The alert table a catch-up last applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    /// The ring's epoch, from the welcome.
    pub epoch: u32,
    /// The feeder's boot count, from its status: the generation counts from
    /// 0 at each boot, so alone it can't say the table is the same.
    pub boot: u32,
    pub generation: Generation,
}

/// What the phone knows of reaching the feeder, for [`NoticeKind::Unreachable`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reach {
    /// The network the feeder last answered on, by the caller's fingerprint
    /// of it: the phone's home for this feeder. Only a failure there counts.
    pub home: Option<String>,
    /// Failures in a row on the home network, and when the first was.
    pub failures: u32,
    pub first_failure: Option<Timestamp>,
    /// When a catch-up last reached the feeder.
    pub reached: Option<Timestamp>,
}

/// One change for the caller to make on the phone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Post this notice, or post it again over its earlier self.
    Post(Tracked),
    /// Take the notice with this key off the phone.
    Remove(NoticeKey),
}

/// Who the phone is and what it mutes, for one catch-up, and what the
/// feeder's status said of its clock.
#[derive(Clone, Copy, Debug)]
pub struct Context<'a> {
    pub now: Timestamp,
    pub mutes: &'a Mutes,
    /// This phone's fingerprint (its key's first 4 bytes): its own joining
    /// is listed, never posted.
    pub own: Hex<4>,
    pub clock: FeederClock,
}

/// Whether the feeder's status this catch-up showed a valid clock. The
/// watch also counts a clock with no zone offset as unknown, by the table's
/// `NoClock`: the plans need local time, which the status doesn't show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeederClock {
    Known,
    /// Invalid, or no status was read.
    Unknown,
}

/// What a phone keeps of one feeder's alerts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Watch {
    /// The table last applied; `None` before the first.
    pub cursor: Option<Cursor>,
    /// The notices open now, oldest first.
    pub open: Vec<Tracked>,
    /// Notices that ended or were dismissed, whose removal is not yet
    /// confirmed.
    pub removing: Vec<NoticeKey>,
    /// Alerts in the last table this build could not read: a newer feeder
    /// build's. The app says to update itself.
    pub unread: u8,
    pub reach: Reach,
}

impl Watch {
    /// Whether the table needs reading: it moved since the last one applied.
    pub fn needs_read(&self, cursor: Cursor) -> bool {
        self.cursor != Some(cursor)
    }

    /// The feeder answered a catch-up on the network `network` (a fingerprint
    /// of it, or `None` when the phone could not say). That network is home
    /// from now on, and an open unreachable notice ends.
    pub fn reached(&mut self, network: Option<String>, now: Timestamp) {
        if network.is_some() {
            self.reach.home = network;
        }
        self.reach.failures = 0;
        self.reach.first_failure = None;
        self.reach.reached = Some(now);
        self.end(|key| *key == NoticeKey::Unreachable);
    }

    /// Nothing answered, on `network`. Only a failure on the feeder's home
    /// network counts: elsewhere, the phone is simply away and judges nothing.
    pub fn unanswered(&mut self, network: Option<&str>, context: &Context<'_>) {
        let home = self.reach.home.as_deref();
        if network.is_none() || network != home {
            return;
        }

        self.reach.failures = self.reach.failures.saturating_add(1);
        let since = *self.reach.first_failure.get_or_insert(context.now);
        let long_enough = context.now.duration_since(since) >= UNREACHABLE_FOR;
        if self.reach.failures >= UNREACHABLE_AFTER && long_enough {
            self.track(
                NoticeKey::Unreachable,
                Notice::Unreachable { since },
                context,
            );
        }
    }

    /// Apply the table `view`, read under `cursor`. Each alert not yet
    /// tracked opens a notice, and each tracked one the table no longer
    /// shows ends, unless the feeder can't yet vouch for its end.
    pub fn apply(&mut self, cursor: Cursor, view: &AlertsView, context: &Context<'_>) {
        // Another epoch is another ring: what this phone tracked from the
        // old one means nothing now.
        if self
            .cursor
            .is_some_and(|before| before.epoch != cursor.epoch)
        {
            self.end(|key| matches!(key, NoticeKey::Alert { .. }));
        }
        self.cursor = Some(cursor);
        self.unread = view.unread;

        for open in &view.alerts {
            let key = NoticeKey::Alert {
                epoch: cursor.epoch,
                key: open.key(),
            };
            self.track(key, Notice::Alert { open: *open }, context);
        }

        let shown = |tracked: &Tracked| {
            view.alerts.iter().any(|open| {
                tracked.key
                    == NoticeKey::Alert {
                        epoch: cursor.epoch,
                        key: open.key(),
                    }
            })
        };
        let lists_no_clock = view
            .alerts
            .iter()
            .any(|open| open.alert.kind() == AlertKind::NoClock);
        let clock = local_time(context.clock, lists_no_clock);
        let mut ended = Vec::new();
        for tracked in &mut self.open {
            let Notice::Alert { open } = tracked.notice else {
                continue;
            };
            if shown(tracked) {
                tracked.absent = None;
                continue;
            }
            let vouched = if open.alert.kind().is_condition() {
                !view.settling
            } else {
                !view.settling && !view.more
            };
            if vouched {
                ended.push(tracked.key);
                continue;
            }
            if let Some(absent) = tracked.hold(&open, clock, context.now)
                && unvouched_over(&open, absent, context.now)
            {
                ended.push(tracked.key);
            }
        }
        self.end(|key| ended.contains(key));
        self.bound();
    }

    /// Alerts the last table left out without vouching for their end, once
    /// they have been absent past their hold: for a catch-up that found the
    /// table unchanged.
    pub fn lapse(&mut self, context: &Context<'_>) {
        let lists_no_clock = self.open.iter().any(|tracked| {
            tracked.absent.is_none()
                && matches!(tracked.notice, Notice::Alert { open } if open.alert.kind() == AlertKind::NoClock)
        });
        let clock = local_time(context.clock, lists_no_clock);
        let mut lapsed = Vec::new();
        for tracked in &mut self.open {
            let Notice::Alert { open } = tracked.notice else {
                continue;
            };
            if tracked.absent.is_none() {
                continue;
            }
            if let Some(absent) = tracked.hold(&open, clock, context.now)
                && unvouched_over(&open, absent, context.now)
            {
                lapsed.push(tracked.key);
            }
        }
        self.end(|key| lapsed.contains(key));
    }

    /// The owner dismissed `key` on this phone. Its notice comes off, and the
    /// app lists it no more until it ends and opens again. Whether it was
    /// open.
    pub fn dismiss(&mut self, key: NoticeKey) -> bool {
        let Some(tracked) = self.open.iter_mut().find(|tracked| tracked.key == key) else {
            return false;
        };
        tracked.delivery = Delivery::Dismissed;
        self.removal(key);
        true
    }

    /// Mutes changed: a notice posted or due of a kind now muted comes off.
    /// One unmuted stays quiet, as the owner has already seen it listed.
    pub fn remute(&mut self, mutes: &Mutes) {
        let mut removed = Vec::new();
        for tracked in &mut self.open {
            let loud = matches!(tracked.delivery, Delivery::Due | Delivery::Posted);
            if loud && mutes.mutes(tracked.notice.kind()) {
                tracked.delivery = Delivery::Quiet;
                removed.push(tracked.key);
            }
        }
        for key in removed {
            self.removal(key);
        }
    }

    /// What the phone still has to do: removals first, so a notice that ends
    /// and opens again under one id is posted last.
    pub fn changes(&self) -> Vec<Change> {
        let removals = self.removing.iter().copied().map(Change::Remove);
        let posts = self
            .open
            .iter()
            .filter(|tracked| tracked.delivery == Delivery::Due)
            .copied()
            .map(Change::Post);
        removals.chain(posts).collect()
    }

    /// The caller made `change`: it is not handed over again. A change made
    /// stale meanwhile (the notice ended or was posted anew) is left as it is.
    pub fn confirm(&mut self, change: &Change) {
        match change {
            Change::Post(posted) => {
                if let Some(tracked) = self
                    .open
                    .iter_mut()
                    .find(|tracked| tracked.key == posted.key && tracked.delivery == Delivery::Due)
                {
                    tracked.delivery = Delivery::Posted;
                }
            }
            Change::Remove(key) => self.removing.retain(|removing| removing != key),
        }
    }

    /// The notices the app lists: open, not dismissed, oldest first.
    pub fn listed(&self) -> impl Iterator<Item = &Tracked> {
        self.open
            .iter()
            .filter(|tracked| tracked.delivery != Delivery::Dismissed)
    }

    /// Every notice this watch may have on the phone, for a feeder the phone
    /// forgets.
    pub fn every_key(&self) -> impl Iterator<Item = NoticeKey> + '_ {
        self.open
            .iter()
            .map(|tracked| tracked.key)
            .chain(self.removing.iter().copied())
    }

    /// Track `notice` under `key`: a new one posts unless muted or this
    /// phone's own joining; one tracked already takes the new words, and
    /// posts again only if its kind changed (an uncertain feed found failed,
    /// a meal with no record found missed), unless dismissed.
    fn track(&mut self, key: NoticeKey, notice: Notice, context: &Context<'_>) {
        let delivery = if context.mutes.mutes(notice.kind()) || is_own_join(&notice, context.own) {
            Delivery::Quiet
        } else {
            Delivery::Due
        };
        let Some(tracked) = self.open.iter_mut().find(|tracked| tracked.key == key) else {
            self.open.push(Tracked {
                key,
                notice,
                seen: context.now,
                delivery,
                absent: None,
            });
            return;
        };

        let kind_changed = tracked.notice.kind() != notice.kind();
        tracked.notice = notice;
        tracked.absent = None;
        if !kind_changed || tracked.delivery == Delivery::Dismissed {
            return;
        }
        // A posted notice whose new kind is muted comes off, as a mute does.
        let showing = matches!(tracked.delivery, Delivery::Due | Delivery::Posted);
        tracked.delivery = delivery;
        if showing && delivery == Delivery::Quiet {
            self.removal(key);
        }
    }

    /// End every tracked notice whose key `ends`, and take each off the
    /// phone whatever this watch thinks it shows: removing one that is not
    /// there costs nothing, and one left up would stay for good.
    fn end(&mut self, ends: impl Fn(&NoticeKey) -> bool) {
        let mut removed = Vec::new();
        self.open.retain(|tracked| {
            if !ends(&tracked.key) {
                return true;
            }
            removed.push(tracked.key);
            false
        });
        for key in removed {
            self.removal(key);
        }
    }

    fn removal(&mut self, key: NoticeKey) {
        if !self.removing.contains(&key) {
            self.removing.push(key);
        }
    }

    /// Past [`TRACKED_MAX`], the oldest reports end: a feeder that can't
    /// vouch for an end for long must not grow the file without bound.
    fn bound(&mut self) {
        while self.open.len() > TRACKED_MAX {
            let Some(oldest) = self
                .open
                .iter()
                .find(|tracked| matches!(tracked.notice, Notice::Alert { open } if !open.alert.kind().is_condition()))
                .map(|tracked| tracked.key)
            else {
                return;
            };
            self.end(|key| *key == oldest);
        }
    }
}

impl Tracked {
    /// Since when this alert, absent from the table now, has counted as
    /// absent, or `None` while its hold doesn't run: one that waits for the
    /// clock, while the feeder has no local time (`clock`), starts its hold
    /// at the first read with it.
    fn hold(&mut self, open: &OpenAlert, clock: FeederClock, now: Timestamp) -> Option<Timestamp> {
        if waits_for_clock(open.alert.kind()) && clock == FeederClock::Unknown {
            self.absent = Some(Absence::NoClock);
            return None;
        }
        match self.absent {
            Some(Absence::Since { since }) => Some(since),
            None | Some(Absence::NoClock) => {
                self.absent = Some(Absence::Since { since: now });
                Some(now)
            }
        }
    }
}

/// Whether the feeder has local time, as the plans need it: a valid clock
/// (`status`), and no `NoClock` in the table (`lists_no_clock`), which the
/// feeder opens also for a clock with no zone offset.
const fn local_time(status: FeederClock, lists_no_clock: bool) -> FeederClock {
    if lists_no_clock {
        FeederClock::Unknown
    } else {
        status
    }
}

/// A condition the feeder can't judge without a clock: the desiccant's days,
/// and plans stuck, whose threshold runs from the clock's arrival.
const fn waits_for_clock(kind: AlertKind) -> bool {
    matches!(kind, AlertKind::DesiccantDue | AlertKind::PlansStuck)
}

/// Whether an alert tables have left out since `absent`, though the feeder
/// could not vouch for its end, has ended anyway. A condition ends after
/// [`SETTLING_HOLD`]. A report ends once past the feeder's own lapse: by its
/// own time, or, with no valid clock when it was made, by how long tables
/// have left it out.
fn unvouched_over(open: &OpenAlert, absent: Timestamp, now: Timestamp) -> bool {
    if open.alert.kind().is_condition() {
        return now.duration_since(absent) > SETTLING_HOLD;
    }
    let since = open.at.unwrap_or(absent);
    now.duration_since(since) > REPORT_LIFE
}

fn is_own_join(notice: &Notice, own: Hex<4>) -> bool {
    matches!(
        notice,
        Notice::Alert { open } if matches!(open.alert, wellfed_model::Alert::ClientJoined { client, .. } if client == own)
    )
}

#[cfg(test)]
mod tests;
