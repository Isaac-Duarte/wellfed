//! Why an install is refused before it starts.

use wellfed_api::ErrorCode;
use wellfed_model::{MaybeKnown, Model};
use wellfed_releases::STOCK_MODEL;

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
    OtherModel {
        release: MaybeKnown<Model>,
        device: Model,
    },
    /// A return to stock is asked of a device of another model than the
    /// one the stock image is built for.
    #[error(
        "a return to stock is for a {} only, and the device is a {}",
        STOCK_MODEL.name(),
        device.name()
    )]
    StockModelOnly { device: Model },
}

impl InstallError {
    pub(crate) fn code(self) -> ErrorCode {
        match self {
            Self::Running => ErrorCode::InstallRunning,
            Self::RestoreMismatch(_) => ErrorCode::RestoreMismatch,
            Self::StockBackupNeeded => ErrorCode::StockBackupNeeded,
            Self::OtherModel { .. } | Self::StockModelOnly { .. } => ErrorCode::OtherModel,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fountain asked to return to stock hears that stock is the
    /// PLAF103's, not that a fountain's restore is for another model.
    #[test]
    fn a_return_to_stock_names_the_one_model_it_is_for() {
        let error = InstallError::StockModelOnly {
            device: Model::Plwf106,
        };

        assert_eq!(error.code(), ErrorCode::OtherModel);
        assert_eq!(
            error.to_string(),
            format!(
                "a return to stock is for a {} only, and the device is a {}",
                Model::Plaf103.name(),
                Model::Plwf106.name()
            )
        );
    }
}
