//! Why an install is refused before it starts.

use wellfed_api::ErrorCode;
use wellfed_model::Model;

/// Installs, one at a time per server, a restore of stock among them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InstallError {
    #[error("another install is running: one runs at a time")]
    Running,
    #[error("the restore does not offer the kept backup: {0}")]
    RestoreMismatch(wellfed_releases::RestoreError),
    #[error(
        "the feeder runs from bank 0 with stock in bank 1, and no verified backup of it is kept: \
         read it from the feeder or upload the installer's copy before installing"
    )]
    StockBackupNeeded,
    /// The release is built for another model than the device's: nothing
    /// is offered.
    #[error("the release is built for a {}, and the device is a {}", release.name(), device.name())]
    OtherModel { release: Model, device: Model },
}

impl InstallError {
    pub(crate) fn code(self) -> ErrorCode {
        match self {
            Self::Running => ErrorCode::InstallRunning,
            Self::RestoreMismatch(_) => ErrorCode::RestoreMismatch,
            Self::StockBackupNeeded => ErrorCode::StockBackupNeeded,
            Self::OtherModel { .. } => ErrorCode::OtherModel,
        }
    }
}
