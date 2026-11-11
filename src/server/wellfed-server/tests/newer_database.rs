//! A database a newer server migrated: the server opens it read-only and
//! refuses to run on it, so going forward to that server again loses
//! nothing.

use jiff::Timestamp;
use wellfed_server::config::DATABASE;
use wellfed_server::key::sealing_key;
use wellfed_server_testkit::fixed::SeededRandom;
use wellfed_store::{Schema, Store};

#[tokio::test]
async fn the_server_refuses_a_database_a_newer_server_migrated_and_leaves_it() {
    let data = tempfile::tempdir().unwrap();
    let path = data.path().join(DATABASE);
    sealing_key(data.path(), &SeededRandom::new(1)).unwrap();
    Store::open(&path, Timestamp::UNIX_EPOCH)
        .await
        .unwrap()
        .close()
        .await;

    // What a newer build's migration leaves in sqlx's record.
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

    let store = Store::open(&path, Timestamp::UNIX_EPOCH).await.unwrap();
    assert!(matches!(
        store.schema(),
        Schema::Newer { applied: 9999, .. }
    ));
    store.close().await;

    for command in [&[][..], &["reset-owner"][..]] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_wellfed-server"))
            .arg("--data")
            .arg(data.path())
            .args(command)
            .output()
            .unwrap();

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{command:?}");
        assert!(stderr.contains("migrated by a newer server"), "{stderr}");
    }
    assert_eq!(std::fs::read(&path).unwrap(), before, "left as it was");
}
