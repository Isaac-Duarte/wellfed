//! The meal call: the clip a plan plays before its meal. The server reads
//! its description from the feeder, uploads a clip, erases it and plays it
//! once, each write through the journal.
//!
//! A clip comes already in the feeder's own stream (`wellfed_client::call`
//! converts a recording on the client), and the server checks only what
//! the feeder checks, its length. The upload runs on the mirror's long
//! lane, a chunk per step, so feeds and status reads keep flowing beside
//! it. A chunk's answer that is lost leaves the entry unknown; a retry
//! under the same key reads where the feeder's upload stands and sends
//! only the rest, or nothing once the feeder stores the clip.

use serde::Serialize;
use sha2::{Digest, Sha256};
use wellfed_api::{CallView, CommandKind, CommandOutcome, FeederId, Hex};
use wellfed_mirror::{CallError, CallStart, Clip, UploadError, UploadFailed};
use wellfed_model::{Answer, Op};

use crate::feeders::commands::{done, refused};
use crate::feeders::journal::{Attempt, Write};
use crate::{
    Caller, CommandError, Error, IdempotencyKey, InstallError, InternalError, Invalid, Result,
    Service,
};

/// An upload as it is hashed into the journal: the clip's length and CRC,
/// as the feeder knows it, and its digest, so another clip under the same
/// key is a reused key.
#[derive(Serialize)]
struct CallUpload {
    bytes: u32,
    crc: u32,
    sha256: Hex<32>,
}

impl Service {
    /// The stored clip, or the upload under way, read from the feeder.
    pub async fn call(&self, id: FeederId) -> Result<CallView> {
        self.row(id).await?;
        self.serves(id, |c| c.call)?;
        match self.ask(id, self.request_id()?, Op::GetCall).await? {
            Answer::Call(view) => Ok(view),
            other => Err(refused(&other)),
        }
    }

    /// Upload `stream`, a clip in the feeder's own format, in place of the
    /// one the feeder holds, and answer the feeder's view once it stores
    /// it. The feeder takes it only from an owner's key (`forbidden`
    /// otherwise). A refusal is final for the key, and one after the
    /// upload began leaves the feeder with no clip.
    pub async fn upload_call(
        &self,
        caller: &Caller,
        id: FeederId,
        stream: Vec<u8>,
        key: Option<IdempotencyKey>,
    ) -> Result<CallView> {
        let clip = Clip::from_stream(stream).map_err(|_| Invalid::CallLength)?;
        self.live_row(id).await?;
        self.serves(id, |c| c.call)?;
        // The long lane is the install's until it ends.
        if self.running_on(id).is_some() {
            return Err(InstallError::Running.into());
        }

        let body = CallUpload {
            bytes: clip.bytes(),
            crc: clip.crc(),
            sha256: Hex(Sha256::digest(clip.stream()).into()),
        };
        let recorded = self
            .journaled(Write {
                caller,
                feeder: id,
                kind: CommandKind::CallUpload,
                body: &body,
                key,
                op: clip,
            })
            .await?;

        match recorded.outcome {
            CommandOutcome::Call { call } => Ok(call),
            CommandOutcome::Refused { refusal } => Err(Error::Refused(refusal)),
            CommandOutcome::Done
            | CommandOutcome::Settings { .. }
            | CommandOutcome::Plans { .. }
            | CommandOutcome::Calibration { .. }
            | CommandOutcome::Feeding { .. }
            | CommandOutcome::Fed { .. }
            | CommandOutcome::Invited { .. }
            | CommandOutcome::Unrecorded => Err(InternalError::Unexpected.into()),
        }
    }

    /// Erase the meal call; plans then play none. Owners' keys only, on the
    /// feeder's side.
    pub async fn delete_call(
        &self,
        caller: &Caller,
        id: FeederId,
        key: Option<IdempotencyKey>,
    ) -> Result<()> {
        self.call_write(caller, id, CommandKind::CallDelete, Op::DeleteCall, key)
            .await
    }

    /// Play the stored clip once, to hear it: refused `muted` while sound
    /// is off, `busy` while the speaker plays, `not_found` with no clip.
    pub async fn play_call(
        &self,
        caller: &Caller,
        id: FeederId,
        key: Option<IdempotencyKey>,
    ) -> Result<()> {
        self.call_write(caller, id, CommandKind::CallPlay, Op::PlayCall, key)
            .await
    }

    async fn call_write(
        &self,
        caller: &Caller,
        id: FeederId,
        kind: CommandKind,
        op: Op,
        key: Option<IdempotencyKey>,
    ) -> Result<()> {
        self.live_row(id).await?;
        self.serves(id, |c| c.call)?;

        let recorded = self
            .journaled(Write {
                caller,
                feeder: id,
                kind,
                body: &(),
                key,
                op,
            })
            .await?;
        done(&recorded.outcome)
    }

    /// Run one attempt of an upload on the long lane and record what came
    /// of it. A retry starts from where the feeder's upload stands.
    pub(crate) async fn send_call(
        &self,
        key: IdempotencyKey,
        feeder: FeederId,
        clip: &Clip,
        attempt: Attempt,
    ) -> Result<CommandOutcome> {
        let start = match attempt {
            Attempt::First => CallStart::Fresh,
            Attempt::Retry => CallStart::Resume,
        };
        let uploaded = match self.mirror.operation(feeder).await {
            Ok(mut operation) => operation.upload_call(clip, start).await,
            Err(error) => Err(CallError::Unsent(error)),
        };
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
                if matches!(attempt, Attempt::First) {
                    self.shared.store.forget_unsent(&key).await?;
                }
                return Err(error.into());
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
}
