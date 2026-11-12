//! The release cache over a temporary directory: uploads verified, kept,
//! read back and listed.

mod support;

use ed25519_dalek::SigningKey;
use support::{keys, now, open, open_with, owner, project, release, versions as ids};
use wellfed_api::{ReleaseQuery, ReleaseRefusal, ReleaseSigner, ReleaseSource};
use wellfed_releases::Keys;
use wellfed_releases::{Added, Error, ReleaseError};
use wellfed_signed::SECTOR;
use wellfed_signed::offer;

#[tokio::test]
async fn an_upload_is_verified_and_kept() {
    let dir = tempfile::tempdir().unwrap();
    let releases = open(&dir, None).await;
    let file = release(&project(), 7);

    let (release, added) = releases.upload(file.clone(), now()).await.unwrap();

    assert_eq!(added, Added::New);
    assert_eq!(release.version, 7);
    assert_eq!(release.signer, ReleaseSigner::Project);
    assert_eq!(release.source, ReleaseSource::Upload);
    assert_eq!(release.added, now());
    assert_eq!(
        release.id.0,
        offer::manifest_digest(&file[..offer::MANIFEST_LEN])
    );

    let (again, added) = releases.upload(file.clone(), now()).await.unwrap();
    assert_eq!((again, added), (release.clone(), Added::Known));

    let (read, bytes) = releases.file(release.id).await.unwrap();
    assert_eq!((read, bytes), (release.clone(), file));

    // The cache outlives the process.
    drop(releases);
    let reopened = open(&dir, None).await;
    assert_eq!(ids(&reopened).await, [7]);
}

#[tokio::test]
async fn an_owners_key_signs_a_self_built_release() {
    let dir = tempfile::tempdir().unwrap();
    let releases = open(&dir, None).await;

    let (release, _) = releases.upload(release(&owner(), 3), now()).await.unwrap();

    assert_eq!(release.signer, ReleaseSigner::Owner);
}

#[tokio::test]
async fn a_bad_signature_is_refused_and_not_kept() {
    let dir = tempfile::tempdir().unwrap();
    let releases = open(&dir, None).await;
    let mut file = release(&project(), 7);
    file[offer::MANIFEST_LEN - 3] ^= 0x10;

    let error = releases.upload(file, now()).await.unwrap_err();

    assert!(
        matches!(error, Error::Refused(ReleaseError::Signature)),
        "{error:?}"
    );
    assert_eq!(error.refusal(), Some(ReleaseRefusal::Signature));
    assert!(ids(&releases).await.is_empty());
}

#[tokio::test]
async fn a_release_signed_by_another_key_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let releases = open(&dir, None).await;
    let stranger = SigningKey::from_bytes(&[0x07; 32]);

    let error = releases
        .upload(release(&stranger, 7), now())
        .await
        .unwrap_err();

    assert_eq!(error.refusal(), Some(ReleaseRefusal::UnknownKey));
    assert!(ids(&releases).await.is_empty());
}

#[tokio::test]
async fn a_corrupted_upload_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let releases = open(&dir, None).await;
    let file = release(&project(), 7);

    let mut flipped = file.clone();
    flipped[offer::MANIFEST_LEN + SECTOR] ^= 0x01;
    let error = releases.upload(flipped, now()).await.unwrap_err();
    assert_eq!(error.refusal(), Some(ReleaseRefusal::Image));

    let cut = file[..file.len() - SECTOR].to_vec();
    let error = releases.upload(cut, now()).await.unwrap_err();
    assert_eq!(error.refusal(), Some(ReleaseRefusal::Image));

    let error = releases.upload(b"hello".to_vec(), now()).await.unwrap_err();
    assert_eq!(error.refusal(), Some(ReleaseRefusal::Size));

    assert!(ids(&releases).await.is_empty());
}

/// The record beside a file is the server's own note: what an install acts
/// on (the build it waits for, the version and length it records) is what
/// the verified manifest says, whatever the record was changed to.
#[tokio::test]
async fn a_record_is_never_believed_over_the_verified_manifest() {
    let edits = [
        ("version", serde_json::json!(99)),
        ("build_id", serde_json::json!("09".repeat(32))),
        ("model", serde_json::json!("plwf106")),
        ("image_len", serde_json::json!(4096)),
        ("image_sha256", serde_json::json!("00".repeat(32))),
        ("key_id", serde_json::json!("ff".repeat(8))),
        ("signer", serde_json::json!("owner")),
    ];
    for (field, value) in edits {
        let dir = tempfile::tempdir().unwrap();
        let releases = open(&dir, None).await;
        let file = release(&project(), 7);
        let (kept, _) = releases.upload(file.clone(), now()).await.unwrap();

        let path = dir.path().join(format!("releases/{}.json", kept.id));
        let record = std::fs::read_to_string(&path).unwrap();
        let mut edited: serde_json::Value = serde_json::from_str(&record).unwrap();
        let old = std::mem::replace(&mut edited["release"][field], value);
        assert!(!old.is_null(), "{field} in {record}");
        std::fs::write(&path, serde_json::to_vec_pretty(&edited).unwrap()).unwrap();

        let (read, _) = releases.file(kept.id).await.unwrap();
        assert_eq!(read, kept, "after {field} was changed");

        // Uploading it again writes the record back as the manifest says.
        let (again, added) = releases.upload(file, now()).await.unwrap();
        assert_eq!((again, added), (kept, Added::Known));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), record);
    }
}

#[tokio::test]
async fn a_record_naming_another_release_is_refused_when_read() {
    let dir = tempfile::tempdir().unwrap();
    let releases = open(&dir, None).await;
    let (seven, _) = releases
        .upload(release(&project(), 7), now())
        .await
        .unwrap();
    let (eight, _) = releases
        .upload(release(&project(), 8), now())
        .await
        .unwrap();

    let cache = dir.path().join("releases");
    std::fs::copy(
        cache.join(format!("{}.plota", eight.id)),
        cache.join(format!("{}.plota", seven.id)),
    )
    .unwrap();

    let error = releases.file(seven.id).await.unwrap_err();
    assert!(matches!(error, Error::Changed), "{error:?}");
}

/// A cached file damaged on disk is refused when read, as the server's own
/// fault rather than the owner's upload, and the same release uploaded
/// again writes it back whole: flipped, cut short or gone.
#[tokio::test]
async fn a_cached_file_changed_on_disk_is_refused_until_uploaded_again() {
    type Damage = fn(&mut Vec<u8>);
    let damages: [(&str, Damage); 3] = [
        ("flipped", |bytes| *bytes.last_mut().unwrap() ^= 0x01),
        ("cut short", |bytes| bytes.truncate(bytes.len() - 1)),
        ("emptied", Vec::clear),
    ];
    for (name, damage) in damages {
        let dir = tempfile::tempdir().unwrap();
        let releases = open(&dir, None).await;
        let file = release(&project(), 7);
        let (release, _) = releases.upload(file.clone(), now()).await.unwrap();

        let path = dir.path().join(format!("releases/{}.plota", release.id));
        let mut bytes = std::fs::read(&path).unwrap();
        damage(&mut bytes);
        std::fs::write(&path, bytes).unwrap();

        let error = releases.file(release.id).await.unwrap_err();
        assert!(matches!(error, Error::Changed), "{name}: {error:?}");

        let (again, added) = releases.upload(file.clone(), now()).await.unwrap();
        assert_eq!((again, added), (release.clone(), Added::Known), "{name}");
        assert_eq!(
            releases.file(release.id).await.unwrap(),
            (release, file),
            "{name}"
        );
    }

    // A file that is gone takes the record with it, and is kept anew.
    let dir = tempfile::tempdir().unwrap();
    let releases = open(&dir, None).await;
    let file = release(&project(), 7);
    let (release, _) = releases.upload(file.clone(), now()).await.unwrap();
    std::fs::remove_file(dir.path().join(format!("releases/{}.plota", release.id))).unwrap();
    let (_, added) = releases.upload(file.clone(), now()).await.unwrap();
    assert_eq!(added, Added::New);
    assert_eq!(releases.file(release.id).await.unwrap().1, file);
}

#[tokio::test]
async fn a_cut_short_write_is_swept_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("releases");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("ab.plota.part"), b"half").unwrap();
    std::fs::write(cache.join("cd.plota"), b"no record").unwrap();

    let releases = open(&dir, None).await;

    assert!(ids(&releases).await.is_empty());
    assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
}

#[tokio::test]
async fn pages_run_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let releases = open(&dir, None).await;
    for version in [4, 9, 6] {
        releases
            .upload(release(&project(), version), now())
            .await
            .unwrap();
    }

    let first = releases
        .list(&ReleaseQuery {
            model: None,
            after: None,
            limit: Some(2),
        })
        .await
        .unwrap();
    let versions: Vec<u32> = first.items.iter().map(|release| release.version).collect();
    assert_eq!(versions, [9, 6]);

    let second = releases
        .list(&ReleaseQuery {
            model: None,
            after: first.next,
            limit: Some(2),
        })
        .await
        .unwrap();
    let versions: Vec<u32> = second.items.iter().map(|release| release.version).collect();
    assert_eq!(versions, [4]);
    assert_eq!(second.next, None);

    for (after, limit) in [(Some("zz".to_owned()), None), (None, Some(0))] {
        let error = releases
            .list(&ReleaseQuery {
                after,
                limit,
                model: None,
            })
            .await
            .unwrap_err();
        assert!(
            matches!(error, Error::Cursor | Error::Limit { .. }),
            "{error:?}"
        );
    }
}

#[tokio::test]
async fn a_damaged_record_is_skipped_not_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let releases = open(&dir, None).await;
    let (kept, _) = releases
        .upload(release(&project(), 7), now())
        .await
        .unwrap();
    let (lost, _) = releases
        .upload(release(&project(), 8), now())
        .await
        .unwrap();
    let (garbled, _) = releases
        .upload(release(&project(), 9), now())
        .await
        .unwrap();

    let cache = dir.path().join("releases");
    std::fs::remove_file(cache.join(format!("{}.plota", lost.id))).unwrap();
    std::fs::write(cache.join(format!("{}.json", garbled.id)), b"{").unwrap();

    assert_eq!(ids(&releases).await, [7]);
    let error = releases.file(lost.id).await.unwrap_err();
    assert!(matches!(error, Error::NotFound), "{error:?}");

    // Uploading it again repairs it.
    let (again, added) = releases
        .upload(release(&project(), 9), now())
        .await
        .unwrap();
    assert_eq!((again.id, added), (garbled.id, Added::New));
    assert_eq!(ids(&releases).await, [9, 7]);
    assert_eq!(releases.file(kept.id).await.unwrap().0.version, 7);
}

#[tokio::test]
async fn the_listing_follows_todays_keys() {
    let dir = tempfile::tempdir().unwrap();
    let releases = open(&dir, None).await;
    releases
        .upload(release(&project(), 7), now())
        .await
        .unwrap();
    let (own, _) = releases.upload(release(&owner(), 8), now()).await.unwrap();
    drop(releases);

    let without_owner = Keys {
        owner: Vec::new(),
        ..keys()
    };
    let releases = open_with(&dir, without_owner, None).await;

    assert_eq!(ids(&releases).await, [7]);
    let error = releases.file(own.id).await.unwrap_err();
    assert_eq!(error.refusal(), Some(ReleaseRefusal::UnknownKey));
}

#[tokio::test]
async fn a_cursor_naming_a_hidden_release_keeps_its_place() {
    let dir = tempfile::tempdir().unwrap();
    let releases = open(&dir, None).await;
    releases
        .upload(release(&project(), 9), now())
        .await
        .unwrap();
    releases.upload(release(&owner(), 8), now()).await.unwrap();
    releases
        .upload(release(&project(), 7), now())
        .await
        .unwrap();
    let first = releases
        .list(&ReleaseQuery {
            model: None,
            after: None,
            limit: Some(2),
        })
        .await
        .unwrap();
    assert_eq!(first.items.last().unwrap().version, 8);
    drop(releases);

    let without_owner = Keys {
        owner: Vec::new(),
        ..keys()
    };
    let releases = open_with(&dir, without_owner, None).await;
    let second = releases
        .list(&ReleaseQuery {
            model: None,
            after: first.next,
            limit: Some(2),
        })
        .await
        .unwrap();

    let versions: Vec<u32> = second.items.iter().map(|release| release.version).collect();
    assert_eq!(versions, [7]);
}
