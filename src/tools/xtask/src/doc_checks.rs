//! The staleness checks over the Markdown the repository publishes: commands
//! and flags, make targets, paths, status words, time-bound wording and link
//! anchors.
//!
//! Each is deliberately narrow. A raw scan of the docs for paths gives hundreds
//! of false hits, and a check people learn to ignore protects nothing. So each
//! looks only at spelled-out code (or, for status and wording, fixed phrases),
//! and each exemption here is named. `## Evidence` sections are exempt from
//! all but anchors: they record what was true on their date.
//!
//! What these cannot catch is a wrong statement about the code. That stays a
//! matter of updating the page with the change ([writing guide]).
//!
//! [writing guide]: ../../../docs/development/writing.md

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::Path;
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;

use crate::clap_tree::{self, Node, TOOLS};
use crate::repository::{local_path, normalize};

/// The one page that holds status: everything else defers to it.
const ROADMAP: &str = "docs/roadmap.md";
/// Its one other home, the column of models we support.
const SUPPORTED_DEVICES: &str = "docs/devices/README.md";
/// The page that defines the status words and these rules, and so names them.
const WRITING_GUIDE: &str = "docs/development/writing.md";
const STATUS_PAGES: [&str; 3] = [ROADMAP, SUPPORTED_DEVICES, WRITING_GUIDE];
const WORDING_PAGES: [&str; 2] = [ROADMAP, WRITING_GUIDE];

const STATUS_WORDS: [&str; 4] = ["works", "partial", "experimental", "not started"];

/// Wording that is true for a while and then quietly wrong.
const TIME_BOUND: [&str; 6] = [
    "come next",
    "comes next",
    "on the way",
    "not built",
    "aren't built",
    "isn't built",
];

/// A dated promise kept on purpose, as `(page, phrase)`.
const KEPT_DATES: [(&str, &str); 1] = [("AGENTS.md", "until October 26")];

/// A command named on a page as history, whose tool is gone: `(page, tool)`.
const RETIRED_TOOLS: [(&str, &str); 1] = [("AGENTS.md", "wellfed-local")];

/// Shell fences hold commands; a `text` fence may just start with a crate name.
const SHELL_FENCES: [&str; 5] = ["sh", "bash", "shell", "console", "zsh"];

/// Top-level folders that make a slashed word a repository path whatever it
/// ends in.
const TOP_LEVEL: [&str; 3] = ["src", "docs", "assets"];
/// Extensions that make a word a repository path. `json` needs a slash, since
/// a bare one is usually a runtime file.
const SLASHED_EXTENSIONS: [&str; 16] = [
    "rs", "md", "toml", "svelte", "dart", "ts", "mjs", "c", "h", "sh", "svg", "kts", "gradle",
    "json", "lock", "blend",
];
const BARE_EXTENSIONS: [&str; 7] = ["rs", "md", "svelte", "dart", "mjs", "sh", "kts"];

/// Words that look like a repository path but aren't in the repository.
/// Folders that exist only once something runs, and output that Git ignores.
const RUNTIME_PREFIXES: [&str; 9] = [
    "private/",
    ".agents/",
    "src/fuzz/artifacts/",
    "target/",
    "./target/",
    "dist/",
    "assets/installer/",
    "src/fuzz/target/",
    "src/fuzz/corpus/",
];
/// Named files of the vendor's SDK and of stock's, which are never tracked.
const VENDOR_FILES: [&str; 1] = ["swlib/string/rand.c"];

static LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\]\(([^)]+)\)").expect("the link pattern is valid"));
static SPAN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"`([^`]+)`").expect("the code-span pattern is valid"));
static STATUS_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[\s*])Status:(?:\s|\*)").expect("the status-line pattern is valid")
});
static UNTIL_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"until (January|February|March|April|May|June|July|August|September|October|November|December) (\d{1,2})\b(?:, (\d{4}))?",
    )
    .expect("the until-date pattern is valid")
});
static PATH_WORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z0-9_.@+-]+(?:/[A-Za-z0-9_.@+-]*)*$").expect("the path pattern is valid")
});
static MAKE_RULE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([A-Za-z0-9_.%-]+(?:[ \t]+[A-Za-z0-9_.%-]+)*)[ \t]*:(.?)")
        .expect("the make-rule pattern is valid")
});
static HTML_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<[a-z]+[^>]*\b(?:id|name)="([^"]+)""#).expect("the html-id pattern is valid")
});
static HEADING_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]*)\]\([^)]*\)").expect("the link pattern is valid"));

/// Where a line of Markdown sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Block<'a> {
    Prose,
    Fence(&'a str),
}

#[derive(Clone, Copy, Debug)]
struct Line<'a> {
    number: usize,
    text: &'a str,
    block: Block<'a>,
    /// Inside a `## Evidence` section.
    evidence: bool,
}

/// A calendar date, compared as written.
type Date = (i64, u32, u32);

/// A page's lines, with its fences and `## Evidence` sections marked. A fence
/// closes only on a run at least as long as the one that opened it, so a page
/// can show a fence inside a fence.
fn lines(content: &str) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    let mut fence: Option<(usize, &str)> = None;
    let mut evidence = false;

    for (index, text) in content.lines().enumerate() {
        let trimmed = text.trim_start();
        let ticks = trimmed.chars().take_while(|&c| c == '`').count();
        let number = index + 1;

        if let Some((open, language)) = fence {
            let closes = ticks >= open && trimmed[ticks..].trim().is_empty();
            out.push(Line {
                number,
                text,
                block: Block::Fence(language),
                evidence,
            });
            if closes {
                fence = None;
            }
            continue;
        }
        if ticks >= 3 {
            let language = trimmed[ticks..].trim();
            fence = Some((ticks, language));
            out.push(Line {
                number,
                text,
                block: Block::Fence(language),
                evidence,
            });
            continue;
        }

        if let Some((level, title)) = heading(text)
            && level <= 2
        {
            evidence = level == 2 && title == "Evidence";
        }
        out.push(Line {
            number,
            text,
            block: Block::Prose,
            evidence,
        });
    }
    out
}

fn heading(text: &str) -> Option<(usize, &str)> {
    let level = text.chars().take_while(|&c| c == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    text[level..]
        .strip_prefix(' ')
        .map(|title| (level, title.trim()))
}

/// Every command line a page spells out outside its Evidence: code spans, and
/// the lines of its shell fences, with continuation lines joined.
fn commands<'a>(lines: &[Line<'a>]) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut held: Option<(usize, String)> = None;

    for line in lines.iter().filter(|line| !line.evidence) {
        match line.block {
            Block::Prose => {
                for capture in SPAN.captures_iter(line.text) {
                    out.push((line.number, capture[1].replace("\\|", "|")));
                }
            }
            Block::Fence(language) if SHELL_FENCES.contains(&language) => {
                let text = line.text.trim();
                let (number, mut joined) =
                    held.take().unwrap_or_else(|| (line.number, String::new()));
                if let Some(start) = text.strip_suffix('\\') {
                    joined.push_str(start);
                    joined.push(' ');
                    held = Some((number, joined));
                } else {
                    joined.push_str(text);
                    out.push((number, joined));
                }
            }
            Block::Fence(_) => {}
        }
    }
    out
}

/// The words of a command up to where the shell would end it.
fn words(command: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for word in command.split_whitespace() {
        if matches!(word, "|" | "||" | "&&" | ";" | ">" | ">>" | "2>&1" | "#") {
            break;
        }
        out.push(word);
    }
    out
}

/// The program a command runs and its arguments, past `sudo`, `$`, variables,
/// a `target/…/` prefix and `cargo run -p NAME --`.
fn program<'a, 'b>(all: &'b [&'a str]) -> Option<(&'a str, &'b [&'a str])> {
    let mut rest = all;
    while let [first, tail @ ..] = rest {
        let is_variable = first.contains('=') && !first.starts_with('-');
        if *first == "sudo" || *first == "$" || is_variable {
            rest = tail;
        } else {
            break;
        }
    }

    if let ["cargo", "run", "-p", name, "--", tail @ ..] = rest {
        return Some((name, tail));
    }
    let [first, tail @ ..] = rest else {
        return None;
    };
    let name = first.strip_prefix("./").unwrap_or(first);
    let name = name
        .strip_prefix("target/")
        .map_or(name, |built| built.rsplit('/').next().unwrap_or(built));
    Some((name, tail))
}

/// Check every command a page spells out against the tool's command line.
fn check_commands(
    name: &str,
    lines: &[Line<'_>],
    trees: &mut Trees<'_>,
    crates: &BTreeSet<String>,
    errors: &mut Vec<String>,
) {
    for (number, command) in commands(lines) {
        let all = words(&command);
        let Some((program, arguments)) = program(&all) else {
            continue;
        };

        if let Some(tree) = trees.get(program, errors) {
            if let Err(refusal) = tree.verify(program, arguments) {
                errors.push(format!("{name}:{number}: `{command}`: {refusal}"));
            }
            continue;
        }

        let names_a_tool = program.starts_with("wellfed-")
            && is_plain_word(program)
            && !crates.contains(program)
            && arguments
                .first()
                .is_some_and(|word| word.starts_with("--") || is_plain_word(word));
        if names_a_tool && !RETIRED_TOOLS.contains(&(name, program)) && trees.is_unknown(program) {
            errors.push(format!(
                "{name}:{number}: `{command}`: no tool called `{program}`"
            ));
        }
    }
}

fn is_plain_word(word: &str) -> bool {
    word.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Each tool's command line, read the first time a page names it.
struct Trees<'a> {
    root: &'a Path,
    names: &'a [String],
    read: HashMap<&'static str, Option<Node>>,
}

impl Trees<'_> {
    fn is_unknown(&self, program: &str) -> bool {
        !TOOLS.iter().any(|tool| tool.name == program)
    }

    /// `None` for a program that is no tool of ours, or a tool that could not
    /// be read, which reports once.
    fn get(&mut self, program: &str, errors: &mut Vec<String>) -> Option<&Node> {
        let tool = TOOLS.iter().find(|tool| tool.name == program)?;
        let (root, names) = (self.root, self.names);
        self.read
            .entry(tool.name)
            .or_insert_with(|| match clap_tree::read(root, names, *tool) {
                Ok(node) => Some(node),
                Err(error) => {
                    errors.push(format!("xtask: {}", crate::chain(&error)));
                    None
                }
            })
            .as_ref()
    }
}

/// Each `make` a page spells out names a target in that Makefile.
fn check_make(
    name: &str,
    lines: &[Line<'_>],
    makefiles: &mut dyn FnMut(&str) -> Option<BTreeSet<String>>,
    errors: &mut Vec<String>,
) {
    for (number, command) in commands(lines) {
        let all = words(&command);
        let Some((program, mut arguments)) = program(&all) else {
            continue;
        };
        if program != "make" {
            continue;
        }

        let mut directory = "";
        if let ["-C", folder, rest @ ..] = arguments {
            directory = folder;
            arguments = rest;
        }
        let Some(targets) = makefiles(directory) else {
            errors.push(format!(
                "{name}:{number}: `{command}`: no Makefile in `{directory}`"
            ));
            continue;
        };

        for word in arguments {
            let word = word.trim_matches(|c| "[](),;:.'\"".contains(c));
            let is_target = is_plain_word(word) && !word.is_empty();
            if is_target && !targets.contains(word) {
                errors.push(format!(
                    "{name}:{number}: `{command}`: no make target `{word}`"
                ));
            }
        }
    }
}

/// The targets a Makefile's rules name.
fn make_targets(makefile: &str) -> BTreeSet<String> {
    let mut targets = BTreeSet::new();
    for line in makefile.lines() {
        let Some(capture) = MAKE_RULE.captures(line) else {
            continue;
        };
        if &capture[2] == "=" {
            continue;
        }
        targets.extend(capture[1].split_whitespace().map(str::to_string));
    }
    targets
}

/// Whether a code span is a repository path worth checking.
fn path_word(span: &str) -> Option<&str> {
    let word = span.split_once(':').map_or(span, |(path, line)| {
        if line.chars().all(|c| c.is_ascii_digit()) {
            path
        } else {
            span
        }
    });
    if !PATH_WORD.is_match(word) {
        return None;
    }
    if RUNTIME_PREFIXES
        .iter()
        .any(|prefix| word.starts_with(prefix))
    {
        return None;
    }

    let last = word
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(word);
    let extension = last.rsplit_once('.');
    let has = |list: &[&str]| {
        extension.is_some_and(|(stem, extension)| !stem.is_empty() && list.contains(&extension))
    };
    let first = word.split('/').next().unwrap_or(word);
    let wanted = if word.contains('/') {
        TOP_LEVEL.contains(&first) || has(&SLASHED_EXTENSIONS)
    } else {
        has(&BARE_EXTENSIONS)
    };
    wanted.then_some(word)
}

/// A shorthand path resolves when it is the tail of some file or folder.
fn path_resolves(word: &str, names: &[String]) -> bool {
    let word = word.strip_prefix("./").unwrap_or(word);
    let word = word.trim_end_matches('/');
    let tail = format!("/{word}");
    let folder = format!("{word}/");
    let inner = format!("/{word}/");
    names.iter().any(|name| {
        name == word || name.ends_with(&tail) || name.starts_with(&folder) || name.contains(&inner)
    })
}

/// Each path a page spells in a code span resolves.
fn check_paths(name: &str, lines: &[Line<'_>], names: &[String], errors: &mut Vec<String>) {
    for line in lines.iter().filter(|line| !line.evidence) {
        if line.block != Block::Prose {
            continue;
        }
        for capture in SPAN.captures_iter(line.text) {
            let Some(word) = path_word(&capture[1]) else {
                continue;
            };
            if !VENDOR_FILES.contains(&word) && !path_resolves(word, names) {
                errors.push(format!(
                    "{name}:{}: `{word}` is not in the repository",
                    line.number
                ));
            }
        }
    }
}

/// Status lives on the roadmap: no `Status:` line, `Status` heading or column,
/// or a table cell holding a status word, anywhere else.
fn check_status(name: &str, lines: &[Line<'_>], errors: &mut Vec<String>) {
    if STATUS_PAGES.contains(&name) {
        return;
    }
    for line in lines.iter().filter(|line| !line.evidence) {
        if line.block != Block::Prose {
            continue;
        }
        let number = line.number;

        if heading(line.text).is_some_and(|(_, title)| title.eq_ignore_ascii_case("status")) {
            errors.push(format!(
                "{name}:{number}: a status section; link the roadmap instead"
            ));
            continue;
        }

        if line.text.trim_start().starts_with('|') {
            for cell in cells(line.text) {
                let plain = cell.replace("**", "");
                let plain = plain.trim().to_lowercase();
                let is_word = STATUS_WORDS.iter().any(|word| {
                    plain == *word
                        || plain.starts_with(&format!("{word}:"))
                        || plain.starts_with(&format!("{word},"))
                });
                if is_word || plain == "status" {
                    errors.push(format!(
                        "{name}:{number}: a status table cell `{cell}`; link the roadmap instead"
                    ));
                }
            }
            continue;
        }

        let prose = SPAN.replace_all(line.text, "");
        if STATUS_LINE.is_match(&prose) {
            errors.push(format!(
                "{name}:{number}: a `Status:` line; link the roadmap instead"
            ));
        }
    }
}

fn cells(row: &str) -> Vec<String> {
    row.replace("\\|", "\u{1}")
        .split('|')
        .map(|cell| cell.replace('\u{1}', "|").trim().to_string())
        .filter(|cell| !cell.is_empty())
        .collect()
}

/// Wording that goes stale by itself: refused outside the roadmap.
fn check_wording(name: &str, lines: &[Line<'_>], today: Date, errors: &mut Vec<String>) {
    if WORDING_PAGES.contains(&name) {
        return;
    }
    for line in lines.iter().filter(|line| !line.evidence) {
        if line.block != Block::Prose {
            continue;
        }
        let number = line.number;

        let lower = line.text.to_lowercase();
        for phrase in TIME_BOUND {
            if lower.contains(phrase) {
                errors.push(format!(
                    "{name}:{number}: \"{phrase}\" goes stale; say what is true now"
                ));
            }
        }

        for capture in UNTIL_DATE.captures_iter(line.text) {
            if KEPT_DATES.contains(&(
                name,
                &capture[0][..capture[0].find(',').unwrap_or(capture[0].len())],
            )) {
                continue;
            }
            if let Some(date) = until_date(&capture, today.0)
                && date < today
            {
                errors.push(format!("{name}:{number}: \"{}\" has passed", &capture[0]));
            }
        }
    }
}

fn until_date(capture: &regex::Captures<'_>, this_year: i64) -> Option<Date> {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let month = MONTHS.iter().position(|month| *month == &capture[1])?;
    let month = u32::try_from(month).ok()? + 1;
    let day = capture[2].parse().ok()?;
    let year = capture
        .get(3)
        .map_or(Some(this_year), |year| year.as_str().parse().ok())?;
    Some((year, month, day))
}

/// A heading's anchor as GitHub writes it.
fn slug(title: &str) -> String {
    let title = HEADING_LINK.replace_all(title, "$1");
    let mut out = String::new();
    for c in title.trim().chars() {
        if c.is_alphanumeric() || c == '-' || c == '_' {
            out.extend(c.to_lowercase());
        } else if c == ' ' {
            out.push('-');
        }
    }
    out
}

/// The anchors a page offers: its headings, numbered when one repeats, and
/// any HTML `id` or `name`.
fn anchors(content: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for line in lines(content) {
        if line.block != Block::Prose {
            continue;
        }
        for capture in HTML_ID.captures_iter(line.text) {
            out.insert(capture[1].to_string());
        }
        let Some((_, title)) = heading(line.text) else {
            continue;
        };
        let base = slug(title);
        let count = seen.entry(base.clone()).or_insert(0);
        out.insert(if *count == 0 {
            base.clone()
        } else {
            format!("{base}-{count}")
        });
        *count += 1;
    }
    out
}

/// `#line` links go to a line of a file, which GitHub resolves for itself.
fn is_line_anchor(anchor: &str) -> bool {
    anchor
        .strip_prefix('L')
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
}

/// Each link's `#anchor` resolves, in the page it names or in this one.
/// Evidence entries are checked too: a record's link should still lead there.
fn check_anchors(
    root: &Path,
    name: &str,
    content: &str,
    cache: &mut HashMap<String, Option<BTreeSet<String>>>,
    errors: &mut Vec<String>,
) {
    let page = root.join(name);
    for line in lines(content) {
        if line.block != Block::Prose {
            continue;
        }
        let prose = SPAN.replace_all(line.text, "");
        for capture in LINK.captures_iter(&prose) {
            let target = capture[1].trim().trim_matches(['<', '>']);
            let Some((path, anchor)) = target.split_once('#') else {
                continue;
            };
            if anchor.is_empty() || is_line_anchor(anchor) || target.contains("://") {
                continue;
            }

            let destination = if path.is_empty() {
                name.to_string()
            } else {
                let Some(local) = local_path(target) else {
                    continue;
                };
                let joined = normalize(&page.parent().unwrap_or(root).join(local));
                let Ok(relative) = joined.strip_prefix(root) else {
                    continue;
                };
                relative.to_string_lossy().into_owned()
            };
            if !destination.ends_with(".md") {
                continue;
            }

            let offered = cache.entry(destination.clone()).or_insert_with(|| {
                fs::read_to_string(root.join(&destination))
                    .ok()
                    .map(|text| anchors(&text))
            });
            // A missing page is the link check's to report.
            if offered.as_ref().is_some_and(|set| !set.contains(anchor)) {
                errors.push(format!(
                    "{name}:{}: `{destination}` has no heading for `#{anchor}`",
                    line.number
                ));
            }
        }
    }
}

/// Days since 1970 to a calendar date (Howard Hinnant's `civil_from_days`).
fn date_from_days(days: i64) -> Option<Date> {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = u32::try_from(doy - (153 * mp + 2) / 5 + 1).ok()?;
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).ok()?;
    let year = yoe + era * 400 + i64::from(month <= 2);
    Some((year, month, day))
}

fn today() -> Option<Date> {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    date_from_days(i64::try_from(seconds / 86_400).ok()?)
}

/// Run every check over the checkout's Markdown.
pub(crate) fn check(root: &Path, names: &[String]) -> Vec<String> {
    let mut errors = Vec::new();
    let Some(today) = today() else {
        errors.push("xtask: the system clock is before 1970".to_string());
        return errors;
    };

    let crates: BTreeSet<String> = names
        .iter()
        .filter_map(|name| name.strip_suffix("/Cargo.toml"))
        .filter_map(|folder| folder.rsplit('/').next())
        .map(str::to_string)
        .collect();
    let mut trees = Trees {
        root,
        names,
        read: HashMap::new(),
    };
    let mut makefiles: HashMap<String, Option<BTreeSet<String>>> = HashMap::new();
    let mut anchor_cache = HashMap::new();

    for name in names.iter().filter(|name| name.ends_with(".md")) {
        let Ok(content) = fs::read_to_string(root.join(name)) else {
            continue;
        };
        let parsed = lines(&content);

        check_commands(name, &parsed, &mut trees, &crates, &mut errors);
        check_make(
            name,
            &parsed,
            &mut |directory: &str| {
                makefiles
                    .entry(directory.to_string())
                    .or_insert_with(|| {
                        fs::read_to_string(root.join(directory).join("Makefile"))
                            .ok()
                            .map(|text| make_targets(&text))
                    })
                    .clone()
            },
            &mut errors,
        );
        check_paths(name, &parsed, names, &mut errors);
        check_status(name, &parsed, &mut errors);
        check_wording(name, &parsed, today, &mut errors);
        check_anchors(root, name, &content, &mut anchor_cache, &mut errors);
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY: Date = (2026, 10, 31);

    fn status(page: &str, content: &str) -> Vec<String> {
        let mut errors = Vec::new();
        check_status(page, &lines(content), &mut errors);
        errors
    }

    fn wording(page: &str, content: &str) -> Vec<String> {
        let mut errors = Vec::new();
        check_wording(page, &lines(content), TODAY, &mut errors);
        errors
    }

    fn paths(content: &str) -> Vec<String> {
        let names = ["src/tools/wellfed/src/main.rs", "docs/a.md"].map(String::from);
        let mut errors = Vec::new();
        check_paths("page.md", &lines(content), &names, &mut errors);
        errors
    }

    fn make(content: &str) -> Vec<String> {
        let mut errors = Vec::new();
        check_make(
            "page.md",
            &lines(content),
            &mut |directory: &str| {
                (directory.is_empty() || directory == "src/sub").then(|| {
                    make_targets("check check-docs: dep\nvar := 1\nother = 2\n\trecipe:\n")
                })
            },
            &mut errors,
        );
        errors
    }

    #[test]
    fn evidence_ends_at_the_next_section() {
        let parsed = lines("# T\n\n## Evidence\n\ntext\n\n### Inner\n\nmore\n\n## Next\n\nafter\n");
        let evidence: Vec<usize> = parsed
            .iter()
            .filter(|line| line.evidence)
            .map(|line| line.number)
            .collect();
        assert_eq!(evidence, [3, 4, 5, 6, 7, 8, 9, 10]);
    }

    #[test]
    fn a_fence_holds_a_shorter_fence() {
        let parsed = lines("````md\n```sh\nx\n```\n````\n## H\n");
        assert!(parsed[..5].iter().all(|line| line.block != Block::Prose));
        assert_eq!(parsed[5].block, Block::Prose);
    }

    #[test]
    fn commands_come_from_spans_and_shell_fences() {
        let parsed = lines(
            "Run `wellfed status` now.\n\n```sh\nwellfed feed \\\n  --portions 2\n```\n\n```text\nwellfed nothing\n```\n\n## Evidence\n\n`wellfed gone`\n",
        );
        let found: Vec<String> = commands(&parsed).into_iter().map(|(_, c)| c).collect();
        assert!(found.contains(&"wellfed status".to_string()));
        assert!(found.iter().any(|c| c.contains("--portions 2")));
        assert!(
            !found
                .iter()
                .any(|c| c.contains("nothing") || c.contains("gone"))
        );
    }

    #[test]
    fn a_program_is_found_past_its_prefixes() {
        for command in [
            "wellfed x",
            "sudo target/release/wellfed x",
            "./target/debug/wellfed x",
            "RUST_LOG=1 wellfed x",
            "cargo run -p wellfed -- x",
        ] {
            let all = words(command);
            let (name, arguments) = program(&all).unwrap();
            assert_eq!((name, arguments), ("wellfed", &["x"][..]), "{command}");
        }
        assert_eq!(words("wellfed a | grep b"), ["wellfed", "a"]);
    }

    #[test]
    fn a_missing_make_target_is_refused() {
        assert!(make("`make check` and `make -C src/sub check-docs VAR=1`\n").is_empty());
        assert_eq!(make("`make gone`\n").len(), 1);
        assert_eq!(make("`make -C src/none check`\n").len(), 1);
        // `var :=` assigns, and an indented line is a recipe.
        assert_eq!(make("`make var`\n").len(), 1);
        assert_eq!(make("`make recipe`\n").len(), 1);
        assert!(make("## Evidence\n\n`make gone`\n").is_empty());
    }

    #[test]
    fn a_missing_path_is_refused() {
        assert!(paths("`src/tools/wellfed/src/main.rs` and `wellfed/src/main.rs:12`\n").is_empty());
        assert!(paths("`main.rs` `src/tools/` `docs/a.md`\n").is_empty());
        assert_eq!(paths("`src/scripts/`\n").len(), 1);
        assert_eq!(paths("`examples/time_jump.rs`\n").len(), 1);
        assert_eq!(paths("`gone.rs`\n").len(), 1);
    }

    #[test]
    fn what_is_not_a_repository_path_is_left_alone() {
        for span in [
            "private/x.json",
            "target/release/wellfed",
            "docs/<name>.md",
            "dl/PLAF103/<id>/post",
            "on/off",
            "flash-prepare/flash-execute",
            "server.json",
            "wellfed_core::feed",
            "/etc/x.rs",
            "a b.rs",
        ] {
            assert!(paths(&format!("`{span}`\n")).is_empty(), "{span}");
        }
        assert!(paths("## Evidence\n\n`src/gone.rs`\n").is_empty());
    }

    #[test]
    fn status_is_refused_off_its_pages() {
        for (content, count) in [
            ("Status: **Works**. It works.\n", 1),
            ("A guide. Status: **Experimental**, here.\n", 1),
            ("## Status\n", 1),
            ("| Step | Status |\n| --- | --- |\n| A | Works |\n", 2),
            ("| A | **Partial** |\n", 1),
            ("| A | Works: and a note |\n", 1),
            ("| A | Not started |\n", 1),
        ] {
            assert_eq!(status("docs/page.md", content).len(), count, "{content}");
        }
    }

    #[test]
    fn status_is_allowed_where_it_belongs() {
        let table = "Status: x\n\n| A | Works |\n";
        for page in STATUS_PAGES {
            assert!(status(page, table).is_empty(), "{page}");
        }
        for content in [
            "## Evidence\n\nStatus: x\n\n| A | Works |\n",
            "```text\nStatus: x\n```\n",
            "The `Status: x` field, and `Status::Ready`.\n",
            "| Works with the app alone | No |\n",
            "It works. The status: fine.\n",
        ] {
            assert!(status("docs/page.md", content).is_empty(), "{content}");
        }
    }

    #[test]
    fn time_bound_wording_is_refused_off_the_roadmap() {
        assert_eq!(wording("docs/page.md", "Schedules come next.\n").len(), 1);
        assert_eq!(wording("docs/page.md", "An app is on the way.\n").len(), 1);
        assert_eq!(
            wording("docs/page.md", "Schedules aren't built.\n").len(),
            1
        );
        assert!(wording(ROADMAP, "Schedules come next.\n").is_empty());
        assert!(wording("docs/page.md", "## Evidence\n\nNot built then.\n").is_empty());
        assert!(wording("docs/page.md", "Only one for now. Not yet.\n").is_empty());
    }

    #[test]
    fn a_past_date_is_refused_and_a_future_one_is_not() {
        assert_eq!(
            wording("docs/page.md", "Until then, until October 26.\n").len(),
            1
        );
        assert_eq!(
            wording("docs/page.md", "Good until March 3, 2026.\n").len(),
            1
        );
        assert!(wording("docs/page.md", "Good until November 3.\n").is_empty());
        assert!(wording("docs/page.md", "Good until March 3, 2027.\n").is_empty());
        assert!(wording("AGENTS.md", "Only until October 26, this.\n").is_empty());
    }

    #[test]
    fn slugs_match_what_github_writes() {
        for (title, anchor) in [
            ("What's next", "whats-next"),
            ("The `wellfed` command line", "the-wellfed-command-line"),
            ("Install, as a job", "install-as-a-job"),
            (
                "Stock to a replacement (`stock-ota`)",
                "stock-to-a-replacement-stock-ota",
            ),
            ("A [link](x.md) here", "a-link-here"),
            ("Delivered-food estimate", "delivered-food-estimate"),
            ("snake_case stays", "snake_case-stays"),
        ] {
            assert_eq!(slug(title), anchor);
        }
    }

    #[test]
    fn a_repeated_heading_is_numbered() {
        let found = anchors("## A\n\n## A\n\n```md\n## Fenced\n```\n\n<a id=\"named\"></a>\n");
        for anchor in ["a", "a-1", "named"] {
            assert!(found.contains(anchor), "{anchor}");
        }
        assert!(!found.contains("fenced"));
    }

    #[test]
    fn a_missing_anchor_is_refused() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("docs")).unwrap();
        fs::write(root.path().join("docs/b.md"), "# B\n\n## Here\n").unwrap();

        let run = |content: &str| {
            fs::write(root.path().join("docs/a.md"), content).unwrap();
            let mut errors = Vec::new();
            check_anchors(
                root.path(),
                "docs/a.md",
                content,
                &mut HashMap::new(),
                &mut errors,
            );
            errors
        };

        assert!(run("[x](b.md#here) [y](b.md) [z](#self)\n\n## Self\n").is_empty());
        assert!(run("[gone](missing.md#here) [l](x.rs#L3) [u](https://e.com/a.md#b)\n").is_empty());
        assert_eq!(run("[x](b.md#gone)\n").len(), 1);
        assert_eq!(run("[x](#gone)\n").len(), 1);
        assert_eq!(run("## Evidence\n\n[x](b.md#gone)\n").len(), 1);
    }

    #[test]
    fn dates_come_from_days() {
        assert_eq!(date_from_days(0), Some((1970, 1, 1)));
        assert_eq!(date_from_days(20_757), Some((2026, 10, 31)));
    }

    #[test]
    fn a_wrong_command_in_a_page_is_refused() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("src/tools/wellfed/src")).unwrap();
        fs::write(
            root.path().join("src/tools/wellfed/src/main.rs"),
            "#[derive(Parser)] struct Cli { #[command(subcommand)] c: C }\n\
             #[derive(Subcommand)] enum C { Feed { #[arg(long)] portions: u8 } }\n",
        )
        .unwrap();
        let names = ["src/tools/wellfed/src/main.rs".to_string()];
        let mut trees = Trees {
            root: root.path(),
            names: &names,
            read: HashMap::new(),
        };
        let crates = BTreeSet::from(["wellfed-real".to_string()]);

        let mut errors = Vec::new();
        check_commands(
            "page.md",
            &lines(
                "`wellfed feed --portions 2`\n`wellfed-real thing`\n`wellfed-real`\n`wellfed gone`\n`wellfed feed --gone`\n`wellfed-retired serve`\n",
            ),
            &mut trees,
            &crates,
            &mut errors,
        );
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert!(errors[0].contains("no command `gone`"));
        assert!(errors[1].contains("no flag `--gone`"));
        assert!(errors[2].contains("no tool called"));
    }
}
