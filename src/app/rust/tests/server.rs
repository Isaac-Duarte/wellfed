//! The bridge's server session against the server in process, in front of
//! the simulated feeder (simulation, not hardware):
//! - a login mints this phone's own token and keeps no password; the
//!   token reads the account, the feeders, a feeder's status and history,
//!   and follows the live stream; logging out revokes it, ends the stream
//!   and leaves it refused;
//! - pets and foods round-trip as their edit forms hold them, and so do a
//!   feeder's food and refills and a pet's feeders and weight log;
//! - each failure a screen tells apart comes back as its own kind.

use std::net::Ipv4Addr;
use std::rc::Rc;
use std::time::Duration;

use tokio::sync::mpsc;
use uuid::Uuid;
use wellfed_api::{AdoptFeeder, ClientRole, NewUser, Role};
use wellfed_app_bridge::api::keys::ClientKey;
use wellfed_app_bridge::api::server::{
    CommandKind, NewRefill, ServerConnection, ServerEvent, ServerSession, ServerUpdate, Species,
};
use wellfed_app_bridge::api::server_error::ServerErrorKind;
use wellfed_app_bridge::api::server_views::{ServerLink, ServerRole};
use wellfed_app_bridge::api::session::HistoryBody;
use wellfed_app_bridge::api::store::{AppStore, KnownServer, ListedFeeder, load_store, swap_store};
use wellfed_server_client::{Auth, Client, Lock};
use wellfed_server_testkit::{TestServer, ada_login, ada_setup, feeder, local};

/// This phone's own key.
fn own() -> ClientKey {
    ClientKey::restore(vec![0x0a; 32]).unwrap()
}

const WAIT: Duration = Duration::from_secs(20);

fn url(server: &TestServer) -> String {
    format!("http://{}", server.address())
}

/// The server with its first owner, and the owner's own client.
async fn started() -> (TestServer, Client) {
    let server = TestServer::start().await;
    let mut owner = server.client();
    owner.setup(&ada_setup()).await.unwrap();
    (server, owner)
}

async fn login(server: &TestServer, name: &str, password: &str) -> ServerSession {
    ServerSession::login(
        url(server),
        name.into(),
        password.into(),
        "pixel".into(),
        None,
    )
    .await
    .unwrap()
}

async fn ada(server: &TestServer) -> ServerSession {
    let credentials = ada_login();
    login(server, &credentials.name, &credentials.password).await
}

/// Follow `session`'s stream on this thread, into a channel.
fn follow(
    session: &Rc<ServerSession>,
    after: Option<String>,
) -> mpsc::UnboundedReceiver<ServerUpdate> {
    let (sender, receiver) = mpsc::unbounded_channel();
    let session = Rc::clone(session);
    tokio::task::spawn_local(async move {
        session
            .follow_with(after, move |update| sender.send(update).is_ok())
            .await;
    });
    receiver
}

/// The next update that `pick` takes, skipping the others.
async fn heard<T>(
    updates: &mut mpsc::UnboundedReceiver<ServerUpdate>,
    what: &str,
    mut pick: impl FnMut(ServerUpdate) -> Option<T>,
) -> T {
    let wanted = tokio::time::timeout(WAIT, async {
        loop {
            let update = updates.recv().await.expect("the follow ends with Closed");
            if let Some(wanted) = pick(update) {
                return wanted;
            }
        }
    })
    .await;
    wanted.unwrap_or_else(|_| panic!("{what}: not within {WAIT:?}"))
}

#[test]
fn a_login_mints_the_phones_token_which_reads_follows_and_logs_out() {
    local(async {
        let sim = feeder::start().await;
        let (server, owner) = started().await;

        let session = Rc::new(ada(&server).await);
        let link = session.link();
        assert_eq!(
            (link.url.as_str(), link.user.as_str()),
            (url(&server).as_str(), "ada")
        );
        let tokens = owner.tokens().await.unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(
            (tokens[0].name.as_str(), tokens[0].id.to_string()),
            ("pixel", link.token_id.clone())
        );

        let account = session.account().await.unwrap();
        assert_eq!(
            (account.name.as_str(), account.role),
            ("ada", ServerRole::Owner)
        );
        assert!(session.health().await.is_ok());
        assert!(session.feeders().await.unwrap().is_empty());

        // The stored link and secret make the same session.
        let restored = ServerSession::restore(link.clone(), session.secret()).unwrap();
        assert_eq!(restored.account().await.unwrap().id, account.id);

        let mut updates = follow(&session, None);
        heard(&mut updates, "the stream opens", |update| {
            matches!(update, ServerUpdate::Opened).then_some(())
        })
        .await;

        let kitchen = owner
            .adopt(&AdoptFeeder {
                invite: feeder::invite(&sim, ClientRole::Server).await,
                name: "kitchen".into(),
                address: Some(sim.address.to_string()),
                phone_key: None,
            })
            .await
            .unwrap()
            .id
            .to_string();
        heard(
            &mut updates,
            "the server reaches the feeder",
            |update| match update {
                ServerUpdate::Event {
                    event:
                        ServerEvent::Connection {
                            feeder,
                            connection: ServerConnection::Online,
                        },
                    ..
                } if feeder == kitchen => Some(()),
                _ => None,
            },
        )
        .await;

        let feeders = session.feeders().await.unwrap();
        assert_eq!(feeders.len(), 1);
        assert_eq!(
            feeders[0].identity,
            wellfed_session::hex::Lower(&sim.identity).to_string()
        );
        assert_eq!(feeders[0].name, "kitchen");

        // A write by this phone's token is heard as its command and its
        // history entry, and the history reads it back. The feeder's
        // backlog is stored after it shows online, so the write waits for
        // it: otherwise the backlog may follow the command on the stream.
        let phone = server
            .client()
            .with_auth(Some(Auth::Token(session.secret())));
        let key = Uuid::from_u128(0x0192c5d0_4444_7000_8000_000000000001);
        let feeder_id = feeders[0].id.parse().map(wellfed_api::FeederId).unwrap();
        server.caught_up(feeder_id).await;
        phone.set_lock(feeder_id, Lock::Locked, key).await.unwrap();
        let resume = heard(&mut updates, "the lock's command", |update| match update {
            ServerUpdate::Event {
                resume,
                event: ServerEvent::Command { command },
            } if command.key == key.to_string() => {
                assert_eq!(command.kind, CommandKind::Lock);
                Some(resume)
            }
            _ => None,
        })
        .await;
        assert!(!resume.is_empty(), "a command names its stream position");
        heard(&mut updates, "the lock's event", |update| match update {
            ServerUpdate::Event {
                event: ServerEvent::History { feeder, entry },
                ..
            } if feeder == kitchen => {
                matches!(entry.entry.body, HistoryBody::Lock { locked: true, .. }).then_some(())
            }
            _ => None,
        })
        .await;

        let mut entries = Vec::new();
        let mut after = None;
        loop {
            let page = session
                .events(kitchen.clone(), None, after, Some(2))
                .await
                .unwrap();
            entries.extend(page.entries);
            match page.next {
                Some(next) if after != Some(next) => after = Some(next),
                _ => break,
            }
        }
        assert!(
            entries
                .iter()
                .any(|entry| matches!(entry.entry.body, HistoryBody::Lock { locked: true, .. })),
            "paged with after, the history holds the lock"
        );
        let seqs: Vec<u64> = entries.iter().map(|entry| entry.entry.seq).collect();
        assert!(seqs.windows(2).all(|pair| pair[0] < pair[1]), "{seqs:?}");

        let detail = session.feeder(kitchen.clone()).await.unwrap();
        let status = detail.status.expect("the server holds the feeder's status");
        assert_eq!(status.card.locked, Some(true));
        let food = session.feeder_food(kitchen.clone()).await.unwrap();
        assert!(food.current.is_none(), "no food was set");

        // Resumed from the command's position, the stream gives what came
        // after it (the lock's event), or a resync when it keeps no more.
        let mut resumed = follow(&session, Some(resume));
        let caught_up =
            heard(
                &mut resumed,
                "the resumed stream's first event",
                |update| match update {
                    ServerUpdate::Event {
                        event: ServerEvent::History { entry, .. },
                        ..
                    } => Some(matches!(
                        entry.entry.body,
                        HistoryBody::Lock { locked: true, .. }
                    )),
                    ServerUpdate::Event {
                        event: ServerEvent::Resync,
                        resume,
                    } => {
                        assert!(resume.is_empty(), "a resync clears the position");
                        Some(true)
                    }
                    _ => None,
                },
            )
            .await;
        assert!(caught_up, "the first event after the command is the lock's");

        session.logout(ada_login().password, &own()).await.unwrap();
        heard(
            &mut updates,
            "the stream ends at logout",
            |update| match update {
                ServerUpdate::Closed { error } => {
                    assert!(error.is_none(), "{error:?}");
                    Some(())
                }
                _ => None,
            },
        )
        .await;
        assert_eq!(
            restored.account().await.map_err(|error| error.kind).err(),
            Some(ServerErrorKind::TokenRevoked)
        );
        // Again: the token is already gone, which is what logging out wants.
        restored.logout(ada_login().password, &own()).await.unwrap();
        assert!(owner.tokens().await.unwrap().is_empty());

        server.stop().await;
    });
}

#[test]
fn a_follow_ends_at_close_and_a_closed_session_follows_no_more() {
    local(async {
        let (server, _owner) = started().await;
        let session = Rc::new(ada(&server).await);

        let mut updates = follow(&session, None);
        heard(&mut updates, "the stream opens", |update| {
            matches!(update, ServerUpdate::Opened).then_some(())
        })
        .await;
        session.close();
        heard(
            &mut updates,
            "the stream ends at close",
            |update| match update {
                ServerUpdate::Closed { error } => {
                    assert!(error.is_none(), "{error:?}");
                    Some(())
                }
                _ => None,
            },
        )
        .await;

        // A closed session follows no more, and says so.
        let mut again = follow(&session, None);
        let error = heard(
            &mut again,
            "a closed session's follow",
            |update| match update {
                ServerUpdate::Closed { error } => Some(error),
                _ => None,
            },
        )
        .await;
        assert_eq!(error.map(|error| error.kind), Some(ServerErrorKind::Closed));
        assert!(session.account().await.is_ok(), "reads still work");

        server.stop().await;
    });
}

#[test]
fn pets_and_foods_round_trip_as_their_forms_hold_them() {
    local(async {
        let (server, _owner) = started().await;
        let session = ada(&server).await;

        let food = session
            .create_food("Acme".into(), "Kibble".into(), 3500, 2000)
            .await
            .unwrap();
        let food = session
            .update_food(food.id, "Acme".into(), "Kibble+".into(), 3600, 2500)
            .await
            .unwrap();
        assert_eq!(
            (food.name.as_str(), food.kcal_per_kg, food.bag_grams),
            ("Kibble+", 3600, 2500)
        );
        assert_eq!(session.foods().await.unwrap(), std::slice::from_ref(&food));

        let pet = session
            .create_pet(
                "Tom".into(),
                Species::Cat,
                Some("2020-05-01".into()),
                Some(250),
            )
            .await
            .unwrap();
        assert_eq!(pet.birth_date.as_deref(), Some("2020-05-01"));
        let pet = session
            .update_pet(pet.id, "Tom".into(), Species::Cat, None, None)
            .await
            .unwrap();
        assert_eq!(
            (pet.birth_date.as_deref(), pet.daily_kcal),
            (None, None),
            "the form cleared them"
        );
        assert_eq!(session.pets().await.unwrap(), std::slice::from_ref(&pet));
        let intake = session.pet_intake(pet.id.clone(), Some(3)).await.unwrap();
        assert_eq!(intake.pet, pet.id);
        assert!(intake.feeders.is_empty() && intake.anomalies.is_empty());

        let refused = session
            .create_pet(String::new(), Species::Dog, None, None)
            .await
            .map_err(|error| error.kind);
        assert_eq!(refused.err(), Some(ServerErrorKind::Invalid));
        let refused = session
            .create_pet("Rex".into(), Species::Dog, Some("May 1".into()), None)
            .await
            .map_err(|error| error.kind);
        assert_eq!(
            refused.err(),
            Some(ServerErrorKind::Invalid),
            "a date that does not parse"
        );

        session.delete_pet(pet.id.clone()).await.unwrap();
        session.delete_food(food.id.clone()).await.unwrap();
        assert!(session.pets().await.unwrap().is_empty());
        let gone = session
            .delete_food(food.id)
            .await
            .map_err(|error| error.kind);
        assert_eq!(gone.err(), Some(ServerErrorKind::NotFound));

        server.stop().await;
    });
}

#[test]
fn refills_weights_and_a_feeders_food_round_trip() {
    local(async {
        let sim = feeder::start().await;
        let (server, owner) = started().await;
        let session = ada(&server).await;
        let kitchen = owner
            .adopt(&AdoptFeeder {
                invite: feeder::invite(&sim, ClientRole::Server).await,
                name: "kitchen".into(),
                address: Some(sim.address.to_string()),
                phone_key: None,
            })
            .await
            .unwrap()
            .id
            .to_string();

        // A full bag needs the feeder's food, whose bag it is.
        let refused = session
            .add_refill(kitchen.clone(), NewRefill::FullBag)
            .await
            .map_err(|error| error.kind);
        assert_eq!(refused.err(), Some(ServerErrorKind::NoCurrentFood));
        let food = session
            .create_food("Acme".into(), "Kibble".into(), 3500, 2000)
            .await
            .unwrap();
        let period = session
            .set_feeder_food(kitchen.clone(), food.id.clone())
            .await
            .unwrap();
        assert_eq!(
            (period.food.id.as_str(), period.to_ms),
            (food.id.as_str(), None)
        );

        let bag = session
            .add_refill(kitchen.clone(), NewRefill::FullBag)
            .await
            .unwrap();
        assert_eq!((bag.grams, bag.full_bag), (2000, true));
        let scoop = session
            .add_refill(kitchen.clone(), NewRefill::Grams { grams: 300 })
            .await
            .unwrap();
        let page = session
            .refills(kitchen.clone(), None, Some(1))
            .await
            .unwrap();
        assert_eq!(page.items, std::slice::from_ref(&bag), "oldest first");
        let rest = session
            .refills(kitchen.clone(), page.next, None)
            .await
            .unwrap();
        assert_eq!(rest.items, std::slice::from_ref(&scoop));
        session
            .delete_refill(kitchen.clone(), scoop.id.clone())
            .await
            .unwrap();
        let food_view = session.feeder_food(kitchen.clone()).await.unwrap();
        assert_eq!(food_view.last_refill, Some(bag));
        assert_eq!(
            food_view.current.map(|current| current.food.id),
            Some(food.id)
        );

        let pet = session
            .create_pet("Tom".into(), Species::Cat, None, None)
            .await
            .unwrap();
        let pet = session
            .assign_feeder(pet.id, kitchen.clone())
            .await
            .unwrap();
        assert_eq!(pet.feeders, std::slice::from_ref(&kitchen));
        let vet = 1_760_000_000_000;
        let earlier = session
            .add_weight(pet.id.clone(), 4_100, Some(vet))
            .await
            .unwrap();
        assert_eq!((earlier.grams, earlier.at_ms), (4_100, vet));
        let now = session
            .add_weight(pet.id.clone(), 4_200, None)
            .await
            .unwrap();
        let log = session.weights(pet.id.clone(), None, None).await.unwrap();
        assert_eq!(log.items, [earlier.clone(), now.clone()]);
        session
            .delete_weight(pet.id.clone(), earlier.id)
            .await
            .unwrap();
        let log = session.weights(pet.id.clone(), None, None).await.unwrap();
        assert_eq!(log.items, [now]);
        let refused = session
            .add_weight(pet.id.clone(), 0, None)
            .await
            .map_err(|error| error.kind);
        assert_eq!(refused.err(), Some(ServerErrorKind::Invalid));
        let pet = session.unassign_feeder(pet.id, kitchen).await.unwrap();
        assert!(pet.feeders.is_empty());

        server.stop().await;
        sim.stop().await;
    });
}

#[test]
fn each_failure_a_screen_tells_apart_has_its_own_kind() {
    local(async {
        let (server, owner) = started().await;
        let kind = |result: Result<ServerSession, _>| {
            result
                .map_err(|error: wellfed_app_bridge::api::server_error::ServerError| error.kind)
                .err()
        };

        for address in ["ftp://192.0.2.7", "not a url", "https://192.0.2.7/?a=1"] {
            let result = ServerSession::login(
                address.into(),
                "ada".into(),
                "x".into(),
                "pixel".into(),
                None,
            )
            .await;
            assert_eq!(kind(result), Some(ServerErrorKind::Address), "{address}");
        }

        // Plain http anywhere but this device sends nothing: no password,
        // no token.
        let link = |url: &str| ServerLink {
            url: url.into(),
            key: "00".repeat(32),
            user: "ada".into(),
            token_id: "t".into(),
        };
        for address in [
            "http://feeder.example",
            "http://203.0.113.4:8080",
            "http://192.168.1.20:8080",
            "http://wellfed.local:8080",
        ] {
            let result = ServerSession::login(
                address.into(),
                "ada".into(),
                "x".into(),
                "pixel".into(),
                None,
            )
            .await;
            assert_eq!(kind(result), Some(ServerErrorKind::PlainHttp), "{address}");
            assert_eq!(
                kind(ServerSession::restore(link(address), "secret".into())),
                Some(ServerErrorKind::PlainHttp),
                "{address}"
            );
        }
        for address in [
            "https://192.168.1.20:8443",
            "https://feeder.example",
            "http://127.0.0.1:8080",
        ] {
            assert!(
                ServerSession::restore(link(address), "secret".into()).is_ok(),
                "{address}"
            );
        }

        // A port nothing listens on.
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let closed = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let result =
            ServerSession::login(closed, "ada".into(), "x".into(), "pixel".into(), None).await;
        assert_eq!(kind(result), Some(ServerErrorKind::Unreachable));

        let session = Rc::new(ada(&server).await);

        // A viewer reads, and may not write.
        let user = NewUser {
            name: "vic".into(),
            password: "temporary pass".into(),
            role: Role::Viewer,
            expires: None,
        };
        owner.create_user(&user).await.unwrap();
        let viewer = login(&server, "vic", "temporary pass").await;
        assert_eq!(viewer.account().await.unwrap().role, ServerRole::Viewer);
        assert!(viewer.pets().await.unwrap().is_empty());
        let refused = viewer
            .create_food("Acme".into(), "Kibble".into(), 3500, 2000)
            .await;
        assert_eq!(
            refused.map_err(|error| error.kind).err(),
            Some(ServerErrorKind::Forbidden)
        );
        let missing = viewer.feeder(Uuid::from_u128(1).to_string()).await;
        assert_eq!(
            missing.map_err(|error| error.kind).err(),
            Some(ServerErrorKind::NotFound)
        );
        let bad_id = viewer.feeder("kitchen".into()).await;
        assert_eq!(
            bad_id.map_err(|error| error.kind).err(),
            Some(ServerErrorKind::Invalid)
        );

        // An owner revoking the phone's token from elsewhere.
        let token = viewer
            .link()
            .token_id
            .parse()
            .map(wellfed_api::TokenId)
            .unwrap();
        owner.revoke_token(token).await.unwrap();
        let revoked = viewer.account().await.map_err(|error| error.kind);
        assert_eq!(revoked.err(), Some(ServerErrorKind::TokenRevoked));

        // Failed logins, until the server takes no more for a while.
        let wrong = || {
            ServerSession::login(
                url(&server),
                "ada".into(),
                "wrong password".into(),
                "pixel".into(),
                None,
            )
        };
        assert_eq!(kind(wrong().await), Some(ServerErrorKind::BadCredentials));
        let limited = loop {
            match wrong().await {
                Err(error) if error.kind == ServerErrorKind::BadCredentials => {}
                Err(error) => break error,
                Ok(_) => panic!("a wrong password logged in"),
            }
        };
        assert_eq!(limited.kind, ServerErrorKind::RateLimited);
        assert!(
            limited.retry_after_seconds.is_some_and(|wait| wait > 0),
            "{limited:?}"
        );

        // A server that stops answering: the stream ends, then nothing connects.
        let mut updates = follow(&session, None);
        heard(&mut updates, "the stream opens", |update| {
            matches!(update, ServerUpdate::Opened).then_some(())
        })
        .await;
        server.stop().await;
        let error = heard(
            &mut updates,
            "the stream ends with the server",
            |update| match update {
                ServerUpdate::Closed { error } => Some(error),
                _ => None,
            },
        )
        .await;
        assert_eq!(
            error.map(|error| error.kind),
            Some(ServerErrorKind::Interrupted)
        );
        let gone = session.feeders().await.map_err(|error| error.kind);
        assert_eq!(gone.err(), Some(ServerErrorKind::Unreachable));
    });
}

#[test]
fn the_app_file_keeps_the_server_link_and_its_list_and_no_secret() {
    local(async {
        let (server, _owner) = started().await;
        let session = ada(&server).await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wellfed.json");
        let path = path.to_str().unwrap();

        // The list a cold start with the server down still routes by.
        let listed = ListedFeeder {
            id: "00000000-0000-0000-0000-000000000007".into(),
            identity: "ab".repeat(32),
            name: "kitchen".into(),
        };
        let store = AppStore {
            server: Some(KnownServer {
                link: session.link(),
                feeders: vec![listed],
                account: Some(session.account().await.unwrap()),
            }),
            ..AppStore::default()
        };
        assert!(swap_store(path, &load_store(path).unwrap(), &store).unwrap());

        assert_eq!(load_store(path).unwrap(), store);
        let text = std::fs::read_to_string(path).unwrap();
        assert!(
            !text.contains(&session.secret()),
            "the token's secret is secure storage's"
        );
        assert!(!text.contains(&ada_login().password));
        server.stop().await;
    });
}
