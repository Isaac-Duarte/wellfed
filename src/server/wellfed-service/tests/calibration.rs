//! The portion calibration through the service: replaced against its
//! generation, a retry under the same key answered once, a stale base
//! answered with the current calibration, and the feeder's own refusals.

use wellfed_api::{
    Bowl, Calibration, ClientRole, Connection as Link, FeederRefusal, Generation, PORTION_MG_MAX,
};
use wellfed_server_testkit::feeder;
use wellfed_server_testkit::{TestService, ada, local};
use wellfed_service::{Error, Invalid};

const WEIGHED: Calibration = Calibration {
    portion_mg: Some(9_800),
    bowl: Bowl::Single,
};

#[test]
fn the_calibration_changes_against_the_current_generation_only() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        let id = server
            .adopt_as(&sim, &sim.address.to_string(), ClientRole::Owner)
            .await;
        server.link(id, Link::Online).await;
        let service = server.service();
        let caller = ada().into();

        let read = service.calibration(id).await.unwrap();
        assert_eq!(read.generation, Generation(0));
        assert_eq!(read.calibration.portion_mg, None, "never measured");

        let changed = service
            .put_calibration(&caller, id, read.generation, WEIGHED, Some([1; 16]))
            .await
            .unwrap();
        assert_eq!(changed.calibration, WEIGHED);
        assert_eq!(changed.generation, Generation(1));

        // A retry under the same key is answered from the journal.
        let retried = service
            .put_calibration(&caller, id, read.generation, WEIGHED, Some([1; 16]))
            .await
            .unwrap();
        assert_eq!(retried, changed);
        assert_eq!(service.calibration(id).await.unwrap(), changed);

        // A stale base gets the current calibration.
        let cleared = Calibration {
            portion_mg: None,
            ..WEIGHED
        };
        let stale = service
            .put_calibration(&caller, id, read.generation, cleared, Some([2; 16]))
            .await;
        let Err(Error::StaleCalibration(current)) = stale else {
            panic!("{stale:?}");
        };
        assert_eq!(current, changed);

        // A weight out of range is refused before anything is sent; a bowl
        // mode the feeder has not verified, by the feeder.
        let heavy = Calibration {
            portion_mg: Some(PORTION_MG_MAX + 1),
            ..WEIGHED
        };
        let refused = service
            .put_calibration(&caller, id, changed.generation, heavy, Some([3; 16]))
            .await;
        assert!(
            matches!(refused, Err(Error::Invalid(Invalid::PortionWeight))),
            "{refused:?}"
        );
        let double = Calibration {
            bowl: Bowl::Double,
            ..WEIGHED
        };
        let refused = service
            .put_calibration(&caller, id, changed.generation, double, Some([4; 16]))
            .await;
        assert!(
            matches!(refused, Err(Error::Refused(FeederRefusal::Unverified))),
            "{refused:?}"
        );
        assert_eq!(service.calibration(id).await.unwrap(), changed);
        server.shutdown().await;
        sim.stop().await;
    });
}

/// The feeder stores a calibration from an owner's key only: a server
/// adopted with the `server` role reads it and is refused the change.
#[test]
fn a_server_without_the_owner_role_reads_but_cannot_calibrate() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        let id = server.adopt(&sim).await;
        server.link(id, Link::Online).await;
        let service = server.service();

        let read = service.calibration(id).await.unwrap();
        let refused = service
            .put_calibration(&ada().into(), id, read.generation, WEIGHED, None)
            .await;
        assert!(
            matches!(refused, Err(Error::Refused(FeederRefusal::Forbidden))),
            "{refused:?}"
        );
        assert_eq!(service.calibration(id).await.unwrap(), read);
        server.shutdown().await;
        sim.stop().await;
    });
}
