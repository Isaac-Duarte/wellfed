//! Each model's reviewed KM0 pad table, reproduced in the SDK source and
//! audited in the linked image, without device I/O.
//!
//! The pinned SDK's `pmap_func` differs from each model's stock table in a
//! few reviewed fields, which the overlay sets to stock's; any other
//! difference is refused. The PLAF103's stock clears eight active pulls. The
//! PLWF106's stock pulls PA22 up and keeps it in sleep, and the boot test
//! departs from stock in one field on purpose: stock pulls PA30, the pump
//! enable, up until its KM4 latches it LOW, and the boot test pulls it down
//! instead. Source text alone cannot detect stale objects, so the linked
//! table's bytes are compared with the expected table too, and pinned.

use std::sync::LazyLock;

use regex::Regex;
use wellfed_formats::audit::{PinmapAudit, pinmap_sha256};
use wellfed_formats::layout::Hex;
use wellfed_proto::device::Model;

use super::word;
use crate::{Result, sha256};
use wellfed_formats::require;

/// Why the SDK pad table or the linked one differs from the reviewed one.
#[derive(Debug, thiserror::Error)]
pub enum PinmapError {
    #[error("Pin table requires the exact verified stock reference")]
    Stock,
    #[error("Unexpected stock pin order or sentinel")]
    StockOrder,
    #[error("Expected one pinned SDK pmap_func table")]
    Tables { found: usize },
    #[error("Unrecognized SDK pad fields")]
    Fields { row: usize },
    #[error("SDK pin order or sentinel differs from stock")]
    Order { row: usize },
    #[error("SDK sleep/low-power fields differ from stock")]
    SleepFields { pin: u32 },
    #[error("Unexpected active pull discrepancy")]
    ActivePull { pin: u32 },
    #[error("Cannot replace SDK active pull")]
    Replace { pin: u32 },
    #[error("Expected 65 records and the reviewed field changes")]
    Changes {
        rows: usize,
        changed: Vec<(u32, Field)>,
    },
    #[error("Candidate KM0 image header is truncated")]
    Truncated,
    #[error("Unexpected candidate KM0 XIP header")]
    Header,
    #[error("Linked pad table is outside the KM0 XIP body")]
    Outside { address: u64 },
    #[error("Linked KM0 pad table differs from the reviewed one")]
    Differs,
    #[error("The reviewed pad table is not the pinned one: {sha256}")]
    Unpinned { sha256: String },
    #[error("An override names pin {pin}, which no table row has")]
    OverridePin { pin: u32 },
}

/// One of a pad row's settings, as the SDK source lists them after the pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    Active,
    Sleep,
    LowPower,
}

impl Field {
    /// Its index in a row: the pin is 0.
    const fn index(self) -> usize {
        match self {
            Self::Active => 1,
            Self::Sleep => 2,
            Self::LowPower => 3,
        }
    }
}

/// A model's stock pad table, where it lies in the reference the builder
/// reads, and what the image changes in it.
struct Reference {
    /// The reference file (ignored private evidence): the stock OTA, or a
    /// full flash backup.
    sha256: &'static str,
    bytes: usize,
    /// The table's first byte in the reference file.
    offset: usize,
    /// The fields in which the pinned SDK table differs from stock, which
    /// the overlay sets to stock's.
    stock_changes: &'static [(u32, Field)],
    /// Active pulls the image sets away from stock, by pin.
    overrides: &'static [(u32, u32)],
}

const XIP_ADDRESS: u64 = 0x0C00_0020;
const TABLE_ROWS: usize = 65;
const ROW_BYTES: usize = 20;
const NO_PIN: u32 = 0xffff_ffff;
const NOPULL: u32 = 0;
const DOWN: u32 = 1;

/// The stock 1.1.15 OTA's KM0 table, at 0x0C013328.
const PLAF103: Reference = Reference {
    sha256: "05147476fc70c91cc77ad7da6d6772376b2081d460ac7b1912e2d73868a09498",
    bytes: 1_376_288,
    offset: 64 + 0x1_3308,
    stock_changes: &[
        (12, Field::Active),
        (13, Field::Active),
        (14, Field::Active),
        (15, Field::Active),
        (18, Field::Active),
        (30, Field::Active),
        (33, Field::Active),
        (34, Field::Active),
    ],
    overrides: &[],
};

/// The full backup's bank-0 KM0 table, at 0x0C012EEC (bank 1's is the same):
/// the KM0 XIP body starts 32 bytes into bank 0 at 0x6000.
const PLWF106: Reference = Reference {
    sha256: "fe2516e378a67616777a6c86f480f642254a12c726a3f352ff51a1d17966376f",
    bytes: 0x80_0000,
    offset: 0x6000 + 32 + 0x1_2ECC,
    stock_changes: &[(22, Field::Active), (22, Field::Sleep)],
    overrides: &[(30, DOWN)],
};

/// The SHA-256 of `model`'s pinned stock reference: the stock OTA, or the
/// full backup the fountain's review also boots its image on.
pub(crate) const fn reference_sha256(model: Model) -> &'static str {
    reference(model).sha256
}

const fn reference(model: Model) -> &'static Reference {
    match model {
        Model::Plaf103 => &PLAF103,
        Model::Plwf106 => &PLWF106,
    }
}

static TABLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?ms)const\s+PMAP_TypeDef\s+pmap_func\[\]\s*=\s*\{(?P<body>.*?)^\};")
        .expect("table pattern")
});
static ROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[ \t]*\{[ \t]*(_P[AB]_\d+|_PNC)[ \t]*,[ \t]*([^{}\r\n]+)\},[^\r\n]*$")
        .expect("row pattern")
});

type Row = [u32; 5];

const PULLS: [(&str, u32); 5] = [
    ("GPIO_PuPd_NOPULL", NOPULL),
    ("GPIO_PuPd_DOWN", DOWN),
    ("GPIO_PuPd_UP", 2),
    ("GPIO_PuPd_SHUTDOWN", 3),
    ("GPIO_PuPd_KEEP", 255),
];

fn pull(name: &str) -> Option<u32> {
    PULLS.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
}

fn pull_name(value: u32) -> Option<&'static str> {
    PULLS.iter().find(|(_, v)| *v == value).map(|(n, _)| *n)
}

fn flag(name: &str) -> Option<u32> {
    match name {
        "FALSE" => Some(0),
        "TRUE" => Some(1),
        _ => None,
    }
}

fn rows_of(data: &[u8]) -> Vec<Row> {
    data.as_chunks::<ROW_BYTES>()
        .0
        .iter()
        .map(|row| {
            let (words, _) = row.as_chunks::<4>();
            std::array::from_fn(|i| u32::from_le_bytes(words[i]))
        })
        .collect()
}

/// The stock table's bytes and rows: pin, active pull, sleep pull, low-power
/// pull, flag.
fn stock_table(model: Model, reference: &[u8]) -> Result<(&[u8], Vec<Row>)> {
    let pinned = self::reference(model);
    require(
        reference.len() == pinned.bytes && sha256(reference) == pinned.sha256,
        || PinmapError::Stock,
    )?;
    let data = &reference[pinned.offset..pinned.offset + TABLE_ROWS * ROW_BYTES];
    let rows = rows_of(data);

    let pins: Vec<u32> = rows.iter().map(|row| row[0]).collect();
    let expected: Vec<u32> = (0..64).chain([NO_PIN]).collect();
    require(pins == expected, || PinmapError::StockOrder)?;
    Ok((data, rows))
}

/// The table the image links: stock's, with the model's overrides.
fn expected_table(model: Model, reference: &[u8]) -> Result<(Vec<u8>, Vec<Row>)> {
    let (data, mut rows) = stock_table(model, reference)?;
    let mut bytes = data.to_vec();
    for &(pin, value) in reference_overrides(model) {
        let at = usize::try_from(pin)
            .ok()
            .filter(|at| *at < TABLE_ROWS - 1)
            .ok_or(PinmapError::OverridePin { pin })?;
        rows[at][Field::Active.index()] = value;
        let field = at * ROW_BYTES + 4 * Field::Active.index();
        bytes[field..field + 4].copy_from_slice(&value.to_le_bytes());
    }
    Ok((bytes, rows))
}

/// How the image's table departs from stock, for the audit's scope:
/// nothing for none, else `: stock's, but PA30 pulled down` and so on.
fn departures(overrides: &[(u32, u32)]) -> String {
    let named: Vec<String> = overrides
        .iter()
        .map(|&(pin, value)| {
            let (port, bit) = if pin < 32 {
                ("A", pin)
            } else {
                ("B", pin - 32)
            };
            let pull = match value {
                NOPULL => "with no pull",
                DOWN => "pulled down",
                2 => "pulled up",
                _ => "pulled otherwise",
            };
            format!("P{port}{bit} {pull}")
        })
        .collect();
    if named.is_empty() {
        String::new()
    } else {
        format!(": stock's, but {}", named.join(", "))
    }
}

fn reference_overrides(model: Model) -> &'static [(u32, u32)] {
    reference(model).overrides
}

/// Set the SDK table to the model's expected one, field by field; refuse
/// any difference that is neither a reviewed stock change nor an override.
pub(crate) fn overlay(source: &str, model: Model, reference: &[u8]) -> Result<String> {
    let (_, expected) = expected_table(model, reference)?;
    let pinned = self::reference(model);
    let tables: Vec<_> = TABLE.captures_iter(source).collect();
    let [table] = &tables[..] else {
        return Err(PinmapError::Tables {
            found: tables.len(),
        }
        .into());
    };
    let body = table.name("body").expect("TABLE always captures its body");

    let (mut output, mut rows, mut changed) = (String::new(), 0usize, Vec::new());
    for line in body.as_str().split_inclusive('\n') {
        let mut line = line.to_string();
        if let Some(m) = ROW.captures(line.trim_end_matches(['\r', '\n'])) {
            let name = &m[1];
            let pin = if name == "_PNC" {
                NO_PIN
            } else {
                (if &name[2..3] == "B" { 32 } else { 0 }) + name[4..].parse::<u32>().unwrap_or(0)
            };
            let values: Vec<&str> = m[2].split(',').collect();
            let parsed: Option<Row> = (values.len() == 4)
                .then(|| {
                    Some([
                        pin,
                        pull(values[0].trim())?,
                        pull(values[1].trim())?,
                        pull(values[2].trim())?,
                        flag(values[3].trim())?,
                    ])
                })
                .flatten();
            let Some(row) = parsed else {
                return Err(PinmapError::Fields { row: rows }.into());
            };

            require(rows < TABLE_ROWS && row[0] == expected[rows][0], || {
                PinmapError::Order { row: rows }
            })?;
            let target = expected[rows];
            require(row[4] == target[4], || PinmapError::SleepFields { pin })?;

            let mut fields: Vec<String> = values.iter().map(|v| (*v).to_string()).collect();
            let mut rewritten = false;
            for field in [Field::Active, Field::Sleep, Field::LowPower] {
                let at = field.index();
                if row[at] == target[at] {
                    continue;
                }
                let reviewed = pinned.stock_changes.contains(&(pin, field))
                    || (field == Field::Active && pinned.overrides.iter().any(|(p, _)| *p == pin));
                require(reviewed, || match field {
                    Field::Active => PinmapError::ActivePull { pin },
                    _ => PinmapError::SleepFields { pin },
                })?;
                let Some(name) = pull_name(target[at]) else {
                    return Err(PinmapError::Replace { pin }.into());
                };
                let token = fields[at - 1].trim().to_string();
                fields[at - 1] = fields[at - 1].replacen(&token, name, 1);
                changed.push((pin, field));
                rewritten = true;
            }
            // Only the changed tokens move: the row's spacing and comment
            // stay as the SDK wrote them.
            if rewritten && let Some(values) = m.get(2) {
                let (start, end) = (values.start(), values.end());
                line = format!("{}{}{}", &line[..start], fields.join(","), &line[end..]);
            }
            rows += 1;
        }
        output.push_str(&line);
    }

    let mut reviewed: Vec<(u32, Field)> = pinned.stock_changes.to_vec();
    reviewed.extend(
        pinned
            .overrides
            .iter()
            .map(|(pin, _)| (*pin, Field::Active)),
    );
    reviewed.sort();
    let mut found = changed.clone();
    found.sort();
    require(rows == TABLE_ROWS && found == reviewed, || {
        PinmapError::Changes { rows, changed }
    })?;
    Ok(format!(
        "{}{output}{}",
        &source[..body.start()],
        &source[body.end()..]
    ))
}

/// Check the linked table's bytes in a candidate OTA image against the
/// model's expected table, and that table against its pin.
pub(crate) fn audit_linked_table(
    candidate: &[u8],
    address: u64,
    model: Model,
    reference: &[u8],
) -> Result<PinmapAudit> {
    let (expected, _) = expected_table(model, reference)?;
    let pinned = self::reference(model);
    let digest = sha256(&expected);
    // The table's pin, shared with the review that accepts it, so that an
    // edit to the reference, the reviewed changes or the overrides shows.
    require(digest == pinmap_sha256(model), || PinmapError::Unpinned {
        sha256: digest.clone(),
    })?;

    let header = [32, 36, 40, 44].map(|at| word(candidate, at));
    let ([Some(magic1), Some(magic2), Some(size), Some(base)], 64..) = (header, candidate.len())
    else {
        return Err(PinmapError::Truncated.into());
    };
    require(
        (magic1, magic2, base) == (0x3539_3138, 0x3131_3738, XIP_ADDRESS),
        || PinmapError::Header,
    )?;

    // The table must lie inside the XIP body, which must lie inside the image.
    let table = expected.len() as u64;
    let start = address
        .checked_sub(base)
        .filter(|offset| offset + table <= size && 64 + size <= candidate.len() as u64)
        .and_then(|offset| usize::try_from(64 + offset).ok());
    let Some(start) = start else {
        return Err(PinmapError::Outside { address }.into());
    };
    let actual = &candidate[start..start + expected.len()];
    require(actual == expected, || PinmapError::Differs)?;

    let matches_stock = pinned.overrides.is_empty();
    Ok(PinmapAudit {
        runtime_address: Hex(address),
        rows: TABLE_ROWS,
        bytes: actual.len(),
        sha256: sha256(actual),
        matches_stock,
        scope: format!(
            "Pad pull table only{}; reset levels, GPIO direction and physical behavior unverified",
            departures(pinned.overrides)
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::amebad::source;
    use crate::{Device, Error, Repo};
    use std::fs;

    /// The PLAF103 stock table's runtime address.
    const TABLE_ADDRESS: u64 = 0x0C01_3328;

    /// The ignored stock image and pinned SDK source, when present.
    fn evidence() -> (Vec<u8>, String) {
        let repo = Repo::here().unwrap();
        let stock = Device::new(&repo, "plaf103").unwrap().stock(&repo).unwrap();
        let source =
            source(&repo).join("component/soc/realtek/amebad/fwlib/usrcfg/rtl8721dlp_pinmapcfg.c");
        (
            fs::read(stock.image).expect("ignored verified stock OTA"),
            fs::read_to_string(source).expect("pinned SDK checkout"),
        )
    }

    #[test]
    #[ignore = "needs the ignored stock OTA and pinned SDK checkout"]
    fn linked_stock_table_and_record_fields() {
        let (stock, _) = evidence();
        let report = audit_linked_table(&stock, TABLE_ADDRESS, Model::Plaf103, &stock).unwrap();
        assert_eq!(report.rows, 65);
        assert_eq!(report.bytes, 1300);
        assert!(report.matches_stock);
        // The shared pin a review checks names this table.
        assert_eq!(report.sha256, pinmap_sha256(Model::Plaf103));
        let (_, rows) = stock_table(Model::Plaf103, &stock).unwrap();
        assert_eq!(rows[13], [13, 0, 255, 255, 1]);
        assert_eq!(rows[33], [33, 0, 255, 255, 0]);
    }

    #[test]
    #[ignore = "needs the ignored stock OTA and pinned SDK checkout"]
    fn overlay_changes_only_eight_active_fields() {
        let (stock, source) = evidence();
        let updated = overlay(&source, Model::Plaf103, &stock).unwrap();
        assert_eq!(source.lines().count(), updated.lines().count());

        let changed: Vec<_> = source
            .lines()
            .zip(updated.lines())
            .filter(|(a, b)| a != b)
            .collect();
        assert_eq!(changed.len(), 8);
        for (before, after) in changed {
            let pull = if before.contains("GPIO_PuPd_DOWN") {
                "GPIO_PuPd_DOWN"
            } else {
                "GPIO_PuPd_UP"
            };
            assert_eq!(after, before.replacen(pull, "GPIO_PuPd_NOPULL", 1));
        }
    }

    /// The pinmap refusal a check returned; any other result fails the test.
    fn pinmap_error<T>(result: Result<T>) -> PinmapError {
        match result {
            Err(Error::Pinmap(error)) => error,
            Err(other) => panic!("{other}"),
            Ok(_) => panic!("accepted"),
        }
    }

    #[test]
    #[ignore = "needs the ignored stock OTA and pinned SDK checkout"]
    fn unreviewed_changes_and_damaged_references_are_rejected() {
        let (stock, source) = evidence();

        let sleep = source
            .lines()
            .find(|l| l.trim_start().starts_with("{_PA_13,"))
            .unwrap();
        let damaged = source.replacen(
            sleep,
            &sleep.replacen("GPIO_PuPd_KEEP", "GPIO_PuPd_DOWN", 1),
            1,
        );
        assert!(matches!(
            pinmap_error(overlay(&damaged, Model::Plaf103, &stock)),
            PinmapError::SleepFields { pin: 13 }
        ));
        for replacement in ["_PA_14,", "_PA_99,"] {
            assert!(matches!(
                pinmap_error(overlay(
                    &source.replacen("_PA_13,", replacement, 1),
                    Model::Plaf103,
                    &stock
                )),
                PinmapError::Order { row: 13 }
            ));
        }

        let mut mutated = stock.clone();
        *mutated.last_mut().unwrap() ^= 1;
        assert!(matches!(
            pinmap_error(overlay(&source, Model::Plaf103, &mutated)),
            PinmapError::Stock
        ));
        assert!(matches!(
            pinmap_error(audit_linked_table(
                &stock,
                TABLE_ADDRESS,
                Model::Plaf103,
                &mutated
            )),
            PinmapError::Stock
        ));

        // A valid container checksum must not hide stale or incorrect pad data.
        let mut stale = stock.clone();
        let active_pull = 64 + (TABLE_ADDRESS - XIP_ADDRESS) as usize + 13 * 20 + 4;
        stale[active_pull..active_pull + 4].copy_from_slice(&1u32.to_le_bytes());
        let sum = stale[32..]
            .iter()
            .fold(0u32, |s, b| s.wrapping_add(*b as u32));
        stale[16..20].copy_from_slice(&sum.to_le_bytes());
        assert!(matches!(
            pinmap_error(audit_linked_table(
                &stale,
                TABLE_ADDRESS,
                Model::Plaf103,
                &stock
            )),
            PinmapError::Differs
        ));

        assert!(matches!(
            pinmap_error(audit_linked_table(&stock, XIP_ADDRESS - 4, Model::Plaf103, &stock)),
            PinmapError::Outside { address } if address == XIP_ADDRESS - 4
        ));
        let mut moved = stock.clone();
        moved[44..48].copy_from_slice(&0x0E00_0020u32.to_le_bytes());
        assert!(matches!(
            pinmap_error(audit_linked_table(
                &moved,
                TABLE_ADDRESS,
                Model::Plaf103,
                &stock
            )),
            PinmapError::Header
        ));
    }

    /// The scope names each departure from stock, and none for the PLAF103.
    #[test]
    fn the_scope_names_the_overrides() {
        assert_eq!(departures(&[]), "");
        assert_eq!(departures(&[(30, DOWN)]), ": stock's, but PA30 pulled down");
        assert_eq!(
            departures(&[(35, NOPULL)]),
            ": stock's, but PB3 with no pull"
        );
    }
}
