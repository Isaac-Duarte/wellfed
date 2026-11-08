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

/// The device API only grows, and each growth is a new revision.
#[test]
fn the_api_only_grows() {
    let current = registry();
    let stored: Stored = serde_json::from_str(&std::fs::read_to_string(STORED).unwrap()).unwrap();

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
