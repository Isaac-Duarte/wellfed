//! The plans store and its changes: saves alternate and reload, a power cut
//! keeps the old or the new plans, a newer schema is read-only, a record
//! that fails its checks is not read, and each change moves the marks and
//! the claim as the schedule relies on.

use wellfed_core::store::slot::crc32;
use wellfed_device_api::mailbox::{Claim, PLANS, Plan, PlansChange, Table, Weekdays};
use wellfed_feeder::store::plans::{Plans, PlansStatus, PlansStore, RECORD_BYTES};
use wellfed_testkit::journal_flash::{FakeJournalFlash, FlashIo, UNIT};

fn open(f: &mut FakeJournalFlash) -> PlansStore<FlashIo> {
    let mut s = PlansStore::default();
    // SAFETY: the fake flash outlives the store in every test.
    unsafe { s.open(f.io()) };
    s
}

const BREAKFAST: Plan = Plan {
    minute: 7 * 60 + 30,
    portions: 2,
    days: Weekdays::EVERY_DAY,
    enabled: true,
    call: 0,
};

const ONCE: Plan = Plan {
    days: Weekdays::ONCE,
    ..BREAKFAST
};

fn table(plans: &[(usize, Plan)]) -> Table {
    let mut table = [None; PLANS];
    for &(slot, plan) in plans {
        table[slot] = Some(plan);
    }
    table
}

/// Plans with breakfast in slot 0, saved at `at_s`.
fn breakfast(at_s: u32) -> Plans {
    let change = PlansChange::Replace {
        table: table(&[(0, BREAKFAST)]),
        at_s: Some(at_s),
    };
    Plans::default().changed(&change).unwrap()
}

fn claim(plan: u8, due_s: u32, high_water: u32) -> Claim {
    Claim {
        plan,
        due_s,
        high_water,
        disabled: false,
    }
}

#[test]
fn nothing_until_saved_then_alternate_and_reload() {
    let mut f = FakeJournalFlash::with_unit(UNIT);
    let mut s = open(&mut f);
    assert_eq!(s.status, PlansStatus::Empty);
    assert_eq!(s.plans, Plans::default());

    let first = breakfast(1_000);
    let second = first
        .changed(&PlansChange::Claim {
            claim: claim(0, 2_000, 4),
            revision: first.revision,
        })
        .unwrap();
    assert!(s.save(&first));
    assert!(s.save(&second));
    let r = open(&mut f);
    assert_eq!((r.status, r.sequence, r.slot), (PlansStatus::Loaded, 2, 1));
    assert_eq!(r.plans, second);
}

#[test]
fn power_cut_at_every_mutation_keeps_old_or_new_and_stays_writable() {
    let old = breakfast(1_000);
    let new = Plans::default()
        .changed(&PlansChange::Replace {
            table: table(&[(3, ONCE)]),
            at_s: None,
        })
        .unwrap();
    let mut probe = FakeJournalFlash::with_unit(UNIT);
    let mut s = open(&mut probe);
    assert!(s.save(&old));
    let start = probe.mutations;
    assert!(s.save(&new));
    let span = i64::from(probe.mutations - start);

    for cut in 0..=span {
        let mut f = FakeJournalFlash::with_unit(UNIT);
        let mut s = open(&mut f);
        assert!(s.save(&old));
        f.budget = cut;
        let _ = s.save(&new);
        f.budget = -1;

        let mut r = open(&mut f);
        assert!(r.plans == old || r.plans == new, "cut {cut}");
        assert!(r.writable(), "cut {cut}: {:?}", r.status);
        assert!(r.save(&new));
        assert_eq!(open(&mut f).plans, new);
    }
}

#[test]
fn a_plans_meal_call_round_trips_in_its_flag_bits() {
    let mut f = FakeJournalFlash::with_unit(UNIT);
    let mut s = open(&mut f);
    let calling = Plan {
        call: wellfed_core::clip::PLAYS_MAX,
        ..BREAKFAST
    };
    let plans = Plans::default()
        .changed(&PlansChange::Replace {
            table: table(&[(0, BREAKFAST), (1, calling)]),
            at_s: Some(1_000),
        })
        .unwrap();
    assert!(s.save(&plans));

    // Slot 1's flags: present, enabled and five plays in bits 2-4; slot 0,
    // with no call, keeps the flags a record had before plans had calls.
    assert_eq!(f.bytes[0][20], 0b0000_0011);
    assert_eq!(f.bytes[0][25], 0b0001_0111);
    let r = open(&mut f);
    assert_eq!(r.status, PlansStatus::Loaded);
    assert_eq!(r.plans.table[1], Some(calling));

    // A call past the limit is no record this build reads: the plans are
    // not written over.
    rewrite(&mut f, |record| record[25] = 0b0001_1011);
    assert_eq!(open(&mut f).status, PlansStatus::Corrupt);
}

/// Rewrite slot 0's record with `change` applied and a valid checksum.
fn rewrite(f: &mut FakeJournalFlash, change: impl FnOnce(&mut [u8])) {
    let mut record = f.bytes[0][..RECORD_BYTES].to_vec();
    change(&mut record);
    let crc_at = RECORD_BYTES - 8;
    let crc = crc32(&record[..crc_at]);
    record[crc_at..crc_at + 4].copy_from_slice(&crc.to_le_bytes());
    f.bytes[0][..RECORD_BYTES].copy_from_slice(&record);
}

#[test]
fn a_newer_schema_is_read_only_with_no_plans() {
    let mut f = FakeJournalFlash::with_unit(UNIT);
    let mut s = open(&mut f);
    assert!(s.save(&breakfast(1_000)));
    rewrite(&mut f, |record| record[8] = 5);

    let mut r = open(&mut f);
    assert_eq!(
        (r.status, r.plans),
        (PlansStatus::ReadOnly, Plans::default())
    );
    assert!(!r.writable());
    assert!(!r.save(&breakfast(2_000)));
}

/// A record of an older major (3, a 128-byte record from before the
/// spare room; 2, from before the skip bit; 1, from before the meal call)
/// is not read, and not written over: the store opens corrupt, with no
/// plans, until a factory reset erases it.
#[test]
fn an_older_record_is_not_read_or_written_over() {
    for version in [1, 2, 3] {
        let mut f = FakeJournalFlash::with_unit(UNIT);
        let mut s = open(&mut f);
        assert!(s.save(&breakfast(1_000)));
        rewrite(&mut f, |record| record[8] = version);
        let before = f.bytes.clone();

        let mut r = open(&mut f);
        let opened = (r.status, r.plans);
        assert_eq!(
            opened,
            (PlansStatus::Corrupt, Plans::default()),
            "{version}"
        );
        assert!(!r.save(&breakfast(2_000)));
        assert_eq!(f.bytes, before, "{version}");
    }
}

/// A current record wins over an older one in the other sector, even
/// one whose save count is higher: the older is not read at all.
#[test]
fn a_current_record_wins_over_an_older_version_with_a_higher_sequence() {
    let mut f = FakeJournalFlash::with_unit(UNIT);
    let mut s = open(&mut f);
    let current = skipped_breakfast();
    assert!(s.save(&breakfast(1_000)));
    assert!(s.save(&current));
    // Sector 0's record: a version 2 save with a higher count.
    rewrite(&mut f, |record| {
        record[8] = 2;
        record[12..16].copy_from_slice(&9u32.to_le_bytes());
    });

    let r = open(&mut f);
    assert_eq!((r.status, r.sequence, r.slot), (PlansStatus::Loaded, 2, 1));
    assert_eq!(r.plans, current);
}

#[test]
fn records_that_fail_their_checks_are_not_read() {
    type Change = fn(&mut [u8]);
    let cases: [(&str, Change); 8] = [
        ("minute past the day", |r| {
            r[21..23].copy_from_slice(&1440u16.to_le_bytes());
        }),
        ("no portions", |r| r[23] = 0),
        ("days past Sunday", |r| r[24] = 0x80),
        ("unknown plan flag", |r| r[20] |= 0x40),
        ("bytes in an empty slot", |r| r[26] = 1),
        ("claim past the last slot", |r| {
            r[110] = 1;
            r[111] = 10;
        }),
        ("unknown claim flag", |r| r[110] = 0xee),
        ("sequence 0", |r| r[12..16].fill(0)),
    ];
    for (name, change) in cases {
        let mut f = FakeJournalFlash::with_unit(UNIT);
        let mut s = open(&mut f);
        assert!(s.save(&breakfast(1_000)));
        rewrite(&mut f, change);
        let r = open(&mut f);
        assert_eq!(
            (r.status, r.plans),
            (PlansStatus::Corrupt, Plans::default()),
            "{name}"
        );
    }
}

#[test]
fn a_replace_starts_new_and_moved_plans_from_their_save() {
    let saved = breakfast(1_000);
    assert_eq!(saved.marks[0], 1_000);
    assert_eq!(saved.revision, 1);

    // Later: slot 0's portions change (its occurrences stay), slot 1 is
    // new and slot 2 moves; each moved or new plan starts at the save.
    let mut earlier = Plans::default()
        .changed(&PlansChange::Replace {
            table: table(&[(0, BREAKFAST), (2, BREAKFAST)]),
            at_s: Some(1_000),
        })
        .unwrap();
    earlier.marks[0] = 5_000;
    let next = earlier
        .changed(&PlansChange::Replace {
            table: table(&[
                (
                    0,
                    Plan {
                        portions: 3,
                        ..BREAKFAST
                    },
                ),
                (1, BREAKFAST),
                (
                    2,
                    Plan {
                        minute: 8 * 60,
                        ..BREAKFAST
                    },
                ),
            ]),
            at_s: Some(9_000),
        })
        .unwrap();
    assert_eq!(next.marks[..3], [5_000, 9_000, 9_000]);

    // A deleted slot forgets its mark; one saved without a clock has none.
    let cleared = next
        .changed(&PlansChange::Replace {
            table: table(&[(0, BREAKFAST), (4, BREAKFAST)]),
            at_s: None,
        })
        .unwrap();
    assert_eq!(cleared.marks[..5], [5_000, 0, 0, 0, 0]);
    assert_eq!(cleared.revision, next.revision + 1);
}

#[test]
fn a_claim_marks_its_occurrence_and_a_plan_that_runs_once_disables_itself() {
    let plans = Plans::default()
        .changed(&PlansChange::Replace {
            table: table(&[(0, BREAKFAST), (1, ONCE)]),
            at_s: Some(1_000),
        })
        .unwrap();

    // A repeating plan's claim leaves the table and its revision alone.
    let first = claim(0, 2_000, 4);
    let claimed = plans
        .changed(&PlansChange::Claim {
            claim: first,
            revision: plans.revision,
        })
        .unwrap();
    assert_eq!(claimed.claim, Some(first));
    assert_eq!(claimed.view(true).mark(0), Some(2_000));
    assert_eq!(claimed.revision, plans.revision);

    // The next claim folds the last into its mark; a plan that runs once
    // is disabled with its claim, which says so.
    let once = claim(1, 3_000, 5);
    let again = claimed
        .changed(&PlansChange::Claim {
            claim: once,
            revision: claimed.revision,
        })
        .unwrap();
    assert_eq!(again.marks[0], 2_000);
    assert_eq!(again.claim.map(|c| c.disabled), Some(true));
    assert_eq!(again.table[1].map(|p| p.enabled), Some(false));
    assert_eq!(again.revision, claimed.revision + 1);

    // Given back, the occurrence and the plan are as they were.
    let held = again.claim.unwrap();
    let released = again
        .changed(&PlansChange::Release { claim: held })
        .unwrap();
    assert_eq!(released.claim, None);
    assert_eq!(released.view(true).mark(1), Some(1_000));
    assert_eq!(released.table[1].map(|p| p.enabled), Some(true));
}

#[test]
fn a_change_against_plans_that_moved_on_is_dropped() {
    let plans = breakfast(1_000);
    let stale = PlansChange::Claim {
        claim: claim(0, 2_000, 4),
        revision: plans.revision + 1,
    };
    assert_eq!(plans.changed(&stale), None);
    let empty_slot = PlansChange::Pass {
        plan: 5,
        due_s: 2_000,
        revision: plans.revision,
    };
    assert_eq!(plans.changed(&empty_slot), None);
    let no_claim = PlansChange::Release {
        claim: claim(0, 2_000, 4),
    };
    assert_eq!(plans.changed(&no_claim), None);
    let disabled = Plans::default()
        .changed(&PlansChange::Replace {
            table: table(&[(
                0,
                Plan {
                    enabled: false,
                    ..BREAKFAST
                },
            )]),
            at_s: Some(1_000),
        })
        .unwrap();
    let claim_disabled = PlansChange::Claim {
        claim: claim(0, 2_000, 4),
        revision: disabled.revision,
    };
    assert_eq!(disabled.changed(&claim_disabled), None);
}

#[test]
fn a_settle_and_a_pass_spend_the_occurrence() {
    let plans = breakfast(1_000);
    let held = claim(0, 2_000, 4);
    let claimed = plans
        .changed(&PlansChange::Claim {
            claim: held,
            revision: plans.revision,
        })
        .unwrap();
    let settled = claimed
        .changed(&PlansChange::Settle { claim: held })
        .unwrap();
    assert_eq!((settled.claim, settled.marks[0]), (None, 2_000));

    let passed = settled
        .changed(&PlansChange::Pass {
            plan: 0,
            due_s: 9_000,
            revision: settled.revision,
        })
        .unwrap();
    assert_eq!(passed.marks[0], 9_000);
    // A mark never moves back.
    let behind = passed
        .changed(&PlansChange::Pass {
            plan: 0,
            due_s: 4_000,
            revision: passed.revision,
        })
        .unwrap();
    assert_eq!(behind.marks[0], 9_000);
}

#[test]
fn a_skip_moves_the_mark_up_to_its_occurrence_and_an_undo_moves_it_back() {
    let plans = breakfast(1_000);
    let skip = PlansChange::Skip {
        plan: 0,
        due_s: 90_000,
        revision: plans.revision,
    };
    let skipped = plans.changed(&skip).unwrap();
    assert_eq!(skipped.marks[0], 90_000);
    assert_eq!(
        skipped.revision, plans.revision,
        "a mark is no table change"
    );
    assert!(skipped.table[0].unwrap().enabled);

    // Skipped once, so a second skip of it changes nothing.
    assert_eq!(skipped.changed(&skip), None);

    let unskip = PlansChange::Unskip {
        plan: 0,
        due_s: 90_000,
        revision: skipped.revision,
    };
    let undone = skipped.changed(&unskip).unwrap();
    assert_eq!(undone.marks[0], 89_999, "the second before the meal");
    assert_eq!(undone.table, plans.table);
    assert_eq!(undone.changed(&unskip), None, "undone once");
}

#[test]
fn skipping_a_plan_that_runs_once_disables_it_and_the_undo_enables_it() {
    let plans = Plans::default()
        .changed(&PlansChange::Replace {
            table: table(&[(2, ONCE)]),
            at_s: Some(1_000),
        })
        .unwrap();
    let skipped = plans
        .changed(&PlansChange::Skip {
            plan: 2,
            due_s: 90_000,
            revision: plans.revision,
        })
        .unwrap();
    assert_eq!(skipped.marks[2], 90_000);
    assert!(!skipped.table[2].unwrap().enabled);
    assert_eq!(skipped.revision, plans.revision + 1);

    // The undo names the revision the skip left.
    let unskip = |revision| PlansChange::Unskip {
        plan: 2,
        due_s: 90_000,
        revision,
    };
    assert_eq!(skipped.changed(&unskip(plans.revision)), None);
    let undone = skipped.changed(&unskip(skipped.revision)).unwrap();
    assert_eq!(undone.marks[2], 89_999);
    assert!(undone.table[2].unwrap().enabled);
    assert_eq!(undone.revision, skipped.revision + 1);
}

#[test]
fn a_skip_or_undo_that_no_longer_fits_the_store_is_dropped() {
    let plans = breakfast(1_000);
    let skip = |plan, due_s, revision| PlansChange::Skip {
        plan,
        due_s,
        revision,
    };

    // A table that moved on, an empty slot, a disabled plan, a mark at or
    // past the occurrence, and a slot out of range.
    assert_eq!(plans.changed(&skip(0, 90_000, plans.revision + 1)), None);
    assert_eq!(plans.changed(&skip(1, 90_000, plans.revision)), None);
    assert_eq!(plans.changed(&skip(0, 1_000, plans.revision)), None);
    assert_eq!(plans.changed(&skip(0, 900, plans.revision)), None);
    assert_eq!(plans.changed(&skip(10, 90_000, plans.revision)), None);
    let disabled = Plans::default()
        .changed(&PlansChange::Replace {
            table: table(&[(
                0,
                Plan {
                    enabled: false,
                    ..BREAKFAST
                },
            )]),
            at_s: Some(1_000),
        })
        .unwrap();
    assert_eq!(disabled.changed(&skip(0, 90_000, disabled.revision)), None);

    // An undo needs the mark exactly at the skipped occurrence and a
    // repeating plan still enabled.
    let skipped = plans.changed(&skip(0, 90_000, plans.revision)).unwrap();
    let unskip = |due_s| PlansChange::Unskip {
        plan: 0,
        due_s,
        revision: skipped.revision,
    };
    assert_eq!(skipped.changed(&unskip(80_000)), None);
    assert_eq!(plans.changed(&unskip(90_000)), None, "never skipped");
    // Disabling the plan keeps its mark, and so its skip: an undo still
    // takes the mark back, and the disabled plan runs nothing.
    let off = skipped
        .changed(&PlansChange::Replace {
            table: table(&[(
                0,
                Plan {
                    enabled: false,
                    ..BREAKFAST
                },
            )]),
            at_s: Some(2_000),
        })
        .unwrap();
    assert_eq!(off.marks[0], 90_000, "disabling keeps the mark");
    assert!(off.skipped(0));
    let unskip_off = PlansChange::Unskip {
        plan: 0,
        due_s: 90_000,
        revision: off.revision,
    };
    let undone = off.changed(&unskip_off).unwrap();
    assert!(!undone.table[0].unwrap().enabled);
}

#[test]
fn a_replace_that_moves_a_skipped_plan_drops_the_skip_and_one_that_keeps_its_times_keeps_it() {
    let plans = breakfast(1_000);
    let skipped = plans
        .changed(&PlansChange::Skip {
            plan: 0,
            due_s: 90_000,
            revision: plans.revision,
        })
        .unwrap();
    let replace = |plan| PlansChange::Replace {
        table: table(&[(0, plan)]),
        at_s: Some(2_000),
    };

    let more = skipped
        .changed(&replace(Plan {
            portions: 5,
            ..BREAKFAST
        }))
        .unwrap();
    assert_eq!(more.marks[0], 90_000);
    let moved = skipped
        .changed(&replace(Plan {
            minute: 8 * 60,
            ..BREAKFAST
        }))
        .unwrap();
    assert_eq!(moved.marks[0], 2_000);
}

#[test]
fn a_skip_saves_and_reloads_with_its_mark() {
    let mut f = FakeJournalFlash::with_unit(UNIT);
    let mut s = open(&mut f);
    assert!(s.save(&breakfast(1_000)));
    let skip = PlansChange::Skip {
        plan: 0,
        due_s: 90_000,
        revision: s.plans.revision,
    };
    assert_eq!(s.apply(&skip), Some(true));
    assert_eq!(s.replaced, 0, "a skip is no client table");

    assert_eq!(open(&mut f).plans.marks[0], 90_000);
}

/// Breakfast in slot 0 with its meal at 90,000 skipped.
fn skipped_breakfast() -> Plans {
    let plans = breakfast(1_000);
    plans
        .changed(&PlansChange::Skip {
            plan: 0,
            due_s: 90_000,
            revision: plans.revision,
        })
        .unwrap()
}

fn unskip(plans: &Plans) -> PlansChange {
    PlansChange::Unskip {
        plan: 0,
        due_s: 90_000,
        revision: plans.revision,
    }
}

#[test]
fn only_a_skip_sets_the_skip_bit_and_its_undo_needs_it() {
    let skipped = skipped_breakfast();
    assert!(skipped.skipped(0));
    assert_eq!(skipped.view(true).skipped(0), Some(90_000));
    let undone = skipped.changed(&unskip(&skipped)).unwrap();
    assert!(!undone.skipped(0));
    assert_eq!(undone.view(true).skipped(0), None);

    // A mark at the same instant that no skip set: a claim settled, a
    // pass, or a save. None is undone.
    let plans = breakfast(1_000);
    let settled = plans
        .changed(&PlansChange::Claim {
            claim: claim(0, 90_000, 4),
            revision: plans.revision,
        })
        .and_then(|claimed| {
            claimed.changed(&PlansChange::Settle {
                claim: claim(0, 90_000, 4),
            })
        })
        .unwrap();
    let passed = plans
        .changed(&PlansChange::Pass {
            plan: 0,
            due_s: 90_000,
            revision: plans.revision,
        })
        .unwrap();
    let saved = breakfast(90_000);
    for marked in [settled, passed, saved] {
        assert_eq!(marked.marks[0], 90_000);
        assert!(!marked.skipped(0));
        assert_eq!(marked.changed(&unskip(&marked)), None);
    }
}

#[test]
fn every_change_that_moves_a_skipped_mark_clears_its_bit() {
    let skipped = skipped_breakfast();
    let revision = skipped.revision;

    let claimed = skipped
        .changed(&PlansChange::Claim {
            claim: claim(0, 176_400, 4),
            revision,
        })
        .unwrap();
    assert!(!claimed.skipped(0), "a claim of the plan");
    let settled = claimed
        .changed(&PlansChange::Settle {
            claim: claimed.claim.unwrap(),
        })
        .unwrap();
    assert!(!settled.skipped(0));
    let passed = skipped
        .changed(&PlansChange::Pass {
            plan: 0,
            due_s: 176_400,
            revision,
        })
        .unwrap();
    assert!(!passed.skipped(0), "a pass");
    let moved = skipped
        .changed(&PlansChange::Replace {
            table: table(&[(
                0,
                Plan {
                    minute: 8 * 60,
                    ..BREAKFAST
                },
            )]),
            at_s: Some(2_000),
        })
        .unwrap();
    assert!(!moved.skipped(0), "a new time");
    let deleted = skipped
        .changed(&PlansChange::Replace {
            table: table(&[]),
            at_s: Some(2_000),
        })
        .unwrap();
    assert!(!deleted.skipped(0), "a deleted plan");

    // A save that keeps the plan's times keeps the skip.
    let more = skipped
        .changed(&PlansChange::Replace {
            table: table(&[(
                0,
                Plan {
                    portions: 5,
                    ..BREAKFAST
                },
            )]),
            at_s: Some(2_000),
        })
        .unwrap();
    assert!(more.skipped(0));
    assert_eq!(more.changed(&unskip(&more)).unwrap().marks[0], 89_999);
}

#[test]
fn folding_another_slot_s_claim_leaves_a_skip_unless_it_raises_the_mark() {
    // Slot 0's older claim folds into its mark when slot 1 claims; it is
    // below the skipped meal, so the skip stands.
    let plans = Plans::default()
        .changed(&PlansChange::Replace {
            table: table(&[(0, BREAKFAST), (1, BREAKFAST)]),
            at_s: Some(1_000),
        })
        .unwrap();
    let claimed = plans
        .changed(&PlansChange::Claim {
            claim: claim(0, 3_600, 4),
            revision: plans.revision,
        })
        .unwrap();
    let skipped = claimed
        .changed(&PlansChange::Skip {
            plan: 0,
            due_s: 90_000,
            revision: claimed.revision,
        })
        .unwrap();
    let other = skipped
        .changed(&PlansChange::Claim {
            claim: claim(1, 4_000, 5),
            revision: skipped.revision,
        })
        .unwrap();
    assert_eq!(other.marks[0], 90_000);
    assert!(other.skipped(0));
}

#[test]
fn a_release_moves_no_mark_and_the_claim_already_cleared_the_skip() {
    let skipped = skipped_breakfast();
    let held = claim(0, 176_400, 4);
    let claimed = skipped
        .changed(&PlansChange::Claim {
            claim: held,
            revision: skipped.revision,
        })
        .unwrap();
    let released = claimed
        .changed(&PlansChange::Release { claim: held })
        .unwrap();
    assert_eq!(released.marks, claimed.marks);
    assert!(!released.skipped(0));
}

#[test]
fn a_skip_survives_a_reload_and_its_undo_then_works() {
    let mut f = FakeJournalFlash::with_unit(UNIT);
    let mut s = open(&mut f);
    assert!(s.save(&skipped_breakfast()));
    // Bit 5 of slot 0's flags.
    assert_eq!(f.bytes[0][20] & 0x20, 0x20);

    let mut r = open(&mut f);
    assert_eq!(r.status, PlansStatus::Loaded);
    assert!(r.plans.skipped(0));
    let undo = unskip(&r.plans);
    assert_eq!(r.apply(&undo), Some(true));
    assert_eq!(open(&mut f).plans.marks[0], 89_999);
    assert!(!open(&mut f).plans.skipped(0));
}
