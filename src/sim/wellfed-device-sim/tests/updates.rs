//! The simulated runtime's give-way on a trial with no network to join,
//! read as the firmware's join half reads the credential store.

use wellfed_board::storage;
use wellfed_core::store::slot::SlotIo;
use wellfed_device::store::net::BuildWifi;
use wellfed_device::store::reset::MARKER;
use wellfed_device_sim::banks::Banks;
use wellfed_device_sim::flash::{Flash, Region};
use wellfed_device_sim::updates::Updates;
use wellfed_hal::ota::confirm::GiveWayReason;

/// A trial boot of a build bank 0 never activated, its first turn's
/// give-way.
fn first_turn(flash: &std::rc::Rc<Flash>) -> Option<GiveWayReason> {
    let banks = Banks::new();
    banks.borrow_mut().bank1_build = [0x77; 32];
    let (mut updates, booted) = Updates::open(flash, &banks, None, 0);
    assert!(booted.trial);
    updates.turn(0, BuildWifi::Absent).gave_way
}

/// An empty store gives way; one a pending factory reset withholds is
/// unread, not empty, and does not (the simulator's platform finishes a
/// reset as it opens, so only the runtime model meets it).
#[test]
fn only_a_store_read_as_empty_gives_way_for_no_network() {
    assert_eq!(first_turn(&Flash::memory()), Some(GiveWayReason::NoNetwork));

    let flash = Flash::memory();
    let marker = Region::new(&flash, storage::RESET);
    assert!(marker.erase(0));
    assert!(marker.program(0, 0, &MARKER));
    assert_eq!(first_turn(&flash), None);
}
