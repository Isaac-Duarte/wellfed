//! Setting up the server route: signing in to the owner's server for a
//! token of Home Assistant's own, listing the feeders it holds, and
//! revoking the token again when the entry leaves the server route.
//!
//! The password is used once, to sign in: the token is created under that
//! session, the session is signed out at once, and only the token is kept.
//! An account that may change nothing (a viewer, or a role a newer server
//! added) is refused at setup and gets no token.

use std::time::Duration;

use uuid::Uuid;
use wellfed_server_client::api::{
    self as server, Credentials, ErrorCode, FeederId, Hex, NewToken, TokenId,
};
use wellfed_server_client::{Auth, Client, Error as CallError, Timeouts, Url};

use crate::api::{AccountRole, ServerProblem};

/// How long a setup step waits on the server: an owner is waiting on it.
const SETUP: Timeouts = Timeouts {
    read: Some(Duration::from_secs(15)),
    request: Some(Duration::from_secs(30)),
};

/// A sign-in that holds: Home Assistant's own token on the server.
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(frozen, get_all, module = "wellfed_homeassistant")
)]
pub struct Login {
    /// The bearer token, shown once; the config entry keeps it.
    pub token: String,
    /// Its id, to revoke it by.
    pub token_id: String,
    pub role: AccountRole,
}

impl std::fmt::Debug for Login {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The token is a secret.
        f.debug_struct("Login")
            .field("token_id", &self.token_id)
            .field("role", &self.role)
            .finish_non_exhaustive()
    }
}

/// A feeder the server holds, as the setup flow offers it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(frozen, get_all, eq, module = "wellfed_homeassistant")
)]
pub struct ServerFeeder {
    /// Its id on the server.
    pub id: String,
    pub name: String,
    /// Its identity key, which names it on either route.
    pub identity: [u8; 32],
}

/// Why a setup call to the server failed.
#[derive(Debug, thiserror::Error)]
pub enum Refused {
    #[error("not a server address: {0}")]
    Address(String),
    #[error("not a token id: {0}")]
    TokenId(String),
    #[error("not a feeder id: {0}")]
    FeederId(String),
    #[error("the account's role ({}) may change nothing", .0.key())]
    Role(AccountRole),
    #[error(transparent)]
    Call(#[from] CallError),
}

impl Refused {
    /// How the setup flow words it.
    pub fn problem(&self) -> ServerProblem {
        match self {
            Self::Address(_) => ServerProblem::NotAServer,
            Self::TokenId(_) | Self::FeederId(_) => ServerProblem::Refused,
            Self::Role(_) => ServerProblem::Forbidden,
            Self::Call(error) => problem(error),
        }
    }
}

/// Sign in to the server at `url` as `name`, and create a token labelled
/// `label` for Home Assistant, which keeps it in place of the password.
pub async fn login(url: &str, name: &str, password: &str, label: &str) -> Result<Login, Refused> {
    let mut client = setup_client(url, None)?;

    // A Wellfed server answers its health with its revision; anything else
    // at that address is not one.
    client.health().await?;
    let user = client
        .login(&Credentials {
            name: name.to_owned(),
            password: password.to_owned(),
        })
        .await?;
    let role = AccountRole::from(user.role);
    if !matches!(user.role, server::Role::Owner | server::Role::Member) {
        let _ = client.logout().await;
        return Err(Refused::Role(role));
    }

    let created = client
        .create_token(&NewToken {
            name: label.to_owned(),
            expires: None,
        })
        .await;
    // The session was only for making the token; the password is not kept.
    let _ = client.logout().await;
    let created = created?;
    Ok(Login {
        token: created.secret,
        token_id: created.token.id.to_string(),
        role,
    })
}

/// The feeders the server at `url` holds and mirrors, as `token` sees them.
pub async fn feeders(url: &str, token: &str) -> Result<Vec<ServerFeeder>, Refused> {
    let client = setup_client(url, Some(token))?;
    let feeders = client.feeders().await?;
    Ok(feeders
        .into_iter()
        .filter(|feeder| feeder.archived.is_none())
        .map(|feeder| ServerFeeder {
            id: feeder.id.to_string(),
            name: feeder.name,
            identity: feeder.identity.0,
        })
        .collect())
}

/// Revoke the token `token_id` with itself, as an entry leaving the
/// server route does.
pub async fn revoke_token(url: &str, token: &str, token_id: &str) -> Result<(), Refused> {
    let id = Uuid::parse_str(token_id).map_err(|_| Refused::TokenId(token_id.to_owned()))?;
    let client = setup_client(url, Some(token))?;
    client.revoke_token(TokenId(id)).await?;
    Ok(())
}

/// Remove the client key `client` from the feeder `feeder_id` through the
/// server, as an entry that moved to the server route drops the key it
/// held directly: a member may not remove its own key, but an owner's
/// account may, on a server the feeder made an owner. Sent once, under one
/// idempotency key; an error says it was not done or may not have been.
pub async fn revoke_client(
    url: &str,
    token: &str,
    feeder_id: &str,
    client: [u8; 32],
) -> Result<(), Refused> {
    let id = feeder(feeder_id)?;
    let client_api = setup_client(url, Some(token))?;
    client_api
        .revoke_client(id, Hex(client), Uuid::new_v4())
        .await?;
    Ok(())
}

/// Whether the feeder `feeder_id` still lists the client key `client`, as
/// the server reads its clients now.
pub async fn client_listed(
    url: &str,
    token: &str,
    feeder_id: &str,
    client: [u8; 32],
) -> Result<bool, Refused> {
    let id = feeder(feeder_id)?;
    let clients = setup_client(url, Some(token))?.clients(id).await?;
    Ok(clients.iter().any(|listed| listed.key.0 == client))
}

/// A feeder id as the server names it.
pub fn feeder(id: &str) -> Result<FeederId, Refused> {
    Uuid::parse_str(id)
        .map(FeederId)
        .map_err(|_| Refused::FeederId(id.to_owned()))
}

/// The server's address as the session's [`super::Target`] takes it.
pub fn address(url: &str) -> Result<Url, Refused> {
    Url::parse(url).map_err(|_| Refused::Address(url.to_owned()))
}

fn setup_client(url: &str, token: Option<&str>) -> Result<Client, Refused> {
    let client = Client::with_timeouts(&address(url)?, SETUP)?;
    Ok(client.with_auth(token.map(|token| Auth::Token(token.to_owned()))))
}

/// How the setup flow words a call that failed.
fn problem(error: &CallError) -> ServerProblem {
    match error {
        CallError::Http(http) if http.is_connect() || http.is_timeout() => {
            ServerProblem::Unreachable
        }
        CallError::Build(_) => ServerProblem::Unreachable,
        CallError::Base(_)
        | CallError::Http(_)
        | CallError::Decode(_)
        | CallError::Unexpected { .. }
        | CallError::NoSession => ServerProblem::NotAServer,
        CallError::Redirect { .. } => ServerProblem::Redirected,
        CallError::Problem { problem, .. } => match problem.code {
            ErrorCode::BadCredentials => ServerProblem::BadCredentials,
            ErrorCode::LoginLimited => ServerProblem::LoginLimited,
            ErrorCode::AccountDisabled => ServerProblem::AccountDisabled,
            ErrorCode::Unauthenticated => ServerProblem::Unauthorized,
            ErrorCode::Forbidden => ServerProblem::Forbidden,
            _ => ServerProblem::Refused,
        },
        CallError::Stale(_) | CallError::StalePlans(_) | CallError::StaleCalibration(_) => {
            ServerProblem::Refused
        }
    }
}
