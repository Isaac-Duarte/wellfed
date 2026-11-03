//! The server's id: `server.id` in the data directory, 16 lowercase hex
//! digits made at random on the first start and kept from then on.
//!
//! It is what the mDNS announcement carries, so the app and Home
//! Assistant's zeroconf recognise the same server after its address
//! changes, and it keeps the announced host name apart from the machine's
//! own. It is not a secret and is never derived from the sealing key: the
//! announcement is public on the LAN. A file that cannot be read or does
//! not hold an id (a first start cut short) is replaced by a new one, with
//! a warning, since nothing but a listener remembering the old one depends
//! on it.

use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::Deserialize as _;
use serde::de::IntoDeserializer as _;
use serde::de::value::StrDeserializer;
use wellfed_api::Hex;
use wellfed_service::Random;

/// The id file, in the data directory.
pub const ID_FILE: &str = "server.id";

/// The server's id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServerId(Hex<8>);

impl ServerId {
    /// Its first 8 hex digits: enough to keep one household's servers'
    /// host names apart.
    pub fn short(&self) -> String {
        let mut text = self.0.to_string();
        text.truncate(8);
        text
    }

    fn parse(text: &str) -> Option<Self> {
        let hex: StrDeserializer<'_, serde::de::value::Error> = text.into_deserializer();
        Hex::<8>::deserialize(hex).ok().map(Self)
    }
}

impl fmt::Display for ServerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IdError {
    #[error("write {}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("make a server id")]
    Random(#[source] wellfed_service::RandomFailed),
}

/// The id kept in `data`, made with `random` when there is none.
pub fn server_id(data: &Path, random: &dyn Random) -> Result<ServerId, IdError> {
    let path = data.join(ID_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => match ServerId::parse(text.trim_end()) {
            Some(id) => return Ok(id),
            None => tracing::warn!("{} holds no server id; making a new one", path.display()),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!("read {}: {error}; making a new server id", path.display());
        }
    }

    let mut bytes = [0; 8];
    random.fill(&mut bytes).map_err(IdError::Random)?;
    let id = ServerId(Hex(bytes));
    // Staged and renamed, so a cut leaves the old file or the whole new one.
    let staged = path.with_extension("id.new");
    let written = std::fs::File::create(&staged)
        .and_then(|mut file| {
            writeln!(file, "{id}")?;
            file.sync_all()
        })
        .and_then(|()| std::fs::rename(&staged, &path));
    written.map_err(|source| IdError::Write { path, source })?;
    tracing::info!("this server's id is {id}");
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(u8);

    impl Random for Fixed {
        fn fill(&self, out: &mut [u8]) -> Result<(), wellfed_service::RandomFailed> {
            out.fill(self.0);
            Ok(())
        }
    }

    #[test]
    fn the_id_is_made_once_and_kept() {
        let data = tempfile::tempdir().unwrap();

        let first = server_id(data.path(), &Fixed(0xab)).unwrap();
        let again = server_id(data.path(), &Fixed(0x01)).unwrap();

        assert_eq!(first.to_string(), "abababababababab");
        assert_eq!(again, first, "a second start reads the kept id");
        assert_eq!(first.short(), "abababab");
        let text = std::fs::read_to_string(data.path().join(ID_FILE)).unwrap();
        assert_eq!(text, "abababababababab\n");
    }

    #[test]
    fn a_file_holding_no_id_is_replaced() {
        for held in [&b"abab"[..], &[0xff, 0xfe, 0x00][..]] {
            let data = tempfile::tempdir().unwrap();
            std::fs::write(data.path().join(ID_FILE), held).unwrap();

            let id = server_id(data.path(), &Fixed(0x02)).unwrap();

            assert_eq!(id.to_string(), "0202020202020202", "{held:?}");
            assert_eq!(server_id(data.path(), &Fixed(0x03)).unwrap(), id);
        }
    }

    /// A path that cannot be read as a file (here a directory) is warned
    /// about, and the start goes on once a new id is written: only that
    /// write failing stops it.
    #[test]
    fn an_unreadable_id_is_never_fatal_by_itself() {
        let data = tempfile::tempdir().unwrap();
        std::fs::create_dir(data.path().join(ID_FILE)).unwrap();

        let made = server_id(data.path(), &Fixed(0x04));

        assert!(matches!(made, Err(IdError::Write { .. })), "{made:?}");
    }
}
