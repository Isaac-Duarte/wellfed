//! Every interleaving of two producers and one consumer over one real
//! [`Handoff`], step by step at its atomics: a producer's claim, its write
//! and its queueing are three steps, and the consumer's take, read, and
//! finish or drop are each one. The run is single-threaded and replays each
//! schedule from the start, so it checks the hand-off's state machine
//! exhaustively rather than sampling thread timing.
//!
//! loom would model the memory orderings too, but only for code built on
//! its own atomics and cells, which are not `const`: the hand-off's statics
//! are, so a loom model would test a copy behind `cfg(loom)`, not this one.
//! A producer suspends between its steps inside its `fill`, which is where
//! the code under test runs the other actors.
//!
//! Shadow state checks exclusivity at every step: no producer writes while
//! another does or while a guard is held, the consumer takes only what a
//! producer finished queueing, and a value is read and finished exactly
//! once, in the order it was queued, or is still waiting at the end.
//!
//! [`Published`](wellfed_hal_common::handoff::Published) has no steps to
//! interleave: its publish and its read each copy the whole view with
//! interrupts masked on the one core, and its loaded flag only ever turns
//! on, so a read finds nothing or a whole view (`tests/handoff.rs`).

use std::cell::{Cell, RefCell};

use wellfed_hal_common::handoff::{Handoff, Taken};

/// Claims each producer tries before it gives up.
const ATTEMPTS: u8 = 2;
/// Takes the consumer tries.
const TAKES: u8 = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// Not holding the slot; `attempts` claims left.
    Idle,
    /// Claimed, not written yet.
    Claimed,
    /// Written, not queued yet.
    Written,
    /// Queued its value, or used up its attempts.
    Done,
}

#[derive(Clone, Copy, Debug)]
enum Step {
    Producer(usize),
    Take,
    Read,
    Finish,
    Drop,
}

/// The schedule as a sequence of choices, replayed from the start each run
/// and advanced like an odometer, so every schedule runs once.
#[derive(Default)]
struct Chooser {
    /// Each decision's choice and how many options it had.
    path: Vec<(usize, usize)>,
    at: usize,
}

impl Chooser {
    fn choose(&mut self, options: usize) -> usize {
        if let Some(&(choice, count)) = self.path.get(self.at) {
            assert_eq!(count, options, "a replayed schedule diverged");
            self.at += 1;
            return choice;
        }
        self.path.push((0, options));
        self.at += 1;
        0
    }

    /// The next schedule; false once every one has run.
    fn advance(&mut self) -> bool {
        self.at = 0;
        while let Some((choice, count)) = self.path.pop() {
            if choice + 1 < count {
                self.path.push((choice + 1, count));
                return true;
            }
        }
        false
    }
}

struct Run<'s> {
    slot: &'s Handoff<u32>,
    chooser: RefCell<Chooser>,
    phases: [Cell<Phase>; 2],
    attempts: [Cell<u8>; 2],
    takes: Cell<u8>,
    guard: RefCell<Option<Taken<'s, u32>>>,
    read: Cell<bool>,
    /// The producer between its claim and its queueing.
    writing: Cell<Option<usize>>,
    /// Values queued, in order.
    queued: RefCell<Vec<u32>>,
    /// Values finished, in order.
    finished: RefCell<Vec<u32>>,
}

impl<'s> Run<'s> {
    fn new(slot: &'s Handoff<u32>, chooser: Chooser) -> Self {
        Self {
            slot,
            chooser: RefCell::new(chooser),
            phases: [Cell::new(Phase::Idle), Cell::new(Phase::Idle)],
            attempts: [Cell::new(ATTEMPTS), Cell::new(ATTEMPTS)],
            takes: Cell::new(TAKES),
            guard: RefCell::new(None),
            read: Cell::new(false),
            writing: Cell::new(None),
            queued: RefCell::new(Vec::new()),
            finished: RefCell::new(Vec::new()),
        }
    }

    fn options(&self) -> Vec<Step> {
        let mut options = Vec::new();
        for (producer, phase) in self.phases.iter().enumerate() {
            if phase.get() != Phase::Done {
                options.push(Step::Producer(producer));
            }
        }
        let held = self.guard.borrow().is_some();
        if !held && self.takes.get() > 0 {
            options.push(Step::Take);
        }
        if held && !self.read.get() {
            options.push(Step::Read);
        }
        if held && self.read.get() {
            options.extend([Step::Finish, Step::Drop]);
        }
        options
    }

    /// Run steps until `own`, a producer inside its `fill`, is chosen for
    /// its next step; at the top level (`None`), until no step is left.
    fn until(&self, own: Option<usize>) {
        loop {
            let options = self.options();
            if options.is_empty() {
                assert_eq!(own, None, "a filling producer always has a step");
                return;
            }
            let choice = self.chooser.borrow_mut().choose(options.len());
            match options[choice] {
                Step::Producer(producer) if self.phases[producer].get() == Phase::Idle => {
                    self.claim(producer);
                }
                Step::Producer(producer) => {
                    // Only the innermost fill can take its next step: a
                    // second claim would have failed the check in `claim`.
                    assert_eq!(Some(producer), own, "two producers inside a fill");
                    return;
                }
                Step::Take => self.take(),
                Step::Read => self.read(),
                Step::Finish => self.release(true),
                Step::Drop => self.release(false),
            }
        }
    }

    fn claim(&self, producer: usize) {
        let attempt = ATTEMPTS - self.attempts[producer].get();
        let value = u32::try_from(producer * 10).unwrap() + u32::from(attempt) + 1;
        let queued = self.slot.offer(|slot| {
            assert_eq!(self.writing.get(), None, "a claim while another writes");
            assert!(
                self.guard.borrow().is_none(),
                "a claim while a guard is held"
            );
            self.writing.set(Some(producer));
            self.phases[producer].set(Phase::Claimed);

            self.until(Some(producer));
            *slot = value;
            self.phases[producer].set(Phase::Written);

            self.until(Some(producer));
            self.writing.set(None);
        });

        if queued {
            self.queued.borrow_mut().push(value);
            self.phases[producer].set(Phase::Done);
            return;
        }
        let left = self.attempts[producer].get() - 1;
        self.attempts[producer].set(left);
        if left == 0 {
            self.phases[producer].set(Phase::Done);
        }
    }

    fn take(&self) {
        self.takes.set(self.takes.get() - 1);
        let Some(taken) = self.slot.take() else {
            return;
        };
        assert_eq!(self.writing.get(), None, "a take while a producer writes");
        *self.guard.borrow_mut() = Some(taken);
        self.read.set(false);
    }

    fn read(&self) {
        assert_eq!(self.writing.get(), None, "a read while a producer writes");
        let guard = self.guard.borrow();
        let value = *guard.as_ref().expect("read only under a guard").get();
        let finished = self.finished.borrow().len();
        assert_eq!(
            Some(&value),
            self.queued.borrow().get(finished),
            "the oldest value not finished"
        );
        self.read.set(true);
    }

    fn release(&self, finish: bool) {
        let taken = self.guard.borrow_mut().take().expect("a guard is held");
        let value = *taken.get();
        if finish {
            taken.finish();
            self.finished.borrow_mut().push(value);
        } else {
            drop(taken);
        }
    }

    /// Every value queued was finished in order, but for at most the last,
    /// which still waits for the next take.
    fn check_end(&self) {
        assert!(self.guard.borrow().is_none());
        let queued = self.queued.borrow();
        let finished = self.finished.borrow();
        assert_eq!(
            &queued[..finished.len()],
            &finished[..],
            "finished in order"
        );
        match &queued[finished.len()..] {
            [] => assert!(!self.slot.waiting(), "nothing left, nothing waits"),
            [last] => {
                let taken = self.slot.take().expect("an unfinished value waits");
                assert_eq!(*taken.get(), *last);
            }
            more => panic!("two values queued at once: {more:?}"),
        }
    }
}

#[test]
fn every_interleaving_of_two_producers_and_one_consumer_keeps_the_slot_exclusive() {
    let mut chooser = Chooser::default();
    let mut schedules = 0_u32;
    let mut both_finished = 0_u32;

    loop {
        let slot = Handoff::new(0_u32);
        let run = Run::new(&slot, chooser);
        run.until(None);
        run.check_end();
        if run.finished.borrow().len() == 2 {
            both_finished += 1;
        }
        schedules += 1;

        chooser = run.chooser.into_inner();
        if !chooser.advance() {
            break;
        }
    }

    // Not vacuous: many schedules ran, and some carried both values through.
    assert!(schedules > 1_000, "{schedules} schedules");
    assert!(both_finished > 0);
}
