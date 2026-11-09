//! Tests for the update logic: the bank-1 sector writer, the boot-trial
//! decision and confirmation records.
//!
//! The writer must withhold the marker until commit and refuse to commit
//! after any order, marker, readback or hash failure, so an incomplete image
//! can never be activated (design: docs/firmware/ota-design.md).

use sha2::{Digest, Sha256};
use wellfed_core::ota::*;
use wellfed_signed::{BANK1_BYTES, BANK1_OFFSET, MARKER, Manifest, SECTOR, SECTOR_BYTES};

fn image(len: usize) -> Vec<u8> {
    let mut out: Vec<u8> = (0..len).map(|i| (i * 31 % 251) as u8).collect();
    out[..8].copy_from_slice(&MARKER);
    out
}

/// The manifest an offer for `payload` names, as the offer check admits it.
fn manifest(version: u32, payload: &[u8]) -> Manifest {
    Manifest {
        version,
        image_len: u32::try_from(payload.len()).unwrap(),
        target_bank: 1,
        image_sha256: Sha256::digest(payload).into(),
        build_id: [0xab; 32],
    }
}

/// Drive a writer to completion; `flash` models bank 1 (offset 0 = BANK1_OFFSET).
fn write_all(
    w: &mut Writer,
    payload: &[u8],
    flash: &mut Vec<u8>,
    bad_readback: Option<u32>,
) -> Result<(), WriteError> {
    loop {
        let wanted: Vec<_> = w.missing().collect();
        if wanted.is_empty() {
            break;
        }

        // Last chunk first: a sector's chunks may arrive in any order.
        for (offset, len) in wanted.into_iter().rev() {
            let chunk = &payload[offset as usize..(offset + len) as usize];
            if let Some(program) = w.accept(offset, chunk)? {
                let at = (program.offset - BANK1_OFFSET) as usize;
                if flash.len() < at + program.data.len() {
                    flash.resize(at + program.data.len(), 0xff);
                }
                flash[at..at + program.data.len()].copy_from_slice(program.data);
                let ok = bad_readback != Some(program.offset);
                w.programmed(ok)?;
            }
        }
    }

    Ok(())
}

#[test]
fn writer_stages_every_sector_with_the_marker_withheld_until_commit() {
    for chunk in [512, 1024, SECTOR] {
        let payload = image(3 * SECTOR + 100);
        let m = manifest(2, &payload);
        let mut w = Writer::new(m, chunk).unwrap();
        let mut flash = Vec::new();
        write_all(&mut w, &payload, &mut flash, None).unwrap();

        // Everything but the marker is in place; the marker bytes are erased.
        assert_eq!(&flash[..8], &[0xff; 8]);
        assert_eq!(&flash[8..], &payload[8..]);
        assert_eq!(w.commit(), Ok((BANK1_OFFSET, MARKER)));
    }

    assert!(Writer::new(manifest(2, &image(SECTOR)), 3000).is_none());
    // More chunks per sector than the received bitmap tracks.
    assert!(Writer::new(manifest(2, &image(SECTOR)), 64).is_none());
}

#[test]
fn writer_refuses_disorder_bad_markers_readback_and_hash_and_never_commits() {
    let payload = image(2 * SECTOR + 10);
    let m = manifest(2, &payload);

    // Only the staged sector's chunks, each once.
    let mut w = Writer::new(m, 1024).unwrap();
    let next = &payload[SECTOR..SECTOR + 1024];
    assert_eq!(w.accept(SECTOR as u32, next), Err(WriteError::Order));
    assert_eq!(w.commit(), Err(WriteError::State));

    let mut w = Writer::new(m, 1024).unwrap();
    assert_eq!(w.accept(1024, &payload[1024..2048]), Ok(None));
    assert_eq!(w.accept(1024, &payload[1024..2048]), Err(WriteError::Order));

    let mut w = Writer::new(m, 1024).unwrap();
    assert_eq!(w.accept(512, &payload[512..1536]), Err(WriteError::Order));

    let mut w = Writer::new(m, 1024).unwrap();
    assert_eq!(w.accept(0, &payload[..1000]), Err(WriteError::Order));

    let mut no_marker = payload.clone();
    no_marker[0] = b'0';
    let mut w = Writer::new(m, 1024).unwrap();
    assert_eq!(w.accept(0, &no_marker[..1024]), Err(WriteError::Marker));

    let mut w = Writer::new(m, 1024).unwrap();
    let result = write_all(
        &mut w,
        &payload,
        &mut Vec::new(),
        Some(BANK1_OFFSET + SECTOR as u32),
    );
    assert_eq!(result, Err(WriteError::Readback));
    assert_eq!(w.failed(), Some(WriteError::Readback));
    assert_eq!(w.commit(), Err(WriteError::State));

    // A corrupted byte after the marker passes every chunk check, then fails the hash.
    let mut corrupt = payload.clone();
    corrupt[SECTOR + 5] ^= 0x10;
    let mut w = Writer::new(m, 1024).unwrap();
    assert_eq!(
        write_all(&mut w, &corrupt, &mut Vec::new(), None),
        Err(WriteError::Hash)
    );
    assert_eq!(w.commit(), Err(WriteError::State));

    // Truncated: the writer keeps asking for the rest and cannot commit.
    let mut w = Writer::new(m, 1024).unwrap();
    for offset in (0..SECTOR as u32).step_by(1024) {
        let _ = w
            .accept(offset, &payload[offset as usize..offset as usize + 1024])
            .unwrap();
    }
    w.programmed(true).unwrap();
    assert_eq!(w.next_request(), Some((SECTOR as u32, 1024)));
    assert_eq!(w.commit(), Err(WriteError::State));
}

#[test]
fn trial_counts_unconfirmed_boots_then_gives_way() {
    assert_eq!(trial(true, 9, 3), Trial::Run);
    assert_eq!(trial(false, 0, 3), Trial::Count(1));
    assert_eq!(trial(false, 2, 3), Trial::Count(3));
    assert_eq!(trial(false, 3, 3), Trial::GiveWay);
    assert_eq!(trial(false, 0, 0), Trial::Count(1));
    assert_eq!(trial(false, 1, 0), Trial::GiveWay);
}

#[test]
fn confirmation_names_exactly_one_image() {
    let sha = [3u8; 32];
    let record = confirm_record(&sha, 7);
    assert_eq!(confirmed(&record, &sha), Some(7));
    assert_eq!(confirmed(&record, &[4u8; 32]), None);
    assert_eq!(confirmed(&record[1..], &sha), None);

    let mut bad = record;
    bad[0] = b'X';
    assert_eq!(confirmed(&bad, &sha), None);
}

/// Bounded properties of what an update server, or anyone posing as one,
/// can send: the chunks of an admitted offer.
mod properties {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Any image, chunked any way the bitmap tracks, its chunks arriving
        /// in any order within a sector, lands intact with the marker
        /// withheld until commit.
        #[test]
        fn any_image_in_any_chunk_order_lands_intact(
            len in SECTOR..3 * SECTOR,
            fill in any::<u8>(),
            chunk in proptest::sample::select(vec![128usize, 256, 512, 1024, 2048, SECTOR]),
            seed in any::<u64>(),
        ) {
            let mut payload: Vec<u8> =
                (0..len).map(|i| fill.wrapping_add((i * 131 % 251) as u8)).collect();
            payload[..MARKER.len()].copy_from_slice(&MARKER);
            let manifest = Manifest {
                version: 2,
                image_len: u32::try_from(len).unwrap(),
                target_bank: 1,
                image_sha256: Sha256::digest(&payload).into(),
                build_id: [0; 32],
            };
            let mut w = Writer::new(manifest, chunk).unwrap();
            let mut flash = vec![0xff; len];
            let mut state = seed | 1;

            loop {
                let mut wanted: Vec<_> = w.missing().collect();
                if wanted.is_empty() {
                    break;
                }
                // A seeded shuffle, so a failure replays.
                for i in (1..wanted.len()).rev() {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    wanted.swap(i, usize::try_from(state % (i as u64 + 1)).unwrap());
                }
                for (offset, len) in wanted {
                    let data = &payload[offset as usize..(offset + len) as usize];
                    if let Some(program) = w.accept(offset, data).unwrap() {
                        let at = (program.offset - BANK1_OFFSET) as usize;
                        flash[at..at + program.data.len()].copy_from_slice(program.data);
                        w.programmed(true).unwrap();
                    }
                }
            }

            prop_assert_eq!(&flash[..MARKER.len()], &[0xff; 8]);
            prop_assert_eq!(&flash[MARKER.len()..], &payload[MARKER.len()..]);
            prop_assert_eq!(w.commit(), Ok((BANK1_OFFSET, MARKER)));
        }

        /// Whatever chunks arrive, in whatever order, the writer never
        /// panics and never commits an image whose hash it did not see.
        #[test]
        fn the_writer_survives_any_chunks(
            image_len in SECTOR_BYTES..=BANK1_BYTES,
            calls in proptest::collection::vec(
                (any::<u32>(), 0usize..SECTOR + 2, any::<u8>(), any::<bool>()),
                0..24,
            ),
        ) {
            let manifest = Manifest {
                version: 2,
                image_len,
                target_bank: 1,
                image_sha256: [0; 32],
                build_id: [0; 32],
            };
            let mut w = Writer::new(manifest, 512).unwrap();
            for (offset, len, byte, readback) in calls {
                // Bias half the offsets onto the chunk grid, where the
                // writer's arithmetic happens.
                let offset = if byte & 1 == 0 { offset % (2 * SECTOR_BYTES) / 512 * 512 } else { offset };
                let data = vec![byte; len];
                if let Ok(Some(_)) = w.accept(offset, &data) {
                    let _ = w.programmed(readback);
                }
                let _ = w.next_request();
            }
            prop_assert!(w.commit().is_err());
        }
    }
}
