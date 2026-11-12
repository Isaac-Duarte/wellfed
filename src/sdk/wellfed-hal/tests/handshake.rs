//! The park handshake every KM4 caller shares (arming and the probes): it
//! binds an authority only with STOP and infrared OFF verified, waits for
//! KM0 at most `READY_TIMEOUT_MS`, and starts nothing while the runtime
//! holds writes for a reset or KM0 refuses the hello.

use wellfed_core::flash_coord::FlashReply;
use wellfed_hal::flash::park;
use wellfed_hal::flash::park::fake::{self as ipc, Ipc, Refusing};
use wellfed_hal::flash_lock::handshake::{Handshake, Handshaking, Unstarted};
use wellfed_hal::flash_lock::provider::READY_TIMEOUT_MS;
use wellfed_hal::sdk::fake;
use wellfed_hal::{stopped, supervision};

const PEER: u64 = 0x0000_0001_4b30_abcd;

fn fresh() {
    fake::reset_hardware();
    ipc::reset();
    supervision::reset();
}

/// KM0 answers the hello, then KM4 polls `now_ms` in.
fn answered(handshake: Handshake, now_ms: u32) -> Handshaking {
    let mut answered = 0;
    park::answer::<Ipc<true>>(PEER, &mut answered);
    handshake.step::<Ipc<false>>(now_ms)
}

#[test]
fn an_authority_is_bound_only_with_the_outputs_verified_stopped() {
    fresh();
    let handshake = Handshake::start::<Ipc<false>>(0).expect("started");
    let Handshaking::Ready {
        inactive,
        bound: Some((_, reply)),
    } = answered(handshake, 1)
    else {
        panic!("KM0 answered");
    };
    assert!(!inactive, "nothing verified the outputs");
    assert_ne!(reply, FlashReply::Accepted);

    fresh();
    assert!(stopped::initialize().verified());
    let handshake = Handshake::start::<Ipc<false>>(0).expect("started");
    let Handshaking::Ready {
        inactive: true,
        bound: Some((_, FlashReply::Accepted)),
    } = answered(handshake, 1)
    else {
        panic!("bound with the outputs verified stopped");
    };
}

#[test]
fn an_unanswered_handshake_waits_then_times_out() {
    fresh();
    let handshake = Handshake::start::<Ipc<false>>(0).expect("started");

    assert!(matches!(
        handshake.step::<Ipc<false>>(READY_TIMEOUT_MS - 1),
        Handshaking::Waiting
    ));
    assert!(matches!(
        handshake.step::<Ipc<false>>(READY_TIMEOUT_MS),
        Handshaking::TimedOut
    ));
}

#[test]
fn nothing_starts_while_writes_are_held_or_km0_refuses() {
    fresh();
    supervision::supervised(false, 0);
    assert!(supervision::writes_held());
    assert_eq!(
        Handshake::start::<Ipc<false>>(0),
        Err(Unstarted::Held),
        "no binding into a decided reset"
    );

    fresh();
    assert_eq!(Handshake::start::<Refusing>(0), Err(Unstarted::Refused));
}
