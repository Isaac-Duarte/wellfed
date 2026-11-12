//! A write's end by its answer: what every write path reads.

use wellfed_client::write::{Uncertain, WriteEnd, write_end};
use wellfed_model::{Answer, Refusal};

/// What a feed brings when it ran.
fn feeding(answer: &Answer) -> Option<u32> {
    match answer {
        Answer::Feeding { feed } => Some(*feed),
        _ => None,
    }
}

/// What a store write brings when it ran.
fn done(answer: &Answer) -> Option<()> {
    matches!(answer, Answer::Done { .. }).then_some(())
}

/// Only the answer a write brings says it ran and only a refusal says it
/// did not: no answer, an unreadable one and one of another kind are each
/// uncertain, whatever the write.
#[test]
fn only_the_expected_answer_or_a_refusal_settles_a_write() {
    let fed = Answer::Feeding { feed: 7 };
    let stored = Answer::Done { generation: None };
    let busy = Answer::Refused(Refusal::Busy);

    assert_eq!(write_end(Some(&fed), feeding), WriteEnd::Done(7));
    assert_eq!(write_end(Some(&stored), done), WriteEnd::Done(()));
    assert_eq!(
        write_end(Some(&busy), feeding),
        WriteEnd::Refused(Refusal::Busy)
    );
    assert_eq!(
        write_end(Some(&busy), done),
        WriteEnd::Refused(Refusal::Busy)
    );

    for (answer, why) in [
        (None, Uncertain::Unanswered),
        (Some(&Answer::Unreadable), Uncertain::Unreadable),
        (Some(&stored), Uncertain::OtherKind),
        (Some(&Answer::Clients(Vec::new())), Uncertain::OtherKind),
    ] {
        assert_eq!(
            write_end(answer, feeding),
            WriteEnd::Uncertain(why),
            "{answer:?}"
        );
    }
    assert_eq!(
        write_end(Some(&fed), done),
        WriteEnd::Uncertain(Uncertain::OtherKind)
    );
    assert_eq!(
        write_end(Some(&Answer::Unreadable), done),
        WriteEnd::Uncertain(Uncertain::Unreadable)
    );
}
