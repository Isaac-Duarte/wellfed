//! The derived views over HTTP against the simulated feeder, which has no
//! calibration, so they count in portions: a feed through
//! the server is counted against the last refill in the feeder's food view
//! and in its pet's intake, every role reads both, and a bad span or an
//! unknown pet is refused.

use uuid::Uuid;
use wellfed_api::{
    AdoptFeeder, ClientRole, Connection, Credentials, ErrorCode, FeederPortions, IntakeQuery,
    NewFood, NewPet, NewRefill, NewUser, PetId, PortionWeight, Role, Species,
};
use wellfed_server_client::{Client, StatusCode};
use wellfed_server_testkit::{TestServer, ada_setup, feeder, local, refused, until};

const PASSWORD: &str = "temporary pass";

async fn viewer(server: &TestServer, owner: &Client) -> Client {
    let new = NewUser {
        name: "vic".into(),
        password: PASSWORD.into(),
        role: Role::Viewer,
        expires: None,
    };
    owner.create_user(&new).await.unwrap();
    let mut client = server.client();
    let login = Credentials {
        name: "vic".into(),
        password: PASSWORD.into(),
    };
    client.login(&login).await.unwrap();
    client
}

#[test]
fn a_feed_through_the_server_counts_against_the_refill_and_the_pet() {
    local(async {
        let sim = feeder::start().await;
        let server = TestServer::start().await;
        let mut owner = server.client();
        owner.setup(&ada_setup()).await.unwrap();
        let kitchen = owner
            .adopt(&AdoptFeeder {
                invite: feeder::invite(&sim, ClientRole::Server).await,
                name: "kitchen".into(),
                address: Some(sim.address.to_string()),
                phone_key: None,
            })
            .await
            .unwrap()
            .id;
        until("the server connects to the feeder", || async {
            owner.feeders().await.unwrap()[0].connection == Connection::Online
        })
        .await;

        // The simulator has no calibration, so the views count portions.
        let dry = NewFood {
            brand: "Acme".into(),
            name: "Dry".into(),
            kcal_per_kg: 3600,
            bag_grams: 2000,
        };
        let dry = owner.create_food(&dry).await.unwrap();
        owner.set_feeder_food(kitchen, dry.id).await.unwrap();
        owner
            .add_refill(kitchen, &NewRefill::FullBag)
            .await
            .unwrap();
        let tom = NewPet {
            name: "Tom".into(),
            species: Species::Cat,
            birth_date: None,
            daily_kcal: Some(144),
        };
        let tom = owner.create_pet(&tom).await.unwrap();
        owner.assign_feeder(tom.id, kitchen).await.unwrap();
        owner
            .feed(
                kitchen,
                2,
                Uuid::from_u128(0x0192c5d0_4444_7000_8000_000000000001),
            )
            .await
            .unwrap();
        until("the feed reaches the food view", || async {
            let view = owner.feeder_food(kitchen).await.unwrap();
            view.since_refill.is_some_and(|since| since.portions == 2)
        })
        .await;

        let reader = viewer(&server, &owner).await;
        let view = reader.feeder_food(kitchen).await.unwrap();
        assert_eq!(view.weight, PortionWeight::Uncalibrated);
        let since = view.since_refill.unwrap();
        assert_eq!((since.uncertain_portions, since.left), (0, None));
        assert_eq!(view.anomalies, []);
        let intake = reader
            .pet_intake(tom.id, &IntakeQuery::default())
            .await
            .unwrap();
        let today = intake.days.last().unwrap();
        assert_eq!(intake.days.len(), 7);
        assert_eq!((today.weighed, today.target_percent), (None, None));
        let portions = FeederPortions {
            feeder: kitchen,
            portions: 2,
            uncertain_portions: 0,
        };
        assert_eq!(today.unweighed, [portions]);
        assert!(!today.estimate);

        let (bad, invalid) = (StatusCode::BAD_REQUEST, ErrorCode::Invalid);
        let none = IntakeQuery { days: Some(0) };
        refused(reader.pet_intake(tom.id, &none).await, bad, invalid);
        let nobody = PetId(Uuid::from_u128(1));
        let unknown = reader.pet_intake(nobody, &IntakeQuery::default()).await;
        refused(unknown, StatusCode::NOT_FOUND, ErrorCode::NotFound);
        server.stop().await;
    });
}
