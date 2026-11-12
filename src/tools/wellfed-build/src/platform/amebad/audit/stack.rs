//! Worst-case static stack of each KM4 application task, from the linked
//! image.
//!
//! A crate-split build overflowed the optics task's 4 KiB stack on hardware:
//! LTO inlined a 3 KiB temporary into the task's frame. The SDK's ARMv8-M
//! port faults on the stack limit, and the trial caught it, but only after
//! an update. This audit catches it at build time instead.
//!
//! For each task created in the runtime or its services
//! (`sdk::task_create(c"name", words, ...)`) or declared by the firmware
//! (`#[wellfed_sdk::task(name = "name", stack = WORDS, priority = N)] fn entry`),
//! it walks the direct call graph from the task's entry function and
//! sums the frames along the deepest path. A function's frame is the sum of
//! every stack decrement in it (`push`, `stmdb sp!`, `vpush`, `str [sp, #-n]!`,
//! `sub sp, #n`), not only the prologue's, and one that sets the stack from
//! a register's value is refused ([`StackError::Variable`]): `sub sp, rN`
//! anywhere, and `mov sp, rN` or `add sp, rN` unless the frame's restore
//! (`pop`, `vpop`, `ldm sp!`) comes next, as in an epilogue's
//! `mov sp, r7; pop {r7, pc}`. A linker veneer
//! (`__name_veneer`, the long branch into RAM or ROM) leads to its target.
//! Recursion is cut at the cycle.
//!
//! **What is code.** The pinned objdump lists some instructions inside a
//! function as data (`blx r3` as `.short 0x4798`, a `sub sp` as half of a
//! `.word`), and sometimes as ARM, because of how it looks mapping symbols up.
//! A call or a frame it hides that way would understate the bound. So the walk
//! disassembles a copy with the mapping symbols stripped, which decodes every
//! byte as Thumb, and reads the symbols themselves, with `readelf`, to tell
//! which lines are data: a literal pool or a jump table the compiler marked,
//! whose decoding means nothing and whose words are the addresses a function
//! loads. Anything in code that still doesn't decode is refused
//! ([`StackError::Undecoded`]), as is an image with no mapping symbols, two
//! mapping symbols that disagree at one address, and a function that starts
//! where no Thumb symbol says code runs. A pool decoded as Thumb can also
//! leave objdump in an IT block, so the code after it prints conditional
//! (`pushhi`): every instruction pattern takes a condition.
//!
//! An indirect call (`blx rN`, or a tail call `bx rN`) is bounded by what it can reach:
//! - inside `core::fmt`, by the deepest formatting method in the image
//!   (`Display::fmt`, `Write::write_str` and the rest), nested up to
//!   [`FMT_NESTING`] levels;
//! - inside the RAM flash hooks, by the deepest provider callback the runtime
//!   installs ([`PROVIDER_CALLBACKS`]);
//! - inside vendor C (lwIP, the codec, the SDK's flash and ADC routines), not
//!   at all: those sites are listed per task, and the margin covers them.
//!
//! A firmware with no SDK runtime (`runtime = "bare"`) is its own task table:
//! it creates its tasks with `task_create`, and nothing in it installs a
//! flash-lease provider, so the flash hooks' provider calls never run and
//! charge nothing.
//!
//! Any other indirect call in Rust is refused, so a new function pointer or
//! `dyn` call needs a class here or a direct call instead. The margin stays
//! for what the walk does not model: exception nesting beyond one frame and
//! the vendor C sites.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::process::Command;
use std::sync::LazyLock;

use regex::Regex;
use wellfed_formats::audit::{StackAudit, TaskBound};

use super::writes::{demangle, loaded};
use super::{Binutils, HEADING, tool};
use crate::firmware::Runtime;
use crate::{Firmware, Repo, Result, text};
use wellfed_formats::require;

/// Why the stack audit could not bound a task, or refused how the firmware
/// starts its tasks. Function names are demangled, as the messages print
/// them.
#[derive(Debug, thiserror::Error)]
pub enum StackError {
    /// Task creations or declarations neither reader matched.
    #[error(
        "{count} task creations do not have the shape the stack audit reads \
         (#[wellfed_sdk::task(name = \"name\", stack = WORDS, priority = N)] on a plain \
         function, or the runtime's task_create(c\"name\", WORDS, priority, entry)): keep \
         the arguments in that order, the priority a literal and the entry a plain \
         function name"
    )]
    Shape { count: usize },
    #[error(
        "Task {task} starts the generic {entry}{generic}: the stack audit cannot find its \
         symbol, so start it through a non-generic entry function"
    )]
    Generic {
        task: String,
        entry: String,
        generic: String,
    },
    #[error(
        "Task {task}'s stack size {size} is neither a literal nor a `const NAME: u16` the \
         stack audit can read"
    )]
    StackSize { task: String, size: String },
    /// Two `const NAME: u16` definitions disagree, so which one a task's
    /// stack takes depends on configuration the audit does not read.
    #[error("The stack size constant {name} is defined twice with different values")]
    Constant { name: String },
    #[error("The flash hooks link, but no provider callback the runtime installs")]
    NoCallback,
    #[error("Provider callbacks link, but no flash-hook site calls them: update HOOK_SITES")]
    NoHookSite,
    #[error("The flash-hook callback {callback} reaches an unbounded indirect call in {call}")]
    CallbackUnbounded { callback: String, call: String },
    #[error(
        "The stack audit walked {visits} calls without finishing: the call graph has cycles \
         too tangled to bound this way, so no bound is given"
    )]
    WalkBudget { visits: u64 },
    #[error("Ambiguous task entry {entry}: {symbols:?}")]
    AmbiguousEntry { entry: String, symbols: Vec<String> },
    #[error(
        "Task {task} reaches an unbounded indirect call in {call}: give it a class in the \
         stack audit or call it directly"
    )]
    Unbounded { task: String, call: String },
    /// `path` is the start of the deepest call path, up to twelve functions.
    #[error(
        "Task {task} may overflow: static bound {bound} bytes of {stack} (margin {:.0}%); \
         deepest path {:?}",
        MARGIN * 100.0,
        &path[..path.len().min(6)]
    )]
    Overflow {
        task: String,
        bound: u64,
        stack: u64,
        path: Vec<String>,
    },
    #[error(
        "{function} has {line:?} in its code, at {address:#x}, with no mapping symbol calling \
         it data: the stack audit cannot say what it does"
    )]
    Undecoded {
        function: String,
        address: u64,
        line: String,
    },
    /// The stack set from a register, to a depth only the run knows.
    #[error(
        "{function} sets the stack from a register's value ({line:?} at {address:#x}): \
         the stack audit cannot bound a frame sized at run time"
    )]
    Variable {
        function: String,
        address: u64,
        line: String,
    },
    #[error("The linked image has no mapping symbols, so its code cannot be told from its data")]
    NoMappingSymbols,
    /// Two mapping symbols at one address name different kinds: which one
    /// readelf lists last would decide what the bytes are.
    #[error("Mapping symbols at {address:#x} disagree on whether it is Thumb, ARM or data")]
    MappingConflict { address: u64 },
    /// A function in the image starts where no Thumb mapping symbol says code
    /// runs, so its bytes would be read as data or refused one by one.
    #[error(
        "{function} starts at {address:#x}, which no Thumb mapping symbol marks as code: \
         the stack audit cannot read it"
    )]
    NotCode { function: String, address: u64 },
    #[error("No application task entries found in the linked image")]
    NoTasks,
    #[error(
        "{count} uses of spawn are not wellfed_sdk::spawn(entry): start each task by that \
         call, so the stack audit can read it"
    )]
    SpawnUses { count: usize },
    #[error(
        "The firmware spawns {entry}, which no #[wellfed_sdk::task] in its own sources \
         declares: declare the task in the firmware crate, so the stack audit reads its stack"
    )]
    Undeclared { entry: String },
    #[error(
        "The firmware creates a task with task_create: declare it with \
         #[wellfed_sdk::task] and start it with wellfed_sdk::spawn"
    )]
    TaskCreate,
    #[error(
        "The firmware starts {} tasks ({tasks:?}); the runtime leaves it {limit} of the \
         SDK's eight slots",
        tasks.len(),
        limit = FIRMWARE_TASKS
    )]
    TooMany { tasks: Vec<String> },
    /// A firmware declared bare links the SDK runtime anyway: its tasks and
    /// provider are not in the table a bare firmware is audited against.
    #[error(
        "The firmware declares runtime = \"bare\" but its image links the SDK runtime: {}",
        functions.join(", ")
    )]
    BareRuntime { functions: Vec<String> },
}

/// What a bare firmware's image may not link: any of `wellfed-sdk`'s code
/// (its runtime's tasks and the provider callbacks it installs). Read from
/// the linked functions, not the firmware's sources, so a dependency that
/// pulls the runtime in is caught too. Returns their demangled names.
fn sdk_runtime_in<'a>(functions: impl Iterator<Item = &'a String>) -> Vec<String> {
    let mut linked: Vec<String> = functions
        .filter(|name| {
            // The fault callback is the chip HAL's, which every image links;
            // the others are the runtime's.
            name.contains("11wellfed_sdk")
                || PROVIDER_CALLBACKS
                    .iter()
                    .filter(|end| end.starts_with("7runtime"))
                    .any(|end| name.ends_with(end))
        })
        .map(|name| demangle(name))
        .collect();
    linked.sort();
    linked
}

/// The SDK runtime's task table.
pub(crate) const RUNTIME_SOURCE: &str = "src/sdk/wellfed-sdk/src/runtime.rs";
/// SDK services a firmware may start, whose tasks count as the firmware's
/// (the button task).
pub(crate) const SERVICE_SOURCES: [&str; 1] = ["src/sdk/wellfed-sdk/src/buttons.rs"];
/// Tasks the vendor C creates whose body is Rust: the task's name, the C
/// entry the walk starts from, and the stack words `example_entry.c` gives
/// it (a test reads them there). The Wi-Fi task's frames are the driver's
/// and the vendor calls' it makes, as much as its Rust's, so the walk counts
/// both, and only an image that links the shim has the entry.
pub(crate) const C_TASKS: [(&str, &str, u64); 1] = [("wellfed_wifi", "wellfed_wifi_task", 1280)];

/// The firmware's share of the SDK's eight task slots: the runtime holds three
/// in the normal role, and one stays spare. `wellfed_sdk::FIRMWARE_TASKS` enforces the
/// same budget at run time, and a test keeps the two equal.
const FIRMWARE_TASKS: usize = 4;

/// The text the audit reads its task table from.
pub(crate) struct TaskSources {
    /// What the image starts from: the SDK runtime's table, or for a bare
    /// firmware, its own sources.
    kind: Runtime,
    runtime: String,
    /// The SDK services a firmware may start.
    services: String,
    /// Every Rust file of the firmware's `src/`, in path order.
    own: String,
}

impl TaskSources {
    /// The sources whose tasks count as the firmware's: its services', then
    /// its own.
    fn firmware(&self) -> String {
        format!("{}\n{}", self.services, self.own)
    }
}

pub(crate) fn task_sources(repo: &Repo, firmware: &Firmware) -> Result<TaskSources> {
    let mut own = Vec::new();
    rust_files(&repo.path(&format!("{}/src", firmware.dir())), &mut own)?;
    own.sort();
    let own = concatenated(&own)?;

    if firmware.runtime() == Runtime::Bare {
        return Ok(TaskSources {
            kind: Runtime::Bare,
            runtime: own,
            services: String::new(),
            own: String::new(),
        });
    }
    let services: Vec<std::path::PathBuf> = SERVICE_SOURCES
        .iter()
        .map(|source| repo.path(source))
        .collect();
    Ok(TaskSources {
        kind: Runtime::Sdk,
        runtime: std::fs::read_to_string(repo.path(RUNTIME_SOURCE))?,
        services: concatenated(&services)?,
        own,
    })
}

fn concatenated(paths: &[std::path::PathBuf]) -> Result<String> {
    let mut text = String::new();
    for path in paths {
        text.push_str(&std::fs::read_to_string(path)?);
        text.push('\n');
    }
    Ok(text)
}

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            rust_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// The SDK trampoline frame and an exception stacked onto the task's stack
/// with the FPU context (26 words), rounded up.
const OVERHEAD: u64 = 256;
/// What the walk does not model (the vendor C indirect sites, deeper
/// exception nesting): the bound must leave this share of the stack free.
const MARGIN: f64 = 0.25;
/// How deeply one formatting call may nest another: a record's `Display`
/// writes fields whose own `Display` writes through the report's `Write`.
const FMT_NESTING: usize = 4;
/// The flash-lock interposer's functions, whose indirect calls reach the
/// installed provider.
const HOOK_SITES: [&str; 5] = [
    "FLASH_Write_Lock",
    "FLASH_Write_Unlock",
    "FLASH_Write_IPC_Int",
    "hook_fail",
    "wellfed_amebad_flash_hooks_install",
];
/// The provider callbacks `wellfed-sdk`'s runtime installs into the hooks,
/// by the end of their (mangled) names.
const PROVIDER_CALLBACKS: [&str; 5] = [
    "7runtime10hook_ready",
    "7runtime12hook_acquire",
    "7runtime12hook_release",
    "7runtime9hook_peer",
    "wellfed_amebad_flash_provider_fault",
];

/// A task the runtime or a service creates: name, stack words, entry, and the
/// entry's turbofish if it names a generic instance.
static TASK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"task_create\(\s*c"([\w-]+)",\s*(\w+),\s*\d+,\s*(\w+)(::<[^>]*>)?\s*[,)]"#)
        .expect("stack audit pattern")
});
/// A firmware's task, as `#[task]` declares it: name, stack words and the
/// entry, which keeps the function's name. Doc comments and other attributes
/// may sit between the attribute and the function.
static DECLARED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r#"#\[\s*(?:(?:::)?\s*wellfed_sdk\s*::\s*)?task\s*\(\s*name\s*=\s*"([\w-]+)"\s*,"#,
        r"\s*stack\s*=\s*(\w+)\s*,\s*priority\s*=\s*\d+\s*,?\s*\)\s*\]",
        r"\s*(?:(?:#\[[^\]]*\]|///[^\n]*)\s*)*(?:pub(?:\([\w\s:]*\))?\s+)?fn\s+(\w+)\s*\(",
    ))
    .expect("stack audit pattern")
});
static CONSTANT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"const (\w+): u16 = (\d+);").expect("stack audit pattern"));
/// A condition suffix, as objdump prints it on an instruction in an IT block.
///
/// Every instruction pattern below takes one. A real conditional prologue or
/// call is rare, and counting it is the safe direction. But with the mapping
/// symbols stripped, objdump decodes a literal pool's halfwords as Thumb too,
/// and one that reads as `it` or `ittee` puts the real code after the pool in
/// an IT block of objdump's making: `push` prints as `pushhi`, `sub sp` as
/// `subhi sp`. Those decode, so nothing refuses them, and a plain-mnemonic
/// pattern would drop the frame or the call without a word.
const COND: &str = "(?:eq|ne|cs|hs|cc|lo|mi|pl|vs|vc|hi|ls|ge|lt|gt|le|al)?";

fn pattern(template: &str) -> Regex {
    Regex::new(&template.replace("{C}", COND)).expect("stack audit pattern")
}

/// A direct call or tail call, conditional ones included: `blne` in an IT
/// block and `bne.w` both leave the function. A branch inside the function
/// names an offset (`<name+0x12>`), which the target's pattern leaves out.
static CALL: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"\t(?:bl|blx|b){C}(?:\.[nw])?\t[0-9a-f]+ <([^>+]+)>"));
/// A call or tail call through a register. objdump names r9 to r12 `sb`, `sl`,
/// `fp` and `ip`, and a tail call through a pointer is `bx rN` (`bx ip` in
/// vendor C and `core::fmt`). `bx lr` is a return, not a call. A linker
/// veneer's `bx ip` is not counted: its target comes from its name.
static INDIRECT: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"\tbl?x{C}(?:\.n)?\t(?:r\d+|sb|sl|fp|ip)\b"));
/// Every task creation or declaration, whatever its shape: [`TASK`] and
/// [`DECLARED`] must account for each. A `Task` built by hand is one neither
/// reads, so it is refused.
static TASK_SITE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"task_create\(\s*c"|#\[\s*(?:(?:::)?\s*wellfed_sdk\s*::\s*)?task\s*\(|Task\s*::\s*__new\s*\("#,
    )
    .expect("stack audit pattern")
});
/// A firmware starting a task: `wellfed_sdk::spawn(entry)`, with the entry a
/// task the firmware declares.
static SPAWNED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"wellfed_sdk\s*::\s*spawn\s*\(\s*(\w+)\s*\)").expect("stack audit pattern")
});
/// Any mention of `spawn` outside a comment: [`SPAWNED`] must account for
/// each, so a renamed import or a task from elsewhere cannot start unread.
static SPAWN_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bspawn\b").expect("stack audit pattern"));
/// Any mention of `task_create` outside a comment, whatever its arguments.
static CREATE_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\btask_create\b").expect("stack audit pattern"));
static LINE_COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"//[^\n]*").expect("stack audit pattern"));
/// Half of an address built in a register: `movw r0, #51187` (low) then
/// `movt r0, #3585` (high).
static MOVE: LazyLock<Regex> = LazyLock::new(|| pattern(r"^mov([wt]){C}\s+(\w+), #(\d+)"));
/// A mapping symbol as `readelf -s` lists it: its address, and `t` for Thumb,
/// `a` for ARM or `d` for data. LLVM numbers them (`$d.12`).
static MAPPING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*\d+:\s+([0-9a-f]+)\s+\d+\s+\S+\s+\S+\s+\S+\s+\S+\s+\$([atd])(?:\.\S*)?\s*$")
        .expect("stack audit pattern")
});
/// A function symbol in one of the image's sections, as `readelf -s` lists
/// it: its address (bit 0 set for Thumb) and name. ROM functions are
/// absolute (`ABS`), with no bytes in the image, and are left out. readelf
/// prints a size of 100000 or more in hex, so the size takes either form.
///
/// Only `FUNC` symbols are checked. An assembly label with no `.type` is
/// `NOTYPE`, like the mapping symbols and the section's other labels, so a
/// function written that way that starts outside Thumb code isn't refused
/// here.
static FUNC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"^\s*\d+:\s+([0-9a-f]+)\s+(?:0x[0-9a-f]+|\d+)\s+FUNC\s+\S+\s+\S+\s+\d+",
        r"\s+(\S+)\s*$",
    ))
    .expect("stack audit pattern")
});
/// A listed line: its address, then its bytes.
static LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*([0-9a-f]+):\t([0-9a-f ]+?)\s*(?:\t|$)").expect("stack audit pattern")
});
/// How many words of a vtable (or other table) a reference may name
/// functions in: the drop, size and alignment slots, then the methods.
const TABLE_WORDS: u64 = 8;
/// A formatting trait method, by its mangled name: `<T as Display>::fmt` and
/// the other `core::fmt` traits, and `Write`'s methods.
static FMT_METHOD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^_RNv[XY].*(?:(?:7Display|5Debug|8LowerHex|8UpperHex|5Octal|6Binary|8LowerExp|8UpperExp|7Pointer)3fmt|5Write(?:9write_str|10write_char|9write_fmt))",
    )
    .expect("stack audit pattern")
});
static PUSH: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"^(?:push{C}(?:\.w)?|stmdb{C}(?:\.w)?\s+sp!,)\s*\{([^}]*)\}"));
static VPUSH: LazyLock<Regex> = LazyLock::new(|| pattern(r"^vpush{C}\s*\{d(\d+)-d(\d+)\}"));
/// A prologue slot saved by writeback: LLVM keeps `push` to the low registers
/// and the frame pointer's pair, then saves the rest as `str.w fp, [sp, #-4]!`.
static WRITEBACK: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"^str{C}(?:\.w)?\s+\w+,\s*\[sp,\s*#-(\d+)\]!"));
static SUB: LazyLock<Regex> = LazyLock::new(|| {
    // `subw` is Thumb-2's 12-bit-immediate form, which LLVM picks for frames
    // up to 4 KiB; missing it hid curve25519's 1948-byte frame.
    pattern(r"^sub(?:w)?{C}(?:\.w)?\s+sp,\s*(?:sp,\s*)?#(\d+)")
});
/// A stack decrement by a register's value: a frame sized at run time
/// (`alloca`, a variable-length array), which no static bound covers.
static SUB_REGISTER: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"^subs?{C}(?:\.w)?\s+sp,\s*(?:sp,\s*)?(?:r\d+|sb|sl|fp|ip|lr)\b"));

/// The stack pointer set from a register (`mov sp, r7`, `add sp, sp, r3`):
/// an epilogue's restore when the frame's pop follows, anything else a
/// stack the listing cannot bound.
static SET_REGISTER: LazyLock<Regex> = LazyLock::new(|| {
    pattern(r"^(?:mov|add)s?{C}(?:\.w)?\s+sp,\s*(?:sp,\s*)?(?:r\d+|sb|sl|fp|ip|lr)\b")
});
/// What ends a frame: a restore of saved registers from the stack.
static RESTORE: LazyLock<Regex> = LazyLock::new(|| {
    pattern(r"^(?:pop{C}(?:\.w)?|vpop{C}|ldmia{C}(?:\.w)?\s+sp!,|ldm{C}(?:\.w)?\s+sp!,)")
});

/// {entry function name: (task name, stack bytes)} from the task sources.
///
/// A task the audit cannot resolve is refused rather than skipped, since a
/// skipped task links unaudited. A generic entry (`step::<S>`) is one: its
/// monomorphized symbol carries the type arguments, so no linked function
/// matches the source's name, and the walk would take it for a variant that
/// does not link the task. So is a stack size that is neither a literal nor a
/// `const NAME: u16` in the same sources.
fn tasks(source: &str) -> Result<BTreeMap<String, (String, u64)>> {
    // Sources are read whole, cfg and all, so two definitions of one name
    // could each be the one compiled; refuse them unless they agree.
    let mut constants: HashMap<&str, u64> = HashMap::new();
    for c in CONSTANT.captures_iter(source) {
        let (Some(name), Ok(value)) = (c.get(1), c[2].parse::<u64>()) else {
            continue;
        };
        if constants
            .insert(name.as_str(), value)
            .is_some_and(|was| was != value)
        {
            return Err(StackError::Constant {
                name: name.as_str().into(),
            }
            .into());
        }
    }

    let (sites, matched) = (
        TASK_SITE.find_iter(source).count(),
        TASK.captures_iter(source).count() + DECLARED.captures_iter(source).count(),
    );
    if sites != matched {
        return Err(StackError::Shape {
            count: sites - matched.min(sites),
        }
        .into());
    }

    let mut found = BTreeMap::new();
    for c in TASK
        .captures_iter(source)
        .chain(DECLARED.captures_iter(source))
    {
        let (task, entry) = (&c[1], &c[3]);
        if let Some(generic) = c.get(4) {
            return Err(StackError::Generic {
                task: task.into(),
                entry: entry.into(),
                generic: generic.as_str().into(),
            }
            .into());
        }
        let Some(words) = constants.get(&c[2]).copied().or_else(|| c[2].parse().ok()) else {
            return Err(StackError::StackSize {
                task: task.into(),
                size: c[2].into(),
            }
            .into());
        };
        found.insert(entry.to_string(), (task.to_string(), words * 4));
    }
    Ok(found)
}

/// What a run of bytes is, by the mapping symbol that starts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mapped {
    Thumb,
    Arm,
    Data,
}

/// The image's mapping symbols, by address.
#[derive(Debug, Default)]
struct CodeMap(BTreeMap<u64, Mapped>);

impl CodeMap {
    /// The mapping symbols in `readelf -sW`'s `symbols`. Refuses two that
    /// disagree at one address, and a function in the image's sections that
    /// starts where no Thumb symbol says code runs: either would leave what
    /// the bytes are to the order readelf lists them in, or read a function
    /// as data without a word.
    fn read(symbols: &str) -> std::result::Result<Self, StackError> {
        let mut map = BTreeMap::new();
        for found in symbols.lines().filter_map(|line| MAPPING.captures(line)) {
            let Ok(address) = u64::from_str_radix(&found[1], 16) else {
                continue;
            };
            let mapped = match &found[2] {
                "t" => Mapped::Thumb,
                "a" => Mapped::Arm,
                _ => Mapped::Data,
            };
            if *map.entry(address).or_insert(mapped) != mapped {
                return Err(StackError::MappingConflict { address });
            }
        }
        let map = Self(map);

        for found in symbols.lines().filter_map(|line| FUNC.captures(line)) {
            let Ok(value) = u64::from_str_radix(&found[1], 16) else {
                continue;
            };
            let address = value & !1;
            require(map.at(address) == Some(Mapped::Thumb), || {
                StackError::NotCode {
                    function: demangle(&found[2]),
                    address,
                }
            })?;
        }
        Ok(map)
    }

    /// `None` before the first symbol.
    fn at(&self, address: u64) -> Option<Mapped> {
        self.0
            .range(..=address)
            .next_back()
            .map(|(_, mapped)| *mapped)
    }

    /// Whether the bytes from `address` for `len` are all `mapped`.
    fn all(&self, address: u64, len: u64, mapped: Mapped) -> bool {
        self.at(address) == Some(mapped)
            && self
                .0
                .range(address..address + len)
                .all(|(_, here)| *here == mapped)
    }
}

/// A listed line's bytes as halfwords, in address order. objdump prints a
/// halfword in value order, and a word as one eight-digit group.
fn halfwords(bytes: &str) -> Vec<u16> {
    let mut halves = Vec::new();
    for group in bytes.split_whitespace() {
        let Ok(value) = u32::from_str_radix(group, 16) else {
            continue;
        };
        match group.len() {
            4 => halves.extend(u16::try_from(value).ok()),
            8 => {
                halves.extend(u16::try_from(value & 0xffff).ok());
                halves.extend(u16::try_from(value >> 16).ok());
            }
            _ => {}
        }
    }
    halves
}

/// The bytes a listed line covers.
fn byte_len(bytes: &str) -> u64 {
    bytes
        .split_whitespace()
        .map(|group| group.len() as u64 / 2)
        .sum()
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Function {
    address: u64,
    frame: u64,
    calls: BTreeSet<String>,
    indirect: bool,
    /// The first stack pointer set from a register (`sub sp, r3`, or a
    /// `mov sp, r0` no restore follows), whose depth no listing shows: its
    /// address and text.
    variable: Option<(u64, String)>,
    /// Addresses the function builds or loads (`movw`/`movt` pairs and
    /// literal words): the formatting methods and vtables it passes on.
    refs: BTreeSet<u64>,
    /// The first line in its code that didn't decode: its address and text.
    undecoded: Option<(u64, String)>,
}

/// Each function's prologue frame, direct callees and whether it makes
/// indirect calls, from `objdump -d` of the image with its mapping symbols
/// stripped, in listing order. `map` says which lines are data.
fn frames(listing: &str, map: &CodeMap) -> Vec<(String, Function)> {
    let mut functions: Vec<(String, Function)> = Vec::new();
    let mut low: HashMap<String, u64> = HashMap::new();
    // A `mov sp, rN` waiting for the next instruction to show it restores
    // the frame; otherwise, or at the function's end, it is variable.
    let mut setting: Option<(u64, String)> = None;
    let unrestored = |functions: &mut Vec<(String, Function)>,
                      setting: &mut Option<(u64, String)>| {
        if let (Some((_, f)), Some(set)) = (functions.last_mut(), setting.take()) {
            f.variable.get_or_insert(set);
        }
    };
    // The low half of a literal word, waiting for its high half.
    let mut low_half: Option<(u64, u16)> = None;
    for line in listing.lines() {
        if let Some(heading) = HEADING.captures(line) {
            unrestored(&mut functions, &mut setting);
            let address = u64::from_str_radix(&heading[1], 16).unwrap_or_default();
            let f = Function {
                address,
                ..Function::default()
            };
            functions.push((heading[2].to_string(), f));
            low.clear();
            continue;
        }
        let Some((name, f)) = functions.last_mut() else {
            continue;
        };
        let Some(listed) = LINE.captures(line) else {
            continue;
        };
        let address = u64::from_str_radix(&listed[1], 16).unwrap_or_default();
        let bytes = &listed[2];
        let insn = line.splitn(3, '\t').last().unwrap_or_default();

        if map.all(address, byte_len(bytes), Mapped::Data) {
            // What this decoded to means nothing. Its words are what the
            // function loads: a pool is word-aligned, a table's halves aren't.
            let mut at = address;
            for half in halfwords(bytes) {
                match low_half.take() {
                    Some((before, low)) if before + 2 == at && before % 4 == 0 => {
                        f.refs.insert(u64::from(half) << 16 | u64::from(low));
                    }
                    _ => low_half = Some((at, half)),
                }
                at += 2;
            }
            continue;
        }
        low_half = None;

        let decoded = !insn.starts_with('.') && !insn.contains("UNDEFINED");
        if !decoded || !map.all(address, byte_len(bytes), Mapped::Thumb) {
            f.undecoded
                .get_or_insert_with(|| (address, line.trim().to_string()));
            continue;
        }

        if let Some(set) = setting.take()
            && !RESTORE.is_match(insn)
        {
            f.variable.get_or_insert(set);
        }
        if SET_REGISTER.is_match(insn) {
            setting = Some((address, line.trim().to_string()));
            continue;
        }

        // Every stack decrement in the function counts, wherever it sits: a
        // frame too large for one immediate is two `sub sp`, and GCC spills
        // stacked argument registers with `sub sp, #8` before its push. The
        // sum can only overstate a frame (an epilogue's `add sp` is not
        // taken off), never understate it.
        if let Some(push) = PUSH.captures(insn) {
            f.frame += 4 * push[1].split(',').filter(|r| !r.trim().is_empty()).count() as u64;
            continue;
        }
        if let Some(vpush) = VPUSH.captures(insn) {
            let (low, high): (u64, u64) =
                (vpush[1].parse().unwrap_or(0), vpush[2].parse().unwrap_or(0));
            f.frame += 8 * (high + 1).saturating_sub(low);
            continue;
        }
        if let Some(slot) = WRITEBACK.captures(insn) {
            f.frame += slot[1].parse::<u64>().unwrap_or(0);
            continue;
        }
        if let Some(sub) = SUB.captures(insn) {
            f.frame += sub[1].parse::<u64>().unwrap_or(0);
            continue;
        }
        if SUB_REGISTER.is_match(insn) {
            f.variable
                .get_or_insert_with(|| (address, line.trim().to_string()));
            continue;
        }
        if let Some(call) = CALL.captures(line) {
            f.calls.insert(call[1].to_string());
        } else if INDIRECT.is_match(line) {
            f.indirect |= veneer(name).is_none();
        } else if let Some(m) = MOVE.captures(insn) {
            let value: u64 = m[3].parse().unwrap_or_default();
            if &m[1] == "w" {
                low.insert(m[2].to_string(), value);
            } else if let Some(low) = low.remove(&m[2]) {
                f.refs.insert(value << 16 | low);
            }
        }
    }
    unrestored(&mut functions, &mut setting);
    functions
}

/// What an indirect call can reach, by the function that makes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Indirect {
    /// `core::fmt`: a formatting method.
    Fmt,
    /// The flash hooks: an installed provider callback.
    Hook,
    /// Anything else: unbounded. Vendor C is listed; Rust is refused.
    Unbounded,
}

fn classify(name: &str) -> Indirect {
    if HOOK_SITES.contains(&name) {
        Indirect::Hook
    } else if name.starts_with("_R") && name.contains("4core3fmt") {
        Indirect::Fmt
    } else {
        Indirect::Unbounded
    }
}

/// What a walk charges each class of indirect call.
#[derive(Debug, Clone, Copy, Default)]
struct Charges {
    fmt: u64,
    hook: u64,
}

/// A function's deepest path: bytes, the path, and the functions on any
/// path whose indirect calls stay unbounded.
type Walk = (u64, Vec<String>, BTreeSet<String>);

/// Calls the walks of one audit may visit before it refuses. A result cut
/// against an ancestor is walked again from each path, so a large tangle of
/// mutually recursive functions could take exponential time; the audit
/// fails closed rather than hang the build. The feeder's image needs far
/// fewer (its whole audit runs in under a second).
const WALK_VISITS_MAX: u64 = 20_000_000;

struct Graph<'a> {
    functions: &'a HashMap<String, Function>,
    classes: HashMap<&'a str, Indirect>,
    by_address: HashMap<u64, &'a str>,
    /// The image's loaded sections, for the tables a reference points into.
    data: &'a [(u64, Vec<u8>)],
    /// Calls visited so far, against `budget`; past it every walk stops
    /// short, and `audit` refuses.
    visits: Cell<u64>,
    budget: u64,
}

impl<'a> Graph<'a> {
    fn new(functions: &'a HashMap<String, Function>, data: &'a [(u64, Vec<u8>)]) -> Self {
        let classes = functions
            .iter()
            .filter(|(_, f)| f.indirect)
            .map(|(name, _)| (name.as_str(), classify(name)))
            .collect();
        let by_address = functions
            .iter()
            .map(|(name, f)| (f.address, name.as_str()))
            .collect();
        Self {
            functions,
            classes,
            by_address,
            data,
            visits: Cell::new(0),
            budget: WALK_VISITS_MAX,
        }
    }

    /// Whether some walk ran out of `budget`, so its numbers are short.
    fn exhausted(&self) -> bool {
        self.visits.get() > self.budget
    }

    /// The function at `value`, a Thumb code address.
    fn function_at(&self, value: u64) -> Option<&'a str> {
        self.by_address.get(&(value & !1)).copied()
    }

    /// The functions `value` names: itself, or those among the first words of
    /// the table it points into (a vtable).
    fn resolve(&self, value: u64, out: &mut BTreeSet<&'a str>) {
        if let Some(name) = self.function_at(value) {
            out.insert(name);
            return;
        }
        let Some((start, bytes)) = self
            .data
            .iter()
            .find(|(start, bytes)| (*start..*start + bytes.len() as u64).contains(&value))
        else {
            return;
        };
        for word in 0..TABLE_WORDS {
            let Ok(at) = usize::try_from(value - start + 4 * word) else {
                break;
            };
            let Some(&raw) = bytes.get(at..at + 4).and_then(|b| b.first_chunk::<4>()) else {
                break;
            };
            if let Some(name) = self.function_at(u64::from(u32::from_le_bytes(raw))) {
                out.insert(name);
            }
        }
    }

    /// The formatting methods `entry` can reach: referenced from any function
    /// it calls, and from those methods in turn.
    fn fmt_targets(&self, entry: &'a str) -> BTreeSet<&'a str> {
        let (mut seen, mut targets, mut stack) = (BTreeSet::new(), BTreeSet::new(), vec![entry]);
        while let Some(name) = stack.pop() {
            if !seen.insert(name) {
                continue;
            }
            let Some(f) = self.functions.get(name) else {
                continue;
            };
            stack.extend(f.calls.iter().map(String::as_str));

            let mut named = BTreeSet::new();
            for value in &f.refs {
                self.resolve(*value, &mut named);
            }
            for target in named {
                if FMT_METHOD.is_match(target) && targets.insert(target) {
                    stack.push(target);
                }
            }
        }
        targets
    }

    /// What `entry`'s formatting calls are charged: its deepest formatting
    /// method, nested [`FMT_NESTING`] deep; and the unbounded indirect calls
    /// those methods reach, which the task's own walk never sees.
    fn fmt_charge(&self, entry: &'a str, hook: u64) -> (u64, BTreeSet<String>) {
        let targets = self.fmt_targets(entry);
        let (mut charges, mut unbounded) = (Charges { fmt: 0, hook }, BTreeSet::new());
        for _ in 0..FMT_NESTING {
            let (depth, nested) = self.deepest(targets.iter().copied(), charges);
            charges.fmt = depth;
            unbounded.extend(nested);
        }
        (charges.fmt, unbounded)
    }

    /// The deepest path from `entry` under `charges`, and the functions where
    /// recursion was cut.
    fn worst(&self, entry: &str, charges: Charges) -> (Walk, Vec<String>) {
        let (mut memo, mut cut) = (HashMap::new(), BTreeSet::new());
        let (walk, _) = self.walk(entry, charges, &mut Vec::new(), &mut memo, &mut cut);
        (walk, cut.into_iter().collect())
    }

    /// `name`'s deepest path, and the shallowest index in `visiting` above
    /// `name` that a cut below it landed on, if any.
    ///
    /// A cut against a function still on the path above `name` makes
    /// `name`'s result depend on how it was reached: from a caller where that
    /// function is not on the path, the cut edge is a real call whose frames
    /// count. Such a result is not memoised, so a later caller walks it again
    /// rather than reuse the smaller number. A result whose cuts all landed on
    /// `name` or below closed its cycles itself, and is memoised.
    fn walk(
        &self,
        name: &str,
        charges: Charges,
        visiting: &mut Vec<String>,
        memo: &mut HashMap<String, Walk>,
        cut: &mut BTreeSet<String>,
    ) -> (Walk, Option<usize>) {
        self.visits.set(self.visits.get().saturating_add(1));
        if self.exhausted() {
            // Short, and never memoised (index 0 is above every caller);
            // `audit` refuses before any bound is used.
            return ((0, Vec::new(), BTreeSet::new()), Some(0));
        }
        if let Some(at) = visiting.iter().position(|v| v == name) {
            cut.insert(name.to_string());
            return ((0, Vec::new(), BTreeSet::new()), Some(at));
        }
        if let Some(known) = memo.get(name) {
            return (known.clone(), None);
        }
        let Some(f) = self.functions.get(name) else {
            return ((0, vec![name.to_string()], BTreeSet::new()), None);
        };

        let (mut best, mut path, mut vendor) = (0, Vec::new(), BTreeSet::new());
        match self.classes.get(name) {
            Some(Indirect::Fmt) => (best, path) = (charges.fmt, vec!["<fmt>".to_string()]),
            Some(Indirect::Hook) => (best, path) = (charges.hook, vec!["<provider>".to_string()]),
            Some(Indirect::Unbounded) => {
                vendor.insert(name.to_string());
            }
            None => {}
        }
        let own = visiting.len();
        let mut reached: Option<usize> = None;
        visiting.push(name.to_string());
        for callee in &f.calls {
            let ((depth, deeper, nested), hit) = self.walk(callee, charges, visiting, memo, cut);
            reached = reached.into_iter().chain(hit).min();
            vendor.extend(nested);
            if depth > best {
                (best, path) = (depth, deeper);
            }
        }
        visiting.pop();

        let mut full = vec![name.to_string()];
        full.extend(path);
        let result = (f.frame + best, full, vendor);
        match reached.filter(|&at| at < own) {
            Some(ancestor) => (result, Some(ancestor)),
            None => {
                memo.insert(name.to_string(), result.clone());
                (result, None)
            }
        }
    }

    /// The deepest of `targets` under `charges`, and the unbounded indirect
    /// calls any of them reaches.
    fn deepest<'n>(
        &self,
        targets: impl Iterator<Item = &'n str>,
        charges: Charges,
    ) -> (u64, BTreeSet<String>) {
        let (mut best, mut unbounded) = (0, BTreeSet::new());
        for target in targets {
            let ((depth, _, nested), _) = self.worst(target, charges);
            best = best.max(depth);
            unbounded.extend(nested);
        }
        (best, unbounded)
    }

    /// What a call into the flash hooks is charged: the deepest provider
    /// callback, with its own formatting charged as for a task.
    /// A callback that reaches an unclassified Rust indirect call is refused,
    /// as a task would be: nothing else walks the callbacks.
    fn hook_charge(&self, runtime: Runtime) -> Result<u64> {
        let hooks = self.classes.values().any(|class| *class == Indirect::Hook);
        let callbacks: Vec<&str> = self
            .functions
            .keys()
            .filter(|name| PROVIDER_CALLBACKS.iter().any(|end| name.ends_with(end)))
            .map(String::as_str)
            .collect();
        match (hooks, callbacks.is_empty()) {
            (false, true) => return Ok(0),
            // Nothing in a bare firmware installs a provider, so the hooks
            // stay on the vendor lock and never call one.
            (true, true) if runtime == Runtime::Bare => return Ok(0),
            (true, true) => return Err(StackError::NoCallback.into()),
            (false, false) => return Err(StackError::NoHookSite.into()),
            (true, false) => {}
        }

        let mut best = 0;
        for callback in callbacks {
            let (fmt, mut unbounded) = self.fmt_charge(callback, 0);
            let ((depth, _, nested), _) = self.worst(callback, Charges { fmt, hook: 0 });
            unbounded.extend(nested);
            if let Some(rust) = unbounded.iter().find(|name| name.starts_with("_R")) {
                return Err(StackError::CallbackUnbounded {
                    callback: demangle(callback),
                    call: demangle(rust),
                }
                .into());
            }
            best = best.max(depth);
        }
        Ok(best)
    }

    /// The charges for `entry`'s task, and the unbounded indirect calls its
    /// formatting methods reach.
    fn charges(&self, entry: &'a str, hook: u64) -> (Charges, BTreeSet<String>) {
        let (fmt, unbounded) = self.fmt_charge(entry, hook);
        (Charges { fmt, hook }, unbounded)
    }
}

/// Functions by name. Two static C functions can share a name: the one
/// entry stands for both, with the larger frame, every call either makes
/// and an indirect call if either makes one, so a call to the deeper one is
/// never charged the shallower frame.
fn merged(functions: Vec<(String, Function)>) -> HashMap<String, Function> {
    let mut merged: HashMap<String, Function> = HashMap::new();
    for (name, f) in functions {
        match merged.entry(name) {
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(f);
            }
            std::collections::hash_map::Entry::Occupied(mut slot) => {
                let kept = slot.get_mut();
                kept.frame = kept.frame.max(f.frame);
                kept.calls.extend(f.calls);
                kept.indirect |= f.indirect;
                kept.refs.extend(f.refs);
                kept.undecoded = kept.undecoded.take().or(f.undecoded);
                kept.variable = kept.variable.take().or(f.variable);
            }
        }
    }
    merged
}

/// `functions` with each veneer leading to its target.
fn link(mut functions: HashMap<String, Function>) -> HashMap<String, Function> {
    for (name, f) in &mut functions {
        if let Some(target) = veneer(name) {
            f.calls.insert(target.to_string());
        }
    }
    functions
}

/// `__name_veneer`'s target.
fn veneer(name: &str) -> Option<&str> {
    name.strip_prefix("__")?.strip_suffix("_veneer")
}

/// The listing of `elf` with its mapping symbols stripped, and the symbols.
fn listing(elf: &Path, binutils: &Binutils) -> Result<(String, CodeMap)> {
    let map = CodeMap::read(&tool(&binutils.readelf(), &["-sW"], elf)?)?;
    if map.0.is_empty() {
        return Err(StackError::NoMappingSymbols.into());
    }

    let scratch = tempfile::tempdir()?;
    let stripped = scratch.path().join("stripped.axf");
    text(
        Command::new(binutils.objcopy())
            .args(["--wildcard", "--strip-symbol=$*"])
            .arg(elf)
            .arg(&stripped),
    )?;
    Ok((tool(&binutils.objdump(), &["-d"], &stripped)?, map))
}

/// The first line of code in `functions` that didn't decode, or that sizes
/// a frame at run time, refused.
fn all_decoded(functions: &HashMap<String, Function>) -> Result<()> {
    let first = |line: fn(&Function) -> Option<&(u64, String)>| {
        functions
            .iter()
            .filter_map(|(name, f)| Some((name, line(f)?)))
            .min_by_key(|(_, (address, _))| *address)
    };

    if let Some((function, (address, line))) = first(|f| f.undecoded.as_ref()) {
        return Err(StackError::Undecoded {
            function: demangle(function),
            address: *address,
            line: line.clone(),
        }
        .into());
    }
    match first(|f| f.variable.as_ref()) {
        Some((function, (address, line))) => Err(StackError::Variable {
            function: demangle(function),
            address: *address,
            line: line.clone(),
        }
        .into()),
        None => Ok(()),
    }
}

/// Per-task bounds; refuses if a task's bound leaves less than MARGIN free,
/// if a function's code can't be read, or if the firmware starts more tasks
/// than its share of the slots.
pub(crate) fn audit(elf: &Path, binutils: &Binutils, sources: &TaskSources) -> Result<StackAudit> {
    let (listed, map) = listing(elf, binutils)?;
    let functions = link(merged(frames(&listed, &map)));
    all_decoded(&functions)?;
    if sources.kind == Runtime::Bare {
        let linked = sdk_runtime_in(functions.keys());
        require(linked.is_empty(), || StackError::BareRuntime {
            functions: linked,
        })?;
    }
    let data = loaded(elf, &binutils.objdump())?;
    let graph = Graph::new(&functions, &data);
    let hook = graph.hook_charge(sources.kind)?;
    let mut by_short: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for name in functions.keys() {
        let short = demangle(name)
            .rsplit("::")
            .next()
            .unwrap_or_default()
            .to_string();
        by_short.entry(short).or_default().push(name.clone());
    }

    let firmware_tasks = firmware_tasks(sources, |entry| by_short.contains_key(entry))?;

    let mut rows = BTreeMap::new();
    let source = format!("{}\n{}", sources.runtime, sources.firmware());
    for (entry, (task, stack)) in tasks(&source)? {
        let symbols = by_short.get(&entry).cloned().unwrap_or_default();
        let symbol = match &symbols[..] {
            // Not linked in this variant.
            [] => continue,
            [symbol] => symbol,
            _ => {
                return Err(StackError::AmbiguousEntry {
                    entry,
                    symbols: symbols.clone(),
                }
                .into());
            }
        };

        rows.insert(
            task.clone(),
            bound(&graph, symbol, &entry, &task, stack, hook)?,
        );
    }

    // A C task's entry is a C symbol, and only an image that links its
    // source has it.
    for (task, entry, words) in C_TASKS {
        if functions.contains_key(entry) {
            let row = bound(&graph, entry, entry, task, words * 4, hook)?;
            rows.insert(task.to_string(), row);
        }
    }

    if rows.is_empty() {
        return Err(StackError::NoTasks.into());
    }
    require(!graph.exhausted(), || StackError::WalkBudget {
        visits: graph.visits.get(),
    })?;
    Ok(StackAudit {
        tasks: rows,
        firmware_tasks,
        firmware_task_limit: FIRMWARE_TASKS,
        overhead: OVERHEAD,
        margin: MARGIN,
        fmt_nesting: FMT_NESTING,
        provider_bound: hook,
        scope: "Direct calls, veneers and prologue frames; indirect calls in core::fmt and the \
                flash hooks bounded by class; vendor C indirect calls listed and covered by the \
                margin"
            .into(),
    })
}

/// `task`'s worst path from `symbol`, refused past the margin of its `stack`
/// bytes or at an unbounded Rust indirect call.
fn bound(
    graph: &Graph<'_>,
    symbol: &str,
    entry: &str,
    task: &str,
    stack: u64,
    hook: u64,
) -> Result<TaskBound> {
    let (charges, mut unbounded) = graph.charges(symbol, hook);
    let ((depth, path, direct), cut) = graph.worst(symbol, charges);
    unbounded.extend(direct);
    if let Some(rust) = unbounded.iter().find(|name| name.starts_with("_R")) {
        return Err(StackError::Unbounded {
            task: task.into(),
            call: demangle(rust),
        }
        .into());
    }
    let bound = depth + OVERHEAD;
    let path: Vec<String> = path.iter().take(12).map(|n| demangle(n)).collect();
    if bound as f64 > stack as f64 * (1.0 - MARGIN) {
        return Err(StackError::Overflow {
            task: task.into(),
            bound,
            stack,
            path,
        }
        .into());
    }
    Ok(TaskBound {
        entry: entry.into(),
        stack,
        bound,
        fmt_bound: charges.fmt,
        unbounded: unbounded.iter().map(|n| demangle(n)).collect(),
        recursion: cut.iter().map(|n| demangle(n)).collect(),
        path,
    })
}

/// Every task the firmware starts is one it declares with `#[task]` in its
/// own sources, where the audit reads its stack. A task declared in a
/// library, behind a renamed attribute or inside `cfg_attr` would otherwise
/// link with its stack unchecked, so starting one is refused; so is a `spawn`
/// the audit cannot read, such as a renamed import.
fn spawned_tasks_are_declared(own: &str) -> Result<()> {
    let code = LINE_COMMENT.replace_all(own, "");
    let declared: BTreeSet<&str> = DECLARED
        .captures_iter(own)
        .filter_map(|c| c.get(3).map(|entry| entry.as_str()))
        .collect();

    let spawned: Vec<String> = SPAWNED
        .captures_iter(&code)
        .map(|c| c[1].to_string())
        .collect();
    let mentions = SPAWN_WORD.find_iter(&code).count();
    if mentions != spawned.len() {
        return Err(StackError::SpawnUses {
            count: mentions - spawned.len().min(mentions),
        }
        .into());
    }
    if let Some(entry) = spawned
        .iter()
        .find(|entry| !declared.contains(entry.as_str()))
    {
        return Err(StackError::Undeclared {
            entry: entry.clone(),
        }
        .into());
    }
    Ok(())
}

/// The firmware's tasks that `linked` finds in the image, refused past its
/// share: a firmware that takes a runtime task's slot leaves the build unable
/// to save settings or see an update offer. A firmware that creates a task
/// itself, past `wellfed_sdk::spawn`, is refused too, since that task would
/// not count against the share at run time.
fn firmware_tasks(sources: &TaskSources, linked: impl Fn(&str) -> bool) -> Result<Vec<String>> {
    if CREATE_WORD.is_match(&LINE_COMMENT.replace_all(&sources.own, "")) {
        return Err(StackError::TaskCreate.into());
    }
    spawned_tasks_are_declared(&sources.own)?;

    let found: Vec<String> = tasks(&sources.firmware())?
        .into_iter()
        .filter(|(entry, _)| linked(entry))
        .map(|(_, (task, _))| task)
        .collect();
    if found.len() > FIRMWARE_TASKS {
        return Err(StackError::TooMany { tasks: found }.into());
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    /// The stack refusal of a result that must be refused.
    fn stack_error<T: std::fmt::Debug>(result: Result<T>) -> StackError {
        match result {
            Err(Error::Stack(error)) => error,
            other => panic!("{other:?}"),
        }
    }

    /// The C task's stack is the audit's, and its name and entry too.
    #[test]
    fn the_c_tasks_are_created_as_the_audit_reads_them() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let entry =
            std::fs::read_to_string(root.join("src/firmware/vendor/ports/amebad/example_entry.c"))
                .unwrap();

        for (task, function, words) in C_TASKS {
            let creation = format!("xTaskCreate({function}, \"{task}\", {words},");
            assert_eq!(entry.matches(&creation).count(), 1, "{creation}");
        }
    }

    /// A task the SDK creates outside these files would link unaudited.
    #[test]
    fn every_sdk_file_that_creates_a_task_is_audited() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let mut files = Vec::new();
        rust_files(&root.join("src/sdk/wellfed-sdk/src"), &mut files).unwrap();

        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            let relative = file
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if TASK.is_match(&text) {
                assert!(
                    relative == RUNTIME_SOURCE || SERVICE_SOURCES.contains(&relative.as_str()),
                    "{relative}"
                );
            }
        }
    }

    #[test]
    fn the_firmware_share_matches_the_sdk_budget() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let tasks = std::fs::read_to_string(root.join("src/sdk/wellfed-sdk/src/tasks.rs")).unwrap();

        assert!(
            tasks.contains(&format!("pub const FIRMWARE_TASKS: u8 = {FIRMWARE_TASKS};")),
            "wellfed-sdk's FIRMWARE_TASKS differs from the audit's"
        );
    }

    #[test]
    fn only_linked_firmware_tasks_count_and_one_past_the_share_is_refused() {
        let sources = |own: &str| TaskSources {
            kind: Runtime::Sdk,
            runtime: "sdk::task_create(c\"wellfed-net\", 1536, 2, net_task, ptr::null_mut());"
                .into(),
            services: "sdk::task_create(c\"wellfed-key\", 512, 3, key_task, ptr::null_mut());"
                .into(),
            own: own.into(),
        };
        let four: String = ["a", "b", "c", "d"]
            .map(|t| format!("#[wellfed_sdk::task(name = \"fw-{t}\", stack = 512, priority = 2)]\nfn {t}_task() -> u32 {{ 0 }}\n"))
            .concat();

        let linked = |entry: &str| entry != "key_task";
        assert_eq!(
            firmware_tasks(&sources(&four), linked).unwrap(),
            ["fw-a", "fw-b", "fw-c", "fw-d"],
            "an unused service links no task"
        );

        let error = stack_error(firmware_tasks(&sources(&four), |_| true));
        assert!(
            matches!(&error, StackError::TooMany { tasks } if tasks.len() == 5),
            "{error}"
        );
    }

    /// A task whose declaration the audit cannot read would link with its
    /// stack unchecked.
    #[test]
    fn a_task_started_but_not_declared_here_is_refused() {
        let own = |text: &str| TaskSources {
            kind: Runtime::Sdk,
            runtime: String::new(),
            services: String::new(),
            own: text.into(),
        };
        let declared = "#[wellfed_sdk::task(name = \"fw-a\", stack = 512, priority = 2)]\n\
                        fn a_task() -> u32 { 0 }\n";

        let fine =
            format!("{declared}// spawn in a comment is not a call\nwellfed_sdk::spawn(a_task);");
        assert_eq!(firmware_tasks(&own(&fine), |_| true).unwrap(), ["fw-a"]);

        for (shape, undeclared) in [
            ("wellfed_sdk::spawn(wellfed_probes::x_task);", false),
            ("wellfed_sdk::spawn(x_task);", true),
            ("use wellfed_sdk::spawn as go;\ngo(a_task);", false),
            (
                "#[cfg_attr(feature = \"x\", wellfed_sdk::task(name = \"fw-x\", stack = 512, priority = 2))]\n\
                 fn x_task() -> u32 { 0 }\nwellfed_sdk::spawn(x_task);",
                true,
            ),
        ] {
            let text = format!("{declared}{shape}");
            let error = stack_error(firmware_tasks(&own(&text), |_| true));
            if undeclared {
                assert!(
                    matches!(&error, StackError::Undeclared { entry } if entry == "x_task"),
                    "{shape}: {error}"
                );
            } else {
                assert!(
                    matches!(&error, StackError::SpawnUses { count: 1 }),
                    "{shape}: {error}"
                );
            }
        }
    }

    /// A task the firmware creates itself would pass the run-time budget.
    #[test]
    fn a_firmware_that_creates_a_task_itself_is_refused() {
        let sources = TaskSources {
            kind: Runtime::Sdk,
            runtime: String::new(),
            services: String::new(),
            own: "unsafe { sdk::task_create(c\"fw-raw\", 512, 2, raw_task, ptr::null_mut()) };"
                .into(),
        };

        assert!(matches!(
            stack_error(firmware_tasks(&sources, |_| true)),
            StackError::TaskCreate
        ));

        let named = TaskSources {
            own: "unsafe { sdk::task_create(name, 512, 2, raw_task, ptr::null_mut()) };".into(),
            ..sources
        };
        assert!(matches!(
            stack_error(firmware_tasks(&named, |_| true)),
            StackError::TaskCreate
        ));
    }

    const LISTING: &str = "\
00001000 <task>:
    1000:\tb5f0      \tpush\t{r4, r5, r6, r7, lr}
    1002:\tf5ad 6d4d \tsub.w\tsp, sp, #3280\t; 0xcd0
    1006:\tf000 f800 \tbl\t2000 <deep>
    100a:\tf000 f800 \tbl\t3000 <shallow>
    100e:\t4798      \tblx\tr3

00002000 <deep>:
    2000:\te92d 4ff0 \tstmdb\tsp!, {r4, r5, r6, r7, r8, r9, sl, fp, lr}
    2004:\ted2d 8b04 \tvpush\t{d8-d9}
    2008:\tb0d1      \tsub\tsp, #836
    200a:\tf000 f800 \tbl\t2000 <deep>
    200e:\tf000 b800 \tb.w\t3000 <shallow>

00003000 <shallow>:
    3000:\tb510      \tpush\t{r4, lr}
    3002:\tbd10      \tpop\t{r4, pc}

00004000 <wide>:
    4000:\tb580      \tpush\t{r7, lr}
    4002:\tf2ad 7d9c \tsubw\tsp, sp, #1948\t; 0x79c
";

    #[test]
    fn tasks_resolve_word_constants() {
        let source = "const PROBE_STACK_WORDS: u16 = 2048;\n\
                      sdk::task_create(\n    c\"wellfed-optics\",\n    PROBE_STACK_WORDS,\n    3,\n    optics_task,\n\
                      let ok = sdk::task_create(c\"wellfed-wdg\", 512, 7, feeder_task, ptr::null_mut());\n\
                      /// The chime.\n\
                      #[wellfed_sdk::task(name = \"wellfed-aud\", stack = 1024, priority = 2)]\n\
                      fn audio_task() -> u32 {\n\
                      #[wellfed_sdk::task(\n    name = \"wellfed-loop\",\n    stack = PROBE_STACK_WORDS,\n    priority = 2,\n)]\n\
                      /// The loop.\n\
                      #[cfg(feature = \"x\")]\n\
                      pub(crate) fn loop_task() -> u32 {";
        let expected: BTreeMap<String, (String, u64)> = [
            ("loop_task".to_string(), ("wellfed-loop".to_string(), 8192)),
            (
                "optics_task".to_string(),
                ("wellfed-optics".to_string(), 8192),
            ),
            ("feeder_task".to_string(), ("wellfed-wdg".to_string(), 2048)),
            ("audio_task".to_string(), ("wellfed-aud".to_string(), 4096)),
        ]
        .into();
        assert_eq!(tasks(source).unwrap(), expected);
    }

    #[test]
    fn a_task_the_audit_cannot_resolve_is_refused() {
        let generic = "sdk::task_create(c\"wellfed-gen\", 2048, 3, generic_step::<S>, state);";
        let error = stack_error(tasks(generic));
        assert!(
            matches!(
                &error,
                StackError::Generic { entry, generic, .. }
                    if entry == "generic_step" && generic == "::<S>"
            ),
            "{error}"
        );

        let unknown = "#[wellfed_sdk::task(name = \"wellfed-x\", stack = ELSEWHERE_WORDS, priority = 2)]\n\
                       fn x_task() -> u32 {";
        let error = stack_error(tasks(unknown));
        assert!(
            matches!(&error, StackError::StackSize { size, .. } if size == "ELSEWHERE_WORDS"),
            "{error}"
        );

        for shape in [
            "sdk::task_create(c\"wellfed-x\", 512, PRIORITY, x_task, ptr::null_mut());",
            "sdk::task_create(c\"wellfed-x\", 512, 2, tasks::run, ptr::null_mut());",
            "#[wellfed_sdk::task(name = \"wellfed-x\", stack = 512, priority = PRIORITY)]\nfn x_task() -> u32 {",
            "#[wellfed_sdk::task(stack = 512, name = \"wellfed-x\", priority = 2)]\nfn x_task() -> u32 {",
            "#[task(name = \"wellfed-x\", stack = 512, priority = 2)]\nstatic X: u32 = 0;",
            "const X: Task = Task::__new(c\"wellfed-x\", 512, 2, x_task);",
        ] {
            let error = stack_error(tasks(shape));
            assert!(
                matches!(error, StackError::Shape { count: 1 }),
                "{shape}: {error}"
            );
        }
    }

    #[test]
    fn register_calls_and_tail_calls_are_indirect() {
        for call in [
            "blx\tr3", "blx\tip", "blx\tfp", "blx\tsl", "blx\tsb", "bx\tr2", "bx\tip", "bx\tsb",
        ] {
            assert!(
                INDIRECT.is_match(&format!("    1000:\t4798      \t{call}")),
                "{call}"
            );
        }
        for other in ["bx\tlr", "blx\t2000 <callee>", "bl\t2000 <callee>"] {
            assert!(
                !INDIRECT.is_match(&format!("    1000:\t4798      \t{other}")),
                "{other}"
            );
        }
    }

    /// Calls and tail calls that only run on a condition still leave the
    /// function, so a path through one counts; a branch inside it is no call.
    #[test]
    fn conditional_calls_are_edges_and_inner_branches_are_not() {
        let listing = "\
00001000 <caller>:
    1000:\tb580      \tpush\t{r7, lr}
    1002:\tbf18      \tit\tne
    1004:\tf000 f800 \tblne\t2000 <linked>
    1008:\tf040 8000 \tbne.w\t3000 <tailed>
    100c:\tf43f affa \tbeq.w\t1004 <caller+0x4>
    1010:\td1f8      \tbne.n\t1004 <caller+0x4>
    1012:\tf47f aff9 \tbne.w\t1008 <caller+0x8>
    1016:\tddfa      \tble.n\t100e <caller+0xe>
    1018:\tb100      \tcbz\tr0, 101c <caller+0x1c>
    101a:\tf000 f800 \tbl\t4000 <plain>
    101e:\tf000 b800 \tb.w\t5000 <jumped>
00002000 <linked>:
    2000:\tb082      \tsub\tsp, #8
00003000 <tailed>:
    3000:\tb084      \tsub\tsp, #16
00004000 <plain>:
    4000:\t4770      \tbx\tlr
00005000 <jumped>:
    5000:\t4770      \tbx\tlr
";
        let functions: HashMap<String, Function> =
            frames(listing, &code_only()).into_iter().collect();

        let calls: Vec<&str> = functions["caller"]
            .calls
            .iter()
            .map(String::as_str)
            .collect();
        assert_eq!(calls, ["jumped", "linked", "plain", "tailed"]);

        let ((depth, path, _), _) = Graph::new(&functions, &[]).worst("caller", Charges::default());
        assert_eq!(depth, 8 + 16);
        assert_eq!(path, ["caller", "tailed"]);
    }

    /// A slot saved by writeback ahead of `sub sp` is part of the frame.
    #[test]
    fn writeback_slots_count_towards_the_frame() {
        let listing = "\
00001000 <fn>:
    1000:\tb5f0      \tpush\t{r4, r5, r6, r7, lr}
    1002:\taf03      \tadd\tr7, sp, #12
    1004:\tf84d bd04 \tstr.w\tfp, [sp, #-4]!
    1008:\tb090      \tsub\tsp, #64
    100a:\tf84d 0d04 \tstr.w\tr0, [sp, #-4]!
00002000 <other>:
    2000:\tf84d 8d08 \tstr.w\tr8, [sp, #-8]!
    2004:\t9001      \tstr\tr0, [sp, #4]
";
        let functions: HashMap<String, Function> =
            frames(listing, &code_only()).into_iter().collect();

        assert_eq!(
            functions["fn"].frame,
            20 + 4 + 64 + 4,
            "a decrement after `sub sp` counts too"
        );
        assert_eq!(functions["other"].frame, 8, "a plain store moves nothing");
    }

    #[test]
    fn deepest_path_sums_prologues_and_cuts_recursion() {
        let functions: HashMap<String, Function> =
            frames(LISTING, &code_only()).into_iter().collect();
        assert_eq!(functions["task"].frame, 20 + 3280);
        assert!(functions["task"].indirect);
        assert_eq!(functions["deep"].frame, 36 + 16 + 836);
        assert_eq!(functions["wide"].frame, 8 + 1948, "subw counts");

        let graph = Graph::new(&functions, &[]);
        let ((depth, path, unbounded), cut) = graph.worst("task", Charges::default());
        assert_eq!(depth, 3300 + 888 + 8);
        assert_eq!(path, ["task", "deep", "shallow"]);
        assert_eq!(unbounded, ["task".to_string()].into(), "vendor C, listed");
        assert_eq!(cut, ["deep"]);
    }

    /// An image that is all Thumb.
    fn code_only() -> CodeMap {
        CodeMap([(0, Mapped::Thumb)].into())
    }

    /// [`CLASSES`]' one literal word.
    fn classes_map() -> CodeMap {
        CodeMap(
            [
                (0, Mapped::Thumb),
                (0x3008, Mapped::Data),
                (0x4000, Mapped::Thumb),
            ]
            .into(),
        )
    }

    #[test]
    fn mapping_symbols_read_from_readelf_with_or_without_a_number() {
        let symbols = "\
   12: 0e001390     0 NOTYPE  LOCAL  DEFAULT    3 $t
   13: 0e0014b8     0 NOTYPE  LOCAL  DEFAULT    3 $d.17
   14: 0e0014cc     0 NOTYPE  LOCAL  DEFAULT    3 $t.18
   15: 0e0014cc     4 FUNC    GLOBAL DEFAULT    3 dns_send
";
        let map = CodeMap::read(symbols).unwrap();

        assert_eq!(map.0.len(), 3);
        assert_eq!(map.at(0x0e00_138f), None);
        assert_eq!(map.at(0x0e00_1390), Some(Mapped::Thumb));
        assert_eq!(map.at(0x0e00_14c0), Some(Mapped::Data));
        assert_eq!(map.at(0x0e00_14cc), Some(Mapped::Thumb));
    }

    /// A pool's words can decode as anything, even a call. They are what the
    /// function loads, and nothing it does.
    #[test]
    fn data_is_read_for_its_words_and_never_as_instructions() {
        let listing = "\
00001000 <fn>:
    1000:\tb500      \tpush\t{lr}
    1002:\t4770      \tbx\tlr
    1004:\tdcc0      \tbgt.n\t1088 <fn+0x88>
    1006:\t1001      \tasrs\tr1, r0, #32
    1008:\tf000 f800 \tbl\t2000 <never>
    100c:\t4798      \tblx\tr3
    100e:\tb082      \tsub\tsp, #8
    1010:\tb084      \tsub\tsp, #16
";
        let map = CodeMap(
            [
                (0x1000, Mapped::Thumb),
                (0x1004, Mapped::Data),
                (0x1010, Mapped::Thumb),
            ]
            .into(),
        );
        let functions: HashMap<String, Function> = frames(listing, &map).into_iter().collect();
        let f = &functions["fn"];

        assert!(f.calls.is_empty(), "{:?}", f.calls);
        assert!(!f.indirect);
        assert!(f.refs.contains(&0x1001_dcc0), "{:x?}", f.refs);
        assert!(f.refs.contains(&0xf800_f000), "{:x?}", f.refs);
        assert_eq!(f.undecoded, None);
    }

    /// Two static functions with one name are one entry with the deeper
    /// frame and every call, whichever the listing prints last.
    #[test]
    fn functions_sharing_a_name_keep_the_deeper_frame_and_every_call() {
        let listing = "\
00001000 <twin>:
    1000:\tb5f0      \tpush\t{r4, r5, r6, r7, lr}
    1002:\tb090      \tsub\tsp, #64
    1004:\tf000 f800 \tbl\t3000 <deep>
00002000 <twin>:
    2000:\tb500      \tpush\t{lr}
    2002:\tf000 f800 \tbl\t4000 <shallow>
";
        let functions = merged(frames(listing, &code_only()));
        assert_eq!(functions["twin"].frame, 20 + 64);
        let calls: Vec<&str> = functions["twin"].calls.iter().map(String::as_str).collect();
        assert_eq!(calls, ["deep", "shallow"]);
    }

    #[test]
    fn stack_constants_that_disagree_are_refused() {
        let source = "const STACK: u16 = 512;\n#[cfg(x)]\nconst STACK: u16 = 256;\n";
        let error = tasks(source).unwrap_err();
        assert!(
            matches!(&error, Error::Stack(StackError::Constant { name }) if name == "STACK"),
            "{error:?}"
        );
        assert!(tasks("const STACK: u16 = 512;\nconst STACK: u16 = 512;\n").is_ok());
    }

    /// A frame of 4 KiB or more is two `sub sp`; GCC spills stacked
    /// argument registers with `sub sp` before its push; and a split
    /// allocation can come in two encodable halves. Each decrement counts.
    #[test]
    fn every_stack_decrement_counts_not_only_the_first_sub() {
        let listing = "\
0e00bebe <split>:
 e00bebe:\tb5f0      \tpush\t{r4, r5, r6, r7, lr}
 e00bec0:\te92d 0f00 \tstmdb\tsp!, {r8, r9, sl, fp}
 e00bec4:\tf5ad 5d88 \tsub.w\tsp, sp, #4352
 e00bec8:\tb085      \tsub\tsp, #20
0e00c000 <spill>:
 e00c000:\tb082      \tsub\tsp, #8
 e00c002:\tb570      \tpush\t{r4, r5, r6, lr}
 e00c004:\tb084      \tsub\tsp, #16
0e00d000 <task>:
 e00d000:\tb580      \tpush\t{r7, lr}
 e00d002:\tf5ad 5d80 \tsub.w\tsp, sp, #4096
 e00d006:\tf5ad 6d00 \tsub.w\tsp, sp, #2048
";
        let functions: HashMap<String, Function> =
            frames(listing, &code_only()).into_iter().collect();

        assert_eq!(functions["split"].frame, 20 + 16 + 4352 + 20);
        assert_eq!(functions["spill"].frame, 8 + 16 + 16);
        assert_eq!(functions["task"].frame, 8 + 4096 + 2048);

        let graph = Graph::new(&functions, &[]);
        let error = bound(&graph, "task", "task", "task", 6144, 0).unwrap_err();
        assert!(
            matches!(
                &error,
                Error::Stack(StackError::Overflow { bound: 6408, .. })
            ),
            "{error:?}"
        );
    }

    /// A frame sized by a register has no static bound, so it is refused
    /// rather than counted as nothing.
    #[test]
    fn a_stack_decrement_by_a_register_is_refused() {
        for line in [
            "    1002:\tebad 0d03 \tsub.w\tsp, sp, r3",
            "    1002:\tebad 0d03 \tsub\tsp, r3",
            "    1002:\tebad 0d03 \tsubne.w\tsp, sp, ip",
        ] {
            let listing = format!("00001000 <fn>:\n    1000:\tb500      \tpush\t{{lr}}\n{line}\n");
            let error = refused(&listing, &code_only());
            assert!(
                matches!(&error, StackError::Variable { function, address: 0x1002, .. } if function == "fn"),
                "{error:?}"
            );
        }
    }

    /// The stack set from a register is refused unless the frame's restore
    /// follows at once, as in an epilogue; the reset entry's switch to its
    /// own stack (`mov sp, r0` then a call) is one.
    #[test]
    fn a_stack_set_from_a_register_outside_an_epilogue_is_refused() {
        for (set, next) in [
            ("4685      \tmov\tsp, r0", "f000 f800 \tbl\t2000 <other>"),
            ("eb0d 0d03 \tadd.w\tsp, sp, r3", "4770      \tbx\tlr"),
            ("4685      \tmov\tsp, r0", ""),
        ] {
            let mut listing =
                format!("00001000 <fn>:\n    1000:\tb500      \tpush\t{{lr}}\n    1002:\t{set}\n");
            if !next.is_empty() {
                listing.push_str(&format!("    1004:\t{next}\n"));
            }
            let error = refused(&listing, &code_only());
            assert!(
                matches!(&error, StackError::Variable { function, address: 0x1002, .. } if function == "fn"),
                "{set} then {next:?}: {error:?}"
            );
        }

        let epilogue = "\
00001000 <fn>:
    1000:\tb580      \tpush\t{r7, lr}
    1002:\taf00      \tadd\tr7, sp, #0
    1004:\t46bd      \tmov\tsp, r7
    1006:\tbd80      \tpop\t{r7, pc}
";
        let functions: HashMap<String, Function> =
            frames(epilogue, &code_only()).into_iter().collect();
        assert_eq!(functions["fn"].variable, None);
        assert_eq!(functions["fn"].frame, 8);
    }

    /// What the pinned objdump hides, once decoded, counts.
    #[test]
    fn a_call_or_a_frame_the_data_view_hid_is_counted() {
        let listing = "\
00001000 <fn>:
    1000:\tb500      \tpush\t{lr}
    1002:\tb087      \tsub\tsp, #28
    1004:\t4798      \tblx\tr3
    1006:\tf000 f800 \tbl\t2000 <other>
";
        let functions: HashMap<String, Function> =
            frames(listing, &code_only()).into_iter().collect();

        assert_eq!(functions["fn"].frame, 4 + 28);
        assert!(functions["fn"].indirect);
        assert!(functions["fn"].calls.contains("other"));
    }

    fn refused(listing: &str, map: &CodeMap) -> StackError {
        let functions: HashMap<String, Function> = frames(listing, map).into_iter().collect();
        match all_decoded(&functions) {
            Err(Error::Stack(error)) => error,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn code_that_still_does_not_decode_is_refused() {
        for line in [
            "    1002:\t4798      \t.short\t0x4798",
            "    1002:\t47984b36 \t.word\t0x47984b36",
            "    1002:\tf04fe7b3 \t\t\t; <UNDEFINED> instruction: 0xf04fe7b3",
        ] {
            let listing = format!("00001000 <fn>:\n    1000:\tb500      \tpush\t{{lr}}\n{line}\n");
            let error = refused(&listing, &code_only());
            assert!(
                matches!(&error, StackError::Undecoded { function, address: 0x1002, .. } if function == "fn"),
                "{error:?}"
            );
        }
    }

    #[test]
    fn a_line_across_the_end_of_a_pool_or_with_no_symbol_is_refused() {
        let listing = "\
00001000 <fn>:
    1000:\tb500      \tpush\t{lr}
    1002:\tf000 f800 \tbl\t2000 <other>
";
        // A desynchronized decode: it starts in code and ends in data.
        let map = CodeMap([(0x1000, Mapped::Thumb), (0x1004, Mapped::Data)].into());
        assert!(matches!(
            refused(listing, &map),
            StackError::Undecoded {
                address: 0x1002,
                ..
            }
        ));

        let map = CodeMap([(0x1002, Mapped::Thumb)].into());
        assert!(matches!(
            refused(listing, &map),
            StackError::Undecoded {
                address: 0x1000,
                ..
            }
        ));

        let map = CodeMap([(0x1000, Mapped::Arm)].into());
        assert!(matches!(
            refused(listing, &map),
            StackError::Undecoded {
                address: 0x1000,
                ..
            }
        ));
    }

    /// A pool word that decodes as `itttt hi` leaves objdump's IT state on,
    /// so the next function's real prologue and calls print conditional.
    /// They decode, so only the patterns can keep them.
    #[test]
    fn a_prologue_and_calls_after_an_it_decoded_from_data_still_count() {
        let listing = "\
00001000 <before>:
    1000:\tb500      \tpush\t{lr}
    1002:\tbd00      \tpop\t{pc}
    1004:\tbf88      \tit\thi
    1006:\tbf81      \titttt\thi

00001008 <after>:
    1008:\tb510      \tpushhi\t{r4, lr}
    100a:\tb082      \tsubhi\tsp, #8
    100c:\tf000 f800 \tblhi\t2000 <other>
    1010:\t4798      \tblxhi\tr3
    1012:\tf2c0 0000 \tmovthi\tr0, #0
";
        let map = CodeMap(
            [
                (0x1000, Mapped::Thumb),
                (0x1004, Mapped::Data),
                (0x1008, Mapped::Thumb),
            ]
            .into(),
        );
        let functions: HashMap<String, Function> = frames(listing, &map).into_iter().collect();
        let after = &functions["after"];

        assert_eq!(after.frame, 8 + 8);
        assert!(after.calls.contains("other"), "{:?}", after.calls);
        assert!(after.indirect);
        assert!(all_decoded(&functions).is_ok());
    }

    /// Each pattern also takes a condition on a prologue that really has one.
    #[test]
    fn every_instruction_pattern_takes_a_condition() {
        assert_eq!(
            &PUSH.captures("pushls\t{r4, r5, r6, lr}").unwrap()[1],
            "r4, r5, r6, lr"
        );
        assert_eq!(
            &PUSH.captures("stmdbhi.w\tsp!, {r4, lr}").unwrap()[1],
            "r4, lr"
        );
        assert_eq!(&VPUSH.captures("vpushne\t{d8-d9}").unwrap()[2], "9");
        assert_eq!(
            &WRITEBACK.captures("strhi.w\tfp, [sp, #-4]!").unwrap()[1],
            "4"
        );
        assert_eq!(&SUB.captures("subhi\tsp, #112").unwrap()[1], "112");
        assert_eq!(&SUB.captures("subwls\tsp, sp, #1032").unwrap()[1], "1032");
        assert_eq!(&SUB.captures("subhi.w\tsp, sp, #1032").unwrap()[1], "1032");
        assert_eq!(&MOVE.captures("movwhi\tr0, #4097").unwrap()[3], "4097");
        assert!(SUB_REGISTER.is_match("subhi.w\tsp, sp, r3"));
        assert!(INDIRECT.is_match("    1010:\t4798      \tblxhi\tr3"));
        assert!(!INDIRECT.is_match("    1010:\t4770      \tbxhi\tlr"));
    }

    #[test]
    fn mapping_symbols_that_disagree_at_one_address_are_refused() {
        let symbols = "\
   12: 0e001390     0 NOTYPE  LOCAL  DEFAULT    3 $t
   13: 0e001390     0 NOTYPE  LOCAL  DEFAULT    3 $t.4
";
        assert_eq!(CodeMap::read(symbols).unwrap().0.len(), 1);

        let symbols = "\
   12: 0e001390     0 NOTYPE  LOCAL  DEFAULT    3 $t
   13: 0e001390     0 NOTYPE  LOCAL  DEFAULT    3 $d.4
";
        assert!(matches!(
            CodeMap::read(symbols),
            Err(StackError::MappingConflict {
                address: 0x0e00_1390
            })
        ));
    }

    /// A function symbol the mapping symbols call data is refused, wherever
    /// readelf lists it: its code would be read as a pool. A ROM function,
    /// with no bytes in the image, is not the map's to judge, and neither is
    /// an object: the listing heads a table in a code section too.
    #[test]
    fn a_function_that_starts_outside_thumb_code_is_refused() {
        let symbols = "\
   12: 0e001390     0 NOTYPE  LOCAL  DEFAULT    3 $t
   13: 0e0014b8     0 NOTYPE  LOCAL  DEFAULT    3 $d.17
   14: 0e001391    40 FUNC    GLOBAL DEFAULT    3 dns_send
   15: 00001234    40 FUNC    GLOBAL DEFAULT  ABS rom_memcpy
   16: 0e0014b9    18 OBJECT  LOCAL  DEFAULT    3 GDMA_IrqNum
";
        assert!(CodeMap::read(symbols).is_ok());

        let symbols =
            format!("{symbols}   17: 0e0014c1    12 FUNC    LOCAL  DEFAULT    3 hidden\n");
        assert!(matches!(
            CodeMap::read(&symbols),
            Err(StackError::NotCode { function, address: 0x0e00_14c0 }) if function == "hidden"
        ));

        // A size of 100000 or more, which readelf prints in hex.
        let large = "\
   12: 0e001390     0 NOTYPE  LOCAL  DEFAULT    3 $t
   13: 0e0014b8     0 NOTYPE  LOCAL  DEFAULT    3 $d.17
   14: 0e0014c1 0x186a0 FUNC    LOCAL  DEFAULT    3 large
";
        assert!(matches!(
            CodeMap::read(large),
            Err(StackError::NotCode { function, address: 0x0e00_14c0 }) if function == "large"
        ));
    }

    /// A Rust task formatting a record through `core::fmt::write`, whose
    /// `Display` writes again, then calls into RAM through a veneer that
    /// takes the flash lock.
    const CLASSES: &str = "\
00001000 <_RNvNtCs1_3app4task>:
    1000:\tb500      \tpush\t{lr}
    100a:\tf243 0001 \tmovw\tr0, #12289\t; 0x3001
    100e:\tf2c0 0000 \tmovt\tr0, #0
    1002:\tf000 f800 \tbl\t2000 <_RNvNtCs2_4core3fmt5write>
    1006:\tf000 f800 \tbl\t5000 <__FLASH_Write_Lock_veneer>

00002000 <_RNvNtCs2_4core3fmt5write>:
    2000:\tb5f0      \tpush\t{r4, r5, r6, r7, lr}
    2002:\tb090      \tsub\tsp, #64
    2004:\t4798      \tblx\tr3

00003000 <_RNvXNtCs1_3app4LineNtNtCs2_4core3fmt7Display3fmt>:
    3000:\tb580      \tpush\t{r7, lr}
    3002:\tb088      \tsub\tsp, #32
    3004:\tf000 f800 \tbl\t2000 <_RNvNtCs2_4core3fmt5write>
    3008:\t00004001 \t.word\t0x00004001

00004000 <_RNvXNtCs1_3app6ReportNtNtCs2_4core3fmt5Write9write_str>:
    4000:\tb510      \tpush\t{r4, lr}

00005000 <__FLASH_Write_Lock_veneer>:
    5000:\tb401      \tpush\t{r0}
    5002:\t4760      \tbx\tip

00006000 <FLASH_Write_Lock>:
    6000:\tb538      \tpush\t{r3, r4, r5, lr}
    6002:\t4790      \tblx\tr2

00007000 <_RNvNtCs3_11wellfed_sdk7runtime12hook_acquire>:
    7000:\tb5f0      \tpush\t{r4, r5, r6, r7, lr}
    7002:\tb0c0      \tsub\tsp, #256

00008000 <_RNvNtCs1_3app6callee>:
    8000:\tb500      \tpush\t{lr}
    8002:\t4798      \tblx\tr3
";

    #[test]
    fn indirect_calls_are_charged_by_class_through_veneers() {
        let functions = link(frames(CLASSES, &classes_map()).into_iter().collect());
        let graph = Graph::new(&functions, &[]);
        let task = "_RNvNtCs1_3app4task";
        assert_eq!(
            graph.fmt_targets(task).into_iter().collect::<Vec<_>>(),
            [
                "_RNvXNtCs1_3app4LineNtNtCs2_4core3fmt7Display3fmt",
                "_RNvXNtCs1_3app6ReportNtNtCs2_4core3fmt5Write9write_str"
            ],
            "the task's Display, and the Write it references"
        );
        assert!(graph.fmt_targets("_RNvNtCs1_3app6callee").is_empty());
        let (charges, through_fmt) = graph.charges(task, graph.hook_charge(Runtime::Sdk).unwrap());
        assert!(through_fmt.is_empty());

        // `write` (84) under each nesting level: Display (40) + write again,
        // four levels deep, the innermost charged nothing.
        let write = 20 + 64;
        let display = 8 + 32;
        assert_eq!(charges.fmt, 4 * (display + write));
        assert_eq!(charges.hook, 20 + 256, "the acquire callback");

        let ((depth, path, unbounded), _) = graph.worst(task, charges);
        let lock = 4 + 16 + charges.hook;
        assert_eq!(depth, 4 + (write + charges.fmt).max(lock));
        assert_eq!(
            path[..3],
            ["_RNvNtCs1_3app4task", "_RNvNtCs2_4core3fmt5write", "<fmt>"]
        );
        assert!(unbounded.is_empty());
    }

    #[test]
    fn an_unclassified_rust_indirect_call_is_reported_for_refusal() {
        let functions = link(frames(CLASSES, &classes_map()).into_iter().collect());
        let graph = Graph::new(&functions, &[]);

        let ((_, _, unbounded), _) = graph.worst("_RNvNtCs1_3app6callee", Charges::default());
        assert_eq!(unbounded, ["_RNvNtCs1_3app6callee".to_string()].into());
        assert!(FMT_METHOD.is_match("_RNvXNtCs1_3app4LineNtNtCs2_4core3fmt7Display3fmt"));
        assert!(!FMT_METHOD.is_match("_RNvNtCs2_4core3fmt5write"));
    }

    #[test]
    fn an_unbounded_call_reached_only_through_formatting_is_reported() {
        let listing = CLASSES.replace(
            "    3008:\t00004001",
            "    3006:\tf000 f800 \tbl\t8000 <_RNvNtCs1_3app6callee>\n    3008:\t00004001",
        );
        let functions = link(frames(&listing, &code_only()).into_iter().collect());
        let graph = Graph::new(&functions, &[]);

        let task = "_RNvNtCs1_3app4task";
        let (charges, through_fmt) = graph.charges(task, graph.hook_charge(Runtime::Sdk).unwrap());
        let ((_, _, direct), _) = graph.worst(task, charges);
        assert!(direct.is_empty(), "the task's own walk never meets it");
        assert_eq!(through_fmt, ["_RNvNtCs1_3app6callee".to_string()].into());
    }

    #[test]
    fn a_hook_callback_reaching_an_unbounded_rust_call_is_refused() {
        let listing = CLASSES.replace(
            "    7002:\tb0c0      \tsub\tsp, #256\n",
            "    7002:\tb0c0      \tsub\tsp, #256\n    7004:\tf000 f800 \tbl\t8000 <_RNvNtCs1_3app6callee>\n",
        );
        assert_ne!(listing, CLASSES);
        let functions = link(frames(&listing, &code_only()).into_iter().collect());
        assert!(matches!(
            stack_error(Graph::new(&functions, &[]).hook_charge(Runtime::Sdk)),
            StackError::CallbackUnbounded { .. }
        ));
    }

    #[test]
    fn provider_callbacks_without_a_hook_site_are_refused() {
        let listing = "\
00007000 <_RNvNtCs3_11wellfed_sdk7runtime12hook_acquire>:
    7000:\tb5f0      \tpush\t{r4, r5, r6, r7, lr}
";
        let functions = link(frames(listing, &code_only()).into_iter().collect());
        assert!(matches!(
            stack_error(Graph::new(&functions, &[]).hook_charge(Runtime::Sdk)),
            StackError::NoHookSite
        ));
    }

    #[test]
    fn hooks_without_a_provider_callback_are_refused() {
        let listing = "\
00006000 <FLASH_Write_Lock>:
    6000:\tb538      \tpush\t{r3, r4, r5, lr}
    6002:\t4790      \tblx\tr2
";
        let functions = link(frames(listing, &code_only()).into_iter().collect());
        assert!(matches!(
            stack_error(Graph::new(&functions, &[]).hook_charge(Runtime::Sdk)),
            StackError::NoCallback
        ));

        // A bare firmware installs no provider: the hooks charge nothing.
        let graph = Graph::new(&functions, &[]);
        assert_eq!(graph.hook_charge(Runtime::Bare).unwrap(), 0);
    }

    /// A bare firmware's own `task_create` calls are its task table, and the
    /// SDK runtime's are not read.
    #[test]
    fn a_bare_firmware_is_its_own_task_table() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let repo = Repo { root };
        let firmware = Firmware::new(&repo, "fountain").unwrap();
        let sources = task_sources(&repo, &firmware).unwrap();

        assert_eq!(sources.kind, Runtime::Bare);
        let found = tasks(&sources.runtime).unwrap();
        assert_eq!(
            found,
            [("beat_task".to_string(), ("wellfed-boot".to_string(), 4096))].into()
        );
        assert!(sources.firmware().trim().is_empty());
    }

    /// A bare firmware's image is refused if it links any of the SDK
    /// runtime, by its own functions or by a provider callback.
    #[test]
    fn a_bare_image_linking_the_sdk_runtime_is_found() {
        let bare = [
            "_RNvNtCs1_19wellfed_fw_fountain3app9beat_task".to_string(),
            "wellfed_amebad_flash_provider_fault".to_string(),
            "_RNvNtCs2_11wellfed_hal7stopped10initialize".to_string(),
        ];
        assert!(sdk_runtime_in(bare.iter()).is_empty());

        let runtime = [
            bare[0].clone(),
            "_RNvNtCs3_11wellfed_sdk7runtime15diagnostic_task".to_string(),
            "_RNvNtCs4_5other7runtime12hook_acquire".to_string(),
        ];
        assert_eq!(
            sdk_runtime_in(runtime.iter()),
            [
                "other::runtime::hook_acquire",
                "wellfed_sdk::runtime::diagnostic_task"
            ]
        );
    }

    /// A call graph from `(name, frame, callees)`.
    fn graph_of(edges: &[(&str, u64, &[&str])]) -> HashMap<String, Function> {
        edges
            .iter()
            .map(|(name, frame, calls)| {
                let function = Function {
                    frame: *frame,
                    calls: calls.iter().map(|c| c.to_string()).collect(),
                    ..Function::default()
                };
                (name.to_string(), function)
            })
            .collect()
    }

    /// `x` is first walked under `a`, so its edge back to `a` is cut and its
    /// result leaves out `a`'s frames. From `b`, where `a` is not on the
    /// path, `x` calls `a` for real, and the worst path goes through it.
    #[test]
    fn a_result_cut_against_an_ancestor_is_walked_again_from_another_path() {
        let functions = graph_of(&[
            ("root", 1, &["a", "b"]),
            ("a", 100, &["d", "x"]),
            ("d", 500, &[]),
            ("x", 10, &["a", "leaf"]),
            ("leaf", 1, &[]),
            ("b", 1, &["x"]),
        ]);
        let graph = Graph::new(&functions, &[]);

        let ((depth, path, _), cut) = graph.worst("root", Charges::default());
        assert_eq!(depth, 1 + 1 + 10 + 100 + 500, "root, b, x, a, d");
        assert_eq!(path, ["root", "b", "x", "a", "d"]);
        assert_eq!(cut, ["a"]);

        let (mut memo, mut cut) = (HashMap::new(), BTreeSet::new());
        graph.walk(
            "a",
            Charges::default(),
            &mut Vec::new(),
            &mut memo,
            &mut cut,
        );
        assert!(!memo.contains_key("x"), "x's result depends on a above it");
        assert_eq!(memo["a"].0, 600, "a closed its own cycle");
    }

    /// A walk past its visit budget stops short and marks the graph, which
    /// `audit` refuses, so a short number is never a bound.
    #[test]
    fn a_walk_past_its_budget_marks_the_graph_exhausted() {
        let functions = graph_of(&[
            ("root", 1, &["a", "b"]),
            ("a", 1, &["b", "c"]),
            ("b", 1, &["c", "a"]),
            ("c", 1, &["a", "b"]),
        ]);
        let mut graph = Graph::new(&functions, &[]);
        graph.worst("root", Charges::default());
        assert!(!graph.exhausted(), "{} visits", graph.visits.get());

        let used = graph.visits.get();
        graph.budget = used / 2;
        graph.visits.set(0);
        graph.worst("root", Charges::default());
        assert!(graph.exhausted());
    }

    /// A cycle that closes on `x` itself leaves `x`'s result the same from
    /// any caller, so it is memoised, and the cut is still reported.
    #[test]
    fn a_cycle_closed_inside_a_function_is_memoised_and_reported() {
        let functions = graph_of(&[
            ("root", 1, &["p", "q", "r"]),
            ("p", 2, &["x"]),
            ("q", 3, &["x"]),
            ("x", 10, &["y"]),
            ("y", 20, &["leaf", "x"]),
            ("leaf", 5, &[]),
            ("r", 100, &["y"]),
        ]);
        let graph = Graph::new(&functions, &[]);

        // From `r`, `y` calls `x` for real: `y`'s result under `x` (where
        // that call was cut) must not be reused, which gave 1 + 100 + 25.
        // Reusing `x`'s memo inside its own cycle may count `y` twice: an
        // over-count, safe, so only the floor is pinned.
        let ((depth, path, _), cut) = graph.worst("root", Charges::default());
        assert!(depth >= 1 + 100 + 20 + 10, "{depth}: root, r, y, x");
        assert_eq!(path[..4], ["root", "r", "y", "x"]);
        assert_eq!(cut, ["x"]);

        let (mut memo, mut cut) = (HashMap::new(), BTreeSet::new());
        let (_, reached) = graph.walk(
            "p",
            Charges::default(),
            &mut Vec::new(),
            &mut memo,
            &mut cut,
        );
        assert_eq!(reached, None);
        assert_eq!(memo["x"].0, 35);
        assert!(!memo.contains_key("y"), "y's cut landed on x above it");
    }
}
