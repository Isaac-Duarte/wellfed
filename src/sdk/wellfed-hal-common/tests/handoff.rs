//! The hand-off's states, each step exclusive: a claim is refused while a
//! value is written, waits or is taken; one guard at a time reads it; only
//! `finish` frees the slot, and a guard dropped unfinished leaves the value
//! waiting.

use wellfed_hal_common::handoff::{Handoff, Published};

#[test]
fn a_claim_is_refused_until_the_value_is_finished() {
    let slot = Handoff::new(0_u32);
    assert!(!slot.waiting());
    assert!(slot.take().is_none(), "nothing queued");

    // While it is written, another producer is refused and the consumer
    // finds nothing.
    assert!(slot.offer(|value| {
        *value = 7;
        assert!(!slot.offer(|_| unreachable!("a second claim while filling")));
        assert!(slot.take().is_none(), "not queued while filling");
    }));
    assert!(slot.waiting());
    assert!(!slot.offer(|_| unreachable!("a claim while queued")));

    let taken = slot.take().expect("queued");
    assert_eq!(*taken.get(), 7);
    assert!(slot.take().is_none(), "one guard at a time");
    assert!(!slot.offer(|_| unreachable!("a claim while taken")));
    taken.finish();

    assert!(!slot.waiting());
    assert!(slot.take().is_none(), "finished once, gone");
    assert!(slot.offer(|value| *value = 8), "free for the next");
    assert_eq!(slot.take().map(|taken| *taken.get()), Some(8));
}

#[test]
fn a_producer_that_declines_under_its_claim_leaves_the_slot_empty() {
    let slot = Handoff::new(0_u32);

    let declined = slot.try_offer(|value| {
        *value = 9;
        Err::<(), _>("refused")
    });
    assert_eq!(declined, Some(Err("refused")));
    assert!(!slot.waiting(), "nothing queued");
    assert!(slot.take().is_none());

    assert_eq!(
        slot.try_offer(|value| {
            *value = 3;
            Ok::<_, ()>(11)
        }),
        Some(Ok(11))
    );
    assert!(
        slot.try_offer(|_| -> Result<(), ()> { unreachable!("a claim while queued") })
            .is_none()
    );
    assert_eq!(slot.take().map(|taken| *taken.get()), Some(3));
}

#[test]
fn a_guard_dropped_unfinished_leaves_the_value_waiting() {
    let slot = Handoff::new([0_u8; 4]);
    assert!(slot.offer(|value| *value = [1, 2, 3, 4]));

    drop(slot.take().expect("queued"));
    assert!(slot.waiting());
    let again = slot.take().expect("put back");
    assert_eq!(*again.get(), [1, 2, 3, 4]);
    again.finish();
    assert!(!slot.waiting());

    // A clear leaves a held value held, so no producer writes under the
    // guard; once it drops back to queued, a clear empties the slot.
    assert!(slot.offer(|value| *value = [5; 4]));
    let held = slot.take().expect("queued");
    slot.clear();
    assert!(!slot.offer(|_| unreachable!("a claim while taken")));
    assert_eq!(*held.get(), [5; 4]);
    drop(held);
    slot.clear();
    assert!(!slot.waiting());
    assert!(slot.take().is_none());

    // Nor does a clear while the value is written free the claim.
    assert!(slot.offer(|value| {
        *value = [6; 4];
        slot.clear();
        assert!(!slot.offer(|_| unreachable!("a second claim while filling")));
    }));
    assert_eq!(slot.take().map(|taken| *taken.get()), Some([6; 4]));
}

#[test]
fn a_view_is_read_whole_and_none_before_the_first() {
    let view = Published::new((0_u32, 0_u32));
    assert_eq!(view.get(), None);

    view.publish((1, 10));
    assert_eq!(view.get(), Some((1, 10)));
    view.publish((2, 20));
    assert_eq!(view.get(), Some((2, 20)), "the generation with its value");

    view.clear((0, 0));
    assert_eq!(view.get(), None);
}
