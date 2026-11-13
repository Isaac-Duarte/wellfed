//! What every scan and connection on one adapter shares, kept apart from
//! `btleplug` so tests drive it with fakes: when each device was last
//! heard, how many scans want discovery running, and looking a feeder up
//! again once the stack has forgotten it.

use std::collections::HashMap;
use std::future::Future;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::Error;

/// How recently a device must have advertised to count as advertising now.
/// A feeder in setup mode advertises every 100 ms, so this is some fifty
/// advertisements: a few lost ones never drop it, and a feeder that left
/// setup mode leaves the list within seconds.
pub(crate) const RECENT: Duration = Duration::from_secs(5);

/// When each device was last heard advertising, by the stack's id for it.
#[derive(Default)]
pub(crate) struct Sightings {
    last: HashMap<String, Instant>,
}

impl Sightings {
    /// `id` was heard `at`. Devices not heard lately are let go here, so
    /// the passers-by an unfiltered scan hears don't pile up.
    pub(crate) fn heard(&mut self, id: String, at: Instant) {
        self.last
            .retain(|_, last| at.saturating_duration_since(*last) <= RECENT);
        self.last.insert(id, at);
    }

    /// Whether `id` advertised within [`RECENT`] of `now`.
    pub(crate) fn recent(&self, id: &str, now: Instant) -> bool {
        self.last
            .get(id)
            .is_some_and(|at| now.saturating_duration_since(*at) <= RECENT)
    }
}

/// Which connection to each device is the newest, by the stack's id for it:
/// the stacks keep one link per device, which every connection to it
/// shares, so only the newest may hang it up.
#[derive(Default)]
pub(crate) struct Links {
    newest: HashMap<String, u64>,
    claims: u64,
}

impl Links {
    /// A new connection to `id`, now the newest.
    pub(crate) fn claim(&mut self, id: &str) -> u64 {
        self.claims += 1;
        self.newest.insert(id.to_owned(), self.claims);
        self.claims
    }

    /// Whether `claim` is still the newest to `id`, which it then lets go.
    pub(crate) fn release(&mut self, id: &str, claim: u64) -> bool {
        let newest = self.newest.get(id) == Some(&claim);
        if newest {
            self.newest.remove(id);
        }
        newest
    }
}

/// Turning the adapter's discovery on and off.
pub(crate) trait Discovery: Send + Sync {
    fn start(&self) -> impl Future<Output = Result<(), Error>> + Send;
    fn stop(&self) -> impl Future<Output = ()> + Send;
}

/// Discovery shared by every scan running at once. The adapter has one
/// discovery, so a scan that started and stopped it by itself would end
/// every other scan with it: it starts with the first scan and stops with
/// the last.
pub(crate) struct Scans<D> {
    discovery: D,
    running: Mutex<usize>,
}

impl<D: Discovery> Scans<D> {
    pub(crate) fn new(discovery: D) -> Self {
        Self {
            discovery,
            running: Mutex::new(0),
        }
    }

    /// One more scan; discovery starts if it is the first. A start that
    /// fails counts no scan.
    pub(crate) async fn join(&self) -> Result<(), Error> {
        let mut running = self.running.lock().await;
        if *running == 0 {
            self.discovery.start().await?;
        }
        *running += 1;
        Ok(())
    }

    /// Start discovery again for the scans still running: turning the
    /// adapter off ended it under them. A start that fails leaves them
    /// hearing nothing until the next time it comes on.
    pub(crate) async fn resume(&self) {
        let running = self.running.lock().await;
        if *running > 0 {
            let _ = self.discovery.start().await;
        }
    }

    /// One scan fewer; discovery stops with the last.
    pub(crate) async fn leave(&self) {
        let mut running = self.running.lock().await;
        match *running {
            0 => {}
            1 => {
                *running = 0;
                self.discovery.stop().await;
            }
            more => *running = more - 1,
        }
    }
}

/// `look` for something at once, and if it is not there, again every
/// `every` for at most `wait` with a scan from `start` running, which comes
/// back for the caller to stop. `None` when it never turned up.
pub(crate) async fn look_again<T, G, Look, Start>(
    mut look: impl FnMut() -> Look,
    start: impl FnOnce() -> Start,
    every: Duration,
    wait: Duration,
) -> (Result<Option<T>, Error>, Option<G>)
where
    Look: Future<Output = Result<Option<T>, Error>>,
    Start: Future<Output = Result<G, Error>>,
{
    match look().await {
        Ok(None) => {}
        known => return (known, None),
    }

    let scan = match start().await {
        Ok(scan) => scan,
        Err(error) => return (Err(error), None),
    };
    let found = timeout(wait, async {
        loop {
            if let Some(found) = look().await? {
                return Ok(found);
            }
            tokio::time::sleep(every).await;
        }
    })
    .await;
    let found = match found {
        Ok(found) => found.map(Some),
        Err(_) => Ok(None),
    };
    (found, Some(scan))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn only_a_recent_sighting_is_advertising_now() {
        let start = Instant::now();
        let mut sightings = Sightings::default();
        sightings.heard("feeder".into(), start);

        assert!(sightings.recent("feeder", start + RECENT));
        assert!(!sightings.recent("feeder", start + RECENT + Duration::from_millis(1)));
        assert!(!sightings.recent("never-heard", start));

        sightings.heard("feeder".into(), start + RECENT * 2);
        assert!(sightings.recent("feeder", start + RECENT * 2));
    }

    #[derive(Default)]
    struct Counted {
        starts: AtomicUsize,
        stops: AtomicUsize,
        refuse: bool,
    }

    impl Discovery for Arc<Counted> {
        async fn start(&self) -> Result<(), Error> {
            if self.refuse {
                return Err(Error::Off);
            }
            self.starts.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn stop(&self) {
            self.stops.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn overlapping_scans_share_one_discovery_and_the_last_stops_it() {
        let counted = Arc::new(Counted::default());
        let scans = Scans::new(counted.clone());

        scans.join().await.unwrap();
        scans.join().await.unwrap();
        assert_eq!(counted.starts.load(Ordering::SeqCst), 1);

        scans.leave().await;
        assert_eq!(
            counted.stops.load(Ordering::SeqCst),
            0,
            "the other scan still runs"
        );
        scans.leave().await;
        assert_eq!(counted.stops.load(Ordering::SeqCst), 1);

        // A stray leave changes nothing; the next scan starts discovery again.
        scans.leave().await;
        scans.join().await.unwrap();
        assert_eq!(counted.stops.load(Ordering::SeqCst), 1);
        assert_eq!(counted.starts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn discovery_starts_again_when_bluetooth_comes_back_on_under_a_scan() {
        let counted = Arc::new(Counted::default());
        let scans = Scans::new(counted.clone());

        // Nothing held: coming back on starts nothing.
        scans.resume().await;
        assert_eq!(counted.starts.load(Ordering::SeqCst), 0);

        scans.join().await.unwrap();
        // Bluetooth went off, which ended discovery, and came back on.
        scans.resume().await;
        assert_eq!(counted.starts.load(Ordering::SeqCst), 2);

        scans.leave().await;
        assert_eq!(counted.stops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn only_the_newest_connection_to_a_feeder_hangs_it_up() {
        let mut links = Links::default();
        let late = links.claim("feeder");
        let retry = links.claim("feeder");
        let other = links.claim("another");

        assert!(
            !links.release("feeder", late),
            "the retry holds the link now"
        );
        assert!(links.release("feeder", retry));
        assert!(!links.release("feeder", retry), "a hang-up happens once");
        assert!(links.release("another", other));
    }

    #[test]
    fn sightings_let_go_of_devices_not_heard_lately() {
        let start = Instant::now();
        let mut sightings = Sightings::default();
        sightings.heard("passer-by".into(), start);
        sightings.heard("feeder".into(), start + RECENT * 2);

        assert_eq!(sightings.last.len(), 1);
        assert!(sightings.recent("feeder", start + RECENT * 2));
    }

    #[tokio::test]
    async fn a_scan_that_could_not_start_counts_for_nothing() {
        let counted = Arc::new(Counted {
            refuse: true,
            ..Counted::default()
        });
        let scans = Scans::new(counted.clone());

        assert!(scans.join().await.is_err());
        scans.leave().await;

        assert_eq!(counted.stops.load(Ordering::SeqCst), 0);
    }

    /// A lookup's answer, ready at once.
    type Looked = std::future::Ready<Result<Option<&'static str>, Error>>;

    /// A lookup that misses `misses` times, then finds the feeder, and how
    /// many looks it took.
    fn looking(misses: usize) -> (Arc<AtomicUsize>, impl FnMut() -> Looked) {
        let looks = Arc::new(AtomicUsize::new(0));
        let counted = looks.clone();
        let look = move || {
            let seen = counted.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok((seen >= misses).then_some("feeder")))
        };
        (looks, look)
    }

    #[tokio::test(start_paused = true)]
    async fn a_feeder_still_known_is_taken_with_no_scan() {
        let (_, look) = looking(0);
        let starts = AtomicUsize::new(0);

        let (found, scan) = look_again(
            look,
            || async {
                starts.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            Duration::from_millis(500),
            Duration::from_secs(10),
        )
        .await;

        assert_eq!(found.unwrap(), Some("feeder"));
        assert!(scan.is_none());
        assert_eq!(starts.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_forgotten_feeder_is_heard_again_by_a_scan() {
        // Forgotten when its last link closed, as btleplug does on macOS and
        // Windows; heard again on the third look.
        let (looks, look) = looking(3);

        let (found, scan) = look_again(
            look,
            || async { Ok("scan") },
            Duration::from_millis(500),
            Duration::from_secs(10),
        )
        .await;

        assert_eq!(found.unwrap(), Some("feeder"));
        assert_eq!(scan, Some("scan"), "the scan comes back to be stopped");
        assert_eq!(looks.load(Ordering::SeqCst), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn a_feeder_that_never_comes_back_is_not_found_in_the_wait() {
        let (_, look) = looking(usize::MAX);

        let (found, scan) = look_again(
            look,
            || async { Ok("scan") },
            Duration::from_millis(500),
            Duration::from_secs(10),
        )
        .await;

        assert_eq!(found.unwrap(), None);
        assert_eq!(scan, Some("scan"));
    }
}
