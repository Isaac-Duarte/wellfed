//! Installs: an owner's request to install one of the server's releases on
//! one feeder, or to restore its stock firmware from the server's backup
//! (`stock.rs`), run in the background on the feeder's long-operation lane.
//!
//! One install runs at a time per server, held by a [`Claim`] on the
//! service's one slot: a second request while it runs is
//! [`InstallError::Running`]. The slot also holds the running install's
//! progress, which `GET` reads and the stream carries; the store keeps each
//! install when it starts, when the feeder takes its offer, and when it
//! ends, so a restart finds the one that was running and marks it failed,
//! and the claim, being in memory, is free again. That end is `interrupted`
//! when the offer never took, and otherwise uncertain (`unreachable`,
//! `offered`): bank 1 may be partly written, or activated with its answer
//! lost. An end the store would not take, even a second time, is kept in
//! memory for `GET` until the server restarts and marks its row so.
//!
//! Nothing is offered again on the owner's behalf. The driver resumes a
//! transfer across a dropped session (chunks are idempotent by offset), but
//! an offer the feeder refuses, an install it stops, or a feeder that never
//! comes back ends the install; another is another request.

use tokio::sync::{oneshot, watch};
use tracing::{Instrument, info, info_span, warn};
use wellfed_api::{
    ClientRole, Connection as Link, FeederId, FeederRefusal, Install, InstallFailure, InstallId,
    InstallRequest, InstallState, InstallTarget, StreamEvent,
};
use wellfed_mirror::{Operation, PushError, Pushed, Restore, Trial, Update, UpdateError};
use wellfed_model::{Refusal, UpdatePhase, UpdateView};
use wellfed_releases::{STOCK_VERSION, StockBackup};

use crate::{Caller, CommandError, Error, InternalError, Resource, Result, Service, lock};

mod claim;
mod error;

pub(crate) use claim::Slot;
pub use error::InstallError;

use claim::Claim;

/// Progress reaches the stream each time this many more bytes are written.
const PROGRESS_STEP: u32 = 32 * 1024;

/// What an install pushes.
pub(crate) enum Offer {
    /// A release file as the cache keeps it, and the build it carries.
    Release { file: Vec<u8>, build: [u8; 32] },
    /// The kept stock backup, with the manifest and statement the owner
    /// signed for it.
    Stock {
        manifest: Vec<u8>,
        downgrade: Vec<u8>,
        backup: StockBackup,
    },
}

impl Service {
    /// Start installing `request.release` on the feeder, and return the
    /// install as it starts. It runs in the background; its progress and
    /// its end reach the stream, and [`Self::install_state`] reads it.
    ///
    /// The server installs only where its key is an owner's: the feeder
    /// refuses updates from any other role, so that is refused here first,
    /// as the feeder would. A feeder the server has no session with is
    /// refused too, rather than waited for.
    pub async fn install(
        &self,
        caller: &Caller,
        id: FeederId,
        request: InstallRequest,
    ) -> Result<Install> {
        let row = self.row(id).await?;
        if row.role != ClientRole::Owner {
            return Err(Error::Refused(FeederRefusal::Forbidden));
        }
        let (release, file) = self.release_file(request.release).await?;
        // An image for another model is never offered: the device would
        // refuse it too, but only once it is sent.
        if release.model != row.model.into() {
            return Err(InstallError::OtherModel {
                release: release.model,
                device: row.model,
            }
            .into());
        }
        // A feeder of another build takes an install: the update path is
        // frozen, and the install is what brings it to this one.
        if !matches!(self.mirror.link(id), Some(Link::Online)) {
            return Err(CommandError::Offline.into());
        }
        self.keeps_stock(id, row.identity).await?;

        let install = self.new_install(
            caller,
            id,
            InstallTarget::Release {
                release: release.id,
            },
            release.version,
            release.image_len,
        )?;
        let offer = Offer::Release {
            file,
            build: release.build_id.0,
        };
        self.begin(install, offer).await
    }

    /// An install of `target` on the feeder `id`, as it starts.
    pub(crate) fn new_install(
        &self,
        caller: &Caller,
        id: FeederId,
        target: InstallTarget,
        version: u32,
        image_len: u32,
    ) -> Result<Install> {
        Ok(Install {
            id: InstallId(self.shared.new_id()?),
            feeder: id,
            target,
            version,
            by: caller.user.id,
            started: self.shared.clock.now(),
            state: InstallState::Running,
            written: 0,
            image_len,
            failure: None,
            finished: None,
        })
    }

    /// Take the slot for `install` (refused while another runs), keep it,
    /// and push `offer` in the background.
    ///
    /// The row is kept by the install's own task, not the caller: a caller
    /// dropped while the row is written would otherwise free the slot with
    /// the row left `running` and nothing to end it. The caller only waits
    /// for the row to be kept, and an install it stops waiting for runs on.
    pub(crate) async fn begin(&self, install: Install, offer: Offer) -> Result<Install> {
        let claim = Claim::take(&self.operations.installing, install)?;
        let (kept, was_kept) = oneshot::channel::<Result<()>>();

        let service = self.clone();
        let cancel = self.cancel.child_token();
        let span = info_span!(parent: None, "install", feeder = %install.feeder);
        self.tasks.spawn(
            async move {
                if let Err(error) = service.shared.store.save_install(&install).await {
                    drop(claim);
                    let _unheard = kept.send(Err(error.into()));
                    return;
                }
                service.started(install);
                let _unheard = kept.send(Ok(()));

                // On shutdown the push stops and the row stays `running`
                // until the next start marks it failed, as after a crash.
                // Cancellation is looked at first: shutting down also ends
                // the mirror's session, and a push polled after that would
                // end `unreachable` and store it, though the server stopped
                // it.
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => {}
                    () = service.run_install(claim, offer) => {}
                }
            }
            .instrument(span),
        );

        was_kept.await.map_err(|_| InternalError::Task)??;
        Ok(install)
    }

    /// Say that `install` started: in the log and on the stream.
    fn started(&self, install: Install) {
        match install.target {
            InstallTarget::Release { release } => info!(
                feeder = %install.feeder,
                %release,
                "install of version {} started",
                install.version
            ),
            InstallTarget::Stock { sha256 } => info!(
                feeder = %install.feeder,
                backup = %sha256,
                "restore of stock {STOCK_VERSION} started, as version {}",
                install.version
            ),
            // A newer server's target, which this one never starts.
            InstallTarget::Unknown => {}
        }
        self.publish_install(install);
    }

    /// The running install, if it is the feeder `id`'s.
    pub(crate) fn running_on(&self, id: FeederId) -> Option<Install> {
        lock(&self.operations.installing)
            .running
            .filter(|install| install.feeder == id)
    }

    /// `last` as it ended, if its end is held in memory rather than in the
    /// store.
    pub(crate) fn unsaved(&self, last: Install) -> Install {
        let installs = lock(&self.operations.installing);
        let unsaved = installs.unsaved.iter().find(|ended| ended.id == last.id);
        unsaved.copied().unwrap_or(last)
    }

    /// The feeder's running install, or else its last one.
    pub async fn install_state(&self, id: FeederId) -> Result<Install> {
        self.row(id).await?;
        if let Some(install) = self.running_on(id) {
            return Ok(install);
        }

        let last = self.shared.store.latest_install(id).await?;
        let last = last.ok_or(Error::NotFound(Resource::Install))?;
        Ok(self.unsaved(last))
    }

    /// Mark the install a stopped server left running failed:
    /// `interrupted`, or uncertain once its offer took.
    pub(crate) async fn interrupt_installs(&self) -> Result<()> {
        let now = self.shared.clock.now();
        let interrupted = self.shared.store.interrupt_installs(now).await?;
        if interrupted > 0 {
            warn!("an install was running when the server stopped: marked as it stood");
        }
        Ok(())
    }

    /// Run the install to its end, keep that, and free the slot. A
    /// release's activation is followed through its trial on the same
    /// turn, so the session that answered it is the one the follow counts
    /// from.
    async fn run_install(&self, mut claim: Claim, offer: Offer) {
        let feeder = claim.install.feeder;
        let Ok(mut operation) = self.mirror.operation(feeder).await else {
            // The only refusal: the feeder was let go before the turn came.
            let failure = InstallFailure::Unreachable { offered: false };
            return self.end(claim, InstallState::Failed, Some(failure)).await;
        };
        let ended = self.push(&mut claim, &mut operation, &offer).await;
        let failed = |failure| (InstallState::Failed, Some(failure));
        let (state, failure) = match ended {
            Ok(Pushed::Activated) => match offer {
                Offer::Release { build, .. } => {
                    self.follow_trial(&mut claim, &mut operation, build).await
                }
                // Stock serves no device API to follow.
                Offer::Stock { .. } => (InstallState::Activated, None),
            },
            Ok(Pushed::AlreadyRunning) => (InstallState::AlreadyRunning, None),
            Ok(Pushed::Failed(error)) => failed(failure(error)),
            Ok(Pushed::Gone { offered }) => failed(InstallFailure::Unreachable { offered }),
            // The cache verified the file just now, so it is never short.
            Err(PushError::Short) => failed(InstallFailure::Unexpected),
            Err(PushError::Random(error)) => {
                warn!("the install stopped: {error}");
                failed(InstallFailure::Server)
            }
        };

        drop(operation);
        self.end(claim, state, failure).await;
    }

    /// Follow the release `build` an install just activated on
    /// `operation`'s turn, through its restart and trial: the install's
    /// state meanwhile is `confirming`, kept and published, and its end
    /// the trial's verdict, or `activated` when none came.
    async fn follow_trial(
        &self,
        claim: &mut Claim,
        operation: &mut Operation,
        build: [u8; 32],
    ) -> (InstallState, Option<InstallFailure>) {
        let install = claim.update(|install| install.state = InstallState::Confirming);
        info!(
            "activated: the feeder restarts into version {}, and its trial is followed",
            install.version
        );
        if let Err(error) = self.shared.store.save_install(&install).await {
            warn!("that the install's trial is followed was not stored: {error}");
        }
        self.publish_install(install);

        match operation.follow_trial(build, install.version).await {
            Ok(Trial::Confirmed) => (InstallState::Confirmed, None),
            Ok(Trial::FellBack) => (InstallState::Failed, Some(InstallFailure::RolledBack)),
            Ok(Trial::WrongBuild) => (InstallState::Failed, Some(InstallFailure::WrongBuild)),
            Ok(Trial::Unseen) => (InstallState::Activated, None),
            Err(error) => {
                warn!("the trial is no longer followed: {error}");
                (InstallState::Activated, None)
            }
        }
    }

    /// End `claim`'s install as `state`, with `failure`: logged, kept and
    /// published, and the slot freed.
    async fn end(&self, mut claim: Claim, state: InstallState, failure: Option<InstallFailure>) {
        let now = self.shared.clock.now();
        let install = claim.update(|install| {
            install.state = state;
            install.failure = failure;
            install.finished = Some(now);
        });
        match (state, failure) {
            (_, Some(failure)) => warn!("the install failed: {failure:?}"),
            (InstallState::AlreadyRunning, None) => info!(
                "the feeder already runs version {}: nothing was sent",
                install.version
            ),
            (InstallState::Confirmed, None) => info!(
                "the feeder runs version {}, and it confirmed itself",
                install.version
            ),
            (_, None) => match install.target {
                InstallTarget::Release { .. } => warn!(
                    "activated, and no verdict of version {}'s trial came: the feeder's update \
                     state tells",
                    install.version
                ),
                InstallTarget::Stock { .. } => info!(
                    "activated: the feeder restarts into stock {STOCK_VERSION}, which serves no \
                     device API"
                ),
                InstallTarget::Unknown => {}
            },
        }

        self.save_end(&claim, install).await;
        self.publish_install(install);
        drop(claim);
    }

    /// Store the install's end, trying twice; an end the store would not
    /// take stays in memory, so `GET` does not read the row's `running`.
    async fn save_end(&self, claim: &Claim, install: Install) {
        for attempt in 1..=2 {
            match self.shared.store.save_install(&install).await {
                Ok(()) => return,
                Err(error) => {
                    warn!("the install's end was not stored (attempt {attempt}): {error}");
                }
            }
        }
        claim.unsaved(install);
    }

    /// Push the offer over the feeder's long-operation turn, publishing
    /// the progress as it goes.
    async fn push(
        &self,
        claim: &mut Claim,
        operation: &mut Operation,
        offer: &Offer,
    ) -> Result<Pushed, PushError> {
        // Once the feeder took the offer, the row says so, so a restart
        // before the end reads the install as uncertain, not as one that
        // changed nothing. The note is kept beside the push, which a store
        // write must not hold up; the push's end is kept only after it.
        let install = claim.install.id;
        let (took, mut taken) = watch::channel(false);
        let mut published = 0;
        let progress = move |view: &UpdateView| {
            if view.phase != UpdatePhase::Idle && !*took.borrow() {
                took.send_replace(true);
            }
            let install = claim.update(|install| install.written = view.written);
            let step = |written: u32| written / PROGRESS_STEP;
            let done = view.written == install.image_len;
            if step(view.written) != step(published) || (done && published != view.written) {
                published = view.written;
                self.publish_install(install);
            }
        };
        let pushing = async {
            match offer {
                Offer::Release { file, build } => {
                    let update = Update {
                        file,
                        build: *build,
                    };
                    operation.install(update, progress).await
                }
                Offer::Stock {
                    manifest,
                    downgrade,
                    backup,
                } => {
                    let restore = Restore {
                        manifest,
                        downgrade,
                        payload: backup.payload(),
                        build: backup.payload_sha256(),
                    };
                    Ok(operation.restore(restore, progress).await?)
                }
            }
        };
        // A note the store would not take leaves a row a restart would read
        // as an install that changed nothing, while bank 1 may be partly
        // written: the push stops there, and the install ends uncertain now
        // rather than carry on unrecorded. The sender goes with the push, so
        // the note waits for nothing once it has ended.
        let note = async {
            if taken.wait_for(|took| *took).await.is_ok()
                && let Err(error) = self.shared.store.install_offered(install).await
            {
                warn!(
                    "that the feeder took the offer was not stored, so the install stops: {error}"
                );
                return;
            }
            std::future::pending::<()>().await;
        };
        tokio::select! {
            pushed = pushing => pushed,
            () = note => Ok(Pushed::Gone { offered: true }),
        }
    }

    fn publish_install(&self, install: Install) {
        self.shared.hub.publish(StreamEvent::Install {
            feeder: install.feeder,
            install,
        });
    }
}

/// The install's end as the API names it.
fn failure(error: UpdateError) -> InstallFailure {
    match error {
        UpdateError::Refused(Refusal::Update(refusal)) => InstallFailure::UpdateRefused { refusal },
        UpdateError::Refused(refusal) => InstallFailure::Refused {
            refusal: refusal.into(),
        },
        UpdateError::Stopped(stop) => InstallFailure::Stopped { stop },
        UpdateError::Lost => InstallFailure::Lost,
        // The driver ends a push it cannot settle `Gone`, never with this;
        // read the same way here, it stays uncertain whoever carries it.
        UpdateError::Uncertain(_) => InstallFailure::Unreachable { offered: true },
    }
}
