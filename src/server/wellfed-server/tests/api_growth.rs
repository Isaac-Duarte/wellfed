//! `/api/v1` only grows. The app, the CLI and Home Assistant run other
//! builds than the server they reach, so from the beta the published
//! document must keep every path, operation, field and variant it had:
//! `fixtures/api-v1.json` holds it as of [`REVISION`], and the current
//! document must read as a growth of it.
//!
//! A growth is new paths, operations, response statuses and schemas, new
//! optional fields and query parameters, and new enum variants on an enum
//! that has a catch-all (an `unknown` value, which its older readers
//! decode a newer one as) or whose entries older readers skip
//! ([`SKIPPED`]). JSON names a variant, so a variant is matched by its
//! name, wherever it sits. Anything else is a change in place: a removed or
//! retyped field, a field made required or optional, a variant renamed or
//! removed, a new variant on a closed enum, or a narrower check on a value. No blessing stores a change in place. A growth needs
//! the next [`REVISION`] and a blessing (`WELLFED_BLESS=1`), which stores
//! it ([API evolution](../../../../docs/server/README.md#api-evolution)).
//!
//! Prose (descriptions, summaries, titles, examples) and the document's
//! `info` and tag list are not part of the contract, and are left out.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use wellfed_api::REVISION;

/// The frozen document and the revision it was published at.
#[derive(Serialize, Deserialize)]
struct Fixture {
    revision: u32,
    document: Value,
}

/// Object keys whose values are maps of names (fields, paths, schemas,
/// statuses, media types) rather than schema keywords: a name there may be
/// `title` or `description` and is kept, and a new name is a growth.
const NAME_MAPS: &[&str] = &["properties", "paths", "schemas", "responses", "content"];

/// The keywords that are prose, not contract.
const PROSE: &[&str] = &[
    "description",
    "summary",
    "title",
    "example",
    "examples",
    "tags",
];

/// The HTTP methods a path item holds; a new one is a new operation.
const METHODS: &[&str] = &["get", "put", "post", "delete", "patch", "head", "options"];

/// The enums older clients need no catch-all for: what holds one is an
/// entry `wellfed-server-client` leaves out when it cannot read it (an
/// event in a history page or on the stream, a journal or audit entry, a
/// statistics section, an update on the stream), so a newer variant costs
/// an older client that entry alone. A command outside a page or the
/// stream is one the client sent, whose kind it knows; a sample is read
/// by a metric the client names.
const SKIPPED: &[&str] = &[
    "AuditAction",
    "CommandKind",
    "CoreEvent",
    "FeederEvent",
    "LoginFailure",
    "SampleValue",
    "Stats",
    "StatsSection",
    "StreamEvent",
];

/// The values an enum's catch-all goes by: `unknown`, or `newer` where
/// `unknown` already names a state of its own.
const CATCH_ALLS: &[&str] = &["unknown", "newer"];

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/api-v1.json")
}

/// The document as the contract sees it.
fn current() -> Value {
    let text = wellfed_server::openapi()
        .to_json()
        .expect("the document serializes");
    let mut document: Value = serde_json::from_str(&text).expect("the document is JSON");
    if let Value::Object(top) = &mut document {
        top.remove("info");
        top.remove("tags");
    }
    contract(document, false)
}

/// `value` without its prose. `names` says the object's keys are names.
fn contract(value: Value, names: bool) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .filter(|(key, _)| names || !PROSE.contains(&key.as_str()))
                .map(|(key, value)| {
                    let inner = !names && NAME_MAPS.contains(&key.as_str());
                    (key, contract(value, inner))
                })
                .collect(),
        ),
        Value::Array(values) => {
            Value::Array(values.into_iter().map(|v| contract(v, false)).collect())
        }
        other => other,
    }
}

/// Every way `now` fails to be a growth of `then`, each by where it is.
fn changes(then: &Value, now: &Value) -> Vec<String> {
    let mut found = Vec::new();
    compare(then, now, "", false, &mut found);
    found
}

fn compare(then: &Value, now: &Value, at: &str, names: bool, found: &mut Vec<String>) {
    match (then, now) {
        (Value::Object(then), Value::Object(now)) => objects(then, now, at, names, found),
        (Value::Array(then), Value::Array(now)) => {
            if then.len() != now.len() {
                found.push(format!("{at}: {} entries became {}", then.len(), now.len()));
                return;
            }
            for (index, (then, now)) in then.iter().zip(now).enumerate() {
                compare(then, now, &format!("{at}/{index}"), false, found);
            }
        }
        _ if then == now => {}
        _ => found.push(format!("{at}: {then} became {now}")),
    }
}

fn objects(
    then: &Map<String, Value>,
    now: &Map<String, Value>,
    at: &str,
    names: bool,
    found: &mut Vec<String>,
) {
    for (key, was) in then {
        let here = format!("{at}/{key}");
        let Some(is) = now.get(key) else {
            found.push(format!("{here}: removed"));
            continue;
        };

        match key.as_str() {
            _ if names => compare(was, is, &here, false, found),
            "required" => same_set(was, is, &here, found),
            "enum" => grown_list(was, is, &here, found, |_, _, _, _| {}),
            "oneOf" => grown_list(was, is, &here, found, |then, now, at, found| {
                compare(then, now, at, false, found);
            }),
            "parameters" => parameters(was, is, &here, found),
            _ => compare(was, is, &here, NAME_MAPS.contains(&key.as_str()), found),
        }
    }

    for key in now.keys().filter(|key| !then.contains_key(*key)) {
        let new_name = names || METHODS.contains(&key.as_str());
        if !new_name {
            found.push(format!("{at}/{key}: added"));
        }
    }
}

/// `required` lists: a field made required, or no longer required, is a
/// change in place either way.
fn same_set(then: &Value, now: &Value, at: &str, found: &mut Vec<String>) {
    let set = |value: &Value| {
        let mut items: Vec<String> = value
            .as_array()
            .into_iter()
            .flatten()
            .map(Value::to_string)
            .collect();
        items.sort();
        items
    };
    if set(then) != set(now) {
        found.push(format!("{at}: {then} became {now}"));
    }
}

/// An enum's values or a tagged enum's variants: each kept, matched by
/// its name, and new ones only past a catch-all or on a skipped enum.
/// `entry` compares a kept variant with its match.
fn grown_list(
    then: &Value,
    now: &Value,
    at: &str,
    found: &mut Vec<String>,
    entry: fn(&Value, &Value, &str, &mut Vec<String>),
) {
    let (Some(then), Some(now)) = (then.as_array(), now.as_array()) else {
        found.push(format!("{at}: no longer a list"));
        return;
    };
    let named = |list: &[Value]| list.iter().map(name).collect::<Option<Vec<Value>>>();
    let (Some(then_names), Some(now_names)) = (named(then), named(now)) else {
        // Variants with no name to go by are matched by place.
        if then.len() != now.len() {
            found.push(format!(
                "{at}: {} variants became {}",
                then.len(),
                now.len()
            ));
            return;
        }
        for (index, (was, is)) in then.iter().zip(now).enumerate() {
            entry(was, is, &format!("{at}/{index}"), found);
        }
        return;
    };

    for (was, key) in then.iter().zip(&then_names) {
        let here = format!("{at}/{key}");
        match now_names.iter().position(|name| name == key) {
            Some(index) if was.is_object() => entry(was, &now[index], &here, found),
            Some(_) => {}
            None => found.push(format!("{here}: removed")),
        }
    }
    let grew = now_names.iter().any(|name| !then_names.contains(name));
    if grew && !then.iter().any(is_catch_all) && !skipped(at) {
        found.push(format!(
            "{at}: a variant was added to an enum with no catch-all for older readers"
        ));
    }
}

/// What a variant goes by: a plain value itself, a tagged variant its tag
/// (the first single-valued enum in it), an untagged one its schema.
fn name(variant: &Value) -> Option<Value> {
    fn tag(value: &Value) -> Option<Value> {
        match value {
            Value::Object(fields) => {
                if let Some(Value::Array(values)) = fields.get("enum")
                    && let [single] = values.as_slice()
                {
                    return Some(single.clone());
                }
                fields.values().find_map(tag)
            }
            Value::Array(values) => values.iter().find_map(tag),
            _ => None,
        }
    }

    if !variant.is_object() {
        return Some(variant.clone());
    }
    tag(variant).or_else(|| variant.get("$ref").cloned())
}

/// Whether the list at `at` is one of the [`SKIPPED`] enums'.
fn skipped(at: &str) -> bool {
    at.strip_prefix("/components/schemas/")
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|schema| SKIPPED.contains(&schema))
}

/// Whether a value or variant is an enum's catch-all.
fn is_catch_all(value: &Value) -> bool {
    let named = |value: &Value| {
        value
            .as_str()
            .is_some_and(|name| CATCH_ALLS.contains(&name))
    };
    let only_catch_all = |values: &Value| {
        values
            .as_array()
            .is_some_and(|values| values.len() == 1 && named(&values[0]))
    };

    named(value)
        || value.get("enum").is_some_and(only_catch_all)
        || value
            .get("properties")
            .and_then(Value::as_object)
            .is_some_and(|fields| {
                fields
                    .values()
                    .any(|field| field.get("enum").is_some_and(only_catch_all))
            })
}

/// An operation's parameters, by name and place: each kept as it was, and
/// a new one optional.
fn parameters(then: &Value, now: &Value, at: &str, found: &mut Vec<String>) {
    let key = |parameter: &Value| (parameter.get("name").cloned(), parameter.get("in").cloned());
    let (Some(then), Some(now)) = (then.as_array(), now.as_array()) else {
        found.push(format!("{at}: no longer a list"));
        return;
    };

    for was in then {
        let here = format!("{at}/{}", was.get("name").unwrap_or(&Value::Null));
        match now.iter().find(|is| key(is) == key(was)) {
            Some(is) => compare(was, is, &here, false, found),
            None => found.push(format!("{here}: removed")),
        }
    }
    for is in now
        .iter()
        .filter(|is| !then.iter().any(|was| key(was) == key(is)))
    {
        if is.get("required") == Some(&Value::Bool(true)) {
            let name = is.get("name").unwrap_or(&Value::Null);
            found.push(format!("{at}/{name}: a new parameter is required"));
        }
    }
}

#[test]
fn the_api_only_grows() {
    let path = fixture_path();
    let now = current();
    let bless = std::env::var_os("WELLFED_BLESS").is_some();

    let Ok(text) = std::fs::read_to_string(&path) else {
        // The first fixture is the beta's document, at revision 1.
        assert!(
            bless && REVISION == 1,
            "{} is missing: bless revision 1 with WELLFED_BLESS=1",
            path.display()
        );
        return store(&path, now);
    };
    let frozen: Fixture = serde_json::from_str(&text).expect("the fixture reads");

    let found = changes(&frozen.document, &now);
    assert!(
        found.is_empty(),
        "/api/v1 changed in place, which no client of an older build reads; \
         add instead of changing:\n{}",
        found.join("\n")
    );
    if frozen.document == now {
        assert_eq!(
            REVISION, frozen.revision,
            "wellfed_api::REVISION moved without the API growing"
        );
        return;
    }

    assert!(
        bless,
        "/api/v1 grew: raise wellfed_api::REVISION to {} and bless with WELLFED_BLESS=1",
        frozen.revision + 1
    );
    assert_eq!(
        REVISION,
        frozen.revision + 1,
        "a growth takes the next revision, {}",
        frozen.revision + 1
    );
    store(&path, now);
}

fn store(path: &PathBuf, document: Value) {
    let fixture = Fixture {
        revision: REVISION,
        document,
    };
    let mut text = serde_json::to_string_pretty(&fixture).expect("the fixture serializes");
    text.push('\n');
    std::fs::write(path, text).expect("the fixture is written");
}

/// A schema with a plain enum and a tagged one, as the document spells
/// them, for the checker's own tests.
fn sample() -> Value {
    json!({
        "components": { "schemas": {
            "Body": {
                "type": "object",
                "required": ["a"],
                "properties": {
                    "a": { "type": "integer" },
                    "title": { "type": ["string", "null"] }
                }
            },
            "Closed": { "type": "string", "enum": ["one", "two"] },
            "Open": { "type": "string", "enum": ["one", "unknown"] },
            "Tagged": { "oneOf": [
                { "type": "object", "properties": { "kind": { "type": "string", "enum": ["a"] } } },
                { "type": "object", "properties": { "kind": { "type": "string", "enum": ["unknown"] } } }
            ] },
            "CoreEvent": { "oneOf": [
                { "type": "object", "properties": { "kind": { "type": "string", "enum": ["boot"] } } }
            ] }
        } },
        "paths": { "/x": { "get": { "parameters": [
            { "name": "after", "in": "query", "required": false, "schema": { "type": "integer" } }
        ] } } }
    })
}

fn edited(edit: impl FnOnce(&mut Value)) -> Vec<String> {
    let then = contract(sample(), false);
    let mut now = sample();
    edit(&mut now);
    changes(&then, &contract(now, false))
}

#[test]
fn growth_reads_as_growth() {
    assert_eq!(edited(|_| {}), Vec::<String>::new());

    let grown = edited(|now| {
        let schemas = &mut now["components"]["schemas"];
        schemas["Body"]["properties"]["b"] = json!({ "type": ["string", "null"] });
        schemas["Body"]["description"] = json!("prose may change");
        // Rust keeps the catch-all last, so a new variant comes before it.
        schemas["Open"]["enum"] = json!(["one", "three", "unknown"]);
        schemas["Tagged"]["oneOf"].as_array_mut().unwrap().insert(
            1,
            json!({ "type": "object", "properties": { "kind": { "enum": ["c"] } } }),
        );
        schemas["CoreEvent"]["oneOf"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "type": "object", "properties": { "kind": { "enum": ["new"] } } }));
        schemas["New"] = json!({ "type": "string" });
        now["paths"]["/x"]["post"] = json!({});
        now["paths"]["/y"] = json!({ "get": {} });
        now["paths"]["/x"]["get"]["parameters"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "name": "limit", "in": "query", "required": false }));
    });
    assert_eq!(grown, Vec::<String>::new());
}

#[test]
fn a_change_in_place_is_named() {
    type Edit = fn(&mut Value);
    let cases: [(&str, Edit); 9] = [
        ("removed", |now| {
            now["components"]["schemas"]["Body"]["properties"]
                .as_object_mut()
                .unwrap()
                .remove("a");
        }),
        ("retyped", |now| {
            now["components"]["schemas"]["Body"]["properties"]["a"] = json!({ "type": "string" });
        }),
        ("made required", |now| {
            now["components"]["schemas"]["Body"]["required"] = json!(["a", "title"]);
        }),
        ("a closed enum grew", |now| {
            now["components"]["schemas"]["Closed"]["enum"] = json!(["one", "two", "three"]);
        }),
        ("a value renamed", |now| {
            now["components"]["schemas"]["Open"]["enum"] = json!(["uno", "unknown"]);
        }),
        ("a variant's field retyped", |now| {
            now["components"]["schemas"]["Tagged"]["oneOf"][0]["properties"]["n"] =
                json!({ "type": "integer" });
            now["components"]["schemas"]["Tagged"]["oneOf"][0]["required"] = json!(["n"]);
        }),
        ("narrowed", |now| {
            now["components"]["schemas"]["Body"]["properties"]["a"]["maximum"] = json!(9);
        }),
        ("a required parameter", |now| {
            now["paths"]["/x"]["get"]["parameters"]
                .as_array_mut()
                .unwrap()
                .push(json!({ "name": "must", "in": "query", "required": true }));
        }),
        ("an operation removed", |now| {
            now["paths"]["/x"].as_object_mut().unwrap().remove("get");
        }),
    ];

    for (case, edit) in cases {
        assert!(!edited(edit).is_empty(), "{case} passed as growth");
    }
}
