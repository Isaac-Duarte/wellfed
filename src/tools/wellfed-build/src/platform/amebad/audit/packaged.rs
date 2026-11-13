//! The ELF text a linked-code audit reads must be the text that was packaged
//! (from `verify_packaged_code` in `src/tools/audit/audit-sdk-flash-users.py`),
//! so a stale ELF is refused even if older packaged artifacts still match.

use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use wellfed_formats::audit::PackagedText;
use wellfed_formats::layout::Hex;
use wellfed_image::ota::Image;

use super::{index, parse_hex, tool};
use crate::platform::amebad::Core;
use crate::{Result, sha256};
use wellfed_formats::require;

/// Why a core's linked text differs from what was packaged.
#[derive(Debug, thiserror::Error)]
pub enum PackagedError {
    #[error("No packaged region {region}")]
    NoRegion { region: String },
    #[error("ELF text is outside packaged region: {section}")]
    OutsideRegion { section: String },
    #[error("ELF text does not match packaged image: {section}")]
    Differs { section: String },
    #[error("Expected exactly the packaged XIP and RAM text sections")]
    Sections { found: Vec<String> },
}

static TEXT_SECTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*\d+\s+(\.(?:ram|xip)_image2\.text)\s+([0-9a-f]+)\s+([0-9a-f]+)\s+[0-9a-f]+\s+([0-9a-f]+)",
    )
    .expect("section pattern")
});

/// Byte-match a core's RAM and XIP text against the packaged regions of the
/// OTA payload.
pub(crate) fn verify(
    elf: &Path,
    objdump: &Path,
    payload: &[u8],
    regions: &[Image],
    core: Core,
) -> Result<Vec<PackagedText>> {
    let data = fs::read(elf)?;
    let headers = tool(objdump, &["-h"], elf)?;
    verify_sections(&headers, &data, payload, regions, core)
}

/// [`verify`] on the ELF's `objdump -h` listing and its bytes.
fn verify_sections(
    headers: &str,
    data: &[u8],
    payload: &[u8],
    regions: &[Image],
    core: Core,
) -> Result<Vec<PackagedText>> {
    let mut checked = Vec::new();

    for line in headers.lines() {
        let Some(m) = TEXT_SECTION.captures(line) else {
            continue;
        };
        let name = &m[1];
        let (size, address, offset) = (
            index(parse_hex(&m[2])?)?,
            parse_hex(&m[3])?,
            index(parse_hex(&m[4])?)?,
        );

        let region_name = format!(
            "{}{}",
            core.name().to_lowercase(),
            if name.starts_with(".ram") {
                "_ram"
            } else {
                "_xip"
            }
        );
        let Some(region) = regions.iter().find(|r| r.name == region_name) else {
            return Err(PackagedError::NoRegion {
                region: region_name,
            }
            .into());
        };
        let runtime = parse_hex(&region.runtime_address)?;

        let relative = address
            .checked_sub(runtime)
            .and_then(|r| usize::try_from(r).ok());
        let Some(relative) = relative
            .filter(|r| size > 0 && offset + size <= data.len() && r + size <= region.body_bytes)
        else {
            return Err(PackagedError::OutsideRegion {
                section: name.into(),
            }
            .into());
        };
        let start = region.payload_header_offset + 32 + relative;
        require(
            payload.get(start..start + size) == Some(&data[offset..offset + size]),
            || PackagedError::Differs {
                section: name.into(),
            },
        )?;

        checked.push(PackagedText {
            section: name.to_string(),
            address: Hex(address),
            bytes: size,
            sha256: sha256(&data[offset..offset + size]),
        });
    }

    let mut sections: Vec<&str> = checked.iter().map(|c| c.section.as_str()).collect();
    sections.sort();
    require(
        checked.len() == 2 && sections == [".ram_image2.text", ".xip_image2.text"],
        || PackagedError::Sections {
            found: sections.iter().map(|s| s.to_string()).collect(),
        },
    )?;
    Ok(checked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    /// An ELF whose two text sections sit at the start of the KM4 regions,
    /// with its bytes, the payload holding them and the regions.
    struct Linked {
        headers: String,
        elf: Vec<u8>,
        payload: Vec<u8>,
        regions: Vec<Image>,
    }

    fn linked() -> Linked {
        let payload = wellfed_image::fixtures::synthetic_payload(9000, 30000, 3);
        let regions = wellfed_image::ota::images(&payload).unwrap();
        let region = |name: &str| regions.iter().find(|r| r.name == name).unwrap();
        let (xip, ram) = (region("km4_xip"), region("km4_ram"));

        // The ELF's sections are copies of the packaged bodies' first bytes.
        let mut elf = vec![0u8; 0x100];
        let xip_offset = elf.len();
        let xip_start = xip.payload_header_offset + 32;
        elf.extend_from_slice(&payload[xip_start..xip_start + 0x40]);
        let ram_offset = elf.len();
        let ram_start = ram.payload_header_offset + 32;
        elf.extend_from_slice(&payload[ram_start..ram_start + 0x20]);

        let headers = format!(
            "  1 .xip_image2.text 00000040  {}  {}  {xip_offset:08x}  2**2\n\
             \x20 2 .ram_image2.text 00000020  {}  {}  {ram_offset:08x}  2**2\n",
            xip.runtime_address.trim_start_matches("0x"),
            xip.runtime_address.trim_start_matches("0x"),
            ram.runtime_address.trim_start_matches("0x"),
            ram.runtime_address.trim_start_matches("0x"),
        );
        Linked {
            headers,
            elf,
            payload,
            regions,
        }
    }

    fn refused(l: &Linked) -> PackagedError {
        match verify_sections(&l.headers, &l.elf, &l.payload, &l.regions, Core::Hp) {
            Err(Error::Packaged(error)) => error,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn text_that_matches_the_package_passes() {
        let l = linked();
        let checked =
            verify_sections(&l.headers, &l.elf, &l.payload, &l.regions, Core::Hp).unwrap();
        assert_eq!(checked.len(), 2);
    }

    /// A stale ELF, one byte off the packaged text, is refused.
    #[test]
    fn text_that_differs_from_the_package_is_refused() {
        let mut l = linked();
        l.elf[0x100] ^= 1;
        assert!(
            matches!(refused(&l), PackagedError::Differs { section } if section == ".xip_image2.text")
        );
    }

    #[test]
    fn text_outside_its_region_is_refused() {
        let mut l = linked();
        l.headers = l.headers.replacen("00000040", "00100000", 1);
        assert!(
            matches!(refused(&l), PackagedError::OutsideRegion { section } if section == ".xip_image2.text")
        );
    }

    /// A listing missing one text section, or a core with no packaged
    /// region, scans nothing for it, so it is refused.
    #[test]
    fn a_missing_section_or_region_is_refused() {
        let mut l = linked();
        l.headers = l.headers.lines().next().unwrap().to_string();
        assert!(
            matches!(refused(&l), PackagedError::Sections { found } if found == [".xip_image2.text"])
        );

        let mut l = linked();
        l.regions.retain(|r| r.name != "km4_ram");
        assert!(matches!(refused(&l), PackagedError::NoRegion { region } if region == "km4_ram"));
    }
}
