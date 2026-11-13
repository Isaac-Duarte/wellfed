//! The actual factory loader on the pinned baseline: bank checks, mappings,
//! selection and the OTA marker sequence. Emulation only.
//!
//! Needs the ignored baseline, SDK ROM and stock OTA (see
//! `docs/development/fixtures.md`); `WELLFED_FIXTURE_ROOT` names another
//! checkout whose `private/` holds them.

use std::fs;
use std::path::PathBuf;

use wellfed_core::ota::{CONFIRM_LEN, confirm_record};
use wellfed_emu::factory::{self, Entry, execute, inputs, selected};
use wellfed_emu::{BANKS, Error, FLASH_BYTES};
use wellfed_formats::canonical::sha256;
use wellfed_formats::review::{FactoryRun, Stop};
use wellfed_hal_common::trial::{self, Decision};

const STOCK: &str = "private/firmware/PLAF103_OTA_010115.bin";
const STOCK_SHA256: &str = "05147476fc70c91cc77ad7da6d6772376b2081d460ac7b1912e2d73868a09498";
const MARKER: &[u8; 8] = b"81958711";
const BANK_BYTES: usize = 0x15_0000;

fn root() -> PathBuf {
    std::env::var_os("WELLFED_FIXTURE_ROOT").map_or_else(
        || PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../..")),
        PathBuf::from,
    )
}

struct Fixture {
    flash: Vec<u8>,
    rom: Vec<u8>,
    stock: Vec<u8>,
}

fn fixture() -> Fixture {
    let root = root();
    let (flash, rom) = inputs(&root).unwrap();
    let stock = fs::read(root.join(STOCK)).unwrap();
    assert_eq!(sha256(&stock), STOCK_SHA256);
    Fixture { flash, rom, stock }
}

fn bank(index: u8) -> usize {
    BANKS[usize::from(index)] as usize
}

/// The stock payload (the OTA container without its 32-byte header) in `bank`.
fn with_stock(flash: &[u8], stock: &[u8], index: u8) -> Vec<u8> {
    let mut flash = flash.to_vec();
    let payload = &stock[32..];
    flash[bank(index)..bank(index) + payload.len()].copy_from_slice(payload);
    flash
}

/// A valid stock bank check maps both cores over exactly the stock extents.
fn assert_stock_mapping(run: &FactoryRun, index: u8) {
    assert_eq!((run.stop, run.result), (Stop::Returned, 1));
    assert_eq!(run.configs.len(), 4);
    assert_eq!(run.commands, [[0, 1], [1, 1]]);
    for (core, (start, size, split)) in
        [(0x0C00_0000, 0x17000, 0), (0x0E00_0000, 0x13_9000, 0x17000)]
            .into_iter()
            .enumerate()
    {
        let (initial, last) = (&run.configs[core * 2], &run.configs[core * 2 + 1]);
        assert_eq!(
            initial.end,
            initial.start + 0x1FF_FFFF,
            "temporary 32 MiB map"
        );
        let physical = 0x0800_0000 + BANKS[usize::from(index)] + split;
        assert_eq!((last.start, last.end), (start, start + size - 1));
        assert_eq!((last.minus, last.offset), (1, start - physical));
        assert_eq!(run.final_registers[core][0], 3, "enabled and subtractive");
    }
    assert_eq!(run.configs[3].end, 0x0E13_8FFF);
}

#[test]
#[ignore = "needs the private baseline, SDK ROM and stock OTA"]
fn actual_backup_selects_bank_1_and_maps_stock_in_either_bank() {
    let f = fixture();

    assert_stock_mapping(&execute(&f.flash, &f.rom, Entry::Check(1)).unwrap(), 1);
    let invalid = execute(&f.flash, &f.rom, Entry::Check(0)).unwrap();
    assert_eq!((invalid.stop, invalid.result), (Stop::Returned, 0));
    assert_eq!(selected(&f.flash, &f.rom).unwrap(), Some(1));

    for index in [0, 1] {
        let flash = with_stock(&f.flash, &f.stock, index);
        assert_stock_mapping(
            &execute(&flash, &f.rom, Entry::Check(index)).unwrap(),
            index,
        );
    }

    // With both valid the selector checks bank 1 first and keeps it; bank 0
    // is the fallback; with neither it waits.
    let mut both = with_stock(&f.flash, &f.stock, 0);
    assert_eq!(selected(&both, &f.rom).unwrap(), Some(1));
    both[bank(1)] = 0;
    assert_eq!(selected(&both, &f.rom).unwrap(), Some(0));
    both[bank(0)] = 0;
    assert_eq!(selected(&both, &f.rom).unwrap(), None);
}

#[test]
#[ignore = "needs the private baseline, SDK ROM and stock OTA"]
fn a_changed_rounding_instruction_changes_the_final_extent() {
    let f = fixture();
    let mut changed = f.flash.clone();
    changed[0x96] ^= 0x40; // LSLS #12 -> #13.

    let outcome = execute(&changed, &f.rom, Entry::Check(1));
    let extent = outcome
        .ok()
        .and_then(|run| run.configs.get(3).map(|c| c.end));
    assert_ne!(extent, Some(0x0E13_8FFF));
}

#[test]
#[ignore = "needs the private baseline, SDK ROM and stock OTA"]
fn a_mismatched_km4_signature_stops_at_the_warning_loop() {
    let f = fixture();
    let mut changed = f.flash.clone();
    changed[bank(1) + 0x17000] ^= 1;

    let run = execute(&changed, &f.rom, Entry::Check(1)).unwrap();
    assert_eq!(run.stop, Stop::Warning);
}

/// Refused before any instruction runs, so no fixture is needed: a byte
/// short of the reviewed 8 MiB.
#[test]
fn an_unreviewed_flash_size_is_refused_before_execution() {
    let short = vec![0xFF; FLASH_BYTES - 1];
    assert!(matches!(
        execute(&short, &[], Entry::Select),
        Err(Error::Factory(factory::FactoryError::FlashSize { .. }))
    ));
}

/// Erase `bank`, then write `image`'s payload (unless `body` is false) and
/// `marker` as its first bytes, the rest of the marker erased.
fn place(flash: &mut [u8], index: u8, image: &[u8], marker: &[u8], body: bool) {
    let base = bank(index);
    flash[base..base + BANK_BYTES].fill(0xFF);
    if body {
        let payload = &image[32..];
        flash[base..base + payload.len()].copy_from_slice(payload);
    }
    let mut full = [0xFF; 8];
    full[..marker.len()].copy_from_slice(marker);
    flash[base..base + 8].copy_from_slice(&full);
}

/// Each modeled step of the update flow (docs/firmware/ota-design.md), with
/// the bank the design requires the loader to pick.
#[test]
#[ignore = "needs the private baseline, SDK ROM and stock OTA"]
fn every_ota_step_and_marker_cut_keeps_a_bootable_bank() {
    let f = fixture();
    let image = &f.stock;
    let mut cleared = *MARKER;
    cleared[0] = 0;

    type Step<'a> = (String, Box<dyn Fn(&mut [u8]) + 'a>, Option<u8>);
    let mut steps: Vec<Step> = vec![
        (
            "installed: normal in bank 1, recovery in bank 0".into(),
            Box::new(|f| {
                place(f, 0, image, MARKER, true);
                place(f, 1, image, MARKER, true)
            }),
            Some(1),
        ),
        (
            "give way: bank 1 marker first byte cleared".into(),
            Box::new(|f| {
                place(f, 0, image, MARKER, true);
                place(f, 1, image, &cleared, true)
            }),
            Some(0),
        ),
        (
            "download: bank 1 erased".into(),
            Box::new(|f| {
                place(f, 0, image, MARKER, true);
                place(f, 1, image, b"", false)
            }),
            Some(0),
        ),
        (
            "download: bank 1 body staged, marker erased".into(),
            Box::new(|f| {
                place(f, 0, image, MARKER, true);
                place(f, 1, image, b"", true)
            }),
            Some(0),
        ),
        (
            "activate: bank 1 marker written".into(),
            Box::new(|f| {
                place(f, 0, image, MARKER, true);
                place(f, 1, image, MARKER, true)
            }),
            Some(1),
        ),
        (
            "fallback: unconfirmed trial cleared".into(),
            Box::new(|f| {
                place(f, 0, image, MARKER, true);
                place(f, 1, image, &cleared, true)
            }),
            Some(0),
        ),
    ];
    // With no recovery image, giving way leaves nothing to boot, which is
    // why the design forbids it.
    steps.push((
        "forbidden: give way with no recovery".into(),
        Box::new(|f| {
            place(f, 0, image, &cleared, true);
            place(f, 1, image, &cleared, true)
        }),
        None,
    ));
    // Installed and activate, and give way and fallback, are one flash
    // state each, reached by two flows; a cut inside the marker's write is
    // `every_partial_marker_write_gives_way_to_bank_0`'s.
    assert_eq!(steps.len(), 7, "every step and the forbidden state");

    for (label, build, expected) in &steps {
        let mut flash = f.flash.clone();
        build(&mut flash);
        assert_eq!(selected(&flash, &f.rom).unwrap(), *expected, "{label}");
    }
}

/// Every value a byte can hold partway through programming toward `target`
/// from erased: NOR only clears bits, so each subset of the bits `target`
/// clears, from none (still erased) to all (`target` itself).
fn partial_bytes(target: u8) -> Vec<u8> {
    let cleared = !target;
    (0..=u8::MAX)
        .filter(|subset| subset & !cleared == 0)
        .map(|subset| !subset)
        .collect()
}

/// A cut anywhere in the 8-byte marker's write: the bytes before it whole,
/// the byte it cut at any partial value, the rest erased. Every such state
/// but the whole marker selects bank 0.
#[test]
#[ignore = "needs the private baseline, SDK ROM and stock OTA"]
fn every_partial_marker_write_gives_way_to_bank_0() {
    let f = fixture();
    let mut flash = f.flash.clone();
    place(&mut flash, 0, &f.stock, MARKER, true);

    let mut states = 0;
    for n in 0..MARKER.len() {
        for byte in partial_bytes(MARKER[n]) {
            let mut marker = [0xFF; 8];
            marker[..n].copy_from_slice(&MARKER[..n]);
            marker[n] = byte;
            if marker == *MARKER {
                continue;
            }
            place(&mut flash, 1, &f.stock, &marker, true);
            assert_eq!(
                selected(&flash, &f.rom).unwrap(),
                Some(0),
                "cut at byte {n}: {marker:02x?}"
            );
            states += 1;
        }
    }
    assert!(states > 100, "{states}");
}

/// Clearing marker bits gives way: any one set bit of any byte, and any
/// part of the give-way's clear of the first byte to zero, as a cut leaves
/// it.
#[test]
#[ignore = "needs the private baseline, SDK ROM and stock OTA"]
fn any_cleared_marker_bits_give_way_to_bank_0() {
    let f = fixture();
    let mut flash = f.flash.clone();
    place(&mut flash, 0, &f.stock, MARKER, true);
    place(&mut flash, 1, &f.stock, MARKER, true);
    assert_eq!(selected(&flash, &f.rom).unwrap(), Some(1));

    let mut markers = Vec::new();
    for index in 0..MARKER.len() {
        for bit in 0..8 {
            if MARKER[index] & (1 << bit) != 0 {
                let mut marker = *MARKER;
                marker[index] &= !(1 << bit);
                markers.push(marker);
            }
        }
    }
    assert_eq!(
        markers.len(),
        MARKER
            .iter()
            .map(|b| b.count_ones() as usize)
            .sum::<usize>()
    );
    // The give-way clears the first byte to zero: any subset of its set
    // bits cleared is where a cut can leave it.
    let set = MARKER[0];
    for subset in (1..=u8::MAX).filter(|subset| subset & !set == 0) {
        let mut marker = *MARKER;
        marker[0] = set & !subset;
        markers.push(marker);
    }

    for marker in markers {
        place(&mut flash, 1, &f.stock, &marker, true);
        assert_eq!(selected(&flash, &f.rom).unwrap(), Some(0), "{marker:02x?}");
    }
}

/// KM0's trial over `flash` for `boots` boots in one power-on, each ending in
/// a reset nobody asked for, with the confirmation `record`: bank 0's
/// headers read from the flash as the glue maps them, and a give-way
/// clearing bank 1's first marker byte as the glue does. After every boot
/// the factory loader must still select a bank. The decisions.
fn trial_boots(
    flash: &mut [u8],
    rom: &[u8],
    record: &[u8; CONFIRM_LEN],
    boots: usize,
) -> Vec<Decision> {
    let identity = [0x5a; 32];
    let mut register = 0u32;
    let mut out = Vec::new();
    for boot in 0..boots {
        let bank1: [u8; 8] = flash[bank(1)..bank(1) + 8].try_into().unwrap();
        let bank0 = trial::fallback(|at, header| {
            let at = bank(0) + at as usize;
            header.copy_from_slice(&flash[at..at + 32]);
            true
        });
        register = (register & 0xffff_0000) | ((register + 1) & 0xffff);
        let (decision, value) = trial::decide(&bank1, &bank0, record, register, &identity);
        register = value;
        if decision.gives_way() {
            flash[bank(1)] = 0;
        }
        out.push(decision);

        let chosen = selected(flash, rom);
        assert!(
            matches!(chosen, Ok(Some(_))),
            "boot {boot} ({decision:?}) leaves {chosen:?}"
        );
    }
    out
}

/// Bank 0 with its marker intact but its KM4 header damaged, which the
/// loader refuses (its warning loop): neither a trial that runs out nor a
/// storm may clear bank 1's marker over it, which would leave nothing to
/// boot. With bank 0 intact, both give way and the loader boots bank 0.
#[test]
#[ignore = "needs the private baseline, SDK ROM and stock OTA"]
fn a_trial_or_storm_never_gives_way_onto_a_bank_0_the_loader_refuses() {
    let f = fixture();
    let mut both = f.flash.clone();
    place(&mut both, 0, &f.stock, MARKER, true);
    place(&mut both, 1, &f.stock, MARKER, true);
    let mut damaged = both.clone();
    damaged[bank(0) + 0x17000] ^= 1;
    let alone = {
        let mut flash = damaged.clone();
        flash[bank(1)] = 0;
        flash
    };
    let refused = execute(&alone, &f.rom, Entry::Select).unwrap();
    assert_eq!(
        refused.stop,
        Stop::Warning,
        "the loader refuses that bank 0"
    );

    let unconfirmed = [0xff; CONFIRM_LEN];
    let confirmed = confirm_record(&[0x5a; 32], 0);
    let limit = trial::LIMIT as usize + 1;
    let storm = trial::STORM_LIMIT as usize + 1;
    for (record, boots) in [(&unconfirmed, limit), (&confirmed, storm)] {
        let mut flash = both.clone();
        let out = trial_boots(&mut flash, &f.rom, record, boots);
        assert!(out.last().is_some_and(|d| d.gives_way()), "{out:?}");
        assert_eq!(selected(&flash, &f.rom).unwrap(), Some(0));

        let mut flash = damaged.clone();
        let out = trial_boots(&mut flash, &f.rom, record, boots + 3);
        assert!(!out.iter().any(|d| d.gives_way()), "{out:?}");
        assert_eq!(selected(&flash, &f.rom).unwrap(), Some(1));
    }
}
