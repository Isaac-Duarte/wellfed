//! The shared offer and confirmation rules, arm by arm: an image is offered
//! only when it is another build, newer than the feeder's floor, to a feeder
//! not ahead of this client, with bank 0 not blocking; and an activated
//! update ends confirmed, fallen back, on the wrong build or still pending,
//! by what the session after its restart reads.

use wellfed_client::offer::{Candidate, Confirmation, Gate, Offer, Restarted, confirmation, offer};
use wellfed_model::{Bank0Blocked, Bank0Check, Skew};

const RUNNING: Option<&str> = Some("aa11");

/// An update state read with floor `confirmed` and bank 0's `bank0`.
fn read(confirmed: u32, bank0: Bank0Check) -> Option<Gate> {
    Some(Gate { confirmed, bank0 })
}
const IMAGE: Candidate<'static> = Candidate {
    build: "BB22",
    version: 5,
};

#[test]
fn an_image_is_offered_only_when_newer_another_build_and_bank0_allows() {
    let skews = [Skew::Same, Skew::FeederBehind, Skew::Legacy, Skew::Unknown];
    for skew in skews {
        for bank0 in [Bank0Check::Unchecked, Bank0Check::Match] {
            assert_eq!(
                offer(IMAGE, RUNNING, skew, read(4, bank0)),
                Offer::Newer,
                "{skew:?}"
            );
        }
    }

    let same = Candidate {
        build: "AA11",
        ..IMAGE
    };
    assert_eq!(
        offer(same, RUNNING, Skew::Same, read(0, Bank0Check::Match)),
        Offer::Current,
        "any case"
    );
    // The running build comes first: it is current even at the floor.
    assert_eq!(
        offer(same, RUNNING, Skew::FeederAhead, read(9, Bank0Check::Match)),
        Offer::Current
    );

    for floor in [5, 6] {
        assert_eq!(
            offer(IMAGE, RUNNING, Skew::Same, read(floor, Bank0Check::Match)),
            Offer::Older
        );
    }
    assert_eq!(
        offer(
            IMAGE,
            RUNNING,
            Skew::FeederAhead,
            read(0, Bank0Check::Match)
        ),
        Offer::Older,
        "the client is the side to update"
    );
    // Older wins over a blocked bank 0: nothing would be offered anyway.
    let blocked = Bank0Check::Blocked(Bank0Blocked::Unrecorded);
    assert_eq!(
        offer(IMAGE, RUNNING, Skew::Same, read(5, blocked)),
        Offer::Older
    );

    assert_eq!(
        offer(IMAGE, RUNNING, Skew::Same, read(4, blocked)),
        Offer::Blocked(Bank0Blocked::Unrecorded)
    );
    assert_eq!(
        offer(IMAGE, RUNNING, Skew::Same, read(4, Bank0Check::Unknown)),
        Offer::Blocked(Bank0Blocked::Unknown),
        "a state this client cannot name blocks"
    );
}

/// Before the update state is read, or when its read fails, what runs and
/// the skew still say a feeder is current or ahead; anything else waits for
/// the state, and nothing is offered meanwhile.
#[test]
fn an_unread_update_state_decides_only_what_runs_and_the_skew_say() {
    let same = Candidate {
        build: "AA11",
        ..IMAGE
    };
    assert_eq!(offer(same, RUNNING, Skew::Same, None), Offer::Current);
    assert_eq!(offer(IMAGE, RUNNING, Skew::FeederAhead, None), Offer::Older);

    let skews = [Skew::Same, Skew::FeederBehind, Skew::Legacy, Skew::Unknown];
    for skew in skews {
        for running in [RUNNING, None] {
            assert_eq!(offer(IMAGE, running, skew, None), Offer::Unread, "{skew:?}");
        }
    }
}

#[test]
fn an_activated_update_ends_by_what_the_restarted_session_reads() {
    let read = |build, bank, confirmed, ready| Restarted {
        build,
        bank,
        confirmed,
        ready,
    };
    let served = "bb22";

    assert_eq!(
        confirmation(served, 5, read("BB22", 1, 5, false)),
        Confirmation::Confirmed
    );
    assert_eq!(
        confirmation(served, 5, read(served, 1, 6, false)),
        Confirmation::Confirmed
    );
    assert_eq!(
        confirmation(served, 5, read(served, 1, 4, true)),
        Confirmation::Confirmed,
        "a reinstall the feeder confirmed before, ready"
    );
    assert_eq!(
        confirmation(served, 5, read(served, 1, 4, false)),
        Confirmation::Pending
    );

    for build in [served, "aa11"] {
        for ready in [false, true] {
            assert_eq!(
                confirmation(served, 5, read(build, 0, 5, ready)),
                Confirmation::FellBack,
                "bank 0 answers"
            );
        }
    }
    for confirmed in [4, 5] {
        assert_eq!(
            confirmation(served, 5, read("aa11", 1, confirmed, true)),
            Confirmation::WrongBuild
        );
    }
}
