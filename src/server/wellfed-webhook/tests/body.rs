//! The webhook's body is frozen from the beta: receivers (a Home Assistant
//! automation among them) are written against it. A newer server only adds
//! fields, so this pins every field there is today, as JSON spells it.

use jiff::Timestamp;
use serde_json::json;
use uuid::Uuid;
use wellfed_api::{AlertLevel, FeederId};
use wellfed_client::alerts::NoticeKind;
use wellfed_webhook::{AlertRef, Delivery, FeederRef, Transition, VERSION};

#[test]
fn the_body_reads_as_version_1() {
    let at: Timestamp = "2026-11-09T08:00:00Z".parse().expect("a time");
    let delivery = Delivery {
        version: VERSION,
        id: "0123456789abcdef0123456789abcdef".into(),
        event: Transition::Opened,
        time: "2026-11-09T08:00:05Z".parse().expect("a time"),
        feeder: FeederRef {
            id: FeederId(Uuid::from_u128(0x0192_0000_0000_7000_8000_0000_0000_0001)),
            name: "Kitchen".into(),
        },
        alert: AlertRef {
            key: "7:feed:12".into(),
            kind: NoticeKind::MealFailed,
            level: AlertLevel::Urgent,
            words: "A meal failed: 0 of 2 portions dispensed.".into(),
            at: Some(at),
        },
    };

    assert_eq!(
        serde_json::to_value(&delivery).expect("the body serializes"),
        json!({
            "version": 1,
            "id": "0123456789abcdef0123456789abcdef",
            "event": "opened",
            "time": "2026-11-09T08:00:05Z",
            "feeder": { "id": "01920000-0000-7000-8000-000000000001", "name": "Kitchen" },
            "alert": {
                "key": "7:feed:12",
                "kind": "meal_failed",
                "level": "urgent",
                "words": "A meal failed: 0 of 2 portions dispensed.",
                "at": "2026-11-09T08:00:00Z"
            }
        })
    );

    let cleared = serde_json::to_value(Transition::Cleared).expect("serializes");
    assert_eq!(cleared, "cleared");
    let unreachable = serde_json::to_value(NoticeKind::Unreachable).expect("serializes");
    assert_eq!(unreachable, "unreachable");
}
