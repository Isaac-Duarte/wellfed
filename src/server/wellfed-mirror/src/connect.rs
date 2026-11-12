//! Reaching a feeder: at its stored address, or wherever mDNS finds it now,
//! for a task's session or to redeem an invite.

use std::net::SocketAddr;
use std::time::Duration;

use tracing::{debug, warn};
use wellfed_api::FeederId;
use wellfed_connect::Connection;
use wellfed_model::{ClientRole, Joined, Model};
use wellfed_session::hex::Lower;
use wellfed_session::noise::Keypair;

use crate::{ConnectError, Discovery, Invite, Mirror, Shared, Unreachable};

/// A key the feeder enrolled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Enrolled {
    pub role: ClientRole,
    /// The device's model, as the redemption's answer names it.
    pub model: Model,
    /// The enrolled key's public half, as the feeder lists it.
    pub public: [u8; 32],
}

impl Mirror {
    /// Where to reach a feeder: `address` when one is given, or the feeder
    /// found over mDNS.
    pub async fn locate(
        &self,
        identity: [u8; 32],
        address: Option<&str>,
    ) -> Result<SocketAddr, ConnectError> {
        match address.map(str::trim).filter(|address| !address.is_empty()) {
            Some(address) => wellfed_connect::locate(&identity, Some(address), Duration::ZERO)
                .map_err(|_| ConnectError::Address),
            None if self.shared.config.discovery == Discovery::Off => {
                Err(ConnectError::AddressRequired)
            }
            None => discover(identity).await,
        }
    }

    /// Redeem `invite` at `address` with the key `secret`, listed on the
    /// feeder as `label`.
    pub async fn enroll(
        &self,
        invite: Invite,
        label: &str,
        address: SocketAddr,
        secret: [u8; 32],
    ) -> Result<Enrolled, ConnectError> {
        let keys = Keypair::from_secret(secret);
        let ephemeral = self.shared.random()?;
        let joined = match wellfed_connect::join(&keys, invite, label, address, ephemeral).await {
            Ok(joined) => joined,
            // The feeder drops an unknown or expired invite unanswered.
            Err(wellfed_connect::Error::Closed) => return Err(ConnectError::InviteExpired),
            // The join went and its answer was lost: the key may be
            // enrolled, holding a client slot with nothing to use it unless
            // it is kept. A session with it says.
            Err(wellfed_connect::Error::JoinUnanswered) => {
                self.joined_unanswered(&keys, invite.feeder, address)
                    .await?
            }
            Err(other) => return Err(Unreachable(other).into()),
        };
        match joined {
            Joined::Enrolled { role, model } => Ok(Enrolled {
                role,
                model,
                public: *keys.public(),
            }),
            Joined::Refused(refusal) => Err(ConnectError::Refused(refusal)),
        }
    }
}

impl Mirror {
    /// Whether a join whose answer was lost enrolled `keys`: a session
    /// with them opens only if it did, and names the role and model the
    /// answer would have. When the session fails for another reason the
    /// key may be enrolled all the same, and the log names it to revoke.
    async fn joined_unanswered(
        &self,
        keys: &Keypair,
        identity: [u8; 32],
        address: SocketAddr,
    ) -> Result<Joined, ConnectError> {
        let ephemeral = self.shared.random()?;
        let timing = self.shared.config.timing.session;
        match Connection::open_with(keys, identity, address, ephemeral, timing).await {
            Ok(session) => Ok(Joined::Enrolled {
                role: session.role(),
                model: session.model(),
            }),
            // Not enrolled: the invite may be spent, and another is needed.
            Err(error @ wellfed_connect::Error::UnknownKey) => Err(Unreachable(error).into()),
            Err(error) => {
                warn!(
                    "feeder {} took this server's join and its answer was lost, and a session \
                     to check failed: {error}; it may hold this server's key ({}), which \
                     nothing uses: revoke that client on the feeder",
                    Lower(&identity[..4]),
                    Lower(&keys.public()[..4]),
                );
                Err(Unreachable(wellfed_connect::Error::JoinUnanswered).into())
            }
        }
    }
}

/// Open a session at the stored address, or, when that fails and discovery
/// is on, wherever mDNS finds the feeder now.
pub(crate) async fn open(
    shared: &Shared,
    id: FeederId,
    identity: [u8; 32],
    keys: &Keypair,
    address: Option<&str>,
) -> Result<(Connection, SocketAddr), ConnectError> {
    let stored = address
        .and_then(|address| wellfed_connect::locate(&identity, Some(address), Duration::ZERO).ok());
    if let Some(address) = stored {
        let ephemeral = shared.random()?;
        match Connection::open_with(
            keys,
            identity,
            address,
            ephemeral,
            shared.config.timing.session,
        )
        .await
        {
            Ok(connection) => return Ok((connection, address)),
            // The feeder answered: looking elsewhere would not help.
            Err(
                error @ (wellfed_connect::Error::UnknownKey
                | wellfed_connect::Error::Version(_)
                | wellfed_connect::Error::Full),
            ) => return Err(Unreachable(error).into()),
            Err(error) if shared.config.discovery == Discovery::Off => {
                return Err(Unreachable(error).into());
            }
            Err(error) => debug!("{address}: {error}; looking over mDNS"),
        }
    } else if shared.config.discovery == Discovery::Off {
        let address = address.unwrap_or_default().to_owned();
        return Err(Unreachable(wellfed_connect::Error::Address(address)).into());
    }

    let found = discover(identity).await?;
    if let Err(error) = shared.store.set_address(id, &found.to_string()).await {
        warn!("remember the address {found}: {error}");
    }
    let ephemeral = shared.random()?;
    let connection = Connection::open_with(
        keys,
        identity,
        found,
        ephemeral,
        shared.config.timing.session,
    )
    .await
    .map_err(Unreachable)?;
    Ok((connection, found))
}

/// The feeder's address over mDNS, which blocks a thread while it browses.
async fn discover(identity: [u8; 32]) -> Result<SocketAddr, ConnectError> {
    tokio::task::spawn_blocking(move || {
        wellfed_connect::locate(&identity, None, wellfed_connect::DISCOVERY)
    })
    .await
    .map_err(|_| ConnectError::Lookup)?
    .map_err(|error| Unreachable(error).into())
}
