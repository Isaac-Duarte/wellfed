//! `SCHEMA` follows the protocol's structure: every type a message reaches,
//! traced field by field and variant by variant. A variant inserted
//! mid-enum, a field added, removed, renamed or retyped anywhere, even in a
//! type no byte fixture exercises, changes the trace and so needs a new
//! revision. A client of another revision then speaks only the frozen part
//! to that build ([device API](../../../../docs/firmware/device-api.md#evolution)).
//!
//! The frozen types are traced too: they never change, so they never move
//! the hash.

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
/// so [`Event`] is a root; so are the clear and handshake payloads, and BLE
/// setup's. A nested enum is explored past its first variant only when
/// traced on its own, and the registry names any left incomplete, so a new
/// one fails here until it is added.
fn roots() -> Vec<Root> {
    roots!(
        ClientMessage<'_>,
        DeviceMessage<'_>,
        Event,
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
        Readiness,
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

fn registry() -> Registry {
    let mut tracer = Tracer::new(TracerConfig::default());
    let samples = Traced::new();
    for root in roots() {
        (root.trace)(&mut tracer, &samples);
    }
    tracer
        .registry()
        .expect("every enum traced on its own: trace the ones named here too")
}

/// FNV-1a over the registry's JSON: a `BTreeMap` by type name, so the
/// bytes do not depend on the order types were met.
fn hash(registry: &Registry) -> u32 {
    let json = serde_json::to_vec(registry).unwrap();
    json.iter().fold(0x811c_9dc5_u32, |hash, &byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

#[test]
fn the_schema_revision_follows_the_types() {
    let registry = registry();
    let hash = hash(&registry);
    assert_eq!(
        SCHEMA, hash,
        "the device API's types changed: set wellfed_proto::api::SCHEMA to {hash:#010x}, so \
         clients of the old revision speak only the frozen part to this build"
    );
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
    let samples = Traced::new();
    let mut owners: BTreeMap<String, &str> = BTreeMap::new();
    for root in roots() {
        let mut alone = Tracer::new(TracerConfig::default());
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
