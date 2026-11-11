//! Claiming a key a phone already holds on a feeder: only a phone's token
//! with no expiry on it or its account claims one; a claim refused because
//! the feeder could not be read is settled by claiming again; a claim
//! racing the account's deletion or the token's revocation is revoked; a
//! member claims only a key already recorded as theirs, so it can never
//! take another person's; and a key an owner cannot claim (kept, a
//! server's, the server's own, another token's) is refused.

use std::time::Duration;

use jiff::Timestamp;
use wellfed_api::{
    ClientOwner, ClientRole, Connection as Link, FeederId, Hex, NewPhoneInvite, NewToken, NewUser,
    PhoneClaim, PhoneKey, Role, User,
};
use wellfed_server_testkit::feeder::{self, Flash, Running};
use wellfed_server_testkit::fixed::NOW_MS;
use wellfed_server_testkit::{TestService, ada_setup, config, local, until};
use wellfed_service::{Caller, ClientError, Error, Result, Service};

fn secret(n: u8) -> [u8; 32] {
    [n; 32]
}

fn tomorrow() -> Timestamp {
    Timestamp::from_millisecond(NOW_MS + 86_400_000).unwrap()
}

/// A phone signed in as `user`, with a token expiring at `expires`.
async fn on_phone(service: &Service, user: User, expires: Option<Timestamp>) -> Caller {
    let login = Caller::from(user);
    let new = NewToken {
        name: "phone".into(),
        expires,
    };
    let token = service.create_token(&login, new).await.unwrap();
    Caller {
        user: login.user,
        token: Some(token.token.id),
    }
}

async fn account(service: &Service, owner: &Caller, name: &str, role: Role) -> User {
    let new = NewUser {
        name: name.into(),
        password: "temporary pass".into(),
        role,
        expires: None,
    };
    service.create_user(owner, new).await.unwrap()
}

/// Phone `n` joins `sim` with an invite for `role` its owner made, before
/// the server knows it: the key it then claims.
async fn paired(sim: &Running, n: u8, role: ClientRole) -> [u8; 32] {
    let invite = feeder::invite(sim, role).await;
    assert_eq!(feeder::join_phone(sim, &invite, secret(n)).await, role);
    feeder::phone_key(secret(n))
}

async fn claim(
    service: &Service,
    caller: &Caller,
    id: FeederId,
    key: [u8; 32],
) -> Result<PhoneKey> {
    service
        .claim_phone(caller, id, PhoneClaim { key: Hex(key) })
        .await
}

/// A feeder adopted with an owner's invite, online, and its first owner.
struct Adopted {
    sim: Running,
    server: TestService,
    id: FeederId,
    ada: Caller,
}

async fn adopted() -> Adopted {
    let sim = feeder::start().await;
    adopted_on(sim, TestService::start().await).await
}

async fn adopted_on(sim: Running, server: TestService) -> Adopted {
    let address = sim.address.to_string();
    let id = server.adopt_as(&sim, &address, ClientRole::Owner).await;
    server.link(id, Link::Online).await;
    let ada = Caller::from(server.service().setup(ada_setup()).await.unwrap().0);
    Adopted {
        sim,
        server,
        id,
        ada,
    }
}

#[test]
fn an_expiring_token_or_account_claims_nothing() {
    local(async {
        let Adopted {
            sim,
            server,
            id,
            ada,
            ..
        } = adopted().await;
        let service = server.service();
        let key = paired(&sim, 1, ClientRole::Member).await;
        let expiring_token = on_phone(service, ada.user.clone(), Some(tomorrow())).await;
        let bob = account(service, &ada, "bob", Role::Member).await;
        let bob = on_phone(service, bob, None).await;
        let patch = wellfed_api::UserPatch {
            expires: Some(Some(tomorrow())),
            ..wellfed_api::UserPatch::default()
        };
        let sitter = service
            .update_user(&ada, None, bob.user.id, patch)
            .await
            .unwrap();
        let sitter = Caller {
            user: sitter,
            ..bob
        };

        for caller in [&expiring_token, &sitter] {
            assert!(matches!(
                claim(service, caller, id, key).await,
                Err(Error::Client(ClientError::DirectKeyRefused))
            ));
        }
        assert!(service.phones(&ada.user).await.unwrap().is_empty());
    });
}

#[test]
fn an_unconfirmed_claim_is_settled_by_claiming_again() {
    local(async {
        let mut quick = config();
        quick.timing.answer = Duration::from_secs(2);
        let flash = Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let address = sim.address;
        let Adopted {
            sim,
            server,
            id,
            ada,
        } = adopted_on(sim, TestService::start_with(quick).await).await;
        let service = server.service();
        let phone = on_phone(service, ada.user.clone(), None).await;
        let key = paired(&sim, 2, ClientRole::Owner).await;

        sim.stop().await;
        server.link(id, Link::Offline).await;
        assert!(matches!(
            claim(service, &phone, id, key).await,
            Err(Error::Client(ClientError::PhoneKeyUnconfirmed))
        ));
        assert!(service.phones(&ada.user).await.unwrap().is_empty());

        let _sim = feeder::restart(&flash, address).await;
        server.link(id, Link::Online).await;
        let recorded = claim(service, &phone, id, key).await.unwrap();
        let again = claim(service, &phone, id, key).await.unwrap();

        assert_eq!((recorded.user, recorded.token), (ada.user.id, phone.token));
        assert_eq!(recorded.role, ClientRole::Owner);
        assert_eq!(again, recorded, "asked again, the claim answers its record");
        assert_eq!(service.phones(&ada.user).await.unwrap(), [recorded]);
    });
}

/// The claim checked the caller, then the account was deleted or the
/// token revoked before the key was recorded: the reconcile marks the key,
/// the claim is refused and the feeder drops the key.
#[test]
fn a_claim_racing_a_deletion_or_a_revoked_token_is_revoked() {
    local(async {
        let Adopted {
            sim,
            server,
            id,
            ada,
            ..
        } = adopted().await;
        let service = server.service();
        let bob = account(service, &ada, "bob", Role::Owner).await;
        let bob = on_phone(service, bob, None).await;
        let ada_phone = on_phone(service, ada.user.clone(), None).await;
        let deleted = paired(&sim, 3, ClientRole::Owner).await;
        let revoked = paired(&sim, 4, ClientRole::Owner).await;

        service.delete_user(&ada, bob.user.id).await.unwrap();
        let token = ada_phone.token.unwrap();
        service.revoke_token(&ada, token).await.unwrap();

        for (caller, key) in [(&bob, deleted), (&ada_phone, revoked)] {
            assert!(matches!(
                claim(service, caller, id, key).await,
                Err(Error::Client(ClientError::DirectKeyRefused))
            ));
        }
        until("both claimed keys are revoked", || async {
            let held = feeder::client_keys(&sim).await;
            let recorded = service.phones(&ada.user).await.unwrap();
            [deleted, revoked]
                .iter()
                .all(|key| !held.contains(key) && !recorded.iter().any(|phone| phone.key.0 == *key))
        })
        .await;
    });
}

#[test]
fn a_key_the_account_cannot_hold_is_refused() {
    local(async {
        let Adopted {
            sim,
            server,
            id,
            ada,
            ..
        } = adopted().await;
        let service = server.service();
        let owner = on_phone(service, ada.user.clone(), None).await;
        let kept = paired(&sim, 5, ClientRole::Owner).await;
        service.keep_client(&ada, id, kept).await.unwrap();
        let server_role = paired(&sim, 6, ClientRole::Server).await;
        let members = paired(&sim, 8, ClientRole::Member).await;
        let clients = service.clients(id).await.unwrap();
        let own = clients
            .iter()
            .find(|client| client.owner == ClientOwner::Server)
            .unwrap()
            .key
            .0;

        let refused = |result: Result<PhoneKey>| match result {
            Err(Error::Client(error)) => error,
            other => panic!("expected a refusal, got {other:?}"),
        };
        assert_eq!(
            refused(claim(service, &owner, id, kept).await),
            ClientError::PhoneKeyKept
        );
        assert_eq!(
            refused(claim(service, &owner, id, server_role).await),
            ClientError::PhoneKeyNotAPhone
        );
        assert_eq!(
            refused(claim(service, &owner, id, own).await),
            ClientError::OwnKey
        );
        assert_eq!(
            refused(claim(service, &owner, id, feeder::phone_key(secret(9))).await),
            ClientError::PhoneKeyNotListed
        );

        let recorded = claim(service, &owner, id, members).await.unwrap();
        assert_eq!(
            recorded.role,
            ClientRole::Member,
            "the role the feeder lists"
        );
        let second = on_phone(service, ada.user.clone(), None).await;
        assert_eq!(
            refused(claim(service, &second, id, members).await),
            ClientError::KeyTaken,
            "another token's key"
        );
    });
}

/// A member's claim of a key the server has not recorded is refused before
/// anything is recorded or revoked: the claim could be of another person's
/// key. The key the server invited for the member's phone is answered.
#[test]
fn a_member_claims_only_a_key_recorded_as_theirs() {
    local(async {
        let Adopted {
            sim,
            server,
            id,
            ada,
            ..
        } = adopted().await;
        let service = server.service();
        let bob = account(service, &ada, "bob", Role::Member).await;
        let member = on_phone(service, bob, None).await;
        let cys = paired(&sim, 11, ClientRole::Member).await;
        let owners = paired(&sim, 12, ClientRole::Owner).await;
        let invite = service
            .invite_phone(
                &member,
                id,
                NewPhoneInvite {
                    key: Hex(feeder::phone_key(secret(13))),
                },
            )
            .await
            .unwrap();
        feeder::join_phone(&sim, &invite.invite, secret(13)).await;

        for key in [cys, owners] {
            assert!(matches!(
                claim(service, &member, id, key).await,
                Err(Error::Client(ClientError::PhoneClaimOwnersOnly))
            ));
        }
        let invited = claim(service, &member, id, feeder::phone_key(secret(13)))
            .await
            .unwrap();

        assert_eq!(
            (invited.user, invited.token),
            (member.user.id, member.token)
        );
        let phones = service.phones(&ada.user).await.unwrap();
        assert_eq!(phones, [invited], "nothing else recorded");
        let held = feeder::client_keys(&sim).await;
        assert!(
            held.contains(&cys) && held.contains(&owners),
            "nothing revoked"
        );
    });
}
