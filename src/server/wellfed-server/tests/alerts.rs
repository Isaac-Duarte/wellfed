//! Alerts away from home: the server reads each feeder's alert table over
//! its own session and serves it at `GET /feeders/{id}/alerts`, judges the
//! feeder unreachable by a phone's rule, and POSTs each alert's opening and
//! clearing to the owner's webhook, once.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use wellfed_api::{AdoptFeeder, Alert, ClientRole, Connection, FeederId};
use wellfed_server_client::Client;
use wellfed_server_testkit::feeder::{self, Flash, Running};
use wellfed_server_testkit::fixed::ManualClock;
use wellfed_server_testkit::{TestServer, ada_setup, config, local, until};
use wellfed_service::{AlertReads, Url, WebhookPlainHttp, WebhookTarget};

/// A webhook receiver on localhost: every body it was sent, as JSON, in
/// order. It answers each with `204`.
async fn receiver() -> (Url, mpsc::UnboundedReceiver<serde_json::Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (bodies, received) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            // Headers, then as much body as they announce.
            let body = loop {
                let read = socket.read(&mut buffer).await.unwrap();
                assert!(read > 0, "the request ended early");
                request.extend_from_slice(&buffer[..read]);
                let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") else {
                    continue;
                };
                let head = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                let length: usize = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .map_or(0, |value| value.trim().parse().unwrap());
                if request.len() >= end + 4 + length {
                    break request[end + 4..end + 4 + length].to_vec();
                }
            };
            socket
                .write_all(
                    b"HTTP/1.1 204 No Content\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            let _ = bodies.send(serde_json::from_slice(&body).unwrap());
        }
    });
    let url = Url::parse(&format!("http://{address}/api/webhook/secret")).unwrap();
    (url, received)
}

/// A server that reads alert tables often, with its webhook at `url`.
async fn server(url: Url, clock: Arc<ManualClock>) -> TestServer {
    let mut config = config();
    config.alerts = AlertReads::Read {
        poll: Duration::from_millis(100),
    };
    config.webhook = Some(WebhookTarget::new(url, WebhookPlainHttp::Refused).unwrap());
    TestServer::start_with_config_and_clock(config, clock).await
}

async fn adopt(http: &Client, sim: &Running) -> FeederId {
    let id = http
        .adopt(&AdoptFeeder {
            invite: feeder::invite(sim, ClientRole::Server).await,
            name: "kitchen".into(),
            address: Some(sim.address.to_string()),
            phone_key: None,
        })
        .await
        .unwrap()
        .id;
    until("the server connects to the feeder", || async {
        http.feeders().await.unwrap()[0].connection == Connection::Online
    })
    .await;
    id
}

async fn next(bodies: &mut mpsc::UnboundedReceiver<serde_json::Value>) -> serde_json::Value {
    tokio::time::timeout(wellfed_server_testkit::WAIT, bodies.recv())
        .await
        .expect("a webhook delivery in time")
        .expect("the receiver runs")
}

#[test]
fn a_new_client_is_served_and_sent_to_the_webhook_once() {
    local(async {
        let (url, mut bodies) = receiver().await;
        let clock = Arc::new(ManualClock::new());
        let sim = feeder::start().await;
        let server = server(url, Arc::clone(&clock)).await;
        let mut http = server.client();
        http.setup(&ada_setup()).await.unwrap();
        let kitchen = adopt(&http, &sim).await;

        // The server's own joining is in the table, never sent.
        until("the server reads the table", || async {
            http.feeder_alerts(kitchen).await.unwrap().read.is_some()
        })
        .await;
        let alerts = http.feeder_alerts(kitchen).await.unwrap();
        assert!(alerts.unreachable.is_none());
        assert!(
            alerts
                .alerts
                .iter()
                .all(|open| matches!(open.alert, Alert::ClientJoined { .. })),
            "{alerts:?}"
        );
        let before = alerts.alerts.len();

        let invite = feeder::invite(&sim, ClientRole::Member).await;
        feeder::join_phone(&sim, &invite, [0x42; 32]).await;
        until("the phone's joining is served", || async {
            http.feeder_alerts(kitchen).await.unwrap().alerts.len() > before
        })
        .await;

        let body = next(&mut bodies).await;
        assert_eq!(body["version"], 1);
        assert_eq!(body["event"], "opened");
        assert_eq!(body["feeder"]["id"], kitchen.to_string());
        assert_eq!(body["feeder"]["name"], "kitchen");
        assert_eq!(body["alert"]["kind"], "client_joined");
        assert_eq!(body["alert"]["level"], "security");
        let phone = hex(&feeder::phone_key([0x42; 32])[..4]);
        assert!(
            body["alert"]["words"].as_str().unwrap().contains(&phone),
            "{body}"
        );

        // Reading the same table again sends nothing more.
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(bodies.try_recv().is_err());

        server.stop().await;
    });
}

#[test]
fn a_feeder_out_of_reach_is_unreachable_until_it_answers_again() {
    local(async {
        let (url, mut bodies) = receiver().await;
        let clock = Arc::new(ManualClock::new());
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let address = sim.address;
        let server = server(url, Arc::clone(&clock)).await;
        let mut http = server.client();
        http.setup(&ada_setup()).await.unwrap();
        let kitchen = adopt(&http, &sim).await;

        sim.stop().await;
        until("the server loses the feeder", || async {
            http.feeders().await.unwrap()[0].connection != Connection::Online
        })
        .await;
        // A minute's failures are not yet a lost feeder.
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(
            http.feeder_alerts(kitchen)
                .await
                .unwrap()
                .unreachable
                .is_none()
        );

        clock.advance(Duration::from_secs(11 * 60));
        until("the server calls the feeder unreachable", || async {
            http.feeder_alerts(kitchen)
                .await
                .unwrap()
                .unreachable
                .is_some()
        })
        .await;
        let opened = next(&mut bodies).await;
        assert_eq!(opened["event"], "opened");
        assert_eq!(opened["alert"]["kind"], "unreachable");
        assert_eq!(opened["alert"]["key"], "unreachable");

        let _sim = feeder::restart(&flash, address).await;
        until("the server reaches the feeder again", || async {
            http.feeder_alerts(kitchen)
                .await
                .unwrap()
                .unreachable
                .is_none()
        })
        .await;
        let cleared = next(&mut bodies).await;
        assert_eq!(cleared["event"], "cleared");
        assert_eq!(cleared["alert"]["key"], "unreachable");
        assert_ne!(cleared["id"], opened["id"]);

        server.stop().await;
    });
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
