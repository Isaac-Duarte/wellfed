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
//! A plan is executed only once the job has checked it names that review's
//! image; the updater checks the rest, as for any plan. Any step that fails
//! stops the job without a retry, and it says where the feeder stands and
//! the commands that go on from there.

use std::fs;
use std::path::{Path, PathBuf};

use wellfed_core::ota::install::{Bank0Check, Bank0Error, Blocked};
use wellfed_core::record::{Event, Record, Update};
use wellfed_formats::bench::Paths;
use wellfed_formats::flash_plan::GiveWayPlan;
use wellfed_formats::review::ImageReview;

use super::{
    Bench, CommandsError, ExecuteArgs, GiveWayArgs, GiveWayExecuteArgs, Prepared, UpdateArgs,
    give_way_name,
};
use crate::Result;

/// Bank 0's boot: it records itself within seconds, once the settings
/// writer armed the flash and its image is hashed, 32 KiB a step.
pub(crate) const BANK0_BOOT_SECONDS: f64 = 60.0;
/// Bank 1's boot: it checks bank 0 only once its trial has confirmed
/// (30 s healthy, after its Wi-Fi join), then hashes it. Under the flash
/// client's 180 s cap on one capture.
pub(crate) const BANK1_BOOT_SECONDS: f64 = 170.0;

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

/// The commands that go on from a stop, by session names the job has not
/// used.
struct Next<'a> {
    name: &'a str,
    target: &'a Path,
    bank1: &'a Path,
    motion: &'a str,
}

impl Next<'_> {
    fn give_way(&self, session: &str) -> Vec<String> {
        let (name, target) = (self.name, self.target.display());
        vec![
            format!("wellfed-bench give-way-prepare {name}-{session} --fallback {target}"),
            format!(
                "wellfed-bench give-way-execute {name}-{session} --fallback {target} \
                 --expected-plan-sha256 <the plan's> --boot-seconds {BANK0_BOOT_SECONDS}"
            ),
            "in that boot's capture: PLOTA1 bank0 bytes=… recorded=…".to_owned(),
        ]
    }

    fn restore(&self, session: &str, interrupted: bool) -> Vec<String> {
        let (name, bank1, motion) = (self.name, self.bank1.display(), self.motion);
        let interrupted = if interrupted { " --interrupted" } else { "" };
        let images =
            format!("--bank 1 --source {bank1} --target {bank1} --restore-marker{interrupted}");
        vec![
            format!("wellfed-bench flash-prepare {name}-{session} {images}{motion}"),
            format!(
                "wellfed-bench flash-execute {name}-{session} {images}{motion} \
                 --expected-plan-sha256 <the plan's> --boot-seconds {BANK1_BOOT_SECONDS}"
            ),
            "in that boot's capture: PLOTA1 bank0 check=match (only this finishes it)".to_owned(),
        ]
    }

    /// Bank 0 itself is not an image bank 1 can hand over to: write it again.
    fn rewrite(&self) -> Vec<String> {
        let (name, target, bank1) = (self.name, self.target.display(), self.bank1.display());
        vec![
            format!("wellfed-bench dump {name}-reread"),
            format!(
                "wellfed-bench flash-prepare {name}-again --bank 0 --original-read <that full read> \
                 --target {target} --bank1-source {bank1}{}",
                self.motion
            ),
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
}

impl Bench {
    /// After a verified bank-0 write: give way, boot bank 0, restore bank
    /// 1's marker, boot bank 1, and require its `check=match`.
    pub(super) fn finish_bank0(&mut self, args: &ExecuteArgs) -> Result<u8> {
        let update = &args.update;
        let bank1 = update
            .bank1_source
            .clone()
            .ok_or(CommandsError::Bank1SourceRequired)?;
        let target = update.target.clone();
        let motion = if update.attended_motion {
            " --attended-motion"
        } else {
            ""
        };
        let next = Next {
            name: &update.name,
            target: &target,
            bank1: &bank1,
            motion,
        };
        say!(
            "Bank 0 is written and verified but has not recorded itself, so bank 1 refuses every \
             update. Finishing: give way, boot bank 0, restore bank 1's marker, boot bank 1."
        );

        // 1. Give way to the image just written.
        let written = "bank 0 holds the new image, unrecorded; bank 1 is unchanged and \
                       boots, and refuses every update";
        let give_way = GiveWayArgs {
            name: give_way_name(&update.name),
            fallback: target.clone(),
        };
        let Some(sha) = self.give_way_plan(&give_way)? else {
            let steps = [next.give_way("giveway2"), next.restore("restore", false)].concat();
            return Ok(self.unfinished(written, &steps));
        };
        let plan: GiveWayPlan = self.operation(&give_way.name)?;
        let image = self.ota_sha256(&target)?;
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
            let steps = [next.give_way("giveway2"), next.restore("restore", false)].concat();
            return Ok(self.unfinished(written, &steps));
        }
        let gave_way = self.give_way_execute(&GiveWayExecuteArgs {
            give_way: give_way.clone(),
            expected_plan_sha256: sha,
            boot_seconds: BANK0_BOOT_SECONDS,
        })?;
        if gave_way != 0 {
            let now = "the give-way stopped: bank 1's first marker byte may or may not be \
                       cleared, and the feeder is powered and not booted";
            let steps = [next.give_way("giveway2"), next.restore("restore", false)].concat();
            return Ok(self.unfinished(now, &steps));
        }

        // 2. Bank 0's boot: it records itself.
        let restore_steps = next.restore("restore", false);
        match seen(&Paths::new(&self.root, &give_way.name).boot).recorded {
            Some(Ok(bytes)) => say!("Bank 0 recorded itself ({bytes} bytes)"),
            Some(Err(error)) => {
                let now = format!(
                    "bank 1 gave way and bank 0 runs, but it could not record itself \
                     (error={error}); updates stay refused"
                );
                let steps = match error {
                    Bank0Error::Store => [next.capture(), restore_steps].concat(),
                    Bank0Error::Extent | Bank0Error::Read => next.rewrite(),
                };
                return Ok(self.unfinished(&now, &steps));
            }
            None => {
                let now = format!(
                    "bank 1 gave way and bank 0 runs, but no PLOTA1 bank0 line came within \
                     {BANK0_BOOT_SECONDS} s"
                );
                let look = format!(
                    "wellfed-bench capture {}-bank0 --seconds {BANK0_BOOT_SECONDS}, and in it: \
                     PLOTA1 bank0 bytes=… recorded=…",
                    update.name
                );
                return Ok(self.unfinished(&now, &[vec![look], restore_steps].concat()));
            }
        }

        // 3. Restore bank 1's marker against its source review.
        let recorded = "bank 0 recorded itself and runs; bank 1 holds its source with the \
                        first marker byte cleared";
        let restore = UpdateArgs {
            name: format!("{}-restore", update.name),
            source: Some(bank1.clone()),
            bank: 1,
            original_read: None,
            target: bank1.clone(),
            interrupted: false,
            restore_marker: true,
            attended_motion: update.attended_motion,
            bank1_source: None,
        };
        let Some((sha, plan)) = self.update_plan(&restore)? else {
            return Ok(self.unfinished(recorded, &next.restore("restore2", false)));
        };
        let image = self.ota_sha256(&bank1)?;
        if !(plan.restore_marker
            && plan.bank == 1
            && plan.target_ota_sha256.eq_ignore_ascii_case(&image))
        {
            say!(
                "The restore plan is not bank 1's marker restore of {image} (bank {}, \
                 restore_marker {}, target {}); not executed",
                plan.bank,
                plan.restore_marker,
                plan.target_ota_sha256
            );
            return Ok(self.unfinished(recorded, &next.restore("restore2", false)));
        }
        let restored = self.flash_execute(&ExecuteArgs {
            update: restore.clone(),
            expected_plan_sha256: sha,
            boot_seconds: BANK1_BOOT_SECONDS,
        })?;
        if restored != 0 {
            let now = "the marker restore stopped: bank 0 recorded itself, bank 1 is part \
                       written, and the feeder is powered and not booted";
            return Ok(self.unfinished(now, &next.restore("restore2", true)));
        }

        // 4. Bank 1's boot check: the proof.
        match seen(&Paths::new(&self.root, &restore.name).boot).check {
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
                        [next.give_way("giveway2"), next.restore("restore2", false)].concat()
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

    /// Stop the finish without a retry: where the feeder stands, and what
    /// goes on from there.
    fn unfinished(&mut self, now: &str, steps: &[String]) -> u8 {
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

    /// The OTA image hash a frozen review pins, lowercase.
    fn ota_sha256(&self, review: &Path) -> Result<String> {
        let path: PathBuf = self.root.join(review).join("review.json");
        let frozen: ImageReview =
            wellfed_formats::load(&path).map_err(|error| CommandsError::ReviewUnreadable {
                review: self.rel(&path),
                error,
            })?;
        Ok(frozen.ota_sha256.to_ascii_lowercase())
    }
}
