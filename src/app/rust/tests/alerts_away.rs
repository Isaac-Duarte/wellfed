//! Alerts away from home through the owner's server, against the server in
//! process in front of the simulated feeder (simulation, not hardware):
//! - away, a catch-up reads the server's copy of the feeder's table and
//!   posts its alerts;
//! - home again, a catch-up reads the feeder itself and posts nothing new,
//!   and away once more nothing either: one alert, one notification;
//! - a server that turns the phone away, or doesn't answer, judges nothing.

use std::time::Duration;

use wellfed_api::{AdoptFeeder, Alert, ClientRole, Connection, Hex};
use wellfed_app_bridge::api::alerts::{
    AlertServer, Attempt, CatchUp, CatchUpEnd, NoticeChange, PhoneNetwork, PostedNotice,
    catch_up_with, confirm_notices,
};
use wellfed_app_bridge::api::keys::ClientKey;
use wellfed_app_bridge::api::model::NoticeKind;
use wellfed_app_bridge::api::server::ServerSession;
use wellfed_app_bridge::api::store::{
    AppStore, KnownFeeder, KnownServer, ListedFeeder, save_store,
};
use wellfed_server_testkit::feeder::{self, Running};
use wellfed_server_testkit::{TestServer, ada_login, ada_setup, config, local, until};
use wellfed_service::AlertReads;

/// This phone's key, joined to the feeder as a member.
const PHONE: [u8; 32] = [0x31; 32];
/// Another member's, whose joining is the alert.
const VISITOR: [u8; 32] = [0x32; 32];

fn home() -> PhoneNetwork {
    PhoneNetwork::Wifi {
        gateway: "192.168.1.1".into(),
        subnet: "192.168.1.0/24".into(),
    }
}

fn hex(identity: &[u8; 32]) -> String {
    identity.iter().map(|byte| format!("{byte:02x}")).collect()
}

async fn catch_up(
    dir: &str,
    sim: &Running,
    network: PhoneNetwork,
    server: Option<AlertServer>,
) -> CatchUp {
    let key = ClientKey::restore(PHONE.to_vec()).unwrap();
    catch_up_with(
        dir.into(),
        &key,
        hex(&sim.identity),
        None,
        Some(sim.address.to_string()),
        Attempt::First,
        network,
        server,
    )
    .await
    .unwrap()
}

/// Post every change, as Dart would, and say which kinds were posted.
async fn post(dir: &str, sim: &Running, done: &CatchUp) -> Vec<NoticeKind> {
    let mut posted = Vec::new();
    let mut ids = Vec::new();
    for change in &done.changes {
        if let NoticeChange::Post { notice } = change {
            posted.push(notice.kind);
            ids.push(PostedNotice::from(notice));
        }
    }
    confirm_notices(dir.into(), hex(&sim.identity), ids, Vec::new())
        .await
        .unwrap();
    posted
}

#[test]
fn an_alert_heard_away_and_then_at_home_posts_once() {
    local(async {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().to_string_lossy().into_owned();
        let sim = feeder::start().await;
        let mut config = config();
        config.alerts = AlertReads::Read {
            poll: Duration::from_millis(100),
        };
        let server = TestServer::start_with(config).await;
        let mut owner = server.client();
        owner.setup(&ada_setup()).await.unwrap();
        let kitchen = owner
            .adopt(&AdoptFeeder {
                invite: feeder::invite(&sim, ClientRole::Server).await,
                name: "kitchen".into(),
                address: Some(sim.address.to_string()),
                phone_key: None,
            })
            .await
            .unwrap()
            .id;
        until("the server connects", || async {
            owner.feeders().await.unwrap()[0].connection == Connection::Online
        })
        .await;

        let identity = hex(&sim.identity);
        let store = AppStore {
            feeders: vec![KnownFeeder {
                identity: identity.clone(),
                name: "Kitchen".into(),
                address: Some(sim.address.to_string()),
                manual_address: None,
            }],
            ..AppStore::default()
        };
        save_store(&format!("{dir}/wellfed.json"), &store).unwrap();
        let phone = feeder::invite(&sim, ClientRole::Member).await;
        feeder::join_phone(&sim, &phone, PHONE).await;
        let visitor = feeder::invite(&sim, ClientRole::Member).await;
        feeder::join_phone(&sim, &visitor, VISITOR).await;
        let [a, b, c, d, ..] = feeder::phone_key(VISITOR);
        let visitor = Hex([a, b, c, d]);
        until("the server reads the visitor's joining", || async {
            let alerts = owner.feeder_alerts(kitchen).await.unwrap();
            alerts.alerts.iter().any(|open| {
                matches!(open.alert, Alert::ClientJoined { client, .. } if client == visitor)
            })
        })
        .await;

        let credentials = ada_login();
        let session = ServerSession::login(
            format!("http://{}", server.address()),
            credentials.name,
            credentials.password,
            "pixel".into(),
        )
        .await
        .unwrap();
        let alert_server = AlertServer {
            server: KnownServer {
                link: session.link(),
                feeders: vec![ListedFeeder {
                    id: kitchen.to_string(),
                    identity: identity.clone(),
                    name: "kitchen".into(),
                }],
                account: None,
            },
            secret: session.secret(),
        };

        // Away: through the server, the server's joining and the
        // visitor's post; this phone's own stays quiet.
        let away = catch_up(
            &dir,
            &sim,
            PhoneNetwork::Elsewhere,
            Some(alert_server.clone()),
        )
        .await;
        assert_eq!(away.end, CatchUpEnd::Server);
        assert_eq!(
            post(&dir, &sim, &away).await,
            [NoticeKind::ClientJoined, NoticeKind::ClientJoined]
        );

        // Home: the feeder itself, the same alert, nothing new.
        let home_read = catch_up(&dir, &sim, home(), Some(alert_server.clone())).await;
        assert!(
            matches!(home_read.end, CatchUpEnd::Read | CatchUpEnd::Unchanged),
            "{:?}",
            home_read.end
        );
        assert!(post(&dir, &sim, &home_read).await.is_empty());

        // Away again: still nothing new.
        let again = catch_up(
            &dir,
            &sim,
            PhoneNetwork::Elsewhere,
            Some(alert_server.clone()),
        )
        .await;
        assert_eq!(again.end, CatchUpEnd::Server);
        assert!(post(&dir, &sim, &again).await.is_empty());

        // A server that turns this phone away judges nothing either, and
        // says it did.
        let turned_away = AlertServer {
            secret: "wf_0000000000000000000000000000000000000000000".into(),
            ..alert_server.clone()
        };
        let refused = catch_up(&dir, &sim, PhoneNetwork::Elsewhere, Some(turned_away)).await;
        assert_eq!(refused.end, CatchUpEnd::ServerRefused);
        assert!(refused.changes.is_empty());

        // A server that doesn't answer judges nothing.
        server.stop().await;
        let silent = catch_up(&dir, &sim, PhoneNetwork::Elsewhere, Some(alert_server)).await;
        assert_eq!(silent.end, CatchUpEnd::ServerUnanswered);
        assert!(silent.changes.is_empty());
    });
}
