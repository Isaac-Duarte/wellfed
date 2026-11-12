//! The OTA container and placement audit of a candidate image against the
//! verified stock layout (port of `src/tools/audit/audit-ota-layout.py`).
//!
//! Structural and known-range only: not a flash plan or full partition
//! validation, and boot acceptance stays unproven.
//!
//! The PLWF106 has no stock OTA image, only a flash backup, so its audit
//! checks the candidate against its layout alone: the container, the
//! layout's size cap and bank-1 placement clear of every preserved range.
//! Its candidate goes into bank 1 only; bank 0 keeps the stock image.

use std::fs;
use std::path::Path;

use wellfed_formats::audit::{LayoutAudit, LayoutChecks, LayoutRegion, Placement};
use wellfed_formats::layout::{Hex, Layout};
use wellfed_image::ota::{STOCK_SHA256, container, images};
use wellfed_proto::device::Model;

use super::word;
use crate::{Result, sha256};
use wellfed_formats::require;

/// Why the stock layout, its reference image or a candidate was refused.
#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error("Unsupported layout/reference identity")]
    Identity,
    #[error("Stock reference is not the verified PLAF103 1.1.15 image")]
    Reference,
    #[error("Invalid bank offsets")]
    BankOffsets,
    #[error("Layout bank offsets do not match the verified stock table")]
    BankTable,
    #[error("Unsupported candidate size cap")]
    SizeCap,
    #[error("Candidate exceeds the known stock reference footprint")]
    Footprint,
    #[error("Candidate crosses the next bank start")]
    CrossesBank,
    #[error("Unsupported sector alignment")]
    Alignment,
    #[error("Candidate overlaps known preserved data: {ranges:?}")]
    Preserved { ranges: Vec<String> },
}

/// The candidate `image` against `model`'s layout, with the PLAF103's stock
/// OTA `stock` as its reference.
pub(crate) fn audit(
    image: &Path,
    stock: &Path,
    layout: &Path,
    model: Model,
) -> Result<LayoutAudit> {
    match model {
        Model::Plaf103 => feeder(image, stock, layout),
        Model::Plwf106 => fountain(image, layout),
    }
}

fn feeder(image: &Path, stock: &Path, layout: &Path) -> Result<LayoutAudit> {
    let layout: Layout = wellfed_formats::load(layout)?;
    require(
        layout.schema_version == 1
            && layout.model == Model::Plaf103
            && layout.stock_sha256.as_deref() == Some(STOCK_SHA256),
        || LayoutError::Identity,
    )?;
    let Some(table_address) = layout.bank_table_runtime_address else {
        return Err(LayoutError::Identity.into());
    };

    let stock = fs::read(stock)?;
    require(
        layout.stock_bytes == Some(stock.len() as u64) && sha256(&stock) == STOCK_SHA256,
        || LayoutError::Reference,
    )?;
    let reference = container(&stock)?;

    let offsets: Vec<u64> = layout.ota_bank_offsets.iter().map(|o| o.0).collect();
    require(offsets.len() == 2 && offsets[0] < offsets[1], || {
        LayoutError::BankOffsets
    })?;
    let mapped = layout.flash_mapping_base.0;
    let table = (0x17040 + table_address.0)
        .checked_sub(0x0E00_0020)
        .and_then(|table| usize::try_from(table).ok())
        .unwrap_or(usize::MAX);
    require(
        word(&stock, table) == Some(mapped + offsets[0])
            && word(&stock, table + 4) == Some(mapped + offsets[1]),
        || LayoutError::BankTable,
    )?;
    require(
        layout.candidate_payload_cap_bytes == reference.len() as u64,
        || LayoutError::SizeCap,
    )?;

    require(fs::metadata(image)?.len() <= stock.len() as u64, || {
        LayoutError::Footprint
    })?;
    let data = fs::read(image)?;
    let payload = container(&data)?;
    require(payload.len() <= reference.len(), || LayoutError::Footprint)?;
    require(payload.len() as u64 <= offsets[1] - offsets[0], || {
        LayoutError::CrossesBank
    })?;
    require(
        layout.sector_bytes == 4096 && payload.len() % 4096 == 0,
        || LayoutError::Alignment,
    )?;
    let image_regions = images(payload)?;

    let mut placements = Vec::new();
    for (bank_index, start) in offsets.iter().enumerate() {
        let end = start + payload.len() as u64;
        let ranges: Vec<String> = layout
            .known_preserve_ranges
            .iter()
            .filter(|range| *start < range.end_exclusive.0 && range.start.0 < end)
            .map(|range| range.name.clone())
            .collect();
        require(ranges.is_empty(), || LayoutError::Preserved { ranges })?;
        placements.push(Placement {
            bank_index,
            flash_offset: Hex(*start),
            end_exclusive: Hex(end),
            erase_bytes: payload.len(),
        });
    }

    Ok(LayoutAudit {
        schema_version: 1,
        image: image.display().to_string(),
        bytes: data.len(),
        sha256: sha256(&data),
        payload_bytes: payload.len(),
        regions: image_regions
            .into_iter()
            .map(|region| LayoutRegion {
                name: region.name.into(),
                payload_header_offset: region.payload_header_offset,
                body_bytes: region.body_bytes,
                runtime_address: region.runtime_address,
            })
            .collect(),
        candidate_placements: placements,
        checks: LayoutChecks {
            container_and_checksum: true,
            reference_table_matches: true,
            known_preserve_ranges_untouched: true,
        },
        boot_acceptance: "unproven".into(),
        physical_flash_capacity: "unverified".into(),
        scope:
            "Offline structural/known-range audit; not a flash plan or full partition validation"
                .into(),
    })
}

/// The fountain's candidate: within its layout's cap, sector-aligned, and in
/// bank 1 clear of every preserved range. Nothing here compares a stock
/// table: there is no stock OTA, and the factory loader's bank table is the
/// PLAF103's, checked by the flash writer against the backup.
fn fountain(image: &Path, layout: &Path) -> Result<LayoutAudit> {
    let layout: Layout = wellfed_formats::load(layout)?;
    require(
        layout.schema_version == 1
            && layout.model == Model::Plwf106
            && layout.stock_sha256.is_none(),
        || LayoutError::Identity,
    )?;
    let offsets: Vec<u64> = layout.ota_bank_offsets.iter().map(|o| o.0).collect();
    let [bank0, bank1] = offsets[..] else {
        return Err(LayoutError::BankOffsets.into());
    };
    require(bank0 < bank1, || LayoutError::BankOffsets)?;

    let data = fs::read(image)?;
    let payload = container(&data)?;
    require(
        payload.len() as u64 <= layout.candidate_payload_cap_bytes,
        || LayoutError::Footprint,
    )?;
    require(
        layout.sector_bytes == 4096 && payload.len() % 4096 == 0,
        || LayoutError::Alignment,
    )?;
    let image_regions = images(payload)?;

    let end = bank1 + payload.len() as u64;
    let ranges: Vec<String> = layout
        .known_preserve_ranges
        .iter()
        .filter(|range| bank1 < range.end_exclusive.0 && range.start.0 < end)
        .map(|range| range.name.clone())
        .collect();
    require(ranges.is_empty(), || LayoutError::Preserved { ranges })?;

    Ok(LayoutAudit {
        schema_version: 1,
        image: image.display().to_string(),
        bytes: data.len(),
        sha256: sha256(&data),
        payload_bytes: payload.len(),
        regions: image_regions
            .into_iter()
            .map(|region| LayoutRegion {
                name: region.name.into(),
                payload_header_offset: region.payload_header_offset,
                body_bytes: region.body_bytes,
                runtime_address: region.runtime_address,
            })
            .collect(),
        candidate_placements: vec![Placement {
            bank_index: 1,
            flash_offset: Hex(bank1),
            end_exclusive: Hex(end),
            erase_bytes: payload.len(),
        }],
        checks: LayoutChecks {
            container_and_checksum: true,
            reference_table_matches: false,
            known_preserve_ranges_untouched: true,
        },
        boot_acceptance: "unproven".into(),
        physical_flash_capacity: "unverified".into(),
        scope: "Offline structural/known-range audit of a bank-1 candidate against the \
                PLWF106 layout's cap; no stock OTA to compare, not a flash plan"
            .into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    /// The fountain's own layout, its model swapped: refused for its model
    /// alone, before the image is read.
    #[test]
    fn the_fountain_audit_refuses_another_models_layout() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
        let path = root.join(wellfed_formats::layout::relative_path(Model::Plwf106));
        let mut layout: Layout = wellfed_formats::load(&path).unwrap();
        layout.model = Model::Plaf103;
        let swapped = std::env::temp_dir().join(format!("layout-{}.json", std::process::id()));
        fs::write(&swapped, serde_json::to_vec(&layout).unwrap()).unwrap();

        let refused = fountain(Path::new("/nonexistent"), &swapped);
        fs::remove_file(&swapped).unwrap();
        assert!(
            matches!(refused, Err(Error::LayoutAudit(LayoutError::Identity))),
            "{refused:?}"
        );
    }
}
