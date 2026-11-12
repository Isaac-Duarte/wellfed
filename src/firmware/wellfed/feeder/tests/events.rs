//! A feed's outcome keeps the event queue's reserved slot on this board:
//! once the shared slots are full a plain record is lost and counted, and
//! the feed's `Fed` event still queues.

use wellfed_device_api::Device;
use wellfed_device_runtime::events::{self, QUEUE, RESERVED};
use wellfed_feeder::feeder::{Feeder, NoFeeds};
use wellfed_fw_feeder::platform::Plaf103;
use wellfed_proto::api::{EventBody, Source};
use wellfed_sdk::hal::sdk::fake;

const LOCK: EventBody = EventBody::Lock {
    locked: true,
    by: Source::Panel,
};

#[test]
fn the_reserved_slot_takes_only_a_feed_s_outcome() {
    fake::reset_hardware();
    events::reset();
    let mut feeder = Box::new(Feeder::new(Plaf103::new()));
    let mut feeds = NoFeeds;
    // The boot's own event takes the first shared slot.
    {
        let mut device = feeder.device(0, &mut feeds);
        for _ in 1..QUEUE - RESERVED {
            device.record(0, LOCK);
        }
    }
    assert_eq!(events::take_lost(), 0, "the shared slots hold every record");

    feeder.device(0, &mut feeds).record(0, LOCK);
    feeder.record_outcome(0, LOCK);
    feeder.record_outcome(0, LOCK);
    // The count is handed over once its `Lost` has a shared slot, so it is
    // read after two records leave (the first outcome holds the reserved
    // one): the plain record past the shared slots and the second outcome
    // were lost.
    events::finish_pending(None);
    assert_eq!(events::take_lost(), 0, "no shared slot for the Lost yet");
    events::finish_pending(None);
    assert_eq!(events::take_lost(), 2);
}
