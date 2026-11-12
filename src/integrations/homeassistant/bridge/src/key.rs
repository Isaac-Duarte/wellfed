//! The integration's client key, and joining a feeder with it.
//!
//! One X25519 static key per config entry is the integration's identity to
//! its feeder. Home Assistant keeps the secret in the entry's data and hands
//! it back at each start, so a restart resumes as the same client.

use wellfed_client::api::Invite;
use wellfed_formats::private;
use wellfed_model::{ClientRole, Joined as Enrollment};
use wellfed_session::noise::Keypair;

use crate::Error;

/// `N` fresh bytes from the OS generator: secrets, ephemeral keys and
/// request ids.
pub(crate) fn random<const N: usize>() -> Result<[u8; N], Error> {
    Ok(private::random()?)
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
/// runs once and never sends the join again. A join whose answer was lost
/// is settled by a session with the key, which the feeder opens only for a
/// key it enrolled: the role it was enrolled with, or [`Error::JoinNotTaken`].
/// When that session fails too, the key may be enrolled
/// ([`Error::JoinUnchecked`]), and [`check_join`] asks again later.
pub async fn join(
    key: &ClientKey,
    invite: &str,
    label: &str,
    address: Option<&str>,
) -> Result<ClientRole, Error> {
    let invite = Invite::parse(invite.trim())?;
    let feeder = invite.feeder;
    let address = locate(&feeder, address).await?;

    let joined = wellfed_connect::join(key.keypair(), invite, label, address, random()?).await;
    match joined {
        Ok(Enrollment::Enrolled { role, .. }) => Ok(role),
        Ok(Enrollment::Refused(refusal)) => Err(Error::Refused(refusal)),
        Err(wellfed_connect::Error::JoinUnanswered) => enrolled(key, &feeder, address)
            .await?
            .ok_or(Error::JoinNotTaken),
        Err(error) => Err(error.into()),
    }
}

/// The role the feeder `feeder` enrolled `key` with, by a session with it;
/// `None` when the feeder does not know the key. A session that fails
/// otherwise leaves it unchecked.
async fn enrolled(
    key: &ClientKey,
    feeder: &[u8; 32],
    address: std::net::SocketAddr,
) -> Result<Option<ClientRole>, Error> {
    let ephemeral = random()?;
    match wellfed_connect::Connection::open(key.keypair(), *feeder, address, ephemeral).await {
        Ok(session) => Ok(Some(session.role())),
        Err(wellfed_connect::Error::UnknownKey) => Ok(None),
        Err(error) => Err(Error::JoinUnchecked(error)),
    }
}

/// How a join that ended unchecked ([`Error::JoinUnchecked`]) ended after
/// all, asked again by a session with `key`: as [`join_member`] reports it
/// once enrolled, or `None` when the feeder does not know the key and a new
/// invite is needed. Still unchecked when no session can tell.
pub async fn check_join(
    key: &ClientKey,
    feeder: &[u8; 32],
    address: Option<&str>,
) -> Result<Option<Joined>, Error> {
    let located = match locate(feeder, address).await {
        Ok(located) => located,
        Err(Error::Connect(error)) => return Err(Error::JoinUnchecked(error)),
        Err(error) => return Err(error),
    };
    match enrolled(key, feeder, located).await? {
        Some(role) => Ok(Some(member_only(key, feeder, address, role).await)),
        None => Ok(None),
    }
}

/// A key a join that ended unchecked may have enrolled, on the feeder it
/// was for: the setup flow holds it, and checks it before it spends another
/// invite for that feeder.
#[derive(Clone, Copy)]
pub struct Unsettled {
    pub secret: [u8; 32],
    pub feeder: [u8; 32],
}

impl std::fmt::Debug for Unsettled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The secret is a secret.
        f.debug_struct("Unsettled")
            .field("feeder", &self.feeder)
            .finish_non_exhaustive()
    }
}

/// A key an invite enrolled, kept only as a member ([`join_member`]).
#[derive(Debug)]
pub struct Redeemed {
    pub key: ClientKey,
    pub feeder: [u8; 32],
    pub joined: Joined,
    /// A key held from an earlier unchecked join for another feeder, still
    /// held: this invite said nothing of it.
    pub held: Option<Unsettled>,
}

/// An invite that did not end enrolled, and the key still held after it.
#[derive(Debug, thiserror::Error)]
#[error("the invite did not enroll a key")]
pub struct Unredeemed {
    #[source]
    pub error: Error,
    /// The key a later join must check first: the one this join may have
    /// enrolled when it ended unchecked, or the one held before when this
    /// invite said nothing of it.
    pub held: Option<Unsettled>,
}

/// Join with `invite`, listed as `label`, at `address` or wherever mDNS finds
/// its feeder, keeping the key only as a member. `held` is a key an earlier
/// unchecked join may have enrolled: when it is for this invite's feeder it
/// is checked first, and the invite is spent only when the feeder does not
/// know it, since an invite works once. A join that ends unchecked holds
/// its own new key in turn.
pub async fn redeem(
    invite: &str,
    label: &str,
    address: Option<&str>,
    held: Option<Unsettled>,
) -> Result<Redeemed, Box<Unredeemed>> {
    let feeder = invite_feeder(invite).map_err(|error| Box::new(Unredeemed { error, held }))?;
    let (ours, other) = match held {
        Some(held) if held.feeder == feeder => (Some(held), None),
        held => (None, held),
    };

    if let Some(held) = ours {
        let key = ClientKey::from_secret(held.secret);
        match check_join(&key, &feeder, address).await {
            Ok(Some(joined)) => {
                return Ok(Redeemed {
                    key,
                    feeder,
                    joined,
                    held: other,
                });
            }
            // The feeder does not know it: the invite is still unspent.
            Ok(None) => {}
            Err(error) => {
                let held = matches!(error, Error::JoinUnchecked(_)).then_some(held);
                return Err(Box::new(Unredeemed { error, held }));
            }
        }
    }

    let key = ClientKey::generate().map_err(|error| Box::new(Unredeemed { error, held: other }))?;
    match join_member(&key, invite, label, address).await {
        Ok(joined) => Ok(Redeemed {
            key,
            feeder,
            joined,
            held: other,
        }),
        Err(error) => {
            let unchecked = matches!(error, Error::JoinUnchecked(_)).then_some(Unsettled {
                secret: key.secret(),
                feeder,
            });
            Err(Box::new(Unredeemed {
                error,
                held: unchecked.or(other),
            }))
        }
    }
}

/// Why a join did not end enrolled, as the setup flow words it.
pub fn join_problem(error: &Error) -> crate::api::JoinProblem {
    use crate::api::JoinProblem;
    match error {
        Error::JoinUnchecked(_) => JoinProblem::Uncertain,
        // The feeder drops an invite it does not hold (unknown, expired or
        // spent) unanswered, before the join goes.
        Error::JoinNotTaken
        | Error::Refused(_)
        | Error::Invite(_)
        | Error::Connect(wellfed_connect::Error::Closed) => JoinProblem::NotAccepted,
        // Not found, not answering, or failing before the join went:
        // nothing was enrolled.
        _ => JoinProblem::Unreachable,
    }
}

/// How a member join ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(frozen, get_all, eq, module = "wellfed_homeassistant")
)]
pub struct Joined {
    /// The role the invite enrolled the key with.
    pub role: crate::api::Role,
    /// For any role but member: whether the key was removed from the
    /// feeder again. When it was not, a key with that role stays listed
    /// (as the label it joined with) until someone revokes it.
    pub removed: bool,
}

/// Redeem `invite` as [`join`] does, keeping the key only as a member.
/// Home Assistant is a member and nothing more: a key an owner invite
/// enrolled could change the Wi-Fi, remove clients and install firmware
/// whatever Home Assistant shows, and an invite does not say its role until
/// it is redeemed. So a key enrolled as anything else revokes itself at once,
/// with the owner's right it was just given.
pub async fn join_member(
    key: &ClientKey,
    invite: &str,
    label: &str,
    address: Option<&str>,
) -> Result<Joined, Error> {
    let feeder = invite_feeder(invite)?;
    let role = join(key, invite, label, address).await?;
    Ok(member_only(key, &feeder, address, role).await)
}

/// A key the feeder `feeder` enrolled with `role`, kept only as a member:
/// any other role revokes itself at once.
async fn member_only(
    key: &ClientKey,
    feeder: &[u8; 32],
    address: Option<&str>,
    role: ClientRole,
) -> Joined {
    if role == ClientRole::Member {
        return Joined {
            role: role.into(),
            removed: false,
        };
    }

    let removed = revoke_self(key, feeder, address).await.unwrap_or(false);
    Joined {
        role: role.into(),
        removed,
    }
}

/// Revoke `key` on the feeder `feeder`, as the key itself; whether it is
/// gone. The revocation closes the key's own sessions, the asking one among
/// them, so its answer may never come: what settles it is a new session,
/// which the feeder refuses once the key is unknown.
async fn revoke_self(
    key: &ClientKey,
    feeder: &[u8; 32],
    address: Option<&str>,
) -> Result<bool, Error> {
    let address = locate(feeder, address).await?;
    let open = || async {
        let ephemeral = random()?;
        Ok::<_, Error>(
            wellfed_connect::Connection::open(key.keypair(), *feeder, address, ephemeral).await,
        )
    };

    let mut connection = open().await??;
    let revoke = wellfed_model::Op::RevokeClient { key: key.public() };
    // Answered or not, the next open says whether it took.
    let _ = connection.ask(random()?, &revoke).await;
    drop(connection);

    match open().await? {
        Err(wellfed_connect::Error::UnknownKey) => Ok(true),
        Ok(_) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// Whether the feeder `feeder` still lists the client key `listed`, as
/// `key`, one of its clients, reads them now; `None` when no session could
/// ask. Any client may list the clients, a member too, so Home Assistant
/// checks a key it left behind on the direct route as well.
pub async fn client_listed(
    key: &ClientKey,
    feeder: &[u8; 32],
    address: Option<&str>,
    listed: [u8; 32],
) -> Option<bool> {
    let address = locate(feeder, address).await.ok()?;
    let ephemeral = random().ok()?;
    let opened = wellfed_connect::Connection::open(key.keypair(), *feeder, address, ephemeral);
    let mut session = opened.await.ok()?;
    let asked = session.ask(random().ok()?, &wellfed_model::Op::ListClients);
    match asked.await {
        Ok(wellfed_model::Answer::Clients(clients)) => {
            Some(clients.iter().any(|client| client.key.0 == listed))
        }
        _ => None,
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
