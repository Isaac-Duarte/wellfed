//! The router against the simulated feeder (simulation, not hardware). A
//! feed's answer is dropped, and the feeder's feed journal proves one feed:
//! - the feed is pinned uncertain, stays pinned across a restart of the
//!   app, holds the next feed back until the owner puts it away, and a
//!   retry only reads: it settles from the feeder's history, never by a
//!   second send;
//! - a feed's pin never takes the end of a feed already running.
//!
//! A pins file that does not read is set aside, and no feed goes until the
//! owner has checked the bowls. One pins file has one router at a time: a
//! newer one in the process takes it over, and the one it took it from
//! sends no write again. A feed's own answer is told by its key, for a
//! screen that holds only the pin.

use std::cell::RefCell;
use std::net::SocketAddr;
use std::rc::Rc;
use std::time::Duration;

use tokio::sync::mpsc;
use wellfed_app_bridge::api::error::ErrorKind;
use wellfed_app_bridge::api::feeder::{FeederReply, FeederRequest};
use wellfed_app_bridge::api::keys::ClientKey;
use wellfed_app_bridge::api::router::{
    FeederRouter, NotSentReason, PendingWrite, RetryOutcome, RoutedFeed, RouterUpdate, Settlement,
    WriteKind,
};
use wellfed_app_bridge::api::session::{FeederSession, SessionUpdate};
use wellfed_feeder_sim::flash::Flash;
use wellfed_feeder_sim::harness::{Relay, Running, local};
use wellfed_session::noise::Keypair;

const WAIT: Duration = Duration::from_secs(30);

/// The owner's secret key, enrolled on every simulator here.
const OWNER: [u8; 32] = [0x21; 32];

/// A simulator on `flash`, with the owner enrolled.
async fn start_on(flash: &Rc<Flash>) -> Running {
    Running::start(flash, Some(*Keypair::from_secret(OWNER).public())).await
}

/// How many feeds the simulator on `flash` has ever reserved in its feed
/// journal: the count a test that retries a feed proves is one.
fn feeds(flash: &Rc<Flash>) -> u64 {
    match wellfed_feeder_sim::feeding::reserved(flash) {
        Ok(reserved) => reserved,
        Err(error) => panic!("the simulator's feed journal does not open: {error:?}"),
    }
}

async fn eventually<T>(what: &str, mut check: impl AsyncFnMut() -> Option<T>) -> T {
    let found = tokio::time::timeout(WAIT, async {
        loop {
            if let Some(found) = check().await {
                return found;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    found.unwrap_or_else(|_| panic!("{what}: not within {WAIT:?}"))
}

/// This phone: the feeder's owner key, its router and the pins' file.
struct Phone {
    key: ClientKey,
    pins: tempfile::TempDir,
    router: Rc<FeederRouter>,
    /// What the router tells the screens.
    updates: RefCell<mpsc::UnboundedReceiver<RouterUpdate>>,
}

impl Phone {
    fn new() -> Self {
        Self::on(tempfile::tempdir().unwrap())
    }

    fn on(pins: tempfile::TempDir) -> Self {
        let (router, updates) = heard(FeederRouter::new(path(&pins)).unwrap());
        Self {
            key: ClientKey::restore(OWNER.to_vec()).unwrap(),
            pins,
            router,
            updates,
        }
    }

    /// The app again, on the same pins, once the last run's router has let
    /// them go: one router holds a pins file.
    async fn restarted(self) -> Self {
        self.router.close();
        drop(self.router);
        let router = eventually(
            "the last router lets its pins go",
            async || match FeederRouter::new(path(&self.pins)) {
                Ok(router) => Some(router),
                Err(error) if error.kind == ErrorKind::InUse => None,
                Err(error) => panic!("{error}"),
            },
        )
        .await;
        let (router, updates) = heard(router);
        Self {
            router,
            updates,
            ..self
        }
    }

    /// Retry `key` until it settles, the way a screen's retry button
    /// would, or the router settles it on its own and says so.
    async fn settled(&self, key: &str) -> Settlement {
        eventually("the pending write settles", async || {
            match self.router.retry(key.to_owned()) {
                RetryOutcome::Settled { settlement, .. } => Some(settlement),
                RetryOutcome::Uncertain { .. } => None,
                RetryOutcome::Unknown => {
                    let mut updates = self.updates.borrow_mut();
                    while let Ok(update) = updates.try_recv() {
                        let RouterUpdate {
                            pending,
                            settlement,
                        } = update;
                        if pending.key == key {
                            return Some(settlement);
                        }
                    }
                    panic!("{key} settled unheard");
                }
            }
        })
        .await
    }

    /// Open this phone's own session with `sim` at `address`, through the
    /// router, and wait for its card.
    async fn open(&self, sim: &Running, address: SocketAddr) -> Direct {
        let session = Rc::new(
            FeederSession::new(&self.key, sim.identity.to_vec(), address.to_string(), false)
                .unwrap(),
        );
        self.router.attach(&session);
        let (sender, mut updates) = mpsc::unbounded_channel();
        let running = Rc::clone(&session);
        tokio::task::spawn_local(async move {
            running
                .run_with(move |update| sender.send(update).is_ok())
                .await;
        });
        tokio::time::timeout(WAIT, async {
            while let Some(update) = updates.recv().await {
                if let SessionUpdate::Status { .. } = update {
                    return;
                }
            }
            panic!("the session ended before its card");
        })
        .await
        .expect("the session opens");
        Direct { session, updates }
    }
}

/// One session, and what it tells Dart.
struct Direct {
    session: Rc<FeederSession>,
    updates: mpsc::UnboundedReceiver<SessionUpdate>,
}

impl Direct {
    /// Wait until the session has ended.
    async fn closed(&mut self) {
        tokio::time::timeout(WAIT, async {
            while let Some(update) = self.updates.recv().await {
                if let SessionUpdate::Closed { .. } = update {
                    return;
                }
            }
        })
        .await
        .expect("the session ends");
    }
}

fn path(dir: &tempfile::TempDir) -> String {
    dir.path().join("pending.json").display().to_string()
}

fn identity(sim: &Running) -> String {
    wellfed_session::hex::Lower(&sim.identity).to_string()
}

fn uncertain(routed: RoutedFeed) -> PendingWrite {
    match routed {
        RoutedFeed::Uncertain { pending } => pending,
        other => panic!("the feed should be uncertain, not {other:?}"),
    }
}

/// `router`, its updates heard into a channel.
fn heard(
    router: FeederRouter,
) -> (
    Rc<FeederRouter>,
    RefCell<mpsc::UnboundedReceiver<RouterUpdate>>,
) {
    let router = Rc::new(router);
    let (sender, updates) = mpsc::unbounded_channel();
    let hearing = Rc::clone(&router);
    tokio::task::spawn_local(async move {
        hearing
            .updates_with(move |update| sender.send(update).is_ok())
            .await;
    });
    (router, RefCell::new(updates))
}

fn fed_once(settlement: &Settlement) {
    assert!(
        matches!(
            settlement,
            Settlement::Fed {
                requested: 1,
                completed: 1,
                ..
            }
        ),
        "{settlement:?}"
    );
}

#[test]
fn a_feed_whose_answer_is_lost_stays_pinned_across_a_restart_and_feeds_once() {
    local(async {
        let flash = Flash::memory();
        let sim = start_on(&flash).await;
        let relay = Relay::start(sim.address).await;
        let phone = Phone::new();
        let mut direct = phone.open(&sim, relay.address).await;
        let kitchen = identity(&sim);

        relay.hold_answers();
        let router = Rc::clone(&phone.router);
        let asked = kitchen.clone();
        let feeding = tokio::task::spawn_local(async move { router.feed(asked, 1).await });
        eventually("the feed is reserved", async || {
            (feeds(&flash) == 1).then_some(())
        })
        .await;
        relay.cut();
        let pending = uncertain(feeding.await.unwrap());
        assert_eq!(pending.kind, WriteKind::Feed);
        assert_eq!(pending.portions, Some(1));
        direct.closed().await;

        // Its session is gone: a retry cannot say, and sends nothing.
        let RetryOutcome::Uncertain { .. } = phone.router.retry(pending.key.clone()) else {
            panic!("nothing has said yet");
        };

        // The app restarts: the pin is still there, and still holds the next
        // feed back, so no feed can double it.
        drop(direct.session);
        let phone = phone.restarted().await;
        assert_eq!(phone.router.pending(), std::slice::from_ref(&pending));
        relay.resume();
        let direct = phone.open(&sim, relay.address).await;
        fed_once(&phone.settled(&pending.key).await);
        assert!(phone.router.pending().is_empty());
        assert_eq!(feeds(&flash), 1, "a retry only reads");
        drop(direct.session);
        sim.stop().await;
    });
}

/// An uncertain feed holds the next feed on its feeder back until the
/// owner, having looked at the bowl, puts it away; put away, it is never
/// sent again, and the history still settles it.
#[test]
fn an_uncertain_feed_holds_the_next_until_the_owner_puts_it_away() {
    local(async {
        let flash = Flash::memory();
        let sim = start_on(&flash).await;
        let relay = Relay::start(sim.address).await;
        let phone = Phone::new();
        let mut direct = phone.open(&sim, relay.address).await;
        let kitchen = identity(&sim);

        relay.hold_answers();
        let router = Rc::clone(&phone.router);
        let asked = kitchen.clone();
        let feeding = tokio::task::spawn_local(async move { router.feed(asked, 1).await });
        eventually("the feed is reserved", async || {
            (feeds(&flash) == 1).then_some(())
        })
        .await;
        relay.cut();
        let pending = uncertain(feeding.await.unwrap());
        direct.closed().await;

        // Held before anything is sent.
        let RoutedFeed::Held { pending: holding } = phone.router.feed(kitchen.clone(), 1).await
        else {
            panic!("an uncertain feed holds the next");
        };
        assert_eq!(holding.key, pending.key);

        // Put away, the next feed is admitted (and goes nowhere: its
        // session has ended), and the old one stays pinned.
        phone.router.acknowledge(pending.key.clone()).unwrap();
        let RoutedFeed::NotSent { reason } = phone.router.feed(kitchen.clone(), 1).await else {
            panic!("put away, the next feed is no longer held");
        };
        assert_eq!(reason, NotSentReason::Session);
        let [kept] = phone.router.pending().try_into().expect("one pin");
        assert_eq!(kept.key, pending.key);
        assert!(kept.acknowledged);

        // The history still settles it, and it never fed again.
        relay.resume();
        let again = phone.open(&sim, relay.address).await;
        fed_once(&phone.settled(&pending.key).await);
        assert_eq!(feeds(&flash), 1);
        drop(direct.session);
        drop(again.session);
        sim.stop().await;
    });
}

#[test]
fn a_feed_never_takes_the_end_of_one_already_running() {
    local(async {
        let flash = Flash::memory();
        let sim = start_on(&flash).await;
        let relay = Relay::start(sim.address).await;
        let phone = Phone::new();
        let mut direct = phone.open(&sim, relay.address).await;
        let kitchen = identity(&sim);

        let RoutedFeed::Started { feed: first, .. } = phone.router.feed(kitchen.clone(), 1).await
        else {
            panic!("the first feed goes");
        };
        // The first feed's end, and the next feed's answer, are held back
        // and lost with the session.
        relay.hold_answers();
        let sent = relay.passed();
        let router = Rc::clone(&phone.router);
        let asked = kitchen.clone();
        let feeding = tokio::task::spawn_local(async move { router.feed(asked, 1).await });
        eventually("the second feed reaches the feeder", async || {
            (relay.passed() > sent).then_some(())
        })
        .await;
        relay.cut();
        let second = match feeding.await.unwrap() {
            RoutedFeed::Uncertain { pending } => pending,
            // The first was still unanswered on the phone: it holds.
            RoutedFeed::Held { .. } => {
                drop(direct.session);
                sim.stop().await;
                return;
            }
            other => panic!("{other:?}"),
        };
        direct.closed().await;

        // A new session reads the history, the first feed's end in it,
        // which comes after the second's mark and asked as many portions.
        relay.resume();
        let _again = phone.open(&sim, relay.address).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        match phone.router.retry(second.key.clone()) {
            // The feeder refused the second while the first ran.
            RetryOutcome::Uncertain { .. } => assert_eq!(feeds(&flash), 1),
            // The first had ended and the second ran: its own end.
            RetryOutcome::Settled {
                settlement: Settlement::Fed { feed, .. },
                ..
            } => {
                assert_ne!(feed, first, "the first feed's end");
                assert_eq!(feeds(&flash), 2);
            }
            other => panic!("{other:?}"),
        }
        drop(direct.session);
        sim.stop().await;
    });
}

#[test]
fn a_pins_file_that_does_not_read_is_set_aside_and_holds_feeds_until_the_bowls_are_checked() {
    local(async {
        let flash = Flash::memory();
        let sim = start_on(&flash).await;
        let pins = tempfile::tempdir().unwrap();
        std::fs::write(path(&pins), b"{ not json").unwrap();
        let phone = Phone::on(pins);
        let _direct = phone.open(&sim, sim.address).await;
        let kitchen = identity(&sim);

        let set_aside = phone
            .router
            .set_aside()
            .expect("the router says it set the file aside");
        assert_eq!(
            std::fs::read(&set_aside.moved_to).unwrap(),
            b"{ not json",
            "kept, not deleted"
        );
        let RoutedFeed::NotSent { reason } = phone.router.feed(kitchen.clone(), 1).await else {
            panic!("a lost pin may have been a feed that ran");
        };
        assert_eq!(reason, NotSentReason::SetAside);
        assert_eq!(feeds(&flash), 0);

        // The hold is on every feeder, and across a restart.
        let phone = phone.restarted().await;
        let _direct = phone.open(&sim, sim.address).await;
        assert_eq!(phone.router.set_aside(), Some(set_aside));
        let RoutedFeed::NotSent { reason } = phone.router.feed(kitchen.clone(), 1).await else {
            panic!("still held after a restart");
        };
        assert_eq!(reason, NotSentReason::SetAside);

        phone.router.bowls_checked().unwrap();
        assert_eq!(phone.router.set_aside(), None);
        let RoutedFeed::Started { .. } = phone.router.feed(kitchen, 1).await else {
            panic!("the owner checked the bowls");
        };

        // Written down: the next start holds nothing back.
        let phone = phone.restarted().await;
        assert_eq!(phone.router.set_aside(), None);
        sim.stop().await;
    });
}

#[test]
fn a_router_left_behind_is_fenced_by_the_next_and_sends_no_write() {
    local(async {
        let flash = Flash::memory();
        let sim = start_on(&flash).await;
        let phone = Phone::new();
        let _direct = phone.open(&sim, sim.address).await;
        let kitchen = identity(&sim);

        // The app's isolate restarts in the live process: its router is
        // never closed, and the new one opens the same pins.
        let next = FeederRouter::new(path(&phone.pins)).expect("taken over");
        let RoutedFeed::NotSent { reason } = phone.router.feed(kitchen.clone(), 1).await else {
            panic!("the old router writes nothing down, so it sends nothing");
        };
        assert_eq!(reason, NotSentReason::Store);
        let reply = phone
            .router
            .ask(kitchen.clone(), FeederRequest::SetLock { locked: true })
            .await;
        assert!(
            matches!(
                reply.reply,
                FeederReply::NotSent {
                    reason: NotSentReason::Store,
                }
            ),
            "{reply:?}"
        );
        assert_eq!(feeds(&flash), 0);

        // The new one, with its own session, feeds.
        let session = FeederSession::new(
            &phone.key,
            sim.identity.to_vec(),
            sim.address.to_string(),
            false,
        )
        .unwrap();
        next.attach(&session);
        let running = Rc::new(session);
        let ran = Rc::clone(&running);
        tokio::task::spawn_local(async move { ran.run_with(|_| true).await });
        eventually("the new router feeds", async || {
            match next.feed(kitchen.clone(), 1).await {
                RoutedFeed::Started { .. } => Some(()),
                RoutedFeed::NotSent { .. } => None,
                other => panic!("{other:?}"),
            }
        })
        .await;
        sim.stop().await;
    });
}

#[test]
fn a_feed_s_answer_is_told_by_its_key_for_a_screen_that_holds_only_the_pin() {
    local(async {
        let flash = Flash::memory();
        let sim = start_on(&flash).await;
        let phone = Phone::new();
        let _direct = phone.open(&sim, sim.address).await;
        let kitchen = identity(&sim);

        let RoutedFeed::Started { key, feed, .. } = phone.router.feed(kitchen, 1).await else {
            panic!("the feed starts");
        };
        let told = eventually("the feed's answer is told", async || {
            let mut updates = phone.updates.borrow_mut();
            std::iter::from_fn(|| updates.try_recv().ok()).find_map(|update| {
                let RouterUpdate {
                    pending,
                    settlement,
                } = update;
                (pending.key == key).then_some(settlement)
            })
        })
        .await;
        assert_eq!(told, Settlement::Started { feed });
        sim.stop().await;
    });
}

/// A feeder the phone holds no session with is no write's: nothing is
/// pinned, and nothing goes.
#[test]
fn a_feeder_with_no_session_is_sent_nothing_and_pins_nothing() {
    local(async {
        let phone = Phone::new();
        let RoutedFeed::NotSent { reason } = phone.router.feed("ab".repeat(32), 1).await else {
            panic!("no session, no feed");
        };
        assert_eq!(reason, NotSentReason::NoPath);
        let reply = phone
            .router
            .ask("ab".repeat(32), FeederRequest::SetLock { locked: true })
            .await;
        assert_eq!(
            reply.reply,
            FeederReply::NotSent {
                reason: NotSentReason::NoPath
            }
        );
        assert!(phone.router.pending().is_empty());
    });
}
