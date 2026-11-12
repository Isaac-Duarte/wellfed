//! An install across a server restart: the one running when the server
//! stopped is failed (`interrupted`) and readable after it starts again,
//! the slot is free, and the next install takes up the transfer the feeder
//! kept receiving.

use std::time::Duration;

use wellfed_api::{
    ClientRole, Connection as Link, InstallFailure, InstallRequest, InstallState, MaybeKnown,
    Model, ReleaseQuery,
};
use wellfed_server_testkit::feeder::{self, Banks, Relay};
use wellfed_server_testkit::release::{self, SECTOR, project, signed};
use wellfed_server_testkit::{TestService, WAIT, ada, config, local, until};
use wellfed_service::{Error, InstallError, ReleaseAdded, Service};

const BUILD: [u8; 32] = [0x77; 32];
const LEN: usize = 16 * SECTOR;

async fn ended(service: &Service, id: wellfed_api::FeederId) -> wellfed_api::Install {
    let ended = tokio::time::timeout(4 * WAIT, async {
        loop {
            let install = service.install_state(id).await.unwrap();
            if install.state != InstallState::Running {
                return install;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
    ended.await.expect("the install ends in time")
}

#[test]
fn a_restart_mid_install_fails_it_frees_the_slot_and_the_next_activates() {
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
        let interrupted = server.service().install_state(id).await.unwrap();
        assert_eq!(interrupted.id, first.id);
        assert_eq!(interrupted.state, InstallState::Failed);
        assert_eq!(interrupted.failure, Some(InstallFailure::Interrupted));
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
            (second.id, InstallState::Activated)
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
        assert_eq!(ended.state, InstallState::Activated);
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
