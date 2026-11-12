//! A key the journal was read for and held no write under is retired: a
//! client takes that answer as never sent, so a write under the key that
//! reaches the journal later, after a stall in front of it, is refused and
//! never reaches the feeder. A key with a write is never retired, and a
//! retired key is freed after a week.

use std::sync::Arc;
use std::time::Duration;

use wellfed_api::{Connection as Link, EventKind, EventQuery, FeederId, Role, User, UserId};
use wellfed_server_testkit::fixed::ManualClock;
use wellfed_server_testkit::{TestService, ada, feeder, local};
use wellfed_service::{CommandError, Error, IdempotencyKey, Resource, Service};

async fn locks(service: &Service, id: FeederId) -> usize {
    let query = EventQuery {
        kind: Some(EventKind::Lock),
        limit: Some(500),
        ..EventQuery::default()
    };
    service.events(id, query).await.unwrap().events.len()
}

fn not_found<T: std::fmt::Debug>(result: &Result<T, Error>) -> bool {
    matches!(result, Err(Error::NotFound(Resource::Command)))
}

fn retired<T: std::fmt::Debug>(result: &Result<T, Error>) -> bool {
    matches!(result, Err(Error::Command(CommandError::Retired)))
}

/// The client's request stalls in front of the journal while the client
/// reads its key and finds nothing, so takes the write as never sent. The
/// request then arrives: it is refused, and the feeder never locks.
#[test]
fn a_write_arriving_after_its_key_was_read_unfound_never_runs() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        let id = server.adopt(&sim).await;
        server.link(id, Link::Online).await;
        let service = server.service();
        let caller = ada().into();
        let key: IdempotencyKey = [0x69; 16];

        let read = service.command(&ada(), key).await;
        assert!(not_found(&read), "{read:?}");
        let late = service.set_lock(&caller, id, true, Some(key)).await;
        assert!(retired(&late), "{late:?}");
        let again = service.set_lock(&caller, id, true, Some(key)).await;
        assert!(retired(&again), "{again:?}");
        let read = service.command(&ada(), key).await;
        assert!(not_found(&read), "nothing was journaled: {read:?}");

        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(locks(service, id).await, 0, "the feeder never locked");
        service
            .set_lock(&caller, id, true, Some([0x6a; 16]))
            .await
            .unwrap();
        server.shutdown().await;
        sim.stop().await;
    });
}

/// A key whose write is journaled reads as that write, and another
/// account's read of it, which finds nothing it may see, retires nothing:
/// a retry under it is answered as always.
#[test]
fn a_journaled_key_is_never_retired() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        let id = server.adopt(&sim).await;
        server.link(id, Link::Online).await;
        let service = server.service();
        let caller = ada().into();
        let key: IdempotencyKey = [0x6b; 16];

        service
            .set_lock(&caller, id, true, Some(key))
            .await
            .unwrap();
        let grace = User {
            id: UserId(uuid::Uuid::from_u128(77)),
            name: "grace".into(),
            role: Role::Member,
            expires: None,
        };
        let theirs = service.command(&grace, key).await;
        assert!(not_found(&theirs), "{theirs:?}");
        service.command(&ada(), key).await.unwrap();
        service
            .set_lock(&caller, id, true, Some(key))
            .await
            .unwrap();
        server.shutdown().await;
        sim.stop().await;
    });
}

/// No request outlasts a week, so the expiry job frees a key retired that
/// long ago, and a write under it is taken again.
#[test]
fn a_retired_key_is_freed_after_a_week() {
    local(async {
        let sim = feeder::start().await;
        let clock = ManualClock::new();
        let server = TestService::start_with_clock(Arc::new(clock.clone())).await;
        let id = server.adopt(&sim).await;
        server.link(id, Link::Online).await;
        let service = server.service();
        let caller = ada().into();
        let key: IdempotencyKey = [0x6c; 16];

        assert!(not_found(&service.command(&ada(), key).await));
        clock.advance(Duration::from_secs(6 * 24 * 3600));
        service
            .run_expiry(wellfed_service::Clock::now(&clock))
            .await
            .unwrap();
        let held = service.set_lock(&caller, id, true, Some(key)).await;
        assert!(retired(&held), "still retired after six days: {held:?}");

        clock.advance(Duration::from_secs(2 * 24 * 3600));
        service
            .run_expiry(wellfed_service::Clock::now(&clock))
            .await
            .unwrap();
        service
            .set_lock(&caller, id, true, Some(key))
            .await
            .unwrap();
        server.shutdown().await;
        sim.stop().await;
    });
}
