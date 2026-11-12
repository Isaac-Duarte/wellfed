//! An install across a server restart: the one running when the server
//! stopped is failed and readable after it starts again, `interrupted` if
//! the feeder never took its offer and uncertain once it did; the slot is
//! free, and the next install takes up the transfer the feeder kept
//! receiving.

use std::time::Duration;

use wellfed_api::{
    ClientRole, Connection as Link, InstallFailure, InstallRequest, InstallState, MaybeKnown,
    Model, ReleaseQuery,
};
use wellfed_model::{Answer, Op, UpdatePhase};
use wellfed_server_testkit::feeder::{self, Banks, Relay, Trial};
use wellfed_server_testkit::release::{
    self, MANIFEST_LEN, SECTOR, UPDATE_CHUNK_MAX, project, signed,
};
use wellfed_server_testkit::{TestService, WAIT, ada, config, local, until};
use wellfed_service::{Error, InstallError, ReleaseAdded, Service};

const BUILD: [u8; 32] = [0x77; 32];
const LEN: usize = 16 * SECTOR;

async fn ended(service: &Service, id: wellfed_api::FeederId) -> wellfed_api::Install {
    let ended = tokio::time::timeout(4 * WAIT, async {
        loop {
            let install = service.install_state(id).await.unwrap();
            if !matches!(
                install.state,
                InstallState::Running | InstallState::Confirming
            ) {
                return install;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
    ended.await.expect("the install ends in time")
}

#[test]
fn a_restart_mid_transfer_ends_it_uncertain_and_the_next_activates() {
    local(async {
        let banks = Banks::new();
        let sim = feeder::start_updatable(&banks).await;
        let server = TestService::start_with(release::trusting(config())).await;
        let relay = Relay::start(sim.address).await;
        let address = relay.address.to_string();
        let id = server.adopt_as(&sim, &address, ClientRole::Owner).await;
        server.link(id, Link::Online).await;
        let file = signed(&project(), 7, BUILD, LEN);
        let (release, added) = server.service().upload_release(file).await.unwrap();
        assert_eq!(added, ReleaseAdded::New);
        let request = InstallRequest {
            release: release.id,
        };

        // Half the image reaches the feeder, then its answers are held: the
        // transfer stands part-way however fast the host is.
        relay.hold_answers_after(LEN / 2);
        let first = server
            .service()
            .install(&ada().into(), id, request)
            .await
            .unwrap();
        until("the transfer is held half-way", || async {
            relay.holding_answers()
        })
        .await;
        assert!(server.service().install_state(id).await.unwrap().written > 0);
        let store = server.stop().await;
        relay.resume();

        let server = TestService::open(store, release::trusting(config())).await;
        // Half of bank 1 is written over: the feeder no longer has the
        // build it had to keep running, so the end is not `interrupted`.
        let interrupted = server.service().install_state(id).await.unwrap();
        assert_eq!(interrupted.id, first.id);
        assert_eq!(interrupted.state, InstallState::Failed);
        let uncertain = InstallFailure::Unreachable { offered: true };
        assert_eq!(interrupted.failure, Some(uncertain));
        assert!(interrupted.finished.is_some());

        // The feeder keeps a quiet transfer for a minute: offered again, the
        // release is taken up from where bank 0 stands and activated.
        server.link(id, Link::Online).await;
        let second = server
            .service()
            .install(&ada().into(), id, request)
            .await
            .unwrap();
        assert_ne!(second.id, first.id);
        let ended = ended(server.service(), id).await;
        assert_eq!(
            (ended.id, ended.state),
            (second.id, InstallState::Confirmed)
        );
        until("the feeder restarts into the release", || async {
            banks.borrow().bank1_build == BUILD
        })
        .await;

        server.shutdown().await;
        sim.stop().await;
    });
}

/// A server stopped before its offer reached the feeder changed nothing
/// there: that install is `interrupted`, and the feeder keeps its build.
#[test]
fn a_restart_before_the_offer_took_is_interrupted() {
    local(async {
        let banks = Banks::new();
        let sim = feeder::start_updatable(&banks).await;
        let server = TestService::start_with(release::trusting(config())).await;
        let relay = Relay::start(sim.address).await;
        let address = relay.address.to_string();
        let id = server.adopt_as(&sim, &address, ClientRole::Owner).await;
        server.link(id, Link::Online).await;
        let had = banks.borrow().bank1_build;
        let file = signed(&project(), 7, BUILD, LEN);
        let (release, _) = server.service().upload_release(file).await.unwrap();
        let request = InstallRequest {
            release: release.id,
        };

        // The install reads the feeder's update state before it starts,
        // and the push reads it again before any offer. One read's size
        // lets the first through whole and holds the second's answer, so
        // the push stands before its offer however fast the host is.
        let before = relay.passed();
        server.service().stock(&ada().into(), id).await.unwrap();
        let read = relay.passed() - before;
        relay.hold_answers_after(read + 1);
        let first = server
            .service()
            .install(&ada().into(), id, request)
            .await
            .unwrap();
        until("the push's first read is held", || async {
            relay.holding_answers()
        })
        .await;
        let store = server.stop().await;
        relay.cut();
        relay.resume();

        let server = TestService::open(store, release::trusting(config())).await;
        let interrupted = server.service().install_state(id).await.unwrap();
        assert_eq!(interrupted.id, first.id);
        assert_eq!(interrupted.state, InstallState::Failed);
        assert_eq!(interrupted.failure, Some(InstallFailure::Interrupted));
        assert_eq!(banks.borrow().bank1_build, had);

        server.shutdown().await;
        sim.stop().await;
    });
}

/// A note that the feeder took the offer, which the store would not keep,
/// would leave a row a restart reads as `interrupted` while bank 1 may be
/// partly written: the install stops there and ends uncertain, and a
/// restart reads it so.
#[test]
fn an_offer_the_store_cannot_note_stops_the_install_uncertain() {
    local(async {
        let banks = Banks::new();
        let sim = feeder::start_updatable(&banks).await;
        let server = TestService::start_with(release::trusting(config())).await;
        let id = server
            .adopt_as(&sim, &sim.address.to_string(), ClientRole::Owner)
            .await;
        server.link(id, Link::Online).await;
        let had = banks.borrow().bank1_build;
        let file = signed(&project(), 7, BUILD, LEN);
        let (release, _) = server.service().upload_release(file).await.unwrap();
        let request = InstallRequest {
            release: release.id,
        };

        wellfed_store::testing::refuse_offer_notes(&server.store().path())
            .await
            .unwrap();
        let first = server
            .service()
            .install(&ada().into(), id, request)
            .await
            .unwrap();

        let uncertain = Some(InstallFailure::Unreachable { offered: true });
        let ended = ended(server.service(), id).await;
        assert_eq!(
            (ended.id, ended.state, ended.failure),
            (first.id, InstallState::Failed, uncertain)
        );
        assert_eq!(banks.borrow().bank1_build, had, "never activated");

        let store = server.stop().await;
        let server = TestService::open(store, release::trusting(config())).await;
        let after = server.service().install_state(id).await.unwrap();
        assert_eq!((after.id, after.failure), (first.id, uncertain));

        server.shutdown().await;
        sim.stop().await;
    });
}

/// A server stopped while it followed an activated release's trial never
/// saw the verdict: at the next start that install reads `activated`, ended
/// then, rather than still confirming or failed.
#[test]
fn a_restart_while_the_trial_is_followed_ends_it_activated() {
    local(async {
        let banks = Banks::new();
        // Undecided for some seconds: three trial boots, then the fallback.
        banks.borrow_mut().trial = Trial::NeverConfirms;
        let sim = feeder::start_updatable(&banks).await;
        let server = TestService::start_with(release::trusting(config())).await;
        let id = server
            .adopt_as(&sim, &sim.address.to_string(), ClientRole::Owner)
            .await;
        server.link(id, Link::Online).await;
        let file = signed(&project(), 7, BUILD, LEN);
        let (release, _) = server.service().upload_release(file).await.unwrap();
        let request = InstallRequest {
            release: release.id,
        };
        let started = server
            .service()
            .install(&ada().into(), id, request)
            .await
            .unwrap();
        until("the trial is followed", || async {
            server.service().install_state(id).await.unwrap().state == InstallState::Confirming
        })
        .await;

        let store = server.stop().await;
        let server = TestService::open(store, release::trusting(config())).await;

        let unseen = server.service().install_state(id).await.unwrap();
        assert_eq!(unseen.id, started.id);
        assert_eq!(
            (unseen.state, unseen.failure),
            (InstallState::Activated, None)
        );
        assert!(unseen.finished.is_some());
        server.shutdown().await;
        sim.stop().await;
    });
}

/// A feeder that never comes back after the activation gives no verdict:
/// once the trial time passes, the install ends `activated`, with no
/// failure, rather than confirmed or rolled back.
#[test]
fn a_feeder_that_never_comes_back_ends_the_install_activated() {
    local(async {
        let banks = Banks::new();
        let sim = feeder::start_updatable(&banks).await;
        let mut config = release::trusting(config());
        config.timing.trial = Duration::from_secs(3);
        let server = TestService::start_with(config).await;
        let id = server
            .adopt_as(&sim, &sim.address.to_string(), ClientRole::Owner)
            .await;
        server.link(id, Link::Online).await;
        let file = signed(&project(), 7, BUILD, LEN);
        let (release, _) = server.service().upload_release(file).await.unwrap();
        let request = InstallRequest {
            release: release.id,
        };
        server
            .service()
            .install(&ada().into(), id, request)
            .await
            .unwrap();
        until("the trial is followed", || async {
            server.service().install_state(id).await.unwrap().state == InstallState::Confirming
        })
        .await;

        sim.stop().await;
        until("the install ends", || async {
            server
                .service()
                .install_state(id)
                .await
                .unwrap()
                .finished
                .is_some()
        })
        .await;

        let unseen = server.service().install_state(id).await.unwrap();
        assert_eq!(
            (unseen.state, unseen.failure),
            (InstallState::Activated, None)
        );
        server.shutdown().await;
    });
}

/// An owner's own session with the feeder, once it answers: a restart
/// into bank 0 takes a moment.
async fn session(sim: &feeder::Running) -> feeder::Connection {
    let opened = tokio::time::timeout(WAIT, async {
        loop {
            let ephemeral = [0x5a; 32];
            match feeder::Connection::open(&feeder::owner(), sim.identity, sim.address, ephemeral)
                .await
            {
                Ok(session) => return session,
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    });
    opened.await.expect("the owner's session opens")
}

/// Offer `file` from the owner's own session, write a sector, then send
/// one at an offset the feeder never asked for: the install stops on bank
/// 0 with its candidate left pending, as a quiet minute or a server
/// restart leaves it.
async fn leave_a_stopped_candidate(sim: &feeder::Running, file: &[u8]) {
    let (manifest, payload) = file.split_at(MANIFEST_LEN);
    let offer = Op::OfferUpdate {
        manifest: manifest.to_vec(),
        downgrade: None,
    };
    let mut ids = 0u128;
    let mut next = || {
        ids += 1;
        ids.to_be_bytes()
    };

    let mut owner = session(sim).await;
    let mut view = match owner.ask(next(), &offer).await.unwrap() {
        Answer::Update(view) => view,
        other => panic!("the offer was answered {other:?}"),
    };
    if view.phase == UpdatePhase::GivingWay {
        // Bank 1 answers for a moment yet as it gives way.
        owner = tokio::time::timeout(WAIT, async {
            loop {
                tokio::time::sleep(Duration::from_millis(50)).await;
                let mut owner = session(sim).await;
                if let Ok(Answer::Update(view)) = owner.ask(next(), &Op::UpdateState).await
                    && view.bank == 0
                {
                    return owner;
                }
            }
        })
        .await
        .expect("the feeder restarts into bank 0");
        view = match owner.ask(next(), &offer).await.unwrap() {
            Answer::Update(view) => view,
            other => panic!("bank 0 answered the offer {other:?}"),
        };
    }
    assert_eq!((view.bank, view.phase), (0, UpdatePhase::Receiving));

    // A sector written back, so the stop has overwritten some of bank 1
    // and the server's install is not held as one over stock.
    while view.written == 0 {
        let answer = match view.wanted.first() {
            Some(&offset) => {
                let at = usize::try_from(offset).unwrap();
                let data = payload[at..at + UPDATE_CHUNK_MAX].to_vec();
                owner.ask(next(), &Op::UpdateChunk { offset, data }).await
            }
            None => owner.ask(next(), &Op::UpdateState).await,
        };
        view = match answer.unwrap() {
            Answer::Update(view) => view,
            other => panic!("a chunk was answered {other:?}"),
        };
    }
    // A short chunk where a whole one is wanted stops the install.
    let wanted = loop {
        if let Some(&offset) = view.wanted.first() {
            break offset;
        }
        view = match owner.ask(next(), &Op::UpdateState).await.unwrap() {
            Answer::Update(view) => view,
            other => panic!("the state was answered {other:?}"),
        };
    };
    let stray = Op::UpdateChunk {
        offset: wanted,
        data: vec![0; 16],
    };
    let _stopped = owner.ask(next(), &stray).await;
    let Answer::Update(view) = owner.ask(next(), &Op::UpdateState).await.unwrap() else {
        panic!("the state was not read");
    };
    assert_eq!(view.phase, UpdatePhase::Idle);
    assert!(view.stopped.is_some() && view.digest.is_some(), "{view:?}");
}

/// A stopped install's candidate refuses every other manifest until it is
/// cancelled. The server's next install of another release cancels it
/// first, as the owner's CLI does, and activates.
#[test]
fn an_install_over_a_stopped_candidate_cancels_it_and_activates() {
    local(async {
        let banks = Banks::new();
        let sim = feeder::start_updatable(&banks).await;
        let config = release::keeping_stock(release::trusting(config()), &banks);
        let server = TestService::start_with(config).await;
        let address = sim.address.to_string();
        let id = server.adopt_as(&sim, &address, ClientRole::Owner).await;
        server.link(id, Link::Online).await;

        // Stock gave way to the replacement on bank 0, and the server keeps
        // bank 1's stock: an install from bank 0 may go ahead.
        feeder::stock_gave_way(&sim, &banks);
        until("the feeder serves bank 1 from bank 0", || async {
            let stock = server.service().stock(&ada().into(), id).await.unwrap();
            stock.feeder_state.is_some_and(|feeder| feeder.readable)
        })
        .await;
        server.service().read_stock(id).await.unwrap();

        let stale = signed(&project(), 6, [0x66; 32], LEN);
        leave_a_stopped_candidate(&sim, &stale).await;

        let file = signed(&project(), 7, BUILD, LEN);
        let (release, _) = server.service().upload_release(file).await.unwrap();
        let request = InstallRequest {
            release: release.id,
        };
        server
            .service()
            .install(&ada().into(), id, request)
            .await
            .unwrap();
        let ended = ended(server.service(), id).await;
        assert_eq!(
            (ended.state, ended.failure),
            (InstallState::Confirmed, None)
        );
        until("the feeder restarts into the release", || async {
            banks.borrow().bank1_build == BUILD
        })
        .await;

        server.shutdown().await;
        sim.stop().await;
    });
}

/// An install whose caller is dropped while its row is kept runs on to
/// its end: the slot is never freed with the row left `running` and nothing
/// to end it.
#[test]
fn an_install_whose_caller_is_dropped_as_it_starts_still_ends() {
    local(async {
        let banks = Banks::new();
        let sim = feeder::start_updatable(&banks).await;
        let server = TestService::start_with(release::trusting(config())).await;
        let id = server
            .adopt_as(&sim, &sim.address.to_string(), ClientRole::Owner)
            .await;
        server.link(id, Link::Online).await;
        let file = signed(&project(), 7, BUILD, LEN);
        let (release, _) = server.service().upload_release(file).await.unwrap();
        let service = server.service();

        // Polled until the install holds the slot, then dropped: the row is
        // being kept at that moment.
        let caller = ada().into();
        let mut installing = Box::pin(service.install(
            &caller,
            id,
            InstallRequest {
                release: release.id,
            },
        ));
        loop {
            let polled = futures_util::poll!(installing.as_mut());
            assert!(
                polled.is_pending(),
                "the install started before it was seen"
            );
            let claimed = service.install_state(id).await;
            if claimed.is_ok_and(|install| install.state == InstallState::Running) {
                break;
            }
            tokio::task::yield_now().await;
        }
        drop(installing);

        let ended = ended(service, id).await;
        assert_eq!(ended.state, InstallState::Confirmed);
        server.shutdown().await;
        sim.stop().await;
    });
}

/// A release built for another model is listed under its model and never
/// offered to this device: the install is refused before anything is sent.
#[test]
fn a_release_for_another_model_is_refused_and_listed_by_its_model() {
    local(async {
        let banks = Banks::new();
        let sim = feeder::start_updatable(&banks).await;
        let server = TestService::start_with(release::trusting(config())).await;
        let id = server
            .adopt_as(&sim, &sim.address.to_string(), ClientRole::Owner)
            .await;
        server.link(id, Link::Online).await;
        let file = release::signed_for(Model::Plwf106, &project(), 7, BUILD, LEN);
        let (fountain, _) = server.service().upload_release(file).await.unwrap();
        assert_eq!(fountain.model, Model::Plwf106.into());

        let refused = server
            .service()
            .install(
                &ada().into(),
                id,
                InstallRequest {
                    release: fountain.id,
                },
            )
            .await
            .unwrap_err();
        assert!(
            matches!(
                refused,
                Error::Install(InstallError::OtherModel {
                    release: MaybeKnown::Known(Model::Plwf106),
                    device: Model::Plaf103,
                })
            ),
            "{refused:?}"
        );

        let listed = |model| ReleaseQuery {
            after: None,
            limit: None,
            model: Some(model),
        };
        let service = server.service();
        assert!(
            service
                .releases(&listed(Model::Plaf103))
                .await
                .unwrap()
                .items
                .is_empty()
        );
        let fountains = service.releases(&listed(Model::Plwf106)).await.unwrap();
        assert_eq!(fountains.items, vec![fountain]);
    });
}
