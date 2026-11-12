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
//!
//! On the home network the phone reaches the server only over the server's
//! own HTTPS, and pins its identity key ([server identity]): the key mDNS
//! announced, or for a typed address the key the first handshake proved.
//! Every later connection must prove that key, or ends as
//! [`ServerErrorKind::KeyChanged`] before anything is sent. Plain `http://`
//! is taken only on this device itself. A server reached by a public name
//! is checked by the system's authorities, and its key read from
//! `GET /identity` at each login.
//!
//! [server identity]: ../../../../../docs/server/README.md#server-identity

use std::time::Duration;

use flutter_rust_bridge::frb;
use tokio::sync::watch;
use wellfed_api::{
    Credentials, EventQuery, FoodPatch, Hex, IntakeQuery, LogOut, NewFood, NewPet, NewToken,
    NewWeight, PageQuery, PetPatch, StreamQuery, TokenId, at_home,
};
pub use wellfed_server_client::Timeouts;
use wellfed_server_client::{Auth, Client, Options, PlainHttp, Trust, Url};

use crate::api::keys::ClientKey;
pub use crate::api::model::{FeedOutcome, FeedReason};
use crate::api::router::Settlement;
use crate::api::server_error::{ServerError, ServerErrorKind};
pub use crate::api::server_views::{
    CommandKind, CommandState, InstallState, NewRefill, ServerAccount, ServerConnection,
    ServerEntry, ServerEventPage, ServerFeeder, ServerFeederDetail, ServerFeederFood, ServerFood,
    ServerFoodPeriod, ServerLink, ServerPet, ServerPetIntake, ServerRefill, ServerRefillPage,
    ServerWeight, ServerWeightPage, Species,
};
use crate::frb_generated::StreamSink;
use crate::server::{date, event_page, ids, stream_event, timestamp};

/// How long the phone waits on a server that goes quiet, so a half-open
/// connection ends in an error instead of a screen that waits for good.
/// A stream hears a keep-alive every 15 s, so three missed is a dead link;
/// a plain call outlasts the server's own 15 s wait for a feeder's answer.
pub(crate) const TIMEOUTS: Timeouts = Timeouts {
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
    /// What its outcome settles of the write, by the reading every server
    /// client shares; `None` while it is unknown or in flight, or for an
    /// outcome a newer server added.
    pub settlement: Option<Settlement>,
    /// The feed it started, for a feed.
    pub feed: Option<u32>,
    pub sent_ms: i64,
    pub updated_ms: i64,
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
    /// A refusal a newer feeder build added, which the server cannot name.
    Unknown,
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
    /// login: from here the token alone speaks for the phone. A `url` with
    /// no scheme is `https://`, but on this device itself, and is kept so.
    ///
    /// `key` is the identity key the server's mDNS record named, or the
    /// link's when the owner logs in again; `None` for an address typed
    /// first, whose key is learned from its handshake. Either way it is
    /// checked before the password is sent, and pinned in the link.
    pub async fn login(
        url: String,
        name: String,
        password: String,
        phone: String,
        key: Option<String>,
    ) -> Result<Self, ServerError> {
        let url = with_scheme(&url);
        let key = key.as_deref().map(parse_key).transpose()?;
        let base = base(&url)?;
        let pinned = pins(&base);
        let key = match (pinned, key) {
            (true, None) => {
                let first = options(None, TIMEOUTS);
                Some(wellfed_server_client::identify(&base, first).await?)
            }
            (_, key) => key,
        };
        let mut client = open(&url, key, TIMEOUTS)?;
        // Over the server's own HTTPS the handshake proved the key; over
        // the system's authorities, or plain http on this device, the key
        // is the server's own word.
        let key = match key {
            Some(key) if pinned => key,
            expected => {
                let named = client.identity().await?;
                if expected.is_some_and(|expected| expected != named) {
                    let changed = wellfed_server_client::Error::KeyChanged {
                        offered: Some(named),
                    };
                    return Err(changed.into());
                }
                named
            }
        };

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
            key: Hex(key).to_string(),
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
        let client = open(&link.url, Some(parse_key(&link.key)?), timeouts)?;
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

    /// Log this phone out: revoke its token on the server while its own key
    /// on the feeders (`key`), and the keys it invited, stay the account's,
    /// so it still reaches them directly and the account's removal still
    /// revokes them. Any other key under the token goes with it. The server takes that only from a login, so a
    /// leaked token cannot keep the keys it enrolled: `password` is the
    /// account's, used once for a login that ends here, and never kept. A
    /// token the server already dropped counts as logged out. Every follow
    /// ends once it is; a wrong password changes nothing.
    pub async fn logout(&self, password: String, key: &ClientKey) -> Result<(), ServerError> {
        let id = TokenId(ids::parse(&self.link.token_id)?);
        // The server keeps only the keys the phone names as its own: one
        // someone else got through the token goes with it.
        let request = LogOut {
            keys: vec![Hex(*key.keypair().public())],
        };
        let mut login = open(&self.link.url, Some(parse_key(&self.link.key)?), TIMEOUTS)?;
        let credentials = Credentials {
            name: self.link.user.clone(),
            password,
        };
        login.login(&credentials).await?;
        let ended = login.log_out_token(id, &request).await;
        // The login was only the way to the logout; one that never ends
        // expires on the server.
        let _ = login.logout().await;

        if let Err(error) = ended {
            let error = ServerError::from(error);
            if error.kind != ServerErrorKind::NotFound {
                return Err(error);
            }
        }
        self.close();
        Ok(())
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

    /// Serve `food` from `feeder` from now on: what its food view and the
    /// intake of the pets eating from it count by.
    pub async fn set_feeder_food(
        &self,
        feeder: String,
        food: String,
    ) -> Result<ServerFoodPeriod, ServerError> {
        let period = self
            .client
            .set_feeder_food(ids::feeder(&feeder)?, ids::food(&food)?)
            .await?;
        Ok(period.into())
    }

    /// A page of `feeder`'s refills, oldest first: after `after` (the last
    /// page's `next`), at most `limit` (1-1000, default 100).
    pub async fn refills(
        &self,
        feeder: String,
        after: Option<String>,
        limit: Option<u32>,
    ) -> Result<ServerRefillPage, ServerError> {
        let page = self
            .client
            .refills(ids::feeder(&feeder)?, &PageQuery { after, limit })
            .await?;
        Ok(page.into())
    }

    /// Record a refill of `feeder` made now. The server takes a refill once
    /// per call and keys none, so one whose answer was lost may be recorded
    /// all the same: the screen reads [`Self::refills`] again and shows it,
    /// and never makes it again by itself.
    pub async fn add_refill(
        &self,
        feeder: String,
        refill: NewRefill,
    ) -> Result<ServerRefill, ServerError> {
        let refill = self
            .client
            .add_refill(ids::feeder(&feeder)?, &refill)
            .await?;
        Ok(refill.into())
    }

    /// Remove a refill recorded by mistake.
    pub async fn delete_refill(&self, feeder: String, refill: String) -> Result<(), ServerError> {
        Ok(self
            .client
            .delete_refill(ids::feeder(&feeder)?, ids::refill(&refill)?)
            .await?)
    }

    /// The pet eats from `feeder` too: its intake counts that feeder.
    pub async fn assign_feeder(
        &self,
        pet: String,
        feeder: String,
    ) -> Result<ServerPet, ServerError> {
        let pet = self
            .client
            .assign_feeder(ids::pet(&pet)?, ids::feeder(&feeder)?)
            .await?;
        Ok(pet.into())
    }

    pub async fn unassign_feeder(
        &self,
        pet: String,
        feeder: String,
    ) -> Result<ServerPet, ServerError> {
        let pet = self
            .client
            .unassign_feeder(ids::pet(&pet)?, ids::feeder(&feeder)?)
            .await?;
        Ok(pet.into())
    }

    /// A page of `pet`'s weight log, oldest reading first: after `after`
    /// (the last page's `next`), at most `limit` (1-1000, default 100).
    pub async fn weights(
        &self,
        pet: String,
        after: Option<String>,
        limit: Option<u32>,
    ) -> Result<ServerWeightPage, ServerError> {
        let page = self
            .client
            .weights(ids::pet(&pet)?, &PageQuery { after, limit })
            .await?;
        Ok(page.into())
    }

    /// Log `grams` (1-200000), weighed at `at_ms`, or now. As with a
    /// refill, one whose answer was lost is read back, never made again by
    /// itself.
    pub async fn add_weight(
        &self,
        pet: String,
        grams: u32,
        at_ms: Option<i64>,
    ) -> Result<ServerWeight, ServerError> {
        let weight = NewWeight {
            grams,
            at: at_ms.map(timestamp).transpose()?,
        };
        Ok(self
            .client
            .add_weight(ids::pet(&pet)?, &weight)
            .await?
            .into())
    }

    pub async fn delete_weight(&self, pet: String, weight: String) -> Result<(), ServerError> {
        Ok(self
            .client
            .delete_weight(ids::pet(&pet)?, ids::weight(&weight)?)
            .await?)
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

/// A client of the server in `link` under the phone token `secret`, for a
/// catch-up's one read of a feeder's alerts.
pub(crate) fn alert_client(link: &ServerLink, secret: &str) -> Result<Client, ServerError> {
    let client = open(&link.url, Some(parse_key(&link.key)?), TIMEOUTS)?;
    Ok(client.with_auth(Some(Auth::Token(secret.to_owned()))))
}

/// How many hex digits of the identity key the owner compares.
#[frb(sync)]
pub fn server_fingerprint(key: String) -> Result<String, ServerError> {
    Ok(wellfed_api::fingerprint(&parse_key(&key)?))
}

/// Where a server address leads, which decides how it is checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reach {
    /// This device itself (`localhost`, a loopback address): plain
    /// `http://` crosses no network.
    Device,
    /// The home network: only the server's own HTTPS.
    Home,
    /// Anywhere else: `https://` the system's authorities vouch for.
    Public,
}

fn reach(base: &Url) -> Reach {
    let host = base.host_str().unwrap_or_default();
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let loopback = bare.eq_ignore_ascii_case("localhost")
        || bare
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    if loopback {
        Reach::Device
    } else if at_home(host) || home_name(bare) {
        Reach::Home
    } else {
        Reach::Public
    }
}

/// A name only the home network resolves, which no public authority
/// certifies, so only the server's own HTTPS answers it: `home.arpa` and
/// the common router suffixes `.lan` and `.internal`, and a single label
/// (`nas`). This decides the check only: plain `http://` to one is refused
/// all the same, since anyone can claim such a name on a network.
pub(crate) fn home_name(host: &str) -> bool {
    if host.parse::<std::net::IpAddr>().is_ok() {
        return false;
    }
    let name = host.trim_end_matches('.').to_ascii_lowercase();
    !name.contains('.')
        || [".home.arpa", ".lan", ".internal"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
        || name == "home.arpa"
}

fn base(url: &str) -> Result<Url, ServerError> {
    Url::parse(url.trim()).map_err(|error| ServerError::new(ServerErrorKind::Address, error))
}

/// The identity key a link or a record names.
fn parse_key(key: &str) -> Result<[u8; 32], ServerError> {
    wellfed_session::hex::decode_array_either_case::<32>(key.trim().as_bytes()).ok_or_else(|| {
        ServerError::new(
            ServerErrorKind::NoIdentity,
            "a server identity key is 64 hex digits",
        )
    })
}

fn options(key: Option<[u8; 32]>, timeouts: Timeouts) -> Options {
    Options {
        timeouts,
        plain: PlainHttp::Refused,
        trust: key.map_or(Trust::Platform, Trust::Pinned),
        // Around a Wi-Fi with no way out, for a server off the home
        // network only: the relay decides per request. Only Android binds
        // the process to one network; a desktop keeps its proxy settings.
        proxy: cfg!(target_os = "android").then(crate::relay::proxy),
    }
}

/// Whether the server at `base` is checked by its pinned key: its own
/// HTTPS, on the home network or this device. Beyond, a name the system's
/// authorities vouch for.
fn pins(base: &Url) -> bool {
    base.scheme() == "https" && reach(base) != Reach::Public
}

/// A client of the server at `url`, `key` pinned when [`pins`] says so.
/// Plain `http://` is refused but on this device itself: a login carries
/// the password, and every call a token that never expires and can feed.
fn open(url: &str, key: Option<[u8; 32]>, timeouts: Timeouts) -> Result<Client, ServerError> {
    let base = base(url)?;
    if base.scheme() == "http" && reach(&base) != Reach::Device {
        return Err(ServerError::new(
            ServerErrorKind::PlainHttp,
            "plain http:// is taken only on this device: use the server's https:// address",
        ));
    }
    let key = if pins(&base) {
        Some(key.ok_or_else(|| {
            ServerError::new(ServerErrorKind::NoIdentity, "no identity key to check")
        })?)
    } else {
        None
    };
    Ok(Client::with_options(&base, options(key, timeouts))?)
}

/// The server's address as the owner typed it, given a scheme if it names
/// none: `https://`, so a name typed bare never sends the password in the
/// clear, but on this device itself, where plain `http://` crosses no
/// network.
pub(crate) fn with_scheme(typed: &str) -> String {
    let typed = typed.trim();
    if typed.contains("://") {
        return typed.to_owned();
    }

    let secure = format!("https://{typed}");
    let device = Url::parse(&secure).is_ok_and(|url| reach(&url) == Reach::Device);
    if device {
        format!("http://{typed}")
    } else {
        secure
    }
}

#[cfg(test)]
mod tests {
    use super::with_scheme;

    #[test]
    fn a_name_only_the_home_network_resolves_is_reached_by_its_pinned_key() {
        use super::{Reach, reach};

        let at = |url: &str| reach(&super::Url::parse(url).unwrap());
        for home in [
            "https://192.168.1.20:8443",
            "https://wellfed.local:8443",
            "https://nas.home.arpa:8443",
            "https://nas.lan:8443",
            "https://NAS.Internal.:8443",
            "https://nas:8443",
        ] {
            assert_eq!(at(home), Reach::Home, "{home}");
        }
        for public in [
            "https://feeder.example.org",
            "https://203.0.113.4:8443",
            "https://lan.example.com",
        ] {
            assert_eq!(at(public), Reach::Public, "{public}");
        }
        assert_eq!(at("http://localhost:8080"), Reach::Device);
        assert_eq!(at("http://[::1]:8080"), Reach::Device);
    }

    #[test]
    fn a_bare_address_is_plain_http_only_on_this_device() {
        for (typed, full) in [
            ("192.168.1.20:8443", "https://192.168.1.20:8443"),
            (" 10.0.0.4 ", "https://10.0.0.4"),
            ("wellfed.local:8443", "https://wellfed.local:8443"),
            ("[fd00::4]:8443", "https://[fd00::4]:8443"),
            ("127.0.0.1:8080", "http://127.0.0.1:8080"),
            ("localhost:8080", "http://localhost:8080"),
            ("[::1]:8080", "http://[::1]:8080"),
            ("feeder.example", "https://feeder.example"),
            (
                "203.0.113.4:8443/wellfed",
                "https://203.0.113.4:8443/wellfed",
            ),
            ("nas:8080", "https://nas:8080"),
            ("http://feeder.example", "http://feeder.example"),
            ("https://192.168.1.20", "https://192.168.1.20"),
        ] {
            assert_eq!(with_scheme(typed), full, "{typed}");
        }
    }
}
