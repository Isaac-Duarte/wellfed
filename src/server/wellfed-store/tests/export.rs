//! Every exported table reads out of a snapshot in its key's order and
//! inserts into a fresh database as it was, every column included; an
//! import archives the feeders, gives the accounts no password, and is
//! refused where an account exists.

use futures_util::TryStreamExt;
use jiff::Timestamp;
use sqlx::SqlitePool;
use tempfile::TempDir;
use wellfed_formats::export::{Row, Table};
use wellfed_store::{Decoders, Store};

/// A row in every exported table, and a second with every nullable column
/// null where the table has one, beside the rows that are not exported.
const FILL: &str = "
INSERT INTO users VALUES (x'0192c5d0000070008000000000000001', 'ada', 'hash-a', 'owner', 1, NULL);
INSERT INTO users VALUES (x'0192c5d0000070008000000000000002', 'bob', 'hash-b', 'member', 2, 99);
INSERT INTO sessions VALUES (x'aa', x'0192c5d0000070008000000000000001', 1, 2);
INSERT INTO tokens VALUES (x'0192c5d0000070008000000000000009', x'bb',
    x'0192c5d0000070008000000000000001', 'phone', 1, NULL, NULL);
INSERT INTO homes (id, name, units, time_format, zone, created_ms)
    VALUES (x'0192c5d0000070008000000000000005', 'Home', 'grams', '24h', 'Europe/Berlin', 1);
INSERT INTO feeders (id, home_id, kind, model, identity, name, role, address, key_nonce, key_sealed,
        synced, epoch, created_ms)
    VALUES (x'0192c5d0000070008000000000000003', x'0192c5d0000070008000000000000005', 'feeder',
        'plaf103', x'0101010101010101010101010101010101010101010101010101010101010101',
        'kitchen', 'server', '192.0.2.7:4444', x'070707070707070707070707', x'0102', 5, 9, 3);
INSERT INTO feeders (id, home_id, kind, model, identity, name, role, address, key_nonce, key_sealed,
        synced, epoch, created_ms)
    VALUES (x'0192c5d0000070008000000000000004', x'0192c5d0000070008000000000000005', 'feeder',
        'plaf103', x'0202020202020202020202020202020202020202020202020202020202020202',
        'hall', 'owner', NULL, x'070707070707070707070707', x'03', NULL, NULL, 4);
INSERT INTO phone_keys VALUES (x'0192c5d0000070008000000000000003',
    x'5151515151515151515151515151515151515151515151515151515151515151',
    x'0192c5d0000070008000000000000001',
    x'0192c5d0000070008000000000000009', 'member', 60, 61, 'token_revoked', 62, '{}', x'53535353535353', 3);
INSERT INTO phone_keys VALUES (x'0192c5d0000070008000000000000003',
    x'5050505050505050505050505050505050505050505050505050505050505050',
    x'0192c5d0000070008000000000000002',
    NULL, 'owner', 63, 64, NULL, NULL, NULL, NULL, 0);
INSERT INTO kept_clients VALUES (x'0192c5d0000070008000000000000003',
    x'5252525252525252525252525252525252525252525252525252525252525252', 65);
INSERT INTO invitations VALUES (4, x'0192c5d0000070008000000000000003', 9, 10, x'51515151515151',
    x'53535353535353', 66, 67);
INSERT INTO invitations VALUES (7, x'0192c5d0000070008000000000000003', 9, 11, x'54545454545454',
    x'53535353535353', 68, NULL);
INSERT INTO dropped_phones VALUES (x'0192c5d0000070008000000000000003', x'53535353535353',
    x'0192c5d0000070008000000000000001', x'0192c5d0000070008000000000000009', 61, 69);
INSERT INTO dropped_phones VALUES (x'0192c5d0000070008000000000000003', x'55555555555555',
    x'0192c5d0000070008000000000000002', NULL, 62, 70);
INSERT INTO feeder_state VALUES (x'0192c5d0000070008000000000000003', '{\"a\":1}', 10, '{\"b\":2}', 11,
    '{\"c\":3}', 12);
INSERT INTO feeder_state VALUES (x'0192c5d0000070008000000000000004', NULL, NULL, NULL, NULL, NULL, NULL);
INSERT INTO events VALUES (x'0192c5d0000070008000000000000003', 9, 2, 100, 12, 'fed', x'00ff');
INSERT INTO events VALUES (x'0192c5d0000070008000000000000003', 9, 1, NULL, 11, 'boot', x'01');
INSERT INTO gaps VALUES (x'0192c5d0000070008000000000000003', 9, 3, 4, 13);
INSERT INTO samples (id, feeder_id, received_ms, uptime_ms, metric, number, value)
    VALUES (7, x'0192c5d0000070008000000000000003', 14, 15, 'rssi', -60, x'05');
INSERT INTO samples (id, feeder_id, received_ms, uptime_ms, metric, number, value)
    VALUES (8, x'0192c5d0000070008000000000000003', 16, 17, 'power', NULL, x'06');
INSERT INTO sample_rollups VALUES (x'0192c5d0000070008000000000000003', 'rssi', 0, 1, -60, -60, -60);
UPDATE sample_marks SET rolled = 7, version = 1;
INSERT INTO projection_version VALUES (0, 3, 1);
INSERT INTO feeds VALUES (x'0192c5d0000070008000000000000003', 9, 1, 2, 'client', x'0a0b0c0d',
    2, 2, 'done', 'requested', 100, 12);
INSERT INTO feeds VALUES (x'0192c5d0000070008000000000000003', 9, 2, 3, 'panel', NULL,
    1, 0, 'failed', 'jam', NULL, 13);
INSERT INTO lock_changes VALUES (x'0192c5d0000070008000000000000003', 9, 4, 1, 'client', x'0a0b0c0d', 101, 14);
INSERT INTO lock_changes VALUES (x'0192c5d0000070008000000000000003', 9, 5, 0, 'panel', NULL, NULL, 15);
INSERT INTO boots VALUES (x'0192c5d0000070008000000000000003', 9, 1, 1, 4, NULL, 11);
INSERT INTO clock_syncs VALUES (x'0192c5d0000070008000000000000003', 9, 6, 'client', x'0a0b0c0d', -20, 102, 16);
INSERT INTO clock_syncs VALUES (x'0192c5d0000070008000000000000003', 9, 7, 'feeder', NULL, NULL, NULL, 17);
INSERT INTO blocked_changes VALUES (x'0192c5d0000070008000000000000003', 9, 8, 'empty', 103, 18);
INSERT INTO blocked_changes VALUES (x'0192c5d0000070008000000000000003', 9, 9, NULL, NULL, 19);
INSERT INTO feed_traces VALUES (x'0192c5d0000070008000000000000003', 9, 1, 50,
    '{\"feed\":1,\"requested\":2,\"completed\":2,\"outcome\":\"done\",\"reason\":\"requested\",
      \"drive_ms\":10,\"samples\":1,\"unit\":\"scaled\",\"current_peak\":5,\"rising\":1,
      \"falling\":1,\"beam_breaks\":0,\"reversals\":0,\"stride\":1,\"truncated\":false}',
    1, x'0c', NULL);
INSERT INTO feed_traces VALUES (x'0192c5d0000070008000000000000003', 9, 2, 51, NULL, NULL, NULL, '\"busy\"');
INSERT INTO feeder_stats VALUES (x'0192c5d0000070008000000000000003', 'food', 52, 4,
    '{\"section\":\"food\",\"present\":true,\"ran_low\":1,\"refilled\":2,\"feeds_since_refill\":3}');
INSERT INTO feeder_stats VALUES (x'0192c5d0000070008000000000000003', 'motor', 53, NULL,
    '{\"section\":\"food\",\"ran_low\":0,\"refilled\":0}');
INSERT INTO food_readings VALUES (x'0192c5d0000070008000000000000003', 55, 4, 1, 2, 3, 4);
INSERT INTO food_readings VALUES (x'0192c5d0000070008000000000000003', 54, NULL, NULL, 0, 0, NULL);
INSERT INTO commands (seq, key, user_id, token_id, feeder_id, kind, body_hash, request_id, boot_count,
        state, outcome, feed, sent_ms, updated_ms, history_epoch, history_seq)
    VALUES (1, x'11111111111111111111111111111111', x'0192c5d0000070008000000000000001',
        x'0192c5d0000070008000000000000009', x'0192c5d0000070008000000000000003', 'feed',
        x'2222222222222222222222222222222222222222222222222222222222222222',
        x'33333333333333333333333333333333', 4, 'resolved', '{}', 1, 20, 21, 9, 1);
INSERT INTO commands (seq, key, user_id, token_id, feeder_id, kind, body_hash, request_id, boot_count,
        state, outcome, feed, sent_ms, updated_ms, history_epoch, history_seq)
    VALUES (2, x'12121212121212121212121212121212', x'0192c5d0000070008000000000000002', NULL,
        x'0192c5d0000070008000000000000003', 'lock',
        x'2323232323232323232323232323232323232323232323232323232323232323',
        x'34343434343434343434343434343434', NULL, 'unknown', NULL, NULL, 22, 23, NULL, NULL);
INSERT INTO installs VALUES (x'0192c5d0000070008000000000000005', x'0192c5d0000070008000000000000003',
    x'4444444444444444444444444444444444444444444444444444444444444444', 7,
    x'0192c5d0000070008000000000000001', 24, 'failed', 10, 20, '{\"kind\":\"lost\"}', 25, 'release');
INSERT INTO installs VALUES (x'0192c5d0000070008000000000000006', x'0192c5d0000070008000000000000003',
    x'4545454545454545454545454545454545454545454545454545454545454545', 8,
    x'0192c5d0000070008000000000000001', 26, 'activated', 20, 20, NULL, NULL, 'stock');
INSERT INTO pets VALUES (x'0192c5d0000070008000000000000007', 'Tom', 'cat', '2020-01-02', 250, 27);
INSERT INTO pets VALUES (x'0192c5d0000070008000000000000008', 'Rex', 'dog', NULL, NULL, 28);
INSERT INTO pet_weights VALUES (x'0192c5d000007000800000000000000a', x'0192c5d0000070008000000000000007', 29, 4200);
INSERT INTO pet_feeders VALUES (1, x'0192c5d0000070008000000000000007', x'0192c5d0000070008000000000000003', 30, 31);
INSERT INTO pet_feeders VALUES (2, x'0192c5d0000070008000000000000007', x'0192c5d0000070008000000000000003', 32, NULL);
INSERT INTO foods VALUES (x'0192c5d000007000800000000000000b', 'Acme', 'Dry', 3600, 2000, 33);
INSERT INTO feeder_foods VALUES (1, x'0192c5d0000070008000000000000003', x'0192c5d000007000800000000000000b', 34);
INSERT INTO refills VALUES (1, x'0192c5d000007000800000000000000c', x'0192c5d0000070008000000000000003',
    35, 2000, 1, x'0192c5d000007000800000000000000b', x'0192c5d0000070008000000000000001');
INSERT INTO refills VALUES (2, x'0192c5d000007000800000000000000d', x'0192c5d0000070008000000000000003',
    36, 50, 0, NULL, x'0192c5d0000070008000000000000002');
INSERT INTO audit (seq, at_ms, user_id, user_name, token_id, action, command_seq)
    VALUES (5, 40, x'0192c5d0000070008000000000000001', 'ada', NULL, '{\"kind\":\"user_added\"}', NULL);
INSERT INTO job_runs VALUES ('backup', 1, 2, 'ok');
";

/// Bytes no decoder reads, which [`DECODERS`] refuses and every other
/// value passes, so the fixture's short bodies stand for real ones.
const UNDECODABLE: &[u8] = b"undecodable";

const DECODERS: Decoders = Decoders {
    event: |body| body != UNDECODABLE,
    sample: |value| value != UNDECODABLE,
};

fn at(ms: i64) -> Timestamp {
    Timestamp::from_millisecond(ms).unwrap()
}

async fn open(dir: &TempDir, name: &str) -> (Store, SqlitePool) {
    let path = dir.path().join(name);
    let store = Store::open(&path, at(0)).await.unwrap();
    let pool = SqlitePool::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    (store, pool)
}

/// Every exported row, table by table, from one snapshot.
async fn export(store: &Store) -> Vec<(Table, Vec<Row>)> {
    let mut snapshot = store.snapshot().await.unwrap();
    let mut tables = Vec::new();
    for &table in Table::ALL {
        let rows: Vec<Row> = snapshot.rows(table).try_collect().await.unwrap();
        tables.push((table, rows));
    }
    tables
}

#[tokio::test]
async fn every_table_comes_out_in_order_and_goes_back_in_whole() {
    let dir = tempfile::tempdir().unwrap();
    let (first, pool) = open(&dir, "first.db").await;
    sqlx::raw_sql(FILL).execute(&pool).await.unwrap();

    let exported = export(&first).await;

    for (table, rows) in &exported {
        assert!(!rows.is_empty(), "{table:?} has rows");
    }
    let events = &exported
        .iter()
        .find(|(t, _)| *t == Table::Events)
        .unwrap()
        .1;
    let seqs: Vec<i64> = events
        .iter()
        .map(|row| match row {
            Row::Events(event) => event.seq,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(seqs, [1, 2], "in the key's order, not the insert's");

    let (second, raw) = open(&dir, "second.db").await;
    let mut import = second.begin_import("", DECODERS).await.unwrap().unwrap();
    for (_, rows) in &exported {
        for row in rows {
            import.insert(row).await.unwrap();
        }
    }
    let archived = import.commit(at(500)).await.unwrap();

    assert_eq!(archived, 2);
    assert_eq!(export(&second).await, exported);
    read_everything(&second).await;
    let passwords: Vec<String> = sqlx::query_scalar("SELECT password_hash FROM users")
        .fetch_all(&raw)
        .await
        .unwrap();
    assert_eq!(passwords, ["", ""]);
    let keys: Vec<(Vec<u8>, Vec<u8>, i64)> =
        sqlx::query_as("SELECT key_nonce, key_sealed, archived_ms FROM feeders")
            .fetch_all(&raw)
            .await
            .unwrap();
    assert_eq!(
        keys,
        [(vec![0; 12], vec![], 500), (vec![0; 12], vec![], 500)]
    );
    for (table, query) in [
        ("sessions", "SELECT COUNT(*) FROM sessions"),
        ("tokens", "SELECT COUNT(*) FROM tokens"),
        ("job_runs", "SELECT COUNT(*) FROM job_runs"),
    ] {
        let count: i64 = sqlx::query_scalar(query).fetch_one(&raw).await.unwrap();
        assert_eq!(count, 0, "{table} is not imported");
    }
    // The sample ids came along, and the next one follows them.
    sqlx::query(
        "INSERT INTO samples (feeder_id, received_ms, uptime_ms, metric, value)
         SELECT id, 0, 0, 'rssi', x'00' FROM feeders LIMIT 1",
    )
    .execute(&raw)
    .await
    .unwrap();
    let next: i64 = sqlx::query_scalar("SELECT MAX(id) FROM samples")
        .fetch_one(&raw)
        .await
        .unwrap();
    assert_eq!(next, 9);
}

/// Run the readers a start, a login and the owner's pages run over every
/// table: a committed import reads back without a refusal.
async fn read_everything(store: &Store) {
    store.home().await.unwrap().expect("the home");
    store.users().await.unwrap();
    store.phones(None).await.unwrap();
    store.pets().await.unwrap();
    store.foods().await.unwrap();
    let audit = wellfed_store::AuditFilter {
        user: None,
        after: None,
        limit: 100,
    };
    store.audit(audit).await.unwrap();
    let commands = wellfed_store::CommandFilter {
        user: None,
        feeder: None,
        state: None,
        after: None,
        limit: 100,
    };
    assert_eq!(store.commands(commands).await.unwrap().len(), 2);
    let feeders = store.feeders().await.unwrap();
    assert_eq!(feeders.len(), 2);
    for feeder in feeders {
        store.state(feeder.id).await.unwrap();
        store.kept_clients(feeder.id).await.unwrap();
        store.invitations(feeder.id).await.unwrap();
        store.latest_stats(feeder.id).await.unwrap();
        for feed in [1, 2] {
            store.trace(feeder.id, None, feed).await.unwrap();
        }
    }
}

#[tokio::test]
async fn an_import_is_refused_where_an_account_exists_and_rolls_back_when_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let (first, pool) = open(&dir, "first.db").await;
    sqlx::raw_sql(FILL).execute(&pool).await.unwrap();
    let exported = export(&first).await;

    assert!(first.begin_import("", DECODERS).await.unwrap().is_none());

    let (second, raw) = open(&dir, "second.db").await;
    let mut import = second.begin_import("", DECODERS).await.unwrap().unwrap();
    for row in &exported[0].1 {
        import.insert(row).await.unwrap();
    }
    drop(import);
    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&raw)
        .await
        .unwrap();
    assert_eq!(users, 0);
    assert!(second.begin_import("", DECODERS).await.unwrap().is_some());
}

/// A row with a value the store's readers would refuse is refused before it
/// goes in, and the import rolls back whole: an archive with such a value
/// would otherwise commit a server whose start, logins or pages fail on it.
#[tokio::test]
async fn a_value_the_readers_refuse_is_refused_and_nothing_is_imported() {
    let dir = tempfile::tempdir().unwrap();
    let (first, pool) = open(&dir, "first.db").await;
    sqlx::raw_sql(FILL).execute(&pool).await.unwrap();
    let exported = export(&first).await;

    for (i, what) in REFUSED.iter().copied().enumerate() {
        let (second, raw) = open(&dir, &format!("second-{i}.db")).await;
        let mut import = second.begin_import("", DECODERS).await.unwrap().unwrap();

        let mut refused = None;
        let mut edited = false;
        'rows: for (_, rows) in &exported {
            for row in rows {
                let mut row = row.clone();
                if !edited {
                    edited = unreadable(what, &mut row);
                }
                if let Err(error) = import.insert(&row).await {
                    refused = Some(error);
                    break 'rows;
                }
            }
        }
        drop(import);

        assert!(edited, "{what}: a row to edit");
        assert!(
            matches!(refused, Some(wellfed_store::Error::Unreadable(named)) if named == what),
            "{what}: {refused:?}"
        );
        let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&raw)
            .await
            .unwrap();
        assert_eq!(users, 0, "{what}: nothing kept");
        assert!(
            second.begin_import("", DECODERS).await.unwrap().is_some(),
            "{what}"
        );
    }
}

/// What a reader names in each refusal [`unreadable`] makes.
const REFUSED: &[&str] = &[
    "role",
    "units",
    "time format",
    "feeder role",
    "epoch",
    "cursor without its epoch",
    "cursor",
    "identity",
    "phone key",
    "phone role",
    "revoke reason",
    "revocation without its time",
    "kept client key",
    "invitation epoch",
    "invitee prefix",
    "event epoch",
    "event body",
    "sample value",
    "event source",
    "client fingerprint",
    "trace",
    "statistics",
    "command key",
    "command kind",
    "command state",
    "command body hash",
    "command request id",
    "boot count",
    "command outcome",
    "audit row",
    "install state",
    "install target",
    "install version",
    "species",
    "birth date",
    "daily kcal",
];

/// Give `row` a value its reader refuses as `what`; false for a row that
/// cannot carry it.
fn unreadable(what: &str, row: &mut Row) -> bool {
    const WIDE: i64 = 1 << 32;
    let text = match (what, row) {
        ("role", Row::Users(user)) => &mut user.role,
        ("units", Row::Homes(home)) => &mut home.units,
        ("time format", Row::Homes(home)) => &mut home.time_format,
        ("feeder role", Row::Feeders(feeder)) => &mut feeder.role,
        ("phone role", Row::PhoneKeys(phone)) => &mut phone.role,
        ("revoke reason", Row::PhoneKeys(phone)) if phone.revoke.is_some() => {
            phone.revoke.get_or_insert_default()
        }
        ("command kind", Row::Commands(command)) => &mut command.kind,
        ("command state", Row::Commands(command)) => &mut command.state,
        ("command outcome", Row::Commands(command)) => command.outcome.get_or_insert_default(),
        ("install state", Row::Installs(install)) => &mut install.state,
        ("install target", Row::Installs(install)) => &mut install.target,
        ("species", Row::Pets(pet)) => &mut pet.species,
        ("birth date", Row::Pets(pet)) => pet.birth_date.get_or_insert_default(),
        ("statistics", Row::FeederStats(stat)) => &mut stat.stats,
        ("event source", Row::Feeds(feed)) => {
            feed.client = None;
            return true;
        }
        (what, row) => return unreadable_number(what, row, WIDE),
    };
    *text = "nonsense".into();
    true
}

/// [`unreadable`]'s numbers, byte lengths and column pairs.
fn unreadable_number(what: &str, row: &mut Row, wide: i64) -> bool {
    match (what, row) {
        ("epoch", Row::Feeders(feeder)) => feeder.epoch = Some(wide),
        ("cursor without its epoch", Row::Feeders(feeder)) => feeder.epoch = None,
        ("cursor", Row::Feeders(feeder)) => feeder.synced = Some(-1),
        ("identity", Row::Feeders(feeder)) => feeder.identity.truncate(31),
        ("phone key", Row::PhoneKeys(phone)) => phone.key.truncate(31),
        ("revocation without its time", Row::PhoneKeys(phone)) if phone.revoke.is_some() => {
            phone.revoke_ms = None;
        }
        ("kept client key", Row::KeptClients(kept)) => kept.key.truncate(31),
        ("invitation epoch", Row::Invitations(invitation)) => invitation.epoch = wide,
        ("invitee prefix", Row::Invitations(invitation)) => invitation.invitee.push(0),
        ("event epoch", Row::Events(event)) => event.epoch = wide,
        ("event body", Row::Events(event)) => event.body = UNDECODABLE.to_vec(),
        ("sample value", Row::Samples(sample)) => sample.value = UNDECODABLE.to_vec(),
        ("client fingerprint", Row::Feeds(feed)) => feed.client = Some(vec![1; 3]),
        ("trace", Row::FeedTraces(trace)) => trace.refusal = Some("\"busy\"".into()),
        ("command key", Row::Commands(command)) => command.key.truncate(15),
        ("command body hash", Row::Commands(command)) => command.body_hash.truncate(31),
        ("command request id", Row::Commands(command)) => command.request_id.truncate(15),
        ("boot count", Row::Commands(command)) => command.boot_count = Some(wide),
        ("audit row", Row::Audit(entry)) if entry.action.is_some() => entry.command_seq = Some(1),
        ("install version", Row::Installs(install)) => install.version = -1,
        ("daily kcal", Row::Pets(pet)) => pet.daily_kcal = Some(-1),
        _ => return false,
    }
    true
}

/// The tables an archive leaves out, and why.
const NOT_EXPORTED: &[(&str, &str)] = &[
    ("sessions", "a credential's hash"),
    ("tokens", "a credential's hash"),
    ("job_runs", "this server's schedule"),
    (
        "retired_keys",
        "keys read unfound, which only requests in flight to this server need",
    ),
    (
        "feeder_alerts",
        "a copy of the feeder's RAM, read again from it",
    ),
    ("_sqlx_migrations", "the schema's own history"),
    (
        "sqlite_sequence",
        "SQLite's, which the imported ids move on",
    ),
];

/// The columns of exported tables an archive leaves out, and why.
const COLUMNS_NOT_EXPORTED: &[(&str, &str, &str)] = &[
    ("users", "password_hash", "a credential's hash"),
    (
        "feeders",
        "key_nonce",
        "the server's sealed key: adopted again",
    ),
    (
        "feeders",
        "key_sealed",
        "the server's sealed key: adopted again",
    ),
    ("feeders", "kind", "the model's, which the row names"),
    (
        "feeders",
        "archived_ms",
        "every imported feeder is archived",
    ),
    ("homes", "single", "the one home's fixed key"),
    ("projection_version", "id", "the one row's fixed key"),
    ("sample_marks", "id", "the one row's fixed key"),
    (
        "commands",
        "call_before",
        "a meal-call write's retry after a move finds none and settles superseded",
    ),
    (
        "commands",
        "call_started",
        "a meal-call write's retry after a move finds none and settles superseded",
    ),
];

/// Every table and column of the database is exported, or listed above
/// with its reason: a migration that adds one fails here until it is
/// either in the archive or said to be left out.
#[tokio::test]
async fn every_column_is_exported_or_left_out_on_purpose() {
    let dir = tempfile::tempdir().unwrap();
    let (_store, pool) = open(&dir, "schema.db").await;
    let tables: Vec<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();

    let mut missing = Vec::new();
    for name in &tables {
        let Some(&table) = Table::ALL.iter().find(|table| table.name() == name) else {
            if !NOT_EXPORTED.iter().any(|(left, _)| left == name) {
                missing.push(name.clone());
            }
            continue;
        };
        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info(?) ORDER BY cid")
                .bind(name)
                .fetch_all(&pool)
                .await
                .unwrap();
        for column in columns {
            let exported = table.columns().contains(&column.as_str());
            let left_out = COLUMNS_NOT_EXPORTED
                .iter()
                .any(|(left, col, _)| left == name && *col == column);
            if !exported && !left_out {
                missing.push(format!("{name}.{column}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "neither exported nor left out: {missing:?}"
    );

    for (name, _) in NOT_EXPORTED {
        assert!(tables.contains(&(*name).to_owned()), "{name} is a table");
    }
}
