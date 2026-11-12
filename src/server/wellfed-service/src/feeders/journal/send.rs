//! One attempt of a write on the short lane, and what its answer records
//! in the journal: an outcome, unknown when none came, or unknown for good
//! when the answer settled nothing.

use tracing::debug;
use uuid::Uuid;
use wellfed_api::{CommandOutcome, FeederId};
use wellfed_mirror::AskError;
use wellfed_model::{Answer, Invite, Op, Refusal};

use super::Attempt;
use crate::{CommandError, Error, IdempotencyKey, RequestId, Result, Service};

impl Service {
    /// Send one attempt and record what came of it: the outcome, and an
    /// invite's secret beside it.
    pub(crate) async fn send(
        &self,
        key: IdempotencyKey,
        feeder: FeederId,
        request: RequestId,
        op: Op,
        attempt: Attempt,
    ) -> Result<(CommandOutcome, Option<Invite>)> {
        // A revocation resent after the first landed finds the key gone,
        // which is what it asked: its `not_found` settles it either way.
        let revokes = matches!(op, Op::RevokeClient { .. });
        let settles = Settles::of(&op);
        let answer = self.mirror.ask(feeder, request, op).await;
        let store = &self.shared.store;
        let now = self.shared.clock.now();

        let mut invite = None;
        let outcome = match answer {
            // A settling answer of another write's shape (a feed answered
            // done, say) fits no write, as below.
            Ok(ref answer @ (Answer::Done { .. } | Answer::Invite(_) | Answer::Feeding { .. }))
                if !settles.fits(answer) =>
            {
                return Err(self.unsettled(&key, now).await);
            }
            Ok(Answer::Done { .. }) => CommandOutcome::Done,
            Ok(Answer::Invite(made)) => {
                invite = Some(made);
                CommandOutcome::Invited {
                    invite: made.id,
                    seconds: made.seconds,
                }
            }
            // A resend that finds the feeder no longer remembers its id
            // runs again, and its refusal says nothing of the first send,
            // which may have landed: a settings change refused stale
            // because the first moved its base, a cancel whose feed the
            // first stopped.
            Ok(Answer::Refused(refusal))
                if matches!(attempt, Attempt::Retry)
                    && !(revokes && refusal == Refusal::NotFound) =>
            {
                return Err(self.unsettled(&key, now).await);
            }
            Ok(Answer::Refused(refusal)) => CommandOutcome::Refused {
                refusal: refusal.into(),
            },
            Ok(Answer::Feeding { feed }) => CommandOutcome::Feeding { feed },
            // An answer this server could not read (a newer feeder's), or
            // one that fits no write: what the feeder did is not known
            // from it, and a resend could only run it again.
            Ok(_) => return Err(self.unsettled(&key, now).await),
            Err(AskError::Unknown) => {
                self.unknown(&key, now).await?;
                return Err(CommandError::Unknown.into());
            }
            // Never sent: a first attempt leaves no entry, so the key is
            // free for the same write later. Only the first attempt sends
            // while the entry is `sent`, so no other attempt is using it.
            Err(
                unsent @ (AskError::NotRunning
                | AskError::Offline
                | AskError::Expired
                | AskError::Legacy
                | AskError::Unsendable),
            ) => {
                return Err(match attempt {
                    Attempt::First => {
                        store.forget_unsent(&key).await?;
                        Error::from(unsent)
                    }
                    // The entry stays unknown, since the first send may
                    // have run: this attempt sending nothing does not make
                    // it nothing.
                    Attempt::Retry => CommandError::Unknown.into(),
                });
            }
        };
        self.record_answer(&key, &outcome, now).await?;
        // The feed's `Fed` event may have been stored before its answer
        // was: look for it now rather than at the next event, beside the
        // answer rather than ahead of it.
        if outcome.feed().is_some() {
            let service = self.clone();
            self.tasks
                .spawn(async move { service.reconcile_best_effort(feeder).await });
        }
        Ok((outcome, invite))
    }

    /// Settle `key`'s entry as unknown with the feeder's answer in hand,
    /// so a retry never sends it again: the error for the caller. Should
    /// the store fail, the entry is left plainly unknown at best, which a
    /// retry may resend under the same id, as after any lost answer.
    pub(crate) async fn unsettled(&self, key: &IdempotencyKey, now: jiff::Timestamp) -> Error {
        match self.shared.store.command_unsettled(key, now).await {
            Ok(true) => self.published(key).await,
            // Another attempt answered it first, which stands.
            Ok(false) => {
                debug!(key = %Uuid::from_bytes(*key), "an unsettled write was answered already");
            }
            Err(error) => {
                self.unknown_best_effort(key).await;
                return error.into();
            }
        }
        CommandError::Unknown.into()
    }
}

/// The answer that settles a write: a feed's reservation, an invite, or
/// done for every other write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Settles {
    Feeding,
    Invite,
    Done,
}

impl Settles {
    fn of(op: &Op) -> Self {
        match op {
            Op::Feed { .. } => Self::Feeding,
            Op::CreateInvite { .. } | Op::CreateBoundInvite { .. } => Self::Invite,
            _ => Self::Done,
        }
    }

    /// Whether `answer` is the one this write is settled by.
    const fn fits(self, answer: &Answer) -> bool {
        matches!(
            (self, answer),
            (Self::Feeding, Answer::Feeding { .. })
                | (Self::Invite, Answer::Invite(_))
                | (Self::Done, Answer::Done { .. })
        )
    }
}

#[cfg(test)]
mod tests {
    use wellfed_model::ClientRole;

    use super::*;

    /// A feed is settled by its reservation alone: a feed answered done,
    /// or a lock answered with a reservation, fits no write and stays
    /// unknown.
    #[test]
    fn a_write_is_settled_only_by_its_own_answer() {
        let done = Answer::Done { generation: None };
        let feeding = Answer::Feeding { feed: 3 };

        let feed = Settles::of(&Op::Feed { portions: 1 });
        assert_eq!(feed, Settles::Feeding);
        assert!(feed.fits(&feeding));
        assert!(!feed.fits(&done));

        let lock = Settles::of(&Op::SetLock { locked: true });
        assert!(lock.fits(&done));
        assert!(!lock.fits(&feeding));

        let invite = Settles::of(&Op::CreateInvite {
            role: ClientRole::Member,
        });
        assert_eq!(invite, Settles::Invite);
        assert!(!invite.fits(&done));
    }
}
