//! The command surface: [`Mirror::ask`] for a short command, and an
//! [`Operation`] for a long one that sends its steps one at a time.
//!
//! Each reaches the feeder task on its own lane ([`crate::lanes`]): a
//! short command goes out even while a step is in flight, and the next step
//! waits for the short commands, so a status read or a write waits at most
//! one step. Only one long operation runs per feeder.

use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, mpsc, oneshot, watch};
use tokio::time::{Duration, Instant, timeout_at};
use wellfed_api::{Connection as Link, FeederId};
use wellfed_model::{Answer, Op};

use crate::task::LongLane;
use crate::{AskError, Mirror, Pace, Random, RequestId, Timing};

/// One request for the feeder, and where its answer goes.
pub(crate) struct Command {
    pub(crate) id: RequestId,
    pub(crate) op: Op,
    /// Still waiting for its turn at this time, it is turned away unsent.
    expires: Instant,
    reply: oneshot::Sender<Result<Answer, AskError>>,
}

/// How long a caller waits. Its command waits `queue` at most for its turn
/// on the session; once sent, the session's own answer window reports it
/// unanswered, so the caller hears every answer the session takes in.
/// `backstop` bounds the whole wait should neither come, which only a
/// fault in the task would cause.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Wait {
    queue: Duration,
    backstop: Duration,
}

impl Wait {
    pub(crate) fn of(timing: &Timing) -> Self {
        let session = timing.session;
        // The longest the session waits for one answer, a press's included,
        // and room for the turn that reports it.
        let answered = session.answer.max(session.press) + Duration::from_secs(5);
        Self {
            queue: timing.queue,
            backstop: timing.queue + answered,
        }
    }
}

/// A long operation's turn on a feeder: its steps go out one at a time on
/// the long lane, and short commands go first. Dropping it ends the
/// operation.
pub struct Operation {
    steps: mpsc::Sender<Command>,
    pub(crate) link: watch::Receiver<Link>,
    /// The build the open session's welcome names.
    pub(crate) build: watch::Receiver<Option<String>>,
    wait: Wait,
    /// How an install on this turn waits between its steps.
    pub(crate) pace: Pace,
    /// How long an install on this turn follows its build's trial.
    pub(crate) trial: Duration,
    /// Request ids for the steps an install sends.
    pub(crate) random: Arc<dyn Random>,
    _turn: OwnedSemaphorePermit,
}

impl std::fmt::Debug for Operation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Operation").finish_non_exhaustive()
    }
}

impl Operation {
    /// Send one step and wait for its answer, as [`Mirror::ask`] does.
    pub async fn step(&mut self, request: RequestId, op: Op) -> Result<Answer, AskError> {
        let link = *self.link.borrow();
        send(&self.steps, link, self.wait, request, op).await
    }
}

impl Mirror {
    /// Hand `op` to the feeder's task under `request` and wait for its
    /// answer. Turned away at once while the feeder is offline, and
    /// [`AskError::Expired`] unsent when it waits too long for its turn;
    /// once sent, [`AskError::Unknown`] when the session gets no answer.
    pub async fn ask(&self, id: FeederId, request: RequestId, op: Op) -> Result<Answer, AskError> {
        let (commands, link) = {
            let handles = self.handles();
            let handle = handles.get(&id).ok_or(AskError::NotRunning)?;
            (handle.commands.clone(), handle.link())
        };
        let wait = Wait::of(&self.shared.config.timing);
        send(&commands, link, wait, request, op).await
    }

    /// Wait for the feeder's long-operation turn. The turn lasts until the
    /// [`Operation`] is dropped.
    pub async fn operation(&self, id: FeederId) -> Result<Operation, AskError> {
        let long = {
            let handles = self.handles();
            let handle = handles.get(&id).ok_or(AskError::NotRunning)?;
            handle.long.clone()
        };
        long.operation(self.shared.config.timing, Arc::clone(&self.shared.random))
            .await
    }
}

impl LongLane {
    /// Wait for the turn, and hold it in an [`Operation`] whose steps wait
    /// as a short command does.
    pub(crate) async fn operation(
        self,
        timing: Timing,
        random: Arc<dyn Random>,
    ) -> Result<Operation, AskError> {
        let turn = self
            .turn
            .acquire_owned()
            .await
            .map_err(|_| AskError::NotRunning)?;
        Ok(Operation {
            steps: self.steps,
            link: self.link,
            build: self.build,
            wait: Wait::of(&timing),
            pace: timing.pace,
            trial: timing.trial,
            random,
            _turn: turn,
        })
    }
}

/// Queue a command and wait for its answer, as `wait` says.
pub(crate) async fn send(
    commands: &mpsc::Sender<Command>,
    link: Link,
    wait: Wait,
    request: RequestId,
    op: Op,
) -> Result<Answer, AskError> {
    match link {
        Link::Online => {}
        // `Unknown` is a newer server's state, which this one never holds.
        Link::Offline | Link::Connecting | Link::Refused | Link::Unknown => {
            return Err(AskError::Offline);
        }
    }

    let now = Instant::now();
    let expires = now + wait.queue;
    let (reply, answered) = oneshot::channel();
    let command = Command {
        id: request,
        op,
        expires,
        reply,
    };
    // Never handed over, so never sent.
    match timeout_at(expires, commands.send(command)).await {
        Ok(Ok(())) => {}
        Ok(Err(_)) => return Err(AskError::Offline),
        Err(_) => return Err(AskError::Expired),
    }
    // The task answers each command it took: with the feeder's answer, as
    // unanswered or lost once sent, or as unsent. Should it drop one
    // instead, or the backstop pass, whether it went out is not known.
    match timeout_at(now + wait.backstop, answered).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) | Err(_) => Err(AskError::Unknown),
    }
}

impl Command {
    /// Whether the caller stopped waiting: the command is then dropped
    /// unsent, so the caller's `Unknown` never turns into a late write.
    pub(crate) fn left(&self) -> bool {
        self.reply.is_closed()
    }

    /// Whether it waited past its turn: it is then turned away unsent.
    pub(crate) fn expired(&self) -> bool {
        Instant::now() >= self.expires
    }

    /// Hand the caller its result; a caller that left hears nothing.
    pub(crate) fn reply(self, result: Result<Answer, AskError>) {
        let _ = self.reply.send(result);
    }

    /// Turn the command away unsent: the task has no session.
    pub(crate) fn offline(self) {
        self.reply(Err(AskError::Offline));
    }

    /// Whether the command goes out now: one whose caller left is dropped,
    /// and one that waited past its turn is turned away, both unsent.
    pub(crate) fn still_wanted(self) -> Option<Self> {
        if self.left() {
            None
        } else if self.expired() {
            self.reply(Err(AskError::Expired));
            None
        } else {
            Some(self)
        }
    }
}
