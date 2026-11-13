//! How an install ends: a release's trial followed to its verdict, and
//! the end logged, kept (twice, then in memory) and published, with the
//! slot freed.

use tracing::{info, warn};
use wellfed_api::{Install, InstallFailure, InstallState, InstallTarget};
use wellfed_mirror::{Operation, Trial};
use wellfed_releases::STOCK_VERSION;

use super::Claim;
use crate::Service;

impl Service {
    /// Follow the release `build` an install just activated on
    /// `operation`'s turn, through its restart and trial: the install's
    /// state meanwhile is `confirming`, kept and published, and its end
    /// the trial's verdict, or `activated` when none came.
    pub(super) async fn follow_trial(
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
    pub(super) async fn end(
        &self,
        mut claim: Claim,
        state: InstallState,
        failure: Option<InstallFailure>,
    ) {
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
}
