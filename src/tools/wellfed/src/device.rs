//! A device API session for the CLI: `wellfed-connect`'s, with this
//! computer's key and the system's random source, or the bench server's
//! one session when a server runs for this checkout ([`crate::bench`]).

use std::net::SocketAddr;

use anyhow::{Context, Result};
use wellfed_connect::{AskError, Cursor, DISCOVERY};
use wellfed_formats::private;
use wellfed_model::{Answer, ClientRole, DeviceKind, Op, Pushed};
use wellfed_proto::api::RequestId;

use crate::bench::Lent;
use crate::keys::OwnerKey;

/// The feeder's address: given, or found over mDNS by its identity.
pub(crate) fn locate(identity: &[u8; 32], address: Option<&str>) -> Result<SocketAddr> {
    Ok(wellfed_connect::locate(identity, address, DISCOVERY)?)
}

/// Fresh bytes from the system's random source.
pub(crate) fn random<const N: usize>() -> Result<[u8; N]> {
    private::random().context("read the system random source")
}

/// An open session: this computer's own, or the bench server's.
pub(crate) enum Connection {
    Direct(Box<wellfed_connect::Connection>),
    Bench(Box<Lent>),
}

impl From<wellfed_connect::Connection> for Connection {
    fn from(connection: wellfed_connect::Connection) -> Self {
        Self::Direct(Box::new(connection))
    }
}

impl Connection {
    pub(crate) async fn open(
        key: &OwnerKey,
        identity: [u8; 32],
        address: SocketAddr,
    ) -> Result<Self> {
        let connection =
            wellfed_connect::Connection::open(key.keypair(), identity, address, random()?)
                .await
                .context("open a session")?;
        Ok(Self::Direct(Box::new(connection)))
    }

    pub(crate) fn build(&self) -> &str {
        match self {
            Self::Direct(connection) => connection.build(),
            Self::Bench(lent) => lent.build(),
        }
    }

    pub(crate) fn role(&self) -> ClientRole {
        match self {
            Self::Direct(connection) => connection.role(),
            Self::Bench(lent) => lent.role(),
        }
    }

    /// Which module of the device API the device serves.
    pub(crate) fn kind(&self) -> DeviceKind {
        match self {
            Self::Direct(connection) => connection.kind(),
            Self::Bench(lent) => lent.kind(),
        }
    }

    /// Events after `after`, counted in the feeder's current epoch.
    pub(crate) async fn subscribe(&mut self, after: Option<u64>, samples: bool) -> Result<()> {
        match self {
            Self::Direct(connection) => {
                let epoch = connection.epoch();
                let from = after.map(|seq| Cursor { epoch, seq });
                connection.subscribe(from, samples).await?;
                Ok(())
            }
            Self::Bench(lent) => lent.subscribe(after, samples).await,
        }
    }

    /// The next message within `wait`; `None` when none came in time.
    pub(crate) async fn next(&mut self, wait: std::time::Duration) -> Result<Option<Pushed>> {
        match self {
            Self::Direct(connection) => match connection.next(wait).await {
                Ok(message) => Ok(Some(message)),
                Err(wellfed_connect::Error::Timeout) => Ok(None),
                Err(error) => Err(error.into()),
            },
            Self::Bench(lent) => lent.next(wait).await,
        }
    }

    /// One request under a fresh id, and its answer. A write that went
    /// out and got no answer, or one this build cannot read, may have run:
    /// it comes back [uncertain](crate::uncertain), and is never sent
    /// again.
    pub(crate) async fn ask(&mut self, op: &Op) -> Result<Answer> {
        let answer = match self {
            Self::Direct(connection) => {
                let id: RequestId = random()?;
                connection
                    .ask(id, op)
                    .await
                    .map_err(|error| unanswered(op, error))?
            }
            Self::Bench(lent) => lent.ask(op).await?,
        };

        match answer {
            Answer::Unreadable if writes(op) => Err(crate::uncertain(format!(
                "the feeder answered in a way this build cannot read (a newer feeder build's); \
                 {}",
                check(op)
            ))),
            answer => Ok(answer),
        }
    }
}

/// Whether `op` changes something on the feeder: the device API's own
/// rule, which every op decides.
pub(crate) fn writes(op: &Op) -> bool {
    op.to_proto().is_ok_and(|op| op.writes())
}

/// What reads back whether `op` ran.
fn check(op: &Op) -> &'static str {
    match op {
        Op::Feed { .. } | Op::CancelFeed { .. } => "`wellfed feeds` shows whether it ran",
        _ => "`wellfed events` shows what the feeder recorded",
    }
}

/// `error`, which ended `op`'s wait for its answer, as the command reports
/// it: a write the connection says may have run is uncertain.
fn unanswered(op: &Op, error: AskError) -> anyhow::Error {
    match error {
        AskError::MayHaveRun(error) if writes(op) => {
            crate::uncertain(format!("no answer came ({error}); {}", check(op)))
        }
        error => error.into_error().into(),
    }
}

/// The shared steps (the backup's read, the update key's enrollment) run
/// over whichever session a command holds, the bench server's among them.
impl wellfed_install::session::Session for Connection {
    fn role(&self) -> ClientRole {
        Self::role(self)
    }

    fn legacy(&self) -> bool {
        match self {
            Self::Direct(connection) => connection.skew() == wellfed_model::Skew::Legacy,
            // The bench server's session says what its welcome did.
            Self::Bench(lent) => lent.skew() == wellfed_model::Skew::Legacy,
        }
    }

    async fn ask(&mut self, op: &Op) -> Result<Answer, wellfed_install::PlatformError> {
        Ok(Self::ask(self, op).await?)
    }
}

#[cfg(test)]
mod tests {
    use wellfed_connect::Error as Connect;

    use super::*;
    use crate::Uncertain;

    /// A write queued and then left unanswered may have run, so it is
    /// uncertain and says what settles it; a request refused before a
    /// byte went, and a read, are plain failures.
    #[test]
    fn a_write_with_no_answer_is_uncertain() {
        let feed = Op::Feed { portions: 1 };
        for ended in [Connect::Unanswered, Connect::Closed, Connect::Silent] {
            let error = unanswered(&feed, AskError::MayHaveRun(ended));
            assert!(error.downcast_ref::<Uncertain>().is_some(), "{error:#}");
            let words = format!("{error:#}");
            assert!(words.contains("may have run"), "{words}");
            assert!(words.contains("`wellfed feeds`"), "{words}");
        }

        for unsent in [Connect::Encode, Connect::ReservedId, Connect::Legacy] {
            let error = unanswered(&feed, AskError::Unsent(unsent));
            assert!(error.downcast_ref::<Uncertain>().is_none(), "{error:#}");
        }
        let read = unanswered(&Op::Status, AskError::MayHaveRun(Connect::Unanswered));
        assert!(read.downcast_ref::<Uncertain>().is_none(), "{read:#}");
    }
}
