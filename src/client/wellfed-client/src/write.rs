//! How a write ended, by its answer: the one reading every write path
//! shares. Only the answer a write is expected to bring says it ran, and
//! only a refusal says it did not; no answer, one this build cannot read (a
//! newer build's), or one of another kind may follow a write that ran, so
//! each is uncertain and the write is never sent again.

use wellfed_model::{Answer, Refusal};

/// How one write ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteEnd<T> {
    /// The feeder answered with what the write brings: it ran.
    Done(T),
    /// The feeder refused it: it did not run.
    Refused(Refusal),
    /// It may have run: settle it by reading, never by sending it again.
    Uncertain(Uncertain),
}

/// Why a write's end is not known.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Uncertain {
    #[error("no answer came")]
    Unanswered,
    #[error("the answer is a newer build's, which this build cannot read")]
    Unreadable,
    #[error("the answer is of another kind than the write brings")]
    OtherKind,
}

/// The end of a write answered `answer` (`None`: no answer came). `done`
/// takes the answer the write brings when it ran, and passes over any
/// other kind.
pub fn write_end<'a, T>(
    answer: Option<&'a Answer>,
    done: impl FnOnce(&'a Answer) -> Option<T>,
) -> WriteEnd<T> {
    let Some(answer) = answer else {
        return WriteEnd::Uncertain(Uncertain::Unanswered);
    };

    match answer {
        Answer::Refused(refusal) => WriteEnd::Refused(*refusal),
        Answer::Unreadable => WriteEnd::Uncertain(Uncertain::Unreadable),
        Answer::Done { .. }
        | Answer::Status(_)
        | Answer::Settings(_)
        | Answer::Clients(_)
        | Answer::Invite(_)
        | Answer::Feeding { .. }
        | Answer::Update(_)
        | Answer::Time(_)
        | Answer::Stats(_)
        | Answer::FeedTrace(_)
        | Answer::Calibration(_)
        | Answer::Bank1(_)
        | Answer::Plans(_)
        | Answer::Call(_)
        | Answer::Alerts(_) => {
            done(answer).map_or(WriteEnd::Uncertain(Uncertain::OtherKind), WriteEnd::Done)
        }
    }
}
