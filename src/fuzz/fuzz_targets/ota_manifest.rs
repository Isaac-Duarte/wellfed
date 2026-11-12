//! The update path's first parser: any bytes as the version 2 manifest an
//! offer carries. The bank-1 writer behind it is held by its property tests
//! (`wellfed-core/tests/ota.rs`), which build images in any chunk order.
#![no_main]

use wellfed_signed::offer::{self, Context, KeyRing};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    // No signer holds this key's secret, so nothing may be admitted.
    let built_in = [[0x5a; 32]];
    let context = Context {
        ring: KeyRing {
            built_in: &built_in,
            added: None,
            enrolled: &[],
            revoked: &[],
        },
        floor: 0,
        device: [0; 32],
        running: None,
        model: 1,
    };
    assert!(offer::admit_offer(data, None, &context).is_err());
    if let Some(head) = data.get(..offer::MANIFEST_LEN) {
        assert!(offer::admit_offer(head, None, &context).is_err());
    }
});
