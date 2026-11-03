//! The phone's session with the owner's server, as Dart holds it: a login
//! that mints this phone's own token, the reads the server's screens need,
//! and the server's live stream. Each call is one request through
//! `wellfed-server-client`, the client the CLI uses, and nothing here
//! retries: choosing the server or the feeder, and trying again, is the
//! caller's.
//!
//! The password is used once, to mint the token, and never kept. Dart keeps
//! the token's secret in secure storage beside the client secret, and the
//! [`ServerLink`] in the app's own file.

use std::time::Duration;

use flutter_rust_bridge::frb;
use tokio::sync::watch;
use wellfed_api::{
    Credentials, EventQuery, FoodPatch, IntakeQuery, NewFood, NewPet, NewToken, PetPatch,
    StreamQuery, TokenId,
};
pub use wellfed_server_client::Timeouts;
use wellfed_server_client::{Auth, Client, Url};

pub use crate::api::model::{FeedOutcome, FeedReason};
use crate::api::server_error::{ServerError, ServerErrorKind};
pub use crate::api::server_views::{
    CommandKind, CommandState, InstallState, ServerAccount, ServerConnection, ServerEntry,
    ServerEventPage, ServerFeeder, ServerFeederDetail, ServerFeederFood, ServerFood, ServerLink,
    ServerPet, ServerPetIntake, Species,
};
use crate::frb_generated::StreamSink;
use crate::server::{date, event_page, ids, stream_event};

/// How long the phone waits on a server that goes quiet, so a half-open
/// connection ends in an error instead of a screen that waits for good.
/// A stream hears a keep-alive every 15 s, so three missed is a dead link;
/// a plain call outlasts the server's own 15 s wait for a feeder's answer.
const TIMEOUTS: Timeouts = Timeouts {
    read: Some(Duration::from_secs(45)),
    request: Some(Duration::from_secs(30)),
};

/// What [`ServerSession::follow`] hears.
pub enum ServerUpdate {
    /// The stream is open: what follows is live.
    Opened,
    /// One event. `resume` is the stream's position after it, the
    /// `last_event_id` a later follow resumes from; empty when the server
    /// asked to start afresh, as with [`ServerEvent::Resync`].
    Event { resume: String, event: ServerEvent },
    /// The stream ended; `error` says why when [`ServerSession::close`] did
    /// not end it. A server that ends the stream itself (shutting down, or
    /// the token stopped holding) reads as `Interrupted`, and following
    /// again says which.
    Closed { error: Option<ServerError> },
}

/// One event of the server's stream.
pub enum ServerEvent {
    /// A feeder history entry the server just received: an event or a gap.
    History { feeder: String, entry: ServerEntry },
    /// The server's link to a feeder changed.
    Connection {
        feeder: String,
        connection: ServerConnection,
    },
    /// A write to a feeder moved on, by its idempotency key.
    Command { command: ServerCommand },
    /// An install moved on.
    Install { install: ServerInstall },
    /// The server cannot replay what this phone missed: read every list
    /// again, and resume from the empty position.
    Resync,
}

/// A write to a feeder through the server, as its log keeps it.
pub struct ServerCommand {
    /// The `Idempotency-Key` it was sent under.
    pub key: String,
    pub feeder: String,
    pub kind: CommandKind,
    pub state: CommandState,
    pub outcome: Option<ServerCommandOutcome>,
    /// The feed it started, for a feed.
    pub feed: Option<u32>,
    pub sent_ms: i64,
    pub updated_ms: i64,
}

/// How a command ended, as the feeder answered.
pub enum ServerCommandOutcome {
    Done,
    /// The settings the write left.
    Settings,
    /// The plans the write left.
    Plans,
    /// The calibration the write left.
    Calibration,
    /// The meal call the upload stored.
    Call,
    /// Another change to the meal call came first; the retry sent nothing.
    Superseded,
    Refused {
        refusal: ServerRefusal,
    },
    Invited {
        invite: u32,
        seconds: u16,
    },
    Feeding {
        feed: u32,
    },
    Fed {
        feed: u32,
        requested: u8,
        completed: u8,
        outcome: FeedOutcome,
        reason: FeedReason,
    },
    /// The answer was lost and the feeder's history never named it.
    Unrecorded,
}

/// A firmware install the server runs on a feeder.
pub struct ServerInstall {
    pub id: String,
    pub feeder: String,
    pub version: u32,
    pub state: InstallState,
    pub written: u32,
    pub image_len: u32,
    pub finished_ms: Option<i64>,
}

/// Why the feeder refused a command the server sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerRefusal {
    Forbidden,
    Busy,
    Stale,
    Unavailable,
    Invalid,
    NotWritable,
    Full,
    NotFound,
    Muted,
    Time,
    Fault,
    Portions,
    NoClock,
    Unverified,
}

#[frb(opaque)]
pub struct ServerSession {
    client: Client,
    link: ServerLink,
    secret: String,
    /// `true` once [`Self::close`] ends every follow.
    closed: watch::Sender<bool>,
}

impl ServerSession {
    /// Log in to the server at `url` as `name`, mint a token for this phone
    /// named `phone` (the phone's name, as feeders list it), and end the
    /// login: from here the token alone speaks for the phone.
    pub async fn login(
        url: String,
        name: String,
        password: String,
        phone: String,
    ) -> Result<Self, ServerError> {
        let mut client = open(&url, TIMEOUTS)?;

        let user = client.login(&Credentials { name, password }).await?;
        let created = client
            .create_token(&NewToken {
                name: phone,
                expires: None,
            })
            .await?;
        // The login's session is only the way to the token. A logout that
        // never arrives leaves it to expire on the server, and the token
        // works either way.
        let _ = client.logout().await;

        let link = ServerLink {
            url,
            user: user.name,
            token_id: created.token.id.to_string(),
        };
        Ok(Self::with_token(client, link, created.secret))
    }

    /// The session a stored link and token secret make. Nothing is sent
    /// until a call.
    #[frb(sync)]
    pub fn restore(link: ServerLink, secret: String) -> Result<Self, ServerError> {
        Self::restore_with(link, secret, TIMEOUTS)
    }

    /// [`Self::restore`] with other timeouts: what Rust tests use to see a
    /// quiet server end a call in milliseconds.
    #[frb(ignore)]
    pub fn restore_with(
        link: ServerLink,
        secret: String,
        timeouts: Timeouts,
    ) -> Result<Self, ServerError> {
        let client = open(&link.url, timeouts)?;
        Ok(Self::with_token(client, link, secret))
    }

    /// What the app's file keeps of this session.
    #[frb(sync)]
    pub fn link(&self) -> ServerLink {
        self.link.clone()
    }

    /// The token's secret, for secure storage only.
    #[frb(sync)]
    pub fn secret(&self) -> String {
        self.secret.clone()
    }

    /// The server's version, with no login: whether it answers at all.
    pub async fn health(&self) -> Result<String, ServerError> {
        Ok(self.client.health().await?.version)
    }

    /// Who this phone's token speaks for.
    pub async fn account(&self) -> Result<ServerAccount, ServerError> {
        Ok(self.client.session().await?.into())
    }

    /// Revoke this phone's token on the server and end every follow. The
    /// phone's own keys on its feeders stay: it reaches them directly. A
    /// token the server already stopped counts as revoked.
    pub async fn logout(&self) -> Result<(), ServerError> {
        self.close();

        let id = TokenId(ids::parse(&self.link.token_id)?);
        match self.client.revoke_token(id).await {
            Ok(()) => Ok(()),
            Err(error) => {
                let error = ServerError::from(error);
                match error.kind {
                    ServerErrorKind::TokenRevoked => Ok(()),
                    _ => Err(error),
                }
            }
        }
    }

    pub async fn feeders(&self) -> Result<Vec<ServerFeeder>, ServerError> {
        let feeders = self.client.feeders().await?;
        Ok(feeders.into_iter().map(ServerFeeder::from).collect())
    }

    /// One feeder with its last status and next meal.
    pub async fn feeder(&self, id: String) -> Result<ServerFeederDetail, ServerError> {
        Ok(self.client.feeder(ids::feeder(&id)?).await?.into())
    }

    /// A page of `feeder`'s history after `after` in `epoch` (the server's
    /// current epoch when `None`), at most `limit` (1-1000, default 100).
    pub async fn events(
        &self,
        feeder: String,
        epoch: Option<u32>,
        after: Option<u64>,
        limit: Option<u32>,
    ) -> Result<ServerEventPage, ServerError> {
        let query = EventQuery {
            epoch,
            after,
            limit,
            kind: None,
        };
        let page = self.client.events(ids::feeder(&feeder)?, &query).await?;
        Ok(event_page(&page))
    }

    /// The feeder's food, what is left, and how fast it goes.
    pub async fn feeder_food(&self, feeder: String) -> Result<ServerFeederFood, ServerError> {
        Ok(self.client.feeder_food(ids::feeder(&feeder)?).await?.into())
    }

    /// A pet's intake over the last `days` (1-31, default 7).
    pub async fn pet_intake(
        &self,
        pet: String,
        days: Option<u32>,
    ) -> Result<ServerPetIntake, ServerError> {
        let intake = self
            .client
            .pet_intake(ids::pet(&pet)?, &IntakeQuery { days })
            .await?;
        Ok(intake.into())
    }

    pub async fn pets(&self) -> Result<Vec<ServerPet>, ServerError> {
        let pets = self.client.pets().await?;
        Ok(pets.into_iter().map(ServerPet::from).collect())
    }

    /// Add a pet; `birth_date` is `YYYY-MM-DD`.
    pub async fn create_pet(
        &self,
        name: String,
        species: Species,
        birth_date: Option<String>,
        daily_kcal: Option<u32>,
    ) -> Result<ServerPet, ServerError> {
        let pet = NewPet {
            name,
            species,
            birth_date: birth_date.as_deref().map(date::parse).transpose()?,
            daily_kcal,
        };
        Ok(self.client.create_pet(&pet).await?.into())
    }

    /// Replace a pet's fields with these, as its edit form holds them.
    pub async fn update_pet(
        &self,
        id: String,
        name: String,
        species: Species,
        birth_date: Option<String>,
        daily_kcal: Option<u32>,
    ) -> Result<ServerPet, ServerError> {
        let patch = PetPatch {
            name: Some(name),
            species: Some(species),
            birth_date: Some(birth_date.as_deref().map(date::parse).transpose()?),
            daily_kcal: Some(daily_kcal),
        };
        Ok(self.client.update_pet(ids::pet(&id)?, &patch).await?.into())
    }

    pub async fn delete_pet(&self, id: String) -> Result<(), ServerError> {
        Ok(self.client.delete_pet(ids::pet(&id)?).await?)
    }

    pub async fn foods(&self) -> Result<Vec<ServerFood>, ServerError> {
        let foods = self.client.foods().await?;
        Ok(foods.into_iter().map(ServerFood::from).collect())
    }

    pub async fn create_food(
        &self,
        brand: String,
        name: String,
        kcal_per_kg: u32,
        bag_grams: u32,
    ) -> Result<ServerFood, ServerError> {
        let food = NewFood {
            brand,
            name,
            kcal_per_kg,
            bag_grams,
        };
        Ok(self.client.create_food(&food).await?.into())
    }

    /// Replace a food's fields with these, as its edit form holds them.
    pub async fn update_food(
        &self,
        id: String,
        brand: String,
        name: String,
        kcal_per_kg: u32,
        bag_grams: u32,
    ) -> Result<ServerFood, ServerError> {
        let patch = FoodPatch {
            brand: Some(brand),
            name: Some(name),
            kcal_per_kg: Some(kcal_per_kg),
            bag_grams: Some(bag_grams),
        };
        Ok(self
            .client
            .update_food(ids::food(&id)?, &patch)
            .await?
            .into())
    }

    /// Delete a food no feeder's history names.
    pub async fn delete_food(&self, id: String) -> Result<(), ServerError> {
        Ok(self.client.delete_food(ids::food(&id)?).await?)
    }

    /// Follow the server's live stream, after `last_event_id` when given
    /// (the `resume` of the last event heard), until it ends or
    /// [`Self::close`]: `updates` hears `Opened`, the events, and `Closed`
    /// last. The stream is not opened again here; following again from the
    /// last `resume` misses nothing the server still keeps.
    pub async fn follow(&self, last_event_id: Option<String>, updates: StreamSink<ServerUpdate>) {
        self.follow_with(last_event_id, |update| updates.add(update).is_ok())
            .await;
    }

    /// [`Self::follow`], into `emit` until it returns `false`: what Rust
    /// tests follow with, since only Dart makes a [`StreamSink`].
    #[frb(ignore)]
    pub async fn follow_with(
        &self,
        last_event_id: Option<String>,
        emit: impl FnMut(ServerUpdate) -> bool,
    ) {
        self.handle().follow(last_event_id, emit).await;
    }

    /// End every follow, now and later. The reads still work.
    #[frb(sync)]
    pub fn close(&self) {
        self.closed.send_replace(true);
    }

    fn with_token(client: Client, link: ServerLink, secret: String) -> Self {
        Self {
            client: client.with_auth(Some(Auth::Token(secret.clone()))),
            link,
            secret,
            closed: watch::Sender::new(false),
        }
    }

    /// What the path router calls the server through: this session's
    /// client, its token's id and its close.
    pub(crate) fn handle(&self) -> ServerHandle {
        ServerHandle {
            client: self.client.clone(),
            token: uuid::Uuid::parse_str(&self.link.token_id).ok(),
            closed: self.closed.subscribe(),
        }
    }
}

/// A [`ServerSession`]'s client and close, for the path router.
#[derive(Clone, Debug)]
pub(crate) struct ServerHandle {
    pub(crate) client: Client,
    /// The phone token's id, which one server issued: the router's mark of
    /// which journal a write went to. `None` for a stored id that does not
    /// parse, whose writes no journal then settles as never sent.
    pub(crate) token: Option<uuid::Uuid>,
    closed: watch::Receiver<bool>,
}

impl ServerHandle {
    /// Follow the stream, as [`ServerSession::follow_with`] does.
    pub(crate) async fn follow(
        &self,
        last_event_id: Option<String>,
        mut emit: impl FnMut(ServerUpdate) -> bool,
    ) {
        let error = self.stream(last_event_id, &mut emit).await.err();
        // Nobody listening means nobody to tell.
        emit(ServerUpdate::Closed { error });
    }

    /// The stream's events into `emit`: `Ok` once closed or nobody
    /// listens, the reason otherwise.
    async fn stream(
        &self,
        last_event_id: Option<String>,
        emit: &mut impl FnMut(ServerUpdate) -> bool,
    ) -> Result<(), ServerError> {
        let mut closed = self.closed.clone();
        if *closed.borrow() {
            return Err(ServerError::new(
                ServerErrorKind::Closed,
                "this session's stream was closed",
            ));
        }

        let query = StreamQuery::default();
        let mut stream = tokio::select! {
            opened = self.client.stream(&query, last_event_id.as_deref()) => opened?,
            _ = closed.wait_for(|closed| *closed) => return Ok(()),
        };
        if !emit(ServerUpdate::Opened) {
            return Ok(());
        }

        loop {
            let next = tokio::select! {
                next = stream.next() => next,
                _ = closed.wait_for(|closed| *closed) => return Ok(()),
            };
            let Some(update) = next else {
                return Err(ServerError::new(
                    ServerErrorKind::Interrupted,
                    "the server ended the stream",
                ));
            };
            let update = update?;

            // Samples are asked for by name, and this asks for none.
            let Some(event) = stream_event(update.event) else {
                continue;
            };
            let resume = update.id;
            if !emit(ServerUpdate::Event { resume, event }) {
                return Ok(());
            }
        }
    }
}

/// A client of the server at `url`, with no credentials yet.
fn open(url: &str, timeouts: Timeouts) -> Result<Client, ServerError> {
    let base = Url::parse(url.trim())
        .map_err(|error| ServerError::new(ServerErrorKind::Address, error))?;
    Ok(Client::with_timeouts(&base, timeouts)?)
}
