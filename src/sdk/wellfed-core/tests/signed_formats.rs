//! The signed formats bank 0 checks, as known-answer bytes: what every
//! build must keep accepting from a client of any later build, for good.
//!
//! An update's offer is checked by bank 1 and its image received by bank 0,
//! which keeps the build it was written with. So the version 2 manifest,
//! the downgrade, key-add and revoke statements, the Ed25519 signatures
//! over them and the key ids that name their signers, and the limits and
//! loader layout an offer's image is checked against, are frozen with the
//! device API's update path. A line here that changes strands every
//! deployed bank 0: its feeder could never update again without a UART
//! cable ([device API](../../../../docs/firmware/device-api.md#the-frozen-part)).
//!
//! Ed25519 signatures are deterministic, so every signed line is a fixed
//! answer from a fixed key, and each is checked the way the feeder checks
//! it. The lines are only ever appended
//! (`WELLFED_BLESS=1 cargo nextest run -p wellfed-core --test signed_formats`).

use ed25519_dalek::{Signer as _, SigningKey};
use wellfed_core::ota::offer::*;
use wellfed_core::ota::{BANK1_BYTES, BANK1_OFFSET, MARKER, Manifest, SECTOR_BYTES, image};
use wellfed_session::hex::Lower;

const PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/signed-v1.txt");

const STRANDS: &str = "changing a frozen signed-format line strands every deployed bank 0: \
                       its feeder could never update again without a UART cable. Only a new \
                       major changes the frozen part; append new lines instead";

/// The feeder's identity key a downgrade statement names.
const DEVICE: [u8; 32] = [0xd0; 32];
/// The floor a downgrade statement is bound to.
const FLOOR: u32 = 9;
/// The model code of the first feeder, PLAF103; wellfed-proto's
/// `the_model_codes_signed_manifests_carry_never_change` ties it to `Model`.
const MODEL: u8 = 1;

struct Line {
    name: &'static str,
    bytes: Vec<u8>,
}

fn line(name: &'static str, bytes: &[u8]) -> Line {
    Line {
        name,
        bytes: bytes.to_vec(),
    }
}

fn signer() -> SigningKey {
    SigningKey::from_bytes(&[0x21; 32])
}

fn public(key: &SigningKey) -> [u8; 32] {
    key.verifying_key().to_bytes()
}

fn sign(key: &SigningKey, body: &[u8]) -> Vec<u8> {
    let mut out = body.to_vec();
    out.extend_from_slice(&key.sign(body).to_bytes());
    out
}

fn fields(kind: ImageKind, version: u32) -> Signed {
    Signed {
        manifest: Manifest {
            version,
            image_len: 3 * SECTOR_BYTES,
            target_bank: 1,
            image_sha256: [0x5a; 32],
            build_id: [0xb1; 32],
        },
        kind,
        model: MODEL,
        key_id: key_id(&public(&signer())),
    }
}

/// What a bank 0 of any build checks an offer against, trusting the
/// fixed key as built in.
fn context(built_in: &[[u8; 32]]) -> Context<'_> {
    Context {
        ring: KeyRing {
            built_in,
            added: None,
            enrolled: &[],
            revoked: &[],
        },
        floor: FLOOR,
        device: DEVICE,
        running: None,
        model: MODEL,
    }
}

/// The five 32-byte headers of a bank in the loader's one profile: KM0's
/// XIP and RAM parts, then from the next sector KM4's XIP, RAM and empty
/// PSRAM parts.
fn headers() -> Vec<u8> {
    let parts: [(u32, u32); 5] = [
        (0x0C00_0020, 5000),
        (0x0008_3000, 300),
        (0x0E00_0020, 20_000),
        (0x1000_5000, 700),
        (0x0200_0020, 0),
    ];
    parts
        .iter()
        .flat_map(|&(address, body)| {
            let mut header = MARKER.to_vec();
            header.extend_from_slice(&body.to_le_bytes());
            header.extend_from_slice(&address.to_le_bytes());
            header.extend_from_slice(&[0xff; 16]);
            header
        })
        .collect()
}

/// The bank the headers describe, as the loader lays it out.
fn bank(headers: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for (index, header) in headers.chunks(32).enumerate() {
        if index == 2 {
            out.resize(out.len().next_multiple_of(SECTOR_BYTES as usize), 0xff);
        }
        let body = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
        out.extend_from_slice(header);
        out.extend(std::iter::repeat_n(0x42, body as usize));
    }
    out
}

fn lines() -> Vec<Line> {
    let key = signer();
    let built_in = [public(&key)];
    let context = context(&built_in);
    let id = key_id(&public(&key));

    let replacement = sign(
        &key,
        &manifest_body(&fields(ImageKind::Replacement, FLOOR + 1)),
    );
    let admitted = admit_offer(&replacement, None, &context).unwrap();
    assert_eq!(admitted.trust, Trust::BuiltIn, "{STRANDS}");

    let stock = sign(&key, &manifest_body(&fields(ImageKind::Stock, 3)));
    let downgrade = sign(
        &key,
        &downgrade_body(&id, &DEVICE, &manifest_digest(&stock), FLOOR),
    );
    let admitted = admit_offer(&stock, Some(&downgrade), &context).unwrap();
    assert_eq!(
        admitted.downgrade.map(|signer| signer.key_id),
        Some(id),
        "{STRANDS}"
    );

    let added_key = public(&SigningKey::from_bytes(&[0x22; 32]));
    let key_add = sign(&key, &key_add_body(&id, &added_key));
    assert_eq!(
        admit_statement(&key_add, &context.ring),
        Ok(KeyStatement::Add(added_key)),
        "{STRANDS}"
    );
    let revoke = sign(&key, &revoke_body(&id, &[0x77; KEY_ID_LEN]));
    assert_eq!(
        admit_statement(&revoke, &context.ring),
        Ok(KeyStatement::Revoke([0x77; KEY_ID_LEN])),
        "{STRANDS}"
    );

    let headers = headers();
    let bank = bank(&headers);
    let extent = image::extent(
        |at, header| {
            let at = at as usize;
            let Some(bytes) = bank.get(at..at + 32) else {
                return false;
            };
            header.copy_from_slice(bytes);
            true
        },
        BANK1_BYTES,
    )
    .unwrap();

    let len = |value: usize| u32::try_from(value).unwrap().to_le_bytes();
    vec![
        line("manifest-magic", &MAGIC),
        line("key-add-magic", &KEY_ADD_MAGIC),
        line("revoke-magic", &REVOKE_MAGIC),
        line("downgrade-magic", &DOWNGRADE_MAGIC),
        line("signature-len", &len(SIGNATURE_LEN)),
        line("key-id-len", &len(KEY_ID_LEN)),
        line("manifest-len", &len(MANIFEST_LEN)),
        line("key-add-len", &len(KEY_ADD_BODY + SIGNATURE_LEN)),
        line("revoke-len", &len(REVOKE_BODY + SIGNATURE_LEN)),
        line("downgrade-len", &len(DOWNGRADE_BODY + SIGNATURE_LEN)),
        line("image-kind-replacement", &[ImageKind::Replacement.code()]),
        line("image-kind-stock", &[ImageKind::Stock.code()]),
        line("loader-marker", &MARKER),
        line("sector-bytes", &SECTOR_BYTES.to_le_bytes()),
        line("bank1-offset", &BANK1_OFFSET.to_le_bytes()),
        line("bank1-bytes", &BANK1_BYTES.to_le_bytes()),
        line("signer-public", &public(&key)),
        line("signer-key-id", &id),
        line("manifest-replacement", &replacement),
        line(
            "manifest-replacement-digest",
            &manifest_digest(&replacement),
        ),
        line("manifest-stock", &stock),
        line("statement-downgrade", &downgrade),
        line("statement-key-add", &key_add),
        line("statement-revoke", &revoke),
        line("loader-headers", &headers),
        line("loader-extent", &extent.to_le_bytes()),
    ]
}

fn render(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|line| format!("{} {}\n", line.name, Lower(&line.bytes)))
        .collect()
}

/// The pin: every line already in the file keeps its bytes. Blessing only
/// appends new lines, and refuses to touch a pinned one.
#[test]
fn the_signed_formats_keep_their_bytes() {
    let current = render(&lines());
    if std::env::var_os("WELLFED_BLESS").is_some() {
        let fixed = std::fs::read_to_string(PATH).unwrap_or_default();
        assert!(current.starts_with(&fixed), "{STRANDS}");
        std::fs::write(PATH, &current).unwrap();
    }

    let fixed = std::fs::read_to_string(PATH).unwrap();
    let mut pinned = 0;
    for (fixed, now) in fixed.lines().zip(current.lines()) {
        assert_eq!(fixed, now, "{STRANDS}");
        pinned += 1;
    }
    assert_eq!(
        pinned,
        fixed.lines().count(),
        "a pinned line is gone: {STRANDS}"
    );
    assert_eq!(fixed, current, "a frozen line was added: bless it");
}

/// A signature bank 0 accepts is Ed25519's, checked strictly: one flipped
/// bit anywhere in a signed line is refused.
#[test]
fn a_flipped_bit_in_any_signed_line_is_refused() {
    let key = signer();
    let built_in = [public(&key)];
    let context = context(&built_in);
    let manifest = sign(
        &key,
        &manifest_body(&fields(ImageKind::Replacement, FLOOR + 1)),
    );
    for at in [0, 30, MANIFEST_LEN - 1] {
        let mut flipped = manifest.clone();
        flipped[at] ^= 1;
        assert!(admit_offer(&flipped, None, &context).is_err(), "byte {at}");
    }
    let revoke = sign(
        &key,
        &revoke_body(&key_id(&public(&key)), &[0x77; KEY_ID_LEN]),
    );
    let mut flipped = revoke.clone();
    flipped[REVOKE_BODY] ^= 1;
    assert_eq!(
        admit_statement(&flipped, &context.ring),
        Err(StatementError::Signature)
    );
}
