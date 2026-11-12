//! The portion calibration between the loop and the writer: what one
//! portion weighs.
//!
//! The device API queues a whole new calibration ([`change`]); the writer
//! saves it to NOR (storage sectors 34 and 35) and publishes what the
//! store holds ([`publish`]), which the loop reads for the portion
//! statistics. Nothing here writes flash, and nothing here reaches the
//! feeding owner: a weight changes what a portion is reported to weigh,
//! never how far the motor turns.

use wellfed_core::store::slot::SlotIo;
use wellfed_device_api::mailbox::{Calibration, CalibrationResult, CalibrationView};
use wellfed_feeder::store::calibration::CalibrationStore;
use wellfed_hal::handoff::{Handoff, Published, Taken};

/// The calibration's slots within the storage region.
pub const CALIBRATION_BASE: u32 = wellfed_hal::board::storage::CALIBRATION.first;

/// The queued change, from the device API to the writer.
static PENDING: Handoff<Calibration> = Handoff::new(Calibration { portion_mg: None });

/// What NOR holds, published by the writer as one view: the weight, its
/// generation and whether it may change travel together, so a reader never
/// pairs one generation with another's weight.
static PUBLISHED: Published<CalibrationView> = Published::new(EMPTY);

const EMPTY: CalibrationView = CalibrationView {
    calibration: Calibration { portion_mg: None },
    generation: 0,
    writable: false,
};

/// The calibration as the writer last published it, and whether a change
/// waits; `Busy` until the writer has opened the store.
pub fn current() -> CalibrationResult {
    let Some(view) = PUBLISHED.get() else {
        return CalibrationResult::Busy;
    };
    CalibrationResult::Current {
        view,
        pending: PENDING.waiting(),
    }
}

/// The device API's half: queue the whole new calibration for the writer.
pub fn change(calibration: Calibration) -> CalibrationResult {
    let Some(view) = PUBLISHED.get() else {
        return CalibrationResult::Busy;
    };
    if !view.writable {
        return CalibrationResult::ReadOnly;
    }
    if !PENDING.offer(|pending| *pending = calibration) {
        return CalibrationResult::Busy;
    }
    wellfed_hal::writer_wake::wake();
    CalibrationResult::Current {
        view,
        pending: true,
    }
}

/// The writer's half: the queued change, held until the guard is finished.
pub fn take() -> Option<Taken<'static, Calibration>> {
    PENDING.take()
}

/// The writer publishes what NOR holds; writable only once it is `armed`.
pub fn publish<I: SlotIo>(store: &CalibrationStore<I>, armed: bool) {
    PUBLISHED.publish(CalibrationView {
        calibration: store.calibration,
        // Reported as 32 bits; saturate rather than wrap to an old value.
        generation: u32::try_from(store.generation).unwrap_or(u32::MAX),
        writable: armed && store.writable(),
    });
}

/// Forget everything (host tests).
pub fn reset() {
    PUBLISHED.clear(EMPTY);
    PENDING.clear();
}
