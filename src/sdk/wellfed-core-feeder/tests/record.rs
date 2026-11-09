//! The host tools' view of the feeding owner's UART records (`PLOWN1`),
//! beside the SDK's.

use wellfed_core::record::{Event, Record};
use wellfed_core_feeder::feed::owner::{OwnerFault, OwnerState};
use wellfed_core_feeder::record::{Owner, Tag};

fn event(line: &str) -> Option<Event<'_, Owner>> {
    Record::parse(line)
        .unwrap_or_else(|| panic!("no record in {line:?}"))
        .event_with::<Owner>()
}

#[test]
fn owner_records_decode() {
    assert_eq!(
        event("PLOWN1 state=4 fault=0 supervisor=0 stopped=0 optics_off=0"),
        Some(Event::Kind(Owner::State {
            state: OwnerState::Feeding,
            stopped: false,
        }))
    );
    assert_eq!(
        event("PLOWN1 feed portions=2 held=1"),
        Some(Event::Kind(Owner::Held))
    );
    assert_eq!(
        event("PLOWN1 feed id=7 portions=2 submit=0"),
        Some(Event::Kind(Owner::Submitted {
            id: 7,
            accepted: true
        }))
    );
    assert_eq!(
        event("PLOWN1 feed id=7 portions=2 submit=4"),
        Some(Event::Kind(Owner::Submitted {
            id: 7,
            accepted: false
        }))
    );
}

/// A record of the SDK's still decodes beside the kind's, and the SDK's
/// own reading leaves the owner's records alone.
#[test]
fn the_owner_reads_beside_the_sdk() {
    assert!(matches!(
        event("PLFW1 start build=ab bank=1"),
        Some(Event::Start { bank: Some(1), .. })
    ));
    let owner = Record::parse("PLOWN1 feed portions=2 held=1").unwrap();
    assert_eq!(owner.event(), None);
    assert_eq!(event("PLWIF3 ev seq=3 kind=dns"), None);
}

/// The tags as the firmware prints them; its lines name the tag as a
/// literal (`c"PLOWN1 stall begin=1"`), so a renamed tag fails here.
#[test]
fn tags_are_unchanged() {
    let words: Vec<_> = Tag::ALL.iter().map(|tag| tag.as_str()).collect();
    assert_eq!(words, ["PLOWN1"]);
}

/// The owner's numbers, as `state=` and `fault=` print them, decode back.
#[test]
fn owner_codes_round_trip() {
    for code in 0..9 {
        let state = OwnerState::from_code(code).unwrap();
        assert_eq!(state.code(), code);
    }
    assert_eq!(OwnerState::from_code(9), None);
    for code in 0..8 {
        let fault = OwnerFault::from_code(code).unwrap();
        assert_eq!(fault.code(), code);
    }
    assert_eq!(OwnerFault::from_code(8), None);
}
