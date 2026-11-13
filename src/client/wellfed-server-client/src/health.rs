//! The server's liveness, identity and readiness.

use reqwest::{Method, StatusCode};
use wellfed_api::{Health, ServerIdentity, ServerSkew, ServerStatus};

use crate::{Client, Error, decode};

impl Client {
    /// Liveness and the server's version; needs no login.
    pub async fn health(&self) -> Result<Health, Error> {
        let response = self.request(Method::GET, "/health").send().await?;
        decode(response, StatusCode::OK).await
    }

    /// How the server's API revision stands against this client's, from
    /// its liveness answer: which side to update. Needs no login.
    pub async fn skew(&self) -> Result<ServerSkew, Error> {
        Ok(ServerSkew::of(self.health().await?.revision))
    }

    /// The server's identity key, as it answers it; needs no login. Over
    /// [`crate::Trust::Pinned`] the handshake already proved it.
    pub async fn identity(&self) -> Result<[u8; 32], Error> {
        let response = self.api(Method::GET, "/identity").send().await?;
        let identity: ServerIdentity = decode(response, StatusCode::OK).await?;
        Ok(identity.key.0)
    }

    /// Readiness: the database, each feeder's link and the background
    /// jobs' last runs. Owners only.
    pub async fn status(&self) -> Result<ServerStatus, Error> {
        let response = self.api(Method::GET, "/status").send().await?;
        decode(response, StatusCode::OK).await
    }
}
