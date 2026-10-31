//! Each tool's command line, read from its clap derives with `syn`.
//!
//! The docs check needs to know that `wellfed backup read` is a command and
//! that `--ssid` belongs to `install`. Asking the binaries would mean building
//! every tool (the Bluetooth and emulator stacks included) in a check that
//! otherwise builds nothing, and running each one. Reading the derives needs
//! no build and runs no tool, so it can never touch a feeder.
//!
//! This reads what clap derives, not all clap can do: the tools build no
//! command by hand, and use neither `rename_all` nor `external_subcommand`.
//! A type it cannot resolve is an error, never a pass, so a change that leaves
//! this reader behind fails the check instead of quietly checking nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use syn::punctuated::Punctuated;
use syn::{Attribute, Expr, ExprLit, Field, Fields, Item, Lit, Meta, Token, Type};

/// Why a tool's command line could not be read.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("cannot read {}", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot parse {}", path.display())]
    Parse { path: PathBuf, source: syn::Error },
    #[error("{tool} has no clap type `{ty}`")]
    Unresolved { tool: &'static str, ty: String },
    #[error("{tool}'s `{ty}` could be any of {files:?}")]
    Ambiguous {
        tool: &'static str,
        ty: String,
        files: Vec<String>,
    },
    #[error("{tool}'s `{ty}` contains itself")]
    Cycle { tool: &'static str, ty: String },
}

/// A tool whose command line the docs name, and the crates its clap types
/// live in.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Tool {
    pub(crate) name: &'static str,
    pub(crate) sources: &'static [&'static str],
}

/// Every binary whose commands the docs spell out.
pub(crate) const TOOLS: [Tool; 8] = [
    Tool {
        name: "wellfed",
        sources: &["src/tools/wellfed/src/"],
    },
    Tool {
        name: "wellfed-bench",
        sources: &[
            "src/tools/wellfed-bench/src/",
            "src/tools/wellfed-bench-server/src/",
            "src/tools/wellfed-rig/src/",
            "src/tools/wellfed-itest/src/",
        ],
    },
    Tool {
        name: "wellfed-stock",
        sources: &["src/tools/wellfed-stock/src/"],
    },
    Tool {
        name: "wellfed-flash",
        sources: &["src/tools/wellfed-flash/src/"],
    },
    Tool {
        name: "wellfed-build",
        sources: &["src/tools/wellfed-build/src/"],
    },
    Tool {
        name: "wellfed-audit",
        sources: &["src/tools/wellfed-audit/src/"],
    },
    Tool {
        name: "wellfed-server",
        sources: &["src/server/wellfed-server/src/"],
    },
    Tool {
        name: "wellfed-feeder-sim",
        sources: &["src/tools/wellfed-feeder-sim/src/"],
    },
];

/// Whether a flag eats the word after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Takes {
    Nothing,
    Value,
}

/// One command: its flags and its subcommands.
#[derive(Debug, Default)]
pub(crate) struct Node {
    flags: BTreeMap<String, Takes>,
    /// Flags that hold on every command below this one too.
    globals: BTreeMap<String, Takes>,
    commands: BTreeMap<String, Node>,
    positionals: bool,
}

/// What a command as a doc spells it gets wrong.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    NoCommand { under: String, word: String },
    NoFlag { under: String, flag: String },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoCommand { under, word } => write!(f, "`{under}` has no command `{word}`"),
            Self::NoFlag { under, flag } => write!(f, "`{under}` has no flag `--{flag}`"),
        }
    }
}

impl Node {
    /// Walk `words`, the command line after the tool's name. A placeholder
    /// where a command should be ends the walk, since nothing after it can
    /// be placed.
    pub(crate) fn verify(&self, tool: &str, words: &[&str]) -> Result<(), Refusal> {
        let mut node = self;
        let mut path = tool.to_string();
        let mut inherited = self.globals.clone();
        let mut words = words.iter().copied();

        while let Some(raw) = words.next() {
            if raw == "--" {
                return Ok(());
            }
            let word = raw.trim_matches(|c| "[](),;:.'\"".contains(c));
            if word.is_empty() {
                continue;
            }

            if let Some(flag) = word.strip_prefix("--") {
                if is_placeholder(flag) {
                    continue;
                }
                let (name, value) = match flag.split_once('=') {
                    Some((name, _)) => (name, true),
                    None => (flag, false),
                };
                for name in name.split('|') {
                    let name = name.trim_start_matches('-');
                    if matches!(name, "help" | "version") {
                        continue;
                    }
                    let takes = node.flags.get(name).or_else(|| inherited.get(name));
                    let Some(takes) = takes else {
                        return Err(Refusal::NoFlag {
                            under: path,
                            flag: name.to_string(),
                        });
                    };
                    if *takes == Takes::Value && !value {
                        words.next();
                    }
                }
                continue;
            }
            // A short flag may take a value: nothing after it can be placed.
            if word.starts_with('-') {
                return Ok(());
            }

            if node.commands.is_empty() {
                continue;
            }
            if is_placeholder(word) {
                return Ok(());
            }
            let alternatives: Vec<&str> = word.split(['|', '/']).collect();
            if let [only] = alternatives[..]
                && let Some(next) = node.commands.get(only)
            {
                inherited.extend(next.globals.clone());
                path = format!("{path} {only}");
                node = next;
                continue;
            }
            if node.positionals {
                continue;
            }
            for alternative in &alternatives {
                if !node.commands.contains_key(*alternative) {
                    return Err(Refusal::NoCommand {
                        under: path,
                        word: (*alternative).to_string(),
                    });
                }
            }
            return Ok(());
        }
        Ok(())
    }
}

/// `<review>`, `…`, `NAME` and their like, which stand for a real word.
fn is_placeholder(word: &str) -> bool {
    word.contains(['<', '>', '…', '$', '{', '}', '*'])
        || word.contains("...")
        || !word.chars().any(|c| c.is_ascii_lowercase())
}

/// A clap type and the file it came from.
struct Definition {
    file: String,
    item: Item,
}

/// All of one tool's clap types.
struct Definitions {
    tool: &'static str,
    by_name: BTreeMap<String, Vec<Definition>>,
}

/// Read `tool`'s command line. `names` are the checkout's files.
pub(crate) fn read(root: &Path, names: &[String], tool: Tool) -> Result<Node, Error> {
    let mut definitions = Definitions {
        tool: tool.name,
        by_name: BTreeMap::new(),
    };
    for name in names {
        let is_source = name.ends_with(".rs")
            && !name.ends_with("/tests.rs")
            && tool.sources.iter().any(|source| name.starts_with(source));
        if !is_source {
            continue;
        }
        let path = root.join(name);
        let source = fs::read_to_string(&path).map_err(|source| Error::Read {
            path: path.clone(),
            source,
        })?;
        definitions.add(name, &source)?;
    }

    let mut node = Node::default();
    definitions.merge("Cli", None, &mut node, &mut BTreeSet::new())?;
    Ok(node)
}

impl Definitions {
    fn add(&mut self, file: &str, source: &str) -> Result<(), Error> {
        let parsed = syn::parse_file(source).map_err(|source| Error::Parse {
            path: PathBuf::from(file),
            source,
        })?;
        self.items(file, parsed.items);
        Ok(())
    }

    /// Collect the clap types in `items`, leaving test modules out: a test's
    /// own `Cli` is not the tool's.
    fn items(&mut self, file: &str, items: Vec<Item>) {
        for item in items {
            match &item {
                Item::Struct(s) if derives_clap(&s.attrs) => {
                    let name = s.ident.to_string();
                    self.insert(name, file, item);
                }
                Item::Enum(e) if derives_clap(&e.attrs) => {
                    let name = e.ident.to_string();
                    self.insert(name, file, item);
                }
                Item::Mod(m) if !is_test(&m.attrs) => {
                    if let Item::Mod(syn::ItemMod {
                        content: Some((_, inner)),
                        ..
                    }) = item
                    {
                        self.items(file, inner);
                    }
                }
                _ => {}
            }
        }
    }

    fn insert(&mut self, name: String, file: &str, item: Item) {
        self.by_name.entry(name).or_default().push(Definition {
            file: file.to_string(),
            item,
        });
    }

    /// The definition a type path names, seen from `from`: the only one of
    /// that name, or the one in the module the path qualifies it with, or the
    /// one in the same file. Two modules can share a stem (`plans.rs` and
    /// `server/plans.rs`), and a module path reaches its sibling first.
    fn find(
        &self,
        ty: &str,
        qualifier: Option<&str>,
        from: Option<&str>,
    ) -> Result<&Definition, Error> {
        let unresolved = || Error::Unresolved {
            tool: self.tool,
            ty: ty.to_string(),
        };
        let found = self.by_name.get(ty).ok_or_else(unresolved)?;
        if let [only] = &found[..] {
            return Ok(only);
        }

        if let Some(qualifier) = qualifier {
            let modules: Vec<&Definition> = found
                .iter()
                .filter(|d| stem(&d.file) == qualifier)
                .collect();
            if let [only] = modules[..] {
                return Ok(only);
            }
            let sibling = from.and_then(|from| {
                modules.iter().find(|d| {
                    folder(&d.file) == folder(from)
                        || folder(&d.file).ends_with(&format!("/{qualifier}"))
                            && folder(&d.file).starts_with(folder(from))
                })
            });
            return sibling.copied().ok_or_else(unresolved);
        }
        if let Some(same) = from.and_then(|from| found.iter().find(|d| d.file == from)) {
            return Ok(same);
        }
        Err(Error::Ambiguous {
            tool: self.tool,
            ty: ty.to_string(),
            files: found.iter().map(|d| d.file.clone()).collect(),
        })
    }

    /// Add a type's flags or subcommands to `node`.
    fn merge(
        &self,
        ty: &str,
        qualified: Option<(Option<&str>, &str)>,
        node: &mut Node,
        stack: &mut BTreeSet<String>,
    ) -> Result<(), Error> {
        let (qualifier, from) = qualified.map_or((None, None), |(q, f)| (q, Some(f)));
        let definition = self.find(ty, qualifier, from)?;
        let key = format!("{}::{ty}", definition.file);
        if !stack.insert(key.clone()) {
            return Err(Error::Cycle {
                tool: self.tool,
                ty: ty.to_string(),
            });
        }

        match &definition.item {
            Item::Struct(s) => {
                for field in &s.fields {
                    self.field(field, &definition.file, node, stack)?;
                }
            }
            Item::Enum(e) => {
                for variant in &e.variants {
                    self.variant(variant, &definition.file, node, stack)?;
                }
            }
            _ => {}
        }

        stack.remove(&key);
        Ok(())
    }

    fn variant(
        &self,
        variant: &syn::Variant,
        file: &str,
        node: &mut Node,
        stack: &mut BTreeSet<String>,
    ) -> Result<(), Error> {
        let attributes = Attributes::of(&variant.attrs);
        if attributes.flatten {
            for field in &variant.fields {
                self.nested(&field.ty, file, node, stack)?;
            }
            return Ok(());
        }

        let mut child = Node::default();
        match &variant.fields {
            Fields::Named(fields) => {
                for field in &fields.named {
                    self.field(field, file, &mut child, stack)?;
                }
            }
            Fields::Unnamed(fields) => {
                for field in &fields.unnamed {
                    self.nested(&field.ty, file, &mut child, stack)?;
                }
            }
            Fields::Unit => {}
        }

        let name = attributes
            .name
            .unwrap_or_else(|| kebab(&variant.ident.to_string()));
        if attributes.skip {
            return Ok(());
        }
        node.commands.insert(name, child);
        Ok(())
    }

    fn field(
        &self,
        field: &Field,
        file: &str,
        node: &mut Node,
        stack: &mut BTreeSet<String>,
    ) -> Result<(), Error> {
        let attributes = Attributes::of(&field.attrs);
        if attributes.skip {
            return Ok(());
        }
        if attributes.flatten || attributes.subcommand {
            return self.nested(&field.ty, file, node, stack);
        }

        let name = field.ident.as_ref().map(ToString::to_string);
        let long = attributes
            .long
            .map(|long| long.unwrap_or_else(|| kebab(&name.unwrap_or_default())));
        let Some(long) = long else {
            node.positionals = node.positionals || !attributes.short;
            return Ok(());
        };

        let takes = if is_bool(&field.ty) {
            Takes::Nothing
        } else {
            Takes::Value
        };
        let target = if attributes.global {
            &mut node.globals
        } else {
            &mut node.flags
        };
        target.insert(long, takes);
        Ok(())
    }

    /// Merge the clap type `ty` names, through `Option` or `Box`.
    fn nested(
        &self,
        ty: &Type,
        file: &str,
        node: &mut Node,
        stack: &mut BTreeSet<String>,
    ) -> Result<(), Error> {
        let (name, qualifier) = named_type(ty).ok_or_else(|| Error::Unresolved {
            tool: self.tool,
            ty: "a type with no plain name".to_string(),
        })?;
        self.merge(&name, Some((qualifier.as_deref(), file)), node, stack)
    }
}

/// What `#[arg]`, `#[command]` and `#[clap]` say about a field or a variant.
#[derive(Default)]
struct Attributes {
    /// `long`, with its spelled-out name if it has one.
    long: Option<Option<String>>,
    short: bool,
    global: bool,
    flatten: bool,
    subcommand: bool,
    skip: bool,
    name: Option<String>,
}

impl Attributes {
    fn of(attrs: &[Attribute]) -> Self {
        let mut found = Self::default();
        for attr in attrs {
            let ours = ["arg", "command", "clap"]
                .iter()
                .any(|name| attr.path().is_ident(name));
            if !ours {
                continue;
            }
            let Ok(metas) = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            else {
                continue;
            };
            for meta in metas {
                let key = meta.path().get_ident().map(ToString::to_string);
                match (key.as_deref(), &meta) {
                    (Some("long"), Meta::NameValue(pair)) => {
                        found.long = Some(text(&pair.value));
                    }
                    (Some("long"), _) => found.long = Some(None),
                    (Some("short"), _) => found.short = true,
                    (Some("global"), Meta::NameValue(pair)) => {
                        found.global = !matches!(
                            pair.value,
                            Expr::Lit(ExprLit {
                                lit: Lit::Bool(ref b),
                                ..
                            }) if !b.value
                        );
                    }
                    (Some("global"), _) => found.global = true,
                    (Some("flatten"), _) => found.flatten = true,
                    (Some("subcommand"), _) => found.subcommand = true,
                    (Some("skip"), _) => found.skip = true,
                    (Some("name"), Meta::NameValue(pair)) => found.name = text(&pair.value),
                    _ => {}
                }
            }
        }
        found
    }
}

fn text(value: &Expr) -> Option<String> {
    match value {
        Expr::Lit(ExprLit {
            lit: Lit::Str(s), ..
        }) => Some(s.value()),
        _ => None,
    }
}

fn derives_clap(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("derive")
            && attr
                .parse_args_with(Punctuated::<syn::Path, Token![,]>::parse_terminated)
                .is_ok_and(|paths| {
                    paths.iter().any(|path| {
                        path.segments.last().is_some_and(|last| {
                            last.ident == "Parser"
                                || last.ident == "Args"
                                || last.ident == "Subcommand"
                        })
                    })
                })
    })
}

fn is_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr
                .parse_args::<syn::Ident>()
                .is_ok_and(|ident| ident == "test")
    })
}

fn is_bool(ty: &Type) -> bool {
    matches!(ty, Type::Path(path) if path.path.is_ident("bool"))
}

/// The last name in a type path and the module before it, through `Option`
/// and `Box`.
fn named_type(ty: &Type) -> Option<(String, Option<String>)> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segments: Vec<_> = path.path.segments.iter().collect();
    let last = segments.last()?;

    if last.ident == "Option" || last.ident == "Box" {
        let syn::PathArguments::AngleBracketed(arguments) = &last.arguments else {
            return None;
        };
        let syn::GenericArgument::Type(inner) = arguments.args.first()? else {
            return None;
        };
        return named_type(inner);
    }

    let qualifier = segments
        .len()
        .checked_sub(2)
        .and_then(|before| segments.get(before))
        .map(|segment| segment.ident.to_string());
    Some((last.ident.to_string(), qualifier))
}

/// A source file's module name.
fn stem(file: &str) -> &str {
    let name = file.rsplit('/').next().unwrap_or(file);
    let name = name.strip_suffix(".rs").unwrap_or(name);
    if name == "mod" {
        let mut parts = file.rsplit('/');
        parts.next();
        parts.next().unwrap_or(name)
    } else {
        name
    }
}

/// The folder a source file sits in.
fn folder(file: &str) -> &str {
    file.rsplit_once('/').map_or("", |(folder, _)| folder)
}

/// The name clap derives for a field or a variant.
fn kebab(identifier: &str) -> String {
    let identifier = identifier.strip_prefix("r#").unwrap_or(identifier);
    let chars: Vec<char> = identifier.chars().collect();
    let mut out = String::new();
    for (index, &c) in chars.iter().enumerate() {
        if c == '_' {
            out.push('-');
            continue;
        }
        let previous = index.checked_sub(1).and_then(|i| chars.get(i));
        let next = chars.get(index + 1);
        let boundary = c.is_uppercase()
            && previous.is_some_and(|p| {
                p.is_lowercase()
                    || p.is_ascii_digit()
                    || (p.is_uppercase() && next.is_some_and(|n| n.is_lowercase()))
            });
        if boundary {
            out.push('-');
        }
        out.extend(c.to_lowercase());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = r#"
        #[derive(Parser)]
        struct Cli {
            #[arg(long, global = true)]
            root: String,
            #[command(subcommand)]
            command: Command,
        }

        #[derive(Subcommand)]
        enum Command {
            /// A unit command.
            Status,
            UpdateKey(update::Args),
            #[command(name = "go-home")]
            Home { #[arg(long)] quiet: bool, #[arg(long = "in")] within: String, place: String },
            #[command(flatten)]
            Job(Job),
        }

        #[derive(Subcommand)]
        enum Job { StockOta { #[arg(long)] review: String } }

        #[derive(Args)]
        struct Shared { #[arg(long)] keys: String }

        #[cfg(test)]
        mod tests {
            #[derive(Parser)]
            struct Cli { #[arg(long)] only_in_a_test: bool }
        }
    "#;

    const UPDATE: &str = r#"
        #[derive(Args)]
        struct Args { #[command(flatten)] shared: Shared, #[command(subcommand)] action: Action }
        #[derive(Subcommand)]
        enum Action { New, Enroll }
        #[derive(Args)]
        struct Shared { #[arg(long)] update_file: String }
    "#;

    fn tree() -> Node {
        let mut definitions = Definitions {
            tool: "tool",
            by_name: BTreeMap::new(),
        };
        definitions.add("src/main.rs", SOURCE).unwrap();
        definitions.add("src/update.rs", UPDATE).unwrap();
        let mut node = Node::default();
        definitions
            .merge("Cli", None, &mut node, &mut BTreeSet::new())
            .unwrap();
        node
    }

    fn verify(line: &str) -> Result<(), Refusal> {
        let words: Vec<&str> = line.split_whitespace().collect();
        tree().verify("tool", &words)
    }

    #[test]
    fn a_real_command_passes() {
        for line in [
            "",
            "status",
            "status --root here",
            "--root here status",
            "update-key --update-file f new",
            "update-key new|enroll",
            "update-key new/enroll",
            "go-home somewhere --quiet --in 5",
            "go-home --quiet somewhere",
            "stock-ota --review <review>",
            "stock-ota --review=x",
            "<command> --help",
            "NAME --anything",
            "update-key … --anything",
            "status --help",
        ] {
            assert_eq!(verify(line), Ok(()), "{line}");
        }
    }

    #[test]
    fn a_wrong_command_is_refused() {
        assert_eq!(
            verify("retired"),
            Err(Refusal::NoCommand {
                under: "tool".to_string(),
                word: "retired".to_string(),
            })
        );
        assert_eq!(
            verify("update-key new|retired"),
            Err(Refusal::NoCommand {
                under: "tool update-key".to_string(),
                word: "retired".to_string(),
            })
        );
    }

    #[test]
    fn a_flag_on_the_wrong_subcommand_is_refused() {
        assert_eq!(
            verify("status --review x"),
            Err(Refusal::NoFlag {
                under: "tool status".to_string(),
                flag: "review".to_string(),
            })
        );
        // `--quiet` takes no value, so `place` is not eaten; a test's `Cli`
        // does not count.
        assert!(verify("--only-in-a-test status").is_err());
    }

    #[test]
    fn a_qualified_type_resolves_by_its_module() {
        // `Shared` is in both files: `update.rs`'s reaches `--update-file`.
        assert_eq!(verify("update-key --update-file x new"), Ok(()));
        assert!(verify("update-key --keys x new").is_err());
    }

    #[test]
    fn an_unknown_type_is_an_error_not_a_pass() {
        let mut definitions = Definitions {
            tool: "tool",
            by_name: BTreeMap::new(),
        };
        definitions
            .add(
                "src/main.rs",
                "#[derive(Parser)] struct Cli { #[command(flatten)] gone: Gone }",
            )
            .unwrap();
        let result = definitions.merge("Cli", None, &mut Node::default(), &mut BTreeSet::new());
        assert!(matches!(result, Err(Error::Unresolved { .. })));
    }

    #[test]
    fn names_are_kebab_case_as_clap_derives_them() {
        for (identifier, name) in [
            ("UpdateKey", "update-key"),
            ("from_bank0", "from-bank0"),
            ("StockOta", "stock-ota"),
            ("Bank0Read", "bank0-read"),
            ("r#type", "type"),
            ("NTPServer", "ntp-server"),
        ] {
            assert_eq!(kebab(identifier), name);
        }
    }

    #[test]
    fn a_module_path_reaches_its_sibling_over_a_namesake() {
        let mut definitions = Definitions {
            tool: "tool",
            by_name: BTreeMap::new(),
        };
        let cli = "#[derive(Parser)] struct Cli { #[command(subcommand)] c: C }\n\
                   #[derive(Subcommand)] enum C { Top(plans::Args), Server(server::Args) }";
        let server = "#[derive(Args)] struct Args { #[command(subcommand)] c: S }\n\
                      #[derive(Subcommand)] enum S { Plans(plans::Args) }";
        definitions.add("src/main.rs", cli).unwrap();
        definitions
            .add(
                "src/plans.rs",
                "#[derive(Args)] struct Args { #[arg(long)] keys: u8 }",
            )
            .unwrap();
        definitions.add("src/server/mod.rs", server).unwrap();
        definitions
            .add(
                "src/server/plans.rs",
                "#[derive(Args)] struct Args { #[arg(long)] token: u8 }",
            )
            .unwrap();

        let mut node = Node::default();
        definitions
            .merge("Cli", None, &mut node, &mut BTreeSet::new())
            .unwrap();

        assert_eq!(node.verify("tool", &["top", "--keys", "1"]), Ok(()));
        assert_eq!(
            node.verify("tool", &["server", "plans", "--token", "1"]),
            Ok(())
        );
        assert!(
            node.verify("tool", &["server", "plans", "--keys", "1"])
                .is_err()
        );
    }

    #[test]
    fn a_module_file_takes_its_folder_name() {
        assert_eq!(stem("src/server/mod.rs"), "server");
        assert_eq!(stem("src/update.rs"), "update");
    }
}

#[cfg(test)]
mod repository_tests {
    use super::*;

    /// The real tools, so a derive this reader can't follow fails here and
    /// not only as a puzzling line from the docs check.
    #[test]
    fn every_tool_reads_and_holds_what_the_docs_rely_on() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let root = root.canonicalize().unwrap();
        let names = crate::repository::listed(&root).unwrap();

        for (name, line) in [
            ("wellfed", "install --ssid x --from-bank0"),
            ("wellfed", "server login --server x --name y"),
            ("wellfed", "update-key enroll"),
            ("wellfed-bench", "itest --only x --off-seconds 2"),
            ("wellfed-bench", "--root . status"),
            ("wellfed-stock", "network apply --config x"),
            ("wellfed-flash", "read observe"),
            ("wellfed-build", "build --firmware bringup --variant optics"),
            ("wellfed-audit", "backup"),
            ("wellfed-server", "reset-owner"),
            ("wellfed-feeder-sim", "--state x --listen y"),
        ] {
            let tool = TOOLS.iter().find(|tool| tool.name == name).unwrap();
            let tree = read(&root, &names, *tool).unwrap();
            let words: Vec<&str> = line.split_whitespace().collect();
            assert_eq!(tree.verify(name, &words), Ok(()), "{name} {line}");
        }
    }
}
