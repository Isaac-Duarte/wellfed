//! What a session knows of its feeder at once: the status with the
//! settings, the plans, the calibration and the day they come to, built
//! from `wellfed_client::today`'s facts so it agrees with the app's card.

use jiff::tz::TimeZone;
use wellfed_client::today::{Facts, Portions};
use wellfed_model as model;

use super::views::{Feed, Status};

/// The settings Home Assistant shows and changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(frozen, get_all, eq, module = "wellfed_homeassistant")
)]
pub struct Settings {
    pub sound: bool,
    pub lights: bool,
    /// 0-100.
    pub volume: u8,
    /// The desiccant reminder's period, 1-90 days.
    pub desiccant_days: u8,
    /// Minutes ahead of each meal its `meal_soon` event comes; `None` when
    /// off, or when it is a kind of reminder a newer build added.
    pub meal_reminder_minutes: Option<u8>,
}

impl From<&model::FeederSettings> for Settings {
    fn from(settings: &model::FeederSettings) -> Self {
        Self {
            sound: settings.sound,
            lights: settings.lights,
            volume: settings.volume,
            desiccant_days: settings.desiccant_days,
            meal_reminder_minutes: match settings.meal_reminder {
                model::MealReminder::Off => None,
                model::MealReminder::Before { minutes } => Some(minutes),
                // A reminder a newer build added, which this one cannot
                // read: no lead to show.
                model::MealReminder::Unknown => None,
            },
        }
    }
}

/// One feeding plan, by its slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(frozen, get_all, eq, module = "wellfed_homeassistant")
)]
pub struct Plan {
    pub slot: u8,
    /// The minute of the feeder's local day it is due at (0-1439).
    pub minute: u16,
    pub portions: u8,
    /// The weekdays it runs, Monday in bit 0; 0 for a plan that runs once.
    pub days: u8,
    pub enabled: bool,
}

/// The plans' next meal, by the feeder's clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(frozen, get_all, eq, module = "wellfed_homeassistant")
)]
pub struct NextMeal {
    pub slot: u8,
    /// Unix milliseconds.
    pub at_ms: i64,
    /// The minute of the feeder's local day it falls on.
    pub minute: u16,
    pub portions: u8,
}

/// The portions fed on the feeder's local day: a range, since an
/// uncertain feed is never one number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(frozen, get_all, eq, module = "wellfed_homeassistant")
)]
pub struct PortionsToday {
    pub confirmed: u32,
    pub at_most: u32,
    pub feeds: u32,
}

impl From<Portions> for PortionsToday {
    fn from(portions: Portions) -> Self {
        Self {
            confirmed: portions.confirmed,
            at_most: portions.at_most,
            feeds: portions.feeds,
        }
    }
}

/// What the feeder serves this session, as Home Assistant chooses its
/// entities by: hints from the device's kind. A write is gated by its op's
/// own kind (`writes::served`), never by these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(frozen, get_all, eq, module = "wellfed_homeassistant")
)]
pub struct Capabilities {
    pub feed: bool,
    pub plans: bool,
    pub desiccant: bool,
    pub calibration: bool,
    pub time: bool,
    pub update: bool,
}

impl Capabilities {
    /// What a device of a kind this build does not know serves here:
    /// nothing.
    pub(crate) const NONE: Self = Self {
        feed: false,
        plans: false,
        desiccant: false,
        calibration: false,
        time: false,
        update: false,
    };

    /// What a device of `kind` serves: the shared core (time, updates) for
    /// any kind, and the feeder's module (feeds, plans, the desiccant and
    /// the calibration) for a feeder.
    pub(crate) const fn of(kind: model::DeviceKind) -> Self {
        let feeder = matches!(kind, model::DeviceKind::Feeder);
        Self {
            feed: feeder,
            plans: feeder,
            desiccant: feeder,
            calibration: feeder,
            time: true,
            update: true,
        }
    }
}

/// Everything the session knows of the feeder now.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(frozen, get_all, eq, module = "wellfed_homeassistant")
)]
pub struct State {
    pub status: Status,
    /// `None` until read.
    pub settings: Option<Settings>,
    /// The filled plan slots, by slot; `None` until read or on a feeder
    /// without plans.
    pub plans: Option<Vec<Plan>>,
    pub next_meal: Option<NextMeal>,
    /// The newest feed the history has given.
    pub last_fed: Option<Feed>,
    /// `None` while the feeder's clock is invalid.
    pub portions_today: Option<PortionsToday>,
    /// The weight of one portion, milligrams; `None` until measured, or
    /// for a client that does not read the calibration.
    pub portion_mg: Option<u32>,
}

impl State {
    /// The state `facts` come to, once the status is in. The day is the
    /// feeder's, in its own zone, with `fallback` before the time is read:
    /// this host's directly, the home's through the owner's server, which
    /// keeps every feeder on the home's zone.
    pub fn of(
        facts: &Facts,
        calibration: Option<&model::CalibrationView>,
        fallback: &TimeZone,
    ) -> Option<Self> {
        // A device this integration cannot read has no state to show.
        let status = Status::of(facts.status.as_ref()?)?;
        let today = facts.today(fallback.clone());
        let plans = facts.plans.as_ref().map(|plans| {
            plans
                .plans
                .iter()
                .zip(0u8..)
                .filter_map(|(plan, slot)| {
                    let plan = (*plan)?;
                    Some(Plan {
                        slot,
                        minute: plan.minute,
                        portions: plan.portions,
                        days: plan.days.bits(),
                        enabled: plan.enabled,
                    })
                })
                .collect()
        });

        Some(Self {
            status,
            settings: facts.settings.as_ref().map(Settings::from),
            plans,
            next_meal: today.next.map(|meal| NextMeal {
                slot: meal.slot,
                at_ms: meal.at.as_millisecond(),
                minute: meal.minute,
                portions: meal.portions,
            }),
            last_fed: facts.last_fed().map(Feed::from),
            portions_today: facts
                .portions_today(fallback.clone())
                .map(PortionsToday::from),
            portion_mg: calibration.and_then(|view| view.calibration.portion_mg),
        })
    }
}
