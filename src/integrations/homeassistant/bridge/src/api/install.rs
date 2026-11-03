//! A firmware release an owner may install, and how an install went.

use super::enums::{InstallPhase, Installed, NotSent, Signer};

/// A verified release held for an install.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(frozen, get_all, eq, module = "wellfed_homeassistant")
)]
pub struct UpdateOffer {
    /// SHA-256 over the signed manifest, hex: the release's name.
    pub id: String,
    pub version: u32,
    /// The build it carries, hex, as the feeder's status names its own.
    pub build: String,
    pub signer: Signer,
    /// The project release's tag, for one fetched from GitHub.
    pub tag: Option<String>,
}

/// Where an install stands, as the feeder answered it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(frozen, get_all, eq, module = "wellfed_homeassistant")
)]
pub struct InstallProgress {
    pub phase: InstallPhase,
    /// Bytes written and read back so far.
    pub written: u32,
    pub image_len: u32,
}

impl From<&wellfed_model::UpdateView> for InstallProgress {
    fn from(view: &wellfed_model::UpdateView) -> Self {
        Self {
            phase: view.phase.into(),
            written: view.written,
            image_len: view.image_len,
        }
    }
}

/// How an install ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(frozen, get_all, eq, module = "wellfed_homeassistant")
)]
pub struct Install {
    pub result: Installed,
    /// Whether the offer went out: after it, bank 1 may hold part of the
    /// release.
    pub offered: bool,
    pub not_sent: Option<NotSent>,
}

impl Install {
    pub fn not_sent(why: NotSent) -> Self {
        Self {
            result: Installed::NotSent,
            offered: false,
            not_sent: Some(why),
        }
    }
}
