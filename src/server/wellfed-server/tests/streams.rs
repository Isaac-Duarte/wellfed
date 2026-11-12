//! An open event stream ends once its caller may no longer follow it as it
//! opened: its login ends, its token is revoked, its account expires, or
//! its role changes. While the caller holds, it stays open. The test
//! server checks each stream every [`STREAM_CHECK`].

use std::sync::Arc;
use std::time::Duration;

use wellfed_api::{Credentials, NewToken, NewUser, Role, StreamQuery, User, UserPatch};
use wellfed_server_client::{Auth, Client, Updates};
use wellfed_server_testkit::fixed::ManualClock;
use wellfed_server_testkit::{STREAM_CHECK, TestServer, WAIT, ada_setup};

const PASSWORD: &str = "temporary pass";

/// A second account, logged in.
async fn account(server: &TestServer, owner: &Client, role: Role) -> (Client, User) {
    account_until(server, owner, role, None).await
}

async fn account_until(
    server: &TestServer,
    owner: &Client,
    role: Role,
    expires: Option<jiff::Timestamp>,
) -> (Client, User) {
    let new = NewUser {
        name: "grace".into(),
        password: PASSWORD.into(),
        role,
        expires,
    };
    let user = owner.create_user(&new).await.unwrap();
    let mut client = server.client();
    client
        .login(&Credentials {
            name: "grace".into(),
            password: PASSWORD.into(),
        })
        .await
        .unwrap();
    (client, user)
}

async fn open(client: &Client) -> Updates {
    client.stream(&StreamQuery::default(), None).await.unwrap()
}

/// Read the stream until it ends.
async fn ends(updates: &mut Updates) {
    let ended = tokio::time::timeout(WAIT, async { while updates.next().await.is_some() {} });
    assert!(ended.await.is_ok(), "the stream ended in time");
}

/// The stream is still open after a few checks.
async fn stays(updates: &mut Updates) {
    let next = tokio::time::timeout(STREAM_CHECK * 5, updates.next()).await;
    assert!(next.is_err(), "the stream stayed open: {next:?}");
}

/// A demoted owner's stream ends rather than go on showing everyone's
/// journal; opened again, it shows what the new role sees.
#[tokio::test]
async fn a_stream_ends_when_its_users_role_changes() {
    let server = TestServer::start().await;
    let mut owner = server.client();
    owner.setup(&ada_setup()).await.unwrap();
    let (grace, user) = account(&server, &owner, Role::Owner).await;

    let mut updates = open(&grace).await;
    stays(&mut updates).await;
    let demote = UserPatch {
        role: Some(Role::Member),
        ..UserPatch::default()
    };
    owner.update_user(user.id, &demote).await.unwrap();
    ends(&mut updates).await;

    let mut again = open(&grace).await;
    stays(&mut again).await;
    server.stop().await;
}

#[tokio::test]
async fn a_stream_ends_with_its_login() {
    let server = TestServer::start().await;
    let mut owner = server.client();
    owner.setup(&ada_setup()).await.unwrap();
    let (mut grace, _) = account(&server, &owner, Role::Member).await;

    let mut updates = open(&grace).await;
    stays(&mut updates).await;
    grace.logout().await.unwrap();
    ends(&mut updates).await;
    server.stop().await;
}

#[tokio::test]
async fn a_stream_ends_when_its_token_is_revoked() {
    let server = TestServer::start().await;
    let mut owner = server.client();
    owner.setup(&ada_setup()).await.unwrap();
    let new = NewToken {
        name: "phone".into(),
        expires: None,
    };
    let created = owner.create_token(&new).await.unwrap();
    let phone = server
        .client()
        .with_auth(Some(Auth::Token(created.secret.clone())));

    let mut updates = open(&phone).await;
    stays(&mut updates).await;
    owner.revoke_token(created.token.id).await.unwrap();
    ends(&mut updates).await;
    server.stop().await;
}

#[tokio::test]
async fn a_stream_ends_when_its_account_expires() {
    let clock = ManualClock::new();
    let server = TestServer::start_with_clock(Arc::new(clock.clone())).await;
    let mut owner = server.client();
    owner.setup(&ada_setup()).await.unwrap();
    let now = wellfed_service::Clock::now(&clock);
    let expires = now.checked_add(jiff::SignedDuration::from_mins(1)).unwrap();
    let (grace, _) = account_until(&server, &owner, Role::Member, Some(expires)).await;

    let mut updates = open(&grace).await;
    stays(&mut updates).await;
    clock.advance(Duration::from_secs(120));
    ends(&mut updates).await;
    server.stop().await;
}

/// A stream resumed with the empty id a resync leaves starts from now, as
/// a browser's does, rather than being answered with another resync.
#[tokio::test]
async fn a_stream_resumed_with_the_id_a_resync_leaves_is_not_resynced() {
    let server = TestServer::start().await;
    let mut owner = server.client();
    owner.setup(&ada_setup()).await.unwrap();

    let mut updates = owner
        .stream(&StreamQuery::default(), Some(""))
        .await
        .unwrap();
    stays(&mut updates).await;
    server.stop().await;
}
