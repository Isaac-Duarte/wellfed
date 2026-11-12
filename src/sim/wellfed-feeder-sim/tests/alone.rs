//! Simulators of one test share the process's interlock, as the feeder's
//! tasks do: a boot that would drop another live simulator's held
//! interlock panics rather than pass a test that no feeder could.

use std::panic::catch_unwind;

use wellfed_device_sim::harness::Alone;
use wellfed_hal::interlock::{INTERLOCK, WriteStep, WriteTurn};

#[test]
fn a_boot_beside_a_live_simulator_that_holds_the_interlock_panics() {
    let _first = Alone::take();
    assert!(
        matches!(
            WriteTurn::new().begin(&INTERLOCK, 0, || true),
            WriteStep::Go(_)
        ),
        "an idle interlock gives a write its turn"
    );
    // Alone, a simulator's boot drops what its own last boot held.
    Alone::booting();

    let second = Alone::take();
    assert!(
        catch_unwind(Alone::booting).is_err(),
        "the boot would drop the other simulator's held write"
    );

    INTERLOCK.end_write();
    Alone::booting();
    drop(second);
}
