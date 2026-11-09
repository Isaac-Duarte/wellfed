//! The kept alerts' row (`feeder_alerts.state`) is format 1 from the beta.
//! It is made of the phone's `Watch` and the model's `AlertsView`, so a
//! change to either would change what every server has stored without a
//! word: this pins the JSON against a committed sample, both ways, and a
//! change fails here until it raises `KEPT_FORMAT` and is blessed
//! (`WELLFED_BLESS=1`).

use std::path::PathBuf;

use jiff::Timestamp;
use serde_json::{Value, json};
use wellfed_client::alerts::{Context, Cursor, FeederClock, Mutes};
use wellfed_mirror::{KEPT_FORMAT, Kept, ReadTable};
use wellfed_model::{AlertsView, Generation, Hex, OpenAlert};

fn sample_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kept-v1.json")
}

fn at(second: i64) -> Timestamp {
    Timestamp::from_second(1_800_000_000 + second).expect("a time")
}

fn alert(value: Value) -> OpenAlert {
    serde_json::from_value(value).expect("an alert")
}

fn table(generation: u32, settling: bool, alerts: Vec<OpenAlert>, read: Timestamp) -> ReadTable {
    ReadTable {
        cursor: Cursor {
            epoch: 7,
            boot: 3,
            generation: Generation(generation),
        },
        view: AlertsView {
            epoch: 7,
            generation: Generation(generation),
            more: false,
            settling,
            alerts,
            unread: 1,
        },
        clock_valid: true,
        at: read,
    }
}

/// Kept alerts with every part of the row in use: a table, open alerts
/// delivered, due, quiet and absent, a removal waiting, failures counted,
/// the server's own unreachable notice and the webhook's notes.
fn kept() -> Kept {
    let mutes = Mutes::default();
    let context = |now| Context {
        now,
        mutes: &mutes,
        own: Hex([1, 2, 3, 4]),
        clock: FeederClock::Known,
    };
    let failed = alert(json!({
        "at": "2027-01-15T08:00:00Z",
        "alert": {
            "kind": "meal_failed", "feed": 12, "plan": 2, "requested": 2,
            "completed": 0, "outcome": "failed", "reason": "jam"
        }
    }));
    let own = alert(json!({
        "at": "2027-01-15T07:00:00Z",
        "alert": { "kind": "client_joined", "client": "01020304", "role": "server", "path": "invite" }
    }));
    let food = alert(json!({ "at": null, "alert": { "kind": "food_low" } }));
    let clock = alert(json!({ "at": null, "alert": { "kind": "no_clock" } }));

    let mut kept = Kept::default();
    kept.watch.reached(Some("server".into()), at(0));
    kept.take(
        table(4, false, vec![own, food.clone(), clock], at(10)),
        &context(at(10)),
    );
    kept.take(table(5, true, vec![own, failed], at(20)), &context(at(20)));
    kept.watch.unanswered(Some("server"), &context(at(30)));
    kept.watch.relayed_reach(Some(at(25)), &context(at(40)));
    kept
}

#[test]
fn format_1_reads_and_writes_back_as_the_sample() {
    assert_eq!(KEPT_FORMAT, 1, "a new format gets a sample of its own");
    let written = serde_json::to_value(kept()).expect("the row encodes");
    let path = sample_path();
    if std::env::var_os("WELLFED_BLESS").is_some() {
        let text = serde_json::to_string_pretty(&written).expect("pretty");
        std::fs::write(&path, format!("{text}\n")).expect("bless the sample");
    }
    let sample: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("the sample is committed"))
            .expect("the sample is JSON");

    assert_eq!(written, sample, "what this build writes");
    let read: Kept = serde_json::from_value(sample.clone()).expect("the sample reads strictly");
    assert_eq!(
        serde_json::to_value(&read).expect("encodes"),
        sample,
        "what this build reads, written back"
    );
}
