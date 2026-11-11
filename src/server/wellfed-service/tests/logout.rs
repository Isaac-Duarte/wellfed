//! A phone's logout, from a login session: its own keys, named by the
//! phone, and the keys they invited stay the account's with no token, so
//! the phone still reaches its feeders on the LAN and the account's
//! removal, demotion or expiry still revokes them. Any other key under the
//! token, one someone else got through it, goes with the token. The token
//! alone cannot log itself out that way; revoking it revokes every key.
//! A key with no token is left by the reconcile, a restart, another
//! token's revocation and another key's replacement, and a later claim by
//! the account's phone puts it under the claiming token again.

use std::time::Duration;

use wellfed_api::{
    ClientOwner, ClientRole, Connection as Link, FeederId, Hex, LogOut, NewPhoneInvite, NewToken,
    NewUser, PhoneClaim, Role, User, UserPatch,
};
use wellfed_server_testkit::feeder::{self, Running};
use wellfed_server_testkit::fixed::FixedClock;
use wellfed_server_testkit::{TestService, ada_setup, local, until};
use wellfed_service::{AccountError, Caller, Error, Service};

fn secret(n: u8) -> [u8; 32] {
    [n; 32]
}

fn key(n: u8) -> [u8; 32] {
    feeder::phone_key(secret(n))
}

async fn adopted(server: &TestService, sim: &Running) -> FeederId {
    let id = server
        .adopt_as(sim, &sim.address.to_string(), ClientRole::Owner)
        .await;
    server.link(id, Link::Online).await;
    id
}

/// The owner, and an account named `name` with `role` signed in on a
/// phone with a token.
async fn ada_and(service: &Service, name: &str, role: Role) -> (Caller, Caller) {
    let ada: Caller = service.setup(ada_setup()).await.unwrap().0.into();
    let new = NewUser {
        name: name.into(),
        password: "temporary pass".into(),
        role,
        expires: None,
    };
    let user = service.create_user(&ada, new).await.unwrap();
    (ada, token_of(service, user).await)
}

async fn token_of(service: &Service, user: User) -> Caller {
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

/// Phone `n` joins through the server's invite for `caller`.
async fn join(service: &Service, sim: &Running, id: FeederId, caller: &Caller, n: u8) {
    let request = NewPhoneInvite { key: Hex(key(n)) };
    let invite = service.invite_phone(caller, id, request).await.unwrap();
    feeder::join_phone(sim, &invite.invite, secret(n)).await;
}

/// Phone `by` invites phone `n` itself, without the server.
async fn invite_directly(sim: &Running, by: u8, n: u8) {
    let invite = feeder::phone_invite(sim, secret(by), ClientRole::Member).await;
    feeder::join_phone(sim, &invite, secret(n)).await;
}

/// The client list's owner of phone `n`, if the feeder lists it.
async fn owner_of(service: &Service, id: FeederId, n: u8) -> Option<ClientOwner> {
    let clients = service.clients(id).await.unwrap();
    let client = clients.iter().find(|client| client.key.0 == key(n))?;
    Some(client.owner.clone())
}

/// Until phone `n` is listed as `caller`'s, under its token.
async fn attributed(service: &Service, id: FeederId, caller: &Caller, n: u8) {
    until("the key is the token's", || async {
        matches!(owner_of(service, id, n).await,
            Some(ClientOwner::Phone { user, token, revoking: None })
                if user == caller.user.id && token == caller.token)
    })
    .await;
}

/// Phone `n` is listed as `user`'s, with no token.
async fn tokenless(service: &Service, id: FeederId, user: &User, n: u8) {
    let owner = owner_of(service, id, n).await;
    assert!(
        matches!(&owner, Some(ClientOwner::Phone { user: whose, token: None, revoking: None })
            if *whose == user.id),
        "{n}: {owner:?}"
    );
}

/// Until the feeder no longer holds any of `phones`' keys.
async fn revoked(sim: &Running, phones: &[u8]) {
    until("the keys are revoked", || async {
        let held = feeder::client_keys(sim).await;
        phones.iter().all(|&n| !held.contains(&key(n)))
    })
    .await;
}

/// `phone` logs out from its account's login, naming `own` as its keys.
async fn log_out(service: &Service, phone: &Caller, own: &[u8]) {
    let login = Caller {
        user: phone.user.clone(),
        token: None,
    };
    let request = LogOut {
        keys: own.iter().map(|&n| Hex(key(n))).collect(),
    };
    let token = phone.token.unwrap();
    service
        .log_out_token(&login, token, &request)
        .await
        .unwrap();
}

/// A member's phone `p` with a key `p + 1` it invited, logged out.
async fn logged_out_member(
    service: &Service,
    sim: &Running,
    id: FeederId,
    p: u8,
) -> (Caller, Caller) {
    let (ada, bob) = ada_and(service, "bob", Role::Member).await;
    join(service, sim, id, &bob, p).await;
    invite_directly(sim, p, p + 1).await;
    attributed(service, id, &bob, p + 1).await;
    log_out(service, &bob, &[p]).await;
    (ada, bob)
}

/// The logout keeps the phone's key and its invitee under the account
/// with no token; revoking another of the account's tokens leaves them, and
/// demoting the account revokes them. The token alone cannot ask, nor can
/// another account's session.
#[test]
fn a_phone_that_logs_out_keeps_its_keys_under_the_account() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        let id = adopted(&server, &sim).await;
        let service = server.service();
        let (ada, bob) = ada_and(service, "bob", Role::Member).await;
        join(service, &sim, id, &bob, 70).await;
        invite_directly(&sim, 70, 71).await;
        attributed(service, id, &bob, 71).await;
        let token = bob.token.unwrap();
        let request = LogOut {
            keys: vec![Hex(key(70))],
        };

        let by_token = service.log_out_token(&bob, token, &request).await;
        assert!(
            matches!(by_token, Err(Error::Account(AccountError::SessionRequired))),
            "{by_token:?}"
        );
        let by_another = service.log_out_token(&ada, token, &request).await;
        assert!(
            matches!(by_another, Err(Error::NotFound(_))),
            "{by_another:?}"
        );

        log_out(service, &bob, &[70]).await;
        let other = token_of(service, bob.user.clone()).await;
        service
            .revoke_token(&other, other.token.unwrap())
            .await
            .unwrap();
        for n in [70, 71] {
            tokenless(service, id, &bob.user, n).await;
        }

        let viewer = UserPatch {
            role: Some(Role::Viewer),
            ..UserPatch::default()
        };
        service
            .update_user(&ada, None, bob.user.id, viewer)
            .await
            .unwrap();
        revoked(&sim, &[70, 71]).await;
        server.shutdown().await;
        sim.stop().await;
    });
}

/// Deleting the account revokes the keys a logout left it.
#[test]
fn deleting_the_account_revokes_the_keys_a_logout_left_it() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        let id = adopted(&server, &sim).await;
        let service = server.service();
        let (ada, bob) = logged_out_member(service, &sim, id, 74).await;

        tokenless(service, id, &bob.user, 74).await;
        service.delete_user(&ada, bob.user.id).await.unwrap();
        revoked(&sim, &[74, 75]).await;
        server.shutdown().await;
        sim.stop().await;
    });
}

/// An expiry put on the account revokes the keys a logout left it.
#[test]
fn an_expiry_on_the_account_revokes_the_keys_a_logout_left_it() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        let id = adopted(&server, &sim).await;
        let service = server.service();
        let (ada, bob) = logged_out_member(service, &sim, id, 76).await;

        let expiring = UserPatch {
            expires: Some(Some(FixedClock.now() + Duration::from_secs(86_400))),
            ..UserPatch::default()
        };
        service
            .update_user(&ada, None, bob.user.id, expiring)
            .await
            .unwrap();
        revoked(&sim, &[76, 77]).await;
        server.shutdown().await;
        sim.stop().await;
    });
}

/// Someone holding the phone's token gets a key of their own on another
/// feeder. The phone's logout names only its own key, so the other key
/// goes with the token while the phone's key and its invitee stay the
/// account's.
#[test]
fn a_key_someone_else_got_through_the_token_goes_at_logout() {
    local(async {
        let sim = feeder::start().await;
        let hall = feeder::start().await;
        let server = TestService::start().await;
        let kitchen = adopted(&server, &sim).await;
        let id = adopted(&server, &hall).await;
        let service = server.service();
        let (_ada, bob) = ada_and(service, "bob", Role::Member).await;
        join(service, &sim, kitchen, &bob, 80).await;
        invite_directly(&sim, 80, 81).await;
        attributed(service, kitchen, &bob, 81).await;
        // The thief's key, through the same token, on the hall feeder.
        join(service, &hall, id, &bob, 82).await;

        log_out(service, &bob, &[80]).await;
        revoked(&hall, &[82]).await;
        for n in [80, 81] {
            tokenless(service, kitchen, &bob.user, n).await;
        }
        server.shutdown().await;
        hall.stop().await;
        sim.stop().await;
    });
}

/// A key with no token stays so through a restart (the reconcile at start
/// and at each connect) and through the account's new phone token getting
/// a key of its own on the feeder (one key per token replaces only that
/// token's). A claim by the account's phone puts it under the claiming
/// token again, so revoking that token revokes it.
#[test]
fn a_key_with_no_token_survives_the_reconcile_and_is_claimed_again() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        let id = adopted(&server, &sim).await;
        let (ada, _bob) = ada_and(server.service(), "bob", Role::Member).await;
        let phone = token_of(server.service(), ada.user.clone()).await;
        join(server.service(), &sim, id, &phone, 90).await;
        log_out(server.service(), &phone, &[90]).await;

        let store = server.stop().await;
        let server = TestService::reopen(store).await;
        server.link(id, Link::Online).await;
        let service = server.service();
        tokenless(service, id, &ada.user, 90).await;

        let again = token_of(service, ada.user.clone()).await;
        join(service, &sim, id, &again, 92).await;
        tokenless(service, id, &ada.user, 90).await;

        let claimed = service
            .claim_phone(&again, id, PhoneClaim { key: Hex(key(90)) })
            .await
            .unwrap();
        assert_eq!(claimed.token, again.token);
        // One key per token: the claimed key replaces the token's other.
        revoked(&sim, &[92]).await;
        service
            .revoke_token(&again, again.token.unwrap())
            .await
            .unwrap();
        revoked(&sim, &[90]).await;
        server.shutdown().await;
        sim.stop().await;
    });
}

/// An owner who keeps the invitee a logout left the account detaches it,
/// as any invitee: it outlives the account, and the phone's own key, which
/// a keep leaves the account's, does not.
#[test]
fn keeping_a_tokenless_invitee_detaches_it() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        let id = adopted(&server, &sim).await;
        let service = server.service();
        let (ada, bob) = logged_out_member(service, &sim, id, 94).await;

        service.keep_client(&ada, id, key(95)).await.unwrap();
        assert_eq!(owner_of(service, id, 95).await, Some(ClientOwner::Kept));
        service.keep_client(&ada, id, key(94)).await.unwrap();
        tokenless(service, id, &bob.user, 94).await;

        service.delete_user(&ada, bob.user.id).await.unwrap();
        revoked(&sim, &[94]).await;
        assert!(feeder::client_keys(&sim).await.contains(&key(95)));
        server.shutdown().await;
        sim.stop().await;
    });
}

/// A token that logs itself out without a login takes the cascading path:
/// its phone key and the key it invited go, and neither stays.
#[test]
fn a_bare_tokens_logout_revokes_its_keys() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        let id = adopted(&server, &sim).await;
        let service = server.service();
        let (_ada, bob) = ada_and(service, "bob", Role::Member).await;
        join(service, &sim, id, &bob, 84).await;
        invite_directly(&sim, 84, 85).await;
        attributed(service, id, &bob, 85).await;
        let token = bob.token.unwrap();

        let request = LogOut {
            keys: vec![Hex(key(84))],
        };
        let kept = service.log_out_token(&bob, token, &request).await;
        assert!(
            matches!(kept, Err(Error::Account(AccountError::SessionRequired))),
            "{kept:?}"
        );
        service.revoke_token(&bob, token).await.unwrap();
        revoked(&sim, &[84, 85]).await;
        server.shutdown().await;
        sim.stop().await;
    });
}
