//! The server end to end: real HTTP in front, and behind it the service
//! speaking the device API (Noise over TCP) to the simulated feeder.

use jiff::Timestamp;
use uuid::Uuid;
use wellfed_api::{
    AdoptFeeder, ClientRole, Connection, Credentials, ErrorCode, EventBody, EventKind, EventQuery,
    FeederId, NewUser, NextMeal, PLANS, Plan, Plans, PlansChange, Role, Settings, SettingsPatch,
    Source, StreamEvent, StreamQuery, Weekdays,
};
use wellfed_server_client::reqwest::header::ETAG;
use wellfed_server_client::{
    Error, Lock, Method, StatusCode, Update, Updates, decode, expect_status,
};
use wellfed_server_testkit::{TestServer, WAIT, ada_setup, feeder, local, refused, until};

/// The next update, failing the test when the stream ends or breaks.
async fn next(updates: &mut Updates) -> Update {
    updates.next().await.expect("the stream is open").unwrap()
}

#[test]
fn an_owner_adopts_a_feeder_and_works_it_over_http() {
    local(async {
        let sim = feeder::start().await;
        let server = TestServer::start().await;
        let mut http = server.client();

        http.setup(&ada_setup()).await.unwrap();

        let kitchen = http
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
            http.feeders().await.unwrap()[0].connection == Connection::Online
        })
        .await;

        let power = StreamQuery {
            samples: Some("power".into()),
        };
        let mut updates = http.stream(&power, None).await.unwrap();

        let detail = http.feeder(kitchen).await.unwrap();
        let status = detail.status.unwrap();
        assert!(status.live);
        assert!(status.status.clock.is_some(), "the server set the clock");

        // Settings: an edit names its generation; a retry with the same
        // key is answered once; a stale edit gets the current settings.
        let read = http
            .request(Method::GET, &format!("/api/v1/feeders/{kitchen}/settings"))
            .send()
            .await
            .unwrap();
        let read = expect_status(read, StatusCode::OK).await.unwrap();
        let etag = read.headers()[ETAG].to_str().unwrap().to_owned();
        let settings: Settings = decode(read, StatusCode::OK).await.unwrap();
        assert_eq!(etag, format!("\"{}\"", settings.generation));
        assert_eq!(http.settings(kitchen).await.unwrap(), settings);

        let key = Uuid::from_u128(0x0192c5d0_1111_7000_8000_000000000001);
        let patch = SettingsPatch {
            volume: Some(35),
            ..SettingsPatch::default()
        };
        let changed = http
            .put_settings(kitchen, settings.generation, &patch, key)
            .await
            .unwrap();
        assert_eq!(changed.volume, 35);
        let retried = http
            .put_settings(kitchen, settings.generation, &patch, key)
            .await
            .unwrap();
        assert_eq!(retried, changed);
        let other = Uuid::from_u128(0x0192c5d0_1111_7000_8000_000000000002);
        let stale = http.put_settings(kitchen, settings.generation, &patch, other);
        let Err(Error::Stale(current)) = stale.await else {
            panic!("a stale edit is answered with the current settings");
        };
        assert_eq!(*current, changed);

        let lock = Uuid::from_u128(0x0192c5d0_1111_7000_8000_000000000003);
        http.set_lock(kitchen, Lock::Unlocked, lock).await.unwrap();

        // The lock's event and a power sample reach the stream.
        let (mut lock_seen, mut power_seen, mut first) = (None, false, None);
        tokio::time::timeout(WAIT, async {
            while !(lock_seen.is_some() && power_seen) {
                let Update { id, event } = next(&mut updates).await;
                if first.is_none() {
                    first = Some(id.clone());
                }
                match event {
                    StreamEvent::Event { event, .. }
                        if matches!(event.body, EventBody::Lock { locked: false, .. }) =>
                    {
                        lock_seen = Some(id);
                    }
                    StreamEvent::Sample { .. } => power_seen = true,
                    _ => {}
                }
            }
        })
        .await
        .expect("the lock and a sample arrive live");

        // A reconnect from the first update gets everything after it, the
        // lock among them; one from another run is told to resync.
        let first = first.unwrap();
        let mut replay = http
            .stream(&StreamQuery::default(), Some(&first))
            .await
            .unwrap();
        tokio::time::timeout(WAIT, async {
            loop {
                let Update { id, event } = next(&mut replay).await;
                assert_ne!(id, first, "replay starts after the id");
                assert!(
                    !matches!(event, StreamEvent::Sample { .. }),
                    "not asked for"
                );
                if Some(&id) == lock_seen.as_ref() {
                    return;
                }
            }
        })
        .await
        .expect("the replay includes the lock");
        let mut stale = http
            .stream(&StreamQuery::default(), Some("0000000000000001-0"))
            .await
            .unwrap();
        let Update { id, event } = next(&mut stale).await;
        assert_eq!((id.as_str(), event), ("", StreamEvent::Resync));

        let locks = EventQuery {
            kind: Some(EventKind::Lock),
            ..EventQuery::default()
        };
        let page = http.events(kitchen, &locks).await.unwrap();
        let [event] = page.events.as_slice() else {
            panic!("{page:?}");
        };
        assert!(matches!(
            event.body,
            EventBody::Lock {
                locked: false,
                by: Source::Client { .. }
            }
        ));

        server.stop().await;
        sim.stop().await;
    });
}

/// Set up, adopt `sim` and wait until the server holds a session with it.
async fn adopted(http: &mut wellfed_server_client::Client, sim: &feeder::Running) -> FeederId {
    http.setup(&ada_setup()).await.unwrap();
    let id = http
        .adopt(&AdoptFeeder {
            invite: feeder::invite(sim, ClientRole::Server).await,
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

/// Plans over HTTP: the `ETag` is the generation, a change needs
/// `If-Match`, a stale one gets `412` with the current plans, and the
/// feeder's detail carries the next meal.
#[test]
fn plans_change_over_http_against_their_generation() {
    local(async {
        let sim = feeder::start().await;
        let server = TestServer::start().await;
        let mut http = server.client();
        let kitchen = adopted(&mut http, &sim).await;
        let path = format!("/api/v1/feeders/{kitchen}/plans");

        let read = http.request(Method::GET, &path).send().await.unwrap();
        let read = expect_status(read, StatusCode::OK).await.unwrap();
        let etag = read.headers()[ETAG].to_str().unwrap().to_owned();
        let plans: Plans = decode(read, StatusCode::OK).await.unwrap();
        assert_eq!(etag, format!("\"{}\"", plans.generation));

        let mut table = [None; PLANS];
        table[1] = Some(Plan {
            minute: 18 * 60,
            portions: 2,
            days: Weekdays::DAILY,
            enabled: true,
            call: 0,
        });
        let change = PlansChange { plans: table };
        let unconditional = http.request(Method::PUT, &path).json(&change).send();
        refused(
            expect_status(unconditional.await.unwrap(), StatusCode::OK).await,
            StatusCode::PRECONDITION_REQUIRED,
            ErrorCode::PreconditionRequired,
        );

        let key = Uuid::from_u128(0x0192c5d0_2222_7000_8000_000000000001);
        let changed = http
            .put_plans(kitchen, plans.generation, &change, key)
            .await
            .unwrap();
        assert_eq!(changed.plans, table);
        let other = Uuid::from_u128(0x0192c5d0_2222_7000_8000_000000000002);
        let stale = http.put_plans(kitchen, plans.generation, &change, other);
        let Err(Error::StalePlans(current)) = stale.await else {
            panic!("a stale change is answered with the current plans");
        };
        assert_eq!(*current, changed);

        // The fixed clock reads 2026-09-21T14:13:20Z: 18:00 in Berlin's
        // summer time is 16:00Z that day. The server's copy follows the
        // change once its mirror has read the plans again.
        let want = NextMeal {
            at: Timestamp::from_second(1_790_006_400).unwrap(),
            slot: 1,
            portions: 2,
        };
        until("the next meal follows the change", || async {
            http.feeder(kitchen).await.unwrap().next_meal == Some(want)
        })
        .await;

        // A member controls the feeders, plans included.
        let bob = NewUser {
            name: "bob".into(),
            password: "member password".into(),
            role: Role::Member,
            expires: None,
        };
        http.create_user(&bob).await.unwrap();
        let mut member = server.client();
        let login = Credentials {
            name: "bob".into(),
            password: "member password".into(),
        };
        member.login(&login).await.unwrap();
        let empty = PlansChange {
            plans: [None; PLANS],
        };
        let key = Uuid::from_u128(0x0192c5d0_2222_7000_8000_000000000003);
        let emptied = member
            .put_plans(kitchen, changed.generation, &empty, key)
            .await
            .unwrap();
        assert_eq!(emptied.plans, [None; PLANS]);

        server.stop().await;
        sim.stop().await;
    });
}

/// Every event `connection` is pushed, up to and including the first `Fed`.
async fn direct_until_fed(connection: &mut feeder::Connection) -> Vec<(u64, EventBody)> {
    let mut seen = Vec::new();
    loop {
        match connection.next(WAIT).await.unwrap() {
            wellfed_model::Pushed::Event(event) => {
                let fed = matches!(event.body, EventBody::Fed { .. });
                seen.push((event.seq, event.body));
                if fed {
                    return seen;
                }
            }
            wellfed_model::Pushed::Gap(gap) => panic!("events lost: {gap:?}"),
            _ => {}
        }
    }
}

/// The server and two clients talking to the feeder directly, a phone and
/// the owner's, follow the events at once, and each sees the phone's feed
/// end in the same `Fed`: a session of the feeder's own is not starved by
/// the server's.
#[test]
fn the_server_and_two_direct_sessions_each_see_the_same_feed() {
    local(async {
        const PHONE: [u8; 32] = [0x44; 32];

        let sim = feeder::start().await;
        let server = TestServer::start().await;
        let mut http = server.client();
        adopted(&mut http, &sim).await;
        let mut updates = http.stream(&StreamQuery::default(), None).await.unwrap();

        let invite = feeder::invite(&sim, ClientRole::Member).await;
        assert_eq!(
            feeder::join_phone(&sim, &invite, PHONE).await,
            ClientRole::Member
        );
        let mut phone = feeder::open_phone(&sim, PHONE).await;
        let mut owner = feeder::open_owner(&sim).await;
        phone.subscribe(None, false).await.unwrap();
        owner.subscribe(None, false).await.unwrap();

        let answer = phone
            .ask([0x51; 16], &wellfed_model::Op::Feed { portions: 1 })
            .await
            .unwrap();
        assert!(
            matches!(answer, wellfed_model::Answer::Feeding { .. }),
            "{answer:?}"
        );

        let phone_saw = direct_until_fed(&mut phone).await;
        let owner_saw = direct_until_fed(&mut owner).await;
        assert_eq!(phone_saw, owner_saw, "both replay the same history");
        let fed = phone_saw.last().cloned().unwrap();

        let served = tokio::time::timeout(WAIT, async {
            loop {
                if let Update {
                    event: StreamEvent::Event { event, .. },
                    ..
                } = next(&mut updates).await
                    && matches!(event.body, EventBody::Fed { .. })
                {
                    return event;
                }
            }
        })
        .await
        .expect("the server's stream carries the feed");
        assert_eq!(served.body, fed.1, "the same Fed the phone saw");

        server.stop().await;
        sim.stop().await;
    });
}
