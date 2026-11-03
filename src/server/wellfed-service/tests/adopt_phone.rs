//! A phone that adopts a feeder names the key it already holds there: the
//! server records it as the caller's phone key once the feeder lists it,
//! and revokes it as it revokes a key it invited. A key the feeder does not
//! list is refused after the adoption, which stands.

use wellfed_api::{
    AdoptFeeder, ClientOwner, ClientRole, FeederId, Hex, NewToken, NewUser, Role, User,
};
use wellfed_server_testkit::feeder::{self, Running};
use wellfed_server_testkit::{TestService, ada_setup, local, until};
use wellfed_service::{Caller, ClientError, Error, Service};

/// A phone's secret key, by number.
fn secret(n: u8) -> [u8; 32] {
    [n; 32]
}

/// A phone signed in as `user`: the user and a token of theirs.
async fn on_phone(service: &Service, user: User) -> Caller {
    let login = Caller::from(user);
    let new = NewToken {
        name: "phone".into(),
        expires: None,
    };
    let token = service.create_token(&login, new).await.unwrap();
    Caller {
        user: login.user,
        token: Some(token.token.id),
    }
}

/// A second owner, so the first can delete them.
async fn second_owner(service: &Service, first: &Caller) -> User {
    let new = NewUser {
        name: "cy".into(),
        password: "temporary pass".into(),
        role: Role::Owner,
        expires: None,
    };
    service.create_user(first, new).await.unwrap()
}

/// The phone numbered `n` joins `sim` as its owner already paired it, before
/// any server: the key the app holds when it adopts.
async fn paired(sim: &Running, n: u8) -> [u8; 32] {
    let invite = feeder::invite(sim, ClientRole::Owner).await;
    assert_eq!(
        feeder::join_phone(sim, &invite, secret(n)).await,
        ClientRole::Owner
    );
    feeder::phone_key(secret(n))
}

/// Adopt `sim` as `caller` by an owner's invite (the one the app makes with
/// its key), naming `phone_key`.
async fn adopt(
    service: &Service,
    sim: &Running,
    caller: &Caller,
    phone_key: [u8; 32],
) -> Result<wellfed_api::Feeder, Error> {
    service
        .adopt(
            caller,
            AdoptFeeder {
                invite: feeder::invite(sim, ClientRole::Owner).await,
                name: "kitchen".into(),
                address: Some(sim.address.to_string()),
                phone_key: Some(Hex(phone_key)),
            },
        )
        .await
}

async fn adopted(service: &Service) -> Vec<FeederId> {
    let feeders = service.feeders().await.unwrap();
    feeders.iter().map(|feeder| feeder.id).collect()
}

#[test]
fn the_adopting_phones_key_is_recorded_as_its_accounts() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        let service = server.service();
        let ada = service.setup(ada_setup()).await.unwrap().0;
        let ada = on_phone(service, ada).await;
        let key = paired(&sim, 1).await;

        let id = adopt(service, &sim, &ada, key).await.unwrap().id;

        let phones = service.phones(&ada.user).await.unwrap();
        let [phone] = phones.as_slice() else {
            panic!("one phone key, got {phones:?}");
        };
        assert_eq!((phone.feeder, phone.key.0), (id, key));
        assert_eq!((phone.user, phone.token), (ada.user.id, ada.token));
        assert_eq!(phone.role, ClientRole::Owner);
        assert!(phone.revoking.is_none());

        let clients = service.clients(id).await.unwrap();
        let client = clients.iter().find(|client| client.key.0 == key).unwrap();
        let adas = ClientOwner::Phone {
            user: ada.user.id,
            token: ada.token,
            revoking: None,
        };
        assert_eq!(client.owner, adas);
    });
}

#[test]
fn a_key_the_feeder_does_not_hold_is_refused_and_the_adoption_stands() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        let service = server.service();
        let ada = service.setup(ada_setup()).await.unwrap().0;
        let ada = on_phone(service, ada).await;

        let adopted_with = adopt(service, &sim, &ada, feeder::phone_key(secret(2))).await;

        assert!(matches!(
            adopted_with,
            Err(Error::Client(ClientError::PhoneKeyNotListed))
        ));
        assert_eq!(adopted(service).await.len(), 1, "the adoption stands");
        assert!(service.phones(&ada.user).await.unwrap().is_empty());
    });
}

#[test]
fn a_phone_key_from_a_login_is_refused_before_anything_is_enrolled() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        let service = server.service();
        let ada = service.setup(ada_setup()).await.unwrap().0;
        let key = paired(&sim, 3).await;
        let before = feeder::client_keys(&sim).await;

        let adopted_with = adopt(service, &sim, &Caller::from(ada), key).await;

        assert!(matches!(
            adopted_with,
            Err(Error::Client(ClientError::TokenRequired))
        ));
        assert!(adopted(service).await.is_empty());
        assert_eq!(feeder::client_keys(&sim).await, before, "nothing enrolled");
    });
}

#[test]
fn deleting_the_account_revokes_the_adopting_phones_key_on_the_feeder() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        let service = server.service();
        let ada = Caller::from(service.setup(ada_setup()).await.unwrap().0);
        let cy = second_owner(service, &ada).await;
        let cy = on_phone(service, cy).await;
        let key = paired(&sim, 4).await;
        adopt(service, &sim, &cy, key).await.unwrap();

        service.delete_user(&ada, cy.user.id).await.unwrap();

        until("the adopting phone's key is revoked", || async {
            let held = feeder::client_keys(&sim).await.contains(&key);
            let recorded = service.phones(&ada.user).await.unwrap();
            !held && !recorded.iter().any(|phone| phone.key.0 == key)
        })
        .await;
    });
}
