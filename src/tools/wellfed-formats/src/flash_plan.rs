//! Reviewed flash plans: what `wellfed-flash update|give-way prepare`
//! writes and `execute` (and the bench's summaries) read back.
//!
//! A reviewed plan is the session bindings, the inspection it came from and
//! the operation, written as canonical JSON; its SHA-256 is what the operator
//! approves. Every field here is therefore part of a hashed document: a
//! field written as `null` stays a plain `Option`, and only the fields the
//! client adds conditionally are skipped when absent.

use serde::{Deserialize, Serialize};
use wellfed_proto::device::Model;

/// The transaction a plan or binding authorizes. Each names its model, so a
/// plan reviewed against one model's pins never binds a session on another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    #[serde(rename = "plaf103_bank1_development_update")]
    Bank1Update,
    #[serde(rename = "plaf103_bank0_development_update")]
    Bank0Update,
    #[serde(rename = "plaf103_bank1_give_way")]
    GiveWay,
    #[serde(rename = "original_stock_bank1_recovery")]
    StockRecovery,
    /// A candidate image into the fountain's bank 1, the only bank it writes.
    #[serde(rename = "plwf106_bank1_development_update")]
    FountainBank1Update,
    /// Clearing the fountain's bank-1 marker so its loader boots stock bank 0.
    #[serde(rename = "plwf106_bank1_give_way")]
    FountainGiveWay,
    /// The fountain's original bank 1, put back from its pinned backup.
    #[serde(rename = "plwf106_original_bank1_recovery")]
    FountainStockRecovery,
}

impl Kind {
    /// The model whose pins this transaction is planned against.
    pub const fn model(self) -> Model {
        match self {
            Self::Bank1Update | Self::Bank0Update | Self::GiveWay | Self::StockRecovery => {
                Model::Plaf103
            }
            Self::FountainBank1Update | Self::FountainGiveWay | Self::FountainStockRecovery => {
                Model::Plwf106
            }
        }
    }
}

/// The executables a plan was prepared by, by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolHashes {
    #[serde(rename = "wellfed-flash")]
    pub wellfed_flash: String,
}

/// Bindings plus the inspection and operation they authorize.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reviewed<B, P> {
    #[serde(flatten)]
    pub bindings: B,
    pub inspection_report_sha256: String,
    pub operation_plan: P,
}

/// What a development update's inspection and plan are bound to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateBindings {
    pub schema_version: u32,
    pub kind: Kind,
    pub port: String,
    pub interrupted: bool,
    /// Bank 0 without a source review: the full read pinning its contents.
    pub original_read_sha256: Option<String>,
    pub restore_marker: bool,
    pub source_review_sha256: Option<String>,
    pub target_review_sha256: String,
    pub baseline_sha256: String,
    pub rom_report_sha256: String,
    pub bounded_read_report_sha256: String,
    pub tool_sha256: ToolHashes,
    /// A fountain update finished with `interrupted`: the SHA-256 of the
    /// stopped session's reviewed plan, which the finish was checked
    /// against.
    // `default` because only that finish carries the field, not to read an
    // older format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped_plan_sha256: Option<String>,
}

/// What a give-way's inspection and plan are bound to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GiveWayBindings {
    pub schema_version: u32,
    pub kind: Kind,
    pub port: String,
    /// The frozen review of the image bank 0 must hold. `null` for the
    /// fountain, whose fallback is its stock bank 0 as the pinned backup
    /// holds it (`baseline_sha256`).
    pub fallback_review_sha256: Option<String>,
    pub baseline_sha256: String,
    pub rom_report_sha256: String,
    pub bounded_read_report_sha256: String,
    pub tool_sha256: ToolHashes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub offset: u64,
    pub bytes: u64,
}

/// One sector to erase and stage, in write order (the marker sector first).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedSector {
    pub offset: u64,
    pub bytes: u64,
    pub before_sha256: String,
    pub staged_sha256: String,
}

/// A region-scoped development update of one bank.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevelopmentPlan {
    pub schema_version: u32,
    pub kind: Kind,
    pub bank: u8,
    pub interrupted: bool,
    pub source_ota_sha256: Option<String>,
    pub target_ota_sha256: String,
    pub source_payload_bytes: u64,
    pub target_payload_bytes: u64,
    pub region: Region,
    pub current_region_sha256: String,
    pub staged_region_sha256: String,
    pub proposed_region_sha256: String,
    pub ordered_sectors: Vec<PlannedSector>,
    pub erase_sectors: u64,
    pub staging_packets: u64,
    pub commit_packets: u64,
    pub additional_commit_erases: u64,
    pub retained_tail_sha256: String,
    pub scope: String,
    /// The inspected other-bank guard sector, once an inspection pins it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_bank_sha256: Option<String>,
    /// Present (and true) only for a bank-1 marker restore.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub restore_marker: bool,
    /// The fountain only: the bytes sharing an erase block with bank 1
    /// (its FTL), as the inspection read them before any write. Every
    /// later read of them in the session must hash to this.
    // `default` because a PLAF103 plan never carries the field, not to read
    // an older format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neighbors_sha256: Option<String>,
}

/// Clearing bank 1's first marker byte so the loader falls back to bank 0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GiveWayPlan {
    pub schema_version: u32,
    pub kind: Kind,
    pub bank: u8,
    pub offset: u64,
    /// The reviewed fallback OTA image. `null` for the fountain, whose bank 0
    /// must equal its pinned backup's (`bank0_region_sha256`).
    pub fallback_ota_sha256: Option<String>,
    pub bank0_region_sha256: String,
    pub before_sha256: String,
    pub after_sha256: String,
    pub program_frame_sha256: String,
    pub scope: String,
}

/// A reviewed stock-recovery plan as the bench reads it: which bank the
/// pinned original goes back into and how much of it changes. The writer is
/// `wellfed-flash`'s `stock::Reviewed`, which holds much more; its emulated
/// stock-recovery test parses every plan it writes through this view.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StockRecoveryReviewed {
    pub kind: Kind,
    pub recovery_plan: StockRecoveryPlan,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StockRecoveryPlan {
    pub baseline_sha256: String,
    pub target_bank: u8,
    pub changed_sector_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::canonical_of;

    #[test]
    fn conditional_fields_are_absent_and_nulls_stay() {
        let plan = DevelopmentPlan {
            schema_version: 1,
            kind: Kind::Bank0Update,
            bank: 0,
            interrupted: false,
            source_ota_sha256: None,
            target_ota_sha256: "t".into(),
            source_payload_bytes: 0,
            target_payload_bytes: 4096,
            region: Region {
                offset: 0x6000,
                bytes: 4096,
            },
            current_region_sha256: "c".into(),
            staged_region_sha256: "s".into(),
            proposed_region_sha256: "p".into(),
            ordered_sectors: vec![],
            erase_sectors: 0,
            staging_packets: 0,
            commit_packets: 0,
            additional_commit_erases: 0,
            retained_tail_sha256: "r".into(),
            scope: "x".into(),
            other_bank_sha256: None,
            restore_marker: false,
            neighbors_sha256: None,
        };

        let text = String::from_utf8(canonical_of(&plan).unwrap()).unwrap();
        assert!(text.contains("\"kind\":\"plaf103_bank0_development_update\""));
        assert!(text.contains("\"source_ota_sha256\":null"));
        assert!(!text.contains("other_bank_sha256"));
        assert!(!text.contains("restore_marker"));

        let back: DevelopmentPlan = serde_json::from_str(&text).unwrap();
        assert_eq!(back, plan);
    }
}
