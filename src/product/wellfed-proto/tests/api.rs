//! The device API only grows. Every type a message reaches is traced
//! field by field and variant by variant, and `fixtures/api.json` keeps the
//! trace of the newest [`REVISION`]. Feeders, phones, the server and Home
//! Assistant run different builds, so the current trace must hold every
//! stored type unchanged: each struct with the same fields in the same
//! order, each enum with the same variants at the same indices, and only
//! new variants after them. A trace that grew needs the next revision and
//! a blessing (`WELLFED_BLESS=1`), which stores it; nothing blesses a
//! change in place ([device API](../../../../docs/firmware/device-api.md#evolution)).
//!
//! Names are pinned with the shapes: a renamed variant usually means a
//! changed meaning, which is a new variant.

use std::collections::BTreeMap;

use serde_reflection::{
    ContainerFormat, Format, Registry, Samples as Traced, Tracer, TracerConfig,
};
use wellfed_proto::api::*;
use wellfed_proto::device::Model;
use wellfed_proto::setup;

/// One type the trace starts from: its Rust name, and how to trace it.
struct Root {
    rust: &'static str,
    trace: fn(&mut Tracer, &Traced) -> Format,
}

/// The roots, each traced on its own.
macro_rules! roots {
    ($($ty:ty),+ $(,)?) => {
        vec![$(Root {
            rust: std::any::type_name::<$ty>(),
            trace: |tracer, samples| tracer.trace_type::<$ty>(samples).unwrap().0,
        }),+]
    };
}

/// Every type the device and its clients exchange: the top-level messages,
/// then every enum they reach. An event batch carries its events as bytes,
/// and each event its body as bytes, so [`EncodedEvent`] and [`EventBody`]
/// are roots; so are the clear and handshake payloads, and BLE
/// setup's. A nested enum is explored past its first variant only when
/// traced on its own, and the registry names any left incomplete, so a new
/// one fails here until it is added.
fn roots() -> Vec<Root> {
    roots!(
        ClientMessage<'_>,
        DeviceMessage<'_>,
        EncodedEvent<'_>,
        EncodedAlert<'_>,
        Open,
        Hello,
        Welcome<'_>,
        Join<'_>,
        Joined,
        setup::Request<'_>,
        setup::Answer<'_>,
        setup::Reply,
        Op<'_>,
        UpdateOp<'_>,
        FeederOp<'_>,
        Outcome<'_>,
        FeederOutcome<'_>,
        Refusal,
        UpdateRefusal,
        UpdatePhase,
        UpdateStop,
        UpdateCandidate,
        Bank0Check,
        OpenRefusal,
        Role,
        Model,
        Samples,
        EventBody,
        Metric,
        Path,
        Source,
        FeedOutcome,
        FeedReason,
        MissedReason,
        FoodLevel,
        DeviceStatus,
        Blocked,
        Battery,
        Desiccant,
        WifiSource,
        MealReminder,
        Window,
        NtpServer,
        NtpResult,
        TimeSource,
        Stats<'_>,
        StatsSection,
        ResetCause,
        Bank0Blocked,
        Bowl,
        CallClip,
        CurrentUnit,
        Phase,
        Tune,
        Alert,
        setup::Enrolled,
        setup::WifiSaved,
        setup::Refusal,
    )
}

/// The container name a root's trace answers with.
fn name(format: &Format) -> &str {
    match format {
        Format::TypeName(name) => name,
        other => panic!("a root is not a named container: {other:?}"),
    }
}

/// A tracer that knows the values a type can't be made up from: an alert
/// entry's time is a `NonZeroU32`, and the tracer's first guess at a `u32`
/// is 0. The sample's format joins the tracer's registry, as the struct is
/// traced from it.
fn tracer() -> (Tracer, Traced) {
    let config = TracerConfig::default().record_samples_for_structs(true);
    let mut tracer = Tracer::new(config);
    let mut samples = Traced::new();
    // The sample's bytes serialize as bytes, as they deserialize: a
    // borrowed slice's own `Serialize` is a sequence.
    struct Bytes(&'static [u8]);
    impl serde::Serialize for Bytes {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_bytes(self.0)
        }
    }
    #[derive(serde::Serialize)]
    #[serde(rename = "EncodedAlert")]
    struct Entry {
        at: Option<core::num::NonZeroU32>,
        alert: Bytes,
    }
    let entry = Entry {
        at: core::num::NonZeroU32::new(1),
        alert: Bytes(&[6]),
    };
    tracer.trace_value(&mut samples, &entry).unwrap();
    (tracer, samples)
}

fn registry() -> Registry {
    let (mut tracer, samples) = tracer();
    for root in roots() {
        (root.trace)(&mut tracer, &samples);
    }
    tracer
        .registry()
        .expect("every enum traced on its own: trace the ones named here too")
}

/// The stored trace: the revision it is, and every type by name.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Stored {
    revision: u32,
    registry: Registry,
}

const STORED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/api.json");

/// What in `current` breaks a reader of `stored`: each stored type must be
/// there by its name, a struct or other container unchanged, an enum with
/// every stored variant at its index unchanged. A type a field names is
/// checked under its own name, so the whole tree is covered.
fn shrunk(stored: &Registry, current: &Registry) -> Vec<String> {
    let mut broken = Vec::new();
    for (name, old) in stored {
        let Some(new) = current.get(name) else {
            broken.push(format!("{name} is gone"));
            continue;
        };
        match (old, new) {
            (ContainerFormat::Enum(old), ContainerFormat::Enum(new)) => {
                for (index, variant) in old {
                    match new.get(index) {
                        Some(now) if now == variant => {}
                        Some(now) => broken.push(format!(
                            "{name} variant {index} was {variant:?}, is {now:?}"
                        )),
                        None => broken.push(format!("{name} lost variant {index}")),
                    }
                }
            }
            (old, new) if old == new => {}
            (old, new) => broken.push(format!("{name} was {old:?}, is {new:?}")),
        }
    }
    broken
}

/// The enums an older reader survives a new variant of, though a message
/// reaches them plainly, each for its own reason. Every other enum a
/// message reaches is closed ([`closed`]).
const OPEN: &[(&str, &str)] = &[
    (
        "DeviceMessage",
        "a client skips a kind of message it doesn't know",
    ),
    ("Outcome", "a new answer only answers a new op"),
    ("FeederOutcome", "a new answer only answers a new op"),
    ("Stats", "a new section only answers a new StatsSection"),
    (
        "Refusal",
        "a refusal a client can't read is Refused(Unknown)",
    ),
    (
        "UpdatePhase",
        "frozen: grows at its end, read Unreadable by older",
    ),
    (
        "UpdateStop",
        "frozen: grows at its end, read Unreadable by older",
    ),
    (
        "UpdateCandidate",
        "frozen: grows at its end, read Unreadable by older",
    ),
    (
        "Bank0Check",
        "frozen: grows at its end, read Unreadable by older",
    ),
    (
        "Bank0Blocked",
        "frozen: grows at its end, read Unreadable by older",
    ),
];

/// The enums an older reader can't survive a new variant of, so they never
/// gain one: every enum an answer or a subscription reaches plainly, past
/// [`OPEN`]'s. An answer holding a variant its reader doesn't know reads
/// as `Unreadable` whole (a `Status` with a newer battery state would be
/// no status at all), a live sample ends the session it came in, and a
/// newer client's `Subscribe`, or a client message of a new kind, ends an
/// older feeder's. A lenient enum
/// (behind its `Lenient` newtype) and anything behind a length (an event
/// body, an alert entry, a trace page, a tail) is not reached. A request's
/// op is not walked either: a device refuses an op it can't decode
/// `Unavailable` and keeps the session. Such an enum grows by a new op, a
/// new kind of message, a struct's tail or a lenient field instead.
fn closed(registry: &Registry) -> BTreeMap<String, usize> {
    fn walk(registry: &Registry, format: &Format, found: &mut BTreeMap<String, usize>) {
        match format {
            Format::TypeName(name) => container(registry, name, found),
            Format::Option(inner) | Format::Seq(inner) => walk(registry, inner, found),
            Format::TupleArray { content, .. } => walk(registry, content, found),
            Format::Map { key, value } => {
                walk(registry, key, found);
                walk(registry, value, found);
            }
            Format::Tuple(formats) => {
                for format in formats {
                    walk(registry, format, found);
                }
            }
            _ => {}
        }
    }
    fn fields(
        registry: &Registry,
        fields: &[serde_reflection::Named<Format>],
        found: &mut BTreeMap<String, usize>,
    ) {
        for field in fields {
            walk(registry, &field.value, found);
        }
    }
    fn container(registry: &Registry, name: &str, found: &mut BTreeMap<String, usize>) {
        if name.starts_with("Lenient") || name == "Tail" {
            return;
        }
        match registry.get(name) {
            Some(ContainerFormat::Enum(variants)) => {
                let open = OPEN.iter().any(|(open, _)| *open == name);
                if !open && found.insert(name.to_owned(), variants.len()).is_some() {
                    return;
                }
                if open && name == "Refusal" {
                    return;
                }
                for variant in variants.values() {
                    variant_fields(registry, &variant.value, found);
                }
            }
            Some(ContainerFormat::Struct(named)) => fields(registry, named, found),
            Some(ContainerFormat::NewTypeStruct(inner)) => walk(registry, inner, found),
            Some(ContainerFormat::TupleStruct(formats)) => {
                for format in formats {
                    walk(registry, format, found);
                }
            }
            Some(ContainerFormat::UnitStruct) | None => {}
        }
    }
    fn variant_fields(
        registry: &Registry,
        variant: &serde_reflection::VariantFormat,
        found: &mut BTreeMap<String, usize>,
    ) {
        use serde_reflection::VariantFormat;
        match variant {
            VariantFormat::NewType(inner) => walk(registry, inner, found),
            VariantFormat::Tuple(formats) => {
                for format in formats {
                    walk(registry, format, found);
                }
            }
            VariantFormat::Struct(named) => fields(registry, named, found),
            VariantFormat::Unit | VariantFormat::Variable(_) => {}
        }
    }

    let mut found = BTreeMap::new();
    container(registry, "DeviceMessage", &mut found);
    let Some(ContainerFormat::Enum(client)) = registry.get("ClientMessage") else {
        panic!("ClientMessage is not traced as an enum");
    };
    // An older feeder reads a message of a kind it doesn't know as
    // malformed and ends the session, so the kinds themselves are closed.
    found.insert("ClientMessage".to_owned(), client.len());
    for variant in client.values().filter(|variant| variant.name != "Request") {
        variant_fields(registry, &variant.value, &mut found);
    }
    found
}

/// The kinds of message a client sends: `Request` and `Subscribe`. Closed,
/// so a new one is refused here rather than ending older feeders' sessions.
const CLIENT_MESSAGES: usize = 2;

/// What in `current` grew a closed enum of `stored`'s ([`closed`]).
fn opened(stored: &Registry, current: &Registry) -> Vec<String> {
    let mut grown = Vec::new();
    for (name, now) in closed(current) {
        if let Some(ContainerFormat::Enum(then)) = stored.get(&name)
            && then.len() != now
        {
            grown.push(format!(
                "{name} grew from {} to {now} variants, which an older reader can't read",
                then.len()
            ));
        }
    }
    grown
}

/// The device API only grows, and each growth is a new revision.
#[test]
fn the_api_only_grows() {
    let current = registry();
    let stored: Stored = serde_json::from_str(&std::fs::read_to_string(STORED).unwrap()).unwrap();

    let grown = opened(&stored.registry, &current);
    assert!(
        grown.is_empty(),
        "a closed enum grew: add a new op, a new kind of message, a field in a struct's tail \
         or a lenient field instead. {grown:#?}"
    );
    let broken = shrunk(&stored.registry, &current);
    assert!(
        broken.is_empty(),
        "the device API changed in place, which strands every client and feeder of an older \
         build: add a field to a struct's extension or a new variant at an enum's end instead. \
         {broken:#?}"
    );
    assert!(
        REVISION >= stored.revision,
        "REVISION went back from {} to {REVISION}",
        stored.revision
    );
    if current == stored.registry && REVISION == stored.revision {
        return;
    }
    assert!(
        REVISION == stored.revision + 1,
        "the device API grew: set wellfed_proto::api::REVISION to {}",
        stored.revision + 1
    );
    assert!(
        std::env::var_os("WELLFED_BLESS").is_some(),
        "revision {REVISION} grew the API: bless it (WELLFED_BLESS=1) to store its trace"
    );
    let grown = Stored {
        revision: REVISION,
        registry: current,
    };
    let mut json = serde_json::to_string_pretty(&grown).unwrap();
    json.push('\n');
    std::fs::write(STORED, json).unwrap();
}

/// The check refuses each way a type can change in place, and takes a
/// variant added at an enum's end.
#[test]
fn a_change_in_place_is_refused_and_a_new_variant_taken() {
    let stored = registry();
    let enum_of = |registry: &mut Registry, name: &str| match registry.get_mut(name) {
        Some(ContainerFormat::Enum(variants)) => variants.clone(),
        other => panic!("{name} is {other:?}"),
    };

    let mut grown = stored.clone();
    let mut variants = enum_of(&mut grown, "Tune");
    let next = u32::try_from(variants.len()).unwrap();
    variants.insert(
        next,
        serde_reflection::Named {
            name: "Fanfare".to_owned(),
            value: serde_reflection::VariantFormat::Unit,
        },
    );
    grown.insert("Tune".to_owned(), ContainerFormat::Enum(variants));
    assert_eq!(shrunk(&stored, &grown), Vec::<String>::new());

    let mut moved = stored.clone();
    let mut variants = enum_of(&mut moved, "Tune");
    let first = variants.remove(&0).unwrap();
    let second = variants.remove(&1).unwrap();
    variants.insert(0, second);
    variants.insert(1, first);
    moved.insert("Tune".to_owned(), ContainerFormat::Enum(variants));
    assert_eq!(shrunk(&stored, &moved).len(), 2);

    let mut widened = stored.clone();
    let Some(ContainerFormat::Struct(fields)) = widened.get_mut("Plan") else {
        panic!("Plan is not a struct");
    };
    fields.push(serde_reflection::Named {
        name: "grams".to_owned(),
        value: Format::U16,
    });
    assert_eq!(shrunk(&stored, &widened).len(), 1);

    let mut gone = stored.clone();
    gone.remove("Plan");
    assert_eq!(shrunk(&stored, &gone), ["Plan is gone"]);
}

/// The enums an answer or a subscription reaches plainly are closed, and
/// a new variant in one is refused; one only a request's op reaches takes
/// it, as does an open one.
#[test]
fn a_closed_enum_takes_no_new_variant() {
    let stored = registry();
    let closed = closed(&stored);
    for name in [
        "Metric",
        "Battery",
        "Desiccant",
        "Window",
        "MealReminder",
        "NtpServer",
        "CallClip",
        "Samples",
        "DeviceStatus",
        "ClientMessage",
    ] {
        assert!(closed.contains_key(name), "{name} is not closed");
    }
    assert_eq!(closed.get("ClientMessage"), Some(&CLIENT_MESSAGES));
    for name in [
        "Tune",
        "Op",
        "FeederOp",
        "Outcome",
        "Refusal",
        "UpdateRefusal",
    ] {
        assert!(!closed.contains_key(name), "{name} is closed");
    }
    for (name, _) in OPEN {
        assert!(stored.contains_key(*name), "{name} is open but not traced");
    }

    let fanfare = serde_reflection::Named {
        name: "Fanfare".to_owned(),
        value: serde_reflection::VariantFormat::Unit,
    };
    let grow = |name: &str| {
        let mut grown = stored.clone();
        let Some(ContainerFormat::Enum(variants)) = grown.get_mut(name) else {
            panic!("{name} is not an enum");
        };
        let next = u32::try_from(variants.len()).unwrap();
        variants.insert(next, fanfare.clone());
        grown
    };
    assert_eq!(opened(&stored, &grow("Battery")).len(), 1);
    assert_eq!(opened(&stored, &grow("Metric")).len(), 1);
    assert_eq!(opened(&stored, &grow("Samples")).len(), 1);
    assert_eq!(opened(&stored, &grow("Tune")), Vec::<String>::new());
    assert_eq!(opened(&stored, &grow("Outcome")), Vec::<String>::new());
}

/// The enums an answer carries leniently: an older reader reads a variant
/// a newer build added as unknown.
fn lenient() -> Vec<(&'static str, &'static [&'static str])> {
    fn of<T: Unit>() -> (&'static str, &'static [&'static str]) {
        (T::NAME, T::VARIANTS)
    }
    vec![
        of::<Blocked>(),
        of::<FoodLevel>(),
        of::<FeedOutcome>(),
        of::<FeedReason>(),
        of::<WifiSource>(),
        of::<ResetCause>(),
        of::<NtpResult>(),
        of::<TimeSource>(),
        of::<CurrentUnit>(),
        of::<Bowl>(),
        of::<Phase>(),
    ]
}

/// Each lenient enum's index table is the order serde encodes it in, and
/// it holds unit variants alone: a variant with fields could not be skipped
/// by an older reader, so such an enum only ever gains unit variants.
#[test]
fn every_lenient_enum_is_unit_only_in_its_encoded_order() {
    let registry = registry();
    for (name, variants) in lenient() {
        let Some(ContainerFormat::Enum(traced)) = registry.get(name) else {
            panic!("{name} is not traced as an enum");
        };
        let names: Vec<&str> = traced.values().map(|v| v.name.as_str()).collect();
        assert_eq!(names, variants, "{name}'s index table");
        for variant in traced.values() {
            assert_eq!(
                variant.value,
                serde_reflection::VariantFormat::Unit,
                "{name}::{} carries fields",
                variant.name
            );
        }
        assert!(
            registry.contains_key(&format!("Lenient{name}")),
            "no answer carries {name} leniently"
        );
    }

    // A `Lenient` is traced as its index alone, so the enum behind one
    // reaches the trace only as a root: one missing from both lists would
    // have its variants unpinned.
    let listed: Vec<&str> = lenient().iter().map(|(name, _)| *name).collect();
    for key in registry.keys() {
        let Some(name) = key.strip_prefix("Lenient") else {
            continue;
        };
        assert!(
            listed.contains(&name),
            "{key} carries {name}, which lenient() doesn't list"
        );
        assert!(
            matches!(registry.get(name), Some(ContainerFormat::Enum(_))),
            "{key} carries {name}, which is not traced: add it to roots()"
        );
    }
}

/// The trace reaches the types the hash is meant to cover, the event
/// bodies and the feeder's module among them.
#[test]
fn the_trace_reaches_every_module() {
    let registry = registry();
    for name in [
        "EventBody",
        "FeederOp",
        "FeederOutcome",
        "Outcome",
        "Status",
        "Stats",
        "UpdateView",
        "Refusal",
        "Reply",
    ] {
        assert!(registry.contains_key(name), "{name} is not traced");
    }
}

/// The registry keys types by name, and merges two enums of one name
/// variant by variant: two types sharing a name would hash as neither.
/// So each name belongs to one Rust type, every enum is a root, and each
/// root traced alone gives exactly the entry the whole trace holds.
#[test]
fn every_traced_name_is_one_type() {
    let registry = registry();
    let mut owners: BTreeMap<String, &str> = BTreeMap::new();
    for root in roots() {
        let (mut alone, samples) = tracer();
        let format = (root.trace)(&mut alone, &samples);
        let name = name(&format).to_owned();
        if let Some(other) = owners.insert(name.clone(), root.rust) {
            panic!(
                "{other} and {} are both traced as {name}: rename one for serde",
                root.rust
            );
        }
        let alone = alone.registry_unchecked();
        assert_eq!(
            alone.get(&name),
            registry.get(&name),
            "{name} ({}) traced alone differs from the whole trace: another type shares its name",
            root.rust
        );
    }
    for (name, format) in &registry {
        if matches!(format, ContainerFormat::Enum(_)) {
            assert!(
                owners.contains_key(name),
                "the enum {name} is not a root: add it to roots()"
            );
        }
    }
}

/// The device API's `Refusal` keeps its own variants, none of setup's.
#[test]
fn the_traced_refusal_is_the_device_api_s() {
    let registry = registry();
    let Some(ContainerFormat::Enum(variants)) = registry.get("Refusal") else {
        panic!("Refusal is not traced as an enum");
    };
    let names: Vec<&str> = variants
        .values()
        .map(|variant| variant.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "Forbidden",
            "Busy",
            "Unavailable",
            "Invalid",
            "NotWritable",
            "Full",
            "Update",
            "NotFound",
            "Stale",
            "Muted",
            "Time",
            "Fault",
            "Unverified",
            "Portions",
            "NoClock",
        ]
        .as_slice()
    );
    assert!(registry.contains_key("SetupRefusal"));
}
