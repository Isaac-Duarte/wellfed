//! Time for the feeder: the clock sync as the device API carries it
//! (`Op::TimeSync`), and the zone setting's rule (`Op::PutTime`).
//!
//! A sync carries the wall clock, the zone's offset now and its next
//! change, so a feeder without a zone setting keeps local time across one
//! DST change. The zone setting is better: a POSIX TZ rule, which the
//! feeder evaluates for any year without a zone database. The rule is the
//! footer of the zone's TZif file (RFC 8536) in the tzdb bundled here, so
//! the app, the CLI and the server send the same rule for the same name
//! whatever their host's own zone files say.
//!
//! A footer describes the zone only after the file's last listed
//! transition. A few zones list irregular changes years ahead (Morocco's
//! Ramadan changes run to 2087), so a rule is offered only once it agrees
//! with the full zone at every transition of the next ten years; such a
//! zone stays on the syncs' offsets.
//!
//! A plan's next meal ([`next_meal`]) is the feeder's own reckoning
//! (`wellfed_core_feeder::schedule`) run on the host's zone, so a skipped or
//! repeated minute lands where the feeder will feed.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use std::time::Duration;
use wellfed_core::clock::zone::{Rule, Zone};
use wellfed_core_feeder::schedule;

use serde::{Deserialize, Serialize};
use wellfed_model::{
    ClientRole, NextMeal, NtpResult, NtpServer, Plan, Plans, Status, TimeSource, TimeSync, TimeView,
};

/// How far ahead a rule must agree with its zone.
const AGREE_S: i64 = 10 * 366 * 86_400;

/// The sync for `zone` at `now`. A zone with no change ahead (UTC, or one
/// without DST) sends a next change of 0 with the current offset.
pub fn sync(now: Timestamp, zone: &TimeZone) -> TimeSync {
    let offset_s = zone.to_offset(now).seconds();
    let (next_change_s, next_offset_s) = zone
        .following(now)
        .next()
        .and_then(|transition| {
            let at = u64::try_from(transition.timestamp().as_second()).ok()?;
            Some((at, transition.offset().seconds()))
        })
        .unwrap_or((0, offset_s));
    TimeSync {
        unix_ms: u64::try_from(now.as_millisecond()).unwrap_or(0),
        offset_s,
        next_change_s,
        next_offset_s,
    }
}

/// The feeder's reported error at which an owner's client syncs a valid
/// clock that keeps no time server: half of [`MAX_ERROR_MS`], the error past
/// which it reads invalid.
///
/// The error grows at the rate bound from the last trusted fix: 5,000 ppm
/// before the rate is learned, so this comes about 3.3 hours after an
/// owner's sync and leaves as long again before the clock expires at about
/// 6.6. A client that checks in every few hours (the app's sessions, or
/// Android's periodic check, which Doze can space an hour or more apart)
/// so meets it before expiry. Each sync is measured against the first sync
/// of the same owner's key since the rate was forgotten, so that key's syncs
/// after about 11 hours of them teach the rate; the bound then drops to
/// 200 ppm, and the error reaches this about every 3.5 days.
///
/// A clock that asks a time server is left to it: NTP never re-anchors a
/// valid clock, so an owner's sync would, at the phone's time, and a phone
/// some seconds off would put the next answers outside the interval and
/// contest the clock.
///
/// [`MAX_ERROR_MS`]: wellfed_core::clock::wall::MAX_ERROR_MS
pub const OWNER_SYNC_ERROR_MS: u64 = wellfed_core::clock::wall::MAX_ERROR_MS / 2;

/// Room beside the feeder's own error for the trips a time view and a sync
/// each make across the LAN (a sync counts 1 s of its own): a client whose
/// clock is within the feeder's error and this of the feeder's agrees with it.
pub const AGREE_MARGIN_MS: u64 = 2_000;

/// How long a client watches a clock read invalid, while a time server is
/// named, before it sets the clock itself. Once the clock needs the time,
/// NTP asks within 64 s and may re-set it only inside the old interval; a
/// client's sync of an invalid clock is held to nothing. Two reads this far
/// apart that both find it invalid mean NTP did not set it.
pub const INVALID_WAIT_MS: i64 = 120_000;

/// What a client should do about the feeder's clock, by its time view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockCheck {
    Leave,
    Sync,
    /// The client's clock and the feeder's are further apart than the
    /// feeder's error allows: one of them is wrong, and nothing is sent by
    /// itself. An owner may still set the time by hand.
    Disagrees {
        /// The client's clock less the feeder's.
        client_ahead_ms: i64,
    },
    /// The clock reads invalid because the feeder's time server answered,
    /// and outside the interval the clock last trusted
    /// ([`server_disagrees`]): the server or the time it was last set to is
    /// wrong, and a client's sync, trusted while the clock is invalid, could
    /// be the wrong side. Nothing is sent by itself; an owner may still set
    /// the time by hand.
    ServerDisagrees,
}

/// Whether the clock reads invalid while its time server is answering
/// outside the interval it last trusted (`ntp_last: Bounded`): an expiry or
/// a contest the server's answers did not settle. A phone that synced the
/// clock seconds off and left NTP to miss would see this, and a sync of its
/// own would only start the contest again.
pub fn server_disagrees(view: &TimeView) -> bool {
    view.clock_ms.is_none()
        && view.ntp != NtpServer::Off
        && view.ntp_last == Some(NtpResult::Bounded)
}

/// How far the client's clock (`now`, as the view was read) is ahead of the
/// feeder's, when that is more than the feeder's reported error and
/// [`AGREE_MARGIN_MS`] allow; `None` when they agree or the clock is invalid.
pub fn clock_disagreement(view: &TimeView, now: Timestamp) -> Option<i64> {
    let clock = i64::try_from(view.clock_ms?).ok()?;
    let error = u64::from(view.error_ms?);
    let ahead = now.as_millisecond().saturating_sub(clock);
    (ahead.unsigned_abs() > error.saturating_add(AGREE_MARGIN_MS)).then_some(ahead)
}

/// What a client holding `role` should do on reading `view` at its own
/// `now`. `invalid_since` is when this client first found the clock invalid
/// in its run of reads that all did, this one included.
///
/// - A clock with no time server that reads invalid is set by anyone.
/// - One that asks a time server is set only once it has read invalid for
///   [`INVALID_WAIT_MS`], so NTP's own re-set at each expiry, held to the
///   old interval, comes first; and never while that server answers outside
///   the interval ([`server_disagrees`]).
/// - A valid clock is synced only by an owner, only with no time server,
///   and once its error reaches [`OWNER_SYNC_ERROR_MS`]: a member's sync
///   could only narrow it.
/// - Never one the client disagrees with ([`clock_disagreement`]).
pub fn check(
    view: &TimeView,
    role: ClientRole,
    now: Timestamp,
    invalid_since: Option<Timestamp>,
) -> ClockCheck {
    let no_server = view.ntp == NtpServer::Off;
    if server_disagrees(view) {
        return ClockCheck::ServerDisagrees;
    }
    let Some(_) = view.clock_ms else {
        let waited = invalid_since.is_some_and(|since| {
            now.as_millisecond().saturating_sub(since.as_millisecond()) >= INVALID_WAIT_MS
        });
        return if no_server || waited {
            ClockCheck::Sync
        } else {
            ClockCheck::Leave
        };
    };
    if let Some(client_ahead_ms) = clock_disagreement(view, now) {
        return ClockCheck::Disagrees { client_ahead_ms };
    }
    let grown = view
        .error_ms
        .is_some_and(|error| u64::from(error) >= OWNER_SYNC_ERROR_MS);
    if role == ClientRole::Owner && no_server && grown {
        ClockCheck::Sync
    } else {
        ClockCheck::Leave
    }
}

/// What a client keeps about its automatic syncs of one feeder, across its
/// sessions: when it last sent one, when it first saw the clock invalid in
/// its current run of reads, and the view a sync was refused on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoSync {
    /// When this client last sent an automatic sync, Unix milliseconds.
    pub sent_ms: Option<i64>,
    /// The run of invalid reads the clock is in, as this client saw it.
    pub invalid: Option<InvalidRun>,
    /// The view a sync was refused on (`Refused(Time)`): none goes again
    /// until the clock's view moves on from it, or the feeder restarts.
    pub refused: Option<Refused>,
}

/// How far the wall time between two reads of the status may run past the
/// uptime between them in one boot: the phone's clock and the feeder's
/// count drift apart a little, and each read takes its trip.
pub const BOOT_MARGIN_MS: i64 = 60_000;

/// Which boot of the feeder a client read the status in: its boot count and
/// uptime, and when the client read them (Unix milliseconds by its own
/// clock). The boot count lives in a backup register a power cut clears, so
/// the count alone repeats; the uptime tells those boots apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Boot {
    pub count: u32,
    pub uptime_ms: u32,
    pub read_ms: i64,
}

impl Boot {
    /// `status` as read at `at`.
    pub fn read(status: &Status, at: Timestamp) -> Self {
        Self {
            count: status.boot_count,
            uptime_ms: status.uptime_ms,
            read_ms: at.as_millisecond(),
        }
    }

    /// Whether `later` was read in the same boot as `self`: the same count,
    /// and no more wall time between the reads than uptime, give or take
    /// [`BOOT_MARGIN_MS`]. A restart in between, even one that cleared the
    /// count back to where it was, lost the uptime the feeder ran before it
    /// and the time it was off. The uptime is a `u32` of milliseconds, so it
    /// wraps every 49.7 days: an uptime that went down is the same boot only
    /// when the wrapped count ran about as long as the wall time, where a
    /// restart's lower uptime reads as weeks.
    pub fn same_as(&self, later: &Self) -> bool {
        let ran = i64::from(later.uptime_ms.wrapping_sub(self.uptime_ms));
        let wall = later.read_ms.saturating_sub(self.read_ms);
        let wrapped = later.uptime_ms < self.uptime_ms;

        self.count == later.count
            && wall <= ran + BOOT_MARGIN_MS
            && (!wrapped || ran <= wall + BOOT_MARGIN_MS)
    }
}

/// A run of reads that all found the clock invalid with no fix between them:
/// the first one's time, and the feeder's boot and sync count then, which a
/// restart or any fix since would have moved. Reads hours apart, Doze's, may
/// both find it invalid around a valid spell, or around a restart that
/// counted its fixes from 0 again; the boot and the count tell them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvalidRun {
    /// Unix milliseconds.
    pub since_ms: i64,
    pub boot: Boot,
    pub syncs: u32,
}

/// What tells one state of the feeder's clock from the next within a boot:
/// whether it reads valid, how many fixes it took and the last one's source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewMark {
    pub valid: bool,
    pub syncs: u32,
    pub last: Option<TimeSource>,
}

impl ViewMark {
    pub fn of(view: &TimeView) -> Self {
        Self {
            valid: view.clock_ms.is_some(),
            syncs: view.syncs,
            last: view.last,
        }
    }
}

/// A refused sync: the view it was sent on, and the boot it was read in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refused {
    pub on: ViewMark,
    pub boot: Boot,
}

impl AutoSync {
    /// What to do on reading `view` at `now`, with the status read as `boot`,
    /// as [`check`] says, but sending no sync within `spacing` of the last
    /// one sent, nor while a refused one's view stands in the same boot. It
    /// notes the clock's run of invalid reads.
    pub fn read(
        &mut self,
        view: &TimeView,
        boot: Boot,
        role: ClientRole,
        now: Timestamp,
        spacing: Duration,
    ) -> ClockCheck {
        let now_ms = now.as_millisecond();
        self.invalid = match (view.clock_ms, self.invalid) {
            (None, Some(run)) if run.boot.same_as(&boot) && run.syncs == view.syncs => {
                Some(InvalidRun { boot, ..run })
            }
            (None, _) => Some(InvalidRun {
                since_ms: now_ms,
                boot,
                syncs: view.syncs,
            }),
            (Some(_), _) => None,
        };
        if self
            .refused
            .is_some_and(|refused| refused.on != ViewMark::of(view) || !refused.boot.same_as(&boot))
        {
            self.refused = None;
        } else if let Some(refused) = &mut self.refused {
            refused.boot = boot;
        }

        let since = self
            .invalid
            .and_then(|run| Timestamp::from_millisecond(run.since_ms).ok());
        let spacing_ms = i64::try_from(spacing.as_millis()).unwrap_or(i64::MAX);
        // A last sync ahead of now is a client clock that stepped back:
        // its spacing says nothing.
        let rested = self
            .sent_ms
            .is_none_or(|sent| !(0..spacing_ms).contains(&now_ms.saturating_sub(sent)));
        match check(view, role, now, since) {
            ClockCheck::Sync if !rested || self.refused.is_some() => ClockCheck::Leave,
            other => other,
        }
    }

    /// An automatic sync went out at `now`.
    pub fn sent(&mut self, now: Timestamp) {
        self.sent_ms = Some(now.as_millisecond());
    }

    /// The feeder refused a sync sent on `view`, with the status read as
    /// `boot`, as out of range.
    pub fn refused(&mut self, view: &TimeView, boot: Boot) {
        self.refused = Some(Refused {
            on: ViewMark::of(view),
            boot,
        });
    }

    /// The later of what two clients' processes kept: the last sync either
    /// sent, and `newer`'s reading of the clock.
    #[must_use]
    pub fn merged(&self, newer: &Self) -> Self {
        Self {
            sent_ms: self.sent_ms.max(newer.sent_ms),
            ..newer.clone()
        }
    }
}

/// The zone names an owner picks from, sorted: tzdb's `zone.tab`, one zone
/// for each region of each country (`America/Denver`, `Europe/Amsterdam`),
/// those the bundled tzdb holds, and `UTC`. The older names `backward`
/// keeps (`US/Pacific`, `Asia/Calcutta`) and the POSIX-style ones
/// (`EST5EDT`, `Etc/GMT+3`) are not offered; [`rule`] still takes them.
pub fn zones() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = include_str!("zones.txt")
        .lines()
        .filter(|name| jiff_tzdb::get(name).is_some())
        .chain(["UTC"])
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// The earliest occurrence after `now` of any enabled plan in `plans`, in
/// `zone`; `None` when no plan is enabled. A tie goes to the lower slot,
/// which the feeder runs first. A skipped meal is not next: its plan's
/// next is the occurrence after it.
pub fn next_meal(plans: &Plans, now: Timestamp, zone: &TimeZone) -> Option<NextMeal> {
    let now_ms = u64::try_from(now.as_millisecond()).ok()?;
    let offset_at = offsets(zone);

    let mut next: Option<NextMeal> = None;
    for (slot, plan) in plans.plans.iter().enumerate() {
        let Some(plan) = plan.filter(|plan| plan.enabled) else {
            continue;
        };
        let Some(due) = plan_next(plans, slot, &plan, now_ms, offset_at) else {
            continue;
        };
        let Some(at) = i64::try_from(due)
            .ok()
            .and_then(|ms| Timestamp::from_millisecond(ms).ok())
        else {
            continue;
        };
        if next.is_none_or(|next| at < next.at) {
            next = Some(NextMeal {
                at,
                slot: u8::try_from(slot).ok()?,
                portions: plan.portions,
            });
        }
    }
    next
}

/// Slot `slot`'s next occurrence after `now_ms`, past the meal its skip
/// names. Consecutive occurrences are a day apart, so at most one is
/// skipped.
fn plan_next(
    plans: &Plans,
    slot: usize,
    plan: &Plan,
    now_ms: u64,
    offset_at: impl Fn(u64) -> Option<i32> + Copy,
) -> Option<u64> {
    let plan = core_plan(plan);
    let due = schedule::next_due(&plan, now_ms, offset_at)?;
    let skipped = plans
        .skipped
        .get(slot)
        .and_then(|at| u64::try_from(at.as_millisecond()).ok());
    if skipped == Some(due) {
        return schedule::next_due(&plan, due, offset_at);
    }
    Some(due)
}

/// The feeder's zone as its time settings give it: its POSIX rule, or the
/// offset a sync left it, or `fallback` (a host's own zone) with neither, when
/// the feeder has no idea of local time and the host's is the best guess.
pub fn feeder_zone(view: &TimeView, fallback: TimeZone) -> TimeZone {
    if let Some(zone) = view
        .zone
        .as_deref()
        .and_then(|rule| TimeZone::posix(rule).ok())
    {
        return zone;
    }
    view.offset_s
        .and_then(|seconds| jiff::tz::Offset::from_seconds(seconds).ok())
        .map_or(fallback, TimeZone::fixed)
}

/// `zone`'s offset at an instant in Unix milliseconds, as the feeder's
/// schedule asks for it.
pub(crate) fn offsets(zone: &TimeZone) -> impl Fn(u64) -> Option<i32> + Copy + '_ {
    |ms: u64| {
        let at = Timestamp::from_millisecond(i64::try_from(ms).ok()?).ok()?;
        Some(zone.to_offset(at).seconds())
    }
}

/// The model's plan as the feeder's schedule holds it: the same fields,
/// and the same weekday bits.
pub(crate) const fn core_plan(plan: &Plan) -> schedule::Plan {
    schedule::Plan {
        minute: plan.minute,
        portions: plan.portions,
        days: schedule::Weekdays(plan.days.bits()),
        enabled: plan.enabled,
        call: plan.call,
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RuleError {
    #[error("{0:?} is not a time zone in the bundled tzdb")]
    Unknown(String),
    #[error("{0}'s zone file has no POSIX rule")]
    NoRule(String),
    #[error("{name}'s rule {rule:?} is not one the feeder keeps")]
    Unsupported { name: String, rule: String },
    #[error("{name} changes irregularly at {at}: its rule {rule:?} does not describe it")]
    Irregular {
        name: String,
        rule: String,
        at: Timestamp,
    },
    /// The bundled rule agrees with the bundled zone but not with the
    /// host's: one of the two tzdb releases is out of date.
    #[error(
        "{name}'s rule {rule:?} disagrees with this host's time zone data at {at}: \
         one of the two is out of date"
    )]
    HostDisagrees {
        name: String,
        rule: String,
        at: Timestamp,
    },
}

/// The POSIX TZ rule the feeder keeps for the IANA zone `name` from
/// `now`, checked by the feeder's own parser, against the bundled zone and
/// against `host`, the host's own view of the zone if it has one (the
/// caller reads it: this crate does no IO).
pub fn rule(name: &str, now: Timestamp, host: Option<&TimeZone>) -> Result<String, RuleError> {
    let (canonical, tzif) =
        jiff_tzdb::get(name).ok_or_else(|| RuleError::Unknown(name.to_owned()))?;
    let footer = footer(tzif).ok_or_else(|| RuleError::NoRule(canonical.to_owned()))?;
    let rule = String::from_utf8_lossy(footer).into_owned();
    let Ok(zone) = Zone::parse(footer) else {
        return Err(RuleError::Unsupported {
            name: canonical.to_owned(),
            rule,
        });
    };

    // Against the bundled zone and the host's (which `sync` uses): two
    // tzdb releases can disagree about a zone's coming changes, and the
    // feeder must not keep a rule either one contradicts.
    let bundled =
        TimeZone::tzif(canonical, tzif).map_err(|_| RuleError::NoRule(canonical.to_owned()))?;
    if let Some(at) = disagreement(zone.rule(), &bundled, now) {
        return Err(RuleError::Irregular {
            name: canonical.to_owned(),
            rule,
            at,
        });
    }
    if let Some(at) = host.and_then(|host| disagreement(zone.rule(), host, now)) {
        return Err(RuleError::HostDisagrees {
            name: canonical.to_owned(),
            rule,
            at,
        });
    }
    Ok(rule)
}

/// The first instant in the next ten years where `rule` and `zone` differ,
/// checked at `now` and on both sides of each transition of either one.
/// Two offset functions that agree at both sides of every change of both
/// agree in between, where neither changes. The rule's own changes count:
/// a host tzdb newer than the bundled one may have dropped a zone's
/// daylight time, and a rule still keeping it then agrees with that zone
/// all winter and at none of the zone's (absent) transitions.
fn disagreement(rule: Rule, zone: &TimeZone, now: Timestamp) -> Option<Timestamp> {
    let end = now.as_second().saturating_add(AGREE_S);
    let differs = |t: i64| {
        let at = Timestamp::from_second(t).ok()?;
        (rule.offset_at(t) != zone.to_offset(at).seconds()).then_some(at)
    };
    // The earlier side of each change first, so the first found is the
    // earliest there.
    let around = |changes: &mut dyn Iterator<Item = i64>| {
        changes
            .take_while(|&t| t <= end)
            .find_map(|t| differs(t - 1).or_else(|| differs(t)))
    };
    if let Some(at) = differs(now.as_second()) {
        return Some(at);
    }

    let mut zone_changes = zone
        .following(now)
        .map(|transition| transition.timestamp().as_second());
    let mut rule_changes = core::iter::successors(rule.next_change(now.as_second()), |&(t, _)| {
        rule.next_change(t)
    })
    .map(|(t, _)| t);
    match (around(&mut zone_changes), around(&mut rule_changes)) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// A version 2+ TZif file ends with its POSIX rule between two newlines.
fn footer(tzif: &[u8]) -> Option<&[u8]> {
    let text = tzif.strip_suffix(b"\n")?;
    let start = text.iter().rposition(|&b| b == b'\n')? + 1;
    let footer = &text[start..];
    (!footer.is_empty()).then_some(footer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wellfed_core::clock::zone::TEXT_MAX;
    use wellfed_model::{Generation, PLANS, Weekdays};

    // 2026-10-26T00:00:00Z.
    const NOW: i64 = 1_793_059_200;
    /// Denver: 2026-03-08 02:00 MST jumps to 03:00 MDT at 09:00Z.
    const SPRING: i64 = 1_772_960_400;
    /// Denver: 2026-11-01 02:00 MDT falls back to 01:00 MST at 08:00Z.
    const FALL: i64 = 1_793_520_000;
    const HOUR: i64 = 3600;

    fn at(second: i64) -> Timestamp {
        Timestamp::from_second(second).unwrap()
    }

    fn daily(minute: u16, portions: u8) -> Plan {
        Plan {
            minute,
            portions,
            days: Weekdays::DAILY,
            enabled: true,
            call: 0,
        }
    }

    fn table(plans: &[(usize, Plan)]) -> Plans {
        let mut table = [None; PLANS];
        for &(slot, plan) in plans {
            table[slot] = Some(plan);
        }
        Plans {
            generation: Generation(1),
            writable: true,
            plans: table,
            skipped: wellfed_model::SkippedMeals::NONE,
        }
    }

    fn meal(plans: &Plans, now: i64) -> Option<(i64, u8)> {
        let denver = TimeZone::get("America/Denver").unwrap();
        next_meal(plans, at(now), &denver).map(|meal| (meal.at.as_second(), meal.slot))
    }

    /// The minute the spring change skips runs as far past the change; the
    /// one the fall change repeats runs on its first pass, so after it the
    /// next is the day after's. The feeder's own schedule tests pin the
    /// same instants.
    #[test]
    fn the_next_meal_follows_the_daylight_rules() {
        let skipped = table(&[(0, daily(2 * 60 + 30, 1))]);
        assert_eq!(meal(&skipped, SPRING - HOUR), Some((SPRING + 30 * 60, 0)));

        let repeated = table(&[(3, daily(90, 1))]);
        let first = FALL - 30 * 60;
        assert_eq!(meal(&repeated, first - 1), Some((first, 3)));
        assert_eq!(
            meal(&repeated, first),
            Some((FALL + 30 * 60 + 24 * HOUR, 3)),
            "01:30 MST on November 2"
        );
    }

    #[test]
    fn the_earliest_enabled_plan_is_next() {
        let disabled = Plan {
            enabled: false,
            ..daily(6 * 60, 4)
        };
        let plans = table(&[(0, daily(18 * 60, 2)), (1, disabled), (2, daily(7 * 60, 1))]);
        // Midnight MDT; 07:00 MDT is 13:00Z.
        let midnight = NOW + 6 * HOUR;
        let next = next_meal(
            &plans,
            at(midnight),
            &TimeZone::get("America/Denver").unwrap(),
        );
        assert_eq!(
            next,
            Some(NextMeal {
                at: at(midnight + 7 * HOUR),
                slot: 2,
                portions: 1,
            })
        );

        assert_eq!(meal(&table(&[(1, disabled)]), NOW), None);
        assert_eq!(meal(&table(&[]), NOW), None);
    }

    /// Plans due at the same instant run lowest slot first, so that one
    /// is named.
    #[test]
    fn a_tie_goes_to_the_lower_slot() {
        let plans = table(&[(5, daily(7 * 60, 3)), (2, daily(7 * 60, 1))]);
        let midnight = NOW + 6 * HOUR;
        assert_eq!(meal(&plans, midnight), Some((midnight + 7 * HOUR, 2)));
    }

    /// An enabled plan that runs once occurs at its next minute, as the
    /// feeder runs it; once it ran, the feeder disables it.
    #[test]
    fn a_plan_that_runs_once_is_next_at_its_minute() {
        let once = Plan {
            days: Weekdays::ONCE,
            ..daily(7 * 60, 1)
        };
        let midnight = NOW + 6 * HOUR;
        let plans = table(&[(0, once)]);
        assert_eq!(meal(&plans, midnight), Some((midnight + 7 * HOUR, 0)));
        assert_eq!(
            meal(&plans, midnight + 8 * HOUR),
            Some((midnight + 31 * HOUR, 0)),
            "past its minute today, tomorrow's"
        );
        let ran = Plan {
            enabled: false,
            ..once
        };
        assert_eq!(meal(&table(&[(0, ran)]), midnight), None);
    }

    /// A skipped meal is not next, of its plan or of all of them: the
    /// plan's next is its following day's, and another plan's meal comes
    /// first. Once the skipped meal's time has passed, the skip no longer
    /// moves anything.
    #[test]
    fn a_skipped_meal_is_not_next() {
        let midnight = NOW + 6 * HOUR;
        let breakfast = midnight + 7 * HOUR;
        let mut plans = table(&[(0, daily(7 * 60, 1)), (1, daily(18 * 60, 2))]);
        let mut skipped = [None; PLANS];
        skipped[0] = Some(u32::try_from(breakfast).unwrap());
        plans.skipped = wellfed_model::SkippedMeals::from_seconds(skipped);

        assert_eq!(meal(&plans, midnight), Some((midnight + 18 * HOUR, 1)));
        let alone = Plans {
            plans: table(&[(0, daily(7 * 60, 1))]).plans,
            ..plans
        };
        assert_eq!(meal(&alone, midnight), Some((breakfast + 24 * HOUR, 0)));
        assert_eq!(
            meal(&alone, breakfast + HOUR),
            Some((breakfast + 24 * HOUR, 0)),
            "past it, the next is tomorrow's anyway"
        );
    }

    /// The case the server's former time request pinned.
    #[test]
    fn denver_carries_its_next_change_and_utc_none() {
        let now = Timestamp::from_millisecond(1_792_674_000_000).unwrap();
        let denver = TimeZone::get("America/Denver").unwrap();
        assert_eq!(
            sync(now, &denver),
            TimeSync {
                unix_ms: 1_792_674_000_000,
                offset_s: -21_600,
                next_change_s: 1_793_520_000,
                next_offset_s: -25_200,
            }
        );
        let utc = sync(now, &TimeZone::UTC);
        assert_eq!(
            (utc.offset_s, utc.next_change_s, utc.next_offset_s),
            (0, 0, 0)
        );
    }

    #[test]
    fn rules_come_from_the_bundled_tzdb() {
        let now = Timestamp::from_second(NOW).unwrap();
        assert_eq!(
            rule("America/Denver", now, None).unwrap(),
            "MST7MDT,M3.2.0,M11.1.0"
        );
        assert_eq!(rule("Asia/Kolkata", now, None).unwrap(), "IST-5:30");
        assert_eq!(
            rule("Mars/Olympus", now, None),
            Err(RuleError::Unknown("Mars/Olympus".to_owned()))
        );
    }

    /// A rule that misses a change the zone lists is caught at it.
    #[test]
    fn a_rule_that_misses_a_change_disagrees_there() {
        let now = Timestamp::from_second(NOW).unwrap();
        let berlin = TimeZone::get("Europe/Berlin").unwrap();
        let winter_only = Zone::parse(b"CET-1").unwrap().rule();
        // 2027-03-28T01:00:00Z, when Berlin moves to summer time.
        assert_eq!(
            disagreement(winter_only, &berlin, now),
            Some(Timestamp::from_second(1_806_195_600).unwrap())
        );
        let full = Zone::parse(b"CET-1CEST,M3.5.0,M10.5.0/3").unwrap().rule();
        assert_eq!(disagreement(full, &berlin, now), None);
    }

    /// A host whose newer tzdb dropped a zone's daylight time has no
    /// transitions ahead, so only the rule's own changes show the rule
    /// moving the clock an hour the host never will, from winter or summer.
    #[test]
    fn a_rule_keeping_daylight_time_the_host_dropped_disagrees_at_its_change() {
        let cst = TimeZone::fixed(jiff::tz::Offset::from_seconds(-6 * 3600).unwrap());
        let mexico = Zone::parse(b"CST6CDT,M4.1.0,M10.5.0").unwrap().rule();
        // 2027-01-04, in winter: CDT would begin 2027-04-04 at 08:00Z.
        let winter = at(NOW + 70 * 86_400);
        assert_eq!(disagreement(mexico, &cst, winter), Some(at(1_806_825_600)));

        // Through the public call, Berlin against a host with no summer
        // time, checked in December.
        let cet = TimeZone::fixed(jiff::tz::Offset::from_seconds(3600).unwrap());
        let december = at(1_796_083_200);
        assert!(matches!(
            rule("Europe/Berlin", december, Some(&cet)),
            Err(RuleError::HostDisagrees { .. })
        ));
        // In summer the rule already differs.
        let july = at(1_814_400_000);
        assert_eq!(
            disagreement(
                Zone::parse(b"CET-1CEST,M3.5.0,M10.5.0/3").unwrap().rule(),
                &cet,
                july
            ),
            Some(july)
        );
    }

    /// A fixed rule against a fixed zone of the same offset, and a rule
    /// against its own zone in the southern hemisphere, agree.
    #[test]
    fn rules_that_match_their_zone_agree() {
        let now = Timestamp::from_second(NOW).unwrap();
        let ist = TimeZone::fixed(jiff::tz::Offset::from_seconds(19_800).unwrap());
        assert_eq!(
            disagreement(Zone::parse(b"IST-5:30").unwrap().rule(), &ist, now),
            None
        );
        let sydney = TimeZone::get("Australia/Sydney").unwrap();
        let rule = Zone::parse(b"AEST-10AEDT,M10.1.0,M4.1.0/3").unwrap().rule();
        assert_eq!(disagreement(rule, &sydney, now), None);
    }

    /// 2026-10-26T00:00:00Z in milliseconds, as a client reads it.
    const NOW_MS: u64 = 1_793_059_200_000;
    /// The feeder's boot count the views are read in.
    const BOOT: u32 = 7;
    /// The feeder's uptime at [`NOW_MS`].
    const UPTIME_MS: u32 = 3_600_000;

    /// The status of boot [`BOOT`] as read at `at`: its uptime runs with
    /// the wall time.
    fn boot_at(at: Timestamp) -> Boot {
        let since = at.as_millisecond() - now().as_millisecond();
        Boot {
            count: BOOT,
            uptime_ms: u32::try_from(i64::from(UPTIME_MS) + since).unwrap(),
            read_ms: at.as_millisecond(),
        }
    }

    /// A time view read at [`NOW_MS`]: the feeder's clock `clock_ms` with
    /// error `error_ms`, asking `ntp`.
    fn view(clock_ms: Option<u64>, error_ms: Option<u32>, ntp: NtpServer) -> TimeView {
        TimeView {
            generation: Generation(1),
            writable: true,
            zone: None,
            ntp,
            clock_ms,
            error_ms,
            offset_s: None,
            rate_ppb: 0,
            rate_bound_ppb: 5_000_000,
            learned: false,
            syncs: 1,
            last: Some(TimeSource::Owner),
            last_age_s: Some(0),
            ntp_last: None,
        }
    }

    const OFF: NtpServer = NtpServer::Off;
    const GATEWAY: NtpServer = NtpServer::Gateway { port: 123 };

    fn now() -> Timestamp {
        Timestamp::from_millisecond(i64::try_from(NOW_MS).unwrap()).unwrap()
    }

    fn later(ms: i64) -> Timestamp {
        Timestamp::from_millisecond(now().as_millisecond() + ms).unwrap()
    }

    /// With no time server, anyone sets an invalid clock, and only an owner
    /// re-sets a valid one, once its error reaches half the validity limit.
    #[test]
    fn with_no_time_server_an_owner_keeps_the_clock_and_anyone_sets_it() {
        let half = u32::try_from(OWNER_SYNC_ERROR_MS).unwrap();
        let invalid = view(None, None, OFF);
        for role in [ClientRole::Owner, ClientRole::Member, ClientRole::Server] {
            assert_eq!(
                check(&invalid, role, now(), None),
                ClockCheck::Sync,
                "{role:?}"
            );
        }

        let fresh = view(Some(NOW_MS), Some(1_000), OFF);
        let grown = view(Some(NOW_MS), Some(half), OFF);
        assert_eq!(
            check(&fresh, ClientRole::Owner, now(), None),
            ClockCheck::Leave
        );
        assert_eq!(
            check(
                &view(Some(NOW_MS), Some(half - 1), OFF),
                ClientRole::Owner,
                now(),
                None
            ),
            ClockCheck::Leave
        );
        assert_eq!(
            check(&grown, ClientRole::Owner, now(), None),
            ClockCheck::Sync
        );
        for role in [ClientRole::Member, ClientRole::Server, ClientRole::Unknown] {
            assert_eq!(
                check(&grown, role, now(), None),
                ClockCheck::Leave,
                "{role:?}"
            );
        }
    }

    /// NTP never re-anchors a valid clock, so an owner's sync would, at the
    /// phone's time: a clock asking a time server is left to it, however
    /// far its error grew.
    #[test]
    fn a_clock_asking_a_time_server_is_left_to_it_while_valid() {
        let half = u32::try_from(OWNER_SYNC_ERROR_MS).unwrap();
        let grown = view(Some(NOW_MS), Some(half), GATEWAY);
        assert_eq!(
            check(&grown, ClientRole::Owner, now(), None),
            ClockCheck::Leave
        );
    }

    /// With a time server, an invalid clock is NTP's to re-set within its
    /// old interval first: a client sets it only once two reads two minutes
    /// apart both find it invalid.
    #[test]
    fn with_a_time_server_an_invalid_clock_waits_two_minutes() {
        let invalid = view(None, None, GATEWAY);
        let role = ClientRole::Member;
        assert_eq!(check(&invalid, role, now(), None), ClockCheck::Leave);
        assert_eq!(check(&invalid, role, now(), Some(now())), ClockCheck::Leave);
        let first = later(-INVALID_WAIT_MS + 1);
        assert_eq!(check(&invalid, role, now(), Some(first)), ClockCheck::Leave);
        let first = later(-INVALID_WAIT_MS);
        assert_eq!(check(&invalid, role, now(), Some(first)), ClockCheck::Sync);
    }

    /// A client further from the feeder than the feeder's error allows
    /// sends nothing by itself, whichever of the two is wrong.
    #[test]
    fn a_client_that_disagrees_with_the_clock_sends_nothing() {
        let half = u32::try_from(OWNER_SYNC_ERROR_MS).unwrap();
        // The feeder claims 61 s; the phone is 64 s ahead of it.
        let grown = view(Some(NOW_MS - 64_000), Some(half + 1_000), OFF);
        assert_eq!(
            check(&grown, ClientRole::Owner, now(), None),
            ClockCheck::Disagrees {
                client_ahead_ms: 64_000
            }
        );
        assert_eq!(clock_disagreement(&grown, now()), Some(64_000));

        // Within its error and the margin, it agrees.
        let close = view(Some(NOW_MS + 62_000), Some(half + 1_000), OFF);
        assert_eq!(clock_disagreement(&close, now()), None);
        assert_eq!(
            check(&close, ClientRole::Owner, now(), None),
            ClockCheck::Sync
        );
    }

    /// At the unlearned bound the error reaches the sync point about 3.3
    /// hours after a fix, halfway to expiry: the cadence `OWNER_SYNC_ERROR_MS`
    /// describes.
    #[test]
    fn the_owner_s_sync_comes_halfway_to_expiry() {
        use wellfed_core::clock::wall::{LEARNED_FLOOR_PPM, MAX_ERROR_MS, UNLEARNED_PPM};

        let after_s = |ppm: u32| OWNER_SYNC_ERROR_MS * 1000 / u64::from(ppm);
        assert_eq!(OWNER_SYNC_ERROR_MS * 2, MAX_ERROR_MS);
        assert_eq!(after_s(UNLEARNED_PPM), 12_000, "3.3 hours");
        assert_eq!(after_s(LEARNED_FLOOR_PPM), 300_000, "3.5 days");
    }

    /// The memo notes the run of invalid reads, so the two-minute wait counts
    /// from the first, and a new run starts once the clock read valid.
    #[test]
    fn the_memo_counts_the_wait_from_the_first_invalid_read() {
        let mut memo = AutoSync::default();
        let invalid = view(None, None, GATEWAY);
        let spacing = Duration::from_secs(300);
        let role = ClientRole::Member;
        assert_eq!(
            memo.read(&invalid, boot_at(now()), role, now(), spacing),
            ClockCheck::Leave
        );
        assert_eq!(
            memo.read(
                &invalid,
                boot_at(later(60_000)),
                role,
                later(60_000),
                spacing
            ),
            ClockCheck::Leave
        );
        assert_eq!(
            memo.read(
                &invalid,
                boot_at(later(INVALID_WAIT_MS)),
                role,
                later(INVALID_WAIT_MS),
                spacing
            ),
            ClockCheck::Sync
        );

        // NTP set it, and it lapsed again: the wait starts over.
        let valid = view(Some(NOW_MS), Some(1_000), GATEWAY);
        memo.read(
            &valid,
            boot_at(later(INVALID_WAIT_MS)),
            role,
            later(INVALID_WAIT_MS),
            spacing,
        );
        assert_eq!(memo.invalid, None);
        assert_eq!(
            memo.read(
                &invalid,
                boot_at(later(INVALID_WAIT_MS + 1_000)),
                role,
                later(INVALID_WAIT_MS + 1_000),
                spacing
            ),
            ClockCheck::Leave
        );
    }

    /// Two reads hours apart that both find the clock invalid, with a fix
    /// between them that no read saw: the sync count moved, so the second
    /// starts a run of its own rather than ending a long one.
    #[test]
    fn a_fix_between_two_invalid_reads_starts_a_new_run() {
        let mut memo = AutoSync::default();
        let invalid = view(None, None, GATEWAY);
        let spacing = Duration::from_secs(300);
        let role = ClientRole::Owner;
        memo.read(&invalid, boot_at(now()), role, now(), spacing);
        let fixed_since = TimeView {
            syncs: 2,
            ..invalid.clone()
        };
        let hours = later(7 * 3_600_000);
        assert_eq!(
            memo.read(&fixed_since, boot_at(hours), role, hours, spacing),
            ClockCheck::Leave
        );
        assert_eq!(
            memo.invalid,
            Some(InvalidRun {
                since_ms: hours.as_millisecond(),
                boot: boot_at(hours),
                syncs: 2
            })
        );
    }

    /// A restart counts the clock's fixes from 0 again, and a power cut
    /// clears the boot count too, so a read in the next boot may show the
    /// same count and syncs as before it. The uptime tells them apart: a
    /// lower one, or more wall time since the last read than the uptime
    /// ran. Either starts a run of its own and ends a refusal.
    #[test]
    fn a_restart_starts_a_new_run_and_ends_a_refusal() {
        let invalid = view(None, None, GATEWAY);
        let spacing = Duration::from_secs(300);
        let role = ClientRole::Member;
        let then = later(INVALID_WAIT_MS);
        let restarts = [
            // A power cut: the count back where it was, the uptime lower.
            (
                "lower uptime",
                Boot {
                    uptime_ms: 30_000,
                    ..boot_at(then)
                },
            ),
            // Off a while, then up longer than before: the uptime is
            // higher, but by less than the wall time since.
            (
                "wall time past uptime",
                Boot {
                    uptime_ms: UPTIME_MS + 30_000,
                    ..boot_at(then)
                },
            ),
            (
                "another count",
                Boot {
                    count: BOOT + 1,
                    ..boot_at(then)
                },
            ),
        ];
        for (why, restarted) in restarts {
            let mut memo = AutoSync::default();
            memo.read(&invalid, boot_at(now()), role, now(), spacing);
            memo.sent(now());
            memo.refused(&invalid, boot_at(now()));

            assert_eq!(
                memo.read(&invalid, restarted, role, then, spacing),
                ClockCheck::Leave,
                "{why}: a new run waits its two minutes"
            );
            assert_eq!(memo.refused, None, "{why}");
            assert_eq!(
                memo.invalid,
                Some(InvalidRun {
                    since_ms: then.as_millisecond(),
                    boot: restarted,
                    syncs: 1
                }),
                "{why}"
            );
        }

        // The same boot, read as the uptime ran, keeps the run going.
        let mut memo = AutoSync::default();
        memo.read(&invalid, boot_at(now()), role, now(), spacing);
        assert_eq!(
            memo.read(&invalid, boot_at(then), role, then, spacing),
            ClockCheck::Sync
        );
    }

    /// The uptime is a `u32` of milliseconds and wraps after 49.7 days, so
    /// a lower one is the same boot when the wrapped count ran as long as
    /// the wall time, and a restart when it would have run for weeks.
    #[test]
    fn an_uptime_that_wraps_is_the_same_boot() {
        let before = Boot {
            count: BOOT,
            uptime_ms: u32::MAX - 10_000,
            read_ms: 0,
        };
        let wrapped = Boot {
            uptime_ms: 50_000,
            read_ms: 60_000,
            ..before
        };
        let running = Boot {
            uptime_ms: UPTIME_MS,
            ..before
        };
        let restarted = Boot {
            uptime_ms: 50_000,
            read_ms: 60_000,
            ..before
        };

        assert!(before.same_as(&wrapped));
        assert!(!running.same_as(&restarted));
    }

    /// A time server that answers outside the interval the clock trusted
    /// has the clock read invalid: no client sets it by itself then, owner
    /// or member, however long it has read invalid. With no server named,
    /// or a server that is silent, the usual rule stands.
    #[test]
    fn an_invalid_clock_its_server_disagrees_with_is_left_alone() {
        let contested = TimeView {
            ntp_last: Some(NtpResult::Bounded),
            ..view(None, None, GATEWAY)
        };
        let long_ago = Some(later(-10 * INVALID_WAIT_MS));
        assert!(server_disagrees(&contested));
        for role in [ClientRole::Owner, ClientRole::Member] {
            assert_eq!(
                check(&contested, role, now(), long_ago),
                ClockCheck::ServerDisagrees,
                "{role:?}"
            );
        }

        let silent = TimeView {
            ntp_last: Some(NtpResult::Lost),
            ..contested.clone()
        };
        assert_eq!(
            check(&silent, ClientRole::Member, now(), long_ago),
            ClockCheck::Sync
        );
        let no_server = TimeView {
            ntp: OFF,
            ..contested.clone()
        };
        assert_eq!(
            check(&no_server, ClientRole::Member, now(), None),
            ClockCheck::Sync
        );
        let valid = TimeView {
            ntp_last: Some(NtpResult::Bounded),
            ..view(Some(NOW_MS), Some(1_000), GATEWAY)
        };
        assert!(
            !server_disagrees(&valid),
            "a valid clock bounded the answer"
        );
    }

    /// One sync each spacing, whether or not it landed; a clock stepped back
    /// below the last sync waits for nothing.
    #[test]
    fn the_memo_spaces_its_syncs() {
        let mut memo = AutoSync::default();
        let invalid = view(None, None, OFF);
        let spacing = Duration::from_secs(300);
        let role = ClientRole::Member;
        assert_eq!(
            memo.read(&invalid, boot_at(now()), role, now(), spacing),
            ClockCheck::Sync
        );
        memo.sent(now());
        assert_eq!(
            memo.read(
                &invalid,
                boot_at(later(299_999)),
                role,
                later(299_999),
                spacing
            ),
            ClockCheck::Leave
        );
        assert_eq!(
            memo.read(
                &invalid,
                boot_at(later(300_000)),
                role,
                later(300_000),
                spacing
            ),
            ClockCheck::Sync
        );
        assert_eq!(
            memo.read(&invalid, boot_at(later(-1)), role, later(-1), spacing),
            ClockCheck::Sync
        );
    }

    /// A refused sync is not sent again on the same view, only once the
    /// clock moved on (another fix, or a change of validity).
    #[test]
    fn a_refused_sync_waits_for_the_view_to_change() {
        let mut memo = AutoSync::default();
        let invalid = view(None, None, OFF);
        let spacing = Duration::from_secs(300);
        let role = ClientRole::Member;
        memo.sent(now());
        memo.refused(&invalid, boot_at(now()));
        assert_eq!(
            memo.read(
                &invalid,
                boot_at(later(3_600_000)),
                role,
                later(3_600_000),
                spacing
            ),
            ClockCheck::Leave
        );

        let moved = TimeView {
            syncs: 2,
            ..invalid.clone()
        };
        assert_eq!(
            memo.read(
                &moved,
                boot_at(later(3_600_000)),
                role,
                later(3_600_000),
                spacing
            ),
            ClockCheck::Sync
        );
        assert_eq!(memo.refused, None);
    }

    /// Two processes' memos keep the later sync and the newer reading.
    #[test]
    fn merged_memos_keep_the_later_sync() {
        let old = AutoSync {
            sent_ms: Some(2),
            invalid: Some(InvalidRun {
                since_ms: 1,
                boot: boot_at(now()),
                syncs: 1,
            }),
            refused: None,
        };
        let newer = AutoSync {
            sent_ms: Some(1),
            invalid: None,
            refused: None,
        };
        let merged = old.merged(&newer);
        assert_eq!(merged.sent_ms, Some(2));
        assert_eq!(merged.invalid, None);
    }

    /// The names offered are sorted and unique, hold no `backward` alias,
    /// and each one makes a rule.
    #[test]
    fn every_zone_offered_is_a_place_with_a_rule() {
        let now = Timestamp::from_second(NOW).unwrap();
        let names = zones();
        assert!(names.len() > 300, "{} zones", names.len());
        assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
        for kept in ["America/Denver", "Europe/Amsterdam", "Asia/Kolkata", "UTC"] {
            assert!(names.contains(&kept), "{kept}");
        }
        for alias in [
            "US/Pacific",
            "Asia/Calcutta",
            "America/Buenos_Aires",
            "Europe/Kiev",
            "EST5EDT",
            "Etc/GMT+3",
        ] {
            assert!(!names.contains(&alias), "{alias}");
        }

        let refused: Vec<&str> = names
            .iter()
            .copied()
            .filter(|name| rule(name, now, None).is_err())
            .collect();
        assert!(refused.len() <= 4, "{refused:?}");
    }

    /// Nearly every name the list holds is one the bundled tzdb knows, so a
    /// list from another tzdb release loses no more than a rename or two.
    #[test]
    fn the_list_names_zones_the_bundle_holds() {
        let listed = include_str!("zones.txt").lines().count();
        assert!(zones().len() + 3 >= listed, "{} of {listed}", zones().len());
    }

    /// Every zone's rule fits the feeder and agrees with jiff at every
    /// transition of the next ten years, but for the few zones that list
    /// irregular changes ahead.
    #[test]
    fn every_zone_rule_fits_and_agrees_with_jiff() {
        let now = Timestamp::from_second(NOW).unwrap();
        let (mut regular, mut irregular) = (0, Vec::new());
        for name in jiff_tzdb::available() {
            let host = TimeZone::get(name).ok();
            match rule(name, now, host.as_ref()) {
                Ok(rule) => {
                    assert!(rule.len() <= TEXT_MAX, "{name}: {rule}");
                    regular += 1;
                }
                Err(RuleError::Irregular { name, .. } | RuleError::HostDisagrees { name, .. }) => {
                    irregular.push(name);
                }
                Err(error) => panic!("{error}"),
            }
        }
        assert!(regular > 300, "{regular} zones");
        assert!(irregular.len() <= 4, "{irregular:?}");
    }
}
