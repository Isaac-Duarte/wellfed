//! Alerts at home against the simulated feeder (simulation, not hardware):
//! - a catch-up reads the feeder's table and posts a failed meal once; the
//!   next finds the table where it was and posts nothing, and a notice the
//!   app never confirmed is handed over again;
//! - a dismissal hides it on this phone and takes its notification off,
//!   and the feeder still holds it;
//! - a stopped feeder is unanswered at the first try and failed at the
//!   last, and only on its home network does that count; elsewhere the
//!   phone is away and nothing counts;
//! - nothing a catch-up sends is a write: the feeder's journal holds only
//!   the owner's own feed.

use std::rc::Rc;
use std::time::Duration;

use wellfed_app_bridge::api::alerts::{
    Attempt, CatchUp, CatchUpEnd, NoticeChange, NoticeDetail, PhoneNetwork, PostedNotice,
    catch_up_with, confirm_notices, dismiss_notice, feeder_alerts, forget_alerts,
};
use wellfed_app_bridge::api::keys::ClientKey;
use wellfed_app_bridge::api::model::{FeedOutcome, FeedReason, NoticeKind};
use wellfed_app_bridge::api::store::{AppStore, KnownFeeder, save_store};
use wellfed_connect::Connection;
use wellfed_feeder_sim::feeding;
use wellfed_feeder_sim::flash::Flash;
use wellfed_feeder_sim::harness::{Running, local};
use wellfed_feeder_sim::serve::{Control, FeedFault};
use wellfed_model::{Answer, EventBody, FeederEvent, Op, Pushed};
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
        dns: vec!["192.168.1.1".into()],
        domains: None,
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
    save_store(&format!("{dir}/wellfed.json"), &store).unwrap();
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
        None,
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
    owner.subscribe(None, false).await.unwrap();
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

        confirm_notices(
            dir.clone(),
            identity.clone(),
            vec![PostedNotice::from(notice)],
            vec![],
        )
        .await
        .unwrap();
        let quiet = catch_up(&dir, &sim, &identity, Attempt::First, home()).await;
        assert_eq!(quiet.end, CatchUpEnd::Unchanged);
        assert_eq!(quiet.changes, [], "posted once");

        let dismissed = dismiss_notice(dir.clone(), identity.clone(), notice.id)
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

        assert_eq!(feeding::reserved(&flash), Some(1), "the owner's feed alone");
        sim.stop().await;
    });
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
                    None,
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
            dns: vec![],
            domains: None,
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
