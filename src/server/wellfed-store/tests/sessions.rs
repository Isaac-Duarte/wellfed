//! Users and their browser sessions in the store: the last owner kept,
//! a password change and the sessions it ends committed together, and a
//! session opened only on the password its login was checked against.

use std::sync::atomic::{AtomicU64, Ordering};

use jiff::Timestamp;
use tempfile::TempDir;
use uuid::Uuid;
use wellfed_api::{Home, HomeId, Role, TimeFormat, Units, UserId};
use wellfed_store::{Change, Error, NewUser, Sessions, Store, UserUpdate};

async fn fresh() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("server.db"), at(0))
        .await
        .unwrap();
    (dir, store)
}

/// A new id for each call: the store doesn't care how ids are made, and a
/// test reads no clock.
fn fresh_id() -> Uuid {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    Uuid::from_u128(u128::from(NEXT.fetch_add(1, Ordering::Relaxed)))
}

/// Every test user's password hash.
const HASH: &str = "$argon2id$hash";

/// A change of role alone.
fn role(role: Role) -> UserUpdate<'static> {
    UserUpdate {
        role,
        expires: None,
        password_hash: None,
        sessions: Sessions::Kept,
    }
}

fn at(ms: i64) -> Timestamp {
    Timestamp::from_millisecond(ms).unwrap()
}

fn user(name: &str) -> NewUser<'_> {
    NewUser {
        id: UserId(fresh_id()),
        name,
        password_hash: HASH,
        role: Role::Owner,
        expires: None,
        created: at(1_000),
    }
}

fn home() -> Home {
    Home {
        id: HomeId(Uuid::from_u128(0x40)),
        name: "Home".into(),
        units: Units::Grams,
        time_format: TimeFormat::H24,
        zone: "Europe/Berlin".into(),
    }
}

#[tokio::test]
async fn a_session_is_used_idles_out_or_logs_out() {
    let (_dir, store) = fresh().await;
    let new = user("ada");
    let id = new.id;
    store.create_first_owner(new, &home()).await.unwrap();
    let (live, idle) = ([1; 32], [2; 32]);
    assert!(
        store
            .create_session(&live, id, HASH, at(5_000))
            .await
            .unwrap()
    );
    assert!(store.create_session(&idle, id, HASH, at(0)).await.unwrap());

    let session = store.session(&live).await.unwrap().unwrap();
    assert_eq!((session.user.id, session.last_used), (id, at(5_000)));
    assert!(store.session(&[3; 32]).await.unwrap().is_none());
    store.touch_session(&live, at(9_000)).await.unwrap();
    store.touch_session(&live, at(8_000)).await.unwrap();
    let session = store.session(&live).await.unwrap().unwrap();
    assert_eq!(session.last_used, at(9_000), "a use never moves it back");

    assert_eq!(
        store
            .delete_ended_sessions(at(1_000), at(9_000))
            .await
            .unwrap(),
        1
    );
    assert!(store.session(&idle).await.unwrap().is_none());
    store.delete_session(&live).await.unwrap();
    assert!(store.session(&live).await.unwrap().is_none());
}

#[tokio::test]
async fn users_are_listed_changed_and_deleted_but_never_the_last_owner() {
    let (_dir, store) = fresh().await;
    let ada = user("ada");
    let ada_id = ada.id;
    store.create_first_owner(ada, &home()).await.unwrap();
    let bob = NewUser {
        role: Role::Member,
        expires: Some(at(50_000)),
        ..user("bob")
    };
    let bob_id = bob.id;
    store.create_user(bob).await.unwrap();
    assert!(matches!(
        store.create_user(user("bob")).await,
        Err(Error::Duplicate("user"))
    ));

    let users = store.users().await.unwrap();
    let names: Vec<_> = users.iter().map(|u| (u.name.as_str(), u.role)).collect();
    assert_eq!(names, [("ada", Role::Owner), ("bob", Role::Member)]);
    assert_eq!(users[1].expires, Some(at(50_000)));

    // The only owner can be neither demoted nor deleted.
    assert_eq!(
        store.update_user(ada_id, role(Role::Member)).await.unwrap(),
        Change::LastOwner
    );
    assert_eq!(store.delete_user(ada_id).await.unwrap(), Change::LastOwner);
    assert_eq!(
        store.update_user(ada_id, role(Role::Owner)).await.unwrap(),
        Change::Done,
        "an owner staying one is no change of owners"
    );

    // With a second owner, either may go.
    assert_eq!(
        store.update_user(bob_id, role(Role::Owner)).await.unwrap(),
        Change::Done
    );
    assert_eq!(
        store.update_user(ada_id, role(Role::Viewer)).await.unwrap(),
        Change::Done
    );
    assert_eq!(
        store.user(ada_id).await.unwrap().unwrap().role,
        Role::Viewer
    );

    assert!(
        store
            .set_password(ada_id, "$argon2id$new", None, None)
            .await
            .unwrap()
    );
    assert_eq!(
        store.login("ada").await.unwrap().unwrap().password_hash,
        "$argon2id$new"
    );
    let opened = store.create_session(&[4; 32], ada_id, "$argon2id$new", at(0));
    assert!(opened.await.unwrap());
    assert_eq!(store.delete_user(ada_id).await.unwrap(), Change::Done);
    assert!(
        store.session(&[4; 32]).await.unwrap().is_none(),
        "their sessions went with them"
    );
    assert_eq!(store.delete_user(ada_id).await.unwrap(), Change::NotFound);
    assert_eq!(
        store.update_user(ada_id, role(Role::Member)).await.unwrap(),
        Change::NotFound
    );
    assert!(!store.set_password(ada_id, "x", None, None).await.unwrap());
}

/// A change checked against a hash that has changed since replaces
/// nothing and ends no session: the reset that landed in between stands.
#[tokio::test]
async fn a_password_change_checked_against_an_old_hash_changes_nothing() {
    let (_dir, store) = fresh().await;
    let ada = user("ada");
    let ada_id = ada.id;
    store.create_first_owner(ada, &home()).await.unwrap();
    assert!(
        store
            .create_session(&[1; 32], ada_id, HASH, at(0))
            .await
            .unwrap()
    );
    let reset = store.set_password(ada_id, "$argon2id$reset", None, None);
    assert!(reset.await.unwrap());

    let stale = store.set_password(ada_id, "$argon2id$mine", Some(HASH), None);
    assert!(!stale.await.unwrap());
    let login = store.login("ada").await.unwrap().unwrap();
    assert_eq!(login.password_hash, "$argon2id$reset");

    let current = store.set_password(ada_id, "$argon2id$mine", Some("$argon2id$reset"), None);
    assert!(current.await.unwrap());
}

#[tokio::test]
async fn a_password_change_ends_that_users_other_sessions() {
    let (_dir, store) = fresh().await;
    let ada = user("ada");
    let ada_id = ada.id;
    store.create_first_owner(ada, &home()).await.unwrap();
    let bob = user("bob");
    let bob_id = bob.id;
    store.create_user(bob).await.unwrap();
    for (token, id) in [([1; 32], ada_id), ([2; 32], ada_id), ([3; 32], bob_id)] {
        assert!(store.create_session(&token, id, HASH, at(0)).await.unwrap());
    }

    let kept = Some(&[2; 32]);
    assert!(
        store
            .set_password(ada_id, "$argon2id$new", None, kept)
            .await
            .unwrap()
    );
    assert!(store.session(&[1; 32]).await.unwrap().is_none());
    assert!(
        store.session(&[2; 32]).await.unwrap().is_some(),
        "the one kept"
    );
    let reset = UserUpdate {
        password_hash: Some("$argon2id$newer"),
        sessions: Sessions::Ended { keep: None },
        ..role(Role::Owner)
    };
    assert_eq!(
        store.update_user(ada_id, reset).await.unwrap(),
        Change::Done
    );
    assert!(store.session(&[2; 32]).await.unwrap().is_none());
    assert_eq!(
        store.password_hash(ada_id).await.unwrap().as_deref(),
        Some("$argon2id$newer")
    );
    assert!(
        store.session(&[3; 32]).await.unwrap().is_some(),
        "bob's stays"
    );
    assert_eq!(
        store.password_hash(bob_id).await.unwrap().as_deref(),
        Some(HASH)
    );
}

/// A password change whose session delete fails changes nothing: the old
/// password and every session stay, so nothing is left half done.
#[tokio::test]
async fn a_password_change_and_its_ended_sessions_commit_together() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let store = Store::open(&path, at(0)).await.unwrap();
    let ada = user("ada");
    let ada_id = ada.id;
    store.create_first_owner(ada, &home()).await.unwrap();
    assert!(
        store
            .create_session(&[1; 32], ada_id, HASH, at(0))
            .await
            .unwrap()
    );
    let raw = sqlx::SqlitePool::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER hold_sessions BEFORE DELETE ON sessions
         BEGIN SELECT RAISE(ABORT, 'held'); END",
    )
    .execute(&raw)
    .await
    .unwrap();

    let reset = UserUpdate {
        password_hash: Some("$argon2id$new"),
        sessions: Sessions::Ended { keep: None },
        ..role(Role::Member)
    };
    // A second owner, so the demotion itself is allowed.
    let bob = user("bob");
    store.create_user(bob).await.unwrap();
    assert!(
        store
            .set_password(ada_id, "$argon2id$new", None, None)
            .await
            .is_err()
    );
    assert!(store.update_user(ada_id, reset).await.is_err());
    let ada = store.user(ada_id).await.unwrap().unwrap();
    assert_eq!(ada.role, Role::Owner, "the role stays with the password");
    assert_eq!(
        store.password_hash(ada_id).await.unwrap().as_deref(),
        Some(HASH)
    );
    assert!(store.session(&[1; 32]).await.unwrap().is_some());

    sqlx::query("DROP TRIGGER hold_sessions")
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    assert_eq!(
        store.update_user(ada_id, reset).await.unwrap(),
        Change::Done
    );
    assert_eq!(
        store.password_hash(ada_id).await.unwrap().as_deref(),
        Some("$argon2id$new")
    );
    assert!(store.session(&[1; 32]).await.unwrap().is_none());
}

/// A login opens its session only on the hash it was checked against, and
/// only for an account still there and not expired.
#[tokio::test]
async fn a_session_opens_only_on_the_verified_hash_of_a_live_account() {
    let (_dir, store) = fresh().await;
    let ada = user("ada");
    store.create_first_owner(ada, &home()).await.unwrap();
    let bob = NewUser {
        role: Role::Member,
        expires: Some(at(5_000)),
        ..user("bob")
    };
    let bob_id = bob.id;
    store.create_user(bob).await.unwrap();

    assert!(
        store
            .create_session(&[1; 32], bob_id, HASH, at(0))
            .await
            .unwrap()
    );
    assert!(
        !store
            .create_session(&[2; 32], bob_id, HASH, at(5_000))
            .await
            .unwrap(),
        "expired"
    );
    assert!(
        store
            .set_password(bob_id, "$argon2id$new", None, None)
            .await
            .unwrap()
    );
    assert!(
        !store
            .create_session(&[3; 32], bob_id, HASH, at(0))
            .await
            .unwrap(),
        "checked against the old password"
    );
    assert_eq!(store.delete_user(bob_id).await.unwrap(), Change::Done);
    let gone = store.create_session(&[4; 32], bob_id, "$argon2id$new", at(0));
    assert!(!gone.await.unwrap(), "deleted");
    for token in [[1; 32], [2; 32], [3; 32], [4; 32]] {
        assert!(store.session(&token).await.unwrap().is_none());
    }
}
