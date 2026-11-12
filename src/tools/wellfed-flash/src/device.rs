//! The device a writer works on (`--device`), and the pins it writes that
//! model with.
//!
//! Each model has pins of its own: its backup's hash, the window a bank-1
//! write or restore is bounded to, the ranges no write may touch, its plan
//! kinds and its review kind. The PLAF103's are the research feeder's and
//! stay exactly as they were. The PLWF106 fountain's are narrower: only its
//! bank 1 is written, never the stock bank 0 its loader falls back to, and
//! only within stock bank 0's footprint. A session is pinned to one model
//! before it reads anything, and every check below compares against that
//! model's pins, never a loosened shared one.

use wellfed_formats::flash_plan::Kind;
use wellfed_formats::require;
use wellfed_image::bank::{BANK0, BANK1, Layout, PLWF106_BANK1};
use wellfed_proto::device::Model;

/// Why a writer refuses an operation on the device it was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DeviceError {
    #[error(
        "The {} has no reviewed writes to bank {bank}: only its bank 1 takes a candidate, and bank 0 keeps the stock image its loader falls back to",
        model.name()
    )]
    Bank { model: Model, bank: u8 },
    #[error("Bytes {start:#x}..{end:#x} leave the {}'s bank-1 write window", model.name())]
    OutsideWindow {
        model: Model,
        start: usize,
        end: usize,
    },
    #[error("Bytes {start:#x}..{end:#x} touch the {}'s preserved {name} range", model.name())]
    Preserved {
        model: Model,
        start: usize,
        end: usize,
        name: &'static str,
    },
    #[error(
        "A {} plan or layout cannot bind a {} session",
        layout.name(),
        session.name()
    )]
    OtherModel { layout: Model, session: Model },
    #[error("The {} give-way names no fallback review: its fallback is the pinned stock bank 0", model.name())]
    FallbackReview { model: Model },
    #[error("A give-way of the {} needs --fallback: the frozen review bank 0 must hold", model.name())]
    FallbackMissing { model: Model },
    #[error("The {} takes bank 1's original bytes from its pinned backup, not --original-read", model.name())]
    OriginalRead { model: Model },
}

/// A range no write may touch, as the model's `layout.json` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preserve {
    pub name: &'static str,
    pub start: usize,
    pub end: usize,
}

/// Everything a writer leans on for one model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pins {
    pub model: Model,
    /// SHA-256 of the unit's original 8 MiB backup, the only baseline.
    pub baseline_sha256: &'static str,
    /// Bank 1's window: what stock recovery puts back, and on the fountain
    /// all an update may program. Bank 1 starts at `0x186000` on both.
    pub bank1: (usize, usize),
    /// Every byte any writer may program or erase: both PLAF103 banks, or
    /// the fountain's bank 1 alone.
    pub writes: (usize, usize),
    /// The fountain's stock bank 0, which its give-way requires exactly:
    /// its offset, its length and the backup's hash of it. `None` for the
    /// PLAF103, whose fallback is a frozen review.
    pub stock_bank0: Option<(usize, usize, &'static str)>,
    /// The fountain's original bank-1 window, as its backup holds it.
    pub stock_bank1_sha256: Option<&'static str>,
    pub preserve: &'static [Preserve],
    /// Preserved bytes that share a 64 KiB erase block with bank 1's
    /// window, re-read after every erase in that block and at every
    /// acquisition. The fountain's flash part is unknown, so nothing proves
    /// its sector erase never takes a whole block; its FTL pages at
    /// `0x182000` sit in bank 1's first block. `None` for the PLAF103,
    /// whose GD25Q64E sector erase is reviewed.
    pub neighbors: Option<(usize, usize)>,
    /// The fountain's FTL pages, which stock recovery never rewrites. A
    /// recovery that finds them erased while the backup's hold data refuses:
    /// an erase took them, and they need their own reviewed restore.
    pub ftl: Option<(usize, usize)>,
}

/// The erase unit a part may use beyond its 4 KiB sector.
pub const BLOCK: usize = 0x1_0000;

pub const PLAF103: Pins = Pins {
    model: Model::Plaf103,
    baseline_sha256: crate::link::sector::BASELINE_SHA256,
    bank1: (BANK1.start, BANK1.end),
    writes: (BANK0.start, BANK1.end),
    stock_bank0: None,
    stock_bank1_sha256: None,
    neighbors: None,
    ftl: None,
    preserve: &[
        preserve("pre_application", 0x0, 0x6000),
        preserve("observed_settings_sectors", 0x3e_8000, 0x3f_0000),
        preserve("clock_checkpoint_block", 0x3f_0000, 0x40_0000),
        preserve("observed_audio_start_sector", 0x40_0000, 0x40_1000),
        preserve("observed_log_sector", 0x7c_b000, 0x7c_c000),
        preserve("observed_plan_record_sectors", 0x7d_0000, 0x7d_7000),
        preserve("observed_error_record_sector", 0x7d_a000, 0x7d_b000),
    ],
};

/// The fountain on our bench: two matching full reads, and the minimum
/// preservation map of `docs/devices/plwf106/flash-layout.md`.
pub const PLWF106: Pins = Pins {
    model: Model::Plwf106,
    baseline_sha256: "fe2516e378a67616777a6c86f480f642254a12c726a3f352ff51a1d17966376f",
    bank1: (PLWF106_BANK1.start, PLWF106_BANK1.end),
    writes: (PLWF106_BANK1.start, PLWF106_BANK1.end),
    stock_bank0: Some((
        0x6000,
        0xeb000,
        "453c05d76e03293c3ac993f22ea6b2ead7b8d217451902ce132738bc7b61cf58",
    )),
    stock_bank1_sha256: Some("c390bc2933cdb020f1bac2f361188a96a61e42ddc793059c2aaa31a051758005"),
    // The whole of bank 1's first block below the bank: erased flash, the
    // FTL's three pages and an erased sector. The window's last block,
    // `0x270000..0x280000`, holds only erased flash past `0x271000`, which
    // a block erase would leave as it is.
    neighbors: Some((0x18_0000, 0x18_6000)),
    ftl: Some((0x18_2000, 0x18_5000)),
    preserve: &[
        preserve("pre_application", 0x0, 0x6000),
        preserve("ftl_pages", 0x18_2000, 0x18_5000),
        preserve("settings_and_identity_sectors", 0x3e_8000, 0x3f_0000),
        preserve("clock_checkpoint_block", 0x3f_0000, 0x40_0000),
        preserve("binding_counter_sector", 0x40_0000, 0x40_1000),
        preserve("log_system_and_calibration_sectors", 0x7c_b000, 0x7d_0000),
    ],
};

const fn preserve(name: &'static str, start: usize, end: usize) -> Preserve {
    Preserve { name, start, end }
}

pub const fn pins(model: Model) -> &'static Pins {
    match model {
        Model::Plaf103 => &PLAF103,
        Model::Plwf106 => &PLWF106,
    }
}

/// The development layout of `bank` on `model`. The fountain has bank 1
/// alone.
pub fn layout(model: Model, bank: u8) -> crate::Result<Layout> {
    match model {
        Model::Plaf103 => Ok(wellfed_image::bank::layout(bank)?),
        Model::Plwf106 if bank == 1 => Ok(PLWF106_BANK1),
        Model::Plwf106 => Err(DeviceError::Bank { model, bank }.into()),
    }
}

/// The plan kinds each model's writers record.
pub const fn give_way_kind(model: Model) -> Kind {
    match model {
        Model::Plaf103 => Kind::GiveWay,
        Model::Plwf106 => Kind::FountainGiveWay,
    }
}

pub const fn recovery_kind(model: Model) -> Kind {
    match model {
        Model::Plaf103 => Kind::StockRecovery,
        Model::Plwf106 => Kind::FountainStockRecovery,
    }
}

/// A plan or layout of `layout`'s model, refused in another model's session.
pub fn same_model(layout: Model, session: Model) -> Result<(), DeviceError> {
    require(layout == session, || DeviceError::OtherModel {
        layout,
        session,
    })
}

impl Pins {
    /// A write of `start..end` stays inside the model's writable window and
    /// clear of every preserved range. Each writer also bounds its frames to
    /// its own bank; this is the model's outer limit on top of that.
    pub fn writable(&self, start: usize, end: usize) -> Result<(), DeviceError> {
        let model = self.model;
        require(
            self.writes.0 <= start && start <= end && end <= self.writes.1,
            || DeviceError::OutsideWindow { model, start, end },
        )?;

        match self
            .preserve
            .iter()
            .find(|range| start < range.end && range.start < end)
        {
            Some(range) => Err(DeviceError::Preserved {
                model,
                start,
                end,
                name: range.name,
            }),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use wellfed_formats::layout::{Hex, Layout as Stock, relative_path};

    use super::*;

    fn checked_in(model: Model) -> Stock {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
        wellfed_formats::load(&root.join(relative_path(model))).unwrap()
    }

    fn hex(value: usize) -> Hex {
        Hex(value as u64)
    }

    /// The pins agree with each model's checked-in layout, so neither drifts
    /// without the other, and one model's layout never passes for the other.
    #[test]
    fn each_models_pins_are_its_own_layout() {
        for model in Model::ALL {
            let pins = pins(model);
            let preserve: Vec<_> = pins
                .preserve
                .iter()
                .map(|r| (r.name.to_string(), hex(r.start), hex(r.end)))
                .collect();

            for other in Model::ALL {
                let layout = checked_in(other);
                let listed: Vec<_> = layout
                    .known_preserve_ranges
                    .iter()
                    .map(|r| (r.name.clone(), r.start, r.end_exclusive))
                    .collect();
                let cap = u64::try_from(pins.bank1.1 - pins.bank1.0).unwrap();
                let same = listed == preserve
                    && layout.ota_bank_offsets[1] == hex(pins.bank1.0)
                    && layout.candidate_payload_cap_bytes == cap
                    && layout.model == model;
                assert_eq!(same, other == model, "{model:?} pins, {other:?} layout");
            }
        }
    }

    #[test]
    fn the_fountain_writes_bank_1_alone() {
        assert_eq!(layout(Model::Plwf106, 1).unwrap(), PLWF106_BANK1);
        assert!(matches!(
            layout(Model::Plwf106, 0),
            Err(crate::Error::Device(DeviceError::Bank {
                model: Model::Plwf106,
                bank: 0
            }))
        ));
        assert_eq!(
            layout(Model::Plaf103, 0).unwrap(),
            wellfed_image::bank::BANK0
        );
        assert_eq!(layout(Model::Plaf103, 1).unwrap(), BANK1);
    }

    #[test]
    fn fountain_writes_stay_in_bank_1_and_clear_of_every_preserved_range() {
        assert_eq!(PLWF106.writable(0x18_6000, 0x27_1000), Ok(()));

        // Bank 0, the factory region, the FTL's edge and past the cap.
        for (start, end) in [
            (0x6000, 0x7000),
            (0x0, 0x1000),
            (0x18_5000, 0x18_7000),
            (0x27_0000, 0x27_2000),
        ] {
            assert!(
                matches!(
                    PLWF106.writable(start, end),
                    Err(DeviceError::OutsideWindow { .. })
                ),
                "{start:#x}"
            );
        }

        // Each preserved range refuses a write into it even with no window
        // to stop it first.
        let unbounded = Pins {
            writes: (0, usize::MAX),
            ..PLWF106
        };
        for range in PLWF106.preserve {
            for (start, end) in [
                (range.start, range.end),
                (range.end - 1, range.end),
                (range.start, range.start + 1),
            ] {
                assert_eq!(
                    unbounded.writable(start, end),
                    Err(DeviceError::Preserved {
                        model: Model::Plwf106,
                        start,
                        end,
                        name: range.name
                    })
                );
            }
        }
        assert_eq!(unbounded.writable(0x18_5000, 0x18_6000), Ok(()));

        // The PLAF103's own banks stay writable, and its ranges preserved.
        assert_eq!(PLAF103.writable(BANK0.start, BANK1.end), Ok(()));
        assert!(matches!(
            PLAF103.writable(0x5000, 0x7000),
            Err(DeviceError::OutsideWindow { .. })
        ));
    }

    #[test]
    fn a_plan_of_one_model_never_binds_the_other() {
        assert_eq!(same_model(Model::Plaf103, Model::Plaf103), Ok(()));
        assert_eq!(
            same_model(Model::Plaf103, Model::Plwf106),
            Err(DeviceError::OtherModel {
                layout: Model::Plaf103,
                session: Model::Plwf106
            })
        );
        assert_eq!(give_way_kind(Model::Plwf106).model(), Model::Plwf106);
        assert_eq!(recovery_kind(Model::Plaf103), Kind::StockRecovery);
        assert_eq!(PLWF106_BANK1.kind.model(), Model::Plwf106);
    }
}
