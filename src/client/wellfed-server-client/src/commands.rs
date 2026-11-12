//! The command journal: the writes sent through the server, by key or as
//! pages, and what an entry says of its write's end ([`settled`]), the one
//! reading every server client shares.

use reqwest::{Method, StatusCode};
use uuid::Uuid;
use wellfed_api::{
    Command, CommandOutcome, CommandPage, CommandQuery, CommandState, FeedOutcome, FeedReason,
    FeederRefusal,
};

use crate::{Client, Error, decode, decode_page};

impl Client {
    /// The write sent under `key`: the caller's own, or anyone's for an
    /// owner. A write whose outcome was unknown is read here rather than
    /// sent again under a new key.
    pub async fn command(&self, key: Uuid) -> Result<Command, Error> {
        let response = self
            .api(Method::GET, &format!("/commands/{key}"))
            .send()
            .await?;
        decode(response, StatusCode::OK).await
    }

    /// A page of the journal, oldest first. The next page passes the
    /// page's `next` as `after`. An entry this client cannot read (a kind
    /// of write a newer server added) is left out.
    pub async fn commands(&self, query: &CommandQuery) -> Result<CommandPage, Error> {
        let response = self
            .api(Method::GET, "/commands")
            .query(query)
            .send()
            .await?;
        decode_page(response, StatusCode::OK, "items").await
    }
}

/// What a journal entry says of its write's end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settled {
    /// It took effect.
    Done,
    /// A meal call write another change to the call overtook: final, and
    /// whether this one took effect no longer shows.
    Superseded,
    /// The feeder refused it; nothing ran.
    Refused(FeederRefusal),
    /// The feeder reserved the feed under this number and runs it; its
    /// `Fed` follows.
    Started { feed: u32 },
    /// The feed ended, as its `Fed` event says.
    Fed {
        feed: u32,
        requested: u8,
        completed: u8,
        outcome: FeedOutcome,
        reason: FeedReason,
    },
    /// The feeder restarted with no record of the feed: whether food was
    /// dispensed is not known. A `Fed` the server finds later replaces it.
    Unrecorded,
}

/// What the journal entry in `state` with `outcome` settles of its write;
/// `None` while it is in flight or unknown, or for an outcome a newer
/// server added. A write left so is uncertain, never sent again.
pub fn settled(state: CommandState, outcome: Option<&CommandOutcome>) -> Option<Settled> {
    match state {
        CommandState::Sent | CommandState::Unknown => return None,
        CommandState::Answered | CommandState::Resolved => {}
    }
    let settled = match outcome? {
        CommandOutcome::Done
        | CommandOutcome::Settings { .. }
        | CommandOutcome::Plans { .. }
        | CommandOutcome::Calibration { .. }
        | CommandOutcome::Call { .. }
        | CommandOutcome::Invited { .. } => Settled::Done,
        CommandOutcome::Superseded => Settled::Superseded,
        CommandOutcome::Refused { refusal } => Settled::Refused(*refusal),
        CommandOutcome::Feeding { feed } => Settled::Started { feed: *feed },
        CommandOutcome::Fed {
            feed,
            requested,
            completed,
            outcome,
            reason,
        } => Settled::Fed {
            feed: *feed,
            requested: *requested,
            completed: *completed,
            outcome: *outcome,
            reason: *reason,
        },
        CommandOutcome::Unrecorded => Settled::Unrecorded,
        CommandOutcome::Unknown => return None,
    };
    Some(settled)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FED: CommandOutcome = CommandOutcome::Fed {
        feed: 7,
        requested: 2,
        completed: 1,
        outcome: FeedOutcome::Failed,
        reason: FeedReason::Jammed,
    };

    /// Every state against a settling outcome: only an answered or resolved
    /// entry settles its write.
    #[test]
    fn only_an_answered_or_resolved_entry_settles() {
        let cases = [
            (CommandState::Sent, None),
            (CommandState::Unknown, None),
            (CommandState::Answered, Some(Settled::Done)),
            (CommandState::Resolved, Some(Settled::Done)),
        ];
        for (state, expected) in cases {
            assert_eq!(
                settled(state, Some(&CommandOutcome::Done)),
                expected,
                "{state:?}"
            );
        }
        assert_eq!(settled(CommandState::Answered, None), None);
    }

    /// Every outcome a server sends, and one a newer server added.
    #[test]
    fn each_outcome_reads_as_its_end() {
        let refusal = FeederRefusal::Busy;
        let cases = [
            (CommandOutcome::Done, Some(Settled::Done)),
            (CommandOutcome::Superseded, Some(Settled::Superseded)),
            (
                CommandOutcome::Refused { refusal },
                Some(Settled::Refused(refusal)),
            ),
            (
                CommandOutcome::Invited {
                    invite: 3,
                    seconds: 60,
                },
                Some(Settled::Done),
            ),
            (
                CommandOutcome::Feeding { feed: 7 },
                Some(Settled::Started { feed: 7 }),
            ),
            (
                FED,
                Some(Settled::Fed {
                    feed: 7,
                    requested: 2,
                    completed: 1,
                    outcome: FeedOutcome::Failed,
                    reason: FeedReason::Jammed,
                }),
            ),
            (CommandOutcome::Unrecorded, Some(Settled::Unrecorded)),
            (CommandOutcome::Unknown, None),
        ];
        for (outcome, expected) in cases {
            assert_eq!(
                settled(CommandState::Resolved, Some(&outcome)),
                expected,
                "{outcome:?}"
            );
        }
    }
}
