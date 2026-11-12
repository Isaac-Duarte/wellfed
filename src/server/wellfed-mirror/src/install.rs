//! An install on the long lane: `wellfed-connect`'s update driver, the
//! one the owner's CLI runs, with each of its steps an [`Operation`] step.
//! "Restore stock" is the same push of the feeder's own stock image, with
//! the downgrade statement it needs; reading that image from bank 1 is a
//! page per step.
//!
//! The feeder task keeps the session, so the driver never opens one: when
//! the session drops (a cut, or the feeder restarting into bank 0 to take
//! the release), the task reconnects on its own, and the driver waits for
//! the link to come back and reads the feeder's update state from the new
//! session: never its status, since bank 0 may run a build of another
//! schema revision, which only the frozen update path reaches. Feeds and status reads keep their own lane throughout, so each
//! waits at most one chunk.

use tokio::time::{Duration, Instant, sleep, timeout_at};
use tracing::warn;
use wellfed_api::Connection as Link;
use wellfed_client::install::Bank1Reader;
use wellfed_client::offer::{Confirmation, Restarted, confirmation};
use wellfed_client::release::MANIFEST_LEN;
use wellfed_client::release::manifest_digest;
use wellfed_client::update::Image;
use wellfed_connect::update::{self, Opened, Steps, Way};
use wellfed_model::{Answer, Op, Readiness, UpdatePhase, UpdateView};
use wellfed_session::hex::Lower;

use crate::{AskError, Operation, RandomFailed};

pub use wellfed_client::install::ReadError;
pub use wellfed_client::update::InstallError as UpdateError;
pub use wellfed_connect::update::Pushed;

/// From a restart until the feeder answers the device API again.
const REOPEN_WAIT: Duration = Duration::from_secs(5 * 60);
/// Between state reads that find no session to answer them.
const POLL: Duration = Duration::from_secs(1);
/// Between reads of a trial that has not decided.
const TRIAL_POLL: Duration = Duration::from_secs(2);

/// What an install offers: a release file as the server keeps it (the
/// signed manifest, then the image it names) and the build it carries.
#[derive(Clone, Copy, Debug)]
pub struct Update<'a> {
    pub file: &'a [u8],
    pub build: [u8; 32],
}

/// What "Restore stock" offers: the kept backup's bank contents, the
/// manifest the owner signed them with as the stock image, and the
/// downgrade statement that lets this feeder take it.
#[derive(Clone, Copy, Debug)]
pub struct Restore<'a> {
    pub manifest: &'a [u8],
    pub downgrade: &'a [u8],
    pub payload: &'a [u8],
    /// The build the manifest names: the payload's SHA-256.
    pub build: [u8; 32],
}

/// Why reading bank 1 stopped. Reads change nothing on the feeder, so a
/// read that stopped is simply asked for again.
#[derive(Debug, thiserror::Error)]
pub enum BankReadError {
    /// The feeder's answer did not fit (a refusal among them).
    #[error(transparent)]
    Bank(#[from] ReadError),
    /// The session dropped, or a page's answer did not come in time.
    #[error("the feeder stopped answering the read")]
    Lost(#[source] AskError),
    #[error(transparent)]
    Random(#[from] RandomFailed),
}

/// Why an install could not run its steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PushError {
    #[error("the release file is shorter than a manifest")]
    Short,
    #[error(transparent)]
    Random(#[from] RandomFailed),
}

impl Operation {
    /// Install `update` until bank 1 is activated, the install fails, or
    /// the feeder is gone, calling `progress` with each update state the
    /// feeder answers.
    pub async fn install(
        &mut self,
        update: Update<'_>,
        progress: impl FnMut(&UpdateView),
    ) -> Result<Pushed, PushError> {
        let (manifest, payload) = update
            .file
            .split_at_checked(MANIFEST_LEN)
            .ok_or(PushError::Short)?;
        let image = Image {
            manifest,
            downgrade: None,
            payload,
        };

        if let Some(ended) = self.clear_stale(manifest).await? {
            return Ok(ended);
        }
        let pace = self.pace;
        let mut steps = Lane {
            operation: self,
            build: update.build,
            progress,
        };
        Ok(wellfed_connect::update::push(image, &mut steps, pace).await?)
    }

    /// Restore stock: push `restore` until bank 1 is activated, the
    /// install fails, or the feeder is gone. Activated, the feeder
    /// restarts into stock, which serves no device API; an activation
    /// whose answer was lost therefore leaves the feeder gone, never
    /// re-offered.
    pub async fn restore(
        &mut self,
        restore: Restore<'_>,
        progress: impl FnMut(&UpdateView),
    ) -> Result<Pushed, RandomFailed> {
        let image = Image {
            manifest: restore.manifest,
            downgrade: Some(restore.downgrade),
            payload: restore.payload,
        };
        if let Some(ended) = self.clear_stale(restore.manifest).await? {
            return Ok(ended);
        }
        let pace = self.pace;
        let mut steps = Lane {
            operation: self,
            build: restore.build,
            progress,
        };
        wellfed_connect::update::push(image, &mut steps, pace).await
    }

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
            if let Some(reading) = self.reading().await? {
                let restarted_into = Restarted {
                    build: &reading.build,
                    bank: reading.bank,
                    confirmed: reading.confirmed,
                    ready: reading.ready,
                };
                match confirmation(&served, version, restarted_into) {
                    Confirmation::Confirmed => return Ok(Trial::Confirmed),
                    Confirmation::WrongBuild => return Ok(Trial::WrongBuild),
                    // A boot forgets the activation, so bank 0 still
                    // saying so is the activating boot, reached again
                    // before its restart.
                    Confirmation::FellBack
                        if restarted && reading.phase != UpdatePhase::Activated =>
                    {
                        return Ok(Trial::FellBack);
                    }
                    // Bank 0 still answering in the activating boot, or a
                    // trial not yet decided.
                    Confirmation::FellBack | Confirmation::Pending => {}
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

    /// What the open session says it runs, or `None` while no session
    /// answers. The bank comes from the update state and the build from
    /// the welcome, both frozen; readiness from the status, which a build
    /// of another revision may not give, and which then counts as not
    /// ready.
    async fn reading(&mut self) -> Result<Option<Reading>, RandomFailed> {
        let Some(build) = self.build.borrow().clone() else {
            return Ok(None);
        };
        let id = self.request_id()?;
        let Ok(Answer::Update(view)) = self.step(id, Op::UpdateState).await else {
            return Ok(None);
        };
        let id = self.request_id()?;
        let ready = matches!(
            self.step(id, Op::Status).await,
            Ok(Answer::Status(status))
                if status.feeder().is_some_and(|feeder| feeder.readiness == Readiness::Ready)
        );
        Ok(Some(Reading {
            build,
            phase: view.phase,
            bank: view.bank,
            confirmed: view.min_version,
            ready,
        }))
    }

    /// Clear the way for an offer of `manifest`, by the rule every client
    /// shares ([`wellfed_connect::update::clear_stale`]). Any doubt about a
    /// cancel ends the push before the offer: `Gone` without `offered`, or
    /// the feeder's refusal. A running install, or a state that cannot be
    /// read, is left to the push, which reads it again and is refused as
    /// before.
    async fn clear_stale(&mut self, manifest: &[u8]) -> Result<Option<Pushed>, RandomFailed> {
        let id = self.request_id()?;
        let Ok(Answer::Update(view)) = self.step(id, Op::UpdateState).await else {
            return Ok(None);
        };
        let ours = manifest_digest(manifest);
        let mut lane = Lane {
            operation: self,
            build: [0; 32],
            progress: |_: &UpdateView| {},
        };
        let way = update::clear_stale(&mut lane, view, Some(ours), || {
            warn!("cancelling the candidate a stopped install left pending");
        })
        .await?;
        Ok(match way {
            Way::Clear(_) | Way::Running(_) => None,
            Way::Refused(refusal) => Some(Pushed::Failed(UpdateError::Refused(refusal))),
            Way::CancelUnknown => Some(Pushed::Gone { offered: false }),
        })
    }

    /// Read bank 1 whole from the bank-0 role, a page per step, calling
    /// `progress` with the bytes read so far after each page.
    pub async fn read_bank1(
        &mut self,
        mut progress: impl FnMut(usize),
    ) -> Result<Vec<u8>, BankReadError> {
        let mut reader = Bank1Reader::new();
        loop {
            let id = self.request_id()?;
            let answer = self
                .step(id, reader.next_op())
                .await
                .map_err(BankReadError::Lost)?;
            if let Some(bank) = reader.take(answer)? {
                return Ok(bank);
            }
            progress(reader.read());
        }
    }

    pub(crate) fn request_id(&self) -> Result<crate::RequestId, RandomFailed> {
        let mut id = crate::RequestId::default();
        self.random.fill(&mut id)?;
        Ok(id)
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

/// What a session after an activation says it runs.
struct Reading {
    build: String,
    phase: UpdatePhase,
    bank: u8,
    /// The highest version it reports confirmed.
    confirmed: u32,
    ready: bool,
}

/// The long lane, as the driver's steps.
struct Lane<'a, P> {
    operation: &'a mut Operation,
    build: [u8; 32],
    progress: P,
}

impl<P: FnMut(&UpdateView)> Steps for Lane<'_, P> {
    type Error = RandomFailed;

    async fn step(&mut self, op: Op) -> Result<Option<Answer>, RandomFailed> {
        let id = self.operation.request_id()?;
        // Offline, unknown, or the feeder let go: the request may have run,
        // and the driver reopens and reads where the install stands.
        Ok(self.operation.step(id, op).await.ok())
    }

    async fn reopen(&mut self) -> Result<Option<Opened>, RandomFailed> {
        let deadline = Instant::now() + REOPEN_WAIT;
        loop {
            let online = self.operation.link.wait_for(|link| *link == Link::Online);
            match timeout_at(deadline, online).await {
                Ok(Ok(_)) => {}
                // Not back in time, or the feeder's task has ended.
                Err(_) | Ok(Err(_)) => return Ok(None),
            }
            let id = self.operation.request_id()?;
            if let Ok(Answer::Update(view)) = self.operation.step(id, Op::UpdateState).await
                && let Some(build) = self.operation.build.borrow().clone()
            {
                return Ok(Some(Opened {
                    bank: view.bank,
                    build,
                }));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            sleep(POLL).await;
        }
    }

    /// Bank 1 can run the release's build only once bank 0 activated it.
    fn activated(&mut self, opened: &Opened) -> bool {
        opened.bank == 1
            && opened
                .build
                .eq_ignore_ascii_case(&Lower(&self.build).to_string())
    }

    fn progress(&mut self, view: &UpdateView) {
        // The session answering the activation is the last before the
        // restart: what follows the trial counts sessions from here.
        if view.phase == UpdatePhase::Activated {
            self.operation.build.borrow_and_update();
        }
        (self.progress)(view);
    }
}
