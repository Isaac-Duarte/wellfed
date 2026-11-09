use std::sync::Mutex;

use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use wellfed_api::{ClientRole, Home, HomeId, Model, TimeFormat, Units};
use wellfed_client::alerts::{Context, Cursor, FeederClock, Mutes, Notice, NoticeKey, Tracked};
use wellfed_mirror::ReadTable;
use wellfed_model::{Alert, AlertsView, Generation, Hex, OpenAlert};
use wellfed_store::{NewFeeder, SealedKey};

use super::*;

fn tracked(delivery: Shown) -> Tracked {
    Tracked {
        key: NoticeKey::Unreachable,
        notice: Notice::Alert {
            open: OpenAlert {
                at: None,
                alert: Alert::FoodLow,
            },
        },
        seen: Timestamp::UNIX_EPOCH,
        delivery,
        absent: None,
    }
}

#[test]
fn only_an_open_the_receiver_took_is_kept_for_its_clear() {
    let mut hooked = Vec::new();
    hook(&mut hooked, &Change::Post(tracked(Shown::Due)), Told::Not);
    assert!(hooked.is_empty(), "refused, given up or never sent");

    hook(&mut hooked, &Change::Post(tracked(Shown::Due)), Told::Took);
    assert_eq!(hooked.len(), 1);
    hook(&mut hooked, &Change::Post(tracked(Shown::Due)), Told::Took);
    assert_eq!(hooked.len(), 1, "an alert posted again replaces itself");
    hook(&mut hooked, &Change::Post(tracked(Shown::Due)), Told::Not);
    assert!(hooked.is_empty(), "its new kind was not taken: no clear");

    hook(&mut hooked, &Change::Post(tracked(Shown::Due)), Told::Took);
    hook(
        &mut hooked,
        &Change::Remove(NoticeKey::Unreachable),
        Told::Took,
    );
    assert!(hooked.is_empty());
}

#[test]
fn every_try_of_a_change_has_one_id_and_each_change_its_own() {
    let feeder = FeederId(uuid::Uuid::from_u128(1));
    let open = tracked(Shown::Due);
    let first = delivery_id(feeder, Transition::Opened, &open);
    assert_eq!(first, delivery_id(feeder, Transition::Opened, &open));
    assert_eq!(first.len(), 32);
    assert_ne!(first, delivery_id(feeder, Transition::Cleared, &open));
    let later = Tracked {
        seen: Timestamp::from_second(60).expect("a time"),
        ..open
    };
    assert_ne!(first, delivery_id(feeder, Transition::Opened, &later));
}

/// A receiver on loopback that answers every POST with the next status
/// of `answers` (the last one once they run out) and hands each body over.
async fn receiver(answers: Vec<u16>) -> (Url, mpsc::UnboundedReceiver<serde_json::Value>) {
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
    let mutes = Mutes::default();
    let context = Context {
        now: Timestamp::UNIX_EPOCH,
        mutes: &mutes,
        own: Hex([0; 4]),
        clock: FeederClock::Known,
    };
    let view = AlertsView {
        epoch: 1,
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
            epoch: 1,
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
    read(&store, 3, &[]).await;
    webhook.pass().await;
    assert!(bodies.try_recv().is_err(), "no clear for a removed feeder");
    assert_eq!(kept(&store).await, Kept::default(), "it starts afresh");
}
