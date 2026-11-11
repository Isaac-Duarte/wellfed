//! Adopting a feeder this phone paired before the server: the phone makes
//! an owner's invite on the feeder with its own key, through the router
//! like any write, and hands it to `POST /feeders` naming that key, which
//! the server then records as this account's phone key, so deleting,
//! demoting or putting an expiry on the account revokes it. A claim the
//! server could not finish leaves the adoption standing, and
//! `POST /feeders/{id}/phones` finishes it; the server answers a repeated
//! claim with the record the first made.
//!
//! The adoption itself is never sent twice. An invite holds a secret the
//! server redeems once, and an adoption whose answer was lost (cut off,
//! undecodable, or a server failure that may have come after it stored the
//! feeder) may still be standing: it is settled only by reading the server's
//! list, and a second owner invite is made only once that list, read past
//! the router's grace, still lacks the feeder. The first invite then stays
//! on the feeder unredeemed until it expires, as any invite nobody used.
//! Each attempt is written down as unsettled before its invite is made and
//! cleared once the server answers, so this holds through the app stopping
//! while the adoption is out.

use wellfed_api::{AdoptFeeder, FeederId, Hex};

use crate::api::feeder::{ClientRole, FeederReply, FeederRequest};
use crate::api::router::{Adoption, NotSentReason};
use crate::api::server_error::{ServerError, ServerErrorKind};
use crate::api::server_views::ServerFeeder;
use crate::route::core::ServerFailure;
use crate::route::io::{Router, now_ms, nowhere};

fn no_server() -> ServerError {
    ServerError::new(ServerErrorKind::Closed, "no server is configured")
}

fn unlisted() -> ServerError {
    ServerError::new(
        ServerErrorKind::NotFound,
        "the server does not list it: adopting it again makes a new invite",
    )
}

fn unsettled() -> ServerError {
    ServerError::new(
        ServerErrorKind::Interrupted,
        "the server may still be adopting it: check again before adopting anew",
    )
}

impl Router {
    /// Adopt `feeder` as `name`, at `address` when the phone knows it (the
    /// server looks for it on mDNS otherwise), naming `phone`, this phone's
    /// key on it. A feeder the server already lists is only claimed, and one
    /// whose last adoption lost its answer is only read for until the grace
    /// passes.
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
        if self.adoption_lost_at(feeder).is_some() {
            match self.check_adoption(feeder, phone).await {
                // Read past the grace and still not listed: never adopted.
                Adoption::Failed { error } if error.kind == ServerErrorKind::NotFound => {}
                settled => return settled,
            }
        }

        // Written down before the invite is made, or nothing goes: an app
        // that stops while the adoption is out finds it unsettled at its
        // next start, and makes no second owner invite until the server's
        // list, read past the grace, still lacks the feeder.
        if self.guard_adoption(feeder).is_err() {
            return Adoption::NoInvite {
                reply: nowhere(NotSentReason::Store, None),
            };
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
            // No invite, so nothing could be adopted.
            self.adoption_lost(feeder, None);
            return Adoption::NoInvite { reply: asked };
        };
        let request = AdoptFeeder {
            invite: invite.text.clone(),
            name,
            address,
            phone_key: Some(Hex(phone)),
        };

        let failed = match client.adopt(&request).await {
            Ok(adopted) => {
                self.adoption_lost(feeder, None);
                let adopted = ServerFeeder::from(adopted);
                // The list routes by it from here; one that fails to read
                // is read again by the next refresh.
                let _ = self.refresh().await;
                return Adoption::Adopted { feeder: adopted };
            }
            Err(error) => error,
        };
        let lost = matches!(
            failed.failure(),
            ServerFailure::Interrupted | ServerFailure::OutcomeUnknown
        );
        let error = ServerError::from(failed);
        if lost {
            self.adoption_lost(feeder, Some(now_ms()));
            return match self.on_server(feeder).await {
                Some(id) => self.claim(feeder, id, phone).await,
                None => Adoption::Unsettled { error },
            };
        }
        // Answered: the server adopted it, or never will from this attempt.
        self.adoption_lost(feeder, None);
        match error.kind {
            // Someone adopted it first, or a lost attempt of this phone's
            // did: the list has it, and the key is claimed there.
            ServerErrorKind::AlreadyAdopted => match self.on_server(feeder).await {
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

    /// Read the server's list for `feeder`, whose adoption lost its answer:
    /// listed, its key is claimed; not listed while the server may still be
    /// adopting it, still unsettled; not listed past the grace, never
    /// adopted, and safe to adopt anew. Nothing is sent but the claim.
    pub(crate) async fn check_adoption(&self, feeder: &str, phone: [u8; 32]) -> Adoption {
        if self.server_client().is_none() {
            return Adoption::Failed { error: no_server() };
        }
        let lost_at = self.adoption_lost_at(feeder);
        match self.listed_now(feeder).await {
            Ok(Some(id)) => return self.claim(feeder, id, phone).await,
            Ok(None) => {}
            // Unread is not unlisted.
            Err(error) if lost_at.is_some() => return Adoption::Unsettled { error },
            Err(error) => return Adoption::Failed { error },
        }
        match lost_at {
            Some(at) if !self.adoption_waited(feeder, at).passed(self.grace_ms()) => {
                Adoption::Unsettled { error: unsettled() }
            }
            _ => {
                self.adoption_lost(feeder, None);
                Adoption::Failed { error: unlisted() }
            }
        }
    }

    /// Record `phone`, this phone's key on `feeder`, as this account's.
    pub(crate) async fn claim_phone(&self, feeder: &str, phone: [u8; 32]) -> Adoption {
        if self.server_client().is_none() {
            return Adoption::Failed { error: no_server() };
        }
        match self.on_server(feeder).await {
            Some(id) => self.claim(feeder, id, phone).await,
            None => Adoption::Failed { error: unlisted() },
        }
    }

    async fn claim(&self, feeder: &str, id: FeederId, phone: [u8; 32]) -> Adoption {
        let Some(client) = self.server_client() else {
            return Adoption::Failed { error: no_server() };
        };
        // Listed: whatever became of a lost attempt, it stands.
        self.adoption_lost(feeder, None);
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

    /// The server's id for `feeder`, from its list read now; `None` when it
    /// lists no such feeder, or the list could not be read.
    async fn on_server(&self, feeder: &str) -> Option<FeederId> {
        self.listed_now(feeder).await.ok().flatten()
    }

    /// The server's id for `feeder` from its list read now, or why the list
    /// could not be read.
    async fn listed_now(&self, feeder: &str) -> Result<Option<FeederId>, ServerError> {
        self.refresh().await?;
        Ok(self.listed(feeder))
    }

    /// `feeder` as the server lists it now.
    async fn listed_feeder(&self, feeder: &str) -> Option<ServerFeeder> {
        let feeders = self.refresh().await.ok()?;
        feeders.into_iter().find(|listed| listed.identity == feeder)
    }
}
