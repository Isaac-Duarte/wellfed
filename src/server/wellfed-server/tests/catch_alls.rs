//! The growth check (`api_growth`) takes an enum that names `unknown` or
//! `newer` in the document as one with a catch-all: an older reader
//! decodes a value it does not know as that variant, so a new value is
//! growth. The document's name is only a claim; this holds every plain
//! enum that makes it to serde. Each decodes a value no build names as its
//! catch-all (`newer` where it has one, since `unknown` is then a state of
//! its own), and the table is held to every such enum in the document, so
//! a new one is checked from its first build.

use std::collections::BTreeSet;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use wellfed_api::{
    Blocked, Bowl, ClientRole, CommandKind, CommandState, Connection, CurrentUnit, EnrollPath,
    ErrorCode, FeedOutcome, FeedReason, FeederRefusal, FoodLevel, InstallState, Job, JobOutcome,
    LoginFailure, Phase, ReleaseRefusal, ReleaseSigner, RevokeReason, Role, StockBackupState,
    StockSource, StoreState, Units, WifiSource,
};
use wellfed_model::{Bank0Blocked, ResetCause, Skew, UpdateCandidate, UpdateStop};

/// A value no build names.
const NEWER: &str = "a_value_no_build_names";

/// Enums whose `unknown` is a state, with no catch-all: only a client
/// sends them (a query's filter), and a server refuses a value it cannot
/// name rather than reading it as another.
const NO_CATCH_ALL: &[&str] = &["CommandStateFilter"];

/// What `T` reads [`NEWER`] as, by its JSON name; `None` if it refuses it.
fn reads<T: DeserializeOwned + Serialize>() -> Option<String> {
    let value: T = serde_json::from_value(Value::String(NEWER.into())).ok()?;
    let name = serde_json::to_value(value).expect("a read value serializes");
    name.as_str().map(str::to_owned)
}

/// Every plain enum in the document that names a catch-all, and what each
/// reads a value it does not know as.
fn table() -> Vec<(&'static str, Option<String>)> {
    vec![
        ("Bank0Blocked", reads::<Bank0Blocked>()),
        ("Blocked", reads::<Blocked>()),
        ("Bowl", reads::<Bowl>()),
        ("ClientRole", reads::<ClientRole>()),
        ("CommandKind", reads::<CommandKind>()),
        ("CommandState", reads::<CommandState>()),
        ("Connection", reads::<Connection>()),
        ("CurrentUnit", reads::<CurrentUnit>()),
        ("EnrollPath", reads::<EnrollPath>()),
        ("ErrorCode", reads::<ErrorCode>()),
        ("FeedOutcome", reads::<FeedOutcome>()),
        ("FeedReason", reads::<FeedReason>()),
        ("FeederRefusal", reads::<FeederRefusal>()),
        ("FoodLevel", reads::<FoodLevel>()),
        ("InstallState", reads::<InstallState>()),
        ("Job", reads::<Job>()),
        ("JobOutcome", reads::<JobOutcome>()),
        ("LoginFailure", reads::<LoginFailure>()),
        ("Phase", reads::<Phase>()),
        ("ReleaseRefusal", reads::<ReleaseRefusal>()),
        ("ReleaseSigner", reads::<ReleaseSigner>()),
        ("ResetCause", reads::<ResetCause>()),
        ("RevokeReason", reads::<RevokeReason>()),
        ("Role", reads::<Role>()),
        ("Skew", reads::<Skew>()),
        ("StockBackupState", reads::<StockBackupState>()),
        ("StockSource", reads::<StockSource>()),
        ("StoreState", reads::<StoreState>()),
        ("Units", reads::<Units>()),
        ("UpdateCandidate", reads::<UpdateCandidate>()),
        ("UpdateStop", reads::<UpdateStop>()),
        ("WifiSource", reads::<WifiSource>()),
    ]
}

#[test]
fn every_catch_all_the_document_names_reads_a_value_no_build_names() {
    let document = serde_json::to_value(wellfed_server::openapi()).expect("it serializes");
    let schemas = document["components"]["schemas"]
        .as_object()
        .expect("the document has schemas");
    let claimed: Vec<(&String, Vec<&str>)> = schemas
        .iter()
        .filter_map(|(name, schema)| {
            let values: Vec<&str> = schema
                .get("enum")?
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .collect();
            values
                .iter()
                .any(|value| ["unknown", "newer"].contains(value))
                .then_some((name, values))
        })
        .collect();

    let table = table();
    let listed: BTreeSet<&str> = table.iter().map(|(name, _)| *name).collect();
    for (name, values) in &claimed {
        if NO_CATCH_ALL.contains(&name.as_str()) {
            continue;
        }
        assert!(listed.contains(name.as_str()), "{name} is in the table");
        let catch_all = if values.contains(&"newer") {
            "newer"
        } else {
            "unknown"
        };
        let (_, read) = table.iter().find(|(listed, _)| listed == name).unwrap();
        assert_eq!(
            read.as_deref(),
            Some(catch_all),
            "{name} reads a value it does not know as {catch_all}"
        );
    }
    let named: BTreeSet<&str> = claimed.iter().map(|(name, _)| name.as_str()).collect();
    for name in &listed {
        assert!(named.contains(name), "{name} still names a catch-all");
    }
}

/// An enum whose `unknown` is a state, sent only by a client, refuses a
/// value it does not know.
#[test]
fn an_enum_with_no_catch_all_refuses_a_value_no_build_names() {
    assert_eq!(reads::<wellfed_api::CommandStateFilter>(), None);
}
