//! The server's own stored codes, pinned beside the newest migration.
//!
//! A row read back is decoded strictly for most codes the server owns
//! (models, roles, units, species, jobs, install, command and revoke
//! states), so an older build fails on one it doesn't know. The JSON
//! columns (command outcomes, audit actions, install failures, revoke
//! attempts) read a value they don't know as unknown, but an older build
//! would still lose what it said. The rule that keeps either from reaching
//! an owner: a build that stores a new code adds a migration with it, so
//! an older build opens the database read-only instead. The fixture lists
//! every set, the JSON columns' through every type they nest, under the
//! migration version it shipped with, and this test fails when a set
//! changes while the newest migration stays the same.

use std::collections::{BTreeMap, BTreeSet};

use serde::de::DeserializeOwned;
use serde_json::Value;
use sqlx::migrate::Migrator;
use utoipa::ToSchema;
use wellfed_api::{
    AuditAction, ClientRole, CommandKind, CommandOutcome, CommandState, InstallFailure,
    InstallState, InstallTarget, Job, JobOutcome, Model, RevokeAttempt, RevokeReason, Role,
    Species, TimeFormat, Units,
};
use wellfed_store::CallBefore;

static MIGRATOR: Migrator = sqlx::migrate!();

const PINNED: &str = include_str!("fixtures/codes.txt");

/// Every string a schema names as a value or a field: the variants of a
/// unit or tagged enum, and a JSON value's fields, through every type it
/// nests. utoipa names a nested type by reference rather than inline, so
/// each reference is followed into the schemas `T` collects; a set
/// inside a stored value (a weekday in a plan, a clip in a call) is part
/// of what an older build must read.
fn names<T: ToSchema>() -> BTreeSet<String> {
    fn walk(
        value: &Value,
        nested: &BTreeMap<String, Value>,
        seen: &mut BTreeSet<String>,
        names: &mut BTreeSet<String>,
    ) {
        match value {
            Value::Object(object) => {
                for (key, inner) in object {
                    match (key.as_str(), inner) {
                        ("enum", Value::Array(values)) => {
                            names
                                .extend(values.iter().filter_map(Value::as_str).map(str::to_owned));
                        }
                        ("properties", Value::Object(fields)) => {
                            names.extend(fields.keys().cloned());
                            walk(inner, nested, seen, names);
                        }
                        ("$ref", Value::String(reference)) => {
                            let name = reference.trim_start_matches("#/components/schemas/");
                            let Some(schema) = nested.get(name) else {
                                panic!("{reference} is not among the schemas collected");
                            };
                            if seen.insert(name.to_owned()) {
                                walk(schema, nested, seen, names);
                            }
                        }
                        _ => walk(inner, nested, seen, names),
                    }
                }
            }
            Value::Array(items) => items
                .iter()
                .for_each(|item| walk(item, nested, seen, names)),
            _ => {}
        }
    }

    let mut collected = Vec::new();
    T::schemas(&mut collected);
    let nested: BTreeMap<String, Value> = collected
        .into_iter()
        .map(|(name, schema)| (name, serde_json::to_value(schema).unwrap()))
        .collect();

    let mut found = BTreeSet::new();
    walk(
        &serde_json::to_value(T::schema()).unwrap(),
        &nested,
        &mut BTreeSet::new(),
        &mut found,
    );
    found
}

/// The codes a unit enum is stored as: each variant its schema names, read
/// back through serde, by `code`.
fn codes<T: ToSchema + DeserializeOwned>(code: fn(T) -> &'static str) -> BTreeSet<String> {
    names::<T>()
        .into_iter()
        .map(|name| {
            let value: T = serde_json::from_value(Value::String(name)).unwrap();
            code(value).to_owned()
        })
        .collect()
}

/// Check that every value `T`'s schema names, other than the catch-all a
/// newer server's value reads as, is stored as its JSON spelling by `code`
/// and read back as the same variant by `from_code`.
fn round_trip<T>(code: fn(T) -> &'static str, from_code: fn(&str) -> Option<T>)
where
    T: ToSchema + DeserializeOwned + Copy + PartialEq + std::fmt::Debug,
{
    for name in names::<T>() {
        let value: T = serde_json::from_value(Value::String(name.clone())).unwrap();
        if name == "unknown" && from_code(&name).is_none() {
            continue;
        }
        assert_eq!(
            code(value),
            name,
            "{value:?} is stored as its JSON spelling"
        );
        assert_eq!(from_code(&name), Some(value), "{name} reads back");
    }
}

/// Every stored code reads back as the variant that wrote it. A variant
/// left out of the list `from_code` searches would still compile, and each
/// row it wrote would then read as corrupt, breaking every page over it.
#[test]
fn every_stored_code_reads_back_as_the_variant_that_wrote_it() {
    round_trip(ClientRole::code, ClientRole::from_code);
    round_trip(Role::code, Role::from_code);
    round_trip(Units::code, Units::from_code);
    round_trip(TimeFormat::code, TimeFormat::from_code);
    round_trip(Species::code, Species::from_code);
    round_trip(Job::code, Job::from_code);
    round_trip(JobOutcome::code, JobOutcome::from_code);
    round_trip(CommandKind::code, CommandKind::from_code);
    round_trip(CommandState::code, CommandState::from_code);
    round_trip(InstallState::code, InstallState::from_code);
    round_trip(RevokeReason::code, RevokeReason::from_code);
}

/// Every set, as this build has it.
fn sets() -> Vec<(&'static str, BTreeSet<String>)> {
    vec![
        (
            "model",
            Model::ALL
                .iter()
                .map(|model| model.name().to_owned())
                .collect(),
        ),
        ("client_role", codes::<ClientRole>(ClientRole::code)),
        ("role", codes::<Role>(Role::code)),
        ("units", codes::<Units>(Units::code)),
        ("time_format", codes::<TimeFormat>(TimeFormat::code)),
        ("species", codes::<Species>(Species::code)),
        ("job", codes::<Job>(Job::code)),
        ("job_outcome", codes::<JobOutcome>(JobOutcome::code)),
        ("command_kind", codes::<CommandKind>(CommandKind::code)),
        ("command_state", codes::<CommandState>(CommandState::code)),
        ("install_state", codes::<InstallState>(InstallState::code)),
        ("revoke_reason", codes::<RevokeReason>(RevokeReason::code)),
        ("install_target", names::<InstallTarget>()),
        ("command_outcome_json", names::<CommandOutcome>()),
        ("audit_action_json", names::<AuditAction>()),
        ("install_failure_json", names::<InstallFailure>()),
        ("revoke_attempt_json", names::<RevokeAttempt>()),
        ("call_before_json", call_before()),
    ]
}

/// `CallBefore` is the store's own and has no schema: its tag's values.
fn call_before() -> BTreeSet<String> {
    [CallBefore::None, CallBefore::Clip { bytes: 0, crc: 0 }]
        .into_iter()
        .map(|before| {
            // Every variant is named here, so a new one fails to compile.
            match before {
                CallBefore::None | CallBefore::Clip { .. } => {}
            }
            let value = serde_json::to_value(before).unwrap();
            value["held"].as_str().unwrap().to_owned()
        })
        .collect()
}

fn line(version: i64, name: &str, set: &BTreeSet<String>) -> String {
    let codes: Vec<&str> = set.iter().map(String::as_str).collect();
    format!("{version} {name} {}", codes.join(" "))
}

#[test]
fn every_stored_code_set_is_pinned_at_the_newest_migration() {
    let newest = MIGRATOR.iter().map(|m| m.version).max().unwrap();
    let pinned: Vec<&str> = PINNED
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .filter(|line| line.split(' ').next() == Some(newest.to_string().as_str()))
        .collect();

    let now: Vec<String> = sets()
        .iter()
        .map(|(name, set)| line(newest, name, set))
        .collect();

    assert_eq!(
        pinned,
        now,
        "a stored code set changed with no migration: add a migration with it (even an empty \
         one) and append these lines under its version to tests/fixtures/codes.txt:\n{}",
        now.join("\n")
    );
}
