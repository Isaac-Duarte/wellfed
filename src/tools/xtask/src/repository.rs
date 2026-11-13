//! The repository check behind `cargo xtask check-repository`.
//!
//! It scans the files Git would publish (tracked plus untracked-but-not-ignored),
//! so ignored local evidence is never read. Every finding is collected into
//! [`Summary::errors`] instead of stopping at the first, so one run reports
//! everything; only failing to resolve or list the checkout is an `Err`.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

use regex::Regex;

// Top-level directories that hold local inputs or agent state.
const UNPUBLISHED: [&str; 6] = ["private", ".venv", "target", ".aws", ".codex", ".agents"];
const EVIDENCE: [&str; 5] = ["bin", "pcap", "pcapng", "img", "apk"];
/// The Home Assistant integration's Python, the one Python under `src/`.
const HOME_ASSISTANT: &str = "src/integrations/homeassistant/";
/// Text the publish checks read: the Rust and C sources, the docs and
/// configuration, the app's Dart, Kotlin, Gradle and Android files, the
/// site's Svelte, scripts and styles, and the shell scripts.
const TEXT: [&str; 22] = [
    "md", "rs", "py", "toml", "json", "yml", "yaml", "txt", "c", "h", "ts", "svg", "dart",
    "svelte", "kt", "kts", "gradle", "xml", "sh", "js", "html", "css",
];
/// Product code, which reaches the motor pins, the watchdog and flash only
/// through the HAL's owners: it may name neither the raw writers nor the
/// `bringup` module that re-exports them.
const PRODUCT: [&str; 2] = ["src/firmware/wellfed/feeder/src/", PRODUCT_CRATES];
/// The product crates, whose top-level `tests/` may name the raw writers:
/// they write the SDK fake's flash to set up what a store reads.
const PRODUCT_CRATES: &str = "src/product/";
/// Bring-up code, which may take the raw writers, but only as `bringup::…`,
/// so the one door stays visible at every call.
const BRINGUP: [&str; 2] = ["src/firmware/wellfed/", "src/sdk/wellfed-probes/src/"];
/// The stock server's library. The bench runs it as its own process; only the
/// shared install driver runs it in-process, since an app cannot
/// spawn the binary. cargo-deny cannot ban every dependent without banning
/// the crate itself.
const STOCK_SERVER: &str = "src/client/wellfed-stock/Cargo.toml";
/// The crates that link the stock server: the install driver the Android
/// app runs, and the bench's binary over the library.
const STOCK_LINKERS: [&str; 2] = [
    "src/client/wellfed-install/Cargo.toml",
    "src/tools/wellfed-stock-server/Cargo.toml",
];
/// The flash client's crate, the audited UART writer: every read and write
/// runs it as its own process, so a fault elsewhere never takes a write down
/// with it. What other crates check of images and reviews is `wellfed-image`.
const FLASH_CLIENT: &str = "src/tools/wellfed-flash/Cargo.toml";
/// The web workspace's dependency policy, whose release-age exceptions
/// each name one exact version.
const PNPM_WORKSPACE: &str = "src/web/pnpm-workspace.yaml";
/// Host crate roots allowed `unsafe`: the virtual feeder worker runs the
/// firmware's own ports on the fake SDK, through their C-shaped interface.
const UNSAFE_HOST_ROOTS: [&str; 1] = ["src/sim/wellfed-sim/src/main.rs"];
/// The one host root whose attribute is conditional: Home Assistant's bridge
/// forbids `unsafe` except when built with PyO3, whose macros expand to it.
const CONDITIONAL_HOST_ROOT: (&str, &str) = (
    "src/integrations/homeassistant/bridge/src/lib.rs",
    r#"#![cfg_attr(not(feature = "python"), forbid(unsafe_code))]"#,
);
/// Sources under `src/` that must not cite a board card.
const CARD_FREE: [&str; 13] = [
    "rs", "c", "h", "toml", "md", "ts", "py", "dart", "svelte", "kt", "sh", "yml", "yaml",
];
/// The one tracked page that may name cards: it tells agents how the
/// ignored boards are kept. Every other page states the fact instead.
const CARD_PAGE: &str = "AGENTS.md";

macro_rules! regex {
    ($name:ident, $pattern:expr) => {
        static $name: LazyLock<Regex> =
            LazyLock::new(|| Regex::new($pattern).expect("check pattern"));
    };
}

regex!(USER_PATH, r"/(?:home|Users|mnt|media)/[A-Za-z0-9_.-]+/");
regex!(DATED, r#"private/[^\s"']*20\d{6}"#);
// A board card's id: a board's prefix (firmware, feature, server, autonomous
// run, notifications, beta, performance) and a number, or a review finding's
// id. Pinned policy names are lowercase and stay.
regex!(
    CARD,
    r"\b(?:FW|FP|SV|AR|NT|BT|PE)\d{1,3}[a-z]?\b|\bF-\d{3}\b"
);

// The raw output writers `wellfed-amebad-sys` marks `unsafe`; reads such
// as `wdg_vendor` and `bank1_read` are not among them.
regex!(
    RAW_WRITER,
    r"(\bbringup::)?\b(gpio_init|gpio_port_write|gpio_write_bit|wdg_init|wdg_cmd|wdg_refresh|storage_erase|storage_program|bank1_erase|bank1_program|bank1_give_way)\b"
);
regex!(BRINGUP_NAME, r"\bbringup\b");
// A manifest line depending on the stock server's crate (not its API client):
// as a key, as a `[dependencies.…]` table, or renamed through `package`.
regex!(
    STOCK_DEPENDENCY,
    r#"(?m)^\s*(?:wellfed-stock\s*[=.]|\[[^\]#]*\.wellfed-stock\s*\]|[^#\n]*\bpackage\s*=\s*"wellfed-stock")"#
);
// The same for the flash client's crate.
regex!(
    FLASH_DEPENDENCY,
    r#"(?m)^\s*(?:wellfed-flash\s*[=.]|\[[^\]#]*\.wellfed-flash\s*\]|[^#\n]*\bpackage\s*=\s*"wellfed-flash")"#
);

regex!(FORBIDS_UNSAFE, r"(?m)^#!\[forbid\(unsafe_code\)\]");
// A host crate's root: the crates under these folders run only on a
// computer or phone, so they have no C interface or MMIO edge.
regex!(
    HOST_ROOT,
    r"^src/(?:(?:client|tools|sim)/[^/]+|integrations/homeassistant/bridge)/src/(?:lib\.rs|main\.rs|bin/[^/]+\.rs|bin/[^/]+/main\.rs)$"
);
regex!(LINK, r"\]\(([^)]+)\)");
regex!(SCHEME, r"^[A-Za-z][A-Za-z0-9+.-]*:");

/// What one run found: the counts for the success line, and every finding.
pub(crate) struct Summary {
    pub files: usize,
    pub links: usize,
    pub errors: Vec<String>,
}

/// Tracked and untracked-but-not-ignored files, as Git lists them.
pub(crate) fn listed(root: &Path) -> io::Result<Vec<String>> {
    let output = Command::new("git")
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .current_dir(root)
        .output()?;

    if !output.status.success() {
        return Err(io::Error::other("git ls-files failed"));
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect())
}

/// Run every check over `root`. An `Err` means the checkout could not be listed;
/// failed checks are reported in [`Summary::errors`].
pub(crate) fn check(root: &Path) -> io::Result<Summary> {
    let root = root.canonicalize()?;
    let names = listed(&root)?;
    let mut summary = Summary {
        files: names.len(),
        links: 0,
        errors: Vec::new(),
    };

    let published: BTreeSet<&str> = names.iter().map(String::as_str).collect();
    for name in &names {
        check_file(&root, name, &published, &mut summary);
    }
    summary
        .errors
        .extend(crate::op_clients::check(&root, &names));
    summary
        .errors
        .extend(crate::doc_checks::check(&root, &names));
    summary.errors.extend(
        crate::diagrams::check(&root, &names)
            .iter()
            .map(|error| crate::chain(error)),
    );

    Ok(summary)
}

/// `published` is every file a clone has, so a link resolves for every
/// reader only to one of them or a folder holding one.
fn check_file(root: &Path, name: &str, published: &BTreeSet<&str>, summary: &mut Summary) {
    let relative = Path::new(name);
    let first = relative
        .components()
        .next()
        .and_then(|c| c.as_os_str().to_str())
        .unwrap_or("");

    if UNPUBLISHED.contains(&first) {
        summary.errors.push(format!(
            "{name}: local inputs or agent state must not be published"
        ));
        return;
    }

    let extension = relative.extension().and_then(|e| e.to_str()).unwrap_or("");
    // The Python tools were ported to Rust and removed (October 23, 2026).
    // Home Assistant's integration is the one Python left: Home Assistant
    // loads only Python, and its protocol logic is the Rust bridge's.
    if extension == "py" && first == "src" && !name.starts_with(HOME_ASSISTANT) {
        summary.errors.push(format!(
            "{name}: first-party tools are Rust; no Python sources under src/"
        ));
    }
    if EVIDENCE.contains(&extension.to_lowercase().as_str()) {
        summary.errors.push(format!(
            "{name}: binary/evidence files need explicit provenance review"
        ));
    }
    if !TEXT.contains(&extension) {
        return;
    }

    let path = root.join(relative);
    let Ok(bytes) = fs::read(&path) else { return };
    let content = String::from_utf8_lossy(&bytes);

    if USER_PATH.is_match(&content) {
        summary
            .errors
            .push(format!("{name}: user-specific absolute path"));
    }
    if DATED.is_match(&content) {
        summary
            .errors
            .push(format!("{name}: dated private fixture path"));
    }
    // Comments and docs carry the constraint; a card's history lives on the
    // boards, which are not published.
    let cited = (first == "src" && CARD_FREE.contains(&extension))
        || (extension == "md" && name != CARD_PAGE);
    if cited && CARD.is_match(&content) {
        summary.errors.push(format!(
            "{name}: cites a board card; state the constraint instead"
        ));
    }

    if extension == "rs" {
        raw_outputs(name, &content, &mut summary.errors);
        if allows_unsafe(name, &content) {
            summary.errors.push(format!(
                "{name}: a host crate root without `#![forbid(unsafe_code)]`"
            ));
        }
        if let Some(line) = compiled_doc_example(&content) {
            summary.errors.push(format!(
                "{name}:{line}: rustdoc would compile this doc example, and `make check` runs no doctests; fence it `ignore` or `text`, or make it a test"
            ));
        }
    }
    if first == "src" && relative.file_name().is_some_and(|f| f == "Cargo.toml") {
        if name != STOCK_SERVER
            && !STOCK_LINKERS.contains(&name)
            && STOCK_DEPENDENCY.is_match(&content)
        {
            summary.errors.push(format!(
                "{name}: links the stock server; run wellfed-stock as a process, or install through wellfed-install"
            ));
        }
        if name != FLASH_CLIENT && FLASH_DEPENDENCY.is_match(&content) {
            summary.errors.push(format!(
                "{name}: links the flash client; run wellfed-flash as a process and use wellfed-image"
            ));
        }
    }

    if name == PNPM_WORKSPACE {
        for entry in unpinned_exceptions(&content) {
            summary.errors.push(format!(
                "{name}: the release-age exception {entry} names no exact version; write name@version"
            ));
        }
    }

    if extension == "md" {
        // Inline links only; external references are provenance links, not fetched.
        for capture in LINK.captures_iter(&content) {
            let target = capture[1].trim().trim_matches(['<', '>']);
            let Some(local) = local_path(target) else {
                continue;
            };
            summary.links += 1;
            let dest = normalize(&path.parent().unwrap_or(root).join(local));
            if !dest
                .strip_prefix(root)
                .is_ok_and(|dest| is_published(dest, published))
            {
                summary
                    .errors
                    .push(format!("{name}: missing/nonlocal link {target}"));
            }
        }
    }
}

/// The `minimumReleaseAgeExclude` entries that are not one package at one
/// exact version: a bare name or a pattern exempts every release to come.
fn unpinned_exceptions(workspace: &str) -> Vec<&str> {
    workspace
        .lines()
        .skip_while(|line| !line.starts_with("minimumReleaseAgeExclude:"))
        .skip(1)
        .take_while(|line| line.starts_with(' ') || line.is_empty())
        .filter_map(|line| line.trim().strip_prefix("- "))
        .filter(|entry| {
            let entry = entry.trim().trim_matches(['"', '\'']);
            let name = entry.strip_prefix('@').unwrap_or(entry);
            !name.split_once('@').is_some_and(|(package, version)| {
                !package.is_empty()
                    && !package.contains('*')
                    && version.starts_with(|c: char| c.is_ascii_digit())
                    && version
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
            })
        })
        .collect()
}

/// Whether `dest`, relative to the root, is a published file or a folder
/// holding one: an ignored file exists only on this disk.
fn is_published(dest: &Path, published: &BTreeSet<&str>) -> bool {
    let Some(dest) = dest.to_str() else {
        return false;
    };
    if dest.is_empty() || published.contains(dest) {
        return true;
    }
    let folder = format!("{dest}/");
    published
        .range(folder.as_str()..)
        .next()
        .is_some_and(|name| name.starts_with(&folder))
}

/// The local file a Markdown link names: no scheme, no host, a nonempty path.
pub(crate) fn local_path(target: &str) -> Option<String> {
    if SCHEME.is_match(target) || target.starts_with("//") {
        return None;
    }
    let path = target.split(['#', '?']).next().unwrap_or("");
    (!path.is_empty()).then(|| unquote(path))
}

/// Percent-decoding, as `urllib.parse.unquote`.
fn unquote(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(value) = text
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(value);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolve symlinks where the path exists, and `..` lexically where it does not.
pub(crate) fn normalize(path: &Path) -> PathBuf {
    if let Ok(real) = path.canonicalize() {
        return real;
    }
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// A host crate's root (`src/lib.rs`, `src/main.rs`, or a binary under
/// `src/bin/`) without the inner attribute on a line of its own, and not a
/// listed exception. The text in a comment or a module's outer attribute
/// doesn't count.
fn allows_unsafe(name: &str, source: &str) -> bool {
    if !HOST_ROOT.is_match(name) || UNSAFE_HOST_ROOTS.contains(&name) {
        return false;
    }
    let (conditional, attribute) = CONDITIONAL_HOST_ROOT;
    let forbids = FORBIDS_UNSAFE.is_match(source)
        || (name == conditional && source.lines().any(|line| line.trim_end() == attribute));
    !forbids
}

/// The line of the first code block in a `///` or `//!` comment that rustdoc
/// would compile as a doctest. `make check` runs none: nextest skips them,
/// and `cargo test --doc` cost every crate a rustdoc pass on each check
/// while no example in the workspace was compiled. So a compiled example
/// would go unchecked; one that shows code fences it `ignore` or `text`.
fn compiled_doc_example(source: &str) -> Option<usize> {
    let mut open = false;
    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        let Some(doc) = trimmed
            .strip_prefix("///")
            .or_else(|| trimmed.strip_prefix("//!"))
        else {
            continue;
        };
        let Some(info) = doc.trim_start().strip_prefix("```") else {
            continue;
        };
        if open {
            open = false;
            continue;
        }

        open = true;
        if compiled_fence(info) {
            return Some(index + 1);
        }
    }
    None
}

/// Whether rustdoc compiles a block with this fence: it does unless a word
/// is `ignore` or names something other than Rust or a rustdoc attribute.
fn compiled_fence(info: &str) -> bool {
    const RUSTDOC: [&str; 6] = [
        "rust",
        "no_run",
        "should_panic",
        "compile_fail",
        "test_harness",
        "standalone_crate",
    ];
    let mut words = info
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|word| !word.is_empty());
    words.all(|word| RUSTDOC.contains(&word) || word.starts_with("edition"))
}

/// Product code names no raw writer and no `bringup`; bring-up code names a
/// raw writer only through `bringup::`. Comments are not code.
fn raw_outputs(name: &str, source: &str, errors: &mut Vec<String>) {
    let product_test = name
        .strip_prefix(PRODUCT_CRATES)
        .and_then(|rest| rest.split_once('/'))
        .is_some_and(|(_, inside)| inside.starts_with("tests/"));
    let product = !product_test && PRODUCT.iter().any(|root| name.starts_with(root));
    let bringup = !product && BRINGUP.iter().any(|root| name.starts_with(root));
    if !product && !bringup {
        return;
    }

    for line in source.lines() {
        let code = line.split("//").next().unwrap_or("");
        if product && BRINGUP_NAME.is_match(code) {
            errors.push(format!(
                "{name}: product code may not use the bringup module"
            ));
        }
        for capture in RAW_WRITER.captures_iter(code) {
            let writer = &capture[2];
            if product {
                errors.push(format!(
                    "{name}: product code may not call {writer}; use the HAL's owner"
                ));
            } else if capture.get(1).is_none() {
                errors.push(format!(
                    "{name}: bring-up code calls {writer} only as bringup::{writer}"
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_and_finding_ids_are_caught() {
        // Built from halves, so this file does not cite one itself.
        for (prefix, number) in [
            ("see FW", "12"),
            ("NT", "3a"),
            ("BT", "07"),
            ("PE", "25"),
            ("F-", "117"),
            ("SV", "69"),
        ] {
            let text = format!("{prefix}{number}");
            assert!(CARD.is_match(&text), "{text}");
        }
        for text in ["sha256", "F-1", "PA_12", "FWD3"] {
            assert!(!CARD.is_match(text), "{text}");
        }
        assert!(DATED.is_match(&format!("private/captures-{}/x", "20261112")));
    }

    #[test]
    fn a_host_crate_root_forbids_unsafe() {
        let forbidden = "//! A crate.\n\n#![forbid(unsafe_code)]\n";
        for root in [
            "src/client/wellfed-connect/src/lib.rs",
            "src/sim/wellfed-feeder-sim/src/main.rs",
            "src/tools/wellfed-bench/src/bin/report.rs",
            "src/client/wellfed-model/src/bin/gen/main.rs",
            "src/integrations/homeassistant/bridge/src/lib.rs",
        ] {
            assert!(allows_unsafe(root, "//! A crate.\n"), "{root}");
            assert!(!allows_unsafe(root, forbidden), "{root}");
            for only_the_text in [
                "//! Roots carry #![forbid(unsafe_code)].\n",
                "// #![forbid(unsafe_code)]\n",
                "#[forbid(unsafe_code)]\nmod inner;\n",
                "#![cfg_attr(any(), forbid(unsafe_code))]\n",
            ] {
                assert!(
                    allows_unsafe(root, only_the_text),
                    "{root}: {only_the_text}"
                );
            }
        }

        let conditional = "#![cfg_attr(not(feature = \"python\"), forbid(unsafe_code))]\n";
        assert!(!allows_unsafe(
            "src/integrations/homeassistant/bridge/src/lib.rs",
            conditional
        ));
        assert!(allows_unsafe(
            "src/client/wellfed-connect/src/lib.rs",
            conditional
        ));

        for not_held in [
            "src/client/wellfed-connect/src/discover.rs",
            "src/server/wellfed-store/src/lib.rs",
            "src/sdk/wellfed-core/src/lib.rs",
            "src/firmware/wellfed/feeder/src/lib.rs",
            "src/sim/wellfed-sim/src/main.rs",
        ] {
            assert!(!allows_unsafe(not_held, "//! A crate.\n"), "{not_held}");
        }
    }

    #[test]
    fn markdown_link_targets() {
        assert_eq!(local_path("docs/a.md#part").as_deref(), Some("docs/a.md"));
        assert_eq!(local_path("a%20b.md").as_deref(), Some("a b.md"));
        for skipped in ["https://example.com/x", "mailto:x@y", "#anchor", "//host/x"] {
            assert_eq!(local_path(skipped), None, "{skipped}");
        }
    }

    #[test]
    fn raw_outputs_rules() {
        let mut errors = Vec::new();
        let feeder = "src/firmware/wellfed/feeder/src/app.rs";
        raw_outputs(
            feeder,
            "// gpio_port_write is the board's\nlet v = sdk::wdg_vendor();\nlet e = h.storage_erases;\n",
            &mut errors,
        );
        assert!(errors.is_empty(), "{errors:?}");

        raw_outputs(
            feeder,
            "unsafe { wellfed_sdk::hal::sdk::gpio_port_write(0, MASK, 0) };\nuse wellfed_sdk::hal::bringup;\n",
            &mut errors,
        );
        raw_outputs(
            "src/sdk/wellfed-probes/src/buttons.rs",
            "unsafe { bringup::gpio_init(pin, false) };\nunsafe { sdk::wdg_refresh() };\n",
            &mut errors,
        );
        raw_outputs(
            "src/sdk/wellfed-hal/src/board.rs",
            "unsafe { sdk::gpio_port_write(0, 0, 0) };\n",
            &mut errors,
        );
        raw_outputs(
            "src/product/wellfed-feeder-runtime/tests/writer.rs",
            "unsafe { sdk::storage_erase(sector) };\n",
            &mut errors,
        );
        raw_outputs(
            "src/product/wellfed-feeder-runtime/src/writer.rs",
            "unsafe { sdk::storage_erase(sector) };\n",
            &mut errors,
        );
        raw_outputs(
            "src/product/wellfed-feeder-runtime/src/tests/fake.rs",
            "unsafe { sdk::storage_erase(sector) };\n",
            &mut errors,
        );

        assert_eq!(
            errors,
            [
                "src/firmware/wellfed/feeder/src/app.rs: product code may not call gpio_port_write; use the HAL's owner",
                "src/firmware/wellfed/feeder/src/app.rs: product code may not use the bringup module",
                "src/sdk/wellfed-probes/src/buttons.rs: bring-up code calls wdg_refresh only as bringup::wdg_refresh",
                "src/product/wellfed-feeder-runtime/src/writer.rs: product code may not call storage_erase; use the HAL's owner",
                "src/product/wellfed-feeder-runtime/src/tests/fake.rs: product code may not call storage_erase; use the HAL's owner",
            ]
        );
    }

    #[test]
    fn published_paths_links_and_sources() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("src/tools")).unwrap();
        fs::write(root.join("src/tools/a.py"), "print()\n").unwrap();
        fs::write(
            root.join("README.md"),
            "[ok](src/tools/a.py) [gone](missing.md) [web](https://x)\n",
        )
        .unwrap();

        let mut summary = Summary {
            files: 0,
            links: 0,
            errors: Vec::new(),
        };
        let card = ["FW", "16"].concat();
        fs::write(root.join("src/tools/b.rs"), format!("// {card}\n")).unwrap();
        fs::write(root.join("src/tools/c.rs"), "// fw16-pulse-v1\n").unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join("docs/page.md"), format!("See {card}.\n")).unwrap();
        fs::write(root.join("AGENTS.md"), format!("Board card {card}.\n")).unwrap();
        fs::create_dir_all(root.join("src/tools/page")).unwrap();
        fs::write(
            root.join("src/tools/page/Cargo.toml"),
            "[dependencies]\nwellfed-stock-extra = { path = \"../wellfed-stock-extra\" }\nwellfed-stock.workspace = true\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("src/tools/client")).unwrap();
        fs::write(
            root.join("src/tools/client/Cargo.toml"),
            "[dependencies]\nwellfed-stock-extra = { path = \"../wellfed-stock-extra\" }\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("src/tools/auditor")).unwrap();
        fs::write(
            root.join("src/tools/auditor/Cargo.toml"),
            "[dependencies]\nwellfed-image.workspace = true\n\n[dev-dependencies]\nwellfed-flash = { path = \"../wellfed-flash\" }\n",
        )
        .unwrap();
        let names = [
            "README.md",
            "private/x.json",
            "dump.bin",
            "src/tools/a.py",
            "src/integrations/homeassistant/custom_components/wellfed/sensor.py",
            "src/tools/b.rs",
            "src/tools/c.rs",
            "docs/page.md",
            "AGENTS.md",
            "src/tools/page/Cargo.toml",
            "src/tools/client/Cargo.toml",
            "src/tools/auditor/Cargo.toml",
        ];
        let published = BTreeSet::from(names);
        for name in names {
            check_file(&root, name, &published, &mut summary);
        }

        assert_eq!(summary.links, 2);
        assert_eq!(
            summary.errors,
            [
                "README.md: missing/nonlocal link missing.md",
                "private/x.json: local inputs or agent state must not be published",
                "dump.bin: binary/evidence files need explicit provenance review",
                "src/tools/a.py: first-party tools are Rust; no Python sources under src/",
                "src/tools/b.rs: cites a board card; state the constraint instead",
                "docs/page.md: cites a board card; state the constraint instead",
                "src/tools/page/Cargo.toml: links the stock server; run wellfed-stock as a process, or install through wellfed-install",
                "src/tools/auditor/Cargo.toml: links the flash client; run wellfed-flash as a process and use wellfed-image",
            ]
        );
    }

    #[test]
    fn a_release_age_exception_names_one_exact_version() {
        let workspace = "\
minimumReleaseAge: 10080
minimumReleaseAgeExclude:
  - vite@8.3.5
  - \"@oxc-project/types@0.155.0\"
  # A comment.
  - \"@rolldown/binding-*\"
  - rolldown
  - postcss@^8
savePrefix: \"\"
";
        assert_eq!(
            unpinned_exceptions(workspace),
            ["\"@rolldown/binding-*\"", "rolldown", "postcss@^8"]
        );
        assert!(unpinned_exceptions("minimumReleaseAge: 10080\n").is_empty());
    }

    /// A link resolves for every reader only to a file a clone has: one
    /// that exists here but is ignored (private evidence, agent state, a
    /// build output) is as missing as one that does not exist.
    #[test]
    fn a_link_to_an_ignored_file_is_missing() {
        let checkout = tempfile::tempdir().unwrap();
        let root = checkout.path().canonicalize().unwrap();
        for folder in ["private", "docs", ".agents"] {
            fs::create_dir_all(root.join(folder)).unwrap();
        }
        fs::write(root.join("private/x.json"), "{}").unwrap();
        fs::write(root.join(".agents/RESUME.md"), "x").unwrap();
        fs::write(root.join("docs/page.md"), "x").unwrap();
        fs::write(
            root.join("README.md"),
            "[x](private/x.json) [resume](.agents/RESUME.md) [page](docs/page.md) [docs](docs) [root](.)\n",
        )
        .unwrap();

        let mut summary = Summary {
            files: 0,
            links: 0,
            errors: Vec::new(),
        };
        let published = BTreeSet::from(["README.md", "docs/page.md"]);
        check_file(&root, "README.md", &published, &mut summary);

        assert_eq!(summary.links, 5);
        assert_eq!(
            summary.errors,
            [
                "README.md: missing/nonlocal link private/x.json",
                "README.md: missing/nonlocal link .agents/RESUME.md",
            ]
        );
    }

    #[test]
    fn a_doc_example_rustdoc_would_compile_is_named_by_its_line() {
        let fenced =
            |info: &str| format!("//! Intro.\n//!\n//! ```{info}\n//! let x = 1;\n//! ```\n");

        for compiled in ["", "rust", "no_run", "should_panic", "rust,edition2024"] {
            assert_eq!(
                compiled_doc_example(&fenced(compiled)),
                Some(3),
                "{compiled:?}"
            );
        }
        for shown in ["ignore", "text", "rust,ignore", "sh", "mermaid", "toml"] {
            assert_eq!(compiled_doc_example(&fenced(shown)), None, "{shown:?}");
        }
        assert_eq!(
            compiled_doc_example("/// ```text\n/// a\n/// ```\n///\n/// ```\n/// b();\n/// ```\n"),
            Some(5),
            "a closing fence is not an opening one"
        );
        assert_eq!(
            compiled_doc_example("// ```\nlet s = \"```\";\n"),
            None,
            "only doc comments hold doctests"
        );
    }

    #[test]
    fn process_only_crates_are_found_in_every_manifest_form() {
        for manifest in [
            "wellfed-flash.workspace = true",
            "[dev-dependencies.wellfed-flash]\npath = \"../wellfed-flash\"",
            "writer = { package = \"wellfed-flash\", path = \"../wellfed-flash\" }",
        ] {
            assert!(FLASH_DEPENDENCY.is_match(manifest), "{manifest}");
        }
        for manifest in [
            "wellfed-image.workspace = true",
            "# wellfed-flash = { path = \"../wellfed-flash\" }",
            "description = \"runs wellfed-flash as a process\"",
        ] {
            assert!(!FLASH_DEPENDENCY.is_match(manifest), "{manifest}");
        }

        assert!(STOCK_DEPENDENCY.is_match("[dependencies.wellfed-stock]"));
        for manifest in [
            "[dependencies.wellfed-stock-extra]",
            "api = { package = \"wellfed-stock-extra\", path = \"x\" }",
        ] {
            assert!(!STOCK_DEPENDENCY.is_match(manifest), "{manifest}");
        }
    }
}
