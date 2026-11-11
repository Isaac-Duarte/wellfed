//! Keys a phone key invited: a member's phone (or whoever holds its token)
//! asks the feeder for a plain invite over its own session and enrolls
//! another key with it. The server attributes that key to the inviting
//! phone's account and token from the feeder's report of the join, so
//! deleting the account, revoking the token or revoking the inviting key
//! revokes it too, and a key it invited in turn. An owner who keeps such a
//! key detaches it, and then nothing of the inviter's revokes it. A join
//! the server read before it knew the inviting key is not attributed, and
//! a chain the feeder cut while the server was away still goes whole.

use std::sync::Arc;
use std::time::Duration;

use wellfed_api::{
    ClientOwner, ClientRole, Connection as Link, CoreEvent, EventBody, EventQuery, FeederId, Hex,
    NewPhoneInvite, NewToken, NewUser, PhoneClaim, Role, User,
};
use wellfed_server_testkit::feeder::{self, Running};
use wellfed_server_testkit::fixed::ManualClock;
use wellfed_server_testkit::{TestService, ada_setup, local, until};
use wellfed_service::{Caller, ClientError, Error, Service};

/// A phone's secret key, by number.
fn secret(n: u8) -> [u8; 32] {
    [n; 32]
}

fn key(n: u8) -> [u8; 32] {
    feeder::phone_key(secret(n))
}

async fn started() -> (Running, TestService, FeederId) {
    let sim = feeder::start().await;
    let server = TestService::start().await;
    let id = server
        .adopt_as(&sim, &sim.address.to_string(), ClientRole::Owner)
        .await;
    server.link(id, Link::Online).await;
    (sim, server, id)
}

/// The owner, and a member signed in on a phone with a token.
async fn ada_and_bob(service: &Service) -> (Caller, Caller) {
    let ada: Caller = service.setup(ada_setup()).await.unwrap().0.into();
    let new = NewUser {
        name: "bob".into(),
        password: "temporary pass".into(),
        role: Role::Member,
        expires: None,
    };
    let bob = service.create_user(&ada, new).await.unwrap();
    (ada, token_of(service, bob).await)
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

/// Phone `by`, already joined, invites phone `n` itself, without the
/// server, and phone `n` joins with that invite.
async fn invite_directly(sim: &Running, by: u8, n: u8) {
    let invite = feeder::phone_invite(sim, secret(by), ClientRole::Member).await;
    assert_eq!(
        feeder::join_phone(sim, &invite, secret(n)).await,
        ClientRole::Member
    );
}

/// Until the server records phone `n` as `caller`'s.
async fn attributed(service: &Service, id: FeederId, caller: &Caller, n: u8) {
    until("the invited key is attributed", || async {
        let clients = service.clients(id).await.unwrap();
        clients.iter().any(|client| {
            client.key.0 == key(n)
                && matches!(
                    &client.owner,
                    ClientOwner::Phone { user, token, revoking: None }
                        if *user == caller.user.id && *token == caller.token
                )
        })
    })
    .await;
}

/// Until the feeder no longer holds any of `phones`' keys and the server
/// records none of them.
async fn revoked(service: &Service, sim: &Running, phones: &[u8]) {
    let keys: Vec<[u8; 32]> = phones.iter().map(|&n| key(n)).collect();
    until("the keys are revoked", || async {
        let held = feeder::client_keys(sim).await;
        let owner = wellfed_server_testkit::ada();
        let recorded = service.phones(&owner).await.unwrap();
        keys.iter()
            .all(|key| !held.contains(key) && !recorded.iter().any(|phone| phone.key.0 == *key))
    })
    .await;
}

#[test]
fn a_members_invitees_go_when_the_member_is_deleted() {
    local(async {
        let (sim, server, id) = started().await;
        let service = server.service();
        let (ada, bob) = ada_and_bob(service).await;
        join(service, &sim, id, &bob, 1).await;

        // Bob's phone invites a second key, and that key a third.
        invite_directly(&sim, 1, 2).await;
        attributed(service, id, &bob, 2).await;
        invite_directly(&sim, 2, 3).await;
        attributed(service, id, &bob, 3).await;

        service.delete_user(&ada, bob.user.id).await.unwrap();
        revoked(service, &sim, &[1, 2, 3]).await;
        server.shutdown().await;
        sim.stop().await;
    });
}

#[test]
fn a_members_invitee_goes_when_the_token_is_revoked() {
    local(async {
        let (sim, server, id) = started().await;
        let service = server.service();
        let (_ada, bob) = ada_and_bob(service).await;
        join(service, &sim, id, &bob, 4).await;
        invite_directly(&sim, 4, 5).await;
        attributed(service, id, &bob, 5).await;

        let token = bob.token.unwrap();
        service.revoke_token(&bob, token).await.unwrap();
        revoked(service, &sim, &[4, 5]).await;
        server.shutdown().await;
        sim.stop().await;
    });
}

/// An owner revoking the inviting key itself takes its invitee with it.
#[test]
fn an_invitee_goes_with_the_key_that_invited_it() {
    local(async {
        let (sim, server, id) = started().await;
        let service = server.service();
        let (ada, bob) = ada_and_bob(service).await;
        join(service, &sim, id, &bob, 6).await;
        invite_directly(&sim, 6, 7).await;
        attributed(service, id, &bob, 7).await;

        service.revoke_client(&ada, id, key(6), None).await.unwrap();
        revoked(service, &sim, &[6, 7]).await;
        server.shutdown().await;
        sim.stop().await;
    });
}

/// The client list's owner of phone `n`, if the feeder lists it.
async fn owner_of(service: &Service, id: FeederId, n: u8) -> Option<ClientOwner> {
    let clients = service.clients(id).await.unwrap();
    let client = clients.iter().find(|client| client.key.0 == key(n))?;
    Some(client.owner.clone())
}

/// An owner who keeps a key a member's phone invited detaches it: it shows
/// as kept, outlives the member's removal, and no phone of the owner's can
/// claim it.
#[test]
fn a_kept_invitee_is_detached_from_its_inviter() {
    local(async {
        let (sim, server, id) = started().await;
        let service = server.service();
        let (ada, bob) = ada_and_bob(service).await;
        join(service, &sim, id, &bob, 10).await;
        invite_directly(&sim, 10, 11).await;
        attributed(service, id, &bob, 11).await;

        service.keep_client(&ada, id, key(11)).await.unwrap();
        assert_eq!(owner_of(service, id, 11).await, Some(ClientOwner::Kept));

        service.delete_user(&ada, bob.user.id).await.unwrap();
        revoked(service, &sim, &[10]).await;
        assert!(feeder::client_keys(&sim).await.contains(&key(11)));
        assert_eq!(owner_of(service, id, 11).await, Some(ClientOwner::Kept));

        let phone = token_of(service, ada.user.clone()).await;
        let claim = PhoneClaim { key: Hex(key(11)) };
        let claimed = service.claim_phone(&phone, id, claim).await;
        assert!(
            matches!(claimed, Err(Error::Client(ClientError::PhoneKeyKept))),
            "{claimed:?}"
        );
        server.shutdown().await;
        sim.stop().await;
    });
}

/// A member's phone that gets a new key keeps the keys its old one
/// invited: they move to the new key, so revoking that one takes them.
#[test]
fn a_replaced_keys_invitees_move_to_the_new_key() {
    local(async {
        let (sim, server, id) = started().await;
        let service = server.service();
        let (ada, bob) = ada_and_bob(service).await;
        join(service, &sim, id, &bob, 20).await;
        invite_directly(&sim, 20, 21).await;
        attributed(service, id, &bob, 21).await;

        join(service, &sim, id, &bob, 22).await;
        revoked(service, &sim, &[20]).await;
        assert!(feeder::client_keys(&sim).await.contains(&key(21)));
        attributed(service, id, &bob, 21).await;

        service
            .revoke_client(&ada, id, key(22), None)
            .await
            .unwrap();
        revoked(service, &sim, &[22, 21]).await;
        server.shutdown().await;
        sim.stop().await;
    });
}

/// Until the server has read the feeder's report that phone `n` joined.
async fn join_read(service: &Service, id: FeederId, n: u8) {
    until("the join is read", || async {
        let query = EventQuery {
            epoch: None,
            after: None,
            limit: Some(500),
            kind: None,
        };
        let page = service.events(id, query).await.unwrap();
        page.events.iter().any(|event| {
            matches!(&event.body, EventBody::Core(CoreEvent::Invited { client, .. })
                if client.0[..] == key(n)[..7])
        })
    })
    .await;
}

/// An owner's phone paired before the server knew it, and claimed later,
/// takes none of the invites it made before the claim: those keys stay
/// unowned. The invites it makes after the claim are its.
#[test]
fn a_claimed_phone_takes_only_the_invites_it_makes_after() {
    local(async {
        let sim = feeder::start().await;
        let clock = ManualClock::new();
        let server = TestService::start_with_clock(Arc::new(clock.clone())).await;
        let id = server
            .adopt_as(&sim, &sim.address.to_string(), ClientRole::Owner)
            .await;
        server.link(id, Link::Online).await;
        let service = server.service();
        let ada: Caller = service.setup(ada_setup()).await.unwrap().0.into();

        let invite = feeder::invite(&sim, ClientRole::Owner).await;
        feeder::join_phone(&sim, &invite, secret(30)).await;
        invite_directly(&sim, 30, 31).await;
        join_read(service, id, 31).await;

        clock.advance(Duration::from_secs(60));
        let phone = token_of(service, ada.user.clone()).await;
        let claim = PhoneClaim { key: Hex(key(30)) };
        service.claim_phone(&phone, id, claim).await.unwrap();
        invite_directly(&sim, 30, 32).await;
        attributed(service, id, &phone, 32).await;
        assert_eq!(owner_of(service, id, 31).await, Some(ClientOwner::Unowned));

        service
            .revoke_client(&ada, id, key(30), None)
            .await
            .unwrap();
        revoked(service, &sim, &[30, 32]).await;
        assert!(feeder::client_keys(&sim).await.contains(&key(31)));
        server.shutdown().await;
        sim.stop().await;
    });
}

/// An owner's phone invites a key, that key invites another, and the
/// phone revokes the first, all while the server is away. Back, the server
/// finds the first key gone, and the key it invited still goes with it.
#[test]
fn a_chain_the_feeder_cut_while_the_server_was_away_still_goes() {
    local(async {
        let (sim, server, id) = started().await;
        let service = server.service();
        let (ada, _bob) = ada_and_bob(service).await;
        let phone = token_of(service, ada.user.clone()).await;
        join(service, &sim, id, &phone, 40).await;

        let store = server.stop().await;
        invite_directly(&sim, 40, 41).await;
        invite_directly(&sim, 41, 42).await;
        feeder::phone_revoke(&sim, secret(40), key(41)).await;

        let server = TestService::reopen(store).await;
        let service = server.service();
        revoked(service, &sim, &[41, 42]).await;
        assert!(feeder::client_keys(&sim).await.contains(&key(40)));
        server.shutdown().await;
        sim.stop().await;
    });
}
