//! The enums inside the app's kept files, pinned per file and version.
//!
//! An enum value an older build can't read makes the whole file unreadable
//! to it, so a new variant in one of these is a new version of the file
//! ([`crate::stored`]), not growth within one. Each enum's variants are read
//! from serde itself: deserializing a name no variant has fails with
//! every name it takes, and an enum that reads unknown names (lenient) takes
//! it. The fixture lists them under each file's version; this test fails
//! when one changes while the version stays the same.

use std::collections::BTreeSet;

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use wellfed_client::alerts::{Absence, Delivery, Notice, NoticeKey, NoticeKind};
use wellfed_model::{
    Alert, AlertKey, AlertKind, AlertLevel, Blocked, ClientRole, EnrollPath, FeedOutcome,
    FeedReason, MissedReason, Source, UpdateStop,
};

use crate::alerts::AlertState;
use crate::api::router::{RoutePath, WriteKind};
use crate::api::server_views::ServerRole;
use crate::api::store::{AppFile, Appearance, StoreKind};
use crate::route::pins::{PinFile, PinState, PinnedWrite};
use crate::stored::Versioned;

const PINNED: &str = include_str!("../tests/fixtures/kept-enums.txt");
const PROBE: &str = "__not_a_variant__";

/// The tags an internally tagged enum in these files is keyed by.
const TAGS: [&str; 5] = ["write", "notice", "absence", "kind", "key"];

/// `T`'s variant names as serde reads them, or `lenient` when it reads a
/// name it doesn't have.
fn variants<T: DeserializeOwned>() -> String {
    let probes = std::iter::once(Value::String(PROBE.to_owned()))
        .chain(TAGS.iter().map(|tag| json!({ *tag: PROBE })));
    for probe in probes {
        let error = match serde_json::from_value::<T>(probe) {
            Ok(_) => return "lenient".to_owned(),
            Err(error) => error.to_string(),
        };
        let Some((_, expected)) = error.split_once(&format!("unknown variant `{PROBE}`")) else {
            continue;
        };
        let names: BTreeSet<&str> = expected.split('`').skip(1).step_by(2).collect();
        return names.into_iter().collect::<Vec<_>>().join(" ");
    }
    panic!("no probe reads {}'s variants", std::any::type_name::<T>());
}

fn file<T: Versioned>(name: &str, enums: &[(&str, String)]) -> Vec<String> {
    enums
        .iter()
        .map(|(enumeration, names)| format!("{name}@{} {enumeration} {names}", T::VERSION))
        .collect()
}

fn now() -> Vec<String> {
    let store_kind = ("StoreKind", variants::<StoreKind>());
    let mut lines = file::<AppFile>(
        "wellfed.json",
        &[
            ("Appearance", variants::<Appearance>()),
            ("ServerRole", variants::<ServerRole>()),
            store_kind.clone(),
        ],
    );
    lines.extend(file::<PinFile>(
        "pending.json",
        &[
            ("RoutePath", variants::<RoutePath>()),
            ("WriteKind", variants::<WriteKind>()),
            ("PinState", variants::<PinState>()),
            ("PinnedWrite", variants::<PinnedWrite>()),
            store_kind.clone(),
        ],
    ));
    lines.extend(file::<AlertState>(
        "alerts.json",
        &[
            store_kind,
            ("NoticeKind", variants::<NoticeKind>()),
            ("NoticeKey", variants::<NoticeKey>()),
            ("Notice", variants::<Notice>()),
            ("Delivery", variants::<Delivery>()),
            ("Absence", variants::<Absence>()),
            ("Alert", variants::<Alert>()),
            ("AlertKey", variants::<AlertKey>()),
            ("AlertKind", variants::<AlertKind>()),
            ("AlertLevel", variants::<AlertLevel>()),
            ("FeedOutcome", variants::<FeedOutcome>()),
            ("FeedReason", variants::<FeedReason>()),
            ("Blocked", variants::<Blocked>()),
            ("MissedReason", variants::<MissedReason>()),
            ("UpdateStop", variants::<UpdateStop>()),
            ("ClientRole", variants::<ClientRole>()),
            ("EnrollPath", variants::<EnrollPath>()),
            ("Source", variants::<Source>()),
        ],
    ));
    lines
}

#[test]
fn every_kept_enum_is_pinned_at_its_files_version() {
    let now = now();
    let pinned: Vec<&str> = PINNED
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .filter(|line| {
            let head = line.split(' ').next().unwrap_or_default();
            now.iter().any(|line| line.split(' ').next() == Some(head))
        })
        .collect();

    assert_eq!(
        pinned,
        now,
        "an enum in a kept file changed within its version: bump that file's version, keep a \
         reader for the old one, and append these lines to tests/fixtures/kept-enums.txt:\n{}",
        now.join("\n")
    );
}
