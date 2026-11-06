//! Home identity and first-owner setup.

use super::*;

#[tokio::test]
async fn the_one_home_keeps_its_id_and_a_change_to_another_is_refused() {
    let (_dir, store) = fresh().await;
    assert_eq!(store.create_home(&home(), at(0)).await.unwrap(), HOME);

    // A second write takes on the stored home: its id is the one answered.
    let other = Home {
        id: HomeId(Uuid::from_u128(0x41)),
        zone: "America/Denver".into(),
        ..home()
    };
    assert_eq!(store.create_home(&other, at(1)).await.unwrap(), HOME);
    let stored = store.home().await.unwrap().unwrap();
    assert_eq!((stored.id, stored.zone.as_str()), (HOME, "America/Denver"));

    // A change naming a home the store does not hold writes nothing.
    let refused = store
        .set_home(&Home {
            zone: "UTC".into(),
            ..other
        })
        .await;
    assert!(
        matches!(refused, Err(Error::Missing("home"))),
        "{refused:?}"
    );
    assert_eq!(store.home().await.unwrap(), Some(stored));
}

#[tokio::test]
async fn setup_writes_the_home_with_the_owner() {
    let (_dir, store) = fresh().await;
    assert_eq!(store.home().await.unwrap(), None);

    store
        .create_first_owner(user("ada"), &home())
        .await
        .unwrap();
    assert_eq!(store.home().await.unwrap(), Some(home()));

    let changed = Home {
        units: Units::Ounces,
        time_format: TimeFormat::H12,
        zone: "America/Denver".into(),
        ..home()
    };
    store.set_home(&changed).await.unwrap();
    assert_eq!(store.home().await.unwrap(), Some(changed));
}
