//! The plans store and its changes: saves alternate and reload, a power cut
//! keeps the old or the new plans, a newer schema is read-only, a record
//! that fails its checks is not read, and each change moves the marks and
//! the claim as the schedule relies on.

use wellfed_core::store::slot::crc32;
use wellfed_device_api::mailbox::{Claim, PLANS, Plan, PlansChange, Table, Weekdays};
use wellfed_feeder::store::plans::{Plans, PlansStatus, PlansStore, RECORD_BYTES};
use wellfed_testkit::journal_flash::{FakeJournalFlash, FlashIo, ORDERS, UNIT};

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

/// The newest valid record wins whichever unit holds it, and a newest that
/// no longer reads falls back to the other, which the next save then
/// replaces in the broken unit.
#[test]
fn the_newest_valid_record_wins_and_a_broken_one_falls_back() {
    let mut f = FakeJournalFlash::with_unit(UNIT);
    let mut s = open(&mut f);
    for at_s in [1_000, 2_000, 3_000] {
        assert!(s.save(&breakfast(at_s)));
    }
    let r = open(&mut f);
    assert_eq!((r.status, r.sequence, r.slot), (PlansStatus::Loaded, 3, 0));
    assert_eq!(r.plans, breakfast(3_000));

    // Unit 0's record, its checksum no longer matching.
    f.bytes[0][20] ^= 1;
    let mut r = open(&mut f);
    assert_eq!((r.status, r.sequence, r.slot), (PlansStatus::Loaded, 2, 1));
    assert_eq!(r.plans, breakfast(2_000));

    assert!(r.save(&breakfast(4_000)));
    let r = open(&mut f);
    assert_eq!((r.sequence, r.slot), (3, 0));
    assert_eq!(r.plans, breakfast(4_000));
}

#[test]
fn power_cut_at_every_mutation_keeps_old_or_new_and_stays_writable() {
    let new = Plans::default()
        .changed(&PlansChange::Replace {
            table: table(&[(3, ONCE)]),
            at_s: None,
        })
        .unwrap();
    // A first save, then a save over one already there.
    let histories = [None, Some(breakfast(1_000))];

    for (order, history) in ORDERS.into_iter().flat_map(|o| histories.map(|h| (o, h))) {
        let old = history.unwrap_or_default();
        let saved = |f: &mut FakeJournalFlash| {
            let mut s = open(f);
            if let Some(plans) = &history {
                assert!(s.save(plans));
            }
            s
        };

        let mut probe = FakeJournalFlash::with_unit(UNIT).ordered(order);
        let mut s = saved(&mut probe);
        let start = probe.mutations;
        assert!(s.save(&new));
        let span = i64::from(probe.mutations - start);

        for cut in 0..=span {
            let mut f = FakeJournalFlash::with_unit(UNIT).ordered(order);
            let mut s = saved(&mut f);
            f.budget = cut;
            let _ = s.save(&new);
            f.budget = -1;

            let mut r = open(&mut f);
            let label = format!("{order:?} first {} cut {cut}", history.is_none());
            assert!(r.plans == old || r.plans == new, "{label}");
            assert!(r.writable(), "{label}: {:?}", r.status);
            assert!(r.save(&new));
            assert_eq!(open(&mut f).plans, new);
        }
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
        ("unknown plan flag", |r| r[20] |= 0x80),
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

/// Dinner at 18:00 for 4 portions every day, saved disabled.
const DINNER_OFF: Plan = Plan {
    minute: 18 * 60,
    portions: 4,
    days: Weekdays::EVERY_DAY,
    enabled: false,
    call: 0,
};

/// A client's table that changes the slot a once plan's claim disabled is
/// the owner's word on that slot: giving the occurrence back enables
/// nothing the owner saved disabled, whether the loop names the claim as
/// it took it or as the store now holds it. A table that leaves the slot
/// as the claim did still gets the plan back.
#[test]
fn a_release_after_a_table_changes_the_claimed_slot_leaves_it_as_saved() {
    let once = Plans::default()
        .changed(&PlansChange::Replace {
            table: table(&[(0, ONCE)]),
            at_s: Some(1_000),
        })
        .unwrap();
    let claimed = once
        .changed(&PlansChange::Claim {
            claim: claim(0, 2_000, 4),
            revision: once.revision,
        })
        .unwrap();
    let taken = claimed.claim.unwrap();
    assert!(taken.disabled);

    let replaced = claimed
        .changed(&PlansChange::Replace {
            table: table(&[(0, DINNER_OFF)]),
            at_s: Some(2_001),
        })
        .unwrap();
    let held = replaced.claim.unwrap();
    assert!(!held.disabled, "the claim no longer enables the slot");
    for claim in [taken, held] {
        let released = replaced.changed(&PlansChange::Release { claim }).unwrap();
        assert_eq!(released.claim, None);
        assert_eq!(released.table[0], Some(DINNER_OFF), "still disabled");
    }

    // The same slot as the claim left it, with another slot changed.
    let kept = claimed
        .changed(&PlansChange::Replace {
            table: {
                let mut table = claimed.table;
                table[1] = Some(DINNER_OFF);
                table
            },
            at_s: Some(2_001),
        })
        .unwrap();
    assert_eq!(kept.claim, Some(taken));
    let released = kept
        .changed(&PlansChange::Release { claim: taken })
        .unwrap();
    assert_eq!(released.table[0], Some(ONCE), "the plan the claim took");
    assert_eq!(released.table[1], Some(DINNER_OFF));
}

/// The fed bit marks a meal the schedule took: a claim folded into the
/// mark. An interrupted claim's settle, a pass, a skip and a save that
/// resets the mark clear it; it saves and reloads in bit 6 of the plan's
/// flags, never beside the skip bit.
#[test]
fn only_a_fed_meal_sets_the_fed_bit_and_it_reloads() {
    let plans = breakfast(1_000);
    let held = claim(0, 2_000, 4);
    let claimed = plans
        .changed(&PlansChange::Claim {
            claim: held,
            revision: plans.revision,
        })
        .unwrap();
    assert!(claimed.view(true).fed(0), "the claim's meal");
    assert_eq!(claimed.fed, 0, "not yet folded");

    // The next claim folds this one into the mark.
    let next = claimed
        .changed(&PlansChange::Claim {
            claim: claim(0, 90_000, 5),
            revision: claimed.revision,
        })
        .unwrap();
    assert_eq!(next.fed, 1);
    let mut f = FakeJournalFlash::with_unit(UNIT);
    let mut s = open(&mut f);
    assert!(s.save(&next));
    assert_eq!(f.bytes[0][20] & 0x40, 0x40);
    assert_eq!(open(&mut f).plans.fed, 1);

    // A settled claim was never fed.
    let settled = claimed
        .changed(&PlansChange::Settle { claim: held })
        .unwrap();
    assert!(!settled.view(true).fed(0));
    // A pass, a skip and a save that moves the plan clear it.
    let passed = next
        .changed(&PlansChange::Pass {
            plan: 0,
            due_s: 200_000,
            revision: next.revision,
        })
        .unwrap();
    assert_eq!(passed.fed, 0);
    let skipped = next
        .changed(&PlansChange::Skip {
            plan: 0,
            due_s: 200_000,
            revision: next.revision,
        })
        .unwrap();
    assert_eq!((skipped.fed, skipped.skips), (0, 1));
    let moved = next
        .changed(&PlansChange::Replace {
            table: table(&[(
                0,
                Plan {
                    minute: 8 * 60,
                    ..BREAKFAST
                },
            )]),
            at_s: Some(100_000),
        })
        .unwrap();
    assert_eq!(moved.fed, 0);
    let kept = next
        .changed(&PlansChange::Replace {
            table: table(&[(
                0,
                Plan {
                    portions: 3,
                    ..BREAKFAST
                },
            )]),
            at_s: Some(100_000),
        })
        .unwrap();
    assert_eq!(kept.fed, 1, "the same times keep the mark and the bit");

    // Both bits at once is no record of ours.
    let mut f = FakeJournalFlash::with_unit(UNIT);
    let mut s = open(&mut f);
    assert!(s.save(&next));
    rewrite(&mut f, |record| record[20] |= 0x20);
    assert_eq!(open(&mut f).status, PlansStatus::Corrupt);
}

mod sequences {
    //! Any order of the loop's and the clients' changes: no plan is ever
    //! enabled but by the owner's own write, or by a claim giving back the
    //! very plan it took.

    use proptest::prelude::*;
    use wellfed_device_api::mailbox::{Claim, PLANS, Plan, PlansChange, Table, Weekdays};
    use wellfed_feeder::store::plans::{Plans, PlansStatus};
    use wellfed_testkit::journal_flash::{FakeJournalFlash, UNIT};

    /// Three slots keep collisions on one slot frequent.
    const SLOTS: u8 = 3;

    #[derive(Clone, Copy, Debug)]
    enum Step {
        /// A client's table: the current one or an empty one, with up to
        /// two slots set or cleared.
        Replace {
            keep: bool,
            edits: [Option<(u8, Option<Plan>)>; 2],
            at_s: u32,
        },
        /// The loop takes an occurrence; `stale` names an old revision.
        Claim {
            slot: u8,
            due_s: u32,
            high_water: u32,
            stale: bool,
        },
        /// The loop gives its claim back, named as it took it.
        Release,
        Settle,
        Skip {
            slot: u8,
            due_s: u32,
        },
        /// An undo of the skip at the slot's mark.
        Unskip {
            slot: u8,
        },
        Pass {
            slot: u8,
            due_s: u32,
        },
    }

    fn plan() -> impl Strategy<Value = Plan> {
        (
            prop_oneof![Just(7 * 60 + 30), Just(18 * 60)],
            prop_oneof![Just(2), Just(4)],
            prop_oneof![Just(Weekdays::ONCE), Just(Weekdays::EVERY_DAY)],
            any::<bool>(),
        )
            .prop_map(|(minute, portions, days, enabled)| Plan {
                minute,
                portions,
                days,
                enabled,
                call: 0,
            })
    }

    fn step() -> impl Strategy<Value = Step> {
        let slot = 0..SLOTS;
        let due = 1_000u32..5_000;
        let edit = || proptest::option::of((0..SLOTS, proptest::option::of(plan())));
        prop_oneof![
            3 => (any::<bool>(), [edit(), edit()], due.clone()).prop_map(
                |(keep, edits, at_s)| Step::Replace { keep, edits, at_s }
            ),
            3 => (slot.clone(), due.clone(), 0u32..4, proptest::bool::weighted(0.1)).prop_map(
                |(slot, due_s, high_water, stale)| Step::Claim {
                    slot,
                    due_s,
                    high_water,
                    stale,
                }
            ),
            2 => Just(Step::Release),
            1 => Just(Step::Settle),
            1 => (slot.clone(), due.clone()).prop_map(|(slot, due_s)| Step::Skip { slot, due_s }),
            1 => slot.clone().prop_map(|slot| Step::Unskip { slot }),
            1 => (slot, due).prop_map(|(slot, due_s)| Step::Pass { slot, due_s }),
        ]
    }

    /// The change `step` asks of `plans`, with `taken` the claim as the
    /// loop took it.
    fn change(plans: &Plans, step: Step, taken: Option<Claim>) -> PlansChange {
        let revision = plans.revision;
        let none = Claim {
            plan: 0,
            due_s: 1,
            high_water: 0,
            disabled: false,
        };
        match step {
            Step::Replace { keep, edits, at_s } => {
                let mut table: Table = if keep { plans.table } else { [None; PLANS] };
                for (slot, plan) in edits.into_iter().flatten() {
                    table[usize::from(slot)] = plan;
                }
                PlansChange::Replace {
                    table,
                    at_s: Some(at_s),
                }
            }
            Step::Claim {
                slot,
                due_s,
                high_water,
                stale,
            } => PlansChange::Claim {
                claim: Claim {
                    plan: slot,
                    due_s,
                    high_water,
                    disabled: false,
                },
                revision: if stale {
                    revision.wrapping_sub(1)
                } else {
                    revision
                },
            },
            Step::Release => PlansChange::Release {
                claim: taken.unwrap_or(none),
            },
            Step::Settle => PlansChange::Settle {
                claim: taken.unwrap_or(none),
            },
            Step::Skip { slot, due_s } => PlansChange::Skip {
                plan: slot,
                due_s,
                revision,
            },
            Step::Unskip { slot } => PlansChange::Unskip {
                plan: slot,
                due_s: plans.marks[usize::from(slot)],
                revision,
            },
            Step::Pass { slot, due_s } => PlansChange::Pass {
                plan: slot,
                due_s,
                revision,
            },
        }
    }

    fn disabled(plan: Option<Plan>) -> Option<Plan> {
        plan.map(|plan| Plan {
            enabled: false,
            ..plan
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(2_000))]

        #[test]
        fn no_plan_is_enabled_but_by_its_owner_or_its_own_claim(
            steps in proptest::collection::vec(step(), 1..32),
        ) {
            let mut plans = Plans::default();
            // The claim as the loop took it, and the plan it disabled.
            let mut taken: Option<Claim> = None;
            let mut took: Option<Plan> = None;

            for step in steps {
                let change = change(&plans, step, taken);
                let Some(next) = plans.changed(&change) else {
                    continue;
                };

                for slot in 0..PLANS {
                    let (before, after) = (plans.table[slot], next.table[slot]);
                    if !matches!(change, PlansChange::Replace { .. }) {
                        prop_assert_eq!(
                            disabled(before),
                            disabled(after),
                            "only a client's table changes a plan"
                        );
                    }
                    let Some(now) = after.filter(|plan| plan.enabled) else {
                        continue;
                    };
                    if before == after {
                        continue;
                    }
                    let enabled_by = match change {
                        PlansChange::Replace { .. } => true,
                        PlansChange::Unskip { plan, .. } => usize::from(plan) == slot,
                        PlansChange::Release { claim } => {
                            usize::from(claim.plan) == slot && took == Some(now)
                        }
                        _ => false,
                    };
                    prop_assert!(enabled_by, "slot {} enabled by {:?}", slot, change);
                }

                match change {
                    PlansChange::Claim { claim, .. } => {
                        let stored = next.claim.unwrap();
                        taken = Some(stored);
                        took = stored
                            .disabled
                            .then(|| plans.table[usize::from(claim.plan)])
                            .flatten();
                    }
                    PlansChange::Release { .. } | PlansChange::Settle { .. } => {
                        taken = None;
                        took = None;
                    }
                    _ => {}
                }

                // A claim that would enable a plan when given back names
                // the very plan it took, still in its slot.
                if let Some(held) = next.claim.filter(|held| held.disabled) {
                    prop_assert_eq!(
                        next.table[usize::from(held.plan)],
                        disabled(took),
                        "the claim's slot holds the plan it took"
                    );
                }

                // A slot's mark is a skip or a meal fed, never both: a
                // record with both reloads as corrupt, with no plans. And
                // whatever the changes leave reloads as it was saved.
                prop_assert_eq!(next.skips & next.fed, 0, "both bits after {:?}", change);
                let mut flash = FakeJournalFlash::with_unit(UNIT);
                prop_assert!(super::open(&mut flash).save(&next));
                let reloaded = super::open(&mut flash);
                prop_assert_eq!(reloaded.status, PlansStatus::Loaded);
                prop_assert_eq!(reloaded.plans, next, "reloaded after {:?}", change);
                plans = next;
            }
        }
    }
}

/// A skip beside mark 0 is no record this store writes; read from flash,
/// its undo is dropped rather than wrapping the mark.
#[test]
fn an_undo_of_a_skip_at_mark_zero_is_dropped() {
    let mut plans = breakfast(1_000);
    plans.marks[0] = 0;
    plans.skips = 1;
    let unskip = PlansChange::Unskip {
        plan: 0,
        due_s: 0,
        revision: plans.revision,
    };
    assert_eq!(plans.changed(&unskip), None);
}
