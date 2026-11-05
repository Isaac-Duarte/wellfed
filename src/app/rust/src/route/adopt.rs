//! Adopting a feeder this phone paired before the server: the phone makes
//! an owner's invite on the feeder with its own key, through the router
//! like any write, and hands it to `POST /feeders` naming that key, which
//! the server then records as this account's phone key, so deleting,
//! demoting or putting an expiry on the account revokes it. A claim the
//! server could not finish leaves the adoption standing, and
//! `POST /feeders/{id}/phones` finishes it; the server answers a repeated
//! claim with the record the first made.
//!
//! The adoption itself is not retried here: an invite holds a secret the
//! server redeems once, and an answer lost on the way back is read from the
//! server's list instead.

use wellfed_api::{AdoptFeeder, FeederId, Hex};

use crate::api::feeder::{ClientRole, FeederReply, FeederRequest};
use crate::api::router::Adoption;
use crate::api::server_error::{ServerError, ServerErrorKind};
use crate::api::server_views::ServerFeeder;
use crate::route::io::Router;

fn no_server() -> ServerError {
    ServerError::new(ServerErrorKind::Closed, "no server is configured")
}

impl Router {
    /// Adopt `feeder` as `name`, at `address` when the phone knows it (the
    /// server looks for it on mDNS otherwise), naming `phone`, this phone's
    /// key on it. A feeder the server already lists is only claimed.
    pub(crate) async fn adopt(
        &self,
        feeder: &str,
        name: String,
        address: Option<String>,
        phone: [u8; 32],
    ) -> Adoption {
        let Some(client) = self.server_client() else {
            return Adoption::Failed { error: no_server() };
        };
        if let Some(id) = self.listed(feeder) {
            return self.claim(feeder, id, phone).await;
        }

        let asked = self
            .ask(
                feeder,
                FeederRequest::CreateInvite {
                    role: ClientRole::Owner,
                },
            )
            .await;
        let FeederReply::Invite { invite } = &asked.reply else {
            return Adoption::NoInvite { reply: asked };
        };
        let request = AdoptFeeder {
            invite: invite.text.clone(),
            name,
            address,
            phone_key: Some(Hex(phone)),
        };

        let error = match client.adopt(&request).await {
            Ok(adopted) => {
                let adopted = ServerFeeder::from(adopted);
                // The list routes by it from here; one that fails to read
                // is read again by the next refresh.
                let _ = self.refresh().await;
                return Adoption::Adopted { feeder: adopted };
            }
            Err(error) => ServerError::from(error),
        };
        match error.kind {
            // The answer was lost: the server's list says whether it ran.
            ServerErrorKind::Interrupted => match self.on_server(feeder).await {
                Some(id) => self.claim(feeder, id, phone).await,
                None => Adoption::Failed { error },
            },
            // Adopted, with the claim of this phone's key left to finish.
            ServerErrorKind::PhoneKeyUnconfirmed | ServerErrorKind::PhoneKeyRefused => {
                match self.listed_feeder(feeder).await {
                    Some(listed) => Adoption::Unclaimed {
                        feeder: listed,
                        error,
                    },
                    None => Adoption::Failed { error },
                }
            }
            _ => Adoption::Failed { error },
        }
    }

    /// Record `phone`, this phone's key on `feeder`, as this account's.
    pub(crate) async fn claim_phone(&self, feeder: &str, phone: [u8; 32]) -> Adoption {
        if self.server_client().is_none() {
            return Adoption::Failed { error: no_server() };
        }
        match self.on_server(feeder).await {
            Some(id) => self.claim(feeder, id, phone).await,
            None => Adoption::Failed {
                error: ServerError::new(ServerErrorKind::NotFound, "the server does not list it"),
            },
        }
    }

    async fn claim(&self, feeder: &str, id: FeederId, phone: [u8; 32]) -> Adoption {
        let Some(client) = self.server_client() else {
            return Adoption::Failed { error: no_server() };
        };
        let claimed = client.claim_phone(id, Hex(phone)).await;

        let Some(listed) = self.listed_feeder(feeder).await else {
            return Adoption::Failed {
                error: match claimed {
                    Err(error) => error.into(),
                    Ok(_) => ServerError::new(ServerErrorKind::NotFound, "the server dropped it"),
                },
            };
        };
        match claimed {
            Ok(_) => Adoption::Adopted { feeder: listed },
            Err(error) => Adoption::Unclaimed {
                feeder: listed,
                error: error.into(),
            },
        }
    }

    /// The server's id for `feeder`, from its list read now.
    async fn on_server(&self, feeder: &str) -> Option<FeederId> {
        let _ = self.refresh().await;
        self.listed(feeder)
    }

    /// `feeder` as the server lists it now.
    async fn listed_feeder(&self, feeder: &str) -> Option<ServerFeeder> {
        let feeders = self.refresh().await.ok()?;
        feeders.into_iter().find(|listed| listed.identity == feeder)
    }
}
