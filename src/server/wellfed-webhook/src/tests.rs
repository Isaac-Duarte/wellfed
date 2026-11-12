use wellfed_client::alerts::{Delivery as Shown, Notice, NoticeKey, Tracked};
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
fn only_an_open_the_receiver_took_is_kept_for_its_clear() {
    let mut hooked = Vec::new();
    hook(&mut hooked, &Change::Post(tracked(Shown::Due)), Told::Not);
    assert!(hooked.is_empty(), "refused, given up or never sent");

    hook(&mut hooked, &Change::Post(tracked(Shown::Due)), Told::Took);
    assert_eq!(hooked.len(), 1);
    hook(&mut hooked, &Change::Post(tracked(Shown::Due)), Told::Took);
    assert_eq!(hooked.len(), 1, "an alert posted again replaces itself");
    hook(&mut hooked, &Change::Post(tracked(Shown::Due)), Told::Not);
    assert_eq!(
        hooked.len(),
        1,
        "its new kind was not taken: the opening taken before still gets its clear"
    );

    hook(
        &mut hooked,
        &Change::Remove(NoticeKey::Unreachable),
        Told::Took,
    );
    assert!(hooked.is_empty());
}

#[test]
fn every_try_of_a_change_has_one_id_and_each_change_its_own() {
    let feeder = FeederId(uuid::Uuid::from_u128(1));
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

mod delivery;

/// A failed send names its cause, never the URL: a refused connection
/// reads as one, not only as reqwest's "error sending request".
#[test]
fn a_failed_send_names_its_cause_without_the_url() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    runtime.block_on(async {
        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = closed.local_addr().unwrap();
        drop(closed);

        let url = format!("http://{address}/api/webhook/secret");
        let error = reqwest::Client::new().post(url).send().await.unwrap_err();
        let words = without_url(error);
        assert!(words.to_lowercase().contains("refused"), "{words}");
        assert!(!words.contains("secret"), "{words}");
    });
}
