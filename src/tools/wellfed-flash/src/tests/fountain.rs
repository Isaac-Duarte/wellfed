//! The PLWF106 fountain's writes against the modeled helper: a candidate
//! into bank 1 from its pinned original bytes, a give-way to stock bank 0,
//! and stock recovery of bank 1 from the pinned backup. Each is bounded to
//! the fountain's bank-1 window and refuses the feeder's pins.

use wellfed_image::bank::PLWF106_BANK1;
use wellfed_proto::device::Model;

use super::*;
use crate::device::{self, DeviceError};
use crate::writers::development::DevelopmentError;
use crate::writers::recovery::{self, RecoveryError};

/// Bank 1's end on the fountain: stock bank 0's rounded footprint past it.
const END: usize = 0x27_1000;
/// Our unit's bank 1 starts with an altered marker, so stock boots bank 0.
const ALTERED: &[u8; 8] = b"\x00\x01\x00\x00\x001 \x00";

/// A synthetic fountain: a valid stock image in bank 0, bank 1's marker
/// altered as on our unit, and settings drift every write must preserve.
fn fountain() -> Unit {
    let mut baseline = noise(CAPACITY, 11);
    let stock = development::payload(&image(40000, 5), &BANK0)
        .unwrap()
        .to_vec();
    baseline[0x6000..0x6000 + stock.len()].copy_from_slice(&stock);
    baseline[0x6000 + stock.len()..0xf_1000].fill(0xff);
    baseline[START..START + 8].copy_from_slice(ALTERED);

    let mut flash = baseline.clone();
    flash[0x3e_9000] ^= 1;
    flash[0x3f_f000] ^= 1;
    Unit {
        baseline: Baseline::synthetic_for(Model::Plwf106, baseline),
        flash,
    }
}

/// An update that installs `target` over the pinned original bank 1.
fn install<'a>(unit: &Unit, target: &'a [u8], original: &'a mut Vec<u8>) -> Update<'a> {
    *original =
        development::original_region(unit.baseline.bytes(), target, &PLWF106_BANK1).unwrap();
    Update {
        source: None,
        target,
        interrupted: false,
        layout: PLWF106_BANK1,
        original: Some(original),
        restore: false,
    }
}

/// Install a candidate, give way to stock bank 0, then put the original
/// bank 1 back: every step inside the window, everything else unchanged.
#[test]
fn a_candidate_installs_gives_way_and_is_recovered_inside_bank_1() {
    let (_checkout, root) = private_root();
    let unit = fountain();
    let target = image(30000, 3);
    let mut original = Vec::new();
    let update = install(&unit, &target, &mut original);
    let mut port = Fake::new(unit.flash.clone());

    develop(&root, &unit, &mut port, &update, "install").unwrap();
    assert!(receipt(&root, "install").verified_terminal);

    let payload = development::payload(&target, &PLWF106_BANK1).unwrap();
    let mut installed = unit.flash.clone();
    installed[START..START + payload.len()].copy_from_slice(payload);
    assert!(port.0.borrow().flash == installed, "bank 1's target only");
    assert!(
        port.0
            .borrow()
            .erased
            .iter()
            .all(|at| (START..END).contains(at)),
        "no erase outside bank 1"
    );

    // A bootable bank is never rewritten in place: it gives way first.
    assert!(matches!(
        recovery::plan(&unit.baseline, &installed),
        Err(crate::Error::Recovery(RecoveryError::MarkerStillValid))
    ));

    let bank0 = installed[0x6000..0xf_1000].to_vec();
    let first = installed[START..START + SECTOR].to_vec();
    let plan = development::pinned_give_way_plan(&unit.baseline, &bank0, &first).unwrap();
    assert_eq!(
        plan.kind,
        wellfed_formats::flash_plan::Kind::FountainGiveWay
    );
    assert_eq!(plan.fallback_ota_sha256, None);

    let mut acquire = |port: &mut dyn Port| -> crate::Result<[Vec<u8>; 3]> {
        Ok([
            protocol::read_region(port, 0, 0x6000)?,
            protocol::read_region(port, 0x6000, 0xeb000)?,
            protocol::read_region(port, START, SECTOR)?,
        ])
    };
    let give_way = development::PinnedGiveWay {
        baseline: &unit.baseline,
        bank0: &bank0,
        bank1_sector: &first,
        reviewed: &plan,
    };
    port.0.borrow_mut().erased.clear();
    development::run_pinned_give_way(
        &mut port,
        &root,
        &give_way,
        &mut acquire,
        &root.join("private/give-way"),
    )
    .unwrap();
    assert_eq!(port.0.borrow().flash[START], 0);
    assert!(port.0.borrow().erased.is_empty(), "a give-way never erases");

    // Stock recovery puts the backup's bank 1 back, its altered marker last.
    let current = port.0.borrow().flash.clone();
    let plan = recovery::plan(&unit.baseline, &current).unwrap();
    assert_eq!(plan.target_range.end_exclusive, END);
    assert_eq!(
        plan.ordered_sector_payloads.last().map(|s| s.offset),
        Some(START)
    );

    let mut snapshot = |port: &mut dyn Port| protocol::read_region(port, 0, CAPACITY);
    recovery::run_transaction(
        &mut port,
        &root,
        &unit.baseline,
        &current,
        &serde_json::to_value(&plan).unwrap(),
        &mut snapshot,
        &root.join("private/recovery"),
    )
    .unwrap();
    assert!(
        port.0.borrow().flash == unit.flash,
        "stock again, drift kept"
    );
    assert!(
        port.0
            .borrow()
            .erased
            .iter()
            .all(|at| (START..END).contains(at))
    );
    assert!(receipt(&root, "recovery").verified_terminal);
}

#[test]
fn an_image_past_the_fountains_cap_is_refused() {
    let oversize = wrap(&synthetic_payload(9000, 0xeb000, 1));

    // It fits the feeder's larger bank, so only the fountain's cap stops it.
    assert!(development::payload(&oversize, &BANK1).is_ok());
    assert!(matches!(
        development::payload(&oversize, &PLWF106_BANK1),
        Err(wellfed_image::Error::Bank(
            wellfed_image::bank::BankError::Footprint { bank: 1 }
        ))
    ));
}

/// No fountain frame reaches bank 0, the factory region, the FTL below
/// bank 1 or anything past the cap.
#[test]
fn fountain_frames_stay_in_its_bank_1_window() {
    let data = [0u8; 1024];
    let frame = |offset| development::frame(&PLWF106_BANK1, 1, offset, &data);
    assert!(frame(START).is_ok());
    assert!(frame(END - 1024).is_ok());

    for offset in [0x6000, 0x0, 0x18_4000, END, 0x3e_9000, 0x7c_e000] {
        assert!(frame(offset).is_err(), "{offset:#x}");
    }

    let fountain = fountain();
    let mut port = Fake::new(fountain.flash.clone());
    let mut journal = |_: crate::evidence::journal::At| Ok(());
    assert!(matches!(
        sector::restore_sector(
            &mut port,
            &fountain.baseline,
            END,
            &sha256(&fountain.flash[END..END + SECTOR]),
            &mut journal,
        ),
        Err(crate::Error::Sector(sector::SectorError::SectorOutside {
            offset: END
        }))
    ));
    assert!(port.0.borrow().erased.is_empty());
}

#[test]
fn a_backup_that_is_not_the_fountains_pin_is_refused() {
    for data in [vec![0xff; CAPACITY], noise(CAPACITY, 11)] {
        assert!(matches!(
            Baseline::pinned_for(Model::Plwf106, data),
            Err(crate::Error::Sector(
                sector::SectorError::UnpinnedBaseline { bytes: CAPACITY }
            ))
        ));
    }
    assert_ne!(
        device::PLWF106.baseline_sha256,
        device::PLAF103.baseline_sha256
    );
}

/// The feeder's layouts never bind a fountain session, nor the fountain's
/// a feeder session, before anything is written.
#[test]
fn one_models_layout_is_refused_for_the_others_session() {
    let fountain = fountain();
    let feeder = unit(development::payload(&image(30000, 3), &BANK1).unwrap());
    let factory = |unit: &Unit| unit.flash[..0x6000].to_vec();
    let other = |unit: &Unit| unit.flash[0x6000..0x7000].to_vec();
    let other_model = |result: crate::Result<()>| {
        matches!(
            result,
            Err(crate::Error::Device(DeviceError::OtherModel { .. }))
        )
    };

    for layout in [BANK1, BANK0] {
        let guards = development::guards(
            &fountain.baseline,
            &factory(&fountain),
            &other(&fountain),
            &layout,
            None,
        );
        assert!(other_model(guards), "{layout:?}");
    }
    let guards = development::guards(
        &feeder.baseline,
        &factory(&feeder),
        &other(&feeder),
        &PLWF106_BANK1,
        None,
    );
    assert!(other_model(guards));

    // A fountain install run against the feeder's baseline stops before its
    // journal exists.
    let (_checkout, root) = private_root();
    let target = image(30000, 3);
    let mut original = Vec::new();
    let update = install(&fountain, &target, &mut original);
    let mut port = Fake::new(fountain.flash.clone());
    assert!(matches!(
        develop(&root, &feeder, &mut port, &update, "crossed"),
        Err(crate::Error::Device(DeviceError::OtherModel {
            layout: Model::Plwf106,
            session: Model::Plaf103
        }))
    ));
    assert!(!root.join("private/crossed").exists());
    assert!(port.0.borrow().erased.is_empty());
}

/// Nothing writes the fountain's bank 0, so an install refuses one that is
/// not exactly stock, even with a valid marker.
#[test]
fn a_fountain_install_needs_stock_bank_0() {
    let unit = fountain();
    let mut other = unit.flash[0x6000..0x7000].to_vec();
    let factory = unit.flash[..0x6000].to_vec();
    assert!(development::guards(&unit.baseline, &factory, &other, &PLWF106_BANK1, None).is_ok());

    other[100] ^= 1;
    assert_eq!(&other[..8], MARKER);
    assert!(matches!(
        development::guards(&unit.baseline, &factory, &other, &PLWF106_BANK1, None),
        Err(crate::Error::Development(DevelopmentError::Bank0NotStock))
    ));

    // The give-way requires all of stock bank 0, not only its first sector.
    let mut bank0 = unit.flash[0x6000..0xf_1000].to_vec();
    bank0[0x9_0000] ^= 1;
    let mut first = unit.flash[START..START + SECTOR].to_vec();
    first[..8].copy_from_slice(MARKER);
    assert!(matches!(
        development::pinned_give_way_plan(&unit.baseline, &bank0, &first),
        Err(crate::Error::Development(DevelopmentError::FallbackDiffers))
    ));

    // And the feeder has no pinned stock bank 0 to give way to.
    let feeder = super::unit(development::payload(&image(30000, 3), &BANK1).unwrap());
    assert!(matches!(
        development::pinned_give_way_plan(&feeder.baseline, &bank0, &first),
        Err(crate::Error::Device(DeviceError::FallbackMissing {
            model: Model::Plaf103
        }))
    ));
}
