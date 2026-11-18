//! A feed's end as the history records it, from the feeding owner's result.
//!
//! A feed runs whatever the food beam reads, as stock's does, so a beam
//! that misreads never skips a meal; the event records what it read
//! (`hopper`), low food included.
//!
//! The owner's result is durable once it reports it; one that is not (the
//! owner faulted before its journal write) may have dispensed anything up
//! to what was asked, so it is recorded as uncertain and never retried. A
//! feed a reset interrupted is resolved at the next boot, durably, as
//! uncertain too, with a reason of its own ([`interrupted`]).
//!
//! The journal keeps no reason for a resolved feed, so the reason is the
//! wire's alone: the boot that resolves the feed names it. It does keep who
//! asked, in this module's encoding ([`requester`]), so the resolved feed
//! names the requester its event would have named had it finished.

use wellfed_core_feeder::feed::controller::FeedReason as CoreReason;
use wellfed_core_feeder::feed::storage::{OwnerOutcome, OwnerResult, Requester};
use wellfed_proto::api::{EventBody, FeedOutcome, FeedReason, Fingerprint, FoodLevel, Source};

/// The kind of requester the journal keeps, in its first byte. Zero is
/// nobody known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum Kind {
    Panel = 1,
    Client = 2,
    Feeder = 3,
    Schedule = 4,
}

impl Kind {
    const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::Panel),
            2 => Some(Self::Client),
            3 => Some(Self::Feeder),
            4 => Some(Self::Schedule),
            _ => None,
        }
    }
}

/// `by` as the journal keeps it: its kind, then a client's key fingerprint
/// or a plan's slot, the rest zero.
pub fn requester(by: Source) -> Requester {
    let mut bytes = [0; Requester::BYTES];
    let kind = match by {
        Source::Panel => Kind::Panel,
        Source::Client(client) => {
            bytes[1..].copy_from_slice(&client);
            Kind::Client
        }
        Source::Feeder => Kind::Feeder,
        Source::Schedule(plan) => {
            bytes[1] = plan;
            Kind::Schedule
        }
    };
    bytes[0] = kind as u8;
    Requester(bytes)
}

/// Who a journaled `requester` names; `None` for nobody known (a
/// reservation whose record a cut lost) or bytes this encoding never
/// writes.
pub fn source(requester: Requester) -> Option<Source> {
    let [kind, rest @ ..] = requester.0;
    let client: Fingerprint = rest;
    match Kind::from_byte(kind)? {
        Kind::Panel if client == [0; 4] => Some(Source::Panel),
        Kind::Client => Some(Source::Client(client)),
        Kind::Feeder if client == [0; 4] => Some(Source::Feeder),
        Kind::Schedule if rest[1..] == [0; 3] => Some(Source::Schedule(rest[0])),
        Kind::Panel | Kind::Feeder | Kind::Schedule => None,
    }
}

/// Whether the journal holds the result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Durability {
    Durable,
    Lost,
}

/// What the food beam read during a feed, as the owner's trace keeps it
/// (`Some(false)` once it read empty).
pub const fn food_level(reading: Option<bool>) -> FoodLevel {
    match reading {
        None => FoodLevel::Unread,
        Some(true) => FoodLevel::Present,
        Some(false) => FoodLevel::Low,
    }
}

/// The `Fed` event for the feed `feed`, which `result` ended, with the food
/// beam reading `hopper` during it.
pub fn fed(
    feed: u32,
    result: &OwnerResult,
    hopper: Option<bool>,
    durability: Durability,
    by: Source,
) -> EventBody {
    let outcome = match (durability, result.outcome) {
        (Durability::Lost, _) | (_, OwnerOutcome::Pending | OwnerOutcome::Uncertain) => {
            FeedOutcome::Uncertain
        }
        (Durability::Durable, OwnerOutcome::Done) => FeedOutcome::Done,
        (Durability::Durable, OwnerOutcome::Cancelled) => FeedOutcome::Cancelled,
        (Durability::Durable, OwnerOutcome::Expired) => FeedOutcome::Expired,
        (Durability::Durable, OwnerOutcome::Failed) => FeedOutcome::Failed,
    };
    EventBody::Fed {
        feed,
        requested: portions(result.requested),
        completed: portions(result.completed),
        outcome: outcome.into(),
        reason: reason(result.reason).into(),
        hopper: food_level(hopper).into(),
        by,
    }
}

/// A count of portions as the event carries it; the owner caps a feed far
/// below 255.
fn portions(count: u16) -> u8 {
    u8::try_from(count).unwrap_or(u8::MAX)
}

/// The `Fed` event for the feed `feed` a reset interrupted, which the
/// owner resolved at this boot as `result`: uncertain, for the reason
/// `Interrupted`, by the requester its reservation kept. A reservation
/// that names nobody is the feeder's, which clients show as unknown and a
/// plan's claim may still attribute (`Schedule::claim_behind`).
pub fn interrupted(feed: u32, result: &OwnerResult) -> EventBody {
    let by = source(result.requester).unwrap_or(Source::Feeder);
    EventBody::Fed {
        feed,
        requested: portions(result.requested),
        completed: portions(result.completed),
        outcome: FeedOutcome::Uncertain.into(),
        reason: FeedReason::Interrupted.into(),
        // The boot that resolves it never saw the feed's reading.
        hopper: FoodLevel::Unread.into(),
        by,
    }
}

const fn reason(reason: CoreReason) -> FeedReason {
    match reason {
        CoreReason::Done => FeedReason::Done,
        CoreReason::Cancelled => FeedReason::Cancelled,
        CoreReason::Clock => FeedReason::Clock,
        CoreReason::ObservationGap => FeedReason::ObservationGap,
        CoreReason::TotalTimeout => FeedReason::Timeout,
        CoreReason::FeedbackTimeout => FeedReason::NoFeedback,
        CoreReason::Power => FeedReason::Power,
        CoreReason::Sensor => FeedReason::Sensor,
        CoreReason::Outlet => FeedReason::OutletBlocked,
        CoreReason::Jammed => FeedReason::Jammed,
        CoreReason::MarkStuck => FeedReason::MarkStuck,
        CoreReason::Overcurrent => FeedReason::Overcurrent,
        CoreReason::FeedbackStream => FeedReason::FeedbackStream,
        // No reason: a result still pending, which ends as an output
        // failure if it ends at all. A feed resolved at boot has none
        // either, but takes `interrupted` instead.
        CoreReason::None | CoreReason::Output => FeedReason::Output,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(outcome: OwnerOutcome, reason: CoreReason) -> OwnerResult {
        OwnerResult {
            id: 7,
            requested: 2,
            completed: 1,
            outcome,
            reason,
            motion_possible: true,
            requester: Requester::NONE,
        }
    }

    #[test]
    fn a_durable_result_keeps_its_outcome_and_a_lost_one_is_uncertain() {
        let failed = result(OwnerOutcome::Failed, CoreReason::FeedbackTimeout);
        assert_eq!(
            fed(7, &failed, Some(false), Durability::Durable, Source::Panel),
            EventBody::Fed {
                feed: 7,
                requested: 2,
                completed: 1,
                outcome: FeedOutcome::Failed.into(),
                reason: FeedReason::NoFeedback.into(),
                hopper: FoodLevel::Low.into(),
                by: Source::Panel,
            }
        );

        let done = result(OwnerOutcome::Done, CoreReason::Done);
        let EventBody::Fed { outcome, .. } =
            fed(7, &done, Some(true), Durability::Lost, Source::Feeder)
        else {
            panic!("not a feed");
        };
        assert_eq!(outcome, FeedOutcome::Uncertain.into());

        let pending = result(OwnerOutcome::Pending, CoreReason::None);
        let EventBody::Fed {
            outcome, reason, ..
        } = fed(7, &pending, None, Durability::Durable, Source::Feeder)
        else {
            panic!("not a feed");
        };
        assert_eq!(
            (outcome, reason),
            (FeedOutcome::Uncertain.into(), FeedReason::Output.into())
        );
    }

    /// A feed runs on low food, as stock's does, and its event says the
    /// hopper read low; one reading of food and none low is food.
    #[test]
    fn the_event_records_what_the_food_beam_read() {
        let done = result(OwnerOutcome::Done, CoreReason::Done);
        for (reading, level) in [
            (Some(false), FoodLevel::Low),
            (Some(true), FoodLevel::Present),
            (None, FoodLevel::Unread),
        ] {
            let EventBody::Fed {
                outcome, hopper, ..
            } = fed(7, &done, reading, Durability::Durable, Source::Panel)
            else {
                panic!("not a feed");
            };
            assert_eq!((outcome, hopper), (FeedOutcome::Done.into(), level.into()));
        }
    }

    #[test]
    fn a_feed_resolved_at_boot_is_uncertain_interrupted_and_counts_no_portion() {
        let resolved = OwnerResult {
            completed: 0,
            motion_possible: true,
            ..result(OwnerOutcome::Uncertain, CoreReason::None)
        };
        assert_eq!(
            interrupted(9, &resolved),
            EventBody::Fed {
                feed: 9,
                requested: 2,
                completed: 0,
                outcome: FeedOutcome::Uncertain.into(),
                reason: FeedReason::Interrupted.into(),
                hopper: FoodLevel::Unread.into(),
                by: Source::Feeder,
            }
        );
    }

    /// Every requester a feed can have reads back as itself, so a feed a
    /// reset interrupted names who asked; nobody known reads as the
    /// feeder, as before the journal kept it.
    #[test]
    fn a_resolved_feed_names_the_requester_its_reservation_kept() {
        for by in [
            Source::Panel,
            Source::Client([0xab, 0xcd, 0x01, 0x02]),
            Source::Client([0; 4]),
            Source::Feeder,
            Source::Schedule(0),
            Source::Schedule(7),
        ] {
            assert_eq!(source(requester(by)), Some(by), "{by:?}");
            let resolved = OwnerResult {
                completed: 0,
                requester: requester(by),
                ..result(OwnerOutcome::Uncertain, CoreReason::None)
            };
            let EventBody::Fed { by: named, .. } = interrupted(9, &resolved) else {
                panic!("not a feed");
            };
            assert_eq!(named, by);
        }

        let nobody = result(OwnerOutcome::Uncertain, CoreReason::None);
        let EventBody::Fed { by, .. } = interrupted(9, &nobody) else {
            panic!("not a feed");
        };
        assert_eq!(by, Source::Feeder);
    }

    /// Bytes this encoding never writes name nobody.
    #[test]
    fn a_requester_this_build_never_writes_names_nobody() {
        for bytes in [
            [0, 0, 0, 0, 0],
            [5, 0, 0, 0, 0],
            [0xff, 0xff, 0xff, 0xff, 0xff],
            [1, 0, 0, 0, 1],
            [3, 1, 0, 0, 0],
            [4, 2, 0, 1, 0],
        ] {
            assert_eq!(source(Requester(bytes)), None, "{bytes:02x?}");
        }
    }
}
