//! One feeder's session for Home Assistant, on a thread of its own.
//!
//! The thread runs a current-thread tokio runtime that opens the session,
//! reads the status, follows the feeder's events and routes the
//! integration's commands. Home Assistant's side only blocks on
//! [`Feeder::next_update`] and on each command's reply, from its executor.
//!
//! A feed or lock change goes out once, under a fresh request id, and is
//! never sent again here or by the integration: an answer that never came
//! means it may have run, so it comes back uncertain, and the
//! feeder's `Fed` or `Lock` event settles it. The event cursor survives a
//! reconnect, so events recorded while the session was down still arrive.
//! A command while no session is open is not sent: it never
//! went out, so the owner may simply press again.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use wellfed_connect::{Connection, Cursor, Received};
use wellfed_model::{Answer, Op, Pushed};

use crate::Error;
use crate::api::{Update, Written};
use crate::key::{ClientKey, locate, random};

/// The device API's request id.
type RequestId = [u8; 16];

/// How often a quiet session reads the status anew, which also keeps the
/// connection's silence limit fed.
const TURN: Duration = Duration::from_secs(30);
/// How long a lost session waits before opening another.
const RETRY: Duration = Duration::from_secs(10);

#[derive(Debug)]
enum Command {
    Write {
        op: Op,
        reply: oneshot::Sender<Written>,
    },
    Refresh,
}

/// What an outstanding request was for.
#[derive(Debug)]
enum Pending {
    Write(oneshot::Sender<Written>),
    Status,
}

/// Where the session finds its feeder.
#[derive(Clone, Debug)]
pub struct Target {
    pub identity: [u8; 32],
    /// An address, or `None` to find the feeder over mDNS at each connect.
    pub address: Option<String>,
}

/// A handle on the session thread. Dropping it ends the session.
#[derive(Debug)]
pub struct Feeder {
    commands: mpsc::UnboundedSender<Command>,
    updates: Mutex<std_mpsc::Receiver<Update>>,
}

impl Feeder {
    /// Start the session thread for `target` as `key`. It connects, and
    /// reconnects after every loss, until the handle is dropped.
    pub fn start(key: ClientKey, target: Target) -> Result<Self, Error> {
        let (commands, inbox) = mpsc::unbounded_channel();
        let (outbox, updates) = std_mpsc::channel();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(Error::Thread)?;
        std::thread::Builder::new()
            .name("wellfed-session".into())
            .spawn(move || runtime.block_on(run(key, target, inbox, outbox)))
            .map_err(Error::Thread)?;

        Ok(Self {
            commands,
            updates: Mutex::new(updates),
        })
    }

    /// The next update within `wait`, or `None`: none came, or the session
    /// thread has stopped.
    pub fn next_update(&self, wait: Duration) -> Option<Update> {
        let updates = self.updates.lock().ok()?;
        updates.recv_timeout(wait).ok()
    }

    /// Start a feed of `portions`; blocks until the feeder answers, or it is
    /// known that no answer will come.
    pub fn feed(&self, portions: u8) -> Written {
        self.write(Op::Feed { portions })
    }

    /// Lock or unlock the panel; blocks as [`Self::feed`] does.
    pub fn set_lock(&self, locked: bool) -> Written {
        self.write(Op::SetLock { locked })
    }

    /// Read the status now, rather than at the next turn.
    pub fn refresh(&self) {
        let _ = self.commands.send(Command::Refresh);
    }

    fn write(&self, op: Op) -> Written {
        let (reply, answer) = oneshot::channel();
        if self.commands.send(Command::Write { op, reply }).is_err() {
            return Written::not_sent("the session has stopped");
        }
        // The session answers every write it takes, uncertain when it ends
        // first, so a dropped reply is a session that ended mid-write.
        answer
            .blocking_recv()
            .unwrap_or_else(|_| Written::uncertain())
    }
}

/// Connect, serve until the session ends, and go again, until the handle is
/// dropped.
async fn run(
    key: ClientKey,
    target: Target,
    mut commands: mpsc::UnboundedReceiver<Command>,
    updates: std_mpsc::Sender<Update>,
) {
    let mut cursor: Option<Cursor> = None;
    loop {
        let reason = match open(&key, &target).await {
            Ok(connection) => match serve(connection, &mut cursor, &mut commands, &updates).await {
                Ended::Dropped => return,
                Ended::Lost(reason) => reason,
            },
            Err(error) => error.to_string(),
        };
        if updates.send(Update::disconnected(reason)).is_err() {
            return;
        }

        // While no session is open a write is refused at once, never held
        // for the next one: a feed sent minutes after its press is not the
        // feed the owner asked for.
        let pause = tokio::time::sleep(RETRY);
        tokio::pin!(pause);
        loop {
            tokio::select! {
                () = &mut pause => break,
                command = commands.recv() => match command {
                    None => return,
                    Some(Command::Write { reply, .. }) => {
                        let _ = reply.send(Written::not_sent("the feeder is not connected"));
                    }
                    Some(Command::Refresh) => {}
                },
            }
        }
    }
}

async fn open(key: &ClientKey, target: &Target) -> Result<Connection, Error> {
    let address: SocketAddr = locate(&target.identity, target.address.as_deref()).await?;
    Ok(Connection::open(key.keypair(), target.identity, address, random()?).await?)
}

enum Ended {
    /// The handle is gone.
    Dropped,
    Lost(String),
}

async fn serve(
    mut connection: Connection,
    cursor: &mut Option<Cursor>,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    updates: &std_mpsc::Sender<Update>,
) -> Ended {
    let send = |update: Update| updates.send(update).is_ok();
    if !send(Update::connected(connection.build(), connection.role())) {
        return Ended::Dropped;
    }

    // The status names the newest event. A first session follows from
    // there, since Home Assistant wants what happens next, not the ring's
    // backlog; a later one resumes after the last event it passed on. A
    // history that started over is a gap, not a backlog to replay.
    let id = match random() {
        Ok(id) => id,
        Err(error) => return Ended::Lost(error.to_string()),
    };
    let status = match connection.ask(id, &Op::Status).await {
        Ok(Answer::Status(status)) => status,
        Ok(other) => return Ended::Lost(format!("the feeder answered {other:?} to a status read")),
        Err(error) => return Ended::Lost(error.to_string()),
    };
    let epoch = connection.epoch();
    let from = match *cursor {
        Some(held) if held.epoch == epoch => Some(held),
        held => {
            if held.is_some() && !send(Update::gap(None)) {
                return Ended::Dropped;
            }
            status.last_event.map(|seq| Cursor { epoch, seq })
        }
    };
    if let Err(error) = connection.subscribe(from, false).await {
        return Ended::Lost(error.to_string());
    }
    *cursor = connection.cursor().or(from);
    if !send(Update::status(&status)) {
        return Ended::Dropped;
    }

    let mut pending: HashMap<RequestId, Pending> = HashMap::new();
    let ended = loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break Ended::Dropped };
                let (op, waiting) = match command {
                    Command::Write { op, reply } => (op, Pending::Write(reply)),
                    Command::Refresh => (Op::Status, Pending::Status),
                };
                let sent = random().map_err(|error| error.to_string()).and_then(|id| {
                    connection.request(id, &op).map(|()| id).map_err(|error| error.to_string())
                });
                let id = match sent {
                    Ok(id) => id,
                    Err(reason) => {
                        // Refused before any byte went out.
                        if let Pending::Write(reply) = waiting {
                            let _ = reply.send(Written::not_sent(reason));
                        }
                        continue;
                    }
                };
                pending.insert(id, waiting);
            }
            received = connection.receive(TURN) => match received {
                Ok(Received::Answer { id, answer }) => {
                    match pending.remove(&id) {
                        Some(Pending::Write(reply)) => {
                            let _ = reply.send(written(answer.as_ref()));
                        }
                        Some(Pending::Status) => {
                            if let Some(Answer::Status(status)) = answer
                                && !send(Update::status(&status))
                            {
                                break Ended::Dropped;
                            }
                        }
                        None => {}
                    }
                }
                Ok(Received::Pushed(pushed)) => {
                    *cursor = connection.cursor();
                    let update = match pushed {
                        Pushed::Event(event) => Update::event(&event),
                        Pushed::Gap(gap) => Update::gap(Some(&gap)),
                        Pushed::Sample(_) | Pushed::Log(_) => continue,
                    };
                    if !send(update) {
                        break Ended::Dropped;
                    }
                    // An event may have changed what the status shows.
                    read_status(&mut connection, &mut pending);
                }
                Err(wellfed_connect::Error::Timeout) => {
                    read_status(&mut connection, &mut pending);
                }
                Err(error) => break Ended::Lost(error.to_string()),
            },
        }
    };

    // Each write still outstanding went out and was never answered.
    for (_, waiting) in pending {
        if let Pending::Write(reply) = waiting {
            let _ = reply.send(Written::uncertain());
        }
    }
    ended
}

fn read_status(connection: &mut Connection, pending: &mut HashMap<RequestId, Pending>) {
    if pending
        .values()
        .any(|waiting| matches!(waiting, Pending::Status))
    {
        return;
    }
    // A read that cannot go out now goes at the next turn.
    let Ok(id) = random() else { return };
    if connection.request(id, &Op::Status).is_ok() {
        pending.insert(id, Pending::Status);
    }
}

fn written(answer: Option<&Answer>) -> Written {
    match answer {
        None => Written::uncertain(),
        Some(Answer::Feeding { feed }) => Written::feeding(*feed),
        Some(Answer::Done { .. }) => Written::done(),
        Some(Answer::Refused(refusal)) => Written::refused(refusal),
        // A write answered with anything else ran or not as the feeder
        // decided; this client cannot tell which.
        Some(_) => Written::uncertain(),
    }
}
