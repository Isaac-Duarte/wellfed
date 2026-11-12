//! The sealing key's file, and the move from `config.json`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

use wellfed_server::config::DATABASE;
use wellfed_server::key::{KEY_DIR, KEY_FILE, KeyError, KeySource, sealing_key};
use wellfed_server_testkit::fixed::SeededRandom;
use wellfed_service::{Random, RandomFailed};

fn key_path(data: &Path) -> PathBuf {
    data.join(KEY_DIR).join(KEY_FILE)
}

/// `config.json` as servers before the settings file wrote it.
fn write_old(data: &Path, key: [u8; 32]) {
    let hex: String = key.iter().map(|byte| format!("{byte:02x}")).collect();
    std::fs::write(
        data.join("config.json"),
        format!("{{\n  \"key\": \"{hex}\"\n}}"),
    )
    .unwrap();
}

/// The staged copies of the key in the key directory.
fn staged(data: &Path) -> usize {
    std::fs::read_dir(data.join(KEY_DIR))
        .unwrap()
        .filter(|entry| {
            let name = entry.as_ref().unwrap().file_name();
            name.to_string_lossy().ends_with(".new")
        })
        .count()
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn the_first_start_makes_a_key_only_its_user_reads() {
    let data = tempfile::tempdir().unwrap();

    let (key, source) = sealing_key(data.path(), &SeededRandom::new(1)).unwrap();

    assert_eq!(source, KeySource::Made);
    assert_eq!(std::fs::read(key_path(data.path())).unwrap(), key);
    #[cfg(unix)]
    {
        assert_eq!(mode(&key_path(data.path())), 0o600);
        assert_eq!(mode(&data.path().join(KEY_DIR)), 0o700);
    }

    let (again, source) = sealing_key(data.path(), &SeededRandom::new(2)).unwrap();
    assert_eq!(
        (again, source),
        (key, KeySource::Read),
        "a restart reads it"
    );
}

/// A server from before the settings file keeps its key, and the old file
/// holding it is gone.
#[test]
fn the_key_moves_from_config_json() {
    let data = tempfile::tempdir().unwrap();
    let old = [0x5a; 32];
    write_old(data.path(), old);
    std::fs::write(data.path().join(DATABASE), b"a database").unwrap();

    let (key, source) = sealing_key(data.path(), &SeededRandom::new(1)).unwrap();

    assert_eq!((key, source), (old, KeySource::Moved));
    assert_eq!(std::fs::read(key_path(data.path())).unwrap(), old);
    #[cfg(unix)]
    assert_eq!(mode(&key_path(data.path())), 0o600);
    assert!(
        !data.path().join("config.json").exists(),
        "the old file is deleted"
    );
    assert_eq!(staged(data.path()), 0, "nothing is left staged");

    let (again, source) = sealing_key(data.path(), &SeededRandom::new(2)).unwrap();
    assert_eq!((again, source), (old, KeySource::Read));
}

/// A start stopped after the key file was written and before `config.json`
/// was deleted: the next one finishes the move.
#[test]
fn an_interrupted_move_finishes() {
    let data = tempfile::tempdir().unwrap();
    let old = [0x5a; 32];
    std::fs::create_dir(data.path().join(KEY_DIR)).unwrap();
    std::fs::write(key_path(data.path()), old).unwrap();
    write_old(data.path(), old);

    let (key, source) = sealing_key(data.path(), &SeededRandom::new(1)).unwrap();

    assert_eq!((key, source), (old, KeySource::Read));
    assert!(!data.path().join("config.json").exists());
}

/// A start stopped after linking the key into place and before removing
/// the staged copy: the next one removes it.
#[test]
fn a_leftover_staged_key_is_removed() {
    let data = tempfile::tempdir().unwrap();
    let (key, _) = sealing_key(data.path(), &SeededRandom::new(1)).unwrap();
    let staged = key_path(data.path()).with_extension("key.new");
    std::fs::hard_link(key_path(data.path()), &staged).unwrap();

    let (again, source) = sealing_key(data.path(), &SeededRandom::new(2)).unwrap();

    assert_eq!((again, source), (key, KeySource::Read));
    assert!(!staged.exists());
}

#[test]
fn two_different_keys_are_refused() {
    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir(data.path().join(KEY_DIR)).unwrap();
    std::fs::write(key_path(data.path()), [1; 32]).unwrap();
    write_old(data.path(), [2; 32]);

    let error = sealing_key(data.path(), &SeededRandom::new(1)).unwrap_err();

    assert!(matches!(error, KeyError::Conflict { .. }), "{error:?}");
    assert!(
        data.path().join("config.json").exists(),
        "neither key is deleted"
    );
    assert_eq!(std::fs::read(key_path(data.path())).unwrap(), [1; 32]);
}

/// A database restored without its key gets no new key, which could not
/// open the feeder keys it holds.
#[test]
fn a_database_without_its_key_is_refused() {
    let data = tempfile::tempdir().unwrap();
    std::fs::write(data.path().join(DATABASE), b"a database").unwrap();

    let error = sealing_key(data.path(), &SeededRandom::new(1)).unwrap_err();

    assert!(matches!(error, KeyError::Missing { .. }), "{error:?}");
    assert!(!key_path(data.path()).exists());
}

/// A key directory others could enter, made before the key, is closed
/// before the key goes in.
#[cfg(unix)]
#[test]
fn a_loose_key_directory_is_tightened() {
    use std::os::unix::fs::PermissionsExt;

    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join(KEY_DIR);
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();

    sealing_key(data.path(), &SeededRandom::new(1)).unwrap();

    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(&key_path(data.path())), 0o600);
}

#[cfg(unix)]
#[test]
fn a_loose_key_read_at_start_is_tightened() {
    use std::os::unix::fs::PermissionsExt;

    let data = tempfile::tempdir().unwrap();
    let (made, _) = sealing_key(data.path(), &SeededRandom::new(1)).unwrap();
    let dir = data.path().join(KEY_DIR);
    let path = key_path(data.path());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();

    let (read, source) = sealing_key(data.path(), &SeededRandom::new(2)).unwrap();

    assert_eq!((read, source), (made, KeySource::Read));
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(&path), 0o600);
}

#[test]
fn a_short_key_file_is_refused() {
    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir(data.path().join(KEY_DIR)).unwrap();
    std::fs::write(key_path(data.path()), [1; 31]).unwrap();

    let error = sealing_key(data.path(), &SeededRandom::new(1)).unwrap_err();

    assert!(
        matches!(error, KeyError::Length { len: 31, .. }),
        "{error:?}"
    );
}

#[test]
fn a_garbled_config_json_is_refused() {
    let data = tempfile::tempdir().unwrap();
    std::fs::write(data.path().join("config.json"), b"{\"key\": \"00\"}").unwrap();

    let error = sealing_key(data.path(), &SeededRandom::new(1)).unwrap_err();

    assert!(matches!(error, KeyError::OldFile { .. }), "{error:?}");
    assert!(data.path().join("config.json").exists());
}

/// Two first starts on one data directory at once: each that succeeds
/// returns the key the file holds, so what it seals opens after a restart.
/// Before each writer staged under its own name, one could link the other's
/// key into place and return its own.
#[test]
fn racing_first_starts_return_the_key_the_file_holds() {
    const STARTS: u64 = 8;

    for trial in 0..100 {
        let data = tempfile::tempdir().unwrap();
        let barrier = std::sync::Barrier::new(usize::try_from(STARTS).unwrap());

        let results: Vec<_> = std::thread::scope(|scope| {
            let starts: Vec<_> = (0..STARTS)
                .map(|start| {
                    let (data, barrier) = (data.path(), &barrier);
                    scope.spawn(move || {
                        let random = SeededRandom::new(trial * STARTS + start + 1);
                        barrier.wait();
                        sealing_key(data, &random)
                    })
                })
                .collect();
            starts
                .into_iter()
                .map(|start| start.join().unwrap())
                .collect()
        });

        let file = std::fs::read(key_path(data.path())).unwrap();
        let mut made = 0;
        for result in results {
            match result {
                Ok((key, _)) => {
                    assert_eq!(key.as_slice(), file, "trial {trial}");
                    made += 1;
                }
                Err(error) => assert!(
                    matches!(error, KeyError::Exists { .. }),
                    "trial {trial}: {error:?}"
                ),
            }
        }
        assert!(made >= 1, "trial {trial}: one start makes the key");

        let (again, source) = sealing_key(data.path(), &SeededRandom::new(0)).unwrap();
        assert_eq!(
            (again.as_slice(), source),
            (file.as_slice(), KeySource::Read)
        );
        assert_eq!(
            staged(data.path()),
            0,
            "trial {trial}: nothing is left staged"
        );
    }
}

/// A writer's random source whose first staged name is every writer's:
/// as in two containers, where each server is PID 1. Its key, and every
/// later name, are its own.
struct SameFirstName {
    writer: u8,
    names: AtomicU8,
}

impl Random for SameFirstName {
    fn fill(&self, out: &mut [u8]) -> Result<(), RandomFailed> {
        if out.len() == 32 {
            out.fill(self.writer);
        } else {
            let name = self.names.fetch_add(1, Ordering::Relaxed);
            let byte = if name == 0 { 0xaa } else { self.writer };
            out.fill(byte);
            out[0] = name;
        }
        Ok(())
    }
}

/// Writers whose first staged name is the same: neither removes or
/// rewrites the other's staged key, so each that succeeds returns the key
/// the file holds.
#[test]
fn starts_that_draw_the_same_staged_name_return_the_key_the_file_holds() {
    const STARTS: u8 = 8;

    for trial in 0..100 {
        let data = tempfile::tempdir().unwrap();
        let barrier = std::sync::Barrier::new(usize::from(STARTS));

        let results: Vec<_> = std::thread::scope(|scope| {
            let starts: Vec<_> = (1..=STARTS)
                .map(|writer| {
                    let (data, barrier) = (data.path(), &barrier);
                    scope.spawn(move || {
                        let random = SameFirstName {
                            writer,
                            names: AtomicU8::new(0),
                        };
                        barrier.wait();
                        sealing_key(data, &random)
                    })
                })
                .collect();
            starts
                .into_iter()
                .map(|start| start.join().unwrap())
                .collect()
        });

        let file = std::fs::read(key_path(data.path())).unwrap();
        let mut made = 0;
        for result in results {
            match result {
                Ok((key, _)) => {
                    assert_eq!(key.as_slice(), file, "trial {trial}");
                    made += 1;
                }
                Err(error) => assert!(
                    matches!(error, KeyError::Exists { .. }),
                    "trial {trial}: {error:?}"
                ),
            }
        }
        assert!(made >= 1, "trial {trial}: one start makes the key");
    }
}
