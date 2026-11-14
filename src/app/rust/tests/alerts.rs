//! Alerts at home against the simulated feeder (simulation, not hardware):
//! - a catch-up reads the feeder's table and posts a failed meal once; the
//!   next finds the table where it was and posts nothing, and a notice the
//!   app never confirmed is handed over again;
//! - a dismissal hides it on this phone and takes its notification off,
//!   and the feeder still holds it;
//! - a stopped feeder is unanswered at the first try and failed at the
//!   last, and only on its home network does that count; elsewhere the
//!   phone is away and nothing counts;
//! - a computer on its network reads the table as a phone does, and one on
//!   a network it can't name still dials;
//! - nothing a catch-up sends is a write but the clock sync, on any network
//!   the phone or computer is on: every store, the clients and the feeds
//!   stay as they were, and the clock's sync count moves only when the clock
//!   rule allowed a sync for the view the catch-up read. A clock that is not
//!   set is set once, and a set one left alone; with a time server named,
//!   only once the clock has read unset across two minutes of catch-ups.

use std::rc::Rc;
use std::time::Duration;
use wellfed_connect::Samples;

use wellfed_app_bridge::api::alerts::{
    Attempt, CatchUp, CatchUpEnd, NoticeChange, NoticeDetail, PhoneNetwork, PostedNotice,
    catch_up_with, confirm_notices, dismiss_notice, feeder_alerts, forget_alerts,
};
use wellfed_app_bridge::api::keys::ClientKey;
use wellfed_app_bridge::api::model::{FeedOutcome, FeedReason, NoticeKind};
use wellfed_app_bridge::api::store::{AppStore, KnownFeeder, load_store, swap_store};
use wellfed_connect::Connection;
use wellfed_feeder_sim::feeding;
use wellfed_feeder_sim::flash::Flash;
use wellfed_feeder_sim::harness::{Running, local};
use wellfed_feeder_sim::serve::{Control, FeedFault};
use wellfed_model::{Answer, EventBody, FeederEvent, NtpServer, Op, Pushed, TimeChanges};
use wellfed_session::noise::Keypair;

const OWNER: [u8; 32] = [0x21; 32];
/// The most any one test here may take: a hang fails rather than blocks.
const LIMIT: Duration = Duration::from_secs(120);

/// `test` on the simulator's runtime, failed if it outlasts [`LIMIT`].
fn bounded(test: impl std::future::Future<Output = ()>) {
    local(async {
        tokio::time::timeout(LIMIT, test)
            .await
            .expect("the test outlasted its limit");
    });
}

fn home() -> PhoneNetwork {
    PhoneNetwork::Wifi {
        gateway: "192.168.1.1".into(),
        subnet: "192.168.1.0/24".into(),
    }
}

/// The app's own file in `dir` lists exactly `identities`, as the app saves
/// a feeder before it ever catches up with it.
fn know(dir: &str, identities: &[&str]) {
    let store = AppStore {
        feeders: identities
            .iter()
            .map(|identity| KnownFeeder {
                identity: (*identity).to_owned(),
                name: "Kitchen".into(),
                address: None,
                manual_address: None,
            })
            .collect(),
        ..AppStore::default()
    };
    let path = format!("{dir}/wellfed.json");
    assert!(swap_store(&path, &load_store(&path).unwrap(), &store).unwrap());
}

fn hex(identity: &[u8; 32]) -> String {
    identity.iter().map(|byte| format!("{byte:02x}")).collect()
}

async fn catch_up(
    dir: &str,
    sim: &Running,
    identity: &str,
    attempt: Attempt,
    network: PhoneNetwork,
) -> CatchUp {
    let key = ClientKey::restore(OWNER.to_vec()).unwrap();
    let known = Some(sim.address.to_string());
    catch_up_with(
        dir.into(),
        &key,
        identity.into(),
        None,
        known,
        attempt,
        network,
    )
    .await
    .unwrap()
}

/// A jammed feed of 2 portions, run to its `Fed` end by the owner.
async fn jam(sim: &Running) {
    sim.control(Control::FaultFeed(FeedFault::Jam));
    let mut owner = Connection::open(
        &Keypair::from_secret(OWNER),
        sim.identity,
        sim.address,
        [0xa1; 32],
    )
    .await
    .unwrap();
    owner.subscribe(None, Samples::Off).await.unwrap();
    let answer = owner.ask([1; 16], &Op::Feed { portions: 2 }).await.unwrap();
    assert!(matches!(answer, Answer::Feeding { .. }), "{answer:?}");
    loop {
        let pushed = owner.next(Duration::from_secs(20)).await.unwrap();
        if let Pushed::Event(event) = pushed
            && matches!(event.body, EventBody::Feeder(FeederEvent::Fed { .. }))
        {
            return;
        }
    }
}

fn posts(catch_up: &CatchUp) -> Vec<NoticeKind> {
    catch_up
        .changes
        .iter()
        .filter_map(|change| match change {
            NoticeChange::Post { notice } => Some(notice.kind),
            NoticeChange::Remove { .. } => None,
        })
        .collect()
}

#[test]
fn a_failed_meal_posts_once_and_a_dismissal_stays_on_the_phone() {
    bounded(async {
        let kept = tempfile::tempdir().unwrap();
        let dir = kept.path().display().to_string();
        let flash: Rc<Flash> = Flash::memory();
        let sim = Running::start(&flash, Some(*Keypair::from_secret(OWNER).public())).await;
        let identity = hex(&sim.identity);
        know(&dir, &[&identity]);

        let first = catch_up(&dir, &sim, &identity, Attempt::First, home()).await;
        assert_eq!(first.end, CatchUpEnd::Read);
        assert_eq!(posts(&first), [], "{first:?}");

        jam(&sim).await;
        let jammed = catch_up(&dir, &sim, &identity, Attempt::First, home()).await;
        assert_eq!(jammed.end, CatchUpEnd::Read);
        assert_eq!(posts(&jammed), [NoticeKind::MealFailed]);
        let NoticeChange::Post { notice } = &jammed.changes[0] else {
            panic!("{jammed:?}");
        };
        assert!(
            matches!(
                notice.detail,
                NoticeDetail::Feed {
                    feed: 1,
                    requested: 2,
                    outcome: Some(FeedOutcome::Failed),
                    reason: FeedReason::Jammed,
                    ..
                }
            ),
            "{notice:?}"
        );

        // Not confirmed: the next catch-up hands it over again, though the
        // table has not moved.
        let again = catch_up(&dir, &sim, &identity, Attempt::First, home()).await;
        assert_eq!(again.end, CatchUpEnd::Unchanged);
        assert_eq!(posts(&again), [NoticeKind::MealFailed]);

        // Dart may spell the feeder in either case; the file keys it once.
        let shouted = identity.to_uppercase();
        let taken_off = confirm_notices(
            dir.clone(),
            shouted.clone(),
            vec![PostedNotice::from(notice)],
            vec![],
        )
        .await
        .unwrap();
        assert_eq!(
            taken_off,
            Vec::<i32>::new(),
            "confirmed, and left on the phone"
        );
        let listed = feeder_alerts(dir.clone(), shouted.clone()).await.unwrap();
        assert_eq!(listed.notices.len(), 1);
        let quiet = catch_up(&dir, &sim, &identity, Attempt::First, home()).await;
        assert_eq!(quiet.end, CatchUpEnd::Unchanged);
        assert_eq!(quiet.changes, [], "posted once");

        let dismissed = dismiss_notice(dir.clone(), shouted.clone(), notice.id)
            .await
            .unwrap();
        assert_eq!(dismissed, [NoticeChange::Remove { id: notice.id }]);
        confirm_notices(dir.clone(), identity.clone(), vec![], vec![notice.id])
            .await
            .unwrap();
        assert_eq!(
            feeder_alerts(dir.clone(), identity.clone())
                .await
                .unwrap()
                .notices,
            []
        );
        let after = catch_up(&dir, &sim, &identity, Attempt::First, home()).await;
        assert_eq!(
            after.changes,
            [],
            "dismissed here only, and not posted again"
        );

        // Forgotten: a catch-up that ends after the app forgot the feeder
        // (one already out, or a background round's) brings nothing back.
        know(&dir, &[]);
        forget_alerts(dir.clone(), identity.clone()).await.unwrap();
        let forgotten = catch_up(&dir, &sim, &identity, Attempt::First, home()).await;
        assert_eq!(forgotten.changes, []);
        let listed = feeder_alerts(dir.clone(), identity.clone()).await.unwrap();
        assert_eq!(listed.notices, []);

        assert_eq!(feeding::reserved(&flash), Ok(1), "the owner's feed alone");
        sim.stop().await;
    });
}

/// The feeder's time view, read as the owner.
async fn time(sim: &Running) -> wellfed_model::TimeView {
    let mut owner = Connection::open(
        &Keypair::from_secret(OWNER),
        sim.identity,
        sim.address,
        [0xa2; 32],
    )
    .await
    .unwrap();
    let Answer::Time(time) = owner.ask([2; 16], &Op::GetTime).await.unwrap() else {
        panic!("no time view");
    };
    time
}

/// The owner sets the feeder's time source.
async fn ntp(sim: &Running, server: NtpServer) {
    let base = time(sim).await.generation;
    let mut owner = Connection::open(
        &Keypair::from_secret(OWNER),
        sim.identity,
        sim.address,
        [0xa3; 32],
    )
    .await
    .unwrap();
    let changes = TimeChanges {
        zone: None,
        ntp: Some(server),
    };
    let answer = owner
        .ask([3; 16], &Op::PutTime { base, changes })
        .await
        .unwrap();
    assert!(matches!(answer, Answer::Done { .. }), "{answer:?}");
}

/// Move the start of the invalid run `alerts.json` keeps back by `by_ms`, as
/// though the catch-ups that saw it were that much further apart.
fn backdate_invalid_run(dir: &str, by_ms: i64) {
    let path = format!("{dir}/alerts.json");
    let mut file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut moved = 0;
    let mut stack = vec![&mut file];
    while let Some(value) = stack.pop() {
        match value {
            serde_json::Value::Object(fields) => {
                if let Some(since) = fields.get_mut("since_ms") {
                    *since = (since.as_i64().unwrap() - by_ms).into();
                    moved += 1;
                }
                stack.extend(fields.values_mut());
            }
            serde_json::Value::Array(items) => stack.extend(items.iter_mut()),
            _ => {}
        }
    }
    assert_eq!(moved, 1, "one feeder's clock reads invalid");
    std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
}

#[test]
fn with_no_time_server_a_catch_up_sets_a_clock_at_once_and_leaves_a_set_one() {
    bounded(async {
        let kept = tempfile::tempdir().unwrap();
        let dir = kept.path().display().to_string();
        let flash: Rc<Flash> = Flash::memory();
        let sim = Running::start(&flash, Some(*Keypair::from_secret(OWNER).public())).await;
        let identity = hex(&sim.identity);
        know(&dir, &[&identity]);
        ntp(&sim, NtpServer::Off).await;
        assert_eq!(
            time(&sim).await.clock_ms,
            None,
            "a fresh feeder needs the time"
        );

        let first = catch_up(&dir, &sim, &identity, Attempt::First, home()).await;
        assert_eq!(first.end, CatchUpEnd::Read);
        let set = time(&sim).await;
        assert!(set.clock_ms.is_some(), "{set:?}");
        assert_eq!(set.last, Some(wellfed_model::TimeSource::Owner));
        assert_eq!(set.syncs, 1);

        // Its error is a second or so, far from the minute an owner's phone
        // waits for, and the last sync was moments ago.
        catch_up(&dir, &sim, &identity, Attempt::First, home()).await;
        assert_eq!(time(&sim).await.syncs, 1, "a set clock is left alone");

        assert_eq!(feeding::reserved(&flash), Ok(0), "no feed");
        sim.stop().await;
    });
}

#[test]
fn with_a_time_server_a_catch_up_gives_it_two_minutes_before_setting_the_clock() {
    bounded(async {
        let kept = tempfile::tempdir().unwrap();
        let dir = kept.path().display().to_string();
        let flash: Rc<Flash> = Flash::memory();
        let sim = Running::start(&flash, Some(*Keypair::from_secret(OWNER).public())).await;
        let identity = hex(&sim.identity);
        know(&dir, &[&identity]);
        assert!(
            !matches!(time(&sim).await.ntp, NtpServer::Off),
            "a fresh feeder asks its router"
        );

        // The router may answer within moments: the first read only notes
        // that the clock wanted the time, and so does one soon after.
        catch_up(&dir, &sim, &identity, Attempt::First, home()).await;
        catch_up(&dir, &sim, &identity, Attempt::First, home()).await;
        let waiting = time(&sim).await;
        assert_eq!((waiting.clock_ms, waiting.syncs), (None, 0), "{waiting:?}");

        // Two minutes on, by the run alerts.json keeps, the router hasn't.
        backdate_invalid_run(&dir, 2 * 60_000);
        catch_up(&dir, &sim, &identity, Attempt::First, home()).await;
        let set = time(&sim).await;
        assert!(set.clock_ms.is_some(), "{set:?}");
        assert_eq!(set.syncs, 1);

        sim.stop().await;
    });
}

/// Everything on the feeder a write could change but the clock: each
/// store's view, the clients, the time settings and the feeds reserved.
async fn writable_state(sim: &Running, flash: &Rc<Flash>) -> (Vec<Answer>, String, u64) {
    let mut owner = Connection::open(
        &Keypair::from_secret(OWNER),
        sim.identity,
        sim.address,
        [0xa4; 32],
    )
    .await
    .unwrap();
    let mut views = Vec::new();
    for (id, op) in [
        (4, Op::GetSettings),
        (5, Op::GetPlans),
        (6, Op::GetCalibration),
        (7, Op::GetCall),
        (8, Op::ListClients),
    ] {
        views.push(owner.ask([id; 16], &op).await.unwrap());
    }
    let time = time(sim).await;
    let settings = format!("{:?} {:?} {:?}", time.generation, time.zone, time.ntp);
    (views, settings, feeding::reserved(flash).unwrap())
}

/// On every network a phone can be on, a catch-up writes nothing but the
/// clock sync, and that only when the clock rule allowed one for the time
/// view it read: with no time server an unset clock is synced at once, with
/// one it waits, and a set clock is left alone.
#[test]
fn a_catch_up_on_any_network_writes_nothing_but_a_sync_the_rule_allowed() {
    // A phone on its Wi-Fi or a computer on its LAN, a computer on a
    // network it can't name, which still dials, and a device off the
    // network, which doesn't.
    let networks = [home(), PhoneNetwork::Unnamed, PhoneNetwork::Elsewhere];
    for network in networks {
        for server in [NtpServer::Off, NtpServer::Gateway { port: 123 }] {
            bounded(async {
                let kept = tempfile::tempdir().unwrap();
                let dir = kept.path().display().to_string();
                let flash: Rc<Flash> = Flash::memory();
                let sim = Running::start(&flash, Some(*Keypair::from_secret(OWNER).public())).await;
                let identity = hex(&sim.identity);
                know(&dir, &[&identity]);
                ntp(&sim, server).await;

                for round in 0..3 {
                    let before = time(&sim).await;
                    let state = writable_state(&sim, &flash).await;
                    let allowed = wellfed_client::time::check(
                        &before,
                        wellfed_model::ClientRole::Owner,
                        jiff::Timestamp::now(),
                        None,
                    ) == wellfed_client::time::ClockCheck::Sync;

                    let caught =
                        catch_up(&dir, &sim, &identity, Attempt::First, network.clone()).await;
                    // Off Wi-Fi with no address learned, it never dials.
                    let reached = matches!(caught.end, CatchUpEnd::Read | CatchUpEnd::Unchanged);
                    assert!(
                        reached || caught.end == CatchUpEnd::Away,
                        "{network:?}: {caught:?}"
                    );

                    let after = time(&sim).await;
                    assert_eq!(
                        writable_state(&sim, &flash).await,
                        state,
                        "{network:?} {server:?} round {round}: nothing but the clock written"
                    );
                    let synced = after.syncs - before.syncs;
                    assert!(synced <= 1, "{network:?} {server:?} round {round}");
                    assert!(
                        synced == 0 || allowed,
                        "{network:?} {server:?} round {round}: a sync the rule refused"
                    );
                    let expected = reached && server == NtpServer::Off && round == 0;
                    assert_eq!(
                        synced == 1,
                        expected,
                        "{network:?} {server:?} round {round}"
                    );
                }
                sim.stop().await;
            });
        }
    }
}

#[test]
fn only_the_home_networks_last_try_counts_towards_unreachable() {
    bounded(async {
        let kept = tempfile::tempdir().unwrap();
        let dir = kept.path().display().to_string();
        let flash: Rc<Flash> = Flash::memory();
        let sim = Running::start(&flash, Some(*Keypair::from_secret(OWNER).public())).await;
        let identity = hex(&sim.identity);
        know(&dir, &[&identity]);
        let address = sim.address;

        let reached = catch_up(&dir, &sim, &identity, Attempt::First, home()).await;
        assert_eq!(reached.end, CatchUpEnd::Read);
        sim.stop().await;

        // Stopped, the address answers nothing.
        let key = ClientKey::restore(OWNER.to_vec()).unwrap();
        let try_at = |attempt: Attempt, network: PhoneNetwork| {
            let (dir, identity, key) = (dir.clone(), identity.clone(), &key);
            async move {
                catch_up_with(
                    dir,
                    key,
                    identity,
                    None,
                    Some(address.to_string()),
                    attempt,
                    network,
                )
                .await
                .unwrap()
            }
        };

        let first = try_at(Attempt::First, home()).await;
        assert_eq!(first.end, CatchUpEnd::Unanswered);
        let last = try_at(Attempt::Last { located: None }, home()).await;
        assert_eq!(last.end, CatchUpEnd::Failed);
        assert_eq!(last.changes, [], "one failure is no outage");

        let cafe = PhoneNetwork::Wifi {
            gateway: "10.0.0.1".into(),
            subnet: "10.0.0.0/24".into(),
        };
        for _ in 0..5 {
            let away = try_at(Attempt::Last { located: None }, cafe.clone()).await;
            assert_eq!(away.end, CatchUpEnd::Failed);
            assert_eq!(away.changes, [], "away, nothing counts");
        }
        let off_wifi = try_at(Attempt::First, PhoneNetwork::Elsewhere).await;
        assert_eq!(off_wifi.end, CatchUpEnd::Away);
    });
}

#[test]
fn a_computer_reads_the_table_as_a_phone_does_on_any_network_it_is_on() {
    bounded(async {
        let kept = tempfile::tempdir().unwrap();
        let dir = kept.path().display().to_string();
        let flash: Rc<Flash> = Flash::memory();
        let sim = Running::start(&flash, Some(*Keypair::from_secret(OWNER).public())).await;
        let identity = hex(&sim.identity);
        know(&dir, &[&identity]);
        let lan = || PhoneNetwork::Lan {
            network: "192.168.1.0/24".into(),
        };

        jam(&sim).await;
        let read = catch_up(&dir, &sim, &identity, Attempt::First, lan()).await;
        assert_eq!(read.end, CatchUpEnd::Read);
        assert!(
            read.changes
                .iter()
                .any(|change| matches!(change, NoticeChange::Post { notice } if notice.kind == NoticeKind::MealFailed)),
            "{:?}",
            read.changes
        );

        // Past a VPN with no LAN to name, it still dials, and the feeder is
        // there; nothing it can't reach would count.
        let unnamed = catch_up(&dir, &sim, &identity, Attempt::First, PhoneNetwork::Unnamed).await;
        assert_eq!(unnamed.end, CatchUpEnd::Unchanged);

        sim.stop().await;
    });
}
