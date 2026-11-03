//! What a device is: its kind, which picks the typed module of the device
//! API it serves, and its model, which picks its board, pins and images.
//!
//! The session's welcome carries both ([`crate::api::Welcome`]), so a
//! client knows which module to speak before its first request. The
//! discriminants are fixed: a record or report carries [`DeviceKind::code`]
//! or [`Model::code`], and a byte read back becomes a variant only through
//! `from_byte`. The wire's serde form is postcard's variant index, which
//! follows the declaration order, so a new variant goes at the end.

use serde::{Deserialize, Serialize};

/// The kind of device: the shared core of the device API plus one typed
/// module per kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum DeviceKind {
    Feeder = 1,
    Fountain = 2,
    /// No model yet: the third kind keeps the shape honest.
    LitterBox = 3,
}

impl DeviceKind {
    pub const ALL: [Self; 3] = [Self::Feeder, Self::Fountain, Self::LitterBox];

    pub const fn code(self) -> u8 {
        self as u8
    }

    pub const fn from_byte(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Feeder),
            2 => Some(Self::Fountain),
            3 => Some(Self::LitterBox),
            _ => None,
        }
    }

    /// The name tools and records print: serde's spelling, so a JSON
    /// field and a log line read alike.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Feeder => "feeder",
            Self::Fountain => "fountain",
            Self::LitterBox => "litter_box",
        }
    }
}

/// A device model Wellfed has evidence for: one board, one firmware crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum Model {
    /// PETLIBRO's PLAF103 feeder.
    Plaf103 = 1,
    /// PETLIBRO's PLWF106 water fountain.
    Plwf106 = 2,
}

impl Model {
    pub const ALL: [Self; 2] = [Self::Plaf103, Self::Plwf106];

    pub const fn code(self) -> u8 {
        self as u8
    }

    pub const fn from_byte(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Plaf103),
            2 => Some(Self::Plwf106),
            _ => None,
        }
    }

    pub const fn kind(self) -> DeviceKind {
        match self {
            Self::Plaf103 => DeviceKind::Feeder,
            Self::Plwf106 => DeviceKind::Fountain,
        }
    }

    /// The name tools and records print, as the vendor writes it in
    /// lower case.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Plaf103 => "plaf103",
            Self::Plwf106 => "plwf106",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode, encode};

    #[test]
    fn every_code_reads_back() {
        for kind in DeviceKind::ALL {
            assert_eq!(DeviceKind::from_byte(kind.code()), Some(kind));
        }
        for model in Model::ALL {
            assert_eq!(Model::from_byte(model.code()), Some(model));
        }
        assert_eq!(DeviceKind::from_byte(0), None);
        assert_eq!(Model::from_byte(0), None);
        assert_eq!(Model::from_byte(3), None);
    }

    #[test]
    fn each_model_has_its_kind() {
        assert_eq!(Model::Plaf103.kind(), DeviceKind::Feeder);
        assert_eq!(Model::Plwf106.kind(), DeviceKind::Fountain);
    }

    /// `name` is the serde spelling.
    #[test]
    fn names_are_the_serde_spelling() {
        assert_eq!(DeviceKind::LitterBox.name(), "litter_box");
        assert_eq!(Model::Plwf106.name(), "plwf106");
    }

    /// The wire carries the declaration index, one byte each.
    #[test]
    fn the_wire_form_is_one_byte() {
        let mut out = [0; 2];
        assert_eq!(encode(&DeviceKind::LitterBox, &mut out), Ok(1));
        assert_eq!(out[0], 2);
        assert_eq!(decode::<Model>(&[1]), Ok(Model::Plwf106));
    }
}
