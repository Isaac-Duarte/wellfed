//! The meal call's upload on the long lane: `wellfed-client`'s upload
//! driver, the one the owner's CLI runs, with each of its asks an
//! [`Operation`] step, so a feed or a status read waits at most one chunk.
//!
//! The stream comes converted: the server takes the feeder's own format
//! from its client and converts nothing. A step whose answer is lost ends
//! the upload as [`CallError::Lost`] rather than resending on its own,
//! since the caller's journal decides whether the upload runs again; the
//! retry then starts from where the feeder says it stands
//! ([`Upload::resume`]).

use tokio::time::{Instant, sleep};
use wellfed_client::call::{Step, Upload};
use wellfed_model::{Answer, CallView, Op};

use crate::{AskError, Operation, RandomFailed};

pub use wellfed_client::call::{Clip, StreamError, UploadError, UploadFailed};

/// Why an upload stopped short of the feeder storing the clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CallError {
    /// Nothing reached the feeder: it was offline before the first step.
    #[error("the upload never reached the feeder")]
    Unsent(#[source] AskError),
    /// A step got no answer, or the session dropped part-way: the feeder
    /// holds what it received, and [`GetCall`](Op::GetCall) says how much.
    #[error("the feeder stopped answering the upload")]
    Lost(#[source] AskError),
    /// The feeder answered, and the upload cannot go on.
    #[error(transparent)]
    Failed(#[from] UploadFailed),
    #[error(transparent)]
    Random(#[from] RandomFailed),
}

/// Where an upload begins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallStart {
    /// From the start: the feeder's clip is erased first.
    Fresh,
    /// From where the feeder's view (read with [`Operation::read_call`])
    /// says this upload stands, after an attempt whose answers were lost.
    Resume(CallView),
}

impl Operation {
    /// Upload `clip` until the feeder stores it, the upload fails, or a
    /// step goes unanswered.
    pub async fn upload_call(
        &mut self,
        clip: &Clip,
        start: CallStart,
    ) -> Result<CallView, CallError> {
        let mut sent = false;
        let mut upload = match start {
            CallStart::Fresh => Upload::new(clip),
            CallStart::Resume(view) => Upload::resume(clip, &view),
        };

        loop {
            match upload.poll(Instant::now().into_std()) {
                Step::Ask(op) => {
                    let answer = self.call_step(op, &mut sent).await?;
                    upload.answer(&answer, Instant::now().into_std());
                }
                Step::Wait(wait) => sleep(wait).await,
                Step::Stored(view) => return Ok(view),
                Step::Failed(failed) => return Err(failed.into()),
            }
        }
    }

    /// The feeder's meal call: its clip, or the upload under way. A read,
    /// so [`CallError::Lost`] here changed nothing on the feeder.
    pub async fn read_call(&mut self) -> Result<CallView, CallError> {
        match self.call_step(Op::GetCall, &mut false).await? {
            Answer::Call(view) => Ok(view),
            Answer::Refused(refusal) => {
                Err(UploadFailed::Upload(UploadError::Refused(refusal)).into())
            }
            _ => Err(UploadFailed::Upload(UploadError::Unexpected).into()),
        }
    }

    /// Send one step; `sent` records that one left, so a later failure is
    /// [`CallError::Lost`] and never [`CallError::Unsent`].
    async fn call_step(&mut self, op: Op, sent: &mut bool) -> Result<Answer, CallError> {
        let id = self.request_id()?;
        match self.step(id, op).await {
            Ok(answer) => {
                *sent = true;
                Ok(answer)
            }
            Err(error @ (AskError::Offline | AskError::NotRunning | AskError::Unsendable))
                if !*sent =>
            {
                Err(CallError::Unsent(error))
            }
            Err(error) => {
                *sent = true;
                Err(CallError::Lost(error))
            }
        }
    }
}
