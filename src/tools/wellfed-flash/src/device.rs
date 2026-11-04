//! The device a writer works on (`--device`), and whether this client
//! holds the pins to write it.
//!
//! Every pin a writer leans on is a PLAF103's: the stock backup's and the
//! original bank's hashes, the reviewed plans' names (`plaf103_*`), the
//! bank map and the loader's marker sequence. So a writer refuses any other
//! model before it opens the port, rather than write one board with
//! another's pins. A second model gets pins of its own here, under new
//! names; the PLAF103's stay exactly as they are.

use wellfed_proto::device::Model;

/// Why a writer refuses the device it was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DeviceError {
    #[error(
        "This client holds no reviewed flash pins for the {}: its writers serve the plaf103 alone",
        model.name()
    )]
    Unpinned { model: Model },
}

/// Refuse a model whose flash this client has no pins for.
pub fn writable(model: Model) -> Result<(), DeviceError> {
    match model {
        Model::Plaf103 => Ok(()),
        Model::Plwf106 => Err(DeviceError::Unpinned { model }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_plaf103_is_written() {
        assert_eq!(writable(Model::Plaf103), Ok(()));
        assert_eq!(
            writable(Model::Plwf106),
            Err(DeviceError::Unpinned {
                model: Model::Plwf106
            })
        );
    }
}
