//! Manual feeding in the loop: whether a feed can start now, how a feed is
//! queued (a Dispense press and a client's `Feed` alike).
//!
//! **Readiness** is one decision ([`readiness`]): the feeding owner's own
//! reason not to feed ([`Feeds::blocked`]), then BLE setup mode (it ends in
//! a restart, which would leave a feed uncertain) and an install (it holds
//! the flash), then the brownout hold ([`super::supply`]). The owner's own
//! reasons include the battery pack below its floor while the adapter is
//! out (`Blocked::Battery`). The status reports it, a feed is asked for only while it is
//! ready, and the owner's own checks stay behind it for a feed that lost a
//! race (a join that began in between). A change of a lasting reason
//! ([`Blocked::lasting`]) is recorded in the history as a `FeedingBlocked`
//! event; every change goes to the log.
//!
//! **One path.** A Dispense press asks for one portion, as stock's does,
//! through [`queue`], which a client's `Feed` and a plan's also take: the
//! same readiness, the same owner, the same `Fed` event (by the panel)
//! with the portions asked for and those delivered. A press while a feed
//! cannot start (one running among the reasons) starts nothing and is not
//! queued; stock logs it (record code 2022), and here a press is a
//! `DispenseRefused` event ([`Watch::refused`]) when its reason differs from
//! the last one recorded since the feeder was last ready, so a child at the
//! panel cannot fill the event queue during a long feed, yet a feed that
//! ends straight into another reason (the battery hold, a Wi-Fi join) is
//! still recorded. Back-to-back feeds share the reason `Feeding`, so presses
//! across them record once. Every press goes to the log.
//!
//! **A claim behind writes.** A feed that finds storage writes ahead of it is
//! accepted and numbered, and its claim on the outputs is tried once a pass
//! ([`Feeds::claim_step`]) for at most `WRITE_WAIT_MS` (5 s), the loop serving everything else
//! meanwhile. So a `queue` that returns a number has not started anything yet:
//! a client's answer waits for the reservation, as it always did, and a press
//! or a plan hears a late refusal through [`super::Feeder`]'s own pass, with
//! the same consequences an immediate one has.
//!
//! **Food level.** The food beam is lit during feeds and in a short window at
//! rest (`wellfed_core_feeder::feed::food_read`), so its level is the latest of
//! either's. With no feed in flight, a level that differs from the last one
//! seen this boot is a `Food` event ([`Watch::food`]): at most one a feed or a
//! window, however the beam flickers as the last kibble passes. A boot's first
//! reading records only if a feed took it (one refused while it waited for
//! its claim never lit the beam): a first window can't tell whether
//! anything happened to the hopper, so a change made while the feeder was off
//! shows in the status and the light, not in History, and each restart costs
//! neither a History line nor a flash write. No feed waits on it or is refused for it, as stock.
//!
//! **Silent.** A feed plays no tune; that is the owner's choice. A tune a
//! client asks for with `play` is refused while a feed is in flight, but one
//! already sounding plays on through a feed that comes after it. It is
//! stepped after the owner and the device API in each pass, and no feed
//! decision reads the speaker, so a speaker that is busy, will not open or
//! stalls does not hold back a feed's start, its stop or its terminal
//! result. Each speaker step still takes its time inside the pass, which
//! counts against the gap allowed between the owner's steps.

use core::fmt;

use wellfed_proto::api::{
    Blocked, EventBody, FoodLevel, PORTIONS_MAX, PORTIONS_MIN, Readiness, Refusal, Source,
};

use crate::feed;

use super::Feeds;
use super::events;
use super::platform::Platform;

/// What a Dispense press asks for: one portion, as stock.
pub const DISPENSE_PORTIONS: u8 = 1;

/// Whether a feed can start now. The owner's lasting reasons come first,
/// since they outlast setup mode and an install; then a restart that was
/// asked for (`restarting`), which would cut a feed started now; then setup
/// mode and an install; then the owner's passing ones; then a brownout's
/// `battery_hold`.
pub fn readiness<F: Feeds>(
    feeds: &F,
    restarting: bool,
    setup: bool,
    installing: bool,
    battery_hold: bool,
) -> Readiness {
    let blocked = match feeds.blocked() {
        Some(blocked) if blocked.lasting() || blocked == Blocked::Unserved => Some(blocked),
        _ if restarting => Some(Blocked::Restarting),
        _ if setup => Some(Blocked::Setup),
        _ if installing => Some(Blocked::Updating),
        None if battery_hold => Some(Blocked::Battery),
        owner => owner,
    };
    blocked.map_or(Readiness::Ready, Readiness::Blocked)
}

/// `ready`, unless a plan's claimed feed must be the next one (`meal`):
/// then a feed is being prepared.
pub const fn held(ready: Readiness, meal: bool) -> Readiness {
    match ready {
        Readiness::Ready if meal => Readiness::Blocked(Blocked::Feeding),
        ready => ready,
    }
}

/// Why a feed asked for while ready must not claim the outputs now, of the
/// reasons that can arise while it waits. The owner's own are behind this: it
/// checks them as it takes the feed.
pub const fn claim_hold(
    restarting: bool,
    setup: bool,
    installing: bool,
    battery_hold: bool,
) -> Option<Blocked> {
    if restarting {
        Some(Blocked::Restarting)
    } else if setup {
        Some(Blocked::Setup)
    } else if installing {
        Some(Blocked::Updating)
    } else if battery_hold {
        Some(Blocked::Battery)
    } else {
        None
    }
}

/// Queue one feed of `portions` for `by` while `ready` allows it. The
/// number the journal holds the feed under, or why it was refused:
/// portions outside [`PORTIONS_MIN`] to [`PORTIONS_MAX`] first, by name,
/// whoever asks.
pub fn queue<F: Feeds>(
    feeds: &mut F,
    ready: Readiness,
    portions: u8,
    by: Source,
) -> Result<u32, Refusal> {
    if !(PORTIONS_MIN..=PORTIONS_MAX).contains(&portions) {
        return Err(Refusal::Portions { asked: portions });
    }
    if let Readiness::Blocked(blocked) = ready {
        return Err(blocked.refusal());
    }
    feeds.submit(portions, by)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Record {
    /// Readiness changed.
    Readiness(Readiness),
    /// A Dispense press asked for a feed.
    Dispense { result: Result<u32, Refusal> },
}

/// One record's body (`PLFEED1`).
pub fn describe(record: Record, f: &mut impl fmt::Write) -> fmt::Result {
    match record {
        Record::Readiness(Readiness::Ready) => f.write_str("readiness state=ready"),
        Record::Readiness(Readiness::Blocked(blocked)) => {
            write!(
                f,
                "readiness state=blocked reason={}",
                blocked_name(blocked)
            )
        }
        Record::Dispense { result: Ok(feed) } => write!(f, "dispense feed={feed}"),
        Record::Dispense {
            result: Err(refusal),
        } => {
            write!(f, "dispense refused={}", refusal_name(refusal))
        }
    }
}

/// Why a refused Dispense press could not feed: the reason of the
/// readiness it was asked under, or, for one the owner refused although it
/// was asked while ready (it lost a race), the owner's own reason now.
pub fn refused_reason<F: Feeds>(feeds: &F, ready: Readiness) -> Blocked {
    match ready {
        Readiness::Blocked(blocked) => blocked,
        Readiness::Ready => feeds.blocked().unwrap_or(Blocked::Feeding),
    }
}

/// Where a feed is, as the food level sees it: only a running feed can
/// light the beam.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flight {
    /// No feed is asked for or running.
    Rest,
    /// A feed waits for its claim on the outputs ([`Feeds::claiming`]):
    /// readings at rest are held, and it may yet be refused having moved
    /// nothing.
    Claiming,
    /// A feed has its claim, or the owner holds it.
    Running,
}

impl Flight {
    /// A waiting claim first: `busy` is true for it too, though nothing has
    /// moved.
    pub fn of<F: Feeds>(feeds: &F) -> Self {
        if feeds.claiming().is_some() {
            Self::Claiming
        } else if feeds.busy() {
            Self::Running
        } else {
            Self::Rest
        }
    }
}

/// The last readiness logged, the last lasting reason recorded, and the
/// reason of the last refused Dispense press recorded since the feeder
/// was last ready.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Watch {
    logged: Option<Readiness>,
    lasting: Option<Blocked>,
    /// The reason the last refused Dispense press was recorded for, since
    /// the feeder was last ready.
    refused: Option<Blocked>,
    /// The food level last seen at rest this boot (`true` with food).
    food: Option<bool>,
    /// A feed has been in flight since the last pass at rest.
    fed: bool,
}

impl Watch {
    pub const fn new() -> Self {
        Self {
            logged: None,
            lasting: None,
            refused: None,
            food: None,
            fed: false,
        }
    }

    /// The food beam's latest reading, `reading`, with the feed `flight`:
    /// the level to record as a `Food` event when it differs from the last
    /// one seen this boot. During a feed the level waits for its end, so a
    /// beam that flickers records once. The first level of a boot only sets
    /// the baseline, unless the feed just ended took it. A feed still
    /// waiting for its claim has lit nothing: one refused there leaves a
    /// reading taken in the pass it was asked in (the boot's first window
    /// can finish just before the press is queued) a baseline, not a feed's.
    pub fn food(&mut self, reading: Option<bool>, flight: Flight) -> Option<FoodLevel> {
        match flight {
            Flight::Rest => {}
            Flight::Claiming => return None,
            Flight::Running => {
                self.fed = true;
                return None;
            }
        }
        let fed = core::mem::take(&mut self.fed);
        let present = reading?;
        let last = self.food.replace(present);
        (last != Some(present) && (last.is_some() || fed)).then(|| feed::food_level(Some(present)))
    }

    /// A Dispense press that could not feed, for `reason`: recorded in the
    /// history when the reason differs from the last one recorded since the
    /// feeder was last ready, otherwise only in the log. A pass that never
    /// sees the feeder ready between two reasons still records the second.
    pub fn refused<P: Platform>(&mut self, platform: &mut P, now_ms: u32, reason: Blocked) {
        if self.refused.replace(reason) != Some(reason) {
            events::record(platform, now_ms, EventBody::DispenseRefused { reason });
        }
    }

    /// Note `now`: a changed readiness is logged, and a changed lasting
    /// reason recorded as an event. Each boot starts with none recorded, so
    /// a reason that holds at boot is recorded again after the `Boot`.
    pub fn note<P: Platform>(
        &mut self,
        platform: &mut P,
        now_ms: u32,
        now: Readiness,
        mut record: impl FnMut(Record),
    ) {
        if self.logged != Some(now) {
            self.logged = Some(now);
            record(Record::Readiness(now));
        }
        if now == Readiness::Ready {
            self.refused = None;
        }
        let lasting = match now {
            Readiness::Blocked(blocked) if blocked.lasting() => Some(blocked),
            Readiness::Ready | Readiness::Blocked(_) => None,
        };
        if lasting != self.lasting {
            self.lasting = lasting;
            events::record(
                platform,
                now_ms,
                EventBody::FeedingBlocked { reason: lasting },
            );
        }
    }
}

const fn blocked_name(blocked: Blocked) -> &'static str {
    match blocked {
        Blocked::Starting => "starting",
        Blocked::Journal => "journal",
        Blocked::Fault => "fault",
        Blocked::Resolving => "resolving",
        Blocked::Feeding => "feeding",
        Blocked::Joining => "joining",
        Blocked::Updating => "updating",
        Blocked::Setup => "setup",
        Blocked::Unserved => "unserved",
        Blocked::Battery => "battery",
        Blocked::Restarting => "restarting",
    }
}

pub(super) const fn refusal_name(refusal: Refusal) -> &'static str {
    match refusal {
        Refusal::Busy => "busy",
        Refusal::Fault => "fault",
        Refusal::Unavailable => "unavailable",
        Refusal::Invalid => "invalid",
        Refusal::NotWritable => "not-writable",
        Refusal::Unverified => "unverified",
        Refusal::Portions { .. } => "portions",
        Refusal::Forbidden
        | Refusal::Stale { .. }
        | Refusal::Full
        | Refusal::NotFound
        | Refusal::Muted
        | Refusal::Time
        | Refusal::NoClock
        | Refusal::Update(_) => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A waiting feed stops for any reason that can arise while it waits, in
    /// the order readiness names them.
    #[test]
    fn a_waiting_feed_is_held_by_what_arises_while_it_waits() {
        assert_eq!(claim_hold(false, false, false, false), None);
        let held = |restarting, setup, installing, battery| {
            claim_hold(restarting, setup, installing, battery)
        };
        assert_eq!(held(true, true, true, true), Some(Blocked::Restarting));
        assert_eq!(held(false, true, true, true), Some(Blocked::Setup));
        assert_eq!(held(false, false, true, true), Some(Blocked::Updating));
        assert_eq!(held(false, false, false, true), Some(Blocked::Battery));
    }

    /// One `Food` event a change, and only at rest: none before the beam
    /// has read, none while a feed runs however it flickers, none for a
    /// level already recorded.
    #[test]
    fn the_food_level_is_recorded_once_a_change_at_rest() {
        let mut watch = Watch::new();
        assert_eq!(watch.food(None, Flight::Rest), None, "no reading at boot");

        // A feed reads food, then low, then food, and ends.
        for reading in [true, false, true] {
            assert_eq!(
                watch.food(Some(reading), Flight::Running),
                None,
                "during a feed"
            );
        }
        assert_eq!(
            watch.food(Some(true), Flight::Rest),
            Some(FoodLevel::Present)
        );
        assert_eq!(watch.food(Some(true), Flight::Rest), None, "no repeat");

        assert_eq!(watch.food(Some(false), Flight::Running), None);
        assert_eq!(watch.food(Some(false), Flight::Rest), Some(FoodLevel::Low));
        assert_eq!(watch.food(Some(false), Flight::Rest), None);
    }

    /// A boot's first reading at rest sets the baseline with no event, low or
    /// not; a change after it records, by a window or a feed alike.
    #[test]
    fn a_boot_s_first_idle_reading_records_nothing() {
        for (first, change) in [(false, FoodLevel::Present), (true, FoodLevel::Low)] {
            let mut watch = Watch::new();
            assert_eq!(watch.food(None, Flight::Rest), None);
            assert_eq!(watch.food(Some(first), Flight::Rest), None, "the baseline");
            assert_eq!(watch.food(Some(first), Flight::Rest), None);
            assert_eq!(
                watch.food(Some(!first), Flight::Rest),
                Some(change),
                "a window"
            );

            let mut watch = Watch::new();
            assert_eq!(watch.food(Some(first), Flight::Rest), None);
            assert_eq!(
                watch.food(Some(!first), Flight::Running),
                None,
                "during a feed"
            );
            assert_eq!(
                watch.food(Some(!first), Flight::Rest),
                Some(change),
                "its end"
            );
        }
    }

    /// A feed that ended unread leaves the next window a first idle reading,
    /// and a feed's reading records even as the boot's first.
    #[test]
    fn only_a_feed_s_own_reading_records_as_the_first() {
        let mut watch = Watch::new();
        assert_eq!(watch.food(None, Flight::Running), None, "a feed, unread");
        assert_eq!(watch.food(None, Flight::Rest), None, "it ended");
        assert_eq!(watch.food(Some(false), Flight::Rest), None, "a window");

        let mut watch = Watch::new();
        assert_eq!(
            watch.food(Some(false), Flight::Running),
            None,
            "a feed reads low"
        );
        assert_eq!(watch.food(Some(false), Flight::Rest), Some(FoodLevel::Low));
    }

    /// A feed refused while it waited for its claim lit nothing: the boot's
    /// first reading, taken in the pass it was asked in, stays a baseline.
    /// One whose claim was taken read the beam, and its reading records.
    #[test]
    fn a_feed_refused_before_its_claim_takes_no_reading() {
        let mut watch = Watch::new();
        assert_eq!(watch.food(Some(true), Flight::Claiming), None);
        assert_eq!(watch.food(Some(true), Flight::Rest), None, "refused");
        assert_eq!(watch.food(Some(true), Flight::Rest), None);

        let mut watch = Watch::new();
        assert_eq!(watch.food(Some(true), Flight::Claiming), None);
        assert_eq!(watch.food(Some(true), Flight::Running), None, "claimed");
        assert_eq!(
            watch.food(Some(true), Flight::Rest),
            Some(FoodLevel::Present)
        );
    }
}
