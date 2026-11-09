//! The alert reader: each feeder's alert table, read over the server's own
//! session and kept ([`crate::kept`]).
//!
//! It reads the status, and the table only when the status shows the
//! table's cursor (epoch, boot, generation) moved since the last one
//! applied, as a phone's catch-up does. It looks at each connect, a moment
//! after each new event (an event is what opens most reports), and every
//! [`AlertReads::Read`]'s `poll` while the feeder is online, since a condition
//! opens or ends with no event. Both reads go on the short lane: they are
//! single frames, answered at once, and a status read is what a short
//! command is for. Reads change nothing on the feeder.
//!
//! It also judges the feeder's reach by the phone's rule
//! ([`wellfed_client::alerts`]), from what each connect came to
//! ([`Attempt`]): a connect nothing answered counts, and after
//! [`UNREACHABLE_AFTER`] of them in a row over [`UNREACHABLE_FOR`] the
//! feeder is unreachable. A feeder that answered resets it, even to say
//! every session is taken or to welcome another model: it is there. A
//! feeder that no longer knows the server's key is unreachable at once,
//! since the server will not read it again until the owner adopts it
//! anew: the table it served last stays, as old as its read time says.
//!
//! [`AlertReads::Read`]: crate::AlertReads::Read
//! [`UNREACHABLE_AFTER`]: wellfed_client::alerts::UNREACHABLE_AFTER
//! [`UNREACHABLE_FOR`]: wellfed_client::alerts::UNREACHABLE_FOR

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};
use wellfed_api::{Connection as Link, FeederId};
use wellfed_client::alerts::{Context, Cursor, FeederClock, Mutes};
use wellfed_model::{Answer, Hex, Op, Skew};

use crate::command::{Command, send};
use crate::kept::{self, Kept, ReadTable};
use crate::task::Opened;
use crate::{ConnectError, MirrorEvent, Shared, Unreachable};

/// Whether each feeder's alert table is read and kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertReads {
    /// At each connect, a moment after each event, and every `poll` while
    /// the feeder is online.
    Read {
        poll: Duration,
    },
    Off,
}

impl Default for AlertReads {
    fn default() -> Self {
        // A condition such as food running low opens with no event: a
        // minute is soon enough for an owner, and one status frame a
        // minute costs the feeder nothing.
        Self::Read {
            poll: Duration::from_secs(60),
        }
    }
}

/// How long after a new event the table is read: long enough for the
/// feeder's rules to have seen it, and for a burst of events to be one read.
const AFTER_EVENT: Duration = Duration::from_secs(2);

/// What the server's watch calls the network it reaches every feeder on.
/// The phone's rule counts only failures on the feeder's home network; the
/// server has one, its own.
const SERVER_NETWORK: &str = "server";

/// What one connect to the feeder came to, as the connect loop tells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Attempt {
    /// The feeder answered: a session, or its own refusal of one.
    Answered,
    /// The feeder no longer knows the server's key.
    ShutOut,
    /// Nothing answered.
    Failed,
}

impl Attempt {
    /// What a connect came to, or `None` when it says nothing of the
    /// feeder (the server's own random source failed).
    pub(crate) const fn of<T>(opened: &Result<T, ConnectError>) -> Option<Self> {
        match opened {
            Ok(_) => Some(Self::Answered),
            Err(ConnectError::Unreachable(Unreachable(wellfed_connect::Error::UnknownKey))) => {
                Some(Self::ShutOut)
            }
            // The feeder answered, to turn this session away.
            Err(
                ConnectError::Unreachable(Unreachable(
                    wellfed_connect::Error::Full | wellfed_connect::Error::Version(_),
                ))
                | ConnectError::Refused(_)
                | ConnectError::InviteExpired,
            ) => Some(Self::Answered),
            Err(ConnectError::Random(_)) => None,
            Err(
                ConnectError::Unreachable(_)
                | ConnectError::Address
                | ConnectError::AddressRequired
                | ConnectError::Lookup,
            ) => Some(Self::Failed),
        }
    }
}

/// The two reads the reader may make: nothing it sends changes the feeder.
#[derive(Clone, Copy, Debug)]
enum Read {
    Status,
    Alerts,
}

impl Read {
    const fn op(self) -> Op {
        match self {
            Self::Status => Op::Status,
            Self::Alerts => Op::GetAlerts,
        }
    }
}

pub(crate) struct Reader {
    pub(crate) shared: Arc<Shared>,
    pub(crate) id: FeederId,
    /// The server's fingerprint on the feeder: its own joining is kept
    /// quiet, as a phone keeps its own.
    pub(crate) own: [u8; 4],
    pub(crate) commands: mpsc::Sender<Command>,
    pub(crate) link: watch::Receiver<Link>,
    pub(crate) opened: watch::Receiver<Option<Opened>>,
    pub(crate) attempts: mpsc::UnboundedReceiver<Attempt>,
    pub(crate) updates: broadcast::Receiver<MirrorEvent>,
    pub(crate) publish: broadcast::Sender<MirrorEvent>,
    pub(crate) cancel: CancellationToken,
    pub(crate) poll: Duration,
}

impl Reader {
    pub(crate) async fn run(mut self) {
        let poll = self.poll;
        let mut due: Option<Instant> = None;
        let mut next_poll = Instant::now() + poll;
        loop {
            let online = *self.link.borrow() == Link::Online;
            let update = tokio::select! {
                biased;
                () = self.cancel.cancelled() => return,
                () = sleep_until_some(due), if due.is_some() && online => {
                    due = None;
                    next_poll = Instant::now() + poll;
                    self.read().await;
                    continue;
                }
                () = tokio::time::sleep_until(next_poll), if online => {
                    next_poll = Instant::now() + poll;
                    self.read().await;
                    continue;
                }
                attempt = self.attempts.recv() => {
                    match attempt {
                        Some(attempt) => self.reach(attempt).await,
                        None => return,
                    }
                    continue;
                }
                update = self.updates.recv() => update,
            };
            match update {
                Ok(MirrorEvent::Link(Link::Online)) => due = Some(Instant::now()),
                Ok(MirrorEvent::Link(_)) => {}
                Ok(MirrorEvent::Event(_) | MirrorEvent::Gap(_)) => {
                    let soon = Instant::now() + AFTER_EVENT;
                    due = Some(due.map_or(soon, |due| due.min(soon)));
                }
                Ok(MirrorEvent::Sample(_) | MirrorEvent::Alerts) => {}
                Err(RecvError::Lagged(_)) => due = Some(Instant::now()),
                Err(RecvError::Closed) => return,
            }
        }
    }

    /// Read the status and, when its cursor moved, the table, and keep
    /// what they show. A read that goes unanswered is left to the next.
    async fn read(&mut self) {
        let Some(opened) = *self.opened.borrow() else {
            return;
        };
        if self.shared.desk.held(self.id) {
            return;
        }
        // A pre-revision build takes only an update.
        if opened.skew == Skew::Legacy {
            return;
        }
        let Some(Answer::Status(status)) = self.ask(Read::Status).await else {
            return;
        };
        let cursor = Cursor {
            epoch: opened.epoch,
            boot: status.boot_count,
            generation: status.alerts_generation,
        };
        let clock = match status.clock {
            Some(_) => FeederClock::Known,
            None => FeederClock::Unknown,
        };

        let kept = match kept::load(&self.shared.store, self.id).await {
            Ok(kept) => kept,
            Err(error) => {
                warn!("read the kept alerts: {error}");
                return;
            }
        };
        if !kept.watch.needs_read(cursor) {
            self.shared.desk.check(self.id, self.shared.clock.now());
            self.keep(clock, |kept, context| kept.watch.lapse(context))
                .await;
            return;
        }

        let Some(Answer::Alerts(view)) = self.ask(Read::Alerts).await else {
            return;
        };
        // The table may have moved since the status: its own generation is
        // the one it was read under.
        let cursor = Cursor {
            epoch: view.epoch,
            generation: view.generation,
            ..cursor
        };
        let at = self.shared.clock.now();
        self.keep(clock, |kept, context| {
            let table = ReadTable {
                cursor,
                view: view.clone(),
                clock_valid: clock == FeederClock::Known,
                at,
            };
            kept.take(table, context);
        })
        .await;
        self.shared.desk.check(self.id, at);
        debug!(generation = %cursor.generation, alerts = view.alerts.len(), "alert table read");
    }

    /// Count a connect towards the feeder's reach.
    async fn reach(&self, attempt: Attempt) {
        if self.shared.desk.held(self.id) {
            return;
        }
        self.keep(FeederClock::Unknown, |kept, context| {
            let watch = &mut kept.watch;
            match attempt {
                Attempt::Answered => watch.reached(Some(SERVER_NETWORK.to_owned()), context.now),
                Attempt::ShutOut => {
                    let since = watch.reach.first_failure.unwrap_or(context.now);
                    watch.relayed_reach(Some(since), context);
                }
                Attempt::Failed => {
                    // A feeder never reached since this server kept its
                    // alerts was still reached once, at its adoption.
                    watch
                        .reach
                        .home
                        .get_or_insert_with(|| SERVER_NETWORK.to_owned());
                    watch.unanswered(Some(SERVER_NETWORK), context);
                }
            }
        })
        .await;
    }

    /// Change the kept alerts with `apply`, then tell the stream when what
    /// the API shows changed, and the webhook when there is anything to
    /// deliver.
    async fn keep(&self, clock: FeederClock, apply: impl Fn(&mut Kept, &Context<'_>)) {
        let now = self.shared.clock.now();
        let mutes = Mutes::default();
        let context = Context {
            now,
            mutes: &mutes,
            own: Hex(self.own),
            clock,
        };
        let changed = kept::change(&self.shared.store, self.id, now, |kept| {
            let before = kept.body();
            apply(kept, &context);
            (kept.body() != before, !kept.watch.changes().is_empty())
        })
        .await;
        match changed {
            Ok((shown, due)) => {
                if shown {
                    // No subscriber is fine.
                    let _ = self.publish.send(MirrorEvent::Alerts);
                }
                if due {
                    self.shared.alerts_moved.send_modify(|count| {
                        *count = count.wrapping_add(1);
                    });
                }
            }
            Err(error) => warn!("keep the alerts: {error}"),
        }
    }

    async fn ask(&self, read: Read) -> Option<Answer> {
        let request = self.shared.random().ok()?;
        let link = *self.link.borrow();
        let answer = self.shared.config.timing.answer;
        match send(&self.commands, link, answer, request, read.op()).await {
            Ok(Answer::Refused(refusal)) => {
                debug!(?read, "the feeder refused the read: {refusal:?}");
                None
            }
            Ok(answer) => Some(answer),
            Err(error) => {
                debug!(?read, "read not answered: {error}");
                None
            }
        }
    }
}

/// Sleep until `at`; `None` is never selected (its branch is disabled).
async fn sleep_until_some(at: Option<Instant>) {
    if let Some(at) = at {
        tokio::time::sleep_until(at).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(error: wellfed_connect::Error) -> Option<Attempt> {
        Attempt::of::<()>(&Err(ConnectError::Unreachable(Unreachable(error))))
    }

    #[test]
    fn only_a_connect_nothing_answered_counts_against_the_feeder() {
        assert_eq!(Attempt::of::<()>(&Ok(())), Some(Attempt::Answered));
        // Every session taken: the feeder is there.
        assert_eq!(of(wellfed_connect::Error::Full), Some(Attempt::Answered));
        assert_eq!(
            of(wellfed_connect::Error::UnknownKey),
            Some(Attempt::ShutOut)
        );
        assert_eq!(of(wellfed_connect::Error::Timeout), Some(Attempt::Failed));
        assert_eq!(
            Attempt::of::<()>(&Err(ConnectError::Lookup)),
            Some(Attempt::Failed)
        );
    }
}
