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
use wellfed_formats::flash_plan::{Kind, Reviewed, ToolHashes, UpdateBindings};

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

/// The fountain's part is unknown, so an erase that takes its whole 64 KiB
/// block would wipe the FTL pages beside bank 1. The install and the
/// restore re-read them after the erase and stop there, without a program
/// frame or a retry; a healthy install journals that it checked.
#[test]
fn an_erase_that_takes_the_ftl_with_it_stops_the_session() {
    let (_checkout, root) = private_root();
    let unit = fountain();
    let target = image(30000, 3);
    let mut original = Vec::new();
    let update = install(&unit, &target, &mut original);

    let mut port = Fake::new(unit.flash.clone());
    develop(&root, &unit, &mut port, &update, "healthy").unwrap();
    let journal = fs::read_to_string(root.join("private/healthy/journal.jsonl")).unwrap();
    assert!(journal.contains("\"event\":\"neighbors_verified\""));

    let mut port = Fake::new(unit.flash.clone());
    port.0.borrow_mut().fault = Fault::BlockErase;
    let error = develop(&root, &unit, &mut port, &update, "block").unwrap_err();
    assert!(
        matches!(
            error,
            crate::Error::Sector(sector::SectorError::NeighborsDiffer {
                start: 0x18_0000,
                end: START
            })
        ),
        "{error}"
    );
    assert_eq!(port.0.borrow().erases, 1, "nothing after the first erase");
    assert_eq!(port.0.borrow().frames, 0, "no program frame");
    assert!(!receipt(&root, "block").verified_terminal);

    // Stock recovery's sector erase is checked the same way.
    let mut current = unit.flash.clone();
    current[START..START + 8].fill(0);
    current[START + 0x100] ^= 1;
    let mut port = Fake::new(current);
    port.0.borrow_mut().fault = Fault::BlockErase;
    let before = sha256(&port.0.borrow().flash[START..START + SECTOR]);
    let neighbors = sha256(&port.0.borrow().flash[0x18_0000..START]);
    let mut journal = |_: crate::evidence::journal::At| Ok(());
    assert!(matches!(
        sector::restore_sector(
            &mut port,
            &unit.baseline,
            START,
            &before,
            Some(&neighbors),
            &mut journal
        ),
        Err(crate::Error::Sector(
            sector::SectorError::NeighborsDiffer { .. }
        ))
    ));
    assert_eq!(port.0.borrow().frames, 0);
}

/// Stock's FTL is live: it changes between the backup and a session, so
/// the check holds it to the session's own read, not the backup's. A plan
/// that binds no read, or a stale one (stock ran, or an earlier session's
/// erase took the FTL), stops before the first erase.
#[test]
fn the_ftl_is_held_to_the_plans_read_not_the_backups() {
    let (_checkout, root) = private_root();
    let unit = fountain();
    let target = image(30000, 3);
    let mut original = Vec::new();
    let update = install(&unit, &target, &mut original);

    // Stock garbage-collected its FTL after the backup was taken.
    let mut flash = unit.flash.clone();
    flash[0x18_3000..0x18_4000].fill(0x5a);
    let mut port = Fake::new(flash.clone());
    develop(&root, &unit, &mut port, &update, "live").unwrap();
    assert!(port.0.borrow().flash[0x18_0000..START] == flash[0x18_0000..START]);

    // A plan without the read, and one with a stale read.
    let length = development::extent(None, &target, &PLWF106_BANK1).unwrap();
    let current = flash[START..START + length].to_vec();
    let other = flash[0x6000..0x7000].to_vec();
    assert!(matches!(
        development::plan(&update, &current, Some(&other)),
        Err(crate::Error::Development(
            DevelopmentError::NeighborsUnbound
        ))
    ));

    let stale = sha256(&unit.flash[0x18_0000..START]);
    let plan = development::plan_bound(&update, &current, Some(&other), Some(stale)).unwrap();
    let mut port = Fake::new(flash);
    let mut acquire = acquire_for(PLWF106_BANK1, length);
    let transaction = development::Transaction {
        baseline: &unit.baseline,
        update: &update,
        current: &current,
        other: Some(&other),
        reviewed: &plan,
    };
    let error = development::run(
        &mut port,
        &root,
        &transaction,
        &mut acquire,
        &root.join("private/stale"),
        &mut |_, _| {},
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            crate::Error::Sector(sector::SectorError::NeighborsDiffer { .. })
        ),
        "{error}"
    );
    assert_eq!(port.0.borrow().erases, 0, "stopped before any erase");
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
            Some(&sha256(&fountain.flash[0x18_0000..START])),
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
    first[..8].copy_from_slice(&MARKER);
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

/// A stopped install of `unit`, under `fault`, as its session directory
/// holds it: the journal in `transaction/`, the reviewed plan beside it.
/// Returns the plan file's bindings.
fn stopped_install(root: &Root, unit: &Unit, fault: Fault, name: &str) -> UpdateBindings {
    let target = image(30000, 3);
    let mut original = Vec::new();
    let update = install(unit, &target, &mut original);
    let mut port = Fake::new(unit.flash.clone());

    let length = development::extent(None, &target, &PLWF106_BANK1).unwrap();
    let current = protocol::read_region(&mut port, START, length).unwrap();
    let other = protocol::read_region(&mut port, 0x6000, SECTOR).unwrap();
    let neighbors = protocol::read_region(&mut port, 0x18_0000, START - 0x18_0000).unwrap();
    let plan =
        development::plan_bound(&update, &current, Some(&other), Some(sha256(&neighbors))).unwrap();

    fs::create_dir_all(root.join("private").join(name)).unwrap();
    port.0.borrow_mut().fault = fault;
    let output = format!("{name}/transaction");
    assert!(develop(root, unit, &mut port, &update, &output).is_err());

    let bindings = UpdateBindings {
        schema_version: 1,
        kind: Kind::FountainBank1Update,
        port: "/dev/fake".into(),
        interrupted: false,
        original_read_sha256: None,
        restore_marker: false,
        source_review_sha256: None,
        target_review_sha256: sha256(b"target review"),
        baseline_sha256: unit.baseline.sha256(),
        rom_report_sha256: sha256(b"rom"),
        bounded_read_report_sha256: sha256(b"small"),
        tool_sha256: ToolHashes {
            wellfed_flash: sha256(b"tool"),
        },
        stopped_plan_sha256: None,
    };
    let reviewed = Reviewed {
        bindings: bindings.clone(),
        inspection_report_sha256: sha256(b"inspection"),
        operation_plan: plan,
    };
    write(
        &root.join("private").join(name).join("reviewed-plan.json"),
        &wellfed_formats::canonical::canonical_of(&reviewed).unwrap(),
    );
    bindings
}

/// Finishing a stopped fountain install is checked against that session:
/// it must have stopped (not on the bytes beside bank 1), its reviewed plan
/// must be this update's, and its journal must open with that plan.
#[test]
fn a_finish_is_held_to_the_session_that_stopped() {
    use crate::writers::update::{UpdateError, stopped_session};

    let (_checkout, root) = private_root();
    let unit = fountain();
    let finish = |bindings: &UpdateBindings| UpdateBindings {
        interrupted: true,
        ..bindings.clone()
    };
    let check = |name: &str, bindings: &UpdateBindings| {
        let directory = root.join("private").join(name);
        stopped_session(&root, Some(&directory), &finish(bindings))
    };

    // A session that stopped on a lost ACK may be finished; its plan's hash
    // goes into the finish's bindings.
    let stopped = stopped_install(&root, &unit, Fault::LostProgramAck(2), "lost");
    let (plan, sha) = check("lost", &stopped).unwrap().unwrap();
    assert!(plan.neighbors_sha256.is_some());
    let path = root.join("private/lost/reviewed-plan.json");
    assert_eq!(sha, sha256(&fs::read(&path).unwrap()));

    // One that stopped because an erase took the FTL may not.
    let blocked = stopped_install(&root, &unit, Fault::BlockErase, "block");
    assert!(matches!(
        check("block", &blocked),
        Err(crate::Error::Update(UpdateError::StoppedOnNeighbors))
    ));

    // Another update's plan, or a hand-made one the journal never opened.
    let other = UpdateBindings {
        target_review_sha256: sha256(b"another target"),
        ..stopped.clone()
    };
    assert!(matches!(
        check("lost", &other),
        Err(crate::Error::Update(UpdateError::StoppedSessionDiffers))
    ));
    let mut edited: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    edited["operation_plan"]["erase_sectors"] = json!(0);
    fs::write(&path, edited.to_string()).unwrap();
    assert!(matches!(
        check("lost", &stopped),
        Err(crate::Error::Update(UpdateError::StoppedJournalPlan))
    ));

    // A fountain finish needs the session; nothing else takes one.
    assert!(matches!(
        stopped_session(&root, None, &finish(&stopped)),
        Err(crate::Error::Update(UpdateError::StoppedSessionMissing))
    ));
    assert!(matches!(
        stopped_session(&root, Some(&root.join("private/lost")), &stopped),
        Err(crate::Error::Update(UpdateError::StoppedSessionScope))
    ));
}

/// Stock recovery binds the current read as it is, so after an erase that
/// took the FTL it would put bank 1 back over a wiped FTL and boot stock on
/// it. It refuses instead.
#[test]
fn stock_recovery_refuses_a_wiped_ftl() {
    let unit = fountain();
    let mut current = unit.flash.clone();
    current[0x18_0000..0x19_0000].fill(0xff);
    assert!(matches!(
        recovery::plan(&unit.baseline, &current),
        Err(crate::Error::Recovery(RecoveryError::FtlErased {
            start: 0x18_2000,
            end: 0x18_5000
        }))
    ));

    // Bank 1's first sector alone erased is an ordinary recovery.
    let mut current = unit.flash.clone();
    current[START..START + SECTOR].fill(0xff);
    assert!(recovery::plan(&unit.baseline, &current).is_ok());
}

/// The fountain's stock marker is not bootable, so a read whose first bank-1
/// sector already matches the backup while a later one differs would plan
/// a recovery without the marker sector, which the run commits last. The
/// plan refuses it, so the run refuses before its first erase.
#[test]
fn a_recovery_without_the_marker_sector_is_refused_before_any_write() {
    let (_checkout, root) = private_root();
    let unit = fountain();
    let mut current = unit.flash.clone();
    current[START + SECTOR + 17] ^= 1;
    assert!(matches!(
        recovery::plan(&unit.baseline, &current),
        Err(crate::Error::Recovery(RecoveryError::MarkerSectorUnchanged {
            offset
        })) if offset == START + SECTOR
    ));

    // A plan written before this refusal is refused at the run, before the
    // journal and with the flash untouched.
    let mut changed = current.clone();
    changed[START] ^= 1;
    let mut plan = serde_json::to_value(recovery::plan(&unit.baseline, &changed).unwrap()).unwrap();
    plan["current_snapshot_sha256"] = serde_json::json!(sha256(&current));
    let mut port = Fake::new(current.clone());
    let mut snapshot = |port: &mut dyn Port| protocol::read_region(port, 0, CAPACITY);
    assert!(
        recovery::run_transaction(
            &mut port,
            &root,
            &unit.baseline,
            &current,
            &plan,
            &mut snapshot,
            &root.join("private/recovery"),
        )
        .is_err()
    );
    assert!(port.0.borrow().flash == current);
    assert!(!root.join("private/recovery").exists());
}

/// A status read assesses the bank 1 of the model it reads, not the
/// feeder's for every model.
#[test]
fn a_status_read_assesses_the_models_own_bank_1() {
    for (model, end) in [(Model::Plaf103, BANK1.end), (Model::Plwf106, END)] {
        let mut port = Fake::new(fountain().flash);
        let inspection =
            crate::writers::read::inspect_status(&mut port, model, &mut |_| Ok(())).unwrap();
        let range = inspection.bank1_assessment.range;
        assert_eq!(
            (range.start, range.end_exclusive),
            (START, end),
            "{model:?}"
        );
    }
}
