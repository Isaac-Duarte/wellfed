//! The feeding plans between the loop and the writer.
//!
//! Two kinds of change share one slot, both queued from the loop task: a
//! client's new table, through the device API, and the schedule's own
//! claims and passes ([`PlansChange`]). The writer applies each to what
//! NOR holds (`wellfed_feeder::store::plans::Plans::changed`), saves it to
//! storage sectors 11 and 12 and publishes the result ([`publish`]), which
//! the loop reads to decide what runs. Nothing here writes flash or
//! reaches the feeding owner.

use wellfed_core::store::slot::SlotIo;
use wellfed_device_api::mailbox::{PlansChange, PlansResult, PlansView, Stamp};
use wellfed_feeder::store::plans::PlansStore;
use wellfed_hal::handoff::{Handoff, Published, Taken};

/// The plans' slots within the storage region.
pub const PLANS_BASE: u32 = wellfed_hal::board::storage::PLANS.first;

/// One queued change, and who asked for it (a client's table, skip or
/// undo), for the event the writer records as it saves it; `None` for the
/// schedule's own.
#[derive(Clone, Copy)]
pub struct Asked {
    pub change: PlansChange,
    pub asked: Option<Stamp>,
}

static PENDING: Handoff<Asked> = Handoff::new(Asked {
    change: PlansChange::Replace {
        table: [None; wellfed_device_api::mailbox::PLANS],
        at_s: None,
    },
    asked: None,
});

/// What NOR holds, published by the writer.
static PUBLISHED: Published<PlansView> = Published::new(EMPTY);

const EMPTY: PlansView = PlansView {
    table: [None; wellfed_device_api::mailbox::PLANS],
    marks: [0; wellfed_device_api::mailbox::PLANS],
    skips: 0,
    fed: 0,
    claim: None,
    revision: 0,
    replaced: 0,
    writable: false,
};

/// The plans as the writer last published them, and whether a change
/// waits; `Busy` until the writer has opened the store.
pub fn current() -> PlansResult {
    let Some(view) = PUBLISHED.get() else {
        return PlansResult::Busy;
    };
    PlansResult::Current {
        view,
        pending: PENDING.waiting(),
    }
}

/// Queue one change for the writer, with who asked for it (`None`: the
/// schedule's own).
pub fn change(change: PlansChange, asked: Option<Stamp>) -> PlansResult {
    let Some(view) = PUBLISHED.get() else {
        return PlansResult::Busy;
    };
    if !view.writable {
        return PlansResult::ReadOnly;
    }
    if !PENDING.offer(|pending| *pending = Asked { change, asked }) {
        return PlansResult::Busy;
    }
    wellfed_hal::writer_wake::wake();
    PlansResult::Current {
        view,
        pending: true,
    }
}

/// The writer's half: the queued change, held until the guard is finished.
pub fn take() -> Option<Taken<'static, Asked>> {
    PENDING.take()
}

/// The writer publishes what NOR holds; writable only once it is `armed`.
pub fn publish<I: SlotIo>(store: &PlansStore<I>, armed: bool) {
    PUBLISHED.publish(store.view(armed));
}

/// Forget everything (host tests).
pub fn reset() {
    PUBLISHED.clear(EMPTY);
    PENDING.clear();
}
