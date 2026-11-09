//! The server route against the server in process, in front of the
//! simulated feeder (simulation, not hardware):
//! - setup signs in for a token of Home Assistant's own, keeps no session,
//!   lists the feeders and refuses a viewer;
//! - the session announces the feeder through the server, its state, and
//!   a feed answered with its number;
//! - a feed whose answer is lost is uncertain, sent once, and settled from
//!   the server's journal; one kept across a restart is settled by the
//!   next session;
//! - a revoked token stops the session, and every write after it is
//!   refused unsent.

use std::sync::Arc;
use std::time::Duration;

use wellfed_homeassistant::api::{
    NotSent, Pending, ServerProblem, Skew, Update, UpdateKind, WriteResult, Written,
};
use wellfed_homeassistant::feeder::Feeder;
use wellfed_homeassistant::server::{self, Clocks, Refused, Target};
use wellfed_server_client::Client;
use wellfed_server_client::api::{
    AdoptFeeder, ClientRole, Connection, FeederId, NewUser, Role, TokenId,
};
use wellfed_server_testkit::feeder::{self, Flash, Relay, Running};
use wellfed_server_testkit::{TestServer, WAIT, ada_login, ada_setup, config, local, until};

/// Short enough that a test that loses a session waits a second.
const SHORT: Clocks = Clocks {
    timeouts: Clocks::DEFAULT.timeouts,
    retry: Duration::from_secs(1),
    recheck: Duration::from_secs(1),
};

fn url(server: &TestServer) -> String {
    format!("http://{}", server.address())
}

/// The server with its first owner, and the owner's own client.
async fn started(server: TestServer) -> (TestServer, Client) {
    let mut owner = server.client();
    owner.setup(&ada_setup()).await.unwrap();
    (server, owner)
}

/// Adopt the feeder at `address` (the simulator's, or a relay's in front of
/// it) and wait until the server reaches it.
async fn adopt(owner: &Client, sim: &Running, address: String) -> (FeederId, [u8; 32]) {
    let feeder = owner
        .adopt(&AdoptFeeder {
            invite: feeder::invite(sim, ClientRole::Server).await,
            name: "kitchen".into(),
            address: Some(address),
            phone_key: None,
        })
        .await
        .unwrap();
    until("the server reaches the feeder", || async {
        owner.feeders().await.unwrap()[0].connection == Connection::Online
    })
    .await;
    (feeder.id, feeder.identity.0)
}

/// Ada's login, as the setup flow makes it.
async fn ada(server: &TestServer) -> server::Login {
    let credentials = ada_login();
    let url = url(server);
    blocking(move || {
        runtime().block_on(server::login(
            &url,
            &credentials.name,
            &credentials.password,
            "home-assistant",
        ))
    })
    .await
    .unwrap()
}

/// The session through `server` to `feeder`, signed in as `login`.
fn session(
    server: &TestServer,
    login: &server::Login,
    feeder: (FeederId, [u8; 32]),
) -> Arc<Feeder> {
    resumed(server, login, feeder, Vec::new())
}

/// The same, handed the feeds a run before it left `pending`.
fn resumed(
    server: &TestServer,
    login: &server::Login,
    feeder: (FeederId, [u8; 32]),
    pending: Vec<Pending>,
) -> Arc<Feeder> {
    let target = Target {
        url: server::address(&url(server)).unwrap(),
        token: login.token.clone(),
        feeder: feeder.0,
        identity: feeder.1,
    };
    Arc::new(Feeder::start_server(target, SHORT, pending).unwrap())
}

/// Run a blocking call off the test's runtime, which serves the server.
async fn blocking<T: Send + 'static>(call: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(call).await.unwrap()
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// The next update of `kind`, skipping the others.
async fn next(feeder: &Arc<Feeder>, kind: UpdateKind) -> Update {
    let feeder = Arc::clone(feeder);
    blocking(move || {
        let deadline = std::time::Instant::now() + WAIT;
        while std::time::Instant::now() < deadline {
            if let Some(update) = feeder.next_update(Duration::from_millis(200))
                && update.kind == kind
            {
                return update;
            }
        }
        panic!("no {kind:?} update within {WAIT:?}");
    })
    .await
}

/// Wait for a state with the settings read: the session's reads at connect
/// are answered.
async fn settled_reads(feeder: &Arc<Feeder>) {
    loop {
        let update = next(feeder, UpdateKind::State).await;
        if update.state.is_some_and(|state| state.settings.is_some()) {
            return;
        }
    }
}

async fn feed(feeder: &Arc<Feeder>, portions: u8) -> Written {
    let feeder = Arc::clone(feeder);
    blocking(move || feeder.feed(portions)).await
}

#[test]
fn setup_signs_in_for_a_token_lists_the_feeders_and_refuses_a_viewer() {
    local(async {
        let sim = feeder::start().await;
        let (server, owner) = started(TestServer::start().await).await;
        let (_, identity) = adopt(&owner, &sim, sim.address.to_string()).await;

        let login = ada(&server).await;
        let tokens = owner.tokens().await.unwrap();
        assert_eq!(tokens.len(), 1, "the token, and no session kept");
        assert_eq!(
            (tokens[0].name.as_str(), tokens[0].id.to_string()),
            ("home-assistant", login.token_id.clone())
        );

        let (address, token) = (url(&server), login.token.clone());
        let feeders = blocking(move || runtime().block_on(server::feeders(&address, &token)))
            .await
            .unwrap();
        assert_eq!(feeders.len(), 1);
        assert_eq!(
            (feeders[0].name.as_str(), feeders[0].identity),
            ("kitchen", identity)
        );

        owner
            .create_user(&NewUser {
                name: "vic".into(),
                password: "viewer password".into(),
                role: Role::Viewer,
                expires: None,
            })
            .await
            .unwrap();
        let address = url(&server);
        let viewer = blocking(move || {
            runtime().block_on(server::login(&address, "vic", "viewer password", "ha"))
        })
        .await;
        let refused = viewer.expect_err("a viewer gets no token");
        assert!(matches!(refused, Refused::Role(_)));
        assert_eq!(refused.problem(), ServerProblem::Forbidden);
        assert_eq!(owner.tokens().await.unwrap().len(), 1);

        let address = url(&server);
        let wrong = blocking(move || {
            runtime().block_on(server::login(&address, "ada", "not the password", "ha"))
        })
        .await;
        assert_eq!(
            wrong.expect_err("a wrong password").problem(),
            ServerProblem::BadCredentials
        );

        let (address, token, id) = (url(&server), login.token.clone(), login.token_id.clone());
        blocking(move || runtime().block_on(server::revoke_token(&address, &token, &id)))
            .await
            .unwrap();
        assert!(owner.tokens().await.unwrap().is_empty());
        server.stop().await;
        sim.stop().await;
    });
}

#[test]
fn the_session_announces_the_feeder_and_a_feed_is_answered_with_its_number() {
    local(async {
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let (server, owner) = started(TestServer::start().await).await;
        let kitchen = adopt(&owner, &sim, sim.address.to_string()).await;
        let login = ada(&server).await;

        let session = session(&server, &login, kitchen);
        let connected = next(&session, UpdateKind::Connected).await;
        assert_eq!(connected.skew, Some(Skew::Same));
        let state = next(&session, UpdateKind::State).await;
        assert!(state.state.is_some_and(|state| state.settings.is_some()));

        let written = feed(&session, 1).await;
        assert_eq!(written.result, WriteResult::Feeding);
        assert_eq!(written.feed, Some(1));
        assert_eq!(feeder::feeds(&flash), 1);
        server.stop().await;
        sim.stop().await;
    });
}

#[test]
fn a_feed_whose_answer_is_lost_is_uncertain_sent_once_and_settled_by_the_journal() {
    local(async {
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let relay = Relay::start(sim.address).await;
        let mut config = config();
        config.timing.answer = Duration::from_secs(2);
        let (server, owner) = started(TestServer::start_with(config).await).await;
        let kitchen = adopt(&owner, &sim, relay.address.to_string()).await;
        let login = ada(&server).await;
        let session = session(&server, &login, kitchen);
        next(&session, UpdateKind::Connected).await;
        // Once the session's own reads through the server are answered, so
        // the answer held is the feed's.
        settled_reads(&session).await;

        relay.hold_answers();
        let written = feed(&session, 1).await;
        assert_eq!(written.result, WriteResult::Uncertain);
        let pending = written.pending.expect("an uncertain feed is kept pending");

        relay.cut();
        relay.resume();
        // Announced as pending before anything settles it.
        let announced = next(&session, UpdateKind::Pending).await;
        assert_eq!(announced.pending.as_ref(), Some(&pending));
        let settled = next(&session, UpdateKind::Settled).await;
        assert_eq!(settled.pending, Some(pending));
        let fed = settled.feed.expect("the feed the journal settled");
        assert_eq!((fed.feed, fed.requested, fed.completed), (1, 1, 1));
        assert_eq!(feeder::feeds(&flash), 1, "fed once, never resent");
        server.stop().await;
        sim.stop().await;
    });
}

#[test]
fn a_lost_feed_carried_across_a_restart_is_settled_from_the_journal() {
    local(async {
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let relay = Relay::start(sim.address).await;
        let mut config = config();
        config.timing.answer = Duration::from_secs(2);
        let (server, owner) = started(TestServer::start_with(config).await).await;
        let kitchen = adopt(&owner, &sim, relay.address.to_string()).await;
        let login = ada(&server).await;
        let session = session(&server, &login, kitchen);
        next(&session, UpdateKind::Connected).await;
        settled_reads(&session).await;

        relay.hold_answers();
        let written = feed(&session, 1).await;
        let pending = written.pending.expect("an uncertain feed is kept pending");
        // Home Assistant restarts before the journal settles it.
        drop(session);
        relay.cut();
        relay.resume();

        let restarted = resumed(&server, &login, kitchen, vec![pending.clone()]);
        let settled = next(&restarted, UpdateKind::Settled).await;
        assert_eq!(settled.pending, Some(pending));
        assert_eq!(settled.feed.map(|fed| fed.completed), Some(1));
        assert_eq!(feeder::feeds(&flash), 1, "fed once, never resent");
        server.stop().await;
        sim.stop().await;
    });
}

#[test]
fn a_revoked_token_stops_the_session_and_refuses_writes_unsent() {
    local(async {
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let (server, owner) = started(TestServer::start().await).await;
        let kitchen = adopt(&owner, &sim, sim.address.to_string()).await;
        let login = ada(&server).await;
        let session = session(&server, &login, kitchen);
        next(&session, UpdateKind::Connected).await;

        let id = TokenId(login.token_id.parse().unwrap());
        owner.revoke_token(id).await.unwrap();
        next(&session, UpdateKind::TokenRefused).await;

        let written = feed(&session, 1).await;
        assert_eq!(
            (written.result, written.not_sent),
            (WriteResult::NotSent, Some(NotSent::Stopped))
        );
        assert_eq!(feeder::feeds(&flash), 0);
        server.stop().await;
        sim.stop().await;
    });
}
