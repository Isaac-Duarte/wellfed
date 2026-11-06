//! A device API session for the CLI: `wellfed-connect`'s, with this
//! computer's key and the system's random source, or the bench server's
//! one session when a server runs for this checkout ([`crate::bench`]).

use std::net::SocketAddr;

use anyhow::{Context, Result};
use wellfed_connect::{Cursor, DISCOVERY};
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
    private::random(OwnerKey::system()).context("read the system random source")
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

    /// One request under a fresh id, and its answer.
    pub(crate) async fn ask(&mut self, op: &Op) -> Result<Answer> {
        match self {
            Self::Direct(connection) => {
                let id: RequestId = random()?;
                Ok(connection.ask(id, op).await?)
            }
            Self::Bench(lent) => lent.ask(op).await,
        }
    }
}

/// The shared steps (the backup's read, the update key's enrollment) run
/// over whichever session a command holds, the bench server's among them.
impl wellfed_install::session::Session for Connection {
    fn role(&self) -> ClientRole {
        Self::role(self)
    }

    fn same_schema(&self) -> bool {
        match self {
            Self::Direct(connection) => connection.same_schema(),
            // The bench server holds its own session, whose shared client
            // refuses any op another build may misread, typed, so a list
            // read still ends with that reason.
            Self::Bench(_) => true,
        }
    }

    async fn ask(&mut self, op: &Op) -> Result<Answer, wellfed_install::PlatformError> {
        Ok(Self::ask(self, op).await?)
    }
}
