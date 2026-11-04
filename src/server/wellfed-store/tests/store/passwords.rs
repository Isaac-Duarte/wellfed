//! Password rotation keeps sessions tied to the verified password.

use super::*;

#[tokio::test]
async fn a_password_rotation_rejects_stale_sessions_and_changes() {
    let (_dir, store) = fresh().await;
    let ada = user("ada");
    let id = ada.id;
    store.create_first_owner(ada, &household()).await.unwrap();
    for token in [[1; 32], [2; 32]] {
        assert!(
            store
                .create_session(&token, id, at(0), "$argon2id$hash")
                .await
                .unwrap()
        );
    }
    assert!(
        store
            .set_password(id, "$argon2id$new", Some("$argon2id$hash"), Some(&[1; 32]))
            .await
            .unwrap()
    );
    assert!(store.session(&[1; 32]).await.unwrap().is_some());
    assert!(store.session(&[2; 32]).await.unwrap().is_none());
    assert!(
        !store
            .create_session(&[3; 32], id, at(1), "$argon2id$hash")
            .await
            .unwrap()
    );
    assert!(store.session(&[3; 32]).await.unwrap().is_none());
    assert!(
        !store
            .set_password(id, "$argon2id$stale", Some("$argon2id$hash"), None)
            .await
            .unwrap()
    );
    assert_eq!(
        store.password_hash(id).await.unwrap().as_deref(),
        Some("$argon2id$new")
    );
    assert!(
        store.session(&[1; 32]).await.unwrap().is_some(),
        "a refused change does not revoke sessions"
    );
    assert!(
        store
            .create_session(&[4; 32], id, at(2), "$argon2id$new")
            .await
            .unwrap()
    );
}
