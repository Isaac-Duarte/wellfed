//! The files one host tool writes and another reads, typed once.
//!
//! Build manifests, device configs, the stock layout, the SDK and toolchain locks,
//! the credential and key files and the flash client's reviewed plans
//! each cross crate boundaries (the builder writes what the audit, flash review
//! and bench read; the bench summarizes the flash plans), so their shape lives
//! here rather than as `Value` lookups in every reader. The relay bench's own
//! protocol, which only host tools speak, is `wellfed-bench-formats`.
//!
//! Whatever is hashed goes through [`canonical`], which fixes the bytes
//! independently of field order; other files are written in declaration
//! order and read as objects.

#![forbid(unsafe_code)]

pub mod audit;
pub mod canonical;
#[cfg(feature = "clap")]
pub mod cli;
pub mod credentials;
pub mod device;
#[cfg(feature = "export")]
pub mod export;
pub mod flash_plan;
pub mod kept;
pub mod layout;
pub mod lock;
pub mod manifest;
pub mod owner;
pub mod private;
pub mod review;
pub mod stock;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Canonical JSON carries integers only")]
    NotInteger,
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("the system's random source failed: {0}")]
    Random(#[from] getrandom::Error),
}

/// Read a JSON file into its type.
pub fn load<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Result<T> {
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

pub type Result<T> = std::result::Result<T, Error>;

/// Root-relative: the bench's evidence of a session, among it the manifests
/// it signed, which `wellfed-signing` reads for the versions signed so far.
pub const VERIFICATION: &str = "private/verification";
/// Root-relative: the owner's stock backups, one directory per feeder,
/// where a run that signed its own promotion also keeps it.
pub const STOCK_BACKUPS: &str = "private/stock-backups";

/// The guard every host tool's checks go through: `Ok` when `condition`
/// holds, else the typed refusal `error` builds. Built lazily, so a refusal
/// that formats its fields costs nothing when the check passes.
pub fn require<E>(condition: bool, error: impl FnOnce() -> E) -> std::result::Result<(), E> {
    if condition { Ok(()) } else { Err(error()) }
}
