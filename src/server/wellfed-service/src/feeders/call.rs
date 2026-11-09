//! The meal call: the clip a plan plays before its meal. The server reads
//! its description from the feeder, uploads a clip, erases it and plays it
//! once, each write through the journal.
//!
//! A clip comes already in the feeder's own stream (`wellfed_client::call`
//! converts a recording on the client), and the server checks only what
//! the feeder checks, its length. The upload runs on the mirror's long
//! lane, a chunk per step, so feeds and status reads keep flowing beside
//! it. How an upload or a delete is sent, and retried without undoing a
//! later change, is `call/send.rs`.

mod send;

use serde::Serialize;
use sha2::{Digest, Sha256};
use wellfed_api::{CallView, CommandKind, CommandOutcome, FeederId, Hex};
use wellfed_mirror::Clip;
use wellfed_model::{Answer, Op};

use crate::feeders::commands::{done, refused};
use crate::feeders::journal::{Sending, Write};
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
        match self.ask(id, self.request_id()?, Op::GetCall).await? {
            Answer::Call(view) => Ok(view),
            other => Err(refused(&other)),
        }
    }

    /// Upload `stream`, a clip in the feeder's own format, in place of the
    /// one the feeder holds, and answer the feeder's view once it stores
    /// it. The feeder takes it only from an owner's key (`forbidden`
    /// otherwise). A refusal is final for the key, and one after the
    /// upload began leaves the feeder with no clip. A retry finding that
    /// the meal call changed since the first send is
    /// [`CommandError::Superseded`].
    pub async fn upload_call(
        &self,
        caller: &Caller,
        id: FeederId,
        stream: Vec<u8>,
        key: Option<IdempotencyKey>,
    ) -> Result<CallView> {
        let clip = Clip::from_stream(stream).map_err(|_| Invalid::CallLength)?;
        self.live_row(id).await?;
        // Every meal call op is the feeder module's, as reading it is.
        self.serves(id, Op::GetCall.kind()).await?;
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
            CommandOutcome::Superseded => Err(CommandError::Superseded.into()),
            CommandOutcome::Done
            | CommandOutcome::Settings { .. }
            | CommandOutcome::Plans { .. }
            | CommandOutcome::Calibration { .. }
            | CommandOutcome::Feeding { .. }
            | CommandOutcome::Fed { .. }
            | CommandOutcome::Invited { .. }
            | CommandOutcome::Unrecorded => Err(InternalError::Unexpected.into()),
            // A newer server's outcome, read back after a downgrade, reads
            // as unknown.
            CommandOutcome::Unknown => Err(CommandError::Unknown.into()),
        }
    }

    /// Erase the meal call; plans then play none. Owners' keys only, on the
    /// feeder's side. A retry erases only the clip the first send found.
    pub async fn delete_call(
        &self,
        caller: &Caller,
        id: FeederId,
        key: Option<IdempotencyKey>,
    ) -> Result<()> {
        self.call_write(
            caller,
            id,
            CommandKind::CallDelete,
            Sending::DeleteCall,
            key,
        )
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
        self.call_write(caller, id, CommandKind::CallPlay, Op::PlayCall.into(), key)
            .await
    }

    async fn call_write(
        &self,
        caller: &Caller,
        id: FeederId,
        kind: CommandKind,
        op: Sending,
        key: Option<IdempotencyKey>,
    ) -> Result<()> {
        self.live_row(id).await?;

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
}
