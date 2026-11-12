//! Adopting feeders, and reading them and their history as the server
//! holds it.

use std::net::SocketAddr;
use tokio::sync::oneshot;
use tracing::{info, warn};
use wellfed_api::{AdoptFeeder, Connection as Link, EventPage, EventQuery, Feeder, FeederId, Hex};
use wellfed_mirror::{Enrolled, Invite, Redeemed};
use wellfed_session::hex;
use wellfed_store::{FeederRow, NewFeeder};

use crate::feeders::keys;
use crate::feeders::phones::{Holder, Possession};
use crate::feeders::turns::Turn;
use crate::names::bounded;
use crate::{Caller, Error, FeederError, InternalError, Invalid, Resource, Result, Service};

const PAGE: u32 = 100;
const PAGE_MAX: u32 = 500;
/// Past every stored sequence number (SQLite integers are `i64`).
const NEWEST: u64 = i64::MAX.unsigned_abs();

impl Service {
    /// Adopt a feeder: redeem the invite with a new key of the server's
    /// own, store the key sealed, and start mirroring the feeder. A feeder
    /// that was removed comes back under its id, its history continuing.
    ///
    /// A phone that adopts names the key it already holds on the feeder,
    /// claimed as [`Self::claim_phone`] claims one. Whether the caller may
    /// hold a key is checked before anything is enrolled; a claim refused
    /// after enrolling leaves the adoption standing, and the phone settles
    /// it by claiming the key again.
    ///
    /// The redemption and the storing run as the service's own task, not
    /// the caller's: a caller that gives up never leaves a key enrolled
    /// with its secret dropped. A join whose answer was lost is answered
    /// [`FeederError::JoinUncertain`] at once, and the task goes on looking
    /// for the key ([`wellfed_mirror::Mirror::find_joined`]); found, the
    /// feeder is adopted as if the answer had come.
    ///
    /// One adoption or removal of a feeder runs at a time, and another
    /// adoption of it meanwhile is refused [`FeederError::Busy`] at once
    /// rather than queued behind a search. Other feeders never wait.
    pub async fn adopt(&self, caller: &Caller, request: AdoptFeeder) -> Result<Feeder> {
        let name = feeder_name(&request.name)?.to_owned();
        let invite = Invite::parse(&request.invite).map_err(FeederError::from)?;
        let phone = match request.phone_key {
            Some(key) => Some((self.direct_key_holder(caller).await?, key.0)),
            None => None,
        };
        let turn = self
            .feeders
            .adopting
            .try_take(invite.feeder)
            .ok_or(FeederError::Busy)?;

        let (answer, answered) = oneshot::channel();
        let adoption = Adoption {
            caller: caller.clone(),
            name,
            phone,
            answer,
            turn,
        };
        let service = self.clone();
        let cancel = self.cancel.child_token();
        self.tasks.spawn(async move {
            // A stop ends a search for a lost join's key too; its warning
            // was logged when the answer was lost.
            tokio::select! {
                biased;
                () = cancel.cancelled() => {}
                () = service.adopting(invite, request.address, adoption) => {}
            }
        });
        answered.await.map_err(|_| InternalError::Task)?
    }

    /// The adoption's task: redeem, then store, answering the caller as
    /// soon as the outcome is known or uncertain. It holds the feeder's
    /// turn until the feeder is stored, through a search for a lost join's
    /// key too, at most that search's window.
    async fn adopting(&self, invite: Invite, address: Option<String>, adoption: Adoption) {
        let redeemed = self.redeem(invite, address.as_deref()).await;
        let (place, redeemed) = match redeemed {
            Ok(redeemed) => redeemed,
            Err(error) => {
                let _gone = adoption.answer.send(Err(error));
                return;
            }
        };

        match redeemed {
            Redeemed::Enrolled(enrolled) => {
                let Adoption {
                    caller,
                    name,
                    phone,
                    answer,
                    turn,
                } = adoption;
                let adopted = self
                    .store_adopted(&caller, place, &name, enrolled, phone, turn)
                    .await;
                let _gone = answer.send(adopted);
            }
            Redeemed::Unanswered(lost) => {
                let Adoption {
                    caller,
                    name,
                    phone,
                    answer,
                    turn,
                } = adoption;
                let _gone = answer.send(Err(FeederError::JoinUncertain.into()));
                let place = Place {
                    secret: lost.secret(),
                    ..place
                };
                match self.mirror.find_joined(&lost).await {
                    Ok(Some(enrolled)) => {
                        let adopted = self
                            .store_adopted(&caller, place, &name, enrolled, phone, turn)
                            .await;
                        match adopted {
                            Ok(feeder) => {
                                info!(feeder = %feeder.id, "the lost join's key was enrolled: adopted")
                            }
                            Err(error) => warn!(
                                "the lost join's key was enrolled, but the adoption ended: {error}"
                            ),
                        }
                    }
                    Ok(None) => {}
                    Err(error) => warn!("the search for a lost join's key stopped: {error}"),
                }
            }
        }
    }

    /// Find the feeder and redeem its invite with a new key.
    async fn redeem(&self, invite: Invite, address: Option<&str>) -> Result<(Place, Redeemed)> {
        let archived = match self.shared.store.feeder_by_identity(&invite.feeder).await? {
            Some(id) if self.row(id).await?.archived.is_some() => Some(id),
            Some(_) => return Err(FeederError::AlreadyAdopted.into()),
            None => None,
        };
        let identity = invite.feeder;
        let address = self.mirror.locate(identity, address).await?;

        let secret = self.shared.random::<32>()?;
        let label = &self.shared.config.label;
        // The redemption's answer names the model, so no second session
        // stands between the key's enrollment and its storing.
        let redeemed = self.mirror.enroll(invite, label, address, secret).await?;
        let place = Place {
            archived,
            identity,
            address,
            secret,
        };
        Ok((place, redeemed))
    }

    /// Store the feeder `place` names under the key it `enrolled`, start
    /// mirroring it, and claim the phone's key.
    async fn store_adopted(
        &self,
        caller: &Caller,
        place: Place,
        name: &str,
        enrolled: Enrolled,
        phone: Option<(Holder, [u8; 32])>,
        turn: Turn,
    ) -> Result<Feeder> {
        // Every device belongs to the home, which setup made.
        let home = self.home().await?.id;
        let id = match place.archived {
            Some(id) => id,
            None => FeederId(self.shared.new_id()?),
        };
        let identity = place.identity;
        let nonce = self.shared.random()?;
        let sealed = keys::seal(&self.shared.config.key, id, &identity, &place.secret, nonce)?;
        let address = place.address.to_string();
        let feeder = NewFeeder {
            id,
            home,
            model: enrolled.model,
            identity,
            name,
            role: enrolled.role,
            address: Some(&address),
            key: sealed,
            created: self.shared.clock.now(),
        };
        let inserted = match place.archived {
            Some(_) => match self.shared.store.readopt_feeder(feeder).await {
                Ok(true) => Ok(()),
                Ok(false) => Err(wellfed_store::Error::Duplicate("feeder")),
                Err(error) => Err(error),
            },
            None => self.shared.store.insert_feeder(feeder).await,
        };
        if let Err(error) = inserted {
            // The feeder holds the key now, but the server cannot use it.
            warn!(
                "feeder {} enrolled this server's key ({}) but it was not stored: {error}; \
                 revoke that client on the feeder",
                hex::Lower(&identity[..4]).to_string(),
                hex::Lower(&enrolled.public[..4]).to_string(),
            );
            return Err(match error {
                wellfed_store::Error::Duplicate(_) => FeederError::AlreadyAdopted.into(),
                other => other.into(),
            });
        }

        let row = self.row(id).await?;
        self.spawn(&row).await?;
        // The feeder is stored: a removal may start while this adoption
        // waits for its session.
        drop(turn);
        if let Some((holder, key)) = phone {
            // An adoption names a key with no proof: a key a logout left
            // with no token is claimed again with one.
            self.confirm_phone(caller, holder, id, key, Possession::Unproven)
                .await?;
        }
        Ok(self.view(row))
    }

    pub async fn feeders(&self) -> Result<Vec<Feeder>> {
        let rows = self.shared.store.feeders().await?;
        Ok(rows.into_iter().map(|row| self.view(row)).collect())
    }

    /// A page of the feeder's history as the server holds it.
    pub async fn events(&self, id: FeederId, query: EventQuery) -> Result<EventPage> {
        let limit = query.limit.unwrap_or(PAGE);
        if !(1..=PAGE_MAX).contains(&limit) {
            return Err(Invalid::Limit.into());
        }
        if query.after.is_some_and(|after| after > NEWEST) {
            return Err(Invalid::Cursor.into());
        }
        let row = self.row(id).await?;
        let Some(epoch) = query.epoch.or(row.synced.map(|synced| synced.epoch)) else {
            return Ok(EventPage {
                epoch: None,
                events: Vec::new(),
                gaps: Vec::new(),
                next: None,
            });
        };

        let rows = self
            .shared
            .store
            .events(id, epoch, query.after, limit, query.kind)
            .await?;
        let full = u32::try_from(rows.len()).is_ok_and(|len| len == limit);
        // A body a newer feeder build added, which this server cannot read,
        // is kept but not listed; the page's cursor still moves past it.
        let events = rows
            .iter()
            .filter_map(
                |row| match (wellfed_mirror::stored_event(epoch, row), row.kind) {
                    (Some(event), _) => Some(Ok(event)),
                    (None, None) => None,
                    (None, Some(_)) => {
                        Some(Err(Error::Internal(InternalError::Undecodable(row.seq))))
                    }
                },
            )
            .collect::<Result<Vec<_>>>()?;

        let next = rows.last().map(|row| row.seq).filter(|_| full);
        let gaps = self
            .shared
            .store
            .gaps(id, epoch, query.after, next.unwrap_or(NEWEST))
            .await?;
        Ok(EventPage {
            epoch: Some(epoch),
            events,
            gaps,
            next,
        })
    }

    /// The feeder as the server holds it, archived or not: a removed
    /// feeder's history stays readable.
    pub(crate) async fn row(&self, id: FeederId) -> Result<FeederRow> {
        let row = self.shared.store.feeder(id).await?;
        row.ok_or_else(|| Error::NotFound(Resource::Feeder))
    }

    /// The feeder, refused [`FeederError::Archived`] once it is removed:
    /// what is sent to a feeder needs it mirrored.
    pub(crate) async fn live_row(&self, id: FeederId) -> Result<FeederRow> {
        let row = self.row(id).await?;
        if row.archived.is_some() {
            return Err(FeederError::Archived.into());
        }
        Ok(row)
    }

    pub(crate) fn view(&self, row: FeederRow) -> Feeder {
        let connection = self.links().get(&row.id).copied().unwrap_or(Link::Offline);
        Feeder {
            id: row.id,
            home: row.home,
            kind: row.kind.into(),
            model: row.model.into(),
            name: row.name,
            identity: Hex(row.identity),
            role: row.role,
            address: row.address,
            connection,
            skew: self.mirror.skew(row.id),
            synced: row.synced.map(|synced| synced.seq),
            epoch: row.synced.map(|synced| synced.epoch),
            archived: row.archived,
        }
    }
}

fn feeder_name(name: &str) -> Result<&str> {
    Ok(bounded(name, Invalid::FeederName)?)
}

/// Who asked for an adoption, and how they are answered.
struct Adoption {
    caller: Caller,
    name: String,
    phone: Option<(Holder, [u8; 32])>,
    answer: oneshot::Sender<Result<Feeder>>,
    /// The feeder's turn, held until it is stored.
    turn: Turn,
}

/// Where a redeemed invite's feeder is, and the key it was redeemed with.
struct Place {
    /// The feeder's id, when it was adopted before and removed.
    archived: Option<FeederId>,
    identity: [u8; 32],
    address: SocketAddr,
    secret: [u8; 32],
}
