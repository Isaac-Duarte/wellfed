//! What the background jobs keep and clear.

use jiff::Timestamp;
use tempfile::TempDir;
use uuid::Uuid;
use wellfed_api::{Home, HomeId, Job, JobOutcome, JobRun, Role, TimeFormat, Units, UserId};
use wellfed_store::{NewUser, Store};

async fn fresh() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("server.db"), at(0))
        .await
        .unwrap();
    (dir, store)
}

fn at(ms: i64) -> Timestamp {
    Timestamp::from_millisecond(ms).unwrap()
}

fn user(id: u128, name: &str, role: Role, expires: Option<Timestamp>) -> NewUser<'_> {
    NewUser {
        id: UserId(Uuid::from_u128(id)),
        name,
        password_hash: "$argon2id$hash",
        role,
        expires,
        created: at(0),
    }
}

fn run(job: Job, started: i64, outcome: JobOutcome) -> JobRun {
    JobRun {
        job,
        started: at(started),
        finished: at(started + 5),
        outcome,
    }
}

#[tokio::test]
async fn a_jobs_last_run_replaces_the_one_before() {
    let (_dir, store) = fresh().await;
    assert!(store.job_runs().await.unwrap().is_empty());

    store
        .record_job_run(&run(Job::Expiry, 100, JobOutcome::Failed))
        .await
        .unwrap();
    store
        .record_job_run(&run(Job::Retention, 200, JobOutcome::Succeeded))
        .await
        .unwrap();
    store
        .record_job_run(&run(Job::Expiry, 300, JobOutcome::Succeeded))
        .await
        .unwrap();

    assert_eq!(
        store.job_runs().await.unwrap(),
        [
            run(Job::Retention, 200, JobOutcome::Succeeded),
            run(Job::Expiry, 300, JobOutcome::Succeeded),
        ],
        "one row a job, in the jobs' order"
    );
}

/// A newer server's job, or a newer outcome, in the table after a
/// downgrade: the rows this build knows still read.
#[tokio::test]
async fn a_job_row_of_a_newer_server_is_left_out() {
    let (dir, store) = fresh().await;
    store
        .record_job_run(&run(Job::Expiry, 100, JobOutcome::Succeeded))
        .await
        .unwrap();
    let pool = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}",
        dir.path().join("server.db").display()
    ))
    .await
    .unwrap();
    sqlx::query("INSERT INTO job_runs VALUES ('compaction', 1, 2, 'succeeded'), ('backup', 1, 2, 'skipped')")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    assert_eq!(
        store.job_runs().await.unwrap(),
        [run(Job::Expiry, 100, JobOutcome::Succeeded)]
    );
}

#[tokio::test]
async fn idle_sessions_and_an_expired_accounts_sessions_end() {
    let (_dir, store) = fresh().await;
    let home = Home {
        id: HomeId(Uuid::from_u128(0x40)),
        name: "Home".into(),
        units: Units::Grams,
        time_format: TimeFormat::H24,
        zone: "UTC".into(),
    };
    let ada = user(1, "ada", Role::Owner, None);
    let ada_id = ada.id;
    store.create_first_owner(ada, &home).await.unwrap();
    let bob = user(2, "bob", Role::Member, Some(at(5_000)));
    let bob_id = bob.id;
    store.create_user(bob).await.unwrap();
    let cy = user(3, "cy", Role::Member, Some(at(50_000)));
    let cy_id = cy.id;
    store.create_user(cy).await.unwrap();

    let sessions = [
        ([1; 32], ada_id, 8_000), // in use
        ([2; 32], ada_id, 500),   // idle
        ([3; 32], bob_id, 8_000), // in use, but bob has expired
        ([4; 32], cy_id, 8_000),  // in use, cy not yet expired
    ];
    // Each opened before bob expired, then used.
    for (token, user, used) in sessions {
        let opened = store.create_session(&token, user, "$argon2id$hash", at(0));
        assert!(opened.await.unwrap());
        store.touch_session(&token, at(used)).await.unwrap();
    }

    let ended = store.delete_ended_sessions(at(1_000), at(9_000)).await;

    assert_eq!(ended.unwrap(), 2);
    for (token, kept) in [
        ([1; 32], true),
        ([2; 32], false),
        ([3; 32], false),
        ([4; 32], true),
    ] {
        let session = store.session(&token).await.unwrap();
        assert_eq!(session.is_some(), kept, "session {}", token[0]);
    }
    let bob = store.user(bob_id).await.unwrap().unwrap();
    assert_eq!(bob.expires, Some(at(5_000)), "the account stays, disabled");
}
