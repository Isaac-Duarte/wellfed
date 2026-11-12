//! The meal call through the service: a clip in the feeder's own stream
//! uploaded a chunk at a time on the long lane, read back, played and
//! erased, each write through the journal. An upload a lost answer cuts
//! short is unknown, and its retry under the same key sends only what the
//! feeder lacks.

use std::time::Duration;

use wellfed_api::{
    CallClip, ClientRole, CommandOutcome, CommandState, Connection as Link, FeederRefusal,
};
use wellfed_model::{Answer, CALL_CHUNK_MAX, Op, Refusal};
use wellfed_server_testkit::feeder::{self, Relay};
use wellfed_server_testkit::{TestService, ada, config, local, until};
use wellfed_service::{CommandError, Config, Error, IdempotencyKey, Invalid};

/// A stream of `bytes` the feeder takes as it is: any bytes decode as its
/// ADPCM.
fn stream(bytes: usize) -> Vec<u8> {
    (0..bytes).map(|n| u8::try_from(n % 251).unwrap()).collect()
}

/// Callers stop waiting for an answer after 2 s, so a lost answer is
/// unknown without waiting out the default.
fn quick() -> Config {
    let mut config = config();
    config.timing.answer = Duration::from_secs(2);
    config
}

#[test]
fn a_clip_is_uploaded_played_and_erased() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        let id = server
            .adopt_as(&sim, &sim.address.to_string(), ClientRole::Owner)
            .await;
        server.link(id, Link::Online).await;
        let service = server.service();
        let caller = ada().into();

        assert_eq!(service.call(id).await.unwrap().clip, CallClip::None);
        let none = service.play_call(&caller, id, Some([1; 16])).await;
        assert!(
            matches!(none, Err(Error::Refused(FeederRefusal::NotFound))),
            "{none:?}"
        );

        let key: IdempotencyKey = [2; 16];
        let clip = stream(3_000);
        let stored = service
            .upload_call(&caller, id, clip.clone(), Some(key))
            .await
            .unwrap();
        let CallClip::Stored { bytes: 3_000, crc } = stored.clip else {
            panic!("{stored:?}");
        };
        assert_eq!(service.call(id).await.unwrap(), stored);
        let entry = service.command(&ada(), key).await.unwrap();
        assert_eq!(entry.state, CommandState::Answered);
        assert_eq!(entry.outcome, Some(CommandOutcome::Call { call: stored }));

        // The same clip under its key is answered from the journal; another
        // clip under it is a reused key, and nothing is sent.
        let again = service.upload_call(&caller, id, clip, Some(key)).await;
        assert_eq!(again.unwrap(), stored);
        let other = service
            .upload_call(&caller, id, stream(2_999), Some(key))
            .await;
        assert!(
            matches!(other, Err(Error::Command(CommandError::KeyReused))),
            "{other:?}"
        );
        assert!(matches!(
            service.call(id).await.unwrap().clip,
            CallClip::Stored { bytes: 3_000, crc: same } if same == crc
        ));

        // The simulator has no speaker: its call plays muted.
        let played = service.play_call(&caller, id, Some([3; 16])).await;
        assert!(
            matches!(played, Err(Error::Refused(FeederRefusal::Muted))),
            "{played:?}"
        );

        service
            .delete_call(&caller, id, Some([4; 16]))
            .await
            .unwrap();
        assert_eq!(service.call(id).await.unwrap().clip, CallClip::None);
        server.shutdown().await;
        sim.stop().await;
    });
}

/// What the feeder would refuse as a clip is refused before anything is
/// sent, and so is an upload to a server without the owner role's key,
/// by the feeder.
#[test]
fn a_clip_out_of_the_feeders_length_is_refused() {
    local(async {
        let sim = feeder::start().await;
        let server = TestService::start().await;
        let id = server.adopt(&sim).await;
        server.link(id, Link::Online).await;
        let service = server.service();
        let caller = ada().into();

        let most = usize::try_from(wellfed_api::CALL_BYTES_MAX).unwrap();
        for wrong in [Vec::new(), stream(most + 1)] {
            let refused = service.upload_call(&caller, id, wrong, None).await;
            assert!(
                matches!(refused, Err(Error::Invalid(Invalid::CallLength))),
                "{refused:?}"
            );
        }
        let refused = service.upload_call(&caller, id, stream(600), None).await;
        assert!(
            matches!(refused, Err(Error::Refused(FeederRefusal::Forbidden))),
            "{refused:?}"
        );
        assert_eq!(service.call(id).await.unwrap().clip, CallClip::None);
        server.shutdown().await;
        sim.stop().await;
    });
}

/// The feeder's answers stop part-way through an upload: it is unknown.
/// Once the session is back, a retry under the same key resumes where the
/// feeder's upload stands. The relay holds every answer again once the
/// retry has sent as many bytes as the whole clip, so a retry that started
/// over would stall there; one that resumes ends inside that budget.
#[test]
fn a_retry_after_a_lost_answer_sends_only_the_rest() {
    local(async {
        let sim = feeder::start().await;
        let relay = Relay::start(sim.address).await;
        let server = TestService::start_with(quick()).await;
        let id = server
            .adopt_as(&sim, &relay.address.to_string(), ClientRole::Owner)
            .await;
        server.link(id, Link::Online).await;
        let service = server.service();
        let caller = ada().into();

        let key: IdempotencyKey = [5; 16];
        let clip = stream(4_096);
        relay.hold_answers_after(1_800);
        let lost = service
            .upload_call(&caller, id, clip.clone(), Some(key))
            .await;
        assert!(
            matches!(lost, Err(Error::Command(CommandError::Unknown))),
            "{lost:?}"
        );
        let entry = service.command(&ada(), key).await.unwrap();
        assert_eq!(entry.state, CommandState::Unknown);

        relay.cut();
        relay.resume();
        server.link(id, Link::Online).await;
        let received = until_receiving(service, id).await;
        assert!((1..4_096).contains(&received), "{received}");

        relay.hold_answers_after(4_096);
        let stored = service
            .upload_call(&caller, id, clip.clone(), Some(key))
            .await
            .unwrap();
        assert!(matches!(stored.clip, CallClip::Stored { bytes: 4_096, .. }));
        assert!(!relay.holding_answers(), "the retry fit in the budget");
        let entry = service.command(&ada(), key).await.unwrap();
        assert_eq!(entry.outcome, Some(CommandOutcome::Call { call: stored }));

        // Settled, a retry is answered from the journal with nothing sent,
        // even with every answer held back.
        relay.hold_answers();
        let again = service.upload_call(&caller, id, clip, Some(key)).await;
        assert_eq!(again.unwrap(), stored);
        server.shutdown().await;
        sim.stop().await;
    });
}

/// An upload's answers are lost after some chunks, then its retry resumes
/// and the feeder refuses a chunk (its store failed). The first send's
/// start erased the clip the feeder held, and the feeder has lost part of
/// this one, so the refusal is no answer: the retry is unknown, and the
/// entry stays unknown, never sent again.
#[test]
fn a_retry_refused_past_the_start_stays_unknown() {
    local(async {
        let flash = feeder::Flash::memory();
        let sim = feeder::start_on(&flash).await;
        let relay = Relay::start(sim.address).await;
        let server = TestService::start_with(quick()).await;
        let id = server
            .adopt_as(&sim, &relay.address.to_string(), ClientRole::Owner)
            .await;
        server.link(id, Link::Online).await;
        let service = server.service();
        let caller = ada().into();

        let key: IdempotencyKey = [6; 16];
        let clip = stream(4_096);
        relay.hold_answers_after(1_800);
        let lost = service
            .upload_call(&caller, id, clip.clone(), Some(key))
            .await;
        assert!(
            matches!(lost, Err(Error::Command(CommandError::Unknown))),
            "{lost:?}"
        );
        relay.cut();
        relay.resume();
        server.link(id, Link::Online).await;
        until_receiving(service, id).await;

        feeder::fail_call(&flash, true);
        let refused = service
            .upload_call(&caller, id, clip.clone(), Some(key))
            .await;
        assert!(
            matches!(refused, Err(Error::Command(CommandError::Unknown))),
            "{refused:?}"
        );
        let entry = service.command(&ada(), key).await.unwrap();
        assert_eq!((entry.state, entry.outcome), (CommandState::Unknown, None));

        // Never sent again: a resend would find the clip gone with the
        // store's failure and settle as superseded.
        let again = service.upload_call(&caller, id, clip, Some(key)).await;
        assert!(
            matches!(again, Err(Error::Command(CommandError::Unknown))),
            "{again:?}"
        );
        server.shutdown().await;
        sim.stop().await;
    });
}

/// The bytes the feeder's upload has, once its session answers again.
async fn until_receiving(service: &wellfed_service::Service, id: wellfed_api::FeederId) -> u32 {
    let received = std::cell::Cell::new(0);
    until("the feeder reports the upload it holds", || async {
        let Ok(view) = service.call(id).await else {
            return false;
        };
        match view.clip {
            CallClip::Receiving { received: at, .. } => {
                received.set(at);
                true
            }
            CallClip::None | CallClip::Stored { .. } => false,
        }
    })
    .await;
    received.get()
}

/// An upload whose answers were lost, then another clip uploaded under
/// another key: the first one's retry would undo that, so it is settled
/// superseded and sends nothing, and the feeder keeps the later clip.
#[test]
fn a_retry_never_overwrites_a_later_clip() {
    local(async {
        let sim = feeder::start().await;
        let relay = Relay::start(sim.address).await;
        let server = TestService::start_with(quick()).await;
        let id = server
            .adopt_as(&sim, &relay.address.to_string(), ClientRole::Owner)
            .await;
        server.link(id, Link::Online).await;
        let service = server.service();
        let caller = ada().into();

        let key: IdempotencyKey = [6; 16];
        relay.hold_answers_after(1_800);
        let lost = service
            .upload_call(&caller, id, stream(4_096), Some(key))
            .await;
        assert!(
            matches!(lost, Err(Error::Command(CommandError::Unknown))),
            "{lost:?}"
        );
        relay.cut();
        relay.resume();
        back(service, id).await;

        let later = service
            .upload_call(&caller, id, stream(2_000), Some([7; 16]))
            .await
            .unwrap();
        for _ in 0..2 {
            let retried = service
                .upload_call(&caller, id, stream(4_096), Some(key))
                .await;
            assert!(
                matches!(retried, Err(Error::Command(CommandError::Superseded))),
                "{retried:?}"
            );
        }
        let entry = service.command(&ada(), key).await.unwrap();
        assert_eq!(entry.outcome, Some(CommandOutcome::Superseded));
        assert_eq!(service.call(id).await.unwrap(), later);
        server.shutdown().await;
        sim.stop().await;
    });
}

/// A delete whose answer was lost: its retry is done once the feeder holds
/// no clip, and superseded, with nothing sent, once another clip was
/// uploaded since.
#[test]
fn a_retried_delete_erases_only_the_clip_it_found() {
    local(async {
        let sim = feeder::start().await;
        let relay = Relay::start(sim.address).await;
        let server = TestService::start_with(quick()).await;
        let id = server
            .adopt_as(&sim, &relay.address.to_string(), ClientRole::Owner)
            .await;
        server.link(id, Link::Online).await;
        let service = server.service();
        let caller = ada().into();

        service
            .upload_call(&caller, id, stream(1_000), Some([0x10; 16]))
            .await
            .unwrap();
        let key: IdempotencyKey = [0x20; 16];
        lose_the_delete(service, &relay, id, key).await;
        assert_eq!(service.call(id).await.unwrap().clip, CallClip::None);
        service.delete_call(&caller, id, Some(key)).await.unwrap();

        // Lost again, then a clip uploaded before the retry.
        let key: IdempotencyKey = [0x30; 16];
        service
            .upload_call(&caller, id, stream(1_000), Some([0x31; 16]))
            .await
            .unwrap();
        lose_the_delete(service, &relay, id, key).await;
        let later = service
            .upload_call(&caller, id, stream(1_500), Some([0x32; 16]))
            .await
            .unwrap();
        let retried = service.delete_call(&caller, id, Some(key)).await;
        assert!(
            matches!(retried, Err(Error::Command(CommandError::Superseded))),
            "{retried:?}"
        );
        assert_eq!(service.call(id).await.unwrap(), later);
        server.shutdown().await;
        sim.stop().await;
    });
}

/// Send a delete under `key` whose answer the relay holds back: its read
/// of the clip passes, the erase reaches the feeder, and its answer is
/// lost. The session is then cut and comes back.
async fn lose_the_delete(
    service: &wellfed_service::Service,
    relay: &Relay,
    id: wellfed_api::FeederId,
    key: IdempotencyKey,
) {
    let before = relay.passed();
    service.call(id).await.unwrap();
    let read = relay.passed() - before;
    relay.hold_answers_after(read + 1);

    let lost = service.delete_call(&ada().into(), id, Some(key)).await;
    assert!(
        matches!(lost, Err(Error::Command(CommandError::Unknown))),
        "{lost:?}"
    );
    relay.cut();
    relay.resume();
    back(service, id).await;
}

/// Until the feeder's session answers again after a cut.
async fn back(service: &wellfed_service::Service, id: wellfed_api::FeederId) {
    until("the session answers again", || async {
        service.call(id).await.is_ok()
    })
    .await;
}

/// The feeder held no clip; an upload's answers were lost while the
/// feeder went on to store it (the rest of its chunks arrive over the
/// owner's own session here); then another client erased it. The retry
/// finds no clip, as the first send did, but the feeder took the upload's
/// start since, so that erase came after it: the retry is superseded and
/// does not bring the clip back.
#[test]
fn a_retry_never_restores_a_clip_erased_after_it() {
    local(async {
        let sim = feeder::start().await;
        let relay = Relay::start(sim.address).await;
        let server = TestService::start_with(quick()).await;
        let id = server
            .adopt_as(&sim, &relay.address.to_string(), ClientRole::Owner)
            .await;
        server.link(id, Link::Online).await;
        let service = server.service();
        let caller = ada().into();

        let key: IdempotencyKey = [0x40; 16];
        let clip = stream(4_096);
        relay.hold_answers_after(1_800);
        let lost = service
            .upload_call(&caller, id, clip.clone(), Some(key))
            .await;
        assert!(
            matches!(lost, Err(Error::Command(CommandError::Unknown))),
            "{lost:?}"
        );
        relay.cut();
        relay.resume();
        back(service, id).await;

        let stored = finish_directly(&sim, &clip).await;
        assert_eq!(service.call(id).await.unwrap(), stored);
        service
            .delete_call(&caller, id, Some([0x41; 16]))
            .await
            .unwrap();

        let retried = service.upload_call(&caller, id, clip, Some(key)).await;
        assert!(
            matches!(retried, Err(Error::Command(CommandError::Superseded))),
            "{retried:?}"
        );
        assert_eq!(service.call(id).await.unwrap().clip, CallClip::None);
        server.shutdown().await;
        sim.stop().await;
    });
}

/// Send the rest of `clip`'s upload over the owner's own session, from
/// where the feeder says it stands, and wait until the feeder stores it.
async fn finish_directly(sim: &feeder::Running, clip: &[u8]) -> wellfed_api::CallView {
    let mut owner = feeder::open_owner(sim).await;
    let mut ids = 0x50u8..;
    let mut ask = async |op: &Op| {
        let id = [ids.next().unwrap(); 16];
        owner.ask(id, op).await.unwrap()
    };

    let Answer::Call(view) = ask(&Op::GetCall).await else {
        panic!("no meal call");
    };
    let CallClip::Receiving { mut received, .. } = view.clip else {
        panic!("{view:?}");
    };
    while let Some(data) = clip
        .get(usize::try_from(received).unwrap()..)
        .filter(|rest| !rest.is_empty())
    {
        let data = data[..data.len().min(CALL_CHUNK_MAX)].to_vec();
        let len = u32::try_from(data.len()).unwrap();
        let op = Op::CallChunk {
            offset: received,
            data,
        };
        match ask(&op).await {
            Answer::Call(_) => received += len,
            Answer::Refused(Refusal::Busy) => tokio::time::sleep(Duration::from_millis(20)).await,
            other => panic!("{other:?}"),
        }
    }
    loop {
        match ask(&Op::GetCall).await {
            Answer::Call(view) if matches!(view.clip, CallClip::Stored { .. }) => return view,
            Answer::Call(_) | Answer::Refused(Refusal::Busy) => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            other => panic!("{other:?}"),
        }
    }
}
