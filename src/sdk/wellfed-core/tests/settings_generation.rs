//! The settings' shared generation and the writer's outcomes: records that
//! agree report their generation, records apart report one past the
//! larger, a kind with no record of its own reports the device's alone,
//! and each finished change keeps its outcome for the client waiting on it.

use wellfed_core::settings::{SaveOutcome, Saved, view_generation};

#[test]
fn records_that_agree_report_their_generation() {
    assert_eq!(view_generation(0, Some(0)), 0);
    assert_eq!(view_generation(7, Some(7)), 7);
    assert_eq!(view_generation(7, None), 7, "no record of the kind's");
}

/// The review's case under the old sum: (5, 3) then (5, 4) then (6, 4),
/// and the kind's newest slot lost reads (6, 3), which summed to 9, a value
/// (5, 4) had named. Under one shared generation each save writes both,
/// so the published values are 5 and 6, and the records apart read 7.
#[test]
fn records_apart_report_one_past_the_larger() {
    assert_eq!(view_generation(6, Some(5)), 7, "the kind's newest lost");
    assert_eq!(view_generation(5, Some(6)), 7, "the device's newest lost");
    assert_eq!(view_generation(6, Some(0)), 7, "the kind's never saved");
    assert_eq!(view_generation(u64::MAX, Some(0)), u64::MAX);
}

#[test]
fn each_finished_change_keeps_its_outcome() {
    let mut saved = Saved::default();
    let first = saved.next();
    assert_eq!(saved.outcome(first), SaveOutcome::Pending);

    saved.finish(false);
    let second = saved.next();
    saved.finish(true);
    let third = saved.next();
    assert_eq!(saved.outcome(first), SaveOutcome::Done);
    assert_eq!(saved.outcome(second), SaveOutcome::Failed);
    assert_eq!(saved.outcome(third), SaveOutcome::Pending);

    // A change 32 or more before the latest has lost its bit: read as
    // failed, never as done.
    for _ in 0..32 {
        saved.finish(false);
    }
    assert_eq!(saved.outcome(first), SaveOutcome::Failed);
    assert_eq!(saved.outcome(third), SaveOutcome::Done);
}
