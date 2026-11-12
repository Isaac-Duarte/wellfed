//! Which `Fed` event settles a feed whose answer never came, on a client's
//! own session with the feeder. The app's router and Home Assistant's bridge
//! both ask here, so the two cannot disagree about which feed ran.
//!
//! A `Fed` names the client that asked, never the request, so a client
//! with a feed whose answer was lost can only infer which of its own feeds
//! an event ends. The rule leans to leaving a feed uncertain: a feed is
//! never given another's end, and nothing is settled while two readings of
//! the history are possible. A lost feed's `Fed`:
//!
//! - names this client, asked for the same portions, and comes after the
//!   newest event the session knew when the feed went out;
//! - is not a feed this client already has the end of: one it was answered
//!   for, or saw start, or settled before;
//! - is the only such event in the history read;
//! - and no other lost feed of this client's could take it too. A feed
//!   still waiting on its answer counts as lost here, since its answer may
//!   yet be lost.
//!
//! A feed a restart interrupted is recorded by the feeder under its own
//! name, so it never settles a client's lost feed: a panel feed a restart
//! interrupted reads the same, and taking it would give a lost feed an end
//! that may be another's.
//!
//! The server settles the feeds it sent from its own journal and history,
//! by its own rule; this is the rule for feeds a client sent directly.

use wellfed_model::Source;

use crate::history::Feed;

/// Where a lost feed's `Fed` can be in the history: after the newest event
/// the session knew when the feed went out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum After {
    /// After this event.
    Event(u64),
    /// The history held no event when it went out: any `Fed` is after it.
    Start,
    /// Not known. Such a feed is never settled, since any `Fed` could be
    /// before it, but it still keeps another lost feed from taking a `Fed`
    /// that could be its own.
    Unknown,
}

impl After {
    /// Whether an event numbered `seq` can be this feed's.
    const fn admits(self, seq: u64) -> bool {
        match self {
            Self::Event(after) => seq > after,
            Self::Start | Self::Unknown => true,
        }
    }
}

/// A feed of this client's, sent on its own session, whose end is not yet
/// known: its answer was lost, or has not come yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lost<'a> {
    pub portions: u8,
    pub after: After,
    /// Feed numbers this client has the end of (answered for, seen start,
    /// or settled): none of them is this feed's.
    pub excluded: &'a [u32],
}

impl Lost<'_> {
    /// Whether `fed`, an end this client asked for, could be this feed's.
    fn could_be(&self, fed: &Feed) -> bool {
        fed.requested == self.portions
            && self.after.admits(fed.seq)
            && !self.excluded.contains(&fed.feed)
    }
}

/// For each of `lost`, in order, the one `Fed` in `history` that settles it,
/// or `None` while none does or more than one reading is possible.
/// `client` is the first four bytes of this client's key, as events name
/// it; `history` may hold anyone's feeds, in any order. `lost` holds every
/// feed of this client's on this feeder whose end is not known, the ones
/// still waiting on their answer too: the caller settles only those it
/// counts as lost, but each one keeps the others from taking its `Fed`.
/// Two feeds are never given the same `Fed`.
pub fn settled<'h>(
    lost: &[Lost<'_>],
    history: &'h [Feed],
    client: [u8; 4],
) -> Vec<Option<&'h Feed>> {
    let ours: Vec<&Feed> = history
        .iter()
        .filter(|fed| matches!(fed.by, Some(Source::Client { client: by }) if by.0 == client))
        .collect();

    lost.iter()
        .enumerate()
        .map(|(index, feed)| {
            if feed.after == After::Unknown {
                return None;
            }
            let mut candidates = ours.iter().copied().filter(|fed| feed.could_be(fed));
            let found = candidates.next()?;
            if candidates.next().is_some() {
                return None;
            }

            let claimed = lost
                .iter()
                .enumerate()
                .any(|(other, lost)| other != index && lost.could_be(found));
            (!claimed).then_some(found)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use wellfed_model::{FeedOutcome, FeedReason, FoodLevel, Hex};

    use super::*;
    use crate::history::Delivered;

    const US: [u8; 4] = [1, 2, 3, 4];
    const OTHER: [u8; 4] = [9, 9, 9, 9];

    fn fed(seq: u64, feed: u32, requested: u8, by: Option<Source>) -> Feed {
        Feed {
            epoch: 1,
            seq,
            feed,
            at: None,
            by,
            requested,
            delivered: Delivered::Counted(requested),
            outcome: FeedOutcome::Done,
            reason: FeedReason::Done,
            hopper: FoodLevel::Present,
        }
    }

    fn ours(seq: u64, feed: u32, requested: u8) -> Feed {
        fed(seq, feed, requested, Some(client(US)))
    }

    fn client(key: [u8; 4]) -> Source {
        Source::Client { client: Hex(key) }
    }

    fn lost(portions: u8, after: After) -> Lost<'static> {
        Lost {
            portions,
            after,
            excluded: &[],
        }
    }

    /// The feed number each lost feed is settled with.
    fn numbers(lost: &[Lost<'_>], history: &[Feed]) -> Vec<Option<u32>> {
        settled(lost, history, US)
            .into_iter()
            .map(|fed| fed.map(|fed| fed.feed))
            .collect()
    }

    #[test]
    fn a_lost_feed_is_settled_by_the_one_matching_fed_after_it() {
        let history = [ours(5, 1, 2), ours(11, 2, 2)];

        assert_eq!(numbers(&[lost(2, After::Event(10))], &history), [Some(2)]);
    }

    #[test]
    fn a_fed_before_the_feed_went_out_settles_nothing() {
        let history = [ours(10, 1, 2)];

        assert_eq!(numbers(&[lost(2, After::Event(10))], &history), [None]);
    }

    #[test]
    fn a_feed_sent_on_an_empty_history_takes_any_matching_fed() {
        let history = [ours(1, 1, 2)];

        assert_eq!(numbers(&[lost(2, After::Start)], &history), [Some(1)]);
    }

    #[test]
    fn a_feed_with_no_mark_is_never_settled() {
        let history = [ours(11, 2, 2)];

        assert_eq!(numbers(&[lost(2, After::Unknown)], &history), [None]);
    }

    #[test]
    fn another_portion_count_settles_nothing() {
        let history = [ours(11, 2, 3)];

        assert_eq!(numbers(&[lost(2, After::Event(10))], &history), [None]);
    }

    #[test]
    fn another_client_s_or_the_feeder_s_fed_settles_nothing() {
        let interrupted = Feed {
            outcome: FeedOutcome::Uncertain,
            delivered: Delivered::Uncertain {
                confirmed: 0,
                requested: 2,
            },
            ..fed(12, 3, 2, Some(Source::Feeder))
        };
        let history = [
            fed(11, 2, 2, Some(client(OTHER))),
            interrupted,
            fed(13, 4, 2, Some(Source::Panel)),
            fed(14, 5, 2, None),
        ];

        assert_eq!(numbers(&[lost(2, After::Event(10))], &history), [None]);
    }

    #[test]
    fn a_feed_this_client_has_the_end_of_settles_nothing() {
        let history = [ours(11, 2, 2)];
        let answered = Lost {
            excluded: &[2],
            ..lost(2, After::Event(10))
        };

        assert_eq!(numbers(&[answered], &history), [None]);
    }

    #[test]
    fn two_matching_feds_settle_neither_reading() {
        let history = [ours(11, 2, 2), ours(14, 3, 2)];

        assert_eq!(numbers(&[lost(2, After::Event(10))], &history), [None]);
    }

    /// Two lost feeds of 2 and 1 portions; only the second ran. Its `Fed`
    /// settles the feed of its portions, never the older one.
    #[test]
    fn of_two_lost_feeds_the_fed_settles_the_one_of_its_portions() {
        let history = [ours(15, 7, 1)];
        let lost = [lost(2, After::Event(10)), lost(1, After::Event(12))];

        assert_eq!(numbers(&lost, &history), [None, Some(7)]);
    }

    #[test]
    fn a_fed_two_lost_feeds_could_claim_settles_neither() {
        let history = [ours(15, 7, 1)];
        let lost = [lost(1, After::Event(10)), lost(1, After::Event(12))];

        assert_eq!(numbers(&lost, &history), [None, None]);
    }

    /// Two of this client's `Fed`s could be the older lost feed's: neither
    /// is settled, though only one reading may be the true one.
    #[test]
    fn a_lost_feed_two_feds_could_end_settles_nothing_by_elimination() {
        let history = [ours(11, 6, 1), ours(15, 7, 1)];
        let lost = [lost(1, After::Event(10)), lost(1, After::Event(12))];

        assert_eq!(numbers(&lost, &history), [None, None]);
    }

    /// A feed still waiting on its answer, or with no mark, keeps a lost one
    /// from taking a `Fed` that may be its own.
    #[test]
    fn a_feed_awaiting_its_answer_keeps_its_fed_from_a_lost_one() {
        let history = [ours(15, 7, 1)];
        let waiting = [lost(1, After::Event(10)), lost(1, After::Unknown)];

        assert_eq!(numbers(&waiting, &history)[0], None);
    }

    /// A feed whose `Fed` a restart replays was settled before, so its
    /// number is excluded: the replay settles no later lost feed, which
    /// went out after it anyway.
    #[test]
    fn a_replayed_fed_of_a_settled_feed_settles_no_later_one() {
        let history = [ours(11, 2, 1), ours(20, 3, 2)];
        let later = Lost {
            excluded: &[2],
            ..lost(1, After::Event(8))
        };

        assert_eq!(numbers(&[later], &history), [None]);
    }

    /// A `Fed` that comes long after the feed went out (late in the
    /// history, after other feeds) still settles it.
    #[test]
    fn a_late_fed_still_settles_the_feed() {
        let history = [
            fed(11, 2, 1, Some(Source::Panel)),
            ours(12, 3, 4),
            ours(40, 9, 1),
        ];
        let waiting = Lost {
            excluded: &[3],
            ..lost(1, After::Event(10))
        };

        assert_eq!(numbers(&[waiting], &history), [Some(9)]);
    }
}
