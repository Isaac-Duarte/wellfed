//! The settings' shared generation and the writer's outcomes: records that
//! read cleanly at one generation report it, anything else reports none,
//! a boot's repair goes two past the larger, a kind with no record of its
//! own counts the device's alone, and each finished change keeps its
//! outcome for the client waiting on it. The rule's model is
//! `wellfed_device`'s `settings_model` test.

use wellfed_core::settings::{
    RecordRead, SaveOutcome, Saved, Slots, repair_generation, view_generation,
};

fn clean(generation: u64) -> RecordRead {
    RecordRead {
        generation,
        slots: Slots::Clean,
    }
}

fn unclean(generation: u64) -> RecordRead {
    RecordRead {
        generation,
        slots: Slots::Unclean,
    }
}

#[test]
fn records_that_agree_report_their_generation() {
    assert_eq!(view_generation(clean(0), Some(clean(0))), Some(0));
    assert_eq!(view_generation(clean(7), Some(clean(7))), Some(7));
    assert_eq!(
        view_generation(clean(7), None),
        Some(7),
        "no record of the kind's"
    );
}

/// Apart, or either not clean, whether or not they agree: no generation.
#[test]
fn records_apart_or_unclean_report_none() {
    assert_eq!(view_generation(clean(6), Some(clean(5))), None);
    assert_eq!(
        view_generation(clean(6), Some(clean(0))),
        None,
        "never saved"
    );
    assert_eq!(view_generation(clean(4), Some(unclean(4))), None);
    assert_eq!(view_generation(unclean(4), Some(clean(4))), None);
    assert_eq!(view_generation(unclean(4), None), None);
}

/// Two past the larger: a newest slot lost on each record can hide the
/// generation both last held, one past what they read.
#[test]
fn a_repair_goes_two_past_the_larger() {
    assert_eq!(repair_generation(3, 2), 5);
    assert_eq!(repair_generation(1, 4), 6);
    assert_eq!(repair_generation(0, 0), 2);
    assert_eq!(repair_generation(u64::MAX, 0), u64::MAX);
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
