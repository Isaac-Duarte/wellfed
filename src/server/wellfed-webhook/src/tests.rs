use wellfed_client::alerts::Notice;
use wellfed_model::{Alert, OpenAlert};

use super::*;

fn tracked(delivery: Shown) -> Tracked {
    Tracked {
        key: NoticeKey::Unreachable,
        notice: Notice::Alert {
            open: OpenAlert {
                at: None,
                alert: Alert::FoodLow,
            },
        },
        seen: Timestamp::UNIX_EPOCH,
        delivery,
        absent: None,
    }
}

#[test]
fn only_an_open_sent_is_kept_for_its_clear() {
    let mut hooked = Vec::new();
    hook(&mut hooked, &Change::Post(tracked(Shown::Due)), false);
    assert!(hooked.is_empty(), "nothing was sent with no webhook");

    hook(&mut hooked, &Change::Post(tracked(Shown::Due)), true);
    assert_eq!(hooked.len(), 1);
    hook(&mut hooked, &Change::Post(tracked(Shown::Due)), true);
    assert_eq!(hooked.len(), 1, "an alert posted again replaces itself");

    hook(&mut hooked, &Change::Remove(NoticeKey::Unreachable), true);
    assert!(hooked.is_empty());
}

#[test]
fn every_try_of_a_change_has_one_id_and_each_change_its_own() {
    let feeder = FeederId(uuid_from(1));
    let open = tracked(Shown::Due);
    let first = delivery_id(feeder, Transition::Opened, &open);
    assert_eq!(first, delivery_id(feeder, Transition::Opened, &open));
    assert_eq!(first.len(), 32);
    assert_ne!(first, delivery_id(feeder, Transition::Cleared, &open));
    let later = Tracked {
        seen: Timestamp::from_second(60).expect("a time"),
        ..open
    };
    assert_ne!(first, delivery_id(feeder, Transition::Opened, &later));
}

fn uuid_from(n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(n)
}
