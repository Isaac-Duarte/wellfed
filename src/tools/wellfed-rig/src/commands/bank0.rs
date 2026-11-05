//! What finishes a bank-0 write. Bank 1 gives way for an update only to a
//! bank 0 that booted and recorded its own image in the update store, which
//! a UART write never does: until then bank 1 refuses every offer
//! (`Fallback`, reason `unrecorded` or `changed`). So `flash-execute --bank
//! 0` goes on, after its verified write, through the same reviewed steps a
//! person would run, each prepared afresh and checked to pin exactly the
//! image it is meant to:
//!
//! 1. give way to the image just written (`give-way` with `--fallback` the
//!    target review), which boots bank 0;
//! 2. bank 0's `PLOTA1 bank0 bytes=… recorded=…` in that boot: it recorded
//!    itself (a means, not the proof);
//! 3. restore bank 1's marker against its source review (`--restore-marker`),
//!    which boots bank 1;
//! 4. bank 1's `PLOTA1 bank0 check=match` in that boot: the proof, and the
//!    only end that counts as done.
//!
//! The bank-1 review is checked against bank 1 at prepare, and bound there,
//! before anything is written. A plan is executed only once the job has
//! checked it names that review's image; the updater checks the rest, as for
//! any plan. Any step that fails, an error included, stops the job without a
//! retry, and it says where the feeder stands and the commands that go on.
//! Where a write stopped part way, the flash client's own read-only
//! inspections say which way on applies, and the job prints only that one;
//! when they cannot tell, it prints each with its condition.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use wellfed_core::ota::SECTOR;
use wellfed_core::ota::install::{Bank0Check, Bank0Error, Blocked};
use wellfed_core::record::{Event, Record, Update};
use wellfed_formats::bench::banks::{Content, Source};
use wellfed_formats::bench::{Paths, VERIFICATION};
use wellfed_formats::flash_plan::GiveWayPlan;
use wellfed_formats::require;
use wellfed_image::bank::{self, Layout};
use wellfed_image::review::Frozen;
use wellfed_image::root::Root;

use super::{
    Bench, CommandsError, ExecuteArgs, GiveWayArgs, GiveWayExecuteArgs, Prepared, UpdateArgs, argv,
    execute_line, give_way_name, update_line,
};
use crate::{Error, Result};

/// Bank 0's boot: it records itself within seconds, once the settings
/// writer armed the flash and its image is hashed, 32 KiB a step.
pub(crate) const BANK0_BOOT_SECONDS: f64 = 60.0;
/// Bank 1's boot: it checks bank 0 only after its healthy window (30 s,
/// after its Wi-Fi join), once the flash hooks are armed, then hashes it.
/// Under the flash client's 180 s cap on one capture.
pub(crate) const BANK1_BOOT_SECONDS: f64 = 170.0;

/// Where a bank-0 prepare binds the bank-1 review it checked.
pub(crate) fn binding(root: &Path, name: &str) -> PathBuf {
    root.join(VERIFICATION)
        .join(format!("{name}-bank1-source.json"))
}

/// The bank-1 review a bank-0 prepare checked against bank 1: its execute
/// must name the same review, unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Bank1Binding {
    review: PathBuf,
    /// SHA-256 of its `review.json`, as the flash client binds reviews.
    review_sha256: String,
}

/// What one boot capture says about bank 0: the last of each line.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Seen {
    /// Bank 0's own record: `Ok` with its length, or why it could not.
    pub recorded: Option<core::result::Result<u32, Bank0Error>>,
    /// Bank 1's boot check.
    pub check: Option<Bank0Check>,
}

/// Read a boot capture's records about bank 0. A capture that cannot be
/// read says nothing, which the job treats as not seen.
pub(crate) fn seen(boot: &Path) -> Seen {
    let rx = fs::read(boot.join("serial-rx.bin")).unwrap_or_default();
    let text = String::from_utf8_lossy(&rx);
    let mut seen = Seen::default();
    for line in text.lines() {
        for event in Record::all(line).filter_map(|record| record.event()) {
            match event {
                Event::Update(Update::Bank0Recorded { bytes, .. }) => {
                    seen.recorded = Some(Ok(bytes));
                }
                Event::Update(Update::Bank0Unrecorded { error }) => {
                    seen.recorded = Some(Err(error));
                }
                Event::Update(Update::Bank0Check { check }) => seen.check = Some(check),
                _ => {}
            }
        }
    }
    seen
}

/// How a marker restore runs: fresh from the given-way state, or finishing
/// one that stopped after it erased or staged the marker sector. The flash
/// client takes `--restore-marker` only for the first and `--interrupted`
/// only without it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Resume {
    Fresh,
    Interrupted,
}

/// A marker restore of bank 1 against its review `bank1`, as `resume` runs
/// it; the job's own restore and every one it prints come from here.
pub(crate) fn restore_args(
    name: String,
    bank1: &Path,
    resume: Resume,
    attended_motion: bool,
) -> UpdateArgs {
    UpdateArgs {
        name,
        source: Some(bank1.to_path_buf()),
        bank: 1,
        original_read: None,
        target: bank1.to_path_buf(),
        interrupted: resume == Resume::Interrupted,
        restore_marker: resume == Resume::Fresh,
        attended_motion,
        bank1_source: None,
    }
}

/// What bank 1 holds after a write that stopped part way, as the flash
/// client's read-only inspections judge it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bank1 {
    /// Its marker is intact and bank 0 is the review: a give-way applies.
    Intact,
    /// Exactly its review with the first marker byte cleared.
    GivenWay,
    /// The marker sector erased or staged by a stopped restore.
    Staged,
    /// Exactly its review, marker and all.
    Restored,
    /// None of these, or the inspections could not run.
    Unknown,
}

/// The read-only inspections that tell [`Bank1`] states apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Probe {
    GiveWay,
    Restore,
    Interrupted,
    Unchanged,
}

impl Probe {
    const fn word(self) -> &'static str {
        match self {
            Self::GiveWay => "giveway",
            Self::Restore => "restore",
            Self::Interrupted => "interrupted",
            Self::Unchanged => "unchanged",
        }
    }

    const fn passed(self) -> Bank1 {
        match self {
            Self::GiveWay => Bank1::Intact,
            Self::Restore => Bank1::GivenWay,
            Self::Interrupted => Bank1::Staged,
            Self::Unchanged => Bank1::Restored,
        }
    }
}

/// Where the finish stands when it stops: what the feeder holds, and so
/// what goes on from there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum At {
    /// Bank 0 written, unrecorded; bank 1 unchanged.
    Written,
    /// The give-way's frame may or may not have landed.
    GiveWayUnknown,
    /// Bank 1 given way; bank 0's record not seen.
    GivenWay,
    /// Bank 0 recorded itself; bank 1 given way.
    Recorded,
    /// A stopped restore left the marker sector erased or staged.
    RestoreStaged,
    /// The restore may have stopped anywhere.
    RestoreUnknown,
    /// Bank 1's marker is back; its check not seen.
    Restored,
}

/// The commands that go on from a stop, under session names the job has
/// not used.
pub(crate) struct Next<'a> {
    pub name: &'a str,
    pub target: &'a Path,
    pub bank1: &'a Path,
    pub attended_motion: bool,
}

impl Next<'_> {
    fn give_way(&self) -> Vec<String> {
        let (name, target) = (self.name, self.target.display());
        vec![
            format!("wellfed-bench give-way-prepare {name}-giveway2 --fallback {target}"),
            format!(
                "wellfed-bench give-way-execute {name}-giveway2 --fallback {target} \
                 --expected-plan-sha256 <the plan's> --boot-seconds {BANK0_BOOT_SECONDS}"
            ),
            "in that boot's capture: PLOTA1 bank0 bytes=… recorded=…".to_owned(),
        ]
    }

    fn boot0(&self) -> Vec<String> {
        vec![
            format!(
                "wellfed-bench boot {}-bank0 --seconds {BANK0_BOOT_SECONDS}",
                self.name
            ),
            "in it: PLOTA1 bank0 bytes=… recorded=…".to_owned(),
        ]
    }

    pub(crate) fn restore(&self, resume: Resume) -> Vec<String> {
        let args = restore_args(
            format!("{}-restore2", self.name),
            self.bank1,
            resume,
            self.attended_motion,
        );
        vec![
            update_line("flash-prepare", &args),
            format!(
                "{} --boot-seconds {BANK1_BOOT_SECONDS}",
                execute_line(&args, "<the plan's>")
            ),
            "in that boot's capture: PLOTA1 bank0 check=match (only this finishes it)".to_owned(),
        ]
    }

    fn boot1(&self) -> Vec<String> {
        vec![
            format!(
                "wellfed-bench boot {}-bank1 --seconds {BANK1_BOOT_SECONDS}",
                self.name
            ),
            "in it: PLOTA1 bank0 check=match (only this finishes it)".to_owned(),
        ]
    }

    fn capture(&self) -> Vec<String> {
        vec![
            format!(
                "wellfed-bench capture {}-check --seconds {BANK1_BOOT_SECONDS}",
                self.name
            ),
            "in it: PLOTA1 bank0 check=match (only this finishes it)".to_owned(),
        ]
    }

    /// Bank 0 itself is not an image bank 1 can hand over to: write it again.
    fn rewrite(&self) -> Vec<String> {
        let (name, target, bank1) = (self.name, self.target.display(), self.bank1.display());
        let motion = if self.attended_motion {
            " --attended-motion"
        } else {
            ""
        };
        vec![
            format!("wellfed-bench dump {name}-reread"),
            format!(
                "wellfed-bench flash-prepare {name}-again --bank 0 --original-read <that full read> \
                 --target {target} --bank1-source {bank1}{motion}"
            ),
        ]
    }

    /// Each step under its condition.
    fn when(condition: &str, steps: Vec<String>) -> Vec<String> {
        steps
            .into_iter()
            .map(|step| format!("if {condition}: {step}"))
            .collect()
    }

    /// Where the feeder stands at `at`, and the commands from there.
    pub(crate) fn at(&self, at: At) -> (&'static str, Vec<String>) {
        match at {
            At::Written => (
                "bank 0 holds the new image, unrecorded; bank 1 is unchanged and boots, and \
                 refuses every update",
                [self.give_way(), self.restore(Resume::Fresh)].concat(),
            ),
            At::GiveWayUnknown => (
                "the give-way stopped, and bank 1's first marker byte may or may not be \
                 cleared; the feeder is powered and not booted",
                [
                    Self::when(
                        "give-way-prepare accepts bank 1 (its marker intact)",
                        [self.give_way(), self.restore(Resume::Fresh)].concat(),
                    ),
                    Self::when(
                        "it refuses NothingToGiveWay (bank 1's first byte cleared)",
                        [self.boot0(), self.restore(Resume::Fresh)].concat(),
                    ),
                ]
                .concat(),
            ),
            At::GivenWay => (
                "bank 1 has given way, so bank 0 boots, but its record was not seen; updates \
                 stay refused",
                [self.boot0(), self.restore(Resume::Fresh)].concat(),
            ),
            At::Recorded => (
                "bank 0 recorded itself and runs; bank 1 holds its source with the first \
                 marker byte cleared",
                self.restore(Resume::Fresh),
            ),
            At::RestoreStaged => (
                "the marker restore stopped with bank 1's marker sector erased or staged; \
                 bank 0 recorded itself, and the feeder is powered and not booted",
                self.restore(Resume::Interrupted),
            ),
            At::RestoreUnknown => (
                "the marker restore stopped, and how far it got is not known; bank 0 \
                 recorded itself, and the feeder is powered and not booted",
                [
                    Self::when(
                        "flash-prepare --restore-marker accepts bank 1 (nothing erased yet)",
                        self.restore(Resume::Fresh),
                    ),
                    Self::when(
                        "flash-prepare --interrupted accepts it (the marker sector erased or \
                         staged)",
                        self.restore(Resume::Interrupted),
                    ),
                    Self::when(
                        "both refuse because bank 1 is already its review, marker and all",
                        self.boot1(),
                    ),
                ]
                .concat(),
            ),
            At::Restored => (
                "bank 1's marker is back, but its boot check was not seen",
                self.boot1(),
            ),
        }
    }

    /// After a stopped write, from what the inspections found.
    fn after_probe(&self, bank1: Bank1, unknown: At) -> At {
        match bank1 {
            Bank1::Intact => At::Written,
            Bank1::GivenWay if unknown == At::GiveWayUnknown => At::GivenWay,
            Bank1::GivenWay => At::Recorded,
            Bank1::Staged => At::RestoreStaged,
            Bank1::Restored => At::Restored,
            Bank1::Unknown => unknown,
        }
    }
}

/// The steps after an older `wellfed-flash` reported a bank-0 write as a
/// plain success: rebuild it, then finish from the give-way.
pub(crate) fn skew_steps(update: &UpdateArgs) -> Vec<String> {
    let mut steps = vec!["cargo build --release -p wellfed-flash".to_owned()];
    if let Some(bank1) = &update.bank1_source {
        let next = Next {
            name: &update.name,
            target: &update.target,
            bank1,
            attended_motion: update.attended_motion,
        };
        steps.extend(next.at(At::Written).1);
    }
    steps
}

impl Bench {
    /// At a bank-0 prepare: bank 1 must hold `bank1`'s image (its first
    /// marker byte aside, which a give-way clears), as the prepare's
    /// inspection read bank 1's first sector. The review is then bound for
    /// the execute.
    pub(super) fn bind_bank1(&mut self, name: &str, bank1: &Path) -> Result<()> {
        let frozen = self.frozen(bank1, &bank::BANK1)?;
        let payload =
            bank::payload(&frozen.image, &bank::BANK1).map_err(|error| CommandsError::Review {
                review: self.rel(bank1),
                error,
            })?;
        let path = Paths::new(&self.root, name)
            .inspect
            .join("other-bank-01.bin");
        let sector = fs::read(&path).map_err(|_| CommandsError::Bank1SectorMissing {
            sector: self.rel(&path),
        })?;
        require(
            sector.len() == SECTOR && payload.len() >= SECTOR && sector[1..] == payload[1..SECTOR],
            || CommandsError::Bank1SourceDiffers {
                review: self.rel(bank1),
            },
        )?;

        let bound = Bank1Binding {
            review: bank1.to_path_buf(),
            review_sha256: frozen.review_sha256,
        };
        fs::write(
            binding(&self.root, name),
            serde_json::to_vec_pretty(&bound)?,
        )?;
        say!(
            "Bank 1 holds {} (review.json {}), its first byte aside; bound for the execute",
            self.rel(bank1),
            bound.review_sha256
        );
        Ok(())
    }

    /// At a bank-0 execute, before the write: the bank-1 review is the one
    /// its prepare checked, unchanged.
    pub(super) fn require_bank1_bound(&self, name: &str, bank1: &Path) -> Result<()> {
        let path = binding(&self.root, name);
        let unbound = || CommandsError::Bank1SourceUnbound {
            binding: self.rel(&path),
            review: self.rel(bank1),
        };
        let bound: Bank1Binding = fs::read(&path)
            .ok()
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .ok_or_else(unbound)?;
        let frozen = self.frozen(bank1, &bank::BANK1)?;
        require(
            bound.review == bank1 && bound.review_sha256 == frozen.review_sha256,
            unbound,
        )?;
        Ok(())
    }

    /// After a verified bank-0 write: give way, boot bank 0, restore bank
    /// 1's marker, boot bank 1, and require its `check=match`. Every stop,
    /// an error's included, says where the feeder stands and what goes on;
    /// an error is still returned.
    pub(super) fn finish_bank0(&mut self, args: &ExecuteArgs) -> Result<u8> {
        let update = args.update.clone();
        let bank1 = update
            .bank1_source
            .clone()
            .ok_or(CommandsError::Bank1SourceRequired)?;
        let next = Next {
            name: &update.name,
            target: &update.target,
            bank1: &bank1,
            attended_motion: update.attended_motion,
        };
        say!(
            "Bank 0 is written and verified but has not recorded itself, so bank 1 refuses every \
             update. Finishing: give way, boot bank 0, restore bank 1's marker, boot bank 1."
        );

        let mut at = At::Written;
        let mut writes = self.written.len();
        match self.bank0_steps(&update, &bank1, &next, &mut at, &mut writes) {
            Ok(code) => Ok(code),
            Err(error) => {
                let at = self.at_error(at, writes);
                let (now, steps) = next.at(at);
                self.unfinished(&format!("{now} (stopped on: {error})"), &steps);
                Err(error)
            }
        }
    }

    /// Where an error left the finish: a write that never started leaves the
    /// state before it, one that landed the state after it, and one that
    /// started and stopped an unknown one.
    fn at_error(&self, at: At, writes: usize) -> At {
        let landed = self.written.get(writes..).and_then(<[_]>::last);
        match (at, landed) {
            (_, None) => at,
            (At::GiveWayUnknown, Some(w)) if w.content == Content::Invalid => At::GivenWay,
            (At::RestoreUnknown, Some(w))
                if w.content != Content::Unknown && w.source == Source::FlashExecute =>
            {
                At::Restored
            }
            (_, Some(_)) => at,
        }
    }

    fn bank0_steps(
        &mut self,
        update: &UpdateArgs,
        bank1: &Path,
        next: &Next<'_>,
        at: &mut At,
        writes: &mut usize,
    ) -> Result<u8> {
        // 1. Give way to the image just written.
        let give_way = GiveWayArgs {
            name: give_way_name(&update.name),
            fallback: update.target.clone(),
        };
        let Some(sha) = self.give_way_plan(&give_way)? else {
            return Ok(self.stopped_at(next, At::Written));
        };
        let plan: GiveWayPlan = self.operation(&give_way.name)?;
        let image = self.frozen(&update.target, &bank::BANK0)?.review.ota_sha256;
        if !plan
            .fallback_ota_sha256
            .as_deref()
            .is_some_and(|sha| sha.eq_ignore_ascii_case(&image))
        {
            say!(
                "The give-way plan pins bank 0 as {:?}, not the image just written ({image}); \
                 not executed",
                plan.fallback_ota_sha256
            );
            return Ok(self.stopped_at(next, At::Written));
        }
        *at = At::GiveWayUnknown;
        *writes = self.written.len();
        let gave_way = self.give_way_execute(&GiveWayExecuteArgs {
            give_way: give_way.clone(),
            expected_plan_sha256: sha,
            boot_seconds: BANK0_BOOT_SECONDS,
        })?;
        if gave_way != 0 {
            let probes = [Probe::GiveWay, Probe::Restore];
            let found = self.probe(update, bank1, &probes);
            return Ok(self.stopped_at(next, next.after_probe(found, At::GiveWayUnknown)));
        }

        // 2. Bank 0's boot: it records itself.
        *at = At::GivenWay;
        match seen(&Paths::new(&self.root, &give_way.name).boot).recorded {
            Some(Ok(bytes)) => say!("Bank 0 recorded itself ({bytes} bytes)"),
            Some(Err(error)) => {
                let now = format!(
                    "bank 1 gave way and bank 0 runs, but it could not record itself \
                     (error={error}); updates stay refused"
                );
                let steps = match error {
                    Bank0Error::Store => [next.boot0(), next.restore(Resume::Fresh)].concat(),
                    Bank0Error::Extent | Bank0Error::Read => next.rewrite(),
                };
                return Ok(self.unfinished(&now, &steps));
            }
            None => return Ok(self.stopped_at(next, At::GivenWay)),
        }

        // 3. Restore bank 1's marker against its source review.
        *at = At::Recorded;
        let restore = restore_args(
            format!("{}-restore", update.name),
            bank1,
            Resume::Fresh,
            update.attended_motion,
        );
        let Some((sha, plan)) = self.update_plan(&restore)? else {
            return Ok(self.stopped_at(next, At::Recorded));
        };
        let image = self.frozen(bank1, &bank::BANK1)?.review.ota_sha256;
        let pins = |sha: &str| sha.eq_ignore_ascii_case(&image);
        let source = plan.source_ota_sha256.as_deref().is_some_and(pins);
        if !(plan.restore_marker && plan.bank == 1 && source && pins(&plan.target_ota_sha256)) {
            say!(
                "The restore plan is not bank 1's marker restore of {image} (bank {}, \
                 restore_marker {}, source {:?}, target {}); not executed",
                plan.bank,
                plan.restore_marker,
                plan.source_ota_sha256,
                plan.target_ota_sha256
            );
            return Ok(self.stopped_at(next, At::Recorded));
        }
        *at = At::RestoreUnknown;
        *writes = self.written.len();
        let restored = self.flash_execute(&ExecuteArgs {
            update: restore,
            expected_plan_sha256: sha,
            boot_seconds: BANK1_BOOT_SECONDS,
        })?;
        if restored != 0 {
            let probes = [Probe::Restore, Probe::Interrupted, Probe::Unchanged];
            let found = self.probe(update, bank1, &probes);
            return Ok(self.stopped_at(next, next.after_probe(found, At::RestoreUnknown)));
        }

        // 4. Bank 1's boot check: the proof.
        *at = At::Restored;
        let restore_name = format!("{}-restore", update.name);
        match seen(&Paths::new(&self.root, &restore_name).boot).check {
            Some(Bank0Check::Match) => {
                say!(
                    "Bank 1 runs and reports PLOTA1 bank0 check=match: the bank-0 write is \
                     finished, and updates can install"
                );
                Ok(0)
            }
            Some(Bank0Check::Blocked(reason)) => {
                let now = format!(
                    "bank 1 runs, and its boot check says bank 0 is blocked (reason={reason}): \
                     updates stay refused"
                );
                let steps = match reason {
                    Blocked::Unrecorded | Blocked::Changed | Blocked::Key => {
                        [next.give_way(), next.restore(Resume::Fresh)].concat()
                    }
                    Blocked::Marker | Blocked::Extent | Blocked::Read => next.rewrite(),
                };
                Ok(self.unfinished(&now, &steps))
            }
            Some(Bank0Check::Unchecked) | None => {
                let now = format!(
                    "bank 1 runs, but reported no bank-0 check within {BANK1_BOOT_SECONDS} s"
                );
                Ok(self.unfinished(&now, &next.capture()))
            }
        }
    }

    /// After a write that stopped: which state bank 1 is in, by the first of
    /// `probes` whose read-only inspection passes. Nothing is written; an
    /// inspection that fails or cannot run says only that it does not apply,
    /// and none passing is [`Bank1::Unknown`]. A cancel probes nothing.
    fn probe(&mut self, update: &UpdateArgs, bank1: &Path, probes: &[Probe]) -> Bank1 {
        if self.cancel.is_cancelled() {
            return Bank1::Unknown;
        }
        match self.try_probe(update, bank1, probes) {
            Ok(found) => found,
            Err(error) => {
                say!("Could not inspect bank 1 after the stop: {error}");
                Bank1::Unknown
            }
        }
    }

    fn try_probe(&mut self, update: &UpdateArgs, bank1: &Path, probes: &[Probe]) -> Result<Bank1> {
        let name = format!("{}-probe", update.name);
        let p = Paths::new(&self.root, &name);
        for path in [&p.loader, &p.bounded] {
            self.fresh(path)?;
        }
        self.loader(&p.loader)?;
        let read = argv(
            &["read", "small"],
            &[&self.read_args(), &self.out(&p.bounded)],
        );
        if self.run(&read)? != 0 {
            return Ok(Bank1::Unknown);
        }

        for &probe in probes {
            let output = self
                .root
                .join(VERIFICATION)
                .join(format!("{name}-{}-inspect", probe.word()));
            self.fresh(&output)?;
            let words = match probe {
                Probe::GiveWay => {
                    let give_way = GiveWayArgs {
                        name: name.clone(),
                        fallback: update.target.clone(),
                    };
                    argv(
                        &["give-way", "inspect"],
                        &[&self.give_way_args(&give_way, &p), &self.out(&output)],
                    )
                }
                Probe::Restore | Probe::Interrupted | Probe::Unchanged => {
                    let mut args = restore_args(
                        name.clone(),
                        bank1,
                        if probe == Probe::Interrupted {
                            Resume::Interrupted
                        } else {
                            Resume::Fresh
                        },
                        update.attended_motion,
                    );
                    args.restore_marker = probe == Probe::Restore;
                    argv(
                        &["update", "inspect"],
                        &[&self.update_args(&args, &p)?, &self.out(&output)],
                    )
                }
            };
            if self.run(&words)? == 0 {
                say!("Bank 1 after the stop: {:?}", probe.passed());
                return Ok(probe.passed());
            }
        }
        Ok(Bank1::Unknown)
    }

    fn stopped_at(&mut self, next: &Next<'_>, at: At) -> u8 {
        let (now, steps) = next.at(at);
        self.unfinished(now, &steps)
    }

    /// Stop without a retry: where the feeder stands, and what goes on.
    pub(super) fn unfinished(&mut self, now: &str, steps: &[String]) -> u8 {
        say!("The bank-0 write is not finished: {now}. No retry. Next, in order:");
        for (n, step) in steps.iter().enumerate() {
            say!("  {}. {step}", n + 1);
        }
        self.status.next(steps.join("; then "));
        1
    }

    /// The operation of the plan a session prepared.
    fn operation<P: serde::de::DeserializeOwned>(&self, session: &str) -> Result<P> {
        let (raw, _) = self.prepared(&Paths::new(&self.root, session))?;
        Ok(serde_json::from_slice::<Prepared<P>>(&raw)?.operation_plan)
    }

    /// A frozen review, verified as the flash client verifies it.
    fn frozen(&self, review: &Path, layout: &Layout) -> Result<Frozen> {
        let failed = |error| -> Error {
            CommandsError::Review {
                review: self.rel(review),
                error,
            }
            .into()
        };
        let root = Root::new(&self.root).map_err(failed)?;
        wellfed_image::review::frozen(&root, &self.root.join(review), layout).map_err(failed)
    }
}
