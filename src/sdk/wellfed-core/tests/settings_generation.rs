//! The settings' shared generation and the writer's outcomes: records that
//! agree report their generation, clean records apart one past the larger,
//! a record that fell back four past it, a kind with no record of its own
//! the device's alone, and each finished change keeps its outcome for the
//! client waiting on it.

use wellfed_core::settings::{RecordRead, SaveOutcome, Saved, Slots, view_generation};

fn clean(generation: u64) -> RecordRead {
    RecordRead {
        generation,
        slots: Slots::Clean,
    }
}

fn fell_back(generation: u64) -> RecordRead {
    RecordRead {
        generation,
        slots: Slots::FellBack,
    }
}

#[test]
fn records_that_agree_report_their_generation() {
    assert_eq!(view_generation(clean(0), Some(clean(0))), 0);
    assert_eq!(view_generation(clean(7), Some(clean(7))), 7);
    assert_eq!(
        view_generation(clean(7), None),
        7,
        "no record of the kind's"
    );
}

/// A save that stopped between the two writes: one past the larger.
#[test]
fn clean_records_apart_report_one_past_the_larger() {
    assert_eq!(view_generation(clean(6), Some(clean(5))), 7);
    assert_eq!(view_generation(clean(6), Some(clean(0))), 7, "never saved");
    assert_eq!(view_generation(clean(u64::MAX), Some(clean(0))), u64::MAX);
}

/// A record that fell back to its older slot: four past the larger,
/// whether the records then agree or not.
#[test]
fn a_record_that_fell_back_reports_four_past_the_larger() {
    assert_eq!(view_generation(clean(2), Some(fell_back(1))), 6);
    assert_eq!(view_generation(fell_back(1), Some(clean(2))), 6);
    assert_eq!(view_generation(fell_back(4), Some(clean(4))), 8);
    assert_eq!(view_generation(fell_back(4), None), 8);
}

/// The reviewer's sequence. Two whole saves publish 1 and 2; a cut right
/// after save 3's device write leaves (3, 2), published as 4; the kind's
/// newest slot (2) then lost leaves (3, 1), other settings, which must not
/// read as 4 again, nor as anything published, and the next save writes
/// past it all. Under one past the larger it read 4; under the sum it read
/// 5, and the next save then wrote 6, the sum the cut's view had named.
#[test]
fn a_cut_then_a_lost_slot_never_repeats_a_generation() {
    let published = [1, 2, view_generation(clean(3), Some(clean(2)))];
    assert_eq!(published[2], 4);

    let after = view_generation(clean(3), Some(fell_back(1)));
    assert_eq!(after, 7);
    assert!(published.iter().all(|&p| p < after));
    let next_save = after + 1;
    assert!(published.iter().all(|&p| p < next_save));
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
