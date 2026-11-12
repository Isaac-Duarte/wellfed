//! The feeding plans over the device API: list them, or add, edit, enable,
//! disable or delete one. A change reads the plans, edits that table and
//! writes it back against the generation it read, so a change made
//! meanwhile (another client's, or a plan that ran once disabling itself)
//! is refused rather than overwritten; it is never retried.
//!
//! A skip names one of today's meals by its instant, which the plans as read,
//! the feeder's clock and its zone give (`wellfed_client::plans::to_skip`), so
//! nobody types a time; the feeder checks it again and refuses a meal that
//! is no longer the plan's next. It is never retried either.
//!
//! `wellfed server plans` makes the same changes through the server, from
//! the same arguments ([`NewPlan`], [`PlanChange`], [`Slot`]) and with the
//! same edit (`wellfed_client::plans::edited`).

use crate::device::Connection;
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use wellfed_client::plans::{Edit, edited, to_skip, to_undo};
use wellfed_client::time::feeder_zone;
use wellfed_model::{
    Answer, CALL_PLAYS_MAX, Op, PLANS, PORTIONS_MAX, PORTIONS_MIN, Plan, Plans, Refusal, TimeOfDay,
    Weekdays,
};

use crate::{Target, connect, print_answer, text};

#[derive(Args)]
pub(crate) struct PlansArgs {
    #[command(subcommand)]
    action: Action,
}

#[derive(Subcommand)]
enum Action {
    /// The plan in each slot.
    List {
        #[command(flatten)]
        target: Target,
        /// The plans as JSON, as the wire spells them, instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Add a plan in the first free slot.
    Add {
        #[command(flatten)]
        target: Target,
        #[command(flatten)]
        plan: NewPlan,
    },
    /// Change a plan's time, portions, days or meal call. A new time or
    /// days runs only from the next occurrence after the change.
    Edit {
        #[command(flatten)]
        target: Target,
        #[command(flatten)]
        change: PlanChange,
    },
    /// Enable a plan: it runs from its next occurrence.
    Enable {
        #[command(flatten)]
        target: Target,
        #[command(flatten)]
        slot: Slot,
    },
    /// Disable a plan; it keeps its slot.
    Disable {
        #[command(flatten)]
        target: Target,
        #[command(flatten)]
        slot: Slot,
    },
    /// Empty a plan's slot.
    Delete {
        #[command(flatten)]
        target: Target,
        #[command(flatten)]
        slot: Slot,
    },
    /// Skip the plan's next meal, if it falls today: it does not feed,
    /// remind or call. `unskip` brings it back until its time.
    Skip {
        #[command(flatten)]
        target: Target,
        #[command(flatten)]
        slot: Slot,
    },
    /// Bring back the plan's skipped meal, while its time is still to come.
    Unskip {
        #[command(flatten)]
        target: Target,
        #[command(flatten)]
        slot: Slot,
    },
}

/// What to do with a plan's meal today.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MealChange {
    Skip,
    Undo,
}

/// A plan to add.
#[derive(Args)]
pub(crate) struct NewPlan {
    /// The feeder's local time to feed at, HH:MM.
    #[arg(long, value_name = "HH:MM")]
    at: TimeOfDay,
    #[arg(long, value_parser = portions())]
    portions: u8,
    /// `daily`, `weekdays`, `weekends`, `once` (runs at its next
    /// occurrence, then disables itself) or a list like `mon,thu`.
    #[arg(long, default_value = "daily")]
    days: Weekdays,
    /// Store it disabled.
    #[arg(long)]
    disabled: bool,
    /// Play the meal call this many times before the meal (0 to 5;
    /// `wellfed call upload` stores the clip).
    #[arg(long, default_value_t = 0, value_parser = plays())]
    call: u8,
}

impl NewPlan {
    pub(crate) const fn edit(&self) -> Edit {
        Edit::Add(Plan {
            minute: self.at.0,
            portions: self.portions,
            days: self.days,
            enabled: !self.disabled,
            call: self.call,
        })
    }
}

/// What to change in one plan; what it does not name stays.
#[derive(Args)]
pub(crate) struct PlanChange {
    #[arg(value_parser = slot())]
    slot: u8,
    #[arg(long, value_name = "HH:MM")]
    at: Option<TimeOfDay>,
    #[arg(long, value_parser = portions())]
    portions: Option<u8>,
    #[arg(long)]
    days: Option<Weekdays>,
    /// The meal call's plays before the meal, 0 for none.
    #[arg(long, value_parser = plays())]
    call: Option<u8>,
}

impl PlanChange {
    pub(crate) const fn edit(&self) -> Edit {
        Edit::Change {
            slot: self.slot,
            at: self.at,
            portions: self.portions,
            days: self.days,
            call: self.call,
            enabled: None,
        }
    }
}

/// A plan's slot, 0-9.
#[derive(Args)]
pub(crate) struct Slot {
    #[arg(value_parser = slot())]
    slot: u8,
}

impl Slot {
    pub(crate) const fn slot(&self) -> u8 {
        self.slot
    }

    pub(crate) const fn enabled(&self, enabled: bool) -> Edit {
        Edit::Enabled {
            slot: self.slot,
            enabled,
        }
    }

    pub(crate) const fn delete(&self) -> Edit {
        Edit::Delete { slot: self.slot }
    }
}

fn slot() -> clap::builder::RangedI64ValueParser<u8> {
    clap::value_parser!(u8).range(0..i64::try_from(PLANS).unwrap_or(i64::MAX))
}

/// Portions for a feed or a plan: within the feeder's limits, which it
/// checks again and refuses by name.
pub(crate) fn portions() -> clap::builder::RangedI64ValueParser<u8> {
    clap::value_parser!(u8).range(i64::from(PORTIONS_MIN)..=i64::from(PORTIONS_MAX))
}

/// The meal call's plays before a meal.
fn plays() -> clap::builder::RangedI64ValueParser<u8> {
    clap::value_parser!(u8).range(0..=i64::from(CALL_PLAYS_MAX))
}

pub(crate) async fn command(args: PlansArgs) -> Result<()> {
    let (target, edit) = match args.action {
        Action::List { target, json } => return list(&target, json).await,
        Action::Add { target, plan } => (target, plan.edit()),
        Action::Edit { target, change } => (target, change.edit()),
        Action::Enable { target, slot } => (target, slot.enabled(true)),
        Action::Disable { target, slot } => (target, slot.enabled(false)),
        Action::Delete { target, slot } => (target, slot.delete()),
        Action::Skip { target, slot } => return skip(&target, slot.slot, MealChange::Skip).await,
        Action::Unskip { target, slot } => {
            return skip(&target, slot.slot, MealChange::Undo).await;
        }
    };

    let (_, _, mut connection) = connect(&target).await?;
    let current = read_plans(&mut connection).await?;
    let plans = edited(&current, edit)?;
    let answer = connection
        .ask(&Op::PutPlans {
            base: current.generation,
            plans,
        })
        .await?;
    if let Answer::Refused(Refusal::Stale { .. }) = answer {
        eprintln!("The plans changed since they were read; read them again and retry.");
    }
    print_answer(&answer)
}

/// The plans, a line a filled slot, or as JSON.
async fn list(target: &Target, json: bool) -> Result<()> {
    let (_, _, mut connection) = connect(target).await?;
    match connection.ask(&Op::GetPlans).await? {
        Answer::Plans(plans) if !json => {
            println!("{}", text::plans(&plans, &TimeZone::system()));
            Ok(())
        }
        answer => print_answer(&answer),
    }
}

async fn read_plans(connection: &mut Connection) -> Result<Plans> {
    match connection.ask(&Op::GetPlans).await? {
        Answer::Plans(plans) => Ok(plans),
        Answer::Refused(Refusal::Unavailable) => bail!("this feeder keeps no plans"),
        answer => {
            print_answer(&answer)?;
            bail!("the feeder did not return its plans");
        }
    }
}

/// Skip slot `slot`'s meal today, or undo its skip: the meal is found in the
/// plans by the feeder's clock and zone, and the change is sent once.
async fn skip(target: &Target, slot: u8, change: MealChange) -> Result<()> {
    let (_, _, mut connection) = connect(target).await?;
    let plans = read_plans(&mut connection).await?;
    let Answer::Status(status) = connection.ask(&Op::Status).await? else {
        bail!("the feeder did not return its status");
    };
    let now = status
        .clock
        .context("the feeder's clock is not set, so it keeps no meals; run `wellfed time`")?;
    let Answer::Time(view) = connection.ask(&Op::GetTime).await? else {
        bail!("the feeder did not return its time settings");
    };
    let zone = feeder_zone(&view, TimeZone::system());

    let (due, op) = match change {
        MealChange::Skip => {
            let due = to_skip(&plans, slot, now, &zone)?;
            (due, Op::SkipMeal { slot, due })
        }
        MealChange::Undo => {
            let due = to_undo(&plans, slot, now)?;
            (due, Op::UnskipMeal { slot, due })
        }
    };
    let answer = connection.ask(&op).await?;
    if let Answer::Done { .. } = answer {
        println!("{}", skipped(slot, due, &zone, change));
        return Ok(());
    }
    if let Answer::Refused(refusal) = answer
        && let Some(why) = refused(refusal, change)
    {
        eprintln!("{why}");
    }
    print_answer(&answer)
}

/// What a skip or undo did, at the meal's minute on the feeder's day.
pub(crate) fn skipped(slot: u8, due: Timestamp, zone: &TimeZone, change: MealChange) -> String {
    let at = due.to_zoned(zone.clone()).strftime("%H:%M");
    match change {
        MealChange::Skip => format!("Skipped plan {slot}'s {at} meal today."),
        MealChange::Undo => format!("Plan {slot}'s {at} meal today runs again."),
    }
}

/// Why the feeder refused a skip or its undo, where the refusal alone does
/// not say.
pub(crate) const fn refused(refusal: Refusal, change: MealChange) -> Option<&'static str> {
    match (refusal, change) {
        (Refusal::Invalid, MealChange::Skip) => Some(
            "The meal is no longer the plan's next today, or is already skipped; \
             read the plans again.",
        ),
        (Refusal::Invalid, MealChange::Undo) => {
            Some("The skipped meal is already due, or its skip has gone; read the plans again.")
        }
        (Refusal::Busy, _) => Some(
            "The feeder is busy with a feed, a plans change or the plan's last meal; \
             try again shortly.",
        ),
        (Refusal::NotFound, _) => Some("That slot holds no enabled plan."),
        (Refusal::NoClock, _) => Some("The feeder's clock is invalid; run `wellfed time` first."),
        _ => None,
    }
}
