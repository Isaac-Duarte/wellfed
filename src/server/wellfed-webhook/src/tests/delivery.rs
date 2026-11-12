//! Deliveries against a receiver on loopback and a store of their own.

use std::sync::Mutex;

use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use wellfed_api::{ClientRole, Home, HomeId, Model, TimeFormat, Units};
use wellfed_client::alerts::{Context, Cursor, FeederClock, Mutes};
use wellfed_mirror::ReadTable;
use wellfed_model::{Alert, AlertsView, Generation, Hex, OpenAlert};
use wellfed_store::{NewFeeder, SealedKey};

use super::*;

/// A receiver on loopback that answers every POST with the next status
/// of `answers` (the last one once they run out) and hands each body over.
async fn receiver(answers: Vec<u16>) -> (Url, mpsc::UnboundedReceiver<serde_json::Value>) {
    held_receiver(answers, None).await
}

/// The same, holding each answer until `release` gives one leave, or
/// answering at once once it is closed: a delivery kept in flight while
/// the test changes the table.
async fn held_receiver(
    answers: Vec<u16>,
    mut release: Option<mpsc::UnboundedReceiver<()>>,
) -> (Url, mpsc::UnboundedReceiver<serde_json::Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
    let address = listener.local_addr().expect("an address");
    let (bodies, received) = mpsc::unbounded_channel();
    let answers = Arc::new(Mutex::new(answers));
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            let body = loop {
                let Ok(read) = socket.read(&mut buffer).await else {
                    break None;
                };
                if read == 0 {
                    break None;
                }
                request.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&request);
                let Some(end) = text.find("\r\n\r\n") else {
                    continue;
                };
                let length = text[..end]
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                if request.len() >= end + 4 + length {
                    break Some(request[end + 4..end + 4 + length].to_vec());
                }
            };
            let Some(body) = body else { continue };
            let status = {
                let mut answers = answers.lock().expect("the answers");
                if answers.len() > 1 {
                    answers.remove(0)
                } else {
                    answers[0]
                }
            };
            let _ = bodies.send(serde_json::from_slice(&body).expect("a JSON body"));
            if let Some(gate) = &mut release {
                // Closed: every answer from now on goes at once.
                if gate.recv().await.is_none() {
                    release = None;
                }
            }
            let answer =
                format!("HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            let _ = socket.write_all(answer.as_bytes()).await;
        }
    });
    let url = Url::parse(&format!("http://{address}/api/webhook/x")).expect("a URL");
    (url, received)
}

struct Fixed;

impl Clock for Fixed {
    fn now(&self) -> Timestamp {
        Timestamp::UNIX_EPOCH
    }
}

const FEEDER: FeederId = FeederId(uuid::Uuid::from_u128(1));

/// A store holding one feeder, and the webhook on it towards `url`, with
/// retries a millisecond apart.
async fn webhook(url: Url) -> (TempDir, Store, Webhook) {
    let dir = tempfile::tempdir().expect("a directory");
    let store = Store::open(&dir.path().join("server.db"), Timestamp::UNIX_EPOCH)
        .await
        .expect("the store");
    let home = Home {
        id: HomeId(uuid::Uuid::from_u128(9)),
        name: "Home".into(),
        units: Units::Grams,
        time_format: TimeFormat::H24,
        zone: "UTC".into(),
    };
    store
        .create_home(&home, Timestamp::UNIX_EPOCH)
        .await
        .expect("a home");
    store
        .insert_feeder(NewFeeder {
            id: FEEDER,
            home: home.id,
            model: Model::Plaf103,
            identity: [1; 32],
            name: "kitchen",
            role: ClientRole::Server,
            address: None,
            key: SealedKey {
                nonce: [7; 12],
                sealed: vec![1, 2, 3],
            },
            created: Timestamp::UNIX_EPOCH,
        })
        .await
        .expect("a feeder");
    let target = Target::new(url, PlainHttp::Refused).expect("a loopback target");
    let (_, moved) = watch::channel(0);
    let mut webhook =
        Webhook::new(store.clone(), Some(target), Arc::new(Fixed), moved).expect("the webhook");
    webhook.backoff = vec![Duration::from_millis(1)];
    (dir, store, webhook)
}

/// The server reads the feeder's table at `generation`, showing `alerts`.
async fn read(store: &Store, generation: u32, alerts: &[Alert]) {
    read_in(store, 1, generation, alerts).await;
}

/// The same, in the ring `epoch`.
async fn read_in(store: &Store, epoch: u32, generation: u32, alerts: &[Alert]) {
    read_at(store, epoch, generation, alerts, Timestamp::UNIX_EPOCH).await;
}

/// The same, at `now` by the server's clock.
async fn read_at(store: &Store, epoch: u32, generation: u32, alerts: &[Alert], now: Timestamp) {
    let mutes = Mutes::default();
    let context = Context {
        now,
        mutes: &mutes,
        own: Hex([0; 4]),
        clock: FeederClock::Known,
    };
    let view = AlertsView {
        epoch,
        generation: Generation(generation),
        more: false,
        settling: false,
        alerts: alerts
            .iter()
            .map(|alert| OpenAlert {
                at: None,
                alert: *alert,
            })
            .collect(),
        unread: 0,
    };
    let table = ReadTable {
        cursor: Cursor {
            epoch,
            boot: 1,
            generation: Generation(generation),
        },
        view,
        clock_valid: true,
        at: Timestamp::UNIX_EPOCH,
    };
    wellfed_mirror::change_kept(store, FEEDER, Timestamp::UNIX_EPOCH, |kept| {
        kept.take(table.clone(), &context);
    })
    .await
    .expect("the kept alerts");
}

async fn kept(store: &Store) -> Kept {
    wellfed_mirror::load_kept(store, FEEDER)
        .await
        .expect("the kept alerts")
}

/// Pass until the feeder has nothing left to deliver.
async fn drain(webhook: &mut Webhook, store: &Store) {
    for _ in 0..100 {
        webhook.pass().await;
        if kept(store).await.watch.changes().is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    panic!("the changes were never done");
}

/// The food runs low after the first table, and ends after; what the
/// receiver was sent, in order.
async fn open_then_end(answers: Vec<u16>) -> (Vec<String>, Kept, Webhook) {
    let (url, mut bodies) = receiver(answers).await;
    let (_dir, store, mut webhook) = webhook(url).await;
    read(&store, 1, &[]).await;
    read(&store, 2, &[Alert::FoodLow]).await;
    drain(&mut webhook, &store).await;
    read(&store, 3, &[]).await;
    drain(&mut webhook, &store).await;

    let mut events = Vec::new();
    while let Ok(body) = bodies.try_recv() {
        events.push(body["event"].as_str().expect("an event").to_owned());
    }
    (events, kept(&store).await, webhook)
}

#[tokio::test]
async fn an_open_taken_is_cleared_once() {
    let (events, kept, webhook) = open_then_end(vec![200]).await;
    assert_eq!(events, ["opened", "cleared"]);
    assert!(kept.hooked.is_empty());
    assert!(webhook.pending.is_empty());
}

#[tokio::test]
async fn an_open_refused_is_never_cleared() {
    let (events, kept, _) = open_then_end(vec![400, 200]).await;
    assert_eq!(
        events,
        ["opened"],
        "refused once, never tried again or cleared"
    );
    assert!(kept.hooked.is_empty());
}

#[tokio::test]
async fn an_open_given_up_is_never_cleared() {
    let (events, kept, webhook) =
        open_then_end(vec![503; 7].into_iter().chain([200]).collect()).await;
    let tries = usize::try_from(TRIES).expect("small");
    assert_eq!(events, vec!["opened"; tries], "every try, then nothing");
    assert!(kept.hooked.is_empty());
    assert!(webhook.pending.is_empty());
}

#[tokio::test]
async fn a_change_gone_before_its_retry_is_forgotten() {
    let (url, mut bodies) = receiver(vec![503]).await;
    let (_dir, store, mut webhook) = webhook(url).await;
    read(&store, 1, &[]).await;
    read(&store, 2, &[Alert::FoodLow]).await;
    webhook.pass().await;
    assert_eq!(webhook.pending.len(), 1);

    // It ended before its retry: never posted, so nothing to take off.
    read(&store, 3, &[]).await;
    webhook.pass().await;
    assert!(webhook.pending.is_empty());
    assert!(kept(&store).await.watch.changes().is_empty());
    assert_eq!(bodies.try_recv().map(|_| ()), Ok(()));
    assert!(bodies.try_recv().is_err());
}

#[tokio::test]
async fn a_feeder_archived_and_adopted_again_starts_afresh() {
    let (url, mut bodies) = receiver(vec![200]).await;
    let (_dir, store, mut webhook) = webhook(url).await;
    read(&store, 1, &[]).await;
    read(&store, 2, &[Alert::FoodLow]).await;
    drain(&mut webhook, &store).await;
    assert_eq!(bodies.try_recv().expect("the open")["event"], "opened");
    assert_eq!(kept(&store).await.hooked.len(), 1);

    store
        .archive_feeder(FEEDER, Timestamp::UNIX_EPOCH)
        .await
        .expect("archived");
    assert_eq!(
        kept(&store).await,
        Kept::default(),
        "dropped as it is removed"
    );
    // A reader's last write, after the archive, comes back; the adoption
    // drops it again.
    read(&store, 3, &[Alert::FoodLow]).await;
    let row = store.feeder(FEEDER).await.expect("read").expect("kept");
    let readopted = store
        .readopt_feeder(NewFeeder {
            id: FEEDER,
            home: row.home,
            model: Model::Plaf103,
            identity: [1; 32],
            name: "kitchen",
            role: ClientRole::Server,
            address: None,
            key: SealedKey {
                nonce: [8; 12],
                sealed: vec![4, 5, 6],
            },
            created: Timestamp::UNIX_EPOCH,
        })
        .await
        .expect("adopted again");
    assert!(readopted);
    assert_eq!(kept(&store).await, Kept::default());

    // Its first table, in a new ring, is taken quietly. When those alerts
    // end, their clears go (they were open as it was adopted), but none for
    // the open told before the removal.
    read_in(&store, 2, 1, &[Alert::FoodLow, Alert::NoClock]).await;
    drain(&mut webhook, &store).await;
    assert!(bodies.try_recv().is_err(), "nothing opened");
    read_in(&store, 2, 2, &[]).await;
    drain(&mut webhook, &store).await;
    let events: Vec<_> = std::iter::from_fn(|| bodies.try_recv().ok()).collect();
    assert_eq!(events.len(), 2, "{events:?}");
    for event in &events {
        assert_eq!(event["event"], "cleared");
        assert!(
            event["alert"]["key"]
                .as_str()
                .is_some_and(|key| key.starts_with("2:")),
            "{event}"
        );
    }
}

#[tokio::test]
async fn a_retry_waiting_holds_the_tries_of_the_changes_behind_it() {
    let (url, _bodies) = receiver(vec![503]).await;
    let (_dir, store, mut webhook) = webhook(url).await;
    read(&store, 1, &[]).await;
    read(&store, 2, &[Alert::FoodLow, Alert::NoClock]).await;
    webhook.pass().await;
    assert_eq!(webhook.pending.len(), 1, "the first change is in backoff");

    // The second change was tried before (a receiver's earlier answers,
    // say); a pass that stops at the first must not forget it.
    webhook.backoff = vec![Duration::from_secs(3600)];
    let kept = kept(&store).await;
    let changes = kept.watch.changes();
    let (transition, tracked) = sendable(&kept, &changes[1]).expect("sendable");
    let behind = (FEEDER, delivery_id(FEEDER, transition, &tracked));
    webhook.pending.insert(
        behind.clone(),
        Pending {
            tries: TRIES - 1,
            next: Instant::now(),
            time: Timestamp::UNIX_EPOCH,
        },
    );
    if let Some(first) = webhook
        .pending
        .values_mut()
        .find(|pending| pending.tries == 1)
    {
        first.next = Instant::now() + Duration::from_secs(3600);
    }
    webhook.pass().await;

    assert_eq!(
        webhook.pending.get(&behind).map(|pending| pending.tries),
        Some(TRIES - 1),
        "its tries are kept, so the cap stays exact"
    );
}

#[tokio::test]
async fn a_removed_feeder_is_sent_nothing_more() {
    let (url, mut bodies) = receiver(vec![200]).await;
    let (_dir, store, mut webhook) = webhook(url).await;
    read(&store, 1, &[]).await;
    read(&store, 2, &[Alert::FoodLow]).await;
    drain(&mut webhook, &store).await;
    assert_eq!(bodies.try_recv().expect("the open")["event"], "opened");

    store
        .archive_feeder(FEEDER, Timestamp::UNIX_EPOCH)
        .await
        .expect("archived");
    assert_eq!(
        kept(&store).await,
        Kept::default(),
        "dropped as it is removed"
    );
    read(&store, 3, &[]).await;
    webhook.pass().await;
    assert!(bodies.try_recv().is_err(), "no clear for a removed feeder");
}

/// A body's event, with its alert's kind.
fn event(body: &serde_json::Value) -> (String, String) {
    let text = |value: &serde_json::Value| value.as_str().expect("text").to_owned();
    (text(&body["event"]), text(&body["alert"]["kind"]))
}

/// The receiver's events in order.
fn events(bodies: &mut mpsc::UnboundedReceiver<serde_json::Value>) -> Vec<(String, String)> {
    std::iter::from_fn(|| bodies.try_recv().ok())
        .map(|body| event(&body))
        .collect()
}

/// An alert opens and is delivered, held at the receiver while `meanwhile`
/// changes the table; then everything left is delivered.
async fn changed_in_flight(
    answers: Vec<u16>,
    opened: Alert,
    meanwhile: impl AsyncFnOnce(&Store),
) -> Vec<(String, String)> {
    let (release, gate) = mpsc::unbounded_channel();
    let (url, mut bodies) = held_receiver(answers, Some(gate)).await;
    let (_dir, store, mut webhook) = webhook(url).await;
    read(&store, 1, &[]).await;
    read(&store, 2, &[opened]).await;

    let mut first = None;
    let change = async {
        first = Some(bodies.recv().await.expect("the open is on the way"));
        meanwhile(&store).await;
        drop(release);
    };
    tokio::join!(webhook.pass(), change);
    drain(&mut webhook, &store).await;

    let mut sent = vec![event(&first.expect("the open"))];
    sent.extend(events(&mut bodies));
    sent
}

fn later(minutes: i64) -> Timestamp {
    Timestamp::UNIX_EPOCH + jiff::SignedDuration::from_mins(minutes)
}

#[tokio::test]
async fn an_alert_reopened_while_its_open_is_in_flight_is_sent_again() {
    let events = changed_in_flight(vec![200], Alert::FoodLow, async |store| {
        read_at(store, 1, 3, &[], later(1)).await;
        read_at(store, 1, 4, &[Alert::FoodLow], later(2)).await;
    })
    .await;
    let kinds = |event: &str| (event.to_owned(), "food_low".to_owned());
    assert_eq!(
        events,
        [kinds("opened"), kinds("cleared"), kinds("opened")],
        "the reopening is not taken for the open on the way"
    );
}

#[tokio::test]
async fn an_alert_whose_kind_changes_while_its_open_is_in_flight_is_sent_again() {
    let due = Timestamp::UNIX_EPOCH;
    let no_record = Alert::MealNoRecord { plan: 1, due };
    let events = changed_in_flight(vec![200], no_record, async |store| {
        let missed = Alert::MealMissed {
            plan: 1,
            due,
            reason: wellfed_model::MissedReason::Late,
        };
        read_at(store, 1, 3, &[missed], later(1)).await;
    })
    .await;
    let event = |kind: &str| ("opened".to_owned(), kind.to_owned());
    assert_eq!(events, [event("meal_no_record"), event("meal_missed")]);
}
