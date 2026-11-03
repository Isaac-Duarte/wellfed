//! The path router against the server in process and the simulated feeder
//! (simulation, not hardware). For each path a feed's answer is dropped,
//! the caller retries, and the feeder's feed journal proves one feed:
//! - through the server: unknown, pinned to the server across a restart of
//!   the app, never sent direct, settled from its `Fed` event;
//! - direct: unanswered, pinned direct, never sent through the server once
//!   it is back, settled from the feeder's history;
//! - the server stopping mid-feed leaves it pinned to the server, unsent
//!   direct although the phone reaches the feeder, until the server returns;
//! - a feeder the server reports offline is fed direct, and the server sends
//!   nothing for that feed when the feeder returns to it;
//! - a viewer goes through the server only.

use std::cell::RefCell;
use std::net::SocketAddr;
use std::rc::Rc;
use std::time::Duration;

use tokio::sync::mpsc;
use wellfed_api::{AdoptFeeder, ClientRole, Connection, ErrorCode, NewUser, Role};
use wellfed_app_bridge::api::keys::ClientKey;
use wellfed_app_bridge::api::router::{
    PathRouter, PendingWrite, RetryOutcome, RoutePath, RoutedFeed, RouterUpdate, Settlement,
    WriteKind,
};
use wellfed_app_bridge::api::server::ServerSession;
use wellfed_app_bridge::api::server_error::ServerErrorKind;
use wellfed_app_bridge::api::session::{FeederSession, SessionUpdate};
use wellfed_server_client::{Client, StatusCode};
use wellfed_server_testkit::feeder::{self, Flash, Relay, Running};
use wellfed_server_testkit::{TempStore, TestServer, ada_login, ada_setup, config, local, refused};

const WAIT: Duration = Duration::from_secs(30);

/// How long the test server waits for a feeder's answer.
const ANSWER: Duration = Duration::from_secs(2);

/// The server with its answer wait cut to [`ANSWER`], its first owner, and
/// the owner's own client.
async fn started() -> (TestServer, Client) {
    let mut config = config();
    config.timing.answer = ANSWER;
    let server = TestServer::start_with(config).await;
    let mut owner = server.client();
    owner.setup(&ada_setup()).await.unwrap();
    (server, owner)
}

/// The server again on its stopped database, where the phone knew it.
async fn reopen(store: TempStore, address: SocketAddr) -> TestServer {
    let mut config = config();
    config.timing.answer = ANSWER;
    TestServer::reopen_at(store, config, address).await
}

/// Adopt `sim` at `address` (a relay in front of it) and wait until the
/// server reaches it.
async fn adopt(owner: &Client, sim: &Running, address: SocketAddr) -> String {
    let id = owner
        .adopt(&AdoptFeeder {
            invite: feeder::invite(sim, ClientRole::Server).await,
            name: "kitchen".into(),
            address: Some(address.to_string()),
            phone_key: None,
        })
        .await
        .unwrap()
        .id;
    linked(owner, true).await;
    id.to_string()
}

/// Wait until the server's link to its one feeder is (or is not) online.
async fn linked(owner: &Client, online: bool) {
    eventually("the server's link", async || {
        let feeders = owner.feeders().await.unwrap();
        ((feeders[0].connection == Connection::Online) == online).then_some(())
    })
    .await;
}

async fn eventually<T>(what: &str, mut check: impl AsyncFnMut() -> Option<T>) -> T {
    let found = tokio::time::timeout(WAIT, async {
        loop {
            if let Some(found) = check().await {
                return found;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    found.unwrap_or_else(|_| panic!("{what}: not within {WAIT:?}"))
}

/// This phone: the feeder's owner key, its router and the pins' file.
struct Phone {
    key: ClientKey,
    pins: tempfile::TempDir,
    router: Rc<PathRouter>,
    /// What the router tells the screens.
    updates: RefCell<mpsc::UnboundedReceiver<RouterUpdate>>,
}

impl Phone {
    fn new() -> Self {
        let pins = tempfile::tempdir().unwrap();
        let (router, updates) = router(&pins);
        Self {
            key: ClientKey::restore(feeder::OWNER.to_vec()).unwrap(),
            pins,
            router,
            updates,
        }
    }

    /// The app again, on the same pins.
    fn restarted(self) -> Self {
        self.router.close();
        let (router, updates) = router(&self.pins);
        Self {
            router,
            updates,
            ..self
        }
    }

    /// Retry `key` until it settles, the way a screen's retry button
    /// would, or the router settles it on its own and says so.
    async fn settled(&self, key: &str) -> Settlement {
        eventually("the pending write settles", async || {
            match self.router.retry(key.to_owned()).await {
                RetryOutcome::Settled { settlement, .. } => Some(settlement),
                RetryOutcome::Uncertain { .. } => None,
                RetryOutcome::Unknown => {
                    let mut updates = self.updates.borrow_mut();
                    while let Ok(update) = updates.try_recv() {
                        if let RouterUpdate::Settled {
                            pending,
                            settlement,
                        } = update
                            && pending.key == key
                        {
                            return Some(settlement);
                        }
                    }
                    panic!("{key} settled unheard");
                }
            }
        })
        .await
    }

    /// Log in to `server` as `name` and route through it.
    async fn log_in(&self, server: &TestServer, name: &str, password: &str) -> Rc<ServerSession> {
        let url = format!("http://{}", server.address());
        let session = ServerSession::login(url, name.into(), password.into(), "pixel".into())
            .await
            .unwrap();
        let account = session.account().await.unwrap();
        self.router.connect_server(&session, account);
        self.router.refresh().await.unwrap();
        Rc::new(session)
    }

    async fn log_in_as_ada(&self, server: &TestServer) -> Rc<ServerSession> {
        let login = ada_login();
        self.log_in(server, &login.name, &login.password).await
    }

    /// Open this phone's own session with `sim` at `address`, through the
    /// router, and wait for its card.
    async fn open(&self, sim: &Running, address: SocketAddr) -> Direct {
        let session = Rc::new(
            FeederSession::new(&self.key, sim.identity.to_vec(), address.to_string(), false)
                .unwrap(),
        );
        self.router.attach(&session);
        let (sender, mut updates) = mpsc::unbounded_channel();
        let running = Rc::clone(&session);
        tokio::task::spawn_local(async move {
            running
                .run_with(move |update| sender.send(update).is_ok())
                .await;
        });
        tokio::time::timeout(WAIT, async {
            while let Some(update) = updates.recv().await {
                if let SessionUpdate::Status { .. } = update {
                    return;
                }
            }
            panic!("the session ended before its card");
        })
        .await
        .expect("the session opens");
        Direct { session, updates }
    }
}

/// One direct session, and what it tells Dart.
struct Direct {
    session: Rc<FeederSession>,
    updates: mpsc::UnboundedReceiver<SessionUpdate>,
}

impl Direct {
    /// Wait until the session has ended.
    async fn closed(&mut self) {
        tokio::time::timeout(WAIT, async {
            while let Some(update) = self.updates.recv().await {
                if let SessionUpdate::Closed { .. } = update {
                    return;
                }
            }
        })
        .await
        .expect("the session ends");
    }
}

fn path(dir: &tempfile::TempDir) -> String {
    dir.path().join("pending.json").display().to_string()
}

fn identity(sim: &Running) -> String {
    wellfed_formats::hex::encode(&sim.identity)
}

fn uncertain(routed: RoutedFeed) -> PendingWrite {
    match routed {
        RoutedFeed::Uncertain { pending } => pending,
        other => panic!("the feed should be uncertain, not {other:?}"),
    }
}

/// Retry `key` until it settles, the way a screen's retry button would.
/// A router on `pins`, its updates heard into a channel.
fn router(
    pins: &tempfile::TempDir,
) -> (
    Rc<PathRouter>,
    RefCell<mpsc::UnboundedReceiver<RouterUpdate>>,
) {
    let router = Rc::new(PathRouter::new(path(pins)).unwrap());
    let (sender, updates) = mpsc::unbounded_channel();
    let hearing = Rc::clone(&router);
    tokio::task::spawn_local(async move {
        hearing
            .updates_with(move |update| sender.send(update).is_ok())
            .await;
    });
    (router, RefCell::new(updates))
}

fn fed_once(settlement: &Settlement) {
    assert!(
        matches!(
            settlement,
            Settlement::Fed {
                requested: 1,
                completed: 1,
                ..
            }
        ),
        "{settlement:?}"
    );
}

#[test]
fn a_server_feed_whose_answer_is_lost_stays_on_the_server_across_a_restart_and_feeds_once() {
    local(async {
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let relay = Relay::start(sim.address).await;
        let (server, owner) = started().await;
        adopt(&owner, &sim, relay.address).await;
        let phone = Phone::new();
        let _session = phone.log_in_as_ada(&server).await;
        let _direct = phone.open(&sim, sim.address).await;
        let kitchen = identity(&sim);
        assert_eq!(phone.router.path(kitchen.clone()), Some(RoutePath::Server));

        relay.hold_answers();
        let pending = uncertain(phone.router.feed(kitchen.clone(), 1).await);
        assert_eq!(
            (pending.path, pending.kind, pending.portions),
            (RoutePath::Server, WriteKind::Feed, Some(1))
        );
        // A retry asks the server under the same key, which sends nothing.
        let RetryOutcome::Uncertain { .. } = phone.router.retry(pending.key.clone()).await else {
            panic!("the server still cannot say");
        };
        // While it is uncertain, no new feed goes on either path.
        let RoutedFeed::Held { pending: holding } = phone.router.feed(kitchen.clone(), 1).await
        else {
            panic!("a second feed waits for the uncertain one");
        };
        assert_eq!(holding.key, pending.key);

        // The app restarts: the pin comes back from disk.
        let phone = phone.restarted();
        let _session = phone.log_in_as_ada(&server).await;
        let _direct = phone.open(&sim, sim.address).await;
        assert_eq!(phone.router.pending(), std::slice::from_ref(&pending));
        assert!(matches!(
            phone.router.feed(kitchen.clone(), 1).await,
            RoutedFeed::Held { .. }
        ));

        relay.cut();
        relay.resume();
        fed_once(&phone.settled(&pending.key).await);
        assert!(phone.router.pending().is_empty());
        assert_eq!(feeder::feeds(&flash), 1, "fed once, never sent direct");
        server.stop().await;
        sim.stop().await;
    });
}

#[test]
fn a_direct_feed_whose_answer_is_lost_stays_direct_and_feeds_once() {
    local(async {
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let relay = Relay::start(sim.address).await;
        let phone = Phone::new();
        let mut direct = phone.open(&sim, relay.address).await;
        let kitchen = identity(&sim);
        assert_eq!(phone.router.path(kitchen.clone()), Some(RoutePath::Direct));

        relay.hold_answers();
        let router = Rc::clone(&phone.router);
        let asked = kitchen.clone();
        let feeding = tokio::task::spawn_local(async move { router.feed(asked, 1).await });
        tokio::time::sleep(Duration::from_millis(300)).await;
        relay.cut();
        let pending = uncertain(feeding.await.unwrap());
        assert_eq!(pending.path, RoutePath::Direct);
        direct.closed().await;

        // Its session is gone: a retry cannot say, and sends nothing.
        let RetryOutcome::Uncertain { .. } = phone.router.retry(pending.key.clone()).await else {
            panic!("nothing has said yet");
        };
        relay.resume();
        let _again = phone.open(&sim, relay.address).await;
        fed_once(&phone.settled(&pending.key).await);
        assert_eq!(feeder::feeds(&flash), 1);
        drop(direct.session);
        sim.stop().await;
    });
}

#[test]
fn a_server_that_stops_mid_feed_keeps_it_pinned_and_unsent_direct_until_it_returns() {
    local(async {
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let relay = Relay::start(sim.address).await;
        let (server, owner) = started().await;
        adopt(&owner, &sim, relay.address).await;
        let phone = Phone::new();
        let _session = phone.log_in_as_ada(&server).await;
        let _direct = phone.open(&sim, sim.address).await;
        let kitchen = identity(&sim);

        relay.hold_answers();
        let router = Rc::clone(&phone.router);
        let asked = kitchen.clone();
        let feeding = tokio::task::spawn_local(async move { router.feed(asked, 1).await });
        tokio::time::sleep(Duration::from_millis(500)).await;
        let address = server.address();
        let store = server.stop_keeping().await;
        let pending = uncertain(feeding.await.unwrap());
        assert_eq!(pending.path, RoutePath::Server);

        // The server is gone and the phone reaches the feeder: new requests
        // go direct, but the uncertain feed stays where it went.
        let RetryOutcome::Uncertain { pending: still } =
            phone.router.retry(pending.key.clone()).await
        else {
            panic!("an unreachable server cannot settle it");
        };
        assert_eq!(still.path, RoutePath::Server);
        assert_eq!(phone.router.path(kitchen.clone()), Some(RoutePath::Direct));
        assert!(matches!(
            phone.router.feed(kitchen.clone(), 1).await,
            RoutedFeed::Held { .. }
        ));
        assert_eq!(feeder::feeds(&flash), 1, "nothing went direct");

        let server = reopen(store, address).await;
        relay.cut();
        relay.resume();
        fed_once(&phone.settled(&pending.key).await);
        assert_eq!(feeder::feeds(&flash), 1);
        server.stop().await;
        sim.stop().await;
    });
}

#[test]
fn a_feeder_the_server_reports_offline_is_fed_direct_and_the_server_sends_nothing_later() {
    local(async {
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let relay = Relay::start(sim.address).await;
        let (server, owner) = started().await;
        adopt(&owner, &sim, relay.address).await;
        let phone = Phone::new();
        let _session = phone.log_in_as_ada(&server).await;
        let _direct = phone.open(&sim, sim.address).await;
        let kitchen = identity(&sim);

        // The server loses the feeder; the router still believes it
        // online, so the feed goes to the server first, which answers
        // `feeder_offline` without sending, and then direct.
        relay.pause();
        relay.cut();
        linked(&owner, false).await;
        let RoutedFeed::Started { key, path, .. } = phone.router.feed(kitchen.clone(), 1).await
        else {
            panic!("the feed goes direct");
        };
        assert_eq!(path, RoutePath::Direct);
        eventually("the direct feed is reserved", async || {
            (feeder::feeds(&flash) == 1).then_some(())
        })
        .await;

        // Once the router has heard the server's word, a new request goes
        // direct at once.
        phone.router.refresh().await.unwrap();
        assert_eq!(phone.router.path(kitchen.clone()), Some(RoutePath::Direct));

        // The feeder returns to the server, which holds no entry for the
        // key and sends nothing for it.
        relay.resume();
        linked(&owner, true).await;
        tokio::time::sleep(Duration::from_secs(3)).await;
        let key = key.parse().unwrap();
        refused(
            owner.command(key).await,
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
        );
        // Nothing is pending, so the router has nothing to retry under it.
        assert!(phone.router.pending().is_empty());
        assert!(matches!(
            phone.router.retry(key.to_string()).await,
            RetryOutcome::Unknown
        ));
        assert_eq!(feeder::feeds(&flash), 1, "fed once, direct");
        phone.router.refresh().await.unwrap();
        assert_eq!(phone.router.path(kitchen), Some(RoutePath::Server));
        server.stop().await;
        sim.stop().await;
    });
}

#[test]
fn a_direct_feed_left_uncertain_is_never_resent_through_the_server_once_it_is_back() {
    local(async {
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let relay = Relay::start(sim.address).await;
        let (server, owner) = started().await;
        adopt(&owner, &sim, sim.address).await;
        let phone = Phone::new();
        let _session = phone.log_in_as_ada(&server).await;
        let mut direct = phone.open(&sim, relay.address).await;
        let kitchen = identity(&sim);

        // The server goes away: the feed finds it unreachable, goes direct,
        // and its answer is lost there.
        let address = server.address();
        let store = server.stop_keeping().await;
        relay.hold_answers();
        let router = Rc::clone(&phone.router);
        let asked = kitchen.clone();
        let feeding = tokio::task::spawn_local(async move { router.feed(asked, 1).await });
        tokio::time::sleep(Duration::from_millis(500)).await;
        relay.cut();
        let pending = uncertain(feeding.await.unwrap());
        assert_eq!(pending.path, RoutePath::Direct);
        direct.closed().await;

        // The server is back and the router routes new requests through it,
        // but the uncertain feed is asked of its own path only.
        let server = reopen(store, address).await;
        phone.router.refresh().await.unwrap();
        assert_eq!(phone.router.path(kitchen.clone()), Some(RoutePath::Server));
        let RetryOutcome::Uncertain { pending: still } =
            phone.router.retry(pending.key.clone()).await
        else {
            panic!("no session has read the history yet");
        };
        assert_eq!(still.path, RoutePath::Direct);
        let journal = owner
            .commands(&wellfed_api::CommandQuery::default())
            .await
            .unwrap();
        assert!(journal.items.is_empty(), "nothing went through the server");

        relay.resume();
        let _again = phone.open(&sim, relay.address).await;
        fed_once(&phone.settled(&pending.key).await);
        assert_eq!(feeder::feeds(&flash), 1);
        server.stop().await;
        sim.stop().await;
    });
}

#[test]
fn a_viewer_goes_through_the_server_only() {
    local(async {
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let (server, owner) = started().await;
        adopt(&owner, &sim, sim.address).await;
        let password = "temporary pass".to_owned();
        owner
            .create_user(&NewUser {
                name: "sitter".into(),
                password: password.clone(),
                role: Role::Viewer,
                expires: None,
            })
            .await
            .unwrap();
        let phone = Phone::new();
        let _session = phone.log_in(&server, "sitter", &password).await;
        // Even with a session of the phone's own open, a viewer has no key
        // to use.
        let _direct = phone.open(&sim, sim.address).await;
        let kitchen = identity(&sim);
        assert_eq!(phone.router.path(kitchen.clone()), Some(RoutePath::Server));

        let RoutedFeed::NotSent { error: Some(error) } =
            phone.router.feed(kitchen.clone(), 1).await
        else {
            panic!("a viewer may not feed");
        };
        assert_eq!(error.kind, ServerErrorKind::Forbidden);

        // The server down: still never direct.
        server.stop().await;
        assert_eq!(phone.router.path(kitchen.clone()), Some(RoutePath::Server));
        let RoutedFeed::NotSent { error: Some(error) } =
            phone.router.feed(kitchen.clone(), 1).await
        else {
            panic!("an unreachable server sends nothing");
        };
        assert_eq!(error.kind, ServerErrorKind::Unreachable);
        assert_eq!(phone.router.path(kitchen), Some(RoutePath::Server));
        assert_eq!(feeder::feeds(&flash), 0);
        sim.stop().await;
    });
}
