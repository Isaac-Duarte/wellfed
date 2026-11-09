//! An unreadable answer can follow a write that ran. Its durable pin must
//! survive the answer and a restart; resolving it never sends the write again.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use wellfed_model::{Answer, Op};

use super::Router;
use crate::api::feeder::{FeederReply, FeederRequest};
use crate::api::router::{RetryOutcome, RoutePath, WriteKind};
use crate::link::DirectLink;
use crate::session::{Command, Heard};

fn no_replay(commands: &mut mpsc::UnboundedReceiver<Command>) {
    assert!(
        matches!(commands.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
        "only the original request may reach the session"
    );
}

#[tokio::test]
async fn an_unreadable_direct_write_keeps_its_pin_until_acknowledged() {
    let directory = tempfile::tempdir().unwrap();
    let pins = directory.path().join("pending.json");
    let (commands, mut received) = mpsc::unbounded_channel();
    let link = Arc::new(DirectLink::new([23; 32], commands, &[24; 32]));
    link.opened();
    let feeder = link.feeder.clone();
    let router = Router::open(&pins).unwrap();
    router.attach(link.clone());

    // Inject the newer build's decoded answer at the session boundary, so
    // the request conversion, routing and real pin store all participate.
    let ask = router.ask(&feeder, FeederRequest::SetLock { locked: true });
    let answer = async {
        let Some(Command::Ask { op, reply }) = received.recv().await else {
            panic!("the router sends the lock request once");
        };
        assert!(matches!(op, Op::SetLock { locked: true }));
        assert!(reply.send(Heard::Answer(Answer::Unreadable)).is_ok());
    };
    let (routed, ()) = tokio::join!(ask, answer);
    assert_eq!(routed.reply, FeederReply::Unreadable);
    assert_eq!(routed.path, Some(RoutePath::Direct));
    assert!(routed.not_sent.is_none());
    assert!(routed.settlement.is_none());
    let pending = routed.pending.expect("an unreadable write stays pinned");
    assert_eq!(pending.kind, WriteKind::Lock);
    assert_eq!(pending.path, RoutePath::Direct);
    let key = pending.key;
    assert_eq!(router.pending().len(), 1);
    no_replay(&mut received);
    drop(router);

    let router = Router::open(&pins).unwrap();
    router.attach(link.clone());
    assert_eq!(router.pending().len(), 1, "the pin survived the restart");
    assert_eq!(router.pending()[0].key, key);
    for _ in 0..3 {
        let retry = tokio::time::timeout(Duration::from_secs(5), router.retry(&key))
            .await
            .expect("resolution never waits for another write answer");
        assert!(matches!(retry, RetryOutcome::Uncertain { pending } if pending.key == key));
        no_replay(&mut received);
    }

    // A direct non-feed write has no history to settle it. Only the
    // owner's explicit acknowledgement puts it away, and that is durable.
    router.acknowledge(&key).unwrap();
    assert!(router.pending().is_empty());
    no_replay(&mut received);
    drop(router);

    let router = Router::open(&pins).unwrap();
    assert!(
        router.pending().is_empty(),
        "acknowledgement survived the restart"
    );
    assert!(matches!(router.retry(&key).await, RetryOutcome::Unknown));
    no_replay(&mut received);
}
