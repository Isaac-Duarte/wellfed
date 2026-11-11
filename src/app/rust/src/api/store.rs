//! The app's own file in its private storage: how feeders list this phone,
//! and the feeders it owns or uses. The client secret is not here; it
//! lives in secure storage. Written atomically, so a crash keeps the old or
//! the new file, never a torn one, and versioned ([`crate::stored`]): a file
//! this build cannot read is set aside at the start, never deleted, and the
//! app carries on with an empty one and says so.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::api::error::{BridgeError, ErrorKind};
pub use crate::api::server_views::{KnownServer, ListedFeeder, ServerLink};
use crate::stored::{self, Opened, Versioned};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppStore {
    /// How feeders list this phone: 1-16 bytes. Empty until setup asks.
    pub label: String,
    pub feeders: Vec<KnownFeeder>,
    pub appearance: Appearance,
    /// The owner's server this phone logged in to, if any, with the
    /// feeders it last listed. Its token's secret is in secure storage, as
    /// the client secret is.
    pub server: Option<KnownServer>,
}

/// `wellfed.json` as it is on disk: the store, and the file it replaced when
/// that one did not read, kept until the owner has read the notice, so an
/// app that dies before then says so again at its next start.
#[flutter_rust_bridge::frb(ignore)]
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AppFile {
    store: AppStore,
    set_aside: Option<SetAside>,
}

impl Versioned for AppFile {
    const VERSION: u32 = 1;
}

/// Which of the app's files was set aside.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreKind {
    /// `wellfed.json`: the feeders this phone knows and its server.
    App,
    /// `pending.json`: the writes that may have run.
    Pending,
    /// `alerts.json`: what the phone keeps of the feeders' alerts.
    Alerts,
}

/// A file of the app's that did not read at the start, kept under another
/// name beside it and replaced with an empty one. Stored in `wellfed.json`
/// and `pending.json` until the owner reads the notice, so a change to it
/// changes those files' shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetAside {
    pub kind: StoreKind,
    /// Where it is kept now.
    pub moved_to: String,
    /// Why it did not read, for a support request.
    pub reason: String,
    /// When it was set aside, by the phone's clock (ms).
    pub at_ms: i64,
}

/// The app's file as its start opened it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenedStore {
    pub store: AppStore,
    /// The file that did not read and was set aside, if it did not.
    pub set_aside: Option<SetAside>,
}

/// Which theme the app draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

/// This app's version, as its About screen says it.
#[flutter_rust_bridge::frb(sync)]
pub fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnownFeeder {
    /// The feeder's identity key, hex: what NSD's `id` matches and the
    /// session pins.
    pub identity: String,
    /// The owner's name for it.
    pub name: String,
    /// Where it last answered (`ip:port`), tried while NSD looks.
    pub address: Option<String>,
    /// Where the owner said to dial it (`ip:port`), in place of NSD: for a
    /// feeder reached over a VPN's route to the home network, where NSD's
    /// multicast never arrives. The session pins the identity key, so this
    /// only says where to dial, and a session there never replaces
    /// `address`, which stays where NSD last found it.
    pub manual_address: Option<String>,
}

/// `text` as a feeder address a session dials, `ip:port`: an IP alone takes
/// the device API's port.
#[flutter_rust_bridge::frb(sync)]
pub fn feeder_address(text: &str) -> Result<String, BridgeError> {
    Ok(wellfed_connect::parse_address(text.trim())?.to_string())
}

/// The store's file name in the app's directory.
pub(crate) const FILE_NAME: &str = "wellfed.json";

/// The store at `path` as the app's start reads it: an empty one when the
/// app has never saved, and an empty one too, with the old file set aside,
/// when it does not read. The client key in secure storage and the stock
/// backups are not in this file, so neither is touched. `set_aside` stays
/// in the answer at every start until [`store_notice_read`].
pub fn open_store(path: &str) -> Result<OpenedStore, BridgeError> {
    let fresh = |set_aside: &SetAside| AppFile {
        store: AppStore::default(),
        set_aside: Some(set_aside.clone()),
    };
    let file = match stored::open(Path::new(path), StoreKind::App, fresh)? {
        Opened::Absent => AppFile::default(),
        Opened::Read(file) | Opened::SetAside { value: file, .. } => file,
    };
    Ok(OpenedStore {
        store: file.store,
        set_aside: file.set_aside,
    })
}

/// The owner read the notice of a set-aside `wellfed.json`: the next start
/// says nothing of it.
#[flutter_rust_bridge::frb(sync)]
pub fn store_notice_read(path: &str) -> Result<(), BridgeError> {
    let path = Path::new(path);
    let mut file = file(path)?;
    if file.set_aside.take().is_some() {
        stored::write(path, &file)?;
    }
    Ok(())
}

/// The file at `path`, strictly, or an empty one when nothing was written.
fn file(path: &Path) -> Result<AppFile, BridgeError> {
    Ok(stored::load(path)?.unwrap_or_default())
}

/// The store at `path`, or an empty one when the app has never saved. Strict:
/// a file that does not read is an error, since the start already set aside
/// any it found, and a change must not empty the store behind the owner.
pub fn load_store(path: &str) -> Result<AppStore, BridgeError> {
    Ok(file(Path::new(path))?.store)
}

/// Replace the store at `path`, keeping a notice the owner has not read.
pub fn save_store(path: &str, store: &AppStore) -> Result<(), BridgeError> {
    let path = Path::new(path);
    let file = AppFile {
        store: store.clone(),
        set_aside: file(path)?.set_aside,
    };
    stored::write(path, &file)
}

/// A key as the store and NSD's `id` spell it: lowercase hex.
#[flutter_rust_bridge::frb(sync)]
pub fn identity_hex(identity: &[u8]) -> String {
    wellfed_session::hex::Lower(identity).to_string()
}

/// The first 8 bytes in hex: the fingerprint the feeder logs and the
/// success screen shows.
#[flutter_rust_bridge::frb(sync)]
pub fn fingerprint(identity: &[u8]) -> String {
    wellfed_session::hex::Lower(identity.get(..8).unwrap_or(identity)).to_string()
}

/// The identity key a store entry names, for opening its session.
#[flutter_rust_bridge::frb(sync)]
pub fn identity_bytes(hex: &str) -> Result<Vec<u8>, BridgeError> {
    wellfed_session::hex::decode_array_either_case::<32>(hex.as_bytes())
        .map(|key| key.to_vec())
        .ok_or_else(|| BridgeError::new(ErrorKind::Key, "a feeder identity is 64 hex digits"))
}
