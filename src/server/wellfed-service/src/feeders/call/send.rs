//! Sending a meal call's upload or delete, and retrying one, without
//! undoing a change that came after it.
//!
//! The feeder keeps one clip, and anyone with a key may replace it. So a
//! write first reads what the feeder holds and keeps it in the journal
//! entry ([`CallBefore`]). A retry reads it again:
//!
//! - An upload whose clip the feeder holds, stored or arriving, ends or
//!   goes on from there. One that finds the feeder still holding what the
//!   first send found starts over. Anything else is a later change (another
//!   upload, or an erase), and the entry settles as `superseded` with
//!   nothing sent.
//! - A delete that finds no clip is done. One that finds the clip the
//!   first send found erases it again under its request id; another clip
//!   is a later upload, and the entry settles as `superseded`.
//!
//! An upload whose first send erased the old clip and was then lost to a
//! restart of the feeder finds no clip where it found one first, so it is
//! superseded too: the caller uploads again under a new key.
//!
//! Each read is a read: one that fails left nothing on the feeder, so a
//! first attempt's entry is dropped and its key is free again.

use wellfed_api::{CallClip, CallView, CommandOutcome, FeederId};
use wellfed_mirror::{CallError, CallStart, Clip, UploadError, UploadFailed};
use wellfed_model::{Answer, Op};
use wellfed_store::CallBefore;

use crate::feeders::commands::refused;
use crate::feeders::journal::Attempt;
use crate::{CommandError, Error, IdempotencyKey, InternalError, RequestId, Result, Service};

/// What the feeder holds of its meal call, as a journal entry keeps it.
fn held(view: &CallView) -> CallBefore {
    match view.clip {
        CallClip::None => CallBefore::None,
        CallClip::Stored { bytes, crc } | CallClip::Receiving { bytes, crc, .. } => {
            CallBefore::Clip { bytes, crc }
        }
    }
}

/// Whether the feeder holds `clip`, stored or arriving.
fn holds(view: &CallView, clip: &Clip) -> bool {
    held(view)
        == CallBefore::Clip {
            bytes: clip.bytes(),
            crc: clip.crc(),
        }
}

/// The error for a read before a write that failed: nothing was written.
fn read_failed(error: CallError) -> Error {
    match error {
        CallError::Unsent(error) | CallError::Lost(error) => error.into(),
        CallError::Failed(UploadFailed::Upload(UploadError::Refused(refusal))) => {
            Error::Refused(refusal.into())
        }
        CallError::Failed(_) => InternalError::Unexpected.into(),
        CallError::Random(failed) => failed.into(),
    }
}

impl Service {
    /// Run one attempt of an upload on the long lane and record what came
    /// of it.
    pub(crate) async fn send_call(
        &self,
        key: IdempotencyKey,
        feeder: FeederId,
        clip: &Clip,
        attempt: Attempt,
    ) -> Result<CommandOutcome> {
        let mut operation = match self.mirror.operation(feeder).await {
            Ok(operation) => operation,
            Err(error) => return Err(self.nothing_sent(&key, attempt, error.into()).await),
        };
        let view = match operation.read_call().await {
            Ok(view) => view,
            Err(error) => return Err(self.nothing_sent(&key, attempt, read_failed(error)).await),
        };

        let start = match attempt {
            Attempt::First => {
                if let Err(error) = self
                    .shared
                    .store
                    .record_call_before(&key, held(&view))
                    .await
                {
                    return Err(self.nothing_sent(&key, attempt, error.into()).await);
                }
                CallStart::Fresh
            }
            Attempt::Retry if holds(&view, clip) => CallStart::Resume(view),
            Attempt::Retry => {
                if self.shared.store.call_before(&key).await? != Some(held(&view)) {
                    return self.superseded(&key).await;
                }
                CallStart::Fresh
            }
        };
        let uploaded = operation.upload_call(clip, start).await;
        let now = self.shared.clock.now();

        let outcome = match uploaded {
            Ok(call) => CommandOutcome::Call { call },
            Err(CallError::Failed(UploadFailed::Upload(UploadError::Refused(refusal)))) => {
                CommandOutcome::Refused {
                    refusal: refusal.into(),
                }
            }
            // As for one request: a first attempt never sent leaves no
            // entry, so the key is free for the same upload later.
            Err(CallError::Unsent(error)) => {
                return Err(self.nothing_sent(&key, attempt, error.into()).await);
            }
            Err(CallError::Failed(UploadFailed::Upload(UploadError::Unexpected))) => {
                self.unknown(&key, now).await?;
                return Err(InternalError::Unexpected.into());
            }
            // The feeder holds what it received, or its restart forgot it:
            // a retry under the key reads which and goes on from there.
            Err(CallError::Lost(_) | CallError::Failed(_)) => {
                self.unknown(&key, now).await?;
                return Err(CommandError::Unknown.into());
            }
            Err(CallError::Random(failed)) => {
                self.unknown_best_effort(&key).await;
                return Err(failed.into());
            }
        };
        self.record_answer(&key, &outcome, now).await?;
        Ok(outcome)
    }

    /// Erase the meal call, once the feeder's clip is read: as the first
    /// send found it, or on a retry, only while the feeder still holds it.
    pub(crate) async fn send_delete(
        &self,
        key: IdempotencyKey,
        feeder: FeederId,
        request: RequestId,
        attempt: Attempt,
    ) -> Result<CommandOutcome> {
        let read = self.request_id()?;
        let view = match self.mirror.ask(feeder, read, Op::GetCall).await {
            Ok(Answer::Call(view)) => view,
            Ok(other) => return Err(self.nothing_sent(&key, attempt, refused(&other)).await),
            Err(error) => return Err(self.nothing_sent(&key, attempt, error.into()).await),
        };

        match attempt {
            Attempt::First => {
                if let Err(error) = self
                    .shared
                    .store
                    .record_call_before(&key, held(&view))
                    .await
                {
                    return Err(self.nothing_sent(&key, attempt, error.into()).await);
                }
            }
            // Erased: by this delete, or by another; either way what it asked.
            Attempt::Retry if view.clip == CallClip::None => {
                let outcome = CommandOutcome::Done;
                self.record_answer(&key, &outcome, self.shared.clock.now())
                    .await?;
                return Ok(outcome);
            }
            Attempt::Retry => {
                if self.shared.store.call_before(&key).await? != Some(held(&view)) {
                    return self.superseded(&key).await;
                }
            }
        }
        let (outcome, _) = self
            .send(key, feeder, request, Op::DeleteCall, attempt)
            .await?;
        Ok(outcome)
    }

    /// Settle `key`'s entry as superseded: the meal call changed after its
    /// first send, and a resend would undo that.
    async fn superseded(&self, key: &IdempotencyKey) -> Result<CommandOutcome> {
        let outcome = CommandOutcome::Superseded;
        self.record_answer(key, &outcome, self.shared.clock.now())
            .await?;
        Ok(outcome)
    }

    /// `error` for an attempt that wrote nothing to the feeder. A first
    /// attempt's entry is dropped, so its key is free for the same write
    /// later; a retry's stays as it was.
    async fn nothing_sent(&self, key: &IdempotencyKey, attempt: Attempt, error: Error) -> Error {
        if matches!(attempt, Attempt::First)
            && let Err(forget) = self.shared.store.forget_unsent(key).await
        {
            return forget.into();
        }
        error
    }
}
