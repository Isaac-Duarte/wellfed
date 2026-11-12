//! What the app's screens take from the device API as Dart constants: the
//! feeder's limits, from `wellfed-model`, which the bridge checks every
//! request against. A control offers only what the feeder takes, and its
//! words name the same numbers, with no copy in Dart to drift.
//!
//! They are generated rather than read through the bridge because a
//! widget runs without the Rust library in the app's widget and golden
//! tests. The files are committed, so the app builds without Rust; this
//! test fails when one is stale, and
//! `UPDATE_BINDINGS=1 cargo test -p wellfed-app-bridge --test protocol`
//! rewrites them.

use std::fmt::Write;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// Each limit as Dart names it, with its doc comment.
fn limits() -> [(&'static str, &'static str, u32); 7] {
    use wellfed_model::{
        CALL_PLAYS_MAX, CLIENTS_MAX, PLANS, PORTION_MG_MAX, PORTION_MG_MIN, PORTIONS_MAX,
        PORTIONS_MIN,
    };

    let count = |n: usize| u32::try_from(n).expect("a count the feeder keeps fits in 32 bits");
    [
        (
            "portionsLeast",
            "The fewest portions one feed or meal asks for.",
            u32::from(PORTIONS_MIN),
        ),
        (
            "portionsMost",
            "The most portions one feed or meal asks for.",
            u32::from(PORTIONS_MAX),
        ),
        ("meals", "The meals a feeder keeps.", count(PLANS)),
        (
            "callPlaysMost",
            "The most times one meal plays the meal call.",
            u32::from(CALL_PLAYS_MAX),
        ),
        (
            "clientsMost",
            "The most keys a feeder lists: owners, members and the server.",
            count(CLIENTS_MAX),
        ),
        (
            "portionMgLeast",
            "The lightest portion weight the feeder stores, in milligrams.",
            PORTION_MG_MIN,
        ),
        (
            "portionMgMost",
            "The heaviest portion weight the feeder stores, in milligrams.",
            PORTION_MG_MAX,
        ),
    ]
}

fn limits_dart() -> String {
    let mut out = String::from(
        "// Generated from wellfed-model by src/app/rust/tests/protocol.rs; do not edit.\n\n\
         /// The feeder's limits: it refuses a value outside them, so a control never\n\
         /// offers one.\n\
         abstract final class Limits {\n",
    );
    for (index, (name, doc, value)) in limits().into_iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        writeln!(out, "  /// {doc}").unwrap();
        writeln!(out, "  static const {name} = {value};").unwrap();
    }
    out.push_str("}\n");
    out
}

/// Compare `want` with the committed file at `relative`, or write it.
fn check(relative: &str, want: &str) {
    let path = root().join(relative);
    if std::env::var_os("UPDATE_BINDINGS").is_some() {
        std::fs::write(&path, want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        have == want,
        "{} is stale: run UPDATE_BINDINGS=1 cargo test -p wellfed-app-bridge --test protocol",
        path.display()
    );
}

#[test]
fn the_dart_limits_are_the_feeder_s() {
    check("src/app/lib/src/limits.g.dart", &limits_dart());
}
