//! The feeding plans on NOR (storage sectors 11 and 12): the plan in each
//! slot, each slot's mark, and the occurrence the loop last took.
//!
//! A 256-byte record in the dual-slot store (`wellfed_core::store::slot`),
//! little-endian:
//!
//! | Bytes | Field |
//! | --- | --- |
//! | 0..12 | magic `PLFPLN01`, major 4, minor, record size |
//! | 12..16 | sequence: the save count, which picks the newest record |
//! | 16..20 | revision: bumped by every change to the table |
//! | 20..70 | ten plans of 5 bytes: flags (bit 0 present, bit 1 enabled, bits 2-4 the meal call's plays, bit 5 the mark is a client's skip, bit 6 the mark is a meal fed), minute (u16), portions, weekdays |
//! | 70..110 | ten marks (Unix seconds, u32; 0 for none) |
//! | 110..120 | the claim: flags (bit 0 present, bit 1 it disabled a plan that runs once), slot, due (Unix seconds, u32), the journal's high water (u32) |
//! | 120..248 | spare, zero |
//! | 248..256 | CRC-32/ISO-HDLC of bytes 0..248, and the `CMIT` commit marker |
//!
//! An empty slot and an absent claim are all zero, and a plan is stored
//! only if it is valid, so one encoding exists per value. This build reads
//! no older major: its first boot over one opens [`PlansStatus::Corrupt`]
//! until a factory reset erases the plans.
//!
//! **The spare room** is for minors that add per-plan grams and more
//! plans. Bank 0 runs this build's reader for good and keeps those bytes
//! when it rewrites the table, but an older build moves plans between slots
//! and changes them without knowing what such a field says. So a minor
//! that adds a per-plan field ties it to the revision it was written at,
//! and a reader that finds another revision ignores it: every table change
//! bumps the revision, so an older build's change voids the field rather
//! than leave it on another plan. Plans past the tenth an older build never
//! runs or marks, so a minor adding them gives each its own revision tag
//! and mark the same way.
//!
//! **Marks** make each occurrence run at most once. A mark is written when
//! the loop takes an occurrence (the claim), passes one it will never run,
//! or a client saves the plan; the loop never runs an occurrence at or
//! before it, whatever the clock does after. A client's skip moves a mark
//! up to an occurrence still to come and sets the slot's skip bit; its
//! undo needs the bit, and moves the mark back to the second before that
//! occurrence. The bit, not the mark alone, tells a skip from a claim, a
//! pass or a save that left the mark at the same instant: after a
//! backward clock jump those can look like a skip still to come, and
//! undoing one would run its occurrence twice. The fed bit likewise tells
//! a meal the schedule took from one it passed as missed: a change of zone
//! repeats only the day's meal that was fed or skipped.
//!
//! The two sectors were reserved for plans before this store, so a record
//! here is ours; the newest valid one wins, and a torn newer slot falls
//! back to the older. With none the table starts empty and writable, but
//! only when nothing was ever committed there: a record of this store the
//! build cannot read (an older major, fields that fail their checks, a
//! torn record beside no valid one) opens [`PlansStatus::Corrupt`], and a
//! newer major [`PlansStatus::ReadOnly`]. Either way there are no plans,
//! so nothing runs that this build cannot read, and the owner's plans are
//! never written over by an empty table. The factory reset erases the
//! sectors, which is the way out of both.

use core::ffi::c_uint;

use wellfed_core::store::slot::{
    DualSlot, Extent, Layout, Newest, SlotIo, Version, get_u32, put_u32, seal,
};
use wellfed_device_api::mailbox::{Claim, PLANS, Plan, PlansChange, PlansView, Table, Weekdays};
use wellfed_proto::api::{EventBody, Source};

pub const VERSION: Version = Version::new(4, 0);
/// The whole record.
pub const RECORD_BYTES: usize = 256;
const CRC_AT: usize = RECORD_BYTES - 8;
const LAYOUT: Layout = Layout::new(
    *b"PLFPLN01",
    VERSION,
    CLAIM_END.next_multiple_of(4),
    Extent::Record,
);
const _: () = assert!(LAYOUT.fits(RECORD_BYTES));
const SEQUENCE_AT: usize = 12;
const REVISION_AT: usize = 16;
const PLANS_AT: usize = 20;
const PLAN_BYTES: usize = 5;
const MARKS_AT: usize = PLANS_AT + PLANS * PLAN_BYTES;
const CLAIM_AT: usize = MARKS_AT + PLANS * 4;
const CLAIM_END: usize = CLAIM_AT + 10;

const PRESENT: u8 = 1;
const ENABLED: u8 = 2;
/// The slot's mark is a client's skip (`Plans::skips`).
const SKIPPED: u8 = 1 << 5;
/// The slot's mark is a meal fed: a claim folded into it (`Plans::fed`).
const FED: u8 = 1 << 6;
/// A plan's meal call, in plays (0 to `clip::PLAYS_MAX`, below 8).
const CALL_SHIFT: u8 = 2;
const CALL_MASK: u8 = 0b111 << CALL_SHIFT;
const DISABLED: u8 = 2;

const _: () = assert!(wellfed_core::clip::PLAYS_MAX <= CALL_MASK >> CALL_SHIFT);

const _: () = assert!(CLAIM_END <= CRC_AT);

/// What the store holds: the table and what the loop keeps beside it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Plans {
    pub table: Table,
    pub marks: [u32; PLANS],
    /// Bit `slot` set: that slot's mark is a client's skip of the
    /// occurrence at it, which an undo may take back. Only a skip sets it,
    /// and every other change that moves the slot's mark clears it, so a
    /// mark left by a claim, a pass or a save is never undone.
    pub skips: u16,
    /// Bit `slot` set: that slot's mark is a meal fed, a claim folded into
    /// it. Only a fold sets it, and every other change that moves the
    /// slot's mark clears it, an interrupted claim's settle among them.
    pub fed: u16,
    pub claim: Option<Claim>,
    pub revision: u32,
}

const _: () = assert!(PLANS <= u16::BITS as usize);

impl Plans {
    /// The view the writer publishes.
    pub const fn view(&self, writable: bool) -> PlansView {
        PlansView {
            table: self.table,
            marks: self.marks,
            skips: self.skips,
            fed: self.fed,
            claim: self.claim,
            revision: self.revision,
            replaced: 0,
            writable,
        }
    }

    /// The plans after `change`; `None` when it is dropped (the loop's,
    /// taken against a table or a claim that has moved on, or naming no
    /// plan). Shared by the writer and the simulator.
    pub fn changed(&self, change: &PlansChange) -> Option<Self> {
        let mut next = *self;
        match *change {
            PlansChange::Replace { table, at_s } => {
                for (slot, (new, old)) in table.iter().zip(&self.table).enumerate() {
                    let kept = match (new, old) {
                        (None, _) => None,
                        // Re-enabling starts the plan afresh, as a new one:
                        // its occurrences while it was off never run.
                        (Some(new), Some(old))
                            if new.same_times(old) && (old.enabled || !new.enabled) =>
                        {
                            Some(self.marks[slot])
                        }
                        (Some(_), _) => None,
                    };
                    // A mark kept keeps its skip; a mark reset drops it.
                    next.marks[slot] = match (kept, new) {
                        (Some(mark), _) => mark,
                        (None, None) => 0,
                        (None, Some(_)) => at_s.unwrap_or(0),
                    };
                    if kept.is_none() {
                        next.set_skip(slot, false);
                        next.set_fed(slot, false);
                    }
                }
                // A claim that disabled its plan enables it again when given
                // back, which is right only for the plan it took. Once the
                // client's table puts anything else in the slot, the table
                // is the owner's word on it, and the release leaves it be.
                if let Some(claim) = self.claim
                    && claim.disabled
                    && table.get(usize::from(claim.plan)) != self.table.get(usize::from(claim.plan))
                {
                    next.claim = Some(Claim {
                        disabled: false,
                        ..claim
                    });
                }
                next.table = table;
                next.revision = self.revision.wrapping_add(1);
            }
            PlansChange::Claim { claim, revision } => {
                let plan = self.plan(claim.plan, revision)?;
                if let Some(old) = self.claim {
                    next.fold(old);
                }
                let disabled = plan.days.once();
                if disabled {
                    next.set_enabled(claim.plan, false);
                }
                // The claim is the slot's mark from now on, past any skip.
                next.set_skip(usize::from(claim.plan), false);
                next.claim = Some(Claim { disabled, ..claim });
            }
            PlansChange::Release { claim } => {
                let held = self.held(claim)?;
                // No mark moves: the claim cleared the slot's skip when
                // it was taken, and nothing gives that back.
                next.claim = None;
                if held.disabled {
                    next.set_enabled(held.plan, true);
                }
            }
            PlansChange::Settle { claim } => {
                // The claim's feed was never reserved: its occurrence is
                // spent, but no meal was fed.
                let held = self.held(claim)?;
                if next.fold(held) {
                    next.set_fed(usize::from(held.plan), false);
                }
                next.set_skip(usize::from(held.plan), false);
                next.claim = None;
            }
            PlansChange::Pass {
                plan,
                due_s,
                revision,
            } => {
                let found = self.plan(plan, revision)?;
                // A plan saved without a clock never expected the
                // occurrence passed before the clock returned: one that
                // runs once keeps waiting for its next.
                let expected = self.marked(plan);
                let mark = next.marks.get_mut(usize::from(plan))?;
                let raised = due_s > *mark;
                *mark = (*mark).max(due_s);
                next.set_skip(usize::from(plan), false);
                if raised {
                    next.set_fed(usize::from(plan), false);
                }
                if found.days.once() && expected {
                    next.set_enabled(plan, false);
                }
            }
            PlansChange::Skip {
                plan,
                due_s,
                revision,
            } => {
                let found = self.plan(plan, revision)?;
                let mark = next.marks.get_mut(usize::from(plan))?;
                if *mark >= due_s {
                    return None;
                }
                *mark = due_s;
                next.set_skip(usize::from(plan), true);
                next.set_fed(usize::from(plan), false);
                if found.days.once() {
                    next.set_enabled(plan, false);
                }
            }
            PlansChange::Unskip {
                plan,
                due_s,
                revision,
            } => {
                let found = (*self.table.get(usize::from(plan))?)?;
                let slot = usize::from(plan);
                if revision != self.revision || !self.skipped(slot) {
                    return None;
                }
                let mark = next.marks.get_mut(slot)?;
                if *mark != due_s {
                    return None;
                }
                // The mark before the skip was below `due_s`, so this is
                // never lower than it, and no occurrence of the plan lies
                // between the two: the meal runs, and nothing else does. A
                // skip at 0 is no record this store writes.
                *mark = due_s.checked_sub(1)?;
                next.set_skip(slot, false);
                next.set_fed(slot, false);
                if found.days.once() {
                    next.set_enabled(plan, true);
                }
            }
        }
        Some(next)
    }

    fn set_fed(&mut self, slot: usize, fed: bool) {
        if slot < PLANS {
            let bit = 1 << slot;
            self.fed = if fed { self.fed | bit } else { self.fed & !bit };
        }
    }

    /// The stored claim, if it is `claim`'s occurrence. Whether it still
    /// enables its plan when given back is the store's to say, not the
    /// loop's copy: a client's table may have changed the slot since the
    /// loop read it.
    fn held(&self, claim: Claim) -> Option<Claim> {
        self.claim.filter(|held| {
            (held.plan, held.due_s, held.high_water) == (claim.plan, claim.due_s, claim.high_water)
        })
    }

    /// Whether slot `slot`'s mark is a client's skip.
    pub const fn skipped(&self, slot: usize) -> bool {
        slot < PLANS && self.skips & (1 << slot) != 0
    }

    fn set_skip(&mut self, slot: usize, skipped: bool) {
        if slot < PLANS {
            let bit = 1 << slot;
            self.skips = if skipped {
                self.skips | bit
            } else {
                self.skips & !bit
            };
        }
    }

    /// The enabled plan in `slot`, if the table is still at `revision`.
    fn plan(&self, slot: u8, revision: u32) -> Option<Plan> {
        let plan = (*self.table.get(usize::from(slot))?)?;
        (revision == self.revision && plan.enabled).then_some(plan)
    }

    /// Whether slot `slot` has a mark, the claim's occurrence included.
    fn marked(&self, slot: u8) -> bool {
        let claimed = self.claim.is_some_and(|claim| claim.plan == slot);
        claimed
            || self
                .marks
                .get(usize::from(slot))
                .is_some_and(|&mark| mark != 0)
    }

    /// The claim's occurrence into its plan's mark; a mark it raises is a
    /// meal fed, and no longer a skip. Whether it raised the mark. A slot a
    /// client's table has emptied since keeps no fed bit: the record has
    /// no flags for an empty slot, and no plan there could repeat a meal.
    fn fold(&mut self, claim: Claim) -> bool {
        let slot = usize::from(claim.plan);
        let present = self.table.get(slot).is_some_and(Option::is_some);
        if let Some(mark) = self.marks.get_mut(slot)
            && claim.due_s > *mark
        {
            *mark = claim.due_s;
            self.set_skip(slot, false);
            self.set_fed(slot, present);
            return true;
        }
        false
    }

    fn set_enabled(&mut self, slot: u8, enabled: bool) {
        if let Some(Some(plan)) = self.table.get_mut(usize::from(slot))
            && plan.enabled != enabled
        {
            plan.enabled = enabled;
            self.revision = self.revision.wrapping_add(1);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlansStatus {
    Unopened,
    /// No record yet: no plans, writable.
    Empty,
    Loaded,
    /// A newer major: no plans, read-only.
    ReadOnly,
    /// A record of this store but none this build reads: no plans,
    /// read-only until the factory reset erases it.
    Corrupt,
    Io,
}

impl PlansStatus {
    /// The status's record word.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Unopened => "unopened",
            Self::Empty => "empty",
            Self::Loaded => "loaded",
            Self::ReadOnly => "read-only",
            Self::Corrupt => "corrupt",
            Self::Io => "io",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PlansStore<I> {
    units: DualSlot<I>,
    pub status: PlansStatus,
    /// The save count of the record loaded or last saved.
    pub sequence: u32,
    pub slot: c_uint,
    pub plans: Plans,
    /// The revision this boot's last client table produced once saved (0
    /// for none): how a client's write learns it landed, whatever the loop
    /// wrote after it. RAM only.
    pub replaced: u32,
}

impl<I> Default for PlansStore<I> {
    fn default() -> Self {
        Self {
            units: DualSlot::unbound(&LAYOUT),
            status: PlansStatus::Unopened,
            sequence: 0,
            slot: 0,
            plans: Plans::default(),
            replaced: 0,
        }
    }
}

fn encode(sequence: u32, plans: &Plans) -> [u8; RECORD_BYTES] {
    let mut p = LAYOUT.record();
    put_u32::<SEQUENCE_AT, _>(&mut p, sequence);
    put_u32::<REVISION_AT, _>(&mut p, plans.revision);

    for (slot, plan) in plans.table.iter().enumerate() {
        let at = PLANS_AT + slot * PLAN_BYTES;
        if let Some(plan) = plan {
            p[at] = PRESENT
                | if plan.enabled { ENABLED } else { 0 }
                | plan.call << CALL_SHIFT
                | if plans.skipped(slot) { SKIPPED } else { 0 }
                | if plans.fed & (1 << slot) != 0 { FED } else { 0 };
            p[at + 1..at + 3].copy_from_slice(&plan.minute.to_le_bytes());
            p[at + 3] = plan.portions;
            p[at + 4] = plan.days.0;
        }
    }
    for (slot, mark) in plans.marks.iter().enumerate() {
        let at = MARKS_AT + slot * 4;
        p[at..at + 4].copy_from_slice(&mark.to_le_bytes());
    }
    if let Some(claim) = plans.claim {
        p[CLAIM_AT] = PRESENT | if claim.disabled { DISABLED } else { 0 };
        p[CLAIM_AT + 1] = claim.plan;
        put_u32::<{ CLAIM_AT + 2 }, _>(&mut p, claim.due_s);
        put_u32::<{ CLAIM_AT + 6 }, _>(&mut p, claim.high_water);
    }
    seal(&mut p);
    p
}

/// The sequence and the plans of a record whose header and checksum hold.
/// A plan outside its limits, a flag this build does not know or a stray
/// byte is no record of ours.
fn decode(p: &[u8; RECORD_BYTES]) -> Option<(u32, Plans)> {
    let sequence = get_u32::<SEQUENCE_AT, _>(p);
    let mut plans = Plans {
        revision: get_u32::<REVISION_AT, _>(p),
        ..Plans::default()
    };

    for (slot, entry) in plans.table.iter_mut().enumerate() {
        let at = PLANS_AT + slot * PLAN_BYTES;
        let bytes = p.get(at..at + PLAN_BYTES)?;
        *entry = match bytes[0] {
            0 if bytes.iter().all(|&b| b == 0) => None,
            flags
                if flags & !(PRESENT | ENABLED | CALL_MASK | SKIPPED | FED) == 0
                    && flags & PRESENT != 0
                    && flags & (SKIPPED | FED) != SKIPPED | FED =>
            {
                if flags & SKIPPED != 0 {
                    plans.skips |= 1 << slot;
                }
                if flags & FED != 0 {
                    plans.fed |= 1 << slot;
                }
                let plan = Plan {
                    minute: u16::from_le_bytes([bytes[1], bytes[2]]),
                    portions: bytes[3],
                    days: Weekdays(bytes[4]),
                    enabled: flags & ENABLED != 0,
                    call: (flags & CALL_MASK) >> CALL_SHIFT,
                };
                if !plan.valid() {
                    return None;
                }
                Some(plan)
            }
            _ => return None,
        };
    }
    for (slot, mark) in plans.marks.iter_mut().enumerate() {
        let at = MARKS_AT + slot * 4;
        *mark = u32::from_le_bytes(p.get(at..at + 4)?.try_into().ok()?);
    }
    plans.claim = match p[CLAIM_AT] {
        0 if p[CLAIM_AT..CLAIM_END].iter().all(|&b| b == 0) => None,
        flags if flags & !(PRESENT | DISABLED) == 0 && flags & PRESENT != 0 => {
            let claim = Claim {
                plan: p[CLAIM_AT + 1],
                due_s: get_u32::<{ CLAIM_AT + 2 }, _>(p),
                high_water: get_u32::<{ CLAIM_AT + 6 }, _>(p),
                disabled: flags & DISABLED != 0,
            };
            if usize::from(claim.plan) >= PLANS {
                return None;
            }
            Some(claim)
        }
        _ => return None,
    };

    if sequence == 0 || p[CLAIM_END..CRC_AT].iter().any(|&b| b != 0) {
        return None;
    }
    Some((sequence, plans))
}

impl<I: SlotIo> PlansStore<I> {
    /// Bind to two exclusive erase units and load the newest valid record.
    pub fn open(&mut self, io: I) -> PlansStatus {
        *self = Self::default();
        // The sequence orders the records as a generation does, and rides
        // along in its own width.
        let decoded =
            |p: &_| decode(p).map(|(sequence, plans)| (u64::from(sequence), (sequence, plans)));
        self.status = match self.units.open_newest(io, decoded) {
            Newest::Io => PlansStatus::Io,
            Newest::Future => PlansStatus::ReadOnly,
            Newest::Loaded {
                slot,
                value: (sequence, plans),
                ..
            } => {
                self.slot = slot;
                self.sequence = sequence;
                self.plans = plans;
                PlansStatus::Loaded
            }
            Newest::Nothing { unwritten: true } => PlansStatus::Empty,
            Newest::Nothing { unwritten: false } => PlansStatus::Corrupt,
        };
        self.status
    }

    pub fn writable(&self) -> bool {
        matches!(self.status, PlansStatus::Empty | PlansStatus::Loaded)
    }

    pub fn view(&self, armed: bool) -> PlansView {
        PlansView {
            replaced: self.replaced,
            ..self.plans.view(armed && self.writable())
        }
    }

    /// Apply `change` and save the result: `None` when the change is
    /// dropped (nothing written), else whether the save held. The writer
    /// and the simulator both apply changes through this.
    pub fn apply(&mut self, change: &PlansChange) -> Option<bool> {
        let plans = self.plans.changed(change)?;
        let saved = self.save(&plans);
        if saved && matches!(change, PlansChange::Replace { .. }) {
            self.replaced = plans.revision;
        }
        Some(saved)
    }

    /// The history event of a client's `change` that [`Self::apply`] just
    /// saved, naming `by`: a new table's `Plans` (with the revision it
    /// produced), a skip's `Skipped` or an undo's `Unskipped`. The
    /// schedule's own changes have none.
    pub fn event(&self, change: &PlansChange, by: Source) -> Option<EventBody> {
        Some(match *change {
            PlansChange::Replace { .. } => EventBody::Plans {
                generation: self.plans.revision,
                by,
            },
            PlansChange::Skip { plan, due_s, .. } => EventBody::Skipped {
                plan,
                due: due_s,
                by,
            },
            PlansChange::Unskip { plan, due_s, .. } => EventBody::Unskipped {
                plan,
                due: due_s,
                by,
            },
            PlansChange::Claim { .. }
            | PlansChange::Release { .. }
            | PlansChange::Settle { .. }
            | PlansChange::Pass { .. } => return None,
        })
    }

    /// Store `plans` to the other slot by the torn-write rule. A failure
    /// keeps the previous plans in memory, and the store refuses every
    /// later save this boot.
    pub fn save(&mut self, plans: &Plans) -> bool {
        if !self.writable() || !self.units.writable() {
            return false;
        }
        // The record numbers its saves in 32 bits: the last one refuses
        // before the shared save counts past it.
        let Some(sequence) = self.sequence.checked_add(1) else {
            self.status = PlansStatus::Io;
            return false;
        };
        let saved = self
            .units
            .save_newest(u64::from(self.sequence), self.slot, |_| {
                encode(sequence, plans)
            });
        let Some((slot, _)) = saved else {
            self.status = PlansStatus::Io;
            return false;
        };
        self.slot = slot;
        self.sequence = sequence;
        self.plans = *plans;
        true
    }
}
