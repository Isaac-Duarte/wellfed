//! A body a newer feeder sent, which a server of an older device API
//! revision stored unread (with no kind), is read again by a server whose
//! revision is higher, at its next start.
//!
//! As in `projections.rs`, the store keeps whatever bytes it is given as the
//! raw body; these tests store the event's JSON and decode it back.

use jiff::Timestamp;
use tempfile::TempDir;
use uuid::Uuid;
use wellfed_api::{
    ClientRole, EventBody, EventKind, FeederEvent, FeederId, Home, HomeId, Model, Source,
    TimeFormat, Units,
};
use wellfed_store::{NewEvent, NewFeeder, Refresh, SealedKey, Store};

/// The device API revision the tests' server reads.
const REVISION: u32 = 1;
const HOME: HomeId = HomeId(Uuid::from_u128(0x40));

fn at(ms: i64) -> Timestamp {
    Timestamp::from_millisecond(ms).unwrap()
}

fn decode(raw: &[u8]) -> Option<EventBody> {
    serde_json::from_slice(raw).ok()
}

async fn fresh() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("server.db"), at(0))
        .await
        .unwrap();
    let home = Home {
        id: HOME,
        name: "Home".into(),
        units: Units::Grams,
        time_format: TimeFormat::H24,
        zone: "UTC".into(),
    };
    store
        .create_home(&home, Timestamp::UNIX_EPOCH)
        .await
        .unwrap();
    (dir, store)
}

async fn adopt(store: &Store, n: u8) -> FeederId {
    let id = FeederId(Uuid::from_u128(u128::from(n) + 1));
    store
        .insert_feeder(NewFeeder {
            id,
            home: HOME,
            model: Model::Plaf103,
            identity: [n; 32],
            name: "kitchen",
            role: ClientRole::Server,
            address: None,
            key: SealedKey {
                nonce: [7; 12],
                sealed: vec![1, 2, 3],
            },
            created: at(2_000),
        })
        .await
        .unwrap();
    id
}

/// A body a newer feeder sent, which the server stored unread, is read
/// again once the server's device API revision rises: it takes its kind and
/// its projection, and stays read.
#[tokio::test]
async fn a_risen_revision_reads_the_unread_rows_again() {
    let (_dir, store) = fresh().await;
    let feeders = [adopt(&store, 1).await];
    store.refresh_projections(decode, REVISION).await.unwrap();
    let lock = EventBody::Feeder(FeederEvent::Lock {
        locked: true,
        by: Source::Feeder,
    });
    let raw = serde_json::to_vec(&lock).unwrap();
    store
        .record_event(
            feeders[0],
            NewEvent {
                epoch: 1,
                seq: 1,
                at: None,
                received: at(1_000),
                body: None,
                raw: &raw,
            },
        )
        .await
        .unwrap();
    let locks = |store: Store| async move {
        store
            .lock_changes(feeders[0], Timestamp::MIN, Timestamp::MAX)
            .await
            .unwrap()
    };
    assert!(locks(store.clone()).await.is_empty(), "stored unread");

    // The same revision reads nothing again; a decoder that still cannot
    // read it leaves it unread, counted.
    assert_eq!(
        store.refresh_projections(decode, REVISION).await.unwrap(),
        Refresh::Current
    );
    assert_eq!(
        store
            .refresh_projections(|_| None, REVISION + 1)
            .await
            .unwrap(),
        Refresh::Reread { read: 0, unread: 1 }
    );
    assert_eq!(
        store
            .refresh_projections(decode, REVISION + 2)
            .await
            .unwrap(),
        Refresh::Reread { read: 1, unread: 0 }
    );
    assert_eq!(locks(store.clone()).await.len(), 1);
    let rows = store.events(feeders[0], 1, None, 10, None).await.unwrap();
    assert_eq!(rows[0].kind, Some(EventKind::Lock));
    assert_eq!(
        store
            .refresh_projections(decode, REVISION + 2)
            .await
            .unwrap(),
        Refresh::Current
    );
}

/// A downgraded server records its lower revision, so the rows it stores
/// unread are read again when the newer server starts once more.
#[tokio::test]
async fn a_downgrade_then_an_upgrade_reads_the_rows_stored_between() {
    let (_dir, store) = fresh().await;
    let feeders = [adopt(&store, 1).await];
    store
        .refresh_projections(decode, REVISION + 1)
        .await
        .unwrap();

    // The older server starts and stores a body it cannot read.
    assert_eq!(
        store.refresh_projections(|_| None, REVISION).await.unwrap(),
        Refresh::Current
    );
    let lock = EventBody::Feeder(FeederEvent::Lock {
        locked: true,
        by: Source::Feeder,
    });
    let raw = serde_json::to_vec(&lock).unwrap();
    store
        .record_event(
            feeders[0],
            NewEvent {
                epoch: 1,
                seq: 1,
                at: None,
                received: at(1_000),
                body: None,
                raw: &raw,
            },
        )
        .await
        .unwrap();

    assert_eq!(
        store
            .refresh_projections(decode, REVISION + 1)
            .await
            .unwrap(),
        Refresh::Reread { read: 1, unread: 0 }
    );
    let rows = store.events(feeders[0], 1, None, 10, None).await.unwrap();
    assert_eq!(rows[0].kind, Some(EventKind::Lock));
}
