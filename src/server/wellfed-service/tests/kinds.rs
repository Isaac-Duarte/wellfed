//! A device is stored with its kind and model, belongs to the home, and an
//! op of another kind is refused by the stored kind before anything is
//! journaled or sent, whether or not the device is online. The device here
//! is a stored row no task mirrors: the refusal never needs one.

use uuid::Uuid;
use wellfed_api::{ClientRole, DeviceKind, FeederId, Model};
use wellfed_server_testkit::fixed::FixedClock;
use wellfed_server_testkit::{TestService, ada, ada_setup};
use wellfed_service::{Error, FeederError};
use wellfed_store::{CommandFilter, NewFeeder, SealedKey};

const FOUNTAIN: FeederId = FeederId(Uuid::from_u128(0x0192c5d0_2222_7000_8000_000000000001));

fn other_kind(error: &Error) -> bool {
    matches!(
        error,
        Error::Feeder(FeederError::OtherKind {
            device: DeviceKind::Fountain,
            needs: DeviceKind::Feeder,
        })
    )
}

#[tokio::test]
async fn a_feeder_op_to_a_device_of_another_kind_is_refused_unjournaled() {
    let test = TestService::start().await;
    let service = test.service();
    service.setup(ada_setup()).await.unwrap();
    let store = test.store().open().await;
    let home = service.home().await.unwrap();
    store
        .insert_feeder(NewFeeder {
            id: FOUNTAIN,
            home: home.id,
            model: Model::Plwf106,
            identity: [6; 32],
            name: "fountain",
            role: ClientRole::Server,
            address: None,
            key: SealedKey {
                nonce: [7; 12],
                sealed: vec![1, 2, 3],
            },
            created: FixedClock.now(),
        })
        .await
        .unwrap();

    // The listing names the home, the kind and the model.
    let device = service
        .feeders()
        .await
        .unwrap()
        .into_iter()
        .find(|device| device.id == FOUNTAIN)
        .expect("the device is listed");
    assert_eq!(device.home, home.id);
    assert_eq!(device.kind, DeviceKind::Fountain);
    assert_eq!(device.model, Model::Plwf106);

    // A read and a write of the feeder's module are refused by the kind,
    // not by the missing session.
    let read = service.settings(FOUNTAIN).await.unwrap_err();
    assert!(other_kind(&read), "{read:?}");
    let caller = ada().into();
    let write = service
        .set_lock(&caller, FOUNTAIN, true, None)
        .await
        .unwrap_err();
    assert!(other_kind(&write), "{write:?}");

    let journal = store
        .commands(CommandFilter {
            user: None,
            feeder: Some(FOUNTAIN),
            state: None,
            after: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert!(journal.is_empty(), "nothing was journaled: {journal:?}");
}
