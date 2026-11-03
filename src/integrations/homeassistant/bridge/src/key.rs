//! The integration's client key, and joining a feeder with it.
//!
//! One X25519 static key per config entry is the integration's identity to
//! its feeder. Home Assistant keeps the secret in the entry's data and hands
//! it back at each start, so a restart resumes as the same client.

use std::path::Path;

use wellfed_client::api::Invite;
use wellfed_formats::private::{self, SYSTEM_RANDOM};
use wellfed_model::{ClientRole, Joined};
use wellfed_session::noise::Keypair;

use crate::Error;

/// `N` fresh bytes from the OS generator: secrets, ephemeral keys and
/// request ids.
pub(crate) fn random<const N: usize>() -> Result<[u8; N], Error> {
    Ok(private::random(Path::new(SYSTEM_RANDOM))?)
}

/// A 32-byte value from Python's bytes, or the length it had.
pub(crate) fn bytes32(bytes: &[u8], error: fn(usize) -> Error) -> Result<[u8; 32], Error> {
    bytes.try_into().map_err(|_| error(bytes.len()))
}

pub struct ClientKey {
    secret: [u8; 32],
    keypair: Keypair,
}

impl std::fmt::Debug for ClientKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientKey")
            .field("public", &self.keypair.public())
            .finish_non_exhaustive()
    }
}

impl ClientKey {
    /// A new key, for a config entry that has none yet.
    pub fn generate() -> Result<Self, Error> {
        Ok(Self::from_secret(random()?))
    }

    /// The key whose secret the config entry kept.
    pub fn restore(secret: &[u8]) -> Result<Self, Error> {
        Ok(Self::from_secret(bytes32(secret, Error::SecretLength)?))
    }

    pub fn from_secret(secret: [u8; 32]) -> Self {
        Self {
            secret,
            keypair: Keypair::from_secret(secret),
        }
    }

    /// The secret, for the config entry only.
    pub fn secret(&self) -> [u8; 32] {
        self.secret
    }

    /// The public key, as the feeder lists this client.
    pub fn public(&self) -> [u8; 32] {
        *self.keypair.public()
    }

    pub(crate) fn keypair(&self) -> &Keypair {
        &self.keypair
    }
}

/// The identity key of the feeder `text` invites to.
pub fn invite_feeder(text: &str) -> Result<[u8; 32], Error> {
    Ok(Invite::parse(text.trim())?.feeder)
}

/// Redeem `invite` as `key`, listed on the feeder as `label`, at `address`
/// or wherever mDNS finds the invite's feeder. An invite works once, so this
/// runs once: a timeout leaves it unknown whether it was spent, and the
/// owner makes a new invite rather than this retrying.
pub async fn join(
    key: &ClientKey,
    invite: &str,
    label: &str,
    address: Option<&str>,
) -> Result<ClientRole, Error> {
    let invite = Invite::parse(invite.trim())?;
    let address = locate(&invite.feeder, address).await?;

    let joined = wellfed_connect::join(key.keypair(), invite, label, address, random()?).await?;
    match joined {
        Joined::Enrolled { role } => Ok(role),
        Joined::Refused(refusal) => Err(Error::Refused(refusal)),
    }
}

/// The feeder's address: given, or found over mDNS by its identity. The
/// browse blocks, so it runs off the session's runtime thread.
pub(crate) async fn locate(
    identity: &[u8; 32],
    address: Option<&str>,
) -> Result<std::net::SocketAddr, Error> {
    if let Some(address) = address {
        return Ok(wellfed_connect::parse_address(address)?);
    }
    let identity = *identity;
    let found = tokio::task::spawn_blocking(move || {
        wellfed_connect::locate(&identity, None, wellfed_connect::DISCOVERY)
    })
    .await
    .map_err(|_| Error::Connect(wellfed_connect::Error::Timeout))?;
    Ok(found?)
}
