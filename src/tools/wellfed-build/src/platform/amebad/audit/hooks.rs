//! The SDK flash-hook overlay and its routing audit in both ELFs.
//!
//! The pinned SDK's public flash-lock entries are renamed to legacy names so
//! the RAM interposer can define the public ones. This proves the selected
//! linked hook routes, not the behavior of an installed protocol provider or
//! complete mediation of raw ROM/indirect flash users.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use wellfed_formats::audit::{HookAudit, HookImage, Route, Symbol};

use super::{
    Archives, Binutils, DIRECT_TRANSFER, HEADING, INSTRUCTION, archive_record, elf, hex, index,
    parse_hex, tool, word,
};
use crate::Result;
use crate::platform::amebad::Core;
use wellfed_formats::require;

/// Why the flash-hook overlay or its routing audit refused.
#[derive(Debug, thiserror::Error)]
pub enum HookError {
    #[error("Expected exactly one pinned SDK definition: {name}")]
    Definition { name: &'static str },
    #[error("Ambiguous SDK hook symbol: {name}")]
    Ambiguous { name: String },
    #[error("Missing SDK hook symbols: {}", names.join(", "))]
    Missing { names: Vec<&'static str> },
    #[error("Hook address is not in initialized ELF bytes")]
    Uninitialized { address: u64 },
    #[error("IPC table is too short")]
    ShortTable { bytes: u64 },
    #[error("Flash IPC table bypasses the public hook")]
    IpcBypass,
    #[error("SDK caller bypasses hook: {caller} -> {legacy}")]
    Bypass {
        caller: String,
        legacy: &'static str,
    },
    /// A literal word holding a legacy entry's address, which an indirect
    /// call could reach it through.
    #[error("SDK code holds a pointer to a legacy hook: {owner} -> {legacy}")]
    Pointer { owner: String, legacy: &'static str },
    #[error("Missing visible wrapper route to legacy hook")]
    NoRoute,
    #[error("SDK hook function is outside application RAM: {name}")]
    FunctionPlacement { name: &'static str },
    #[error("SDK hook state is outside internal application RAM: {name}")]
    StatePlacement { name: &'static str },
}

pub(crate) const SOURCE: &str =
    "component/soc/realtek/amebad/fwlib/ram_common/rtl8721d_flash_ram.c";
pub const RENAMES: [(&str, &str); 3] = [
    ("FLASH_Write_Lock", "wellfed_sdk_flash_legacy_lock"),
    ("FLASH_Write_Unlock", "wellfed_sdk_flash_legacy_unlock"),
    ("FLASH_Write_IPC_Int", "wellfed_sdk_flash_legacy_ipc"),
];
const FUNCTIONS: [&str; 3] = [
    "wellfed_amebad_flash_hooks_install",
    "wellfed_amebad_flash_hooks_armed",
    "hook_fail",
];
const STATE: [&str; 7] = [
    "hook_transport",
    "hook_ticket",
    "hook_last_id",
    "hook_armed",
    "hook_faulted",
    "hook_legacy_locked",
    "hook_lease_held",
];

static SECTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*\d+\s+\.\S+\s+([0-9a-f]+)\s+([0-9a-f]+)\s+[0-9a-f]+\s+([0-9a-f]+)")
        .expect("section pattern")
});
/// A transfer's target symbol, without the `+0x…` offset objdump appends
/// for an address inside it: a transfer into a legacy entry at any offset
/// needs its wrapper as much as one to its start.
static BRANCH_TARGET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b([0-9a-f]+)\s+<([^>+]+)(?:\+0x[0-9a-f]+)?>").expect("branch target pattern")
});

fn functions() -> BTreeSet<&'static str> {
    RENAMES
        .iter()
        .flat_map(|(public, legacy)| [*public, *legacy])
        .chain(FUNCTIONS)
        .collect()
}

/// Rename the three public definitions; each must occur exactly once.
pub(crate) fn overlay(source: &str) -> Result<String> {
    let mut source = source.to_string();
    for (public, legacy) in RENAMES {
        let pattern = Regex::new(&format!(r"(?m)^(void\s+){}(\s*\()", regex::escape(public)))?;
        require(pattern.find_iter(&source).count() == 1, || {
            HookError::Definition { name: public }
        })?;
        source = pattern
            .replace(&source, |c: &regex::Captures| {
                format!("{}{legacy}{}", &c[1], &c[2])
            })
            .into_owned();
    }
    Ok(source)
}

pub fn symbols(elf: &Path, objdump: &Path) -> Result<BTreeMap<String, Symbol>> {
    let mut wanted: BTreeSet<&str> = functions();
    wanted.extend(STATE);
    wanted.insert("ipc_init_config");

    let mut selected = BTreeMap::new();
    for line in tool(objdump, &["-t"], elf)?.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() != 6 || !wanted.contains(fields[5]) {
            continue;
        }
        let name = fields[5].to_string();
        require(!selected.contains_key(&name), || HookError::Ambiguous {
            name: name.clone(),
        })?;
        selected.insert(
            name,
            Symbol {
                address: parse_hex(fields[0])?,
                bytes: parse_hex(fields[4])?,
                kind: fields[2].into(),
                section: fields[3].into(),
            },
        );
    }

    let missing: Vec<&'static str> = wanted
        .iter()
        .filter(|name| !selected.contains_key(**name))
        .copied()
        .collect();
    require(missing.is_empty(), || HookError::Missing { names: missing })?;
    Ok(selected)
}

/// The file offset of `size` initialized bytes at `address` in `elf`.
pub fn file_offset(elf: &Path, objdump: &Path, address: u64, size: u64) -> Result<usize> {
    let length_of_file = fs::metadata(elf)?.len();
    for line in tool(objdump, &["-h"], elf)?.lines() {
        let Some(m) = SECTION.captures(line) else {
            continue;
        };
        let (length, start, offset) = (parse_hex(&m[1])?, parse_hex(&m[2])?, parse_hex(&m[3])?);
        if start <= address && address + size <= start + length {
            let at = offset + address - start;
            if at + size <= length_of_file {
                return index(at);
            }
        }
    }
    Err(HookError::Uninitialized { address }.into())
}

/// The pinned IPC_INIT_TABLE has three 32-bit members per entry; channel
/// 2's handler is at 2*12+4 and must be the public Thumb entry, not legacy.
pub fn verify_ipc_table(
    elf: &Path,
    objdump: &Path,
    names: &BTreeMap<String, Symbol>,
) -> Result<u64> {
    let table = &names["ipc_init_config"];
    require(table.bytes >= 36, || HookError::ShortTable {
        bytes: table.bytes,
    })?;
    let at = file_offset(elf, objdump, table.address + 28, 4)?;
    let public = names["FLASH_Write_IPC_Int"].address | 1;
    match word(&fs::read(elf)?, at) {
        Some(pointer) if pointer == public => Ok(pointer),
        _ => Err(HookError::IpcBypass.into()),
    }
}

/// Each legacy entry's routes.
pub type Routes = BTreeMap<&'static str, Vec<Route>>;

/// Every direct transfer to a legacy entry, which must come from its public
/// wrapper and land at the entry's start: under any condition, width or
/// offset, and through the linker's `__<entry>_veneer`, which reaches the
/// entry for its caller. Only the entry's own branches that land inside its
/// body (its loops and skips) are no way in. A literal
/// word holding an entry's address (Thumb bit set or not) is a pointer an
/// indirect call could use, so only the entry's own veneer, whose body is
/// that word, may hold one. `names` gives the entries' addresses.
pub fn legacy_routes(disassembly: &str, names: &BTreeMap<String, Symbol>) -> Result<Routes> {
    let expected: BTreeMap<&'static str, &'static str> =
        RENAMES.iter().map(|(p, l)| (*l, *p)).collect();
    let range = |name: &'static str| {
        let symbol = names
            .get(name)
            .ok_or(HookError::Missing { names: vec![name] })?;
        let start = symbol.address & !1;
        Ok::<_, crate::Error>(start..start + symbol.bytes)
    };
    let mut addresses = BTreeMap::new();
    let mut bodies = BTreeMap::new();
    let mut wrappers = BTreeMap::new();
    for (legacy, public) in &expected {
        let body = range(legacy)?;
        addresses.insert(body.start, *legacy);
        bodies.insert(*legacy, body);
        wrappers.insert(*legacy, range(public)?);
    }
    let mut routes: Routes = expected.keys().map(|l| (*l, Vec::new())).collect();
    let mut owner: Option<String> = None;

    for line in disassembly.lines() {
        // A heading names the code after it only within its section.
        if line.starts_with("Disassembly of section") {
            owner = None;
            continue;
        }
        if let Some(heading) = HEADING.captures(line) {
            owner = Some(heading[2].to_string());
            continue;
        }
        let Some(m) = INSTRUCTION.captures(line) else {
            continue;
        };
        let caller = owner.as_deref().unwrap_or("None");
        let at = parse_hex(&m[1])?;

        if &m[2] == ".word" {
            let value = m[3].trim();
            let value = parse_hex(value.strip_prefix("0x").unwrap_or(value))?;
            if let Some(legacy) = addresses.get(&(value & !1)) {
                require(caller == veneer(legacy), || HookError::Pointer {
                    owner: caller.into(),
                    legacy,
                })?;
            }
            continue;
        }
        if !DIRECT_TRANSFER.is_match(&m[2]) {
            continue;
        }
        let Some(target) = BRANCH_TARGET.captures(&m[3]) else {
            continue;
        };
        let veneered = target[2]
            .strip_prefix("__")
            .and_then(|name| name.strip_suffix("_veneer"));
        let Some((legacy, public)) = expected.get_key_value(veneered.unwrap_or(&target[2])) else {
            continue;
        };
        let (Some(body), Some(wrapper)) = (bodies.get(legacy), wrappers.get(legacy)) else {
            continue;
        };
        let address = parse_hex(&target[1])?;

        // An entry's own loops and skips, from inside its body to inside
        // it, are no way in. The last heading names the instruction only
        // if its address lies in that symbol's range, so both ends are
        // checked. Any other transfer is a way in, at any offset.
        if caller == *legacy && veneered.is_none() && body.contains(&at) && body.contains(&address)
        {
            continue;
        }
        // Only the wrapper goes in, from inside its own range, and only at
        // the entry's start (or through the veneer, which branches there).
        require(
            caller == *public
                && wrapper.contains(&at)
                && (veneered.is_some() || address == body.start),
            || HookError::Bypass {
                caller: caller.into(),
                legacy,
            },
        )?;
        if let Some(calls) = routes.get_mut(legacy) {
            calls.push(Route {
                caller: caller.to_string(),
                address: format!("0x{}", &m[1]),
            });
        }
    }

    require(routes.values().all(|calls| !calls.is_empty()), || {
        HookError::NoRoute
    })?;
    Ok(routes)
}

/// The linker's veneer for `legacy`: a long branch whose literal is the
/// entry's address.
fn veneer(legacy: &str) -> String {
    format!("__{legacy}_veneer")
}

/// `objects` maps KM0/KM4 to the linked Rust archive containing this code.
pub(crate) fn audit(sdk: &Path, binutils: &Binutils, objects: &Archives) -> Result<HookAudit> {
    let objdump = binutils.objdump();
    let mut images = BTreeMap::new();

    for core in Core::BOTH {
        let elf = elf(sdk, core);
        let names = symbols(&elf, &objdump)?;
        for name in functions() {
            let symbol = &names[name];
            require(
                symbol.kind == "F" && symbol.section == ".ram_image2.text",
                || HookError::FunctionPlacement { name },
            )?;
        }
        for name in STATE {
            let symbol = &names[name];
            require(
                symbol.kind == "O" && symbol.section == ".ram_image2.bss",
                || HookError::StatePlacement { name },
            )?;
        }

        let pointer = verify_ipc_table(&elf, &objdump, &names)?;
        let listing = tool(
            &objdump,
            &["-d", "-j", ".ram_image2.text", "-j", ".xip_image2.text"],
            &elf,
        )?;
        let routes = legacy_routes(&listing, &names)?;

        let archive = archive_record(core, &elf, objects, sdk)?;
        images.insert(
            core.name().to_string(),
            HookImage {
                archive,
                symbols: names,
                legacy_entry_routes: routes
                    .into_iter()
                    .map(|(legacy, routes)| (legacy.to_string(), routes))
                    .collect(),
                ipc_channel_2_handler: hex(pointer),
            },
        );
    }

    Ok(HookAudit {
        images,
        protocol_installed: false,
        global_exclusion_proven: false,
        scope: "Public SDK hooks and IPC table route through RAM interposer; \
                every direct transfer into a legacy entry from outside its own body, under any \
                condition, width or offset or through its linker veneer, originates only in that \
                interposer and lands at the entry's start, and no \
                literal word in the scanned text holds a legacy entry's address but its veneer's. \
                Register-indirect transfers (blx rN, bx rN, ldr pc, mov pc, pop {pc}, tbb, tbh) \
                and data tables outside the scanned text are beyond a listing scan; \
                provider implementation and global exclusion are unproven."
            .into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    const PINNED: &str = "void FLASH_Write_Lock(void)\n{\n}\nvoid FLASH_Write_Unlock(void)\n{\n}\n\
                          void FLASH_Write_IPC_Int(VOID *Data, u32 IrqStatus, u32 ChanNum)\n{\n\
                          \tFLASH_Write_Lock();\n}\n";

    #[test]
    fn definitions_renamed_and_internal_callers_preserved() {
        let changed = overlay(PINNED).unwrap();
        for (public, legacy) in RENAMES {
            assert_eq!(changed.matches(&format!("void {legacy}(")).count(), 1);
            assert!(!changed.contains(&format!("void {public}(")));
        }
        assert!(changed.contains("\tFLASH_Write_Lock();"));
    }

    #[test]
    fn missing_or_duplicate_sdk_definition_is_rejected() {
        for (public, _) in RENAMES {
            let renamed = PINNED.replace(&format!("void {public}("), "void unrelated_hook(");
            for source in [format!("{PINNED}\nvoid {public}(void) {{}}\n"), renamed] {
                assert!(matches!(
                    overlay(&source),
                    Err(Error::Hooks(HookError::Definition { name })) if name == public
                ));
            }
        }
    }

    /// The wrappers and legacy entries, 16 bytes each, where the synthetic
    /// listings put them.
    fn names() -> BTreeMap<String, Symbol> {
        [
            ("FLASH_Write_Lock", 0x1000_0000),
            ("FLASH_Write_Unlock", 0x1000_0010),
            ("FLASH_Write_IPC_Int", 0x1000_0020),
            ("wellfed_sdk_flash_legacy_lock", 0x1000_0100),
            ("wellfed_sdk_flash_legacy_unlock", 0x1000_0200),
            ("wellfed_sdk_flash_legacy_ipc", 0x1000_0300),
        ]
        .into_iter()
        .map(|(name, address)| {
            let symbol = Symbol {
                address,
                bytes: 16,
                kind: "F".into(),
                section: ".ram_image2.text".into(),
            };
            (name.to_owned(), symbol)
        })
        .collect()
    }

    /// An entry's own loop branch lands inside its body and passes; the
    /// same branch from any other function, the wrapper or another legacy
    /// entry included, is a way in and is refused, and so is the entry
    /// branching past its own end.
    #[test]
    fn only_an_entry_s_own_branches_inside_its_body_are_exempt() {
        let rest = "10000010 <FLASH_Write_Unlock>:\n 10000010:\tf000 f800 \tbl\t10000200 <wellfed_sdk_flash_legacy_unlock>\n\
                    10000020 <FLASH_Write_IPC_Int>:\n 10000020:\tf000 b800 \tb.w\t10000300 <wellfed_sdk_flash_legacy_ipc>\n\
                    10000000 <FLASH_Write_Lock>:\n 10000000:\tf000 f800 \tbl\t10000100 <wellfed_sdk_flash_legacy_lock>\n";
        let branch = |from: &str, at: u64, to: &str| {
            format!("{at:08x} <{from}>:\n {at:08x}:\td0fc \tbeq.n\t{to}\n{rest}")
        };
        let into = "1000010a <wellfed_sdk_flash_legacy_lock+0xa>";

        let routes = legacy_routes(
            &branch("wellfed_sdk_flash_legacy_lock", 0x1000_0100, into),
            &names(),
        )
        .unwrap();
        assert_eq!(
            routes["wellfed_sdk_flash_legacy_lock"].len(),
            1,
            "the loop is no route"
        );

        for from in [
            "unmediated_sdk_caller",
            "FLASH_Write_Lock",
            "wellfed_sdk_flash_legacy_unlock",
        ] {
            let error = legacy_routes(&branch(from, 0x2000_0000, into), &names()).unwrap_err();
            assert!(
                matches!(&error, Error::Hooks(HookError::Bypass { caller, .. }) if caller == from),
                "{from}: {error}"
            );
        }

        // Past its own 16 bytes is outside the body, so no loop.
        let beyond = "10000110 <wellfed_sdk_flash_legacy_lock+0x10>";
        let error = legacy_routes(
            &branch("wellfed_sdk_flash_legacy_lock", 0x1000_0100, beyond),
            &names(),
        )
        .unwrap_err();
        assert!(
            matches!(&error, Error::Hooks(HookError::Bypass { caller, .. }) if caller == "wellfed_sdk_flash_legacy_lock"),
            "{error}"
        );
    }

    /// The last heading names an instruction only if the instruction lies
    /// in that symbol's range: an unsymbolled instruction past an entry's
    /// body, or past its wrapper's, or one in a new section before any
    /// heading, is no part of either and is refused.
    #[test]
    fn a_branch_from_past_a_symbol_s_range_is_not_its_own() {
        let rest = "10000010 <FLASH_Write_Unlock>:\n 10000010:\tf000 f800 \tbl\t10000200 <wellfed_sdk_flash_legacy_unlock>\n\
                    10000020 <FLASH_Write_IPC_Int>:\n 10000020:\tf000 b800 \tb.w\t10000300 <wellfed_sdk_flash_legacy_ipc>\n\
                    10000000 <FLASH_Write_Lock>:\n 10000000:\tf000 f800 \tbl\t10000100 <wellfed_sdk_flash_legacy_lock>\n";
        let refused = |listing: String, expected: &str| {
            let error = legacy_routes(&format!("{listing}{rest}"), &names()).unwrap_err();
            assert!(
                matches!(&error, Error::Hooks(HookError::Bypass { caller, .. }) if caller == expected),
                "{error}"
            );
        };

        // Past the entry's 16 bytes, branching back into its body.
        refused(
            "10000100 <wellfed_sdk_flash_legacy_lock>:\n 10000100:\tbf00 \tnop\n \
             10000140:\td0fc \tbeq.n\t1000010a <wellfed_sdk_flash_legacy_lock+0xa>\n"
                .into(),
            "wellfed_sdk_flash_legacy_lock",
        );
        // Past the wrapper's 16 bytes, branching to the entry's start.
        refused(
            "10000000 <FLASH_Write_Lock>:\n 10000000:\tbf00 \tnop\n \
             10000040:\tf000 f800 \tbl\t10000100 <wellfed_sdk_flash_legacy_lock>\n"
                .into(),
            "FLASH_Write_Lock",
        );
        // A new section: the heading before it names nothing in it.
        refused(
            "10000000 <FLASH_Write_Lock>:\n 10000000:\tbf00 \tnop\n\n\
             Disassembly of section .xip_image2.text:\n\n \
             10000004:\tf000 f800 \tbl\t10000100 <wellfed_sdk_flash_legacy_lock>\n"
                .into(),
            "None",
        );
    }

    /// The linker's veneer reaches the entry for its caller, so a transfer
    /// to it needs the wrapper too; and a literal word holding an entry's
    /// address is a pointer only the entry's own veneer may hold.
    #[test]
    fn a_veneer_or_a_literal_pointer_to_a_legacy_entry_needs_its_wrapper() {
        let rest = "10000010 <FLASH_Write_Unlock>:\n 10000010:\tf000 f800 \tbl\t10000200 <wellfed_sdk_flash_legacy_unlock>\n\
                    10000020 <FLASH_Write_IPC_Int>:\n 10000020:\tf000 b800 \tb.w\t10000300 <wellfed_sdk_flash_legacy_ipc>\n";
        let veneer = "20000000 <__wellfed_sdk_flash_legacy_lock_veneer>:\n \
                      20000000:\tf85f f000 \tldr.w\tpc, [pc, #-0]\n \
                      20000004:\t10000101 \t.word\t0x10000101\n";
        let call = |caller: &str| {
            format!(
                "10000000 <{caller}>:\n 10000000:\tf000 f800 \tbl\t20000000 <__wellfed_sdk_flash_legacy_lock_veneer>\n"
            )
        };

        let routes = legacy_routes(
            &format!("{}{rest}{veneer}", call("FLASH_Write_Lock")),
            &names(),
        )
        .unwrap();
        assert_eq!(
            routes["wellfed_sdk_flash_legacy_lock"][0].caller,
            "FLASH_Write_Lock"
        );
        let error = legacy_routes(
            &format!(
                "{}{}{rest}{veneer}",
                call("FLASH_Write_Lock"),
                call("unmediated_sdk_caller")
            ),
            &names(),
        )
        .unwrap_err();
        assert!(
            matches!(&error, Error::Hooks(HookError::Bypass { caller, .. }) if caller == "unmediated_sdk_caller"),
            "{error}"
        );

        for word in ["0x10000101", "0x10000100", "0x10000301"] {
            let pointer = format!(
                "{}{rest}30000000 <unmediated_sdk_caller>:\n 30000000:\t4b01 \tldr\tr3, [pc, #4]\n \
                 30000008:\t10000101 \t.word\t{word}\n",
                call("FLASH_Write_Lock")
            );
            let error = legacy_routes(&format!("{pointer}{veneer}"), &names()).unwrap_err();
            assert!(
                matches!(&error, Error::Hooks(HookError::Pointer { owner, .. }) if owner == "unmediated_sdk_caller"),
                "{word}: {error}"
            );
        }
    }

    /// Each direct transfer shape, under every condition and width, from an
    /// unrelated caller is a bypass; from the wrapper it is a route.
    #[test]
    fn a_conditional_or_compare_branch_to_a_legacy_entry_is_a_bypass() {
        const CONDITIONS: [&str; 17] = [
            "", "eq", "ne", "cs", "hs", "cc", "lo", "mi", "pl", "vs", "vc", "hi", "ls", "ge", "lt",
            "gt", "le",
        ];
        let target = "10000100 <wellfed_sdk_flash_legacy_lock>";
        let wrapped = |caller: &str, mnemonic: &str, operands: &str| {
            format!(
                "10000000 <{caller}>:\n 10000000:\tf000 f800 \t{mnemonic}\t{operands}\n\
                 10000010 <FLASH_Write_Unlock>:\n 10000010:\tf000 f800 \tbl\t10000200 <wellfed_sdk_flash_legacy_unlock>\n\
                 10000020 <FLASH_Write_IPC_Int>:\n 10000020:\tf000 b800 \tb.w\t10000300 <wellfed_sdk_flash_legacy_ipc>\n"
            )
        };
        let mut shapes = Vec::new();
        for base in ["b", "bl", "blx"] {
            for condition in CONDITIONS {
                for width in ["", ".n", ".w"] {
                    shapes.push((format!("{base}{condition}{width}"), target.to_owned()));
                }
            }
        }
        for compare in ["cbz", "cbnz"] {
            shapes.push((compare.to_owned(), format!("r0, {target}")));
        }

        for (mnemonic, operands) in &shapes {
            let routes =
                legacy_routes(&wrapped("FLASH_Write_Lock", mnemonic, operands), &names()).unwrap();
            assert_eq!(
                routes["wellfed_sdk_flash_legacy_lock"][0].caller, "FLASH_Write_Lock",
                "{mnemonic}"
            );

            let error = legacy_routes(
                &wrapped("unmediated_sdk_caller", mnemonic, operands),
                &names(),
            )
            .unwrap_err();
            assert!(
                matches!(
                    &error,
                    Error::Hooks(HookError::Bypass { caller, .. }) if caller == "unmediated_sdk_caller"
                ),
                "{mnemonic}: {error}"
            );
        }

        // A transfer into the entry, past its first instruction.
        let inside = format!(
            "10000000 <unmediated_sdk_caller>:\n 10000000:\tf000 f800 \tbl\t10000104 <wellfed_sdk_flash_legacy_lock+0x4>\n{}",
            wrapped("FLASH_Write_Lock", "bl", target)
        );
        let error = legacy_routes(&inside, &names()).unwrap_err();
        assert!(
            matches!(&error, Error::Hooks(HookError::Bypass { caller, .. }) if caller == "unmediated_sdk_caller"),
            "{error}"
        );
        // The wrapper itself goes in only at the entry's start.
        let error = legacy_routes(
            &wrapped(
                "FLASH_Write_Lock",
                "b.w",
                "10000108 <wellfed_sdk_flash_legacy_lock+0x8>",
            ),
            &names(),
        )
        .unwrap_err();
        assert!(
            matches!(&error, Error::Hooks(HookError::Bypass { caller, .. }) if caller == "FLASH_Write_Lock"),
            "{error}"
        );

        // A register call names no target, and is no direct route.
        assert!(!DIRECT_TRANSFER.is_match("bx"));
        assert!(!DIRECT_TRANSFER.is_match("ldr"));
    }

    #[test]
    fn legacy_entry_bypass_is_rejected() {
        let wrapped = "10000000 <FLASH_Write_Lock>:\n 10000000:\tf000 f800 \tbl\t10000100 <wellfed_sdk_flash_legacy_lock>\n\
                       10000010 <FLASH_Write_Unlock>:\n 10000010:\tf000 f800 \tbl\t10000200 <wellfed_sdk_flash_legacy_unlock>\n\
                       10000020 <FLASH_Write_IPC_Int>:\n 10000020:\tf000 b800 \tb.w\t10000300 <wellfed_sdk_flash_legacy_ipc>\n";
        let routes = legacy_routes(wrapped, &names()).unwrap();
        assert_eq!(
            routes["wellfed_sdk_flash_legacy_ipc"][0].caller,
            "FLASH_Write_IPC_Int"
        );

        let bypass = format!(
            "{wrapped}00000000 <unmediated_sdk_caller>:\n 0:\tf000 f800 \tbl\t10000100 <wellfed_sdk_flash_legacy_lock>\n"
        );
        let error = legacy_routes(&bypass, &names()).unwrap_err();
        assert!(
            matches!(
                &error,
                Error::Hooks(HookError::Bypass { caller, legacy })
                    if caller == "unmediated_sdk_caller"
                        && *legacy == "wellfed_sdk_flash_legacy_lock"
            ),
            "{error}"
        );
    }
}
