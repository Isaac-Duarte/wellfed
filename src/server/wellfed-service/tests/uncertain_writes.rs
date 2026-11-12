//! Writes whose outcome the server cannot know stay unknown, and are never
//! answered as if they were known: an answer from a newer feeder that the
//! server cannot read, and a resend's refusal after the feeder forgot the
//! first send. A write the feeder answered without settling it is never
//! sent again.

use std::time::Duration;

use uuid::Uuid;
use wellfed_api::{
    ClientRole, CommandQuery, CommandState, CommandStateFilter, Connection as Link, FeederId,
    FeederSettingsPatch, Model, Settings,
};
use wellfed_server_testkit::feeder::{self, Control, Relay};
use wellfed_server_testkit::fixed::FixedClock;
use wellfed_server_testkit::{TempStore, TestService, ada, config, home, local};
use wellfed_service::{CommandError, Config, Error, IdempotencyKey, Service, seal_key};
use wellfed_store::NewFeeder;

/// The newer feeder's id on the server.
const NEWER: FeederId = FeederId(Uuid::from_u128(0x0192_c5d0_3333_7000_8000_0000_0000_0001));
/// The server's key on the newer feeder, which takes any client.
const SERVER_KEY: [u8; 32] = [0x55; 32];

fn quick() -> Config {
    let mut config = config();
    config.timing.answer = Duration::from_secs(2);
    config
}

fn unknown<T: std::fmt::Debug>(result: &Result<T, Error>) -> bool {
    matches!(result, Err(Error::Command(CommandError::Unknown)))
}

/// Store the newer feeder on `temp` as adoption would have, at its address,
/// before the service starts: so the service mirrors it from the start.
async fn store_newer(temp: &TempStore, newer: &feeder::Newer) {
    let store = temp.open().await;
    store.create_home(&home(), FixedClock.now()).await.unwrap();
    let key = seal_key(&config().key, NEWER, &newer.identity, &SERVER_KEY, [3; 12]).unwrap();
    let address = newer.address.to_string();
    store
        .insert_feeder(NewFeeder {
            id: NEWER,
            home: home().id,
            model: Model::Plaf103,
            identity: newer.identity,
            name: "newer",
            role: ClientRole::Owner,
            address: Some(&address),
            key,
            created: FixedClock.now(),
        })
        .await
        .unwrap();
}

async fn volume(service: &Service, id: FeederId) -> u8 {
    match service.settings(id).await.unwrap() {
        Settings::Feeder(settings) => settings.volume,
        Settings::Unknown => panic!("a feeder's settings"),
    }
}

/// The writes of `key` the server's uncertain list holds.
async fn listed_uncertain(server: &TestService, key: IdempotencyKey) -> usize {
    let uncertain = CommandQuery {
        state: Some(CommandStateFilter::Unknown),
        ..CommandQuery::default()
    };
    let page = server.service().commands(&ada(), uncertain).await.unwrap();
    page.items
        .iter()
        .filter(|entry| entry.key == Uuid::from_bytes(key))
        .count()
}

/// A newer feeder answers a lock change in an outcome this server cannot
/// read. The write is unknown, on the uncertain list, and a retry under its
/// key is answered unknown and sends nothing, at once or after the server
/// restarts, although a lock's rule would resend a write no answer came
/// for. The newer feeder's live samples, which the server cannot read
/// either, leave the session up throughout.
#[test]
fn an_unreadable_answer_is_unknown_and_never_sent_again() {
    local(async {
        let newer = feeder::Newer::start();
        let temp = TempStore::new();
        store_newer(&temp, &newer).await;
        let server = TestService::open(temp, config()).await;
        server.link(NEWER, Link::Online).await;

        let key: IdempotencyKey = [0x7e; 16];
        let caller = ada().into();
        let first = server
            .service()
            .set_lock(&caller, NEWER, true, Some(key))
            .await;
        assert!(unknown(&first), "{first:?}");
        let again = server
            .service()
            .set_lock(&caller, NEWER, true, Some(key))
            .await;
        assert!(unknown(&again), "{again:?}");
        assert_eq!(newer.writes().len(), 1, "sent once");

        let entry = server.service().command(&ada(), key).await.unwrap();
        assert_eq!((entry.state, entry.outcome), (CommandState::Unknown, None));
        assert_eq!(listed_uncertain(&server, key).await, 1);

        let store = server.stop().await;
        let server = TestService::reopen(store).await;
        server.link(NEWER, Link::Online).await;
        let after = server
            .service()
            .set_lock(&caller, NEWER, true, Some(key))
            .await;
        assert!(unknown(&after), "{after:?}");
        assert_eq!(newer.writes().len(), 1, "never sent again");
        assert_eq!(listed_uncertain(&server, key).await, 1);

        tokio::time::sleep(Duration::from_millis(300)).await;
        server.link(NEWER, Link::Online).await;
        server.shutdown().await;
    });
}

/// A settings change lands, its answer is lost, and the feeder restarts,
/// forgetting the request id. The retry resends it, the feeder runs it
/// again against the base the first run moved past, and refuses it stale.
/// That refusal says nothing of the first send: the retry is answered
/// unknown, the entry stays unknown and is never sent a third time, while
/// the feeder holds the change.
#[test]
fn a_resend_refused_after_a_restart_is_unknown_not_stale() {
    local(async {
        let sim = feeder::start().await;
        let relay = Relay::start(sim.address).await;
        let server = TestService::start_with(quick()).await;
        let id = server.adopt_at(&sim, &relay.address.to_string()).await;
        server.link(id, Link::Online).await;
        let service = server.service();

        let base = service.settings(id).await.unwrap().generation().unwrap();
        let patch = FeederSettingsPatch {
            volume: Some(30),
            ..FeederSettingsPatch::default()
        };
        let key: IdempotencyKey = [0x57; 16];
        let caller = ada().into();
        relay.hold_answers();
        let first = service
            .put_settings(&caller, id, base, patch, Some(key))
            .await;
        assert!(unknown(&first), "{first:?}");
        relay.cut();
        sim.control(Control::Reboot);
        relay.resume();
        server.link(id, Link::Online).await;

        let resent = service
            .put_settings(&caller, id, base, patch, Some(key))
            .await;
        assert!(unknown(&resent), "a stale refusal is no answer: {resent:?}");
        let entry = service.command(&ada(), key).await.unwrap();
        assert_eq!((entry.state, entry.outcome), (CommandState::Unknown, None));
        assert_eq!(volume(service, id).await, 30, "it landed");

        // Never a third time: the feeder would refuse it stale again, and
        // the change it would record is not this write's.
        let changes = service.settings(id).await.unwrap().generation();
        let third = service
            .put_settings(&caller, id, base, patch, Some(key))
            .await;
        assert!(unknown(&third), "{third:?}");
        assert_eq!(service.settings(id).await.unwrap().generation(), changes);
        server.shutdown().await;
        sim.stop().await;
    });
}

/// A revocation lands, its answer is lost, and the feeder restarts. The
/// resend finds the key gone and is refused `not_found`, which is what the
/// revocation asked whichever send removed it: the retry succeeds.
#[test]
fn a_resent_revocation_that_finds_the_key_gone_is_done() {
    local(async {
        let sim = feeder::start().await;
        let relay = Relay::start(sim.address).await;
        let server = TestService::start_with(quick()).await;
        let address = relay.address.to_string();
        let id = server.adopt_as(&sim, &address, ClientRole::Owner).await;
        server.link(id, Link::Online).await;
        let service = server.service();

        let invite = feeder::invite(&sim, ClientRole::Member).await;
        feeder::join_phone(&sim, &invite, [9; 32]).await;
        let stranger = feeder::phone_key([9; 32]);
        let key: IdempotencyKey = [0x4e; 16];
        let caller = ada().into();
        relay.hold_answers();
        let first = service
            .revoke_client(&caller, id, stranger, Some(key))
            .await;
        assert!(unknown(&first), "{first:?}");
        relay.cut();
        sim.control(Control::Reboot);
        relay.resume();
        server.link(id, Link::Online).await;

        service
            .revoke_client(&caller, id, stranger, Some(key))
            .await
            .unwrap();
        assert!(!feeder::client_keys(&sim).await.contains(&stranger));
        let entry = service.command(&ada(), key).await.unwrap();
        assert_eq!(entry.state, CommandState::Answered);
        server.shutdown().await;
        sim.stop().await;
    });
}
