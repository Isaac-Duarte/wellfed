//! The device protocol's codec: any bytes as every message either side
//! decodes, and whatever decodes re-encodes to bytes that read back to
//! themselves. Not to the same value: what a reader skips (a newer build's
//! extension tail or alert entry) is dropped on the way, so the first
//! re-encoding is the fixed point.
#![no_main]

use wellfed_proto::api::{
    ClientMessage, DeviceMessage, EventBody, FRAME_MAX, Hello, Join, Joined, Open, RawEvent,
    Welcome,
};
use wellfed_proto::setup::{Answer, Reply, Request};
use wellfed_proto::{decode, decode_padded, encode};

macro_rules! stable {
    ($ty:ty, $bytes:expr) => {
        if let Ok(value) = decode::<$ty>($bytes) {
            let mut out = [0u8; 4 * FRAME_MAX];
            let len = encode(&value, &mut out).expect("a decoded message re-encodes");
            let again = decode::<$ty>(&out[..len]).expect("a re-encoded message decodes");
            let mut out_again = [0u8; 4 * FRAME_MAX];
            let len_again = encode(&again, &mut out_again).expect("it re-encodes again");
            assert_eq!(out[..len], out_again[..len_again]);
        }
    };
}

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    stable!(Open, data);
    stable!(Hello, data);
    stable!(Join<'_>, data);
    stable!(ClientMessage<'_>, data);
    stable!(Welcome<'_>, data);
    stable!(Joined, data);
    stable!(DeviceMessage<'_>, data);
    stable!(Request<'_>, data);
    stable!(Answer<'_>, data);
    stable!(Reply, data);
    let _ = decode_padded::<EventBody>(data);
    if let Ok(DeviceMessage::Events(batch)) = decode::<DeviceMessage<'_>>(data) {
        // A body this build knows re-encodes as the ring keeps it and reads
        // back; one it does not is skipped, as a client skips it.
        for raw in batch.iter() {
            let Ok(raw) = raw else { break };
            let Some(event) = raw.event() else { continue };
            let again = RawEvent::encode(&event).expect("a decoded event re-encodes");
            assert_eq!(again.event(), Some(event));
        }
    }
});
