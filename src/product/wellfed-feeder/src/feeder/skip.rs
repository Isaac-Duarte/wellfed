//! A client's skip of one of today's meals, and its undo ([schedules]).
//!
//! **The mark is the skip.** Each slot's mark is the latest occurrence
//! that never runs, and the schedule, the meal reminder and the meal call
//! all leave an occurrence at or before it alone. A skip moves the mark up
//! to the skipped occurrence, so none of the three acts on it; nothing
//! else records a skip, and the plans record keeps its format.
//!
//! **Which meal.** Only the plan's next occurrence, while it is still to
//! come and falls on today's local date, and only once every earlier
//! occurrence of the plan is settled (run, claimed, passed or marked by
//! the plan's save): skipping ahead of one the schedule may still run, or
//! still report missed, would spend it unseen. A plan saved without a
//! valid clock has no mark, and its occurrence before the clock returned
//! passes unreported anyway, so it counts as settled.
//!
//! **The undo.** It moves the mark back to the plan's occurrence before the
//! skipped one, only while the skipped one is still to come and the mark
//! is still exactly it (a change to the plan's minute or days resets the
//! mark, and with it the skip). Before the skip the mark was at or after
//! that occurrence, and below the skipped one; consecutive occurrences of
//! one plan are a day or more apart, less an hour at most across a daylight
//! change, so no occurrence lies between the two, and a mark anywhere in
//! that gap runs and reminds exactly the same occurrences. The undo
//! therefore never lets an occurrence run that would not have run without
//! the skip. A plan that runs once is disabled by its skip and enabled by
//! its undo, as a claim and its release do.
//!
//! **Never racing a claim.** The skipped occurrence is still to come when
//! the change is queued, so no claim of it can exist; and while the change
//! waits for the writer, the schedule decides nothing (it waits for every
//! pending plans change), so even an occurrence that comes due meanwhile
//! is judged against the mark the change leaves.
//!
//! [schedules]: ../../../../../docs/firmware/schedules.md

use wellfed_core::schedule::{latest_due, next_due};
use wellfed_device_api::mailbox::{MealSkip, PlansChange, PlansView};
use wellfed_proto::api::Refusal;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// The plans change that skips (or undoes the skip of) slot `slot`'s meal
/// due at `due_s`, checked against `view` at `wall_ms` with the zone's
/// offsets, or why it is refused.
pub fn change(
    view: &PlansView,
    slot: u8,
    due_s: u32,
    skip: MealSkip,
    wall_ms: u64,
    offset_at: impl Fn(u64) -> Option<i32>,
) -> Result<PlansChange, Refusal> {
    let today = local_day(wall_ms, &offset_at).ok_or(Refusal::NoClock)?;
    let index = usize::from(slot);
    let Some(Some(plan)) = view.table.get(index).copied() else {
        return Err(Refusal::NotFound);
    };
    // At or past its instant the meal is the schedule's: run, or missed.
    let due_ms = u64::from(due_s) * 1000;
    if due_ms <= wall_ms {
        return Err(Refusal::Invalid);
    }

    match skip {
        MealSkip::Skip => {
            if !plan.enabled {
                return Err(Refusal::NotFound);
            }
            let next = next_due(&plan, wall_ms, &offset_at).ok_or(Refusal::NoClock)?;
            if next != due_ms || local_day(next, &offset_at) != Some(today) {
                return Err(Refusal::Invalid);
            }
            let mark = view.mark(index).map(|mark| u64::from(mark) * 1000);
            if mark.is_some_and(|mark| mark >= due_ms) {
                return Err(Refusal::Invalid);
            }
            let latest = latest_due(&plan, wall_ms, &offset_at).ok_or(Refusal::NoClock)?;
            if mark.is_some_and(|mark| mark < latest) {
                return Err(Refusal::Busy);
            }
            Ok(PlansChange::Skip {
                plan: slot,
                due_s,
                revision: view.revision,
            })
        }
        MealSkip::Unskip => {
            // A skipped plan that runs once is the disabled one.
            if plan.enabled == plan.days.once() {
                return Err(Refusal::NotFound);
            }
            if view.marks.get(index) != Some(&due_s) {
                return Err(Refusal::Invalid);
            }
            let before = due_ms - 1000;
            let back = latest_due(&plan, before, &offset_at).ok_or(Refusal::NoClock)?;
            // The offsets moved since the skip, so another occurrence now
            // comes first: the mark would skip that one instead.
            if back > wall_ms {
                return Err(Refusal::Invalid);
            }
            Ok(PlansChange::Unskip {
                plan: slot,
                due_s,
                back_s: u32::try_from(back / 1000).map_err(|_| Refusal::Invalid)?,
                revision: view.revision,
            })
        }
    }
}

/// The local date (days since 1970-01-01) of the instant `unix_ms`.
fn local_day(unix_ms: u64, offset_at: impl Fn(u64) -> Option<i32>) -> Option<i64> {
    let offset_ms = i64::from(offset_at(unix_ms)?) * 1000;
    let local_ms = i64::try_from(unix_ms).ok()?.checked_add(offset_ms)?;
    Some(local_ms.div_euclid(DAY_MS))
}
