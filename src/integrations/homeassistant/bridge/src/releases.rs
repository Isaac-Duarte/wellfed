//! The releases an owner's Home Assistant may install: the project's
//! GitHub releases, or a `.plota` file the owner built or downloaded.
//!
//! This is the server's release cache (`wellfed-releases`), kept in a
//! folder of Home Assistant's own. Every file is verified with
//! `wellfed_client::release::verify`, the feeder's own offer check, against
//! the project keys set in the integration's options and the update keys
//! the owner enrolled on the feeder, both when it is taken in and again
//! when it is read for an install. Nothing here talks to the feeder: an
//! install is the session's ([`crate::feeder::Feeder::install`]).

use std::path::PathBuf;

use jiff::Timestamp;
use wellfed_api::{Release, ReleaseQuery, ReleaseSigner, ReleaseSource};
use wellfed_model::Hex;
use wellfed_releases::{Config, Keys, Releases, Source, StockConfig, StockPins};

use crate::Error;
use crate::api::{Signer, UpdateOffer};

/// A release cache in `dir`, trusting `keys`.
#[derive(Clone, Debug)]
pub struct Library {
    pub dir: PathBuf,
    pub keys: Keys,
}

impl Library {
    async fn open(&self, fetch: bool) -> Result<Releases, Error> {
        let config = Config {
            dir: self.dir.join("releases"),
            keys: self.keys.clone(),
            fetch: fetch.then(Source::default),
            // Home Assistant restores no stock: the folder stays empty.
            stock: StockConfig {
                dir: self.dir.join("stock"),
                pins: StockPins::STOCK,
            },
        };
        Ok(Releases::open(config).await?)
    }

    /// The newest release held, after fetching the project's releases the
    /// cache lacks when `fetch`; `None` with none held.
    pub async fn newest(&self, fetch: bool) -> Result<Option<UpdateOffer>, Error> {
        let releases = self.open(fetch).await?;
        if fetch {
            releases.check(Timestamp::now()).await?;
        }
        let query = ReleaseQuery {
            after: None,
            limit: Some(1),
        };
        let page = releases.list(&query).await?;
        Ok(page.items.first().map(offer))
    }

    /// Verify `file` and keep it.
    pub async fn add(&self, file: Vec<u8>) -> Result<UpdateOffer, Error> {
        let releases = self.open(false).await?;
        let (release, _) = releases.upload(file, Timestamp::now()).await?;
        Ok(offer(&release))
    }

    /// The release `id` names, verified again against today's keys, and its
    /// file.
    pub async fn file(&self, id: [u8; 32]) -> Result<(UpdateOffer, Vec<u8>), Error> {
        let releases = self.open(false).await?;
        let (release, file) = releases.file(Hex(id)).await?;
        Ok((offer(&release), file))
    }
}

fn offer(release: &Release) -> UpdateOffer {
    UpdateOffer {
        id: release.id.to_string(),
        version: release.version,
        build: release.build_id.to_string(),
        signer: match release.signer {
            ReleaseSigner::Project => Signer::Project,
            ReleaseSigner::Owner => Signer::Owner,
        },
        tag: match &release.source {
            ReleaseSource::Github { tag, .. } => Some(tag.clone()),
            ReleaseSource::Upload => None,
        },
    }
}

impl Library {
    /// The cache in `dir`, trusting the project keys `project` (the
    /// integration's options) and the owner's enrolled `owner` keys.
    pub fn new(dir: PathBuf, project: Vec<Vec<u8>>, owner: Vec<[u8; 32]>) -> Result<Self, Error> {
        let project = project
            .into_iter()
            .map(|key| crate::key::bytes32(&key, Error::KeyLength))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            dir,
            keys: Keys { project, owner },
        })
    }

    /// Verify the release file at `path` and keep it.
    pub async fn add_file(&self, path: PathBuf) -> Result<UpdateOffer, Error> {
        let len = std::fs::metadata(&path).map_err(Error::File)?.len();
        let fits = usize::try_from(len).is_ok_and(|len| len <= wellfed_releases::MAX_FILE);
        wellfed_formats::require(fits, || Error::FileTooLarge(len))?;
        let file = std::fs::read(&path).map_err(Error::File)?;
        self.add(file).await
    }
}

/// A release's id as an offer names it: its manifest digest in hex.
pub fn release_id(text: &str) -> Result<[u8; 32], Error> {
    wellfed_formats::hex::decode::<32>(text).ok_or(Error::ReleaseId)
}

/// Run `future` to its end on a runtime of this call's own: the calls
/// Home Assistant makes from its executor, off the session's thread.
pub fn block_on<F: std::future::Future>(future: F) -> Result<F::Output, Error> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(Error::Thread)?;
    Ok(runtime.block_on(future))
}

impl crate::feeder::Feeder {
    /// Install the release `id` from `library`, read and verified again
    /// first (owners only).
    pub fn install_release(
        &self,
        library: &Library,
        id: [u8; 32],
    ) -> Result<crate::api::Install, Error> {
        let (offer, file) = block_on(library.file(id))??;
        let build = wellfed_formats::hex::decode::<32>(&offer.build).ok_or(Error::ReleaseId)?;
        Ok(self.install(file, build))
    }
}
