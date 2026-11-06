//! The feeding plans through the server: the same verbs and edit as
//! `wellfed plans` ([`crate::plans`]), so both paths change a table alike.
//! A change reads the plans, edits that table and writes it back once,
//! under a fresh key, against the generation it read: a change made
//! meanwhile is refused with the current plans, never overwritten.
//!
//! A skip, or its undo, names the meal's instant as `wellfed plans skip`
//! finds it, in the home's zone, which every feeder of the home keeps.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use uuid::Uuid;
use wellfed_server_client::Client;
use wellfed_server_client::api::{MealRequest, PlansChange};

use super::feeders::Which;
use super::{Output, Settled, Written, write};
use wellfed_client::plans::{edited, to_skip, to_undo};

use crate::plans::{MealChange, NewPlan, PlanChange, Slot, skipped};

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
        which: Which,
    },
    /// Add a plan in the first free slot.
    Add {
        #[command(flatten)]
        which: Which,
        #[command(flatten)]
        plan: NewPlan,
    },
    /// Change a plan's time, portions, days or meal call. A new time or
    /// days runs only from the next occurrence after the change.
    Edit {
        #[command(flatten)]
        which: Which,
        #[command(flatten)]
        change: PlanChange,
    },
    /// Enable a plan: it runs from its next occurrence.
    Enable {
        #[command(flatten)]
        which: Which,
        #[command(flatten)]
        slot: Slot,
    },
    /// Disable a plan; it keeps its slot.
    Disable {
        #[command(flatten)]
        which: Which,
        #[command(flatten)]
        slot: Slot,
    },
    /// Empty a plan's slot.
    Delete {
        #[command(flatten)]
        which: Which,
        #[command(flatten)]
        slot: Slot,
    },
    /// Skip the plan's next meal, if it falls today: it does not feed,
    /// remind or call. `unskip` brings it back until its time.
    Skip {
        #[command(flatten)]
        which: Which,
        #[command(flatten)]
        slot: Slot,
    },
    /// Bring back the plan's skipped meal, while its time is still to come.
    Unskip {
        #[command(flatten)]
        which: Which,
        #[command(flatten)]
        slot: Slot,
    },
}

pub(super) async fn plans(client: &Client, args: PlansArgs, out: Output) -> Result<Settled> {
    let (which, edit) = match args.action {
        Action::List { which } => {
            let id = which.id(client).await?;
            let plans = client.plans(id).await?;
            out.show(&plans, |plans| {
                crate::text::plans(plans, &TimeZone::system())
            })?;
            return Ok(Settled::Known);
        }
        Action::Add { which, plan } => (which, plan.edit()),
        Action::Edit { which, change } => (which, change.edit()),
        Action::Enable { which, slot } => (which, slot.enabled(true)),
        Action::Disable { which, slot } => (which, slot.enabled(false)),
        Action::Delete { which, slot } => (which, slot.delete()),
        Action::Skip { which, slot } => {
            return skip(client, &which, slot.slot(), MealChange::Skip, out).await;
        }
        Action::Unskip { which, slot } => {
            return skip(client, &which, slot.slot(), MealChange::Undo, out).await;
        }
    };

    let id = which.id(client).await?;
    let current = client.plans(id).await?;
    let change = PlansChange {
        plans: edited(&current, edit)?,
    };

    let key = Uuid::new_v4();
    let sent = client.put_plans(id, current.generation, &change, key);
    let Written::Answered(plans) = write(out, key, sent).await? else {
        return Ok(Settled::Unknown);
    };
    out.show(&plans, |plans| {
        crate::text::plans(plans, &TimeZone::system())
    })?;
    Ok(Settled::Known)
}

/// Skip slot `slot`'s meal today, or undo its skip, once under a fresh key.
async fn skip(
    client: &Client,
    which: &Which,
    slot: u8,
    change: MealChange,
    out: Output,
) -> Result<Settled> {
    let id = which.id(client).await?;
    let plans = client.plans(id).await?;
    let home = client.home().await?;
    let zone = TimeZone::get(&home.zone).with_context(|| {
        format!(
            "the home's zone {} is not one this computer knows",
            home.zone
        )
    })?;
    let now = Timestamp::now();

    let due = match change {
        MealChange::Skip => to_skip(&plans, slot, now, &zone)?,
        MealChange::Undo => to_undo(&plans, slot, now)?,
    };
    let meal = MealRequest { slot, due };
    let key = Uuid::new_v4();
    let sent = match change {
        MealChange::Skip => write(out, key, client.skip_meal(id, meal, key)).await?,
        MealChange::Undo => write(out, key, client.unskip_meal(id, meal, key)).await?,
    };
    let Written::Answered(()) = sent else {
        return Ok(Settled::Unknown);
    };
    out.text(skipped(slot, due, &zone, change));
    Ok(Settled::Known)
}
