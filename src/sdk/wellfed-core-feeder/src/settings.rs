//! The feeder's own settings, beside the device's
//! (`wellfed_core::settings`): the meal reminder, the desiccant reminder
//! and the bowl mode. One `settings` request and one view carry both
//! ([`KindSettings`]); the feeder keeps these in a record of its own
//! (`wellfed_feeder::store::settings`), so a kind without them keeps
//! nothing of theirs.

use wellfed_core::settings::KindSettings;

use crate::desiccant::{self, Count};
use crate::feed::progress::BowlMode;

/// Whether the feeder announces each scheduled meal ahead of it, and how
/// far ahead: a `MealSoon` event on the event stream, never a sound.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MealReminder {
    #[default]
    Off,
    /// [`REMINDER_MINUTES_MIN`] to [`REMINDER_MINUTES_MAX`] minutes before
    /// each occurrence.
    Before { minutes: u8 },
}

/// The shortest lead a meal reminder takes.
pub const REMINDER_MINUTES_MIN: u8 = 1;
/// The longest lead a meal reminder takes: an hour, a choice of ours, as
/// stock's cloud field (`feedingPlanAdvanceNoticeTime`) gives no unit.
pub const REMINDER_MINUTES_MAX: u8 = 60;

impl MealReminder {
    /// Whether the lead is one the store accepts.
    pub const fn valid(self) -> bool {
        match self {
            Self::Off => true,
            Self::Before { minutes } => {
                minutes >= REMINDER_MINUTES_MIN && minutes <= REMINDER_MINUTES_MAX
            }
        }
    }

    /// The lead in minutes, or `None` when off.
    pub const fn minutes(self) -> Option<u8> {
        match self {
            Self::Off => None,
            Self::Before { minutes } => Some(minutes),
        }
    }
}

/// `off`, or the lead in minutes, as records spell it.
impl core::fmt::Display for MealReminder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::Off => f.write_str("off"),
            Self::Before { minutes } => write!(f, "{minutes}"),
        }
    }
}

/// The feeder's settings, as its record keeps them and a `settings` reply
/// reports them (all but the bowl mode, which no request names).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeederSettings {
    /// The bowl mode the record was written for. The build runs its own
    /// (`FeederDevice::bowl`); no request changes this.
    pub bowl_mode: BowlMode,
    pub meal_reminder: MealReminder,
    /// The desiccant reminder's period in days.
    pub desiccant_days: u8,
    /// The local calendar day of the last desiccant replacement; `None`
    /// before the first.
    pub desiccant_reset: Option<i32>,
}

impl FeederSettings {
    /// What a feeder holds before its record is first written.
    pub const DEFAULT: Self = Self {
        bowl_mode: BowlMode::Single,
        meal_reminder: MealReminder::Off,
        desiccant_days: desiccant::PERIOD_DEFAULT,
        desiccant_reset: None,
    };

    /// Whether every value is one the store accepts.
    pub fn valid(&self) -> bool {
        desiccant::period_valid(self.desiccant_days) && self.meal_reminder.valid()
    }

    /// The desiccant reminder on local calendar day `today` (`None` while
    /// the clock is invalid).
    pub fn desiccant(&self, today: Option<i32>) -> Count {
        Count::new(self.desiccant_days, self.desiccant_reset, today)
    }
}

impl Default for FeederSettings {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The feeder's fields of a `settings` request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FeederFields {
    pub meal_reminder: Option<MealReminder>,
    /// The desiccant reminder's period in days
    /// ([`desiccant::PERIOD_MIN`] to [`desiccant::PERIOD_MAX`]).
    pub desiccant_days: Option<u8>,
    /// Record a desiccant replacement made on this local calendar day
    /// ([`desiccant::today`]); set only by the reset request, which reads
    /// the day from a valid clock.
    pub desiccant_reset: Option<i32>,
}

impl KindSettings for FeederSettings {
    type Fields = FeederFields;

    const DEFAULT: Self = Self::DEFAULT;
    const NO_FIELDS: FeederFields = FeederFields {
        meal_reminder: None,
        desiccant_days: None,
        desiccant_reset: None,
    };

    fn changes(fields: &FeederFields) -> bool {
        fields.meal_reminder.is_some()
            || fields.desiccant_days.is_some()
            || fields.desiccant_reset.is_some()
    }

    fn valid(fields: &FeederFields) -> bool {
        fields.desiccant_days.is_none_or(desiccant::period_valid)
            && fields.meal_reminder.is_none_or(MealReminder::valid)
    }

    fn changed(self, fields: &FeederFields) -> Self {
        Self {
            meal_reminder: fields.meal_reminder.unwrap_or(self.meal_reminder),
            desiccant_days: fields.desiccant_days.unwrap_or(self.desiccant_days),
            desiccant_reset: fields.desiccant_reset.or(self.desiccant_reset),
            ..self
        }
    }
}
