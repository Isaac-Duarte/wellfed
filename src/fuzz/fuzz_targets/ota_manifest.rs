//! The update path's first parser: any bytes as the version 2 manifest an
//! offer carries. The bank-1 writer behind it is held by its property tests
//! (`wellfed-core/tests/ota.rs`), which build images in any chunk order.
//! Any bytes alone never pass the signature, so the target also signs
//! them with a key of its own: then every check after the signature runs
//! on fuzzed fields.
#![no_main]

use ed25519_dalek::{Signer, SigningKey};
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

    // The same bytes as a body the fuzz key signed, naming that key.
    let key = SigningKey::from_bytes(&[0x17; 32]);
    let public = key.verifying_key().to_bytes();
    let signer = [public];
    let signed_context = Context {
        ring: KeyRing {
            built_in: &signer,
            ..context.ring
        },
        ..context
    };
    if let Some((&shape, rest)) = data.split_first()
        && let Some(fields) = rest.get(..offer::MANIFEST_BODY)
    {
        let mut body = [0u8; offer::MANIFEST_BODY];
        body.copy_from_slice(fields);
        body[..8].copy_from_slice(&offer::MAGIC);
        body[24..32].copy_from_slice(&offer::key_id(&public));
        // Half the inputs take a well-formed kind, model and bank, so the
        // version, size and floor checks after them see fuzzed values too
        // rather than waiting on four exact bytes at once.
        if shape & 1 == 1 {
            body[16..20].copy_from_slice(&1u32.to_le_bytes());
            body[20..24].copy_from_slice(&[1 + (shape >> 1 & 1), 1, 0, 0]);
        }
        let mut signed = body.to_vec();
        signed.extend_from_slice(&key.sign(&body).to_bytes());
        if let Ok(admitted) = offer::admit_offer(&signed, None, &signed_context) {
            // Without a downgrade statement only a replacement above the
            // floor, for this model and bank 1, may pass.
            let m = admitted.signed.manifest;
            assert!(m.version > signed_context.floor && m.target_bank == 1);
            assert!(wellfed_signed::image_len_fits(m.image_len));
            assert_eq!(admitted.signed.model, signed_context.model);
            assert_eq!(admitted.signed.kind, offer::ImageKind::Replacement);
            assert!(admitted.downgrade.is_none());
        }
    }
});
