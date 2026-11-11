//! The signature scheme and the manifest header outside the offer check:
//! an RFC 8032 vector for Ed25519, and the version a header claims for a
//! signer picking its next version.

use ed25519_dalek::VerifyingKey;
use wellfed_signed::{claimed_version, offer};

#[test]
fn rfc8032_vector_one_verifies() {
    // RFC 8032 section 7.1, test 1 (empty message).
    let public: [u8; 32] = hex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
    let signature: [u8; 64] = hex(
        "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
    );

    let key = VerifyingKey::from_bytes(&public).unwrap();
    assert!(
        key.verify_strict(b"", &ed25519_dalek::Signature::from_bytes(&signature))
            .is_ok()
    );
}

fn hex<const N: usize>(s: &str) -> [u8; N] {
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
    }
    out
}

#[test]
fn a_header_claims_its_version_without_a_signature() {
    // A version 2 manifest keeps its version at byte 8.
    let mut v2 = vec![0; offer::MANIFEST_LEN];
    v2[..8].copy_from_slice(&offer::MAGIC);
    v2[8..12].copy_from_slice(&14u32.to_le_bytes());
    assert_eq!(claimed_version(&v2), Some(14));
    assert_eq!(claimed_version(&v2[..100]), None);
    v2[0] = b'X';
    assert_eq!(claimed_version(&v2), None);

    // So does a retired version 1 manifest a signer finds on disk.
    let mut v1 = vec![0; 148];
    v1[..8].copy_from_slice(b"PLOTA1\0\0");
    v1[8..12].copy_from_slice(&13u32.to_le_bytes());
    assert_eq!(claimed_version(&v1), Some(13));
    assert_eq!(claimed_version(&v1[..147]), None);
}
