//! Following a new build through its restart and trial, once an install
//! activated it: reads only, by the rule every client shares, each one
//! kept only if no session began or ended while it was taken.

use tokio::sync::watch;
use tokio::time::{Duration, Instant, timeout_at};
use wellfed_client::offer::{Confirmation, Restarted, confirmation};
use wellfed_model::{Answer, Op, Readiness, UpdatePhase};
use wellfed_session::hex::Lower;

use crate::{Operation, RandomFailed};

/// Between reads of a trial that has not decided.
const TRIAL_POLL: Duration = Duration::from_secs(2);

impl Operation {
    /// Follow the build `build` at `version` that [`Self::install`] just
    /// activated on this turn: its restart into bank 1 and its trial, until
    /// the trial's verdict or the turn's trial time passes. Reads change
    /// nothing, so looking again sends nothing the feeder could act on.
    ///
    /// The session that answered the activation is bank 0's, about to
    /// restart, so a bank 0 answer counts as a fallback only from a session
    /// opened after that answer; bank 1 runs only after the restart, so its
    /// answer counts from any. The verdict is the rule every client shares
    /// ([`wellfed_client::offer::confirmation`]).
    pub async fn follow_trial(
        &mut self,
        build: [u8; 32],
        version: u32,
    ) -> Result<Trial, RandomFailed> {
        let served = Lower(&build).to_string();
        let deadline = Instant::now() + self.trial;
        // Each session's start and end sends its build, and the push marked
        // it seen as bank 0 answered the activation: a change since means
        // that session is gone.
        let mut restarted = false;

        while Instant::now() < deadline {
            restarted |= self.build.has_changed().unwrap_or(true);
            self.build.borrow_and_update();
            match self.look().await? {
                // A session started or ended between the reading's steps:
                // they may come from two boots, so look again at once.
                Look::Moved => {
                    restarted = true;
                    continue;
                }
                Look::Absent => {}
                Look::Read(reading) => {
                    if let Some(trial) = verdict(&served, version, restarted, &reading) {
                        return Ok(trial);
                    }
                }
            }
            let wait = deadline.min(Instant::now() + TRIAL_POLL);
            // A session ending or opening wakes the look at once, and
            // counts: waiting marks the change seen.
            match timeout_at(wait, self.build.changed()).await {
                Ok(Ok(())) => restarted = true,
                // The feeder's task ended: nothing more will answer.
                Ok(Err(_)) => break,
                Err(_) => {}
            }
        }
        Ok(Trial::Unseen)
    }

    /// What the open session says it runs. The bank comes from the update
    /// state and the build from the welcome, both frozen; readiness from
    /// the status, which a build of another revision may not give, and
    /// which then counts as not ready. The build is the one the session
    /// marked seen before the steps, so a session that changed during them
    /// discards the reading ([`kept`]).
    async fn look(&mut self) -> Result<Look, RandomFailed> {
        let Some(build) = self.build.borrow().clone() else {
            return Ok(Look::Absent);
        };
        let id = self.request_id()?;
        let Ok(Answer::Update(view)) = self.step(id, Op::UpdateState).await else {
            return Ok(Look::Absent);
        };
        let id = self.request_id()?;
        let ready = matches!(
            self.step(id, Op::Status).await,
            Ok(Answer::Status(status))
                if status.feeder().is_some_and(|feeder| feeder.readiness == Readiness::Ready)
        );
        let reading = Reading {
            build,
            phase: view.phase,
            bank: view.bank,
            confirmed: view.min_version,
            ready,
        };
        Ok(kept(&self.build, reading))
    }
}

/// How a new build's trial ended, as the server saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trial {
    /// Bank 1 runs the build, and it confirmed itself.
    Confirmed,
    /// Bank 0 answers after the restart: the trial failed and the feeder
    /// gave way to its recovery copy, which may be older than what it ran
    /// before the install.
    FellBack,
    /// Bank 1 runs some other build: an update from elsewhere came between.
    WrongBuild,
    /// No verdict in the trial time: the feeder did not come back, or its
    /// trial had not decided.
    Unseen,
}

/// One look at the feeder during a trial.
#[derive(Debug)]
enum Look {
    /// No session answered.
    Absent,
    /// A session started or ended while the look's steps ran.
    Moved,
    Read(Reading),
}

/// `reading`, unless the session changed since `build` was marked seen:
/// its update state and its status are separate steps, and a new session
/// may answer the second, so a reading across a change could pair one
/// boot's build with another's bank or readiness.
fn kept(build: &watch::Receiver<Option<String>>, reading: Reading) -> Look {
    if build.has_changed().unwrap_or(true) {
        Look::Moved
    } else {
        Look::Read(reading)
    }
}

/// What a reading decides of the trial of `served` at `version`; `None`
/// to look again. `restarted` says a session ended or began since bank 0
/// answered the activation. Bank 0 counts as the fallback only when idle
/// in a session after that: a boot forgets the activation, so bank 0 still
/// saying activated is the activating boot reached again before its
/// restart, and bank 0 in any other phase is busy with an update of its
/// own (another client's check, give-way or transfer), whose end the look
/// waits for.
fn verdict(served: &str, version: u32, restarted: bool, reading: &Reading) -> Option<Trial> {
    let restarted_into = Restarted {
        build: &reading.build,
        bank: reading.bank,
        confirmed: reading.confirmed,
        ready: reading.ready,
    };
    match confirmation(served, version, restarted_into) {
        Confirmation::Confirmed => Some(Trial::Confirmed),
        Confirmation::WrongBuild => Some(Trial::WrongBuild),
        Confirmation::FellBack if restarted && reading.phase == UpdatePhase::Idle => {
            Some(Trial::FellBack)
        }
        Confirmation::FellBack | Confirmation::Pending => None,
    }
}

/// What a session after an activation says it runs.
#[derive(Debug)]
struct Reading {
    build: String,
    phase: UpdatePhase,
    bank: u8,
    /// The highest version it reports confirmed.
    confirmed: u32,
    ready: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVED: &str = "77";

    fn reading(bank: u8, phase: UpdatePhase, build: &str) -> Reading {
        Reading {
            build: build.into(),
            phase,
            bank,
            confirmed: 0,
            ready: false,
        }
    }

    #[test]
    fn a_reading_taken_across_a_session_change_is_looked_at_again() {
        let (sessions, mut build) = watch::channel(Some(SERVED.to_owned()));
        build.borrow_and_update();

        let quiet = kept(&build, reading(1, UpdatePhase::Idle, SERVED));
        assert!(matches!(quiet, Look::Read(_)), "{quiet:?}");

        // Bank 1's session answered the update state, then the feeder fell
        // back and bank 0's session answered the status.
        sessions.send_replace(Some("00".to_owned()));
        let moved = kept(&build, reading(1, UpdatePhase::Idle, SERVED));
        assert!(matches!(moved, Look::Moved), "{moved:?}");

        drop(sessions);
        build.borrow_and_update();
        let gone = kept(&build, reading(1, UpdatePhase::Idle, SERVED));
        assert!(matches!(gone, Look::Moved), "a task that ended: {gone:?}");
    }

    #[test]
    fn bank_0_is_the_fallback_only_when_idle_after_a_session_change() {
        let fell_back = verdict(SERVED, 7, true, &reading(0, UpdatePhase::Idle, "00"));
        assert_eq!(fell_back, Some(Trial::FellBack));

        assert_eq!(
            verdict(SERVED, 7, false, &reading(0, UpdatePhase::Idle, "00")),
            None,
            "the activating session itself"
        );
        for busy in [
            UpdatePhase::Activated,
            UpdatePhase::Checking,
            UpdatePhase::GivingWay,
            UpdatePhase::Receiving,
        ] {
            assert_eq!(
                verdict(SERVED, 7, true, &reading(0, busy, "00")),
                None,
                "{busy:?}"
            );
        }
    }

    #[test]
    fn bank_1_running_another_build_is_the_wrong_build_and_its_own_confirms() {
        assert_eq!(
            verdict(SERVED, 7, true, &reading(1, UpdatePhase::Idle, "88")),
            Some(Trial::WrongBuild)
        );
        assert_eq!(
            verdict(SERVED, 7, true, &reading(1, UpdatePhase::Idle, SERVED)),
            None,
            "its trial has not decided"
        );
        let mut confirmed = reading(1, UpdatePhase::Idle, SERVED);
        confirmed.confirmed = 7;
        assert_eq!(verdict(SERVED, 7, true, &confirmed), Some(Trial::Confirmed));
    }
}
