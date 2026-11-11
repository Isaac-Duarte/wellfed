//! What a newer server left, read by this one. A database a newer build
//! migrated opens read-only, and refuses a caller that writes. Without a
//! migration (a device API revision grows without one) a server rolled back
//! to an older build reads rows the newer one wrote: each code it does not
//! know reads as unknown, as the device API's own lenient enums do, and the
//! rows beside it read as ever.

use jiff::Timestamp;
use uuid::Uuid;
use wellfed_api::{
    Blocked, ClientRole, EventKind, FeedOutcome, FeedReason, FeederId, Home, HomeId, Model, Source,
    TimeFormat, Units,
};
use wellfed_store::{Error, NewFeeder, Schema, SealedKey, Store};

const HOME: HomeId = HomeId(Uuid::from_u128(0x40));
const FEEDER: FeederId = FeederId(Uuid::from_u128(1));

fn at(ms: i64) -> Timestamp {
    Timestamp::from_millisecond(ms).unwrap()
}

async fn adopted(path: &std::path::Path) -> Store {
    let store = Store::open(path, at(0)).await.unwrap();
    let home = Home {
        id: HOME,
        name: "Home".into(),
        units: Units::Grams,
        time_format: TimeFormat::H24,
        zone: "UTC".into(),
    };
    store.create_home(&home, at(0)).await.unwrap();
    store
        .insert_feeder(NewFeeder {
            id: FEEDER,
            home: HOME,
            model: Model::Plaf103,
            identity: [1; 32],
            name: "kitchen",
            role: ClientRole::Server,
            address: None,
            key: SealedKey {
                nonce: [7; 12],
                sealed: vec![1, 2, 3],
            },
            created: at(1),
        })
        .await
        .unwrap();
    store
}

async fn insert(pool: &sqlx::SqlitePool, sql: &'static str) {
    sqlx::query(sql).bind(FEEDER.0).execute(pool).await.unwrap();
}

#[tokio::test]
async fn codes_a_newer_server_stored_read_as_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let store = adopted(&path).await;

    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    insert(
        &pool,
        "INSERT INTO events (feeder_id, epoch, seq, at_s, received_ms, kind, body)
         VALUES (?, 1, 1, NULL, 10, 'a_newer_kind', x'00')",
    )
    .await;
    insert(
        &pool,
        "INSERT INTO events (feeder_id, epoch, seq, at_s, received_ms, kind, body)
         VALUES (?, 1, 2, NULL, 11, 'boot', x'00')",
    )
    .await;
    insert(
        &pool,
        "INSERT INTO feeds (feeder_id, epoch, feed, seq, source, client, requested, completed,
                            outcome, reason, at_s, received_ms)
         VALUES (?, 1, 1, 3, 'a_newer_source', NULL, 2, 1, 'a_newer_outcome', 'a_newer_reason',
                 NULL, 20)",
    )
    .await;
    insert(
        &pool,
        "INSERT INTO blocked_changes (feeder_id, epoch, seq, reason, at_s, received_ms)
         VALUES (?, 1, 4, 'a_newer_reason', NULL, 30)",
    )
    .await;

    let events = store.events(FEEDER, 1, None, 10, None).await.unwrap();
    let kinds: Vec<_> = events.iter().map(|event| event.kind).collect();
    assert_eq!(kinds, [None, Some(EventKind::Boot)], "read as unread");

    let feeds = store
        .feeds(FEEDER, Timestamp::MIN, Timestamp::MAX)
        .await
        .unwrap();
    let [feed] = feeds.as_slice() else {
        panic!("{feeds:?}");
    };
    assert_eq!(feed.value.by, Source::Unknown);
    assert_eq!(feed.value.outcome, FeedOutcome::Unknown);
    assert_eq!(feed.value.reason, FeedReason::Unknown);
    assert_eq!((feed.value.requested, feed.value.completed), (2, 1));
    let found = store.feed_result(FEEDER, 1, 1).await.unwrap();
    assert_eq!(found.as_ref(), Some(feed));

    let blocked = store
        .blocked_changes(FEEDER, Timestamp::MIN, Timestamp::MAX)
        .await
        .unwrap();
    let values: Vec<_> = blocked.iter().map(|change| change.value).collect();
    assert_eq!(values, [Some(Blocked::Unknown)]);

    // A source this server knows, damaged, is still damage.
    insert(
        &pool,
        "INSERT INTO feeds (feeder_id, epoch, feed, seq, source, client, requested, completed,
                            outcome, reason, at_s, received_ms)
         VALUES (?, 1, 2, 5, 'client', NULL, 1, 1, 'done', 'done', NULL, 40)",
    )
    .await;
    let damaged = store.feeds(FEEDER, Timestamp::MIN, Timestamp::MAX).await;
    assert!(matches!(damaged, Err(Error::Corrupt(_))), "{damaged:?}");

    pool.close().await;
    store.close().await;
}

fn migrations() -> i64 {
    let count = std::fs::read_dir(env!("CARGO_MANIFEST_DIR").to_owned() + "/migrations")
        .unwrap()
        .count();
    i64::try_from(count).unwrap()
}

/// A database whose record names a migration this build lacks opens
/// read-only: it reads, every write is refused, nothing runs or is copied,
/// a caller that writes is refused, and the file is as that build left it.
#[tokio::test]
async fn a_database_a_newer_build_migrated_opens_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    adopted(&path).await.close().await;

    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
         VALUES (9999, 'from a newer build', 1, x'00', 0)",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    let before = std::fs::read(&path).unwrap();

    let store = Store::open(&path, at(1)).await.unwrap();

    let newer = Schema::Newer {
        applied: 9999,
        embedded: migrations(),
    };
    assert_eq!(store.schema(), newer);
    assert!(store.feeder(FEEDER).await.unwrap().is_some(), "it reads");
    let written = store.archive_feeder(FEEDER, at(2)).await;
    assert!(matches!(written, Err(Error::Sqlx(_))), "{written:?}");
    let refused = store.clone().writable().await;
    assert!(
        matches!(refused, Err(Error::Newer { applied: 9999, .. })),
        "{refused:?}"
    );
    store.close().await;

    let copies = std::fs::read_dir(dir.path())
        .unwrap()
        .filter(|entry| {
            let name = entry.as_ref().unwrap().file_name();
            name.to_string_lossy().contains(".before-")
        })
        .count();
    assert_eq!(copies, 0, "nothing ran, so nothing was copied");
    assert_eq!(std::fs::read(&path).unwrap(), before, "left as it was");
}

#[tokio::test]
async fn a_current_database_is_writable() {
    let dir = tempfile::tempdir().unwrap();
    let store = adopted(&dir.path().join("server.db")).await;

    assert_eq!(store.schema(), Schema::Current);
    let store = store.writable().await.unwrap();
    store.close().await;
}
