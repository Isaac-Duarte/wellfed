//! The portion calibration and the meal call over HTTP, through the typed
//! client: anyone reads them, an owner or a member changes them, a viewer
//! is refused every write, a stale calibration is answered `412` with the
//! current one, and the meal call goes up as the feeder's own stream.

use uuid::Uuid;
use wellfed_api::{
    AdoptFeeder, Bowl, Calibration, CallClip, ClientRole, Connection, Credentials, ErrorCode,
    FeederId, Generation, NewUser, Role,
};
use wellfed_server_client::{Client, Error, StatusCode};
use wellfed_server_testkit::{TestServer, ada_setup, feeder, local, refused, refused_for, until};

fn key(n: u128) -> Uuid {
    Uuid::from_u128(0x0192c5d0_5555_7000_8000_000000000000 + n)
}

/// A logged-in client for a new account of `role`, made by `owner`.
async fn account(server: &TestServer, owner: &Client, name: &str, role: Role) -> Client {
    let password = "temporary pass".to_owned();
    owner
        .create_user(&NewUser {
            name: name.into(),
            password: password.clone(),
            role,
            expires: None,
        })
        .await
        .unwrap();
    let mut client = server.client();
    client
        .login(&Credentials {
            name: name.into(),
            password,
        })
        .await
        .unwrap();
    client
}

/// Adopt `sim` with an owner's invite, the role the feeder asks of a
/// calibration or a clip, and wait until the server is connected.
async fn adopt(http: &Client, sim: &feeder::Running) -> FeederId {
    let id = http
        .adopt(&AdoptFeeder {
            invite: feeder::invite(sim, ClientRole::Owner).await,
            name: "kitchen".into(),
            address: Some(sim.address.to_string()),
            phone_key: None,
        })
        .await
        .unwrap()
        .id;
    until("the server connects to the feeder", || async {
        http.feeders().await.unwrap()[0].connection == Connection::Online
    })
    .await;
    id
}

#[test]
fn a_member_calibrates_and_a_viewer_only_reads() {
    local(async {
        let sim = feeder::start().await;
        let server = TestServer::start().await;
        let mut owner = server.client();
        owner.setup(&ada_setup()).await.unwrap();
        let kitchen = adopt(&owner, &sim).await;
        let member = account(&server, &owner, "bob", Role::Member).await;
        let viewer = account(&server, &owner, "vic", Role::Viewer).await;

        let read = viewer.calibration(kitchen).await.unwrap();
        assert_eq!(read.generation, Generation(0));
        let weighed = Calibration {
            portion_mg: Some(9_800),
            bowl: Bowl::Single,
        };
        refused(
            viewer
                .put_calibration(kitchen, read.generation, &weighed, key(1))
                .await,
            StatusCode::FORBIDDEN,
            ErrorCode::Forbidden,
        );
        let changed = member
            .put_calibration(kitchen, read.generation, &weighed, key(2))
            .await
            .unwrap();
        assert_eq!(changed.calibration, weighed);
        assert_eq!(changed.generation, Generation(1));

        let stale = owner
            .put_calibration(kitchen, read.generation, &weighed, key(3))
            .await;
        let Err(Error::StaleCalibration(current)) = stale else {
            panic!("{stale:?}");
        };
        assert_eq!(*current, changed);
        let light = Calibration {
            portion_mg: Some(10),
            ..weighed
        };
        refused(
            owner
                .put_calibration(kitchen, changed.generation, &light, key(4))
                .await,
            StatusCode::BAD_REQUEST,
            ErrorCode::Invalid,
        );
        server.stop().await;
        sim.stop().await;
    });
}

#[test]
fn a_member_uploads_plays_and_erases_the_call_and_a_viewer_only_reads() {
    local(async {
        let sim = feeder::start().await;
        let server = TestServer::start().await;
        let mut owner = server.client();
        owner.setup(&ada_setup()).await.unwrap();
        let kitchen = adopt(&owner, &sim).await;
        let member = account(&server, &owner, "bob", Role::Member).await;
        let viewer = account(&server, &owner, "vic", Role::Viewer).await;

        let clip: Vec<u8> = (0..1_500u32)
            .map(|n| u8::try_from(n % 241).unwrap())
            .collect();
        for (name, attempt) in [
            (
                "upload",
                viewer
                    .upload_call(kitchen, clip.clone(), key(10))
                    .await
                    .map(drop),
            ),
            ("play", viewer.play_call(kitchen, key(11)).await),
            ("delete", viewer.delete_call(kitchen, key(12)).await),
        ] {
            refused_for(name, attempt, StatusCode::FORBIDDEN, ErrorCode::Forbidden);
        }

        let stored = member
            .upload_call(kitchen, clip.clone(), key(13))
            .await
            .unwrap();
        assert!(matches!(stored.clip, CallClip::Stored { bytes: 1_500, .. }));
        assert_eq!(viewer.call(kitchen).await.unwrap(), stored);
        let again = member.upload_call(kitchen, clip, key(13)).await.unwrap();
        assert_eq!(again, stored, "a retry is answered from the journal");
        refused(
            member.upload_call(kitchen, Vec::new(), key(14)).await,
            StatusCode::BAD_REQUEST,
            ErrorCode::Invalid,
        );

        // The simulator has no speaker, so the feeder refuses it muted.
        refused(
            member.play_call(kitchen, key(15)).await,
            StatusCode::CONFLICT,
            ErrorCode::FeederRefused,
        );
        member.delete_call(kitchen, key(16)).await.unwrap();
        assert_eq!(viewer.call(kitchen).await.unwrap().clip, CallClip::None);
        server.stop().await;
        sim.stop().await;
    });
}
