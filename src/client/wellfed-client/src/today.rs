//! What a client shows of the feeder's day: the status with the plans, the
//! time settings and the feeds its session has seen, reckoned on the
//! feeder's own clock and zone.
//!
//! The app's Home card and Home Assistant's sensors read the same
//! [`Facts`], so the last feed, today's meals, the next meal and the
//! portions fed today agree wherever they are shown. Each client keeps its
//! own time read ([`TimeSettings`]): the app also measures how long ago it read
//! the time, which this module never needs.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use wellfed_model::{FeederSettings, Plans, Status, TimeView};

use crate::history::Feed;
use crate::plans::{self, Meal};
use crate::time::{feeder_zone, next_meal};

/// The feeder's time settings as a client read them.
pub trait TimeSettings {
    fn view(&self) -> &TimeView;
}

impl TimeSettings for TimeView {
    fn view(&self) -> &TimeView {
        self
    }
}

/// What a session has read of the feeder.
#[derive(Clone, Debug, PartialEq)]
pub struct Facts<T = TimeView> {
    pub status: Option<Status>,
    pub settings: Option<FeederSettings>,
    pub plans: Option<Plans>,
    pub time: Option<T>,
    /// The feeds its history has given so far, oldest first.
    pub feeds: Vec<Feed>,
}

// By hand: a derive would ask `T: Default`, which no time read is.
impl<T> Default for Facts<T> {
    fn default() -> Self {
        Self {
            status: None,
            settings: None,
            plans: None,
            time: None,
            feeds: Vec::new(),
        }
    }
}

/// The plans' next meal, by the feeder's clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NextMeal {
    /// The plan's slot, which the schedule marks as next.
    pub slot: u8,
    pub at: Timestamp,
    /// The minute of the feeder's local day it falls on.
    pub minute: u16,
    pub portions: u8,
}

/// The next meal and today's, earliest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Today {
    pub next: Option<NextMeal>,
    pub meals: Vec<Meal>,
}

/// The portions today's feeds delivered, by the feeder's local calendar: as
/// a range, since an uncertain feed is never one number.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Portions {
    /// The portions the feedback sensor confirmed.
    pub confirmed: u32,
    /// The most that may have dropped: `confirmed` plus what each uncertain
    /// feed asked for beyond its confirmed portions.
    pub at_most: u32,
    /// Feeds today, whatever they delivered.
    pub feeds: u32,
}

impl<T: TimeSettings> Facts<T> {
    /// The feeder's zone: its rule, or a sync's offset, or `fallback` (the
    /// host's own zone) before the time is read or with neither.
    pub fn zone(&self, fallback: TimeZone) -> TimeZone {
        match &self.time {
            Some(read) => feeder_zone(read.view(), fallback),
            None => fallback,
        }
    }

    /// The next meal and today's, by the feeder's clock as of its status, in
    /// its zone. A feeder with no clock holds its meals, so there are
    /// neither; nor before the plans are read.
    pub fn today(&self, fallback: TimeZone) -> Today {
        let (Some(plans), Some(now)) = (self.plans.as_ref(), self.clock()) else {
            return Today::default();
        };
        let zone = self.zone(fallback);

        let next = next_meal(plans, now, &zone).map(|meal| NextMeal {
            slot: meal.slot,
            at: meal.at,
            minute: minute_of(meal.at, &zone),
            portions: meal.portions,
        });
        let meals = plans::today(plans, &self.feeds, now, &zone);
        Today { next, meals }
    }

    /// The newest feed the history has given.
    pub fn last_fed(&self) -> Option<&Feed> {
        self.feeds.last()
    }

    /// The `Fed` event of the feed the status names as its last fault, once
    /// the history has given it.
    pub fn fault_feed(&self) -> Option<&Feed> {
        let fault = self.status.as_ref()?.feeder()?.feed_fault?;
        self.feeds.iter().rev().find(|fed| fed.feed == fault.feed)
    }

    /// The portions fed on the feeder's local day, by its clock as of its
    /// status; `None` while that clock is invalid. A feed recorded with no
    /// clock has no day, so it is not counted.
    pub fn portions_today(&self, fallback: TimeZone) -> Option<Portions> {
        let now = self.clock()?;
        let zone = self.zone(fallback);
        let day = now.to_zoned(zone.clone()).date();

        let mut portions = Portions::default();
        let today = self.feeds.iter().filter(|fed| {
            fed.at
                .is_some_and(|at| at.to_zoned(zone.clone()).date() == day)
        });
        for fed in today {
            let (confirmed, at_most) = fed.delivered.bounds();
            portions.confirmed += u32::from(confirmed);
            portions.at_most += u32::from(at_most);
            portions.feeds += 1;
        }
        Some(portions)
    }

    fn clock(&self) -> Option<Timestamp> {
        self.status.as_ref()?.clock
    }
}

/// A moment's minute of the local day in `zone`.
fn minute_of(at: Timestamp, zone: &TimeZone) -> u16 {
    let local = at.to_zoned(zone.clone());
    u16::try_from(i32::from(local.hour()) * 60 + i32::from(local.minute())).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use wellfed_model::{
        Battery, Desiccant, DeviceStatus, FeedFault, FeedOutcome, FeedReason, FeederStatus,
        FoodLevel, Generation, NtpServer, PLANS, Plan, Readiness, Source, Supply, Weekdays,
        WifiSource,
    };

    use super::*;
    use crate::history::Delivered;

    /// 12:00 UTC on a Wednesday.
    const NOON: i64 = 1_793_793_600_000;
    const HOUR_MS: i64 = 3_600_000;

    fn at(ms: i64) -> Timestamp {
        Timestamp::from_millisecond(ms).unwrap()
    }

    fn status(clock: Option<Timestamp>) -> Status {
        Status {
            build: "today-test".into(),
            bank: 0,
            uptime_ms: 0,
            boot_count: 1,
            reset_flags: 0,
            faults: 0,
            rssi: -50,
            wifi: WifiSource::Stored,
            link_losses: 0,
            dns_losses: 0,
            clock,
            power: None,
            wifi_generation: Generation(1),
            last_event: None,
            sessions: 1,
            alerts_generation: Generation(0),
            device: DeviceStatus::Feeder(FeederStatus {
                locked: Some(false),
                settings_generation: Generation(1),
                food: FoodLevel::Present,
                feed_fault: None,
                readiness: Readiness::Ready,
                desiccant: Desiccant::Unset,
                supply: Supply {
                    adapter: Some(true),
                    battery: Battery::Absent,
                    below_floor: None,
                },
            }),
        }
    }

    /// A time read whose zone is a fixed `offset_s` east of UTC.
    fn time(offset_s: i32) -> TimeView {
        TimeView {
            generation: Generation(1),
            writable: true,
            zone: None,
            ntp: NtpServer::Off,
            clock_ms: None,
            error_ms: None,
            offset_s: Some(offset_s),
            rate_ppb: 0,
            rate_bound_ppb: 0,
            learned: false,
            syncs: 0,
            last: None,
            last_age_s: None,
            ntp_last: None,
        }
    }

    fn fed(seq: u64, feed: u32, at_ms: Option<i64>, outcome: FeedOutcome, completed: u8) -> Feed {
        Feed {
            epoch: 1,
            seq,
            feed,
            at: at_ms.map(at),
            by: Some(Source::Panel),
            requested: 3,
            delivered: Delivered::of(outcome, completed, 3),
            outcome,
            reason: FeedReason::Done,
            hopper: FoodLevel::Present,
        }
    }

    fn facts(clock: Option<Timestamp>, feeds: Vec<Feed>) -> Facts {
        Facts {
            status: Some(status(clock)),
            time: Some(time(0)),
            feeds,
            ..Facts::default()
        }
    }

    #[test]
    fn portions_today_count_an_uncertain_feed_as_a_range_and_only_today_s() {
        let feeds = vec![
            fed(1, 1, Some(NOON - 24 * HOUR_MS), FeedOutcome::Done, 3),
            fed(2, 2, Some(NOON - 2 * HOUR_MS), FeedOutcome::Done, 2),
            fed(3, 3, Some(NOON - HOUR_MS), FeedOutcome::Uncertain, 1),
            fed(4, 4, None, FeedOutcome::Done, 3),
        ];
        let portions = facts(Some(at(NOON)), feeds).portions_today(TimeZone::UTC);

        assert_eq!(
            portions,
            Some(Portions {
                confirmed: 3,
                at_most: 5,
                feeds: 2,
            })
        );
    }

    #[test]
    fn the_day_is_the_feeder_s_own_by_its_zone() {
        // 11:00 UTC yesterday is 00:00 today, 13 hours east.
        let feeds = vec![fed(1, 1, Some(NOON - 25 * HOUR_MS), FeedOutcome::Done, 3)];
        let mut east = facts(Some(at(NOON)), feeds);
        assert_eq!(east.portions_today(TimeZone::UTC).map(|p| p.feeds), Some(0));

        east.time = Some(time(13 * 3600));
        // The feeder's day began at 11:00 UTC; the feed was the day before.
        assert_eq!(east.portions_today(TimeZone::UTC).map(|p| p.feeds), Some(0));
        east.feeds[0].at = Some(at(NOON - HOUR_MS / 2));
        assert_eq!(east.portions_today(TimeZone::UTC).map(|p| p.feeds), Some(1));
    }

    #[test]
    fn without_a_clock_there_is_no_day_and_no_meal() {
        let mut plans = [None; PLANS];
        plans[0] = Some(Plan {
            minute: 18 * 60,
            portions: 2,
            days: Weekdays::DAILY,
            enabled: true,
            call: 0,
        });
        let mut facts = facts(None, Vec::new());
        facts.plans = Some(Plans {
            generation: Generation(1),
            writable: true,
            plans,
            skipped: wellfed_model::SkippedMeals::NONE,
        });
        assert_eq!(facts.portions_today(TimeZone::UTC), None);
        assert_eq!(facts.today(TimeZone::UTC), Today::default());

        facts.status = Some(status(Some(at(NOON))));
        let today = facts.today(TimeZone::UTC);
        let next = today.next.unwrap();
        assert_eq!((next.slot, next.minute, next.portions), (0, 18 * 60, 2));
        assert_eq!(next.at, at(NOON + 6 * HOUR_MS));
        assert_eq!(today.meals.len(), 1);
    }

    #[test]
    fn the_fault_s_feed_is_found_once_the_history_has_it() {
        let mut facts = facts(
            Some(at(NOON)),
            vec![fed(1, 7, Some(NOON), FeedOutcome::Failed, 0)],
        );
        assert_eq!(facts.fault_feed(), None);
        assert_eq!(facts.last_fed().map(|fed| fed.feed), Some(7));

        let fault = FeedFault {
            feed: 7,
            outcome: FeedOutcome::Failed,
            reason: FeedReason::Jammed,
        };
        facts
            .status
            .as_mut()
            .unwrap()
            .feeder_mut()
            .unwrap()
            .feed_fault = Some(fault);
        assert_eq!(facts.fault_feed().map(|fed| fed.feed), Some(7));
    }

    #[test]
    fn with_no_time_read_the_zone_is_the_host_s() {
        let mut facts = facts(Some(at(NOON)), Vec::new());
        facts.time = None;
        assert_eq!(
            facts.zone(TimeZone::fixed(jiff::tz::offset(2))),
            TimeZone::fixed(jiff::tz::offset(2))
        );
    }
}
