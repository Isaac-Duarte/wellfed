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
//!
//! And a retry only reads. Each of these drops the server's answer to the
//! phone, then the owner puts the write away, feeds again and retries the
//! old key, and the feeder's journal proves the old key never fed again:
//! - the answer lost after the feed ran, which the journal settles;
//! - a `feeder_offline` answer lost, whose entry the server dropped;
//! - a `500` before the journal held the write.
//!
//! A pins file that does not read is set aside, and no feed goes until the
//! owner has checked the bowls. A direct feed's pin never takes the end of a
//! feed already running, and
//! one pins file has one router at a time: a newer one in the process takes
//! it over, and the one it took it from sends no write again. A session offers Dart no request of its
//! own, so nothing reaches a feeder around the router.
//!
//! A server that refuses the phone's token closes the direct path too. The
//! phone adopts a feeder it paired before the server, its key becoming the
//! account's phone key.

use std::cell::RefCell;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::rc::Rc;
use std::time::Duration;

use tokio::sync::mpsc;
use wellfed_api::{AdoptFeeder, ClientRole, Connection, ErrorCode, NewUser, Role, UserPatch};
use wellfed_app_bridge::api::error::ErrorKind;
use wellfed_app_bridge::api::feeder::{Bowl, FeederReply, FeederRequest, PortionChange};
use wellfed_app_bridge::api::keys::ClientKey;
use wellfed_app_bridge::api::router::{
    Adoption, NotSentReason, PathRouter, PendingWrite, RetryOutcome, RoutePath, RoutedFeed,
    RouterUpdate, Settlement, WriteKind,
};
use wellfed_app_bridge::api::server::{ServerSession, Timeouts};
use wellfed_app_bridge::api::server_error::ServerErrorKind;
use wellfed_app_bridge::api::server_views::{
    KnownServer, ListedFeeder, ServerAccount, ServerLink, ServerRole,
};
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
        Self::trusting_after(UNFOUND)
    }

    /// A phone that takes a server write its journal lacks as never sent
    /// `grace_ms` after its attempt.
    fn trusting_after(grace_ms: i64) -> Self {
        let pins = tempfile::tempdir().unwrap();
        let (router, updates) = router(&pins, grace_ms);
        Self {
            key: ClientKey::restore(feeder::OWNER.to_vec()).unwrap(),
            pins,
            router,
            updates,
        }
    }

    /// The app again, on the same pins, once the last run's router has let
    /// them go: one router holds a pins file.
    async fn restarted(self) -> Self {
        self.router.close();
        drop(self.router);
        let router =
            eventually(
                "the last router lets its pins go",
                async || match PathRouter::open_with(path(&self.pins), UNFOUND) {
                    Ok(router) => Some(router),
                    Err(error) if error.kind == ErrorKind::InUse => None,
                    Err(error) => panic!("{error}"),
                },
            )
            .await;
        let (router, updates) = heard(router);
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

    /// Route through `session`'s token at `url` instead, giving up on a
    /// request after [`GIVE_UP`].
    async fn through(&self, session: &ServerSession, url: String) -> ServerSession {
        let link = ServerLink {
            url,
            ..session.link()
        };
        let timeouts = Timeouts {
            read: Some(GIVE_UP),
            request: Some(GIVE_UP),
        };
        let fronted = ServerSession::restore_with(link, session.secret(), timeouts).unwrap();
        let account = session.account().await.unwrap();
        self.router.connect_server(&fronted, account);
        fronted
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
    wellfed_session::hex::Lower(&sim.identity).to_string()
}

fn uncertain(routed: RoutedFeed) -> PendingWrite {
    match routed {
        RoutedFeed::Uncertain { pending } => pending,
        other => panic!("the feed should be uncertain, not {other:?}"),
    }
}

/// How long the phone's own wait for the server is cut to, where a test
/// drops the server's answer to the phone: each dropped answer costs a
/// test this long, and an answer that is not dropped comes over loopback
/// in milliseconds.
const GIVE_UP: Duration = Duration::from_secs(1);

/// The phone's wait before a server write its journal lacks counts as
/// never sent, in tests that do not drop one.
const UNFOUND: i64 = 10 * 60_000;

/// A router on `pins`, its updates heard into a channel.
fn router(
    pins: &tempfile::TempDir,
    grace_ms: i64,
) -> (
    Rc<PathRouter>,
    RefCell<mpsc::UnboundedReceiver<RouterUpdate>>,
) {
    heard(PathRouter::open_with(path(pins), grace_ms).unwrap())
}

/// `router`, its updates heard into a channel.
fn heard(
    router: PathRouter,
) -> (
    Rc<PathRouter>,
    RefCell<mpsc::UnboundedReceiver<RouterUpdate>>,
) {
    let router = Rc::new(router);
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
        let phone = phone.restarted().await;
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

        let RoutedFeed::NotSent {
            error: Some(error), ..
        } = phone.router.feed(kitchen.clone(), 1).await
        else {
            panic!("a viewer may not feed");
        };
        assert_eq!(error.kind, ServerErrorKind::Forbidden);

        // The server down: still never direct.
        server.stop().await;
        assert_eq!(phone.router.path(kitchen.clone()), Some(RoutePath::Server));
        let RoutedFeed::NotSent {
            error: Some(error), ..
        } = phone.router.feed(kitchen.clone(), 1).await
        else {
            panic!("an unreachable server sends nothing");
        };
        assert_eq!(error.kind, ServerErrorKind::Unreachable);
        assert_eq!(phone.router.path(kitchen), Some(RoutePath::Server));
        assert_eq!(feeder::feeds(&flash), 0);
        sim.stop().await;
    });
}

/// A server that fails every request inside, `500` before anything is
/// journaled, as one whose database write failed would.
fn failing_server() -> SocketAddr {
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut request = [0; 8192];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(
                b"HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
            );
        }
    });
    address
}

/// The owner puts `first` away and feeds again, which feeds; then retries
/// `first` until it settles. However its first send ended, the old key
/// never feeds again: the feeder's journal counts one more feed than
/// `fed_before`, the new one.
async fn fed_again_then_retried(
    phone: &Phone,
    flash: &Rc<Flash>,
    kitchen: &str,
    first: &PendingWrite,
    fed_before: u64,
) -> Settlement {
    phone.router.acknowledge(first.key.clone()).unwrap();
    let RoutedFeed::Started { key, .. } = phone.router.feed(kitchen.to_owned(), 1).await else {
        panic!("a feed after the owner put the first away goes");
    };
    assert_ne!(key, first.key);
    eventually("the new feed runs", async || {
        (feeder::feeds(flash) == fed_before + 1).then_some(())
    })
    .await;

    let settlement = phone.settled(&first.key).await;
    // Long enough for a resend to have reached the feeder.
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        feeder::feeds(flash),
        fed_before + 1,
        "the old key never fed again"
    );
    assert!(phone.router.pending().is_empty());
    settlement
}

/// Wait until the server's journal says `key`'s feed ended.
async fn journal_fed(owner: &Client, key: &str) {
    let key = key.parse().unwrap();
    eventually("the journal holds the feed's end", async || {
        let command = owner.command(key).await.ok()?;
        matches!(
            command.outcome,
            Some(wellfed_api::CommandOutcome::Fed { .. })
        )
        .then_some(())
    })
    .await;
}

#[test]
fn a_server_feed_whose_answer_to_the_phone_is_lost_settles_from_the_journal_and_never_feeds_again()
{
    local(async {
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let (server, owner) = started().await;
        adopt(&owner, &sim, sim.address).await;
        let front = Relay::start(server.address()).await;
        let phone = Phone::new();
        let session = phone.log_in_as_ada(&server).await;
        let _fronted = phone
            .through(&session, format!("http://{}", front.address))
            .await;
        let kitchen = identity(&sim);

        // The feed runs; the server's answer never reaches the phone.
        front.hold_answers();
        let first = uncertain(phone.router.feed(kitchen.clone(), 1).await);
        assert_eq!(first.path, RoutePath::Server);
        journal_fed(&owner, &first.key).await;
        front.cut();
        front.resume();

        let settlement = fed_again_then_retried(&phone, &flash, &kitchen, &first, 1).await;
        assert!(
            matches!(
                settlement,
                Settlement::Started { .. } | Settlement::Fed { .. }
            ),
            "{settlement:?}"
        );
        server.stop().await;
        sim.stop().await;
    });
}

#[test]
fn a_lost_feeder_offline_answer_is_settled_unsent_and_its_key_never_feeds() {
    local(async {
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let link = Relay::start(sim.address).await;
        let (server, owner) = started().await;
        adopt(&owner, &sim, link.address).await;
        let front = Relay::start(server.address()).await;
        let phone = Phone::trusting_after(0);
        let session = phone.log_in_as_ada(&server).await;
        let _fronted = phone
            .through(&session, format!("http://{}", front.address))
            .await;
        let kitchen = identity(&sim);

        // The server has lost the feeder and answers `feeder_offline`,
        // dropping its entry; that answer never reaches the phone.
        link.pause();
        link.cut();
        linked(&owner, false).await;
        front.hold_answers();
        let first = uncertain(phone.router.feed(kitchen.clone(), 1).await);
        assert_eq!(first.path, RoutePath::Server);
        front.cut();
        front.resume();
        refused(
            owner.command(first.key.parse().unwrap()).await,
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
        );

        // The feeder returns. A resend of the old key now would be a new
        // feed the server has no entry to stop.
        link.resume();
        linked(&owner, true).await;
        let settlement = fed_again_then_retried(&phone, &flash, &kitchen, &first, 0).await;
        assert_eq!(settlement, Settlement::NotSent);
        server.stop().await;
        sim.stop().await;
    });
}

#[test]
fn a_server_failing_before_its_journal_holds_the_write_never_feeds_its_key() {
    local(async {
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let (server, owner) = started().await;
        adopt(&owner, &sim, sim.address).await;
        let phone = Phone::trusting_after(0);
        let session = phone.log_in_as_ada(&server).await;
        let kitchen = identity(&sim);

        // The server fails inside before it records the feed.
        let failing = phone
            .through(&session, format!("http://{}", failing_server()))
            .await;
        let first = uncertain(phone.router.feed(kitchen.clone(), 1).await);
        assert_eq!(first.path, RoutePath::Server);
        drop(failing);
        refused(
            owner.command(first.key.parse().unwrap()).await,
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
        );

        // The server works again, under the same token and account.
        phone
            .router
            .connect_server(&session, session.account().await.unwrap());
        let settlement = fed_again_then_retried(&phone, &flash, &kitchen, &first, 0).await;
        assert_eq!(settlement, Settlement::NotSent);
        server.stop().await;
        sim.stop().await;
    });
}

#[test]
fn a_journal_that_is_not_the_write_s_own_never_settles_it_unsent() {
    local(async {
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let (server, owner) = started().await;
        adopt(&owner, &sim, sim.address).await;
        let phone = Phone::trusting_after(0);
        let session = phone.log_in_as_ada(&server).await;
        let kitchen = identity(&sim);

        let _failing = phone
            .through(&session, format!("http://{}", failing_server()))
            .await;
        let first = uncertain(phone.router.feed(kitchen.clone(), 1).await);

        // Another token on the same server: its journal is asked, and its
        // not holding the key says nothing of this write.
        let other = phone.log_in_as_ada(&server).await;
        assert_ne!(other.link().token_id, session.link().token_id);
        for _ in 0..3 {
            let RetryOutcome::Uncertain { .. } = phone.router.retry(first.key.clone()).await else {
                panic!("another journal cannot settle it");
            };
        }
        assert_eq!(phone.router.pending().len(), 1);
        assert_eq!(feeder::feeds(&flash), 0);
        server.stop().await;
        sim.stop().await;
    });
}

#[test]
fn a_direct_feed_never_takes_the_end_of_one_already_running() {
    local(async {
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let relay = Relay::start(sim.address).await;
        let phone = Phone::new();
        let mut direct = phone.open(&sim, relay.address).await;
        let kitchen = identity(&sim);

        let RoutedFeed::Started { feed: first, .. } = phone.router.feed(kitchen.clone(), 1).await
        else {
            panic!("the first feed goes");
        };
        // The first feed's end, and the next feed's answer, are held back
        // and lost with the session.
        relay.hold_answers();
        let router = Rc::clone(&phone.router);
        let asked = kitchen.clone();
        let feeding = tokio::task::spawn_local(async move { router.feed(asked, 1).await });
        tokio::time::sleep(Duration::from_millis(300)).await;
        relay.cut();
        let second = uncertain(feeding.await.unwrap());
        direct.closed().await;

        // A new session reads the history, the first feed's end in it,
        // which comes after the second's mark and asked as many portions.
        relay.resume();
        let _again = phone.open(&sim, relay.address).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        match phone.router.retry(second.key.clone()).await {
            // The feeder refused the second while the first ran.
            RetryOutcome::Uncertain { .. } => assert_eq!(feeder::feeds(&flash), 1),
            // The first had ended and the second ran: its own end.
            RetryOutcome::Settled {
                settlement: Settlement::Fed { feed, .. },
                ..
            } => {
                assert_ne!(feed, first, "the first feed's end");
                assert_eq!(feeder::feeds(&flash), 2);
            }
            other => panic!("{other:?}"),
        }
        drop(direct.session);
        sim.stop().await;
    });
}

#[test]
fn the_calibration_and_the_meal_call_go_through_the_server() {
    local(async {
        let sim = feeder::start().await;
        let (server, owner) = started().await;
        // A calibration is an owner's write: the server holds an owner's
        // key on this feeder.
        owner
            .adopt(&AdoptFeeder {
                invite: feeder::invite(&sim, ClientRole::Owner).await,
                name: "kitchen".into(),
                address: Some(sim.address.to_string()),
                phone_key: None,
            })
            .await
            .unwrap();
        linked(&owner, true).await;
        let phone = Phone::new();
        let _session = phone.log_in_as_ada(&server).await;
        let _direct = phone.open(&sim, sim.address).await;
        let kitchen = identity(&sim);

        let read = phone
            .router
            .ask(kitchen.clone(), FeederRequest::GetCalibration)
            .await;
        assert_eq!(read.path, Some(RoutePath::Server));
        let FeederReply::Calibration { calibration } = read.reply else {
            panic!("{:?}", read.reply);
        };

        let wrote = phone
            .router
            .ask(
                kitchen.clone(),
                FeederRequest::PutCalibration {
                    base: calibration.generation,
                    portion: PortionChange::Milligrams { mg: 4_200 },
                    bowl: Bowl::Single,
                },
            )
            .await;
        assert_eq!(wrote.path, Some(RoutePath::Server));
        let FeederReply::Calibration { calibration: after } = wrote.reply else {
            panic!("{:?}", wrote.reply);
        };
        assert_eq!(after.portion_mg, Some(4_200));
        assert!(phone.router.pending().is_empty());

        let call = phone
            .router
            .ask(kitchen.clone(), FeederRequest::GetCall)
            .await;
        assert_eq!(call.path, Some(RoutePath::Server));
        assert!(
            matches!(call.reply, FeederReply::Call { .. }),
            "{:?}",
            call.reply
        );
        server.stop().await;
        sim.stop().await;
    });
}

#[test]
fn a_server_that_refuses_the_phones_token_closes_the_direct_path_too() {
    local(async {
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let (server, owner) = started().await;
        adopt(&owner, &sim, sim.address).await;
        let phone = Phone::new();
        let session = phone.log_in_as_ada(&server).await;
        let _direct = phone.open(&sim, sim.address).await;
        let kitchen = identity(&sim);
        assert_eq!(phone.router.path(kitchen.clone()), Some(RoutePath::Server));

        // The owner revokes the phone's token, as deleting the account or
        // disabling it would refuse it.
        let token = wellfed_api::TokenId(session.link().token_id.parse().unwrap());
        owner.revoke_token(token).await.unwrap();
        let RoutedFeed::NotSent {
            error: Some(error), ..
        } = phone.router.feed(kitchen.clone(), 1).await
        else {
            panic!("a refused token sends nothing");
        };
        assert_eq!(error.kind, ServerErrorKind::TokenRevoked);

        // The phone's own session is open, and nothing goes on it: not a
        // feed, not a read, not a write the server has no route for.
        assert_eq!(phone.router.path(kitchen.clone()), Some(RoutePath::Server));
        let RoutedFeed::NotSent { .. } = phone.router.feed(kitchen.clone(), 1).await else {
            panic!("still nothing");
        };
        let read = phone
            .router
            .ask(kitchen.clone(), FeederRequest::GetCalibration)
            .await;
        assert_eq!(read.path, Some(RoutePath::Server));
        assert_eq!(read.reply, FeederReply::NotSent);
        assert_eq!(
            read.server_error.map(|error| error.kind),
            Some(ServerErrorKind::TokenRevoked)
        );
        let invite = phone
            .router
            .ask(
                kitchen.clone(),
                FeederRequest::CreateInvite {
                    role: ClientRole::Member,
                },
            )
            .await;
        assert_ne!(invite.path, Some(RoutePath::Direct));
        assert!(
            !matches!(invite.reply, FeederReply::Invite { .. }),
            "{:?}",
            invite.reply
        );
        assert_eq!(feeder::feeds(&flash), 0);

        // Removing the server gives the phone its own keys back.
        phone.router.disconnect_server();
        assert_eq!(phone.router.path(kitchen.clone()), Some(RoutePath::Direct));
        let RoutedFeed::Started { path, .. } = phone.router.feed(kitchen, 1).await else {
            panic!("direct-only, the phone feeds with its own key");
        };
        assert_eq!(path, RoutePath::Direct);
        eventually("the direct feed runs", async || {
            (feeder::feeds(&flash) == 1).then_some(())
        })
        .await;
        server.stop().await;
        sim.stop().await;
    });
}

#[test]
fn the_phone_adopts_a_feeder_it_paired_and_its_key_becomes_the_accounts() {
    local(async {
        let sim = feeder::start().await;
        let (server, owner) = started().await;
        let phone = Phone::new();
        let session = phone.log_in_as_ada(&server).await;
        let _direct = phone.open(&sim, sim.address).await;
        let kitchen = identity(&sim);
        // Paired before the server: the server lists nothing.
        assert_eq!(phone.router.path(kitchen.clone()), Some(RoutePath::Direct));

        let adoption = phone
            .router
            .adopt(
                kitchen.clone(),
                "kitchen".into(),
                Some(sim.address.to_string()),
                &phone.key,
            )
            .await;
        let Adoption::Adopted { feeder: adopted } = adoption else {
            panic!("{adoption:?}");
        };
        assert_eq!(
            (adopted.identity.as_str(), adopted.name.as_str()),
            (kitchen.as_str(), "kitchen")
        );
        assert_eq!(adopted.role, ClientRole::Owner, "an owner's invite");
        linked(&owner, true).await;
        phone.router.refresh().await.unwrap();
        assert_eq!(phone.router.path(kitchen.clone()), Some(RoutePath::Server));

        // The phone's own key is the account's, under this phone's token.
        let phones = owner.phones().await.unwrap();
        let ours = phones
            .iter()
            .find(|recorded| recorded.key.0 == phone.key.public().as_slice())
            .expect("the phone's key is recorded");
        assert_eq!(
            ours.token.map(|token| token.to_string()),
            Some(session.link().token_id)
        );

        // Adopting again only claims, which the server answers the same.
        let again = phone
            .router
            .adopt(kitchen.clone(), "kitchen".into(), None, &phone.key)
            .await;
        assert!(matches!(again, Adoption::Adopted { .. }), "{again:?}");
        let claimed = phone.router.claim_phone(kitchen, &phone.key).await;
        assert!(matches!(claimed, Adoption::Adopted { .. }), "{claimed:?}");
        assert_eq!(owner.feeders().await.unwrap().len(), 1);
        server.stop().await;
        sim.stop().await;
    });
}

#[test]
fn adopting_with_no_server_or_no_invite_adopts_nothing() {
    local(async {
        let sim = feeder::start().await;
        let phone = Phone::new();
        let kitchen = identity(&sim);
        let adoption = phone
            .router
            .adopt(kitchen.clone(), "kitchen".into(), None, &phone.key)
            .await;
        let Adoption::Failed { error } = adoption else {
            panic!("{adoption:?}");
        };
        assert_eq!(error.kind, ServerErrorKind::Closed);

        // With a server and no session of the phone's own, no invite.
        let (server, owner) = started().await;
        let _session = phone.log_in_as_ada(&server).await;
        let adoption = phone
            .router
            .adopt(kitchen, "kitchen".into(), None, &phone.key)
            .await;
        assert!(
            matches!(adoption, Adoption::NoInvite { .. }),
            "{adoption:?}"
        );
        assert!(owner.feeders().await.unwrap().is_empty());
        server.stop().await;
        sim.stop().await;
    });
}

#[test]
fn a_pins_file_that_does_not_read_is_set_aside_and_holds_feeds_until_the_bowls_are_checked() {
    local(async {
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let pins = tempfile::tempdir().unwrap();
        std::fs::write(path(&pins), b"{ not json").unwrap();
        let (router, updates) = router(&pins, UNFOUND);
        let phone = Phone {
            key: ClientKey::restore(feeder::OWNER.to_vec()).unwrap(),
            pins,
            router,
            updates,
        };
        let _direct = phone.open(&sim, sim.address).await;
        let kitchen = identity(&sim);

        let set_aside = phone
            .router
            .set_aside()
            .expect("the router says it set the file aside");
        assert_eq!(
            std::fs::read(&set_aside.moved_to).unwrap(),
            b"{ not json",
            "kept, not deleted"
        );
        let RoutedFeed::NotSent { reason, .. } = phone.router.feed(kitchen.clone(), 1).await else {
            panic!("a lost pin may have been a feed that ran");
        };
        assert_eq!(reason, NotSentReason::SetAside);
        assert_eq!(feeder::feeds(&flash), 0);

        phone.router.bowls_checked().unwrap();
        assert_eq!(phone.router.set_aside(), None);
        let RoutedFeed::Started { .. } = phone.router.feed(kitchen, 1).await else {
            panic!("the owner checked the bowls");
        };

        // Written down: the next start holds nothing back.
        let phone = phone.restarted().await;
        assert_eq!(phone.router.set_aside(), None);
        sim.stop().await;
    });
}

#[test]
fn a_router_left_behind_is_fenced_by_the_next_and_sends_no_write() {
    local(async {
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let phone = Phone::new();
        let _direct = phone.open(&sim, sim.address).await;
        let kitchen = identity(&sim);

        // The app's isolate restarts in the live process: its router is
        // never closed, and the new one opens the same pins.
        let next = PathRouter::open_with(path(&phone.pins), UNFOUND).expect("taken over");
        let RoutedFeed::NotSent { reason, .. } = phone.router.feed(kitchen.clone(), 1).await else {
            panic!("the old router writes nothing down, so it sends nothing");
        };
        assert_eq!(reason, NotSentReason::Store);
        let reply = phone
            .router
            .ask(kitchen.clone(), FeederRequest::SetLock { locked: true })
            .await;
        assert_eq!(reply.reply, FeederReply::NotSent);
        assert_eq!(feeder::feeds(&flash), 0);

        // The new one, with its own session, feeds.
        let session = FeederSession::new(
            &phone.key,
            sim.identity.to_vec(),
            sim.address.to_string(),
            false,
        )
        .unwrap();
        next.attach(&session);
        let running = Rc::new(session);
        let ran = Rc::clone(&running);
        tokio::task::spawn_local(async move { ran.run_with(|_| true).await });
        eventually("the new router's session opens", async || {
            (next.path(kitchen.clone()) == Some(RoutePath::Direct)).then_some(())
        })
        .await;
        eventually("the new router feeds", async || {
            match next.feed(kitchen.clone(), 1).await {
                RoutedFeed::Started { .. } => Some(()),
                RoutedFeed::NotSent { .. } => None,
                other => panic!("{other:?}"),
            }
        })
        .await;
        sim.stop().await;
    });
}

#[test]
fn a_refused_token_stays_refused_when_the_server_then_goes_down() {
    local(async {
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let (server, owner) = started().await;
        adopt(&owner, &sim, sim.address).await;
        let phone = Phone::new();
        let session = phone.log_in_as_ada(&server).await;
        let _direct = phone.open(&sim, sim.address).await;
        let kitchen = identity(&sim);

        let token = wellfed_api::TokenId(session.link().token_id.parse().unwrap());
        owner.revoke_token(token).await.unwrap();
        let RoutedFeed::NotSent { .. } = phone.router.feed(kitchen.clone(), 1).await else {
            panic!("a refused token sends nothing");
        };

        // The server then goes away (a restart, the phone off its route): a
        // failed connection is a server down, which does not undo the refusal.
        server.stop().await;
        let read = phone
            .router
            .ask(kitchen.clone(), FeederRequest::GetCalibration)
            .await;
        assert_ne!(read.path, Some(RoutePath::Direct));
        assert_eq!(phone.router.path(kitchen.clone()), Some(RoutePath::Server));
        assert!(!matches!(
            phone.router.feed(kitchen.clone(), 1).await,
            RoutedFeed::Started { .. }
        ));
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert_eq!(feeder::feeds(&flash), 0, "nothing fed around the account");
        sim.stop().await;
    });
}

/// An address nothing listens on: a server that is down.
fn nobody() -> String {
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{address}")
}

#[test]
fn a_cold_start_with_a_refused_token_keeps_the_phones_own_keys_closed() {
    local(async {
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let (server, owner) = started().await;
        adopt(&owner, &sim, sim.address).await;
        let phone = Phone::new();
        let session = phone.log_in_as_ada(&server).await;
        let account = session.account().await.unwrap();
        let token = wellfed_api::TokenId(session.link().token_id.parse().unwrap());
        owner.revoke_token(token).await.unwrap();

        // The app starts again on its kept link and secret.
        let phone = phone.restarted().await;
        let kept = ServerSession::restore(session.link(), session.secret()).unwrap();
        let resumed = phone.router.resume_server(&kept, Some(account)).await;
        assert_eq!(
            resumed.map(|_| ()).map_err(|error| error.kind),
            Err(ServerErrorKind::TokenRevoked)
        );
        let _direct = phone.open(&sim, sim.address).await;
        let kitchen = identity(&sim);
        assert_ne!(phone.router.path(kitchen.clone()), Some(RoutePath::Direct));
        assert!(matches!(
            phone.router.feed(kitchen.clone(), 1).await,
            RoutedFeed::NotSent { .. }
        ));

        // A fresh login opens them again.
        let _again = phone.log_in_as_ada(&server).await;
        assert_eq!(phone.router.path(kitchen), Some(RoutePath::Server));
        assert_eq!(feeder::feeds(&flash), 0);
        server.stop().await;
        sim.stop().await;
    });
}

#[test]
fn a_cold_start_with_the_server_down_goes_by_the_kept_account() {
    local(async {
        let sim = feeder::start().await;
        let kitchen = identity(&sim);
        let down = ServerLink {
            url: nobody(),
            user: "ada".into(),
            token_id: "00000000-0000-0000-0000-000000000001".into(),
        };
        let account = |role, expires_ms| ServerAccount {
            id: "00000000-0000-0000-0000-000000000002".into(),
            name: "ada".into(),
            role,
            expires_ms,
        };
        let cases = [
            (
                Some(account(ServerRole::Owner, None)),
                Some(RoutePath::Direct),
            ),
            (
                Some(account(ServerRole::Viewer, None)),
                Some(RoutePath::Server),
            ),
            (
                Some(account(ServerRole::Member, Some(1_900_000_000_000))),
                Some(RoutePath::Server),
            ),
            // Nothing kept: its standing is unknown, so no key of its own.
            (None, Some(RoutePath::Server)),
        ];
        for (cached, expected) in cases {
            let phone = Phone::new();
            let kept = ServerSession::restore(down.clone(), "secret".into()).unwrap();
            phone.router.configure_server(KnownServer {
                link: down.clone(),
                feeders: vec![ListedFeeder {
                    id: "00000000-0000-0000-0000-000000000003".into(),
                    identity: kitchen.clone(),
                    name: "kitchen".into(),
                }],
                account: cached.clone(),
            });
            let resumed = phone.router.resume_server(&kept, cached.clone()).await;
            assert_eq!(
                resumed.map(|_| ()).map_err(|error| error.kind),
                Err(ServerErrorKind::Unreachable)
            );
            let mut direct = phone.open(&sim, sim.address).await;
            assert_eq!(phone.router.path(kitchen.clone()), expected, "{cached:?}");
            // The feeder's session slots are few.
            direct.session.close();
            direct.closed().await;
        }
        sim.stop().await;
    });
}

#[test]
fn a_server_whose_token_is_gone_closes_the_phones_own_keys_until_it_is_removed() {
    local(async {
        let sim = feeder::start().await;
        let phone = Phone::new();
        let _direct = phone.open(&sim, sim.address).await;
        let kitchen = identity(&sim);
        assert_eq!(phone.router.path(kitchen.clone()), Some(RoutePath::Direct));

        phone.router.refuse_server();
        assert_eq!(phone.router.path(kitchen.clone()), None);
        assert!(matches!(
            phone.router.feed(kitchen.clone(), 1).await,
            RoutedFeed::NotSent {
                reason: NotSentReason::NoPath,
                ..
            }
        ));

        phone.router.disconnect_server();
        assert_eq!(phone.router.path(kitchen), Some(RoutePath::Direct));
        sim.stop().await;
    });
}

#[test]
fn an_adoption_whose_answer_is_lost_is_settled_by_reading_and_never_invited_twice() {
    local(async {
        let sim = feeder::start().await;
        let (server, owner) = started().await;
        let front = Relay::start(server.address()).await;
        let phone = Phone::new();
        let session = phone.log_in_as_ada(&server).await;
        let _fronted = phone
            .through(&session, format!("http://{}", front.address))
            .await;
        let _direct = phone.open(&sim, sim.address).await;
        let kitchen = identity(&sim);

        // The server adopts it; its answer, and the list read after, never
        // reach the phone.
        front.hold_answers();
        let adoption = phone
            .router
            .adopt(
                kitchen.clone(),
                "kitchen".into(),
                Some(sim.address.to_string()),
                &phone.key,
            )
            .await;
        assert!(
            matches!(adoption, Adoption::Unsettled { .. }),
            "{adoption:?}"
        );
        eventually("the server holds it", async || {
            (owner.feeders().await.unwrap().len() == 1).then_some(())
        })
        .await;

        // The record outlives the app: the grace holds through a restart.
        let pins = std::fs::read_to_string(path(&phone.pins)).unwrap();
        assert!(
            pins.contains("adoptions") && pins.contains(&kitchen),
            "{pins}"
        );

        // Adopting again only reads while it is unsettled: no second invite.
        let again = phone
            .router
            .adopt(kitchen.clone(), "kitchen".into(), None, &phone.key)
            .await;
        assert!(matches!(again, Adoption::Unsettled { .. }), "{again:?}");

        front.cut();
        front.resume();
        let checked = phone
            .router
            .check_adoption(kitchen.clone(), &phone.key)
            .await;
        assert!(matches!(checked, Adoption::Adopted { .. }), "{checked:?}");
        let phones = owner.phones().await.unwrap();
        assert!(
            phones
                .iter()
                .any(|recorded| recorded.key.0 == phone.key.public().as_slice()),
            "the phone's key is the account's"
        );
        assert_eq!(owner.feeders().await.unwrap().len(), 1);
        server.stop().await;
        sim.stop().await;
    });
}

/// A server that takes the connection and never answers.
fn silent() -> String {
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming() {
            held.push(stream);
        }
    });
    format!("http://{address}")
}

#[test]
fn a_viewer_never_feeds_direct_while_its_cold_start_waits_for_the_server() {
    local(async {
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let kitchen = identity(&sim);
        let phone = Rc::new(Phone::new());
        let link = ServerLink {
            url: silent(),
            user: "ada".into(),
            token_id: "00000000-0000-0000-0000-000000000001".into(),
        };
        let viewer = ServerAccount {
            id: "00000000-0000-0000-0000-000000000002".into(),
            name: "ada".into(),
            role: ServerRole::Viewer,
            expires_ms: None,
        };
        // The app hands the router what it keeps before anything else.
        phone.router.configure_server(KnownServer {
            link: link.clone(),
            feeders: vec![ListedFeeder {
                id: "00000000-0000-0000-0000-000000000003".into(),
                identity: kitchen.clone(),
                name: "kitchen".into(),
            }],
            account: Some(viewer.clone()),
        });
        let kept = Rc::new(ServerSession::restore(link, "secret".into()).unwrap());
        // The app's sessions attach as the server is asked.
        let _direct = phone.open(&sim, sim.address).await;
        let resuming = {
            let (phone, kept) = (Rc::clone(&phone), Rc::clone(&kept));
            tokio::task::spawn_local(async move {
                let _ = phone.router.resume_server(&kept, Some(viewer)).await;
            })
        };
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_ne!(phone.router.path(kitchen.clone()), Some(RoutePath::Direct));
        assert!(
            !matches!(
                phone.router.feed(kitchen.clone(), 1).await,
                RoutedFeed::Started { .. }
            ),
            "a viewer feeds through the server or not at all"
        );
        tokio::time::sleep(Duration::from_secs(3)).await;
        let feeds = feeder::feeds(&flash);
        resuming.abort();
        sim.stop().await;
        assert_eq!(
            feeds, 0,
            "a viewer fed on the phone's own key while the server was asked"
        );
    });
}

#[test]
fn a_kept_server_with_no_kept_account_uses_no_own_key_until_it_answers() {
    local(async {
        let sim = feeder::start().await;
        let kitchen = identity(&sim);
        let phone = Phone::new();
        phone.router.configure_server(KnownServer {
            link: ServerLink {
                url: silent(),
                user: "ada".into(),
                token_id: "00000000-0000-0000-0000-000000000001".into(),
            },
            feeders: Vec::new(),
            account: None,
        });
        let _direct = phone.open(&sim, sim.address).await;
        assert_eq!(phone.router.path(kitchen.clone()), None);

        // Removing the server gives the phone its own keys back.
        phone.router.disconnect_server();
        assert_eq!(phone.router.path(kitchen), Some(RoutePath::Direct));
        sim.stop().await;
    });
}

#[test]
fn a_change_to_the_account_the_router_reads_is_told() {
    local(async {
        let (server, owner) = started().await;
        let password = "temporary pass".to_owned();
        let sam = owner
            .create_user(&NewUser {
                name: "sam".into(),
                password: password.clone(),
                role: Role::Member,
                expires: None,
            })
            .await
            .unwrap();
        let phone = Phone::new();
        let _session = phone.log_in(&server, "sam", &password).await;

        owner
            .update_user(
                sam.id,
                &UserPatch {
                    role: Some(Role::Viewer),
                    ..UserPatch::default()
                },
            )
            .await
            .unwrap();
        phone.router.refresh().await.unwrap();
        let account = eventually("the router tells the account", async || {
            let mut updates = phone.updates.borrow_mut();
            while let Ok(update) = updates.try_recv() {
                if let RouterUpdate::Account { account } = update
                    && account.role == ServerRole::Viewer
                {
                    return Some(account);
                }
            }
            None
        })
        .await;
        assert_eq!(account.name, "sam");
        server.stop().await;
    });
}

#[test]
fn a_feed_s_answer_is_told_by_its_key_for_a_screen_that_holds_only_the_pin() {
    local(async {
        let sim = feeder::start().await;
        let phone = Phone::new();
        let _direct = phone.open(&sim, sim.address).await;
        let kitchen = identity(&sim);

        let RoutedFeed::Started { key, feed, .. } = phone.router.feed(kitchen, 1).await else {
            panic!("the feed starts");
        };
        let told = eventually("the feed's answer is told", async || {
            let mut updates = phone.updates.borrow_mut();
            std::iter::from_fn(|| updates.try_recv().ok()).find_map(|update| match update {
                RouterUpdate::Settled {
                    pending,
                    settlement,
                } if pending.key == key => Some(settlement),
                _ => None,
            })
        })
        .await;
        assert_eq!(told, Settlement::Started { feed });
        sim.stop().await;
    });
}

#[test]
fn a_login_uses_no_own_key_while_the_server_is_asked_who_the_account_is() {
    local(async {
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let kitchen = identity(&sim);
        let phone = Rc::new(Phone::new());
        let _direct = phone.open(&sim, sim.address).await;
        assert_eq!(phone.router.path(kitchen.clone()), Some(RoutePath::Direct));

        // The app's login: the last server let go, the new one told with no
        // account, then handed over while its answer is out.
        let link = ServerLink {
            url: silent(),
            user: "sam".into(),
            token_id: "00000000-0000-0000-0000-000000000001".into(),
        };
        phone.router.disconnect_server();
        phone.router.configure_server(KnownServer {
            link: link.clone(),
            feeders: Vec::new(),
            account: None,
        });
        let session = Rc::new(ServerSession::restore(link, "secret".into()).unwrap());
        let resuming = {
            let (phone, session) = (Rc::clone(&phone), Rc::clone(&session));
            tokio::task::spawn_local(async move {
                let _ = phone.router.resume_server(&session, None).await;
            })
        };
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert_eq!(phone.router.path(kitchen.clone()), None);
        assert!(!matches!(
            phone.router.feed(kitchen, 1).await,
            RoutedFeed::Started { .. }
        ));
        tokio::time::sleep(Duration::from_secs(3)).await;
        let feeds = feeder::feeds(&flash);
        resuming.abort();
        sim.stop().await;
        assert_eq!(
            feeds, 0,
            "a login fed on the phone's own key before the server said"
        );
    });
}
