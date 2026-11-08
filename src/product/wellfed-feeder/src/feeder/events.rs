//! The event recorder: one durable event queued for the settings writer,
//! which alone appends the ring, stamped with the wall clock when it is
//! valid. The device API's events, the panel's lock changes, the boot and a
//! feed's end all come through here, so the alert rules watch here: they see
//! an event whether or not the queue has room for it.

use wellfed_proto::api::EventBody;

use super::platform::{EventSlot, Platform};
use wellfed_device::store::events::BODY_BYTES;

/// Queue `body` at `now_ms`. A full queue counts it lost, and the loop
/// records the count as an event of its own.
pub fn record(platform: &mut impl Platform, now_ms: u32, body: EventBody) {
    queue(platform, now_ms, body, EventSlot::Shared);
}

/// As [`record`] for a feed's `Fed` event, which may take the queue's
/// reserved slot: appends wait while a feed holds the outputs, so the
/// events of a long feed pile up, and the feed's own result must not be the
/// one lost. `ran_s` stamps it with when the feed ran instead of now, for
/// a feed a restart interrupted.
pub fn record_outcome(
    platform: &mut impl Platform,
    now_ms: u32,
    ran_s: Option<u32>,
    body: EventBody,
) {
    let at = ran_s.or_else(|| wall_s(platform, now_ms));
    push(platform, now_ms, at, body, EventSlot::Outcome);
}

/// As [`record`] for an event that must not be lost to a full queue: false if
/// there was no room, and the caller asks again. The rules hear of it only
/// once it has a place.
#[inline(never)]
pub fn record_or_wait(platform: &mut impl Platform, now_ms: u32, body: EventBody) -> bool {
    let at = wall_s(platform, now_ms);
    let mut bytes = [0u8; BODY_BYTES];
    if wellfed_proto::encode(&body, &mut bytes).is_err()
        || !platform.queue_event(at, bytes, EventSlot::Shared)
    {
        return false;
    }

    if platform.alerts_on() {
        platform.alerts_mut().event(now_ms, at, &body);
    }
    true
}

fn queue(platform: &mut impl Platform, now_ms: u32, body: EventBody, slot: EventSlot) {
    let at = wall_s(platform, now_ms);
    push(platform, now_ms, at, body, slot);
}

/// Tell the alert rules of each enrollment and revocation the settings
/// writer appended itself, in the turn that saved the change, as [`record`]
/// tells them of the loop's own; how many the writer had no room to hand
/// over. The writer records its other events (settings, Wi-Fi, plans) to
/// the ring alone, since no rule acts on them. Out of line: the decoded
/// body must not sit in the pass's frame.
#[inline(never)]
pub fn written(platform: &mut impl Platform, now_ms: u32) -> u32 {
    while let Some((at, bytes)) = platform.take_written() {
        if platform.alerts_on()
            && let Some(body) = wellfed_proto::decode_padded::<EventBody>(&bytes)
        {
            platform.alerts_mut().event(now_ms, at, &body);
        }
    }
    platform.take_written_lost()
}

/// The wall clock's second at `now_ms`, which an event carries; `None`
/// while the clock is not valid.
pub fn wall_s(platform: &mut impl Platform, now_ms: u32) -> Option<u32> {
    platform
        .wall_ms(now_ms)
        .and_then(|ms| u32::try_from(ms / 1000).ok())
}

// Out of line: a caller's frame must not carry the encoding buffer, and two
// of them are on the loop's deepest paths.
#[inline(never)]
fn push(
    platform: &mut impl Platform,
    now_ms: u32,
    at: Option<u32>,
    body: EventBody,
    slot: EventSlot,
) {
    if platform.alerts_on() {
        platform.alerts_mut().event(now_ms, at, &body);
    }

    let mut bytes = [0u8; BODY_BYTES];
    // Every body fits the ring's record; one that did not would be a lost
    // event.
    if wellfed_proto::encode(&body, &mut bytes).is_ok() {
        let _ = platform.queue_event(at, bytes, slot);
    }
}
