//! A feeder whose welcome carries a pre-revision schema hash: the bench
//! unit's bank 0, say, or any feeder updated before API revisions. Its build
//! reads every op outside the frozen part as something else, so the task
//! neither subscribes nor reads its status or sets its clock: it names the
//! feeder `legacy`, refuses every command but the frozen update path before
//! sending it, and keeps the session for an install.

mod support;

use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use wellfed_api::{Connection as Link, Model};
use wellfed_feeder_sim::harness::pre_revision::{PreRevision, Seen};
use wellfed_mirror::{AskError, Config, Diagnostics, Discovery, Feeder, Mirror, MirrorEvent};
use wellfed_model::{Answer, Op, Skew};
use wellfed_server_testkit::fixed::{FixedClock, SeededRandom};
use wellfed_server_testkit::{TempStore, WAIT, local};
use wellfed_store::{NewFeeder, SealedKey};

use support::{HOME, ID, SECRET, next, timing};

#[test]
fn a_pre_revision_feeder_is_sent_only_the_update_path() {
    local(async {
        let feeder = PreRevision::start();
        let address = feeder.address.to_string();

        let temp = TempStore::new();
        let store = temp.open().await;
        support::home(&store).await;
        let identity = feeder.identity;
        store
            .insert_feeder(NewFeeder {
                id: ID,
                home: HOME,
                model: Model::Plaf103,
                identity,
                name: "kitchen",
                role: wellfed_api::ClientRole::Owner,
                address: Some(&address),
                key: SealedKey {
                    nonce: [0; 12],
                    sealed: Vec::new(),
                },
                created: FixedClock.now(),
            })
            .await
            .unwrap();
        let config = Config {
            discovery: Discovery::Off,
            timing: timing(WAIT),
            diagnostics: Diagnostics::Read {
                stats_every: std::time::Duration::from_millis(50),
            },
        };
        let mirror = Mirror::new(
            store.clone(),
            config,
            Arc::new(FixedClock),
            Arc::new(SeededRandom::fresh()),
            CancellationToken::new(),
        );
        let mut updates = mirror
            .start(Feeder {
                id: ID,
                identity,
                secret: SECRET,
                address: Some(address),
            })
            .await;

        let mut links = Vec::new();
        while links.last() != Some(&Link::Online) {
            if let MirrorEvent::Link(link) = next(&mut updates).await {
                links.push(link);
            }
        }
        assert_eq!(links, [Link::Connecting, Link::Online]);
        assert!(mirror.session_open(ID));
        assert_eq!(mirror.skew(ID), Some(Skew::Legacy));

        for op in [Op::Status, Op::Feed { portions: 1 }, Op::ListClients] {
            let asked = mirror.ask(ID, [1; 16], op.clone()).await;
            assert_eq!(asked, Err(AskError::Legacy), "{op:?}");
        }
        let state = mirror.ask(ID, [2; 16], Op::UpdateState).await;
        assert!(matches!(state, Ok(Answer::Update(view)) if view.bank == 0));

        // An install's turn still runs its frozen steps.
        let mut operation = mirror.operation(ID).await.unwrap();
        let step = operation.step([3; 16], Op::UpdateState).await;
        assert!(matches!(step, Ok(Answer::Update(_))));

        assert_eq!(mirror.link(ID), Some(Link::Online));
        assert_eq!(
            feeder.seen(),
            [Seen::Update, Seen::Update],
            "no subscription, status, clock or statistics went to it"
        );
    });
}
