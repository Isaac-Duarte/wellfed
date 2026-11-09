//! The portion calibration: what one portion weighs and the bowl mode,
//! read from the feeder and replaced against the generation read, as the
//! settings are. The food views count in grams only once it is set, from
//! the portions statistics the server keeps: read again after a change, so
//! the views follow it at once.

use serde::Serialize;
use tracing::warn;
use wellfed_api::{
    Calibration, CalibrationView, CommandKind, CommandOutcome, FeederId, FeederRefusal, Generation,
    PORTION_MG_MAX, PORTION_MG_MIN,
};
use wellfed_model::{Answer, Op, StatsSection};

use crate::feeders::commands::{CommandError, refused};
use crate::feeders::journal::Write;
use crate::{Caller, Error, IdempotencyKey, InternalError, Invalid, Result, Service};

/// A calibration change as it is hashed into the journal: the base it
/// names is part of the request.
#[derive(Serialize)]
struct CalibrationWrite {
    base: Generation,
    calibration: Calibration,
}

impl Service {
    /// The calibration, read from the feeder: a change must name the
    /// current generation, so a copy would only invite a stale write.
    pub async fn calibration(&self, id: FeederId) -> Result<CalibrationView> {
        self.row(id).await?;
        self.read_calibration(id).await
    }

    /// Store `calibration` if the feeder's is still at `base`. A stale
    /// base gets [`Error::StaleCalibration`] with the current one. The
    /// feeder refuses a bowl mode it has not verified (`unverified`), and
    /// takes it only from an owner's key (`forbidden` otherwise). After the
    /// change the calibration is read again and kept as the write's
    /// outcome, so a retry under the same key returns it.
    pub async fn put_calibration(
        &self,
        caller: &Caller,
        id: FeederId,
        base: Generation,
        calibration: Calibration,
        key: Option<IdempotencyKey>,
    ) -> Result<CalibrationView> {
        if calibration
            .portion_mg
            .is_some_and(|mg| !(PORTION_MG_MIN..=PORTION_MG_MAX).contains(&mg))
        {
            return Err(Invalid::PortionWeight.into());
        }
        self.live_row(id).await?;

        let recorded = self
            .journaled(Write {
                caller,
                feeder: id,
                kind: CommandKind::Calibration,
                body: &CalibrationWrite { base, calibration },
                key,
                op: Op::PutCalibration { base, calibration },
            })
            .await?;

        match recorded.outcome {
            CommandOutcome::Calibration { calibration } => Ok(calibration),
            // As for settings: the change is made, so a failed read-back is
            // unknown rather than offline, and a retry reads it again.
            CommandOutcome::Done => {
                let calibration = self
                    .read_calibration(id)
                    .await
                    .map_err(|error| match error {
                        Error::Command(CommandError::Offline | CommandError::Legacy) => {
                            CommandError::Unknown.into()
                        }
                        other => other,
                    })?;
                self.amend(&recorded.key, &CommandOutcome::Calibration { calibration })
                    .await?;
                self.refresh_portions(id).await;
                Ok(calibration)
            }
            CommandOutcome::Refused {
                refusal: FeederRefusal::Stale,
            } => Err(Error::StaleCalibration(self.read_calibration(id).await?)),
            CommandOutcome::Refused { refusal } => Err(Error::Refused(refusal)),
            CommandOutcome::Settings { .. }
            | CommandOutcome::Plans { .. }
            | CommandOutcome::Call { .. }
            | CommandOutcome::Superseded
            | CommandOutcome::Feeding { .. }
            | CommandOutcome::Fed { .. }
            | CommandOutcome::Invited { .. }
            | CommandOutcome::Unrecorded => Err(InternalError::Unexpected.into()),
        }
    }

    /// Read the feeder's portions statistics again and keep them, so the
    /// food views weigh by a new calibration at once rather than at the
    /// mirror's next read. A read that fails leaves that next read to it.
    async fn refresh_portions(&self, id: FeederId) {
        let section = StatsSection::Portions;
        let read = match self.request_id() {
            Ok(request) => self.ask(id, request, Op::Stats { section }).await,
            Err(error) => Err(error),
        };
        let stats = match read {
            Ok(Answer::Stats(stats)) => stats,
            Ok(other) => {
                warn!(feeder = %id, "the portions statistics were not read back: {other:?}");
                return;
            }
            Err(error) => {
                warn!(feeder = %id, %error, "the portions statistics were not read back");
                return;
            }
        };

        let store = &self.shared.store;
        let boot_count = match store.state(id).await {
            Ok(state) => state.status.map(|(status, _)| status.boot_count),
            Err(error) => {
                warn!(feeder = %id, %error, "the portions statistics were not kept");
                return;
            }
        };
        let now = self.shared.clock.now();
        if let Err(error) = store.record_stats(id, now, boot_count, &stats).await {
            warn!(feeder = %id, %error, "the portions statistics were not kept");
        }
    }

    async fn read_calibration(&self, id: FeederId) -> Result<CalibrationView> {
        match self.ask(id, self.request_id()?, Op::GetCalibration).await? {
            Answer::Calibration(view) => Ok(view),
            other => Err(refused(&other)),
        }
    }
}
