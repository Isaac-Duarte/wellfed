//! The service on a temporary store, with fixed time and seeded random.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use wellfed_api::{AdoptFeeder, ClientRole, Connection, EventBody, EventQuery, FeederId};
use wellfed_connect::update::Pace;
use wellfed_service::{
    AlertReads, Backups, Clock, Config, Diagnostics, Discovery, Random, ReleaseKeys,
    ReleasesConfig, Service, StockConfig, StockPins, Timing,
};
use wellfed_store::Store;

use crate::feeder::{self, Running};
use crate::fixed::{FixedClock, SeededRandom};
use crate::until;

/// The config every test server starts with: no discovery, reconnects
/// and installs paced for the simulator, no trusted release key and no release
/// fetching, samples written within a tenth of a second and kept raw
/// forever, and seven backups. Its release cache and backup directories
/// are placeholders: a [`TestService`] keeps both beside its database.
pub fn config() -> Config {
    Config {
        key: [0x42; 32],
        host_zone: Some("UTC".into()),
        label: "test-server".into(),
        discovery: Discovery::Off,
        timing: Timing {
            retry_min: Duration::from_millis(50),
            retry_max: Duration::from_millis(400),
            time_sync: Duration::from_secs(3600),
            queue: Duration::from_secs(12),
            answer: Duration::from_secs(10),
            sample_wait: Duration::from_millis(100),
            session: Duration::from_secs(3600),
            transfer_idle: Duration::from_secs(30),
            // The simulator writes a sector within a pass or two, and
            // restarts a second after it answers a give-way, as the update
            // stage does, where the board takes seconds more.
            pace: Pace {
                settle: Duration::from_millis(20),
                restart: Duration::from_millis(1_500),
            },
            join_settle: Duration::from_secs(5),
        },
        // The diagnostics' own tests turn them on: elsewhere their reads
        // would share the lanes and the seeded ids with what is tested.
        diagnostics: Diagnostics::Off,
        // As the diagnostics: the alerts' own tests turn the reads on.
        alerts: AlertReads::Off,
        releases: ReleasesConfig {
            dir: PathBuf::from("releases"),
            keys: ReleaseKeys::default(),
            fetch: None,
            stock: StockConfig {
                dir: PathBuf::from("stock"),
                pins: StockPins::STOCK,
            },
        },
        raw_sample_days: None,
        backups: Backups {
            dir: PathBuf::from("backups"),
            keep: NonZeroU32::new(7).expect("seven is not zero"),
        },
        webhook: None,
    }
}

/// A database in a temporary directory, deleted with it.
#[derive(Debug)]
pub struct TempStore(TempDir);

impl TempStore {
    pub fn new() -> Self {
        Self(tempfile::tempdir().expect("make a temporary directory"))
    }

    pub fn path(&self) -> PathBuf {
        self.0.path().join("server.db")
    }

    /// The release cache, as the server keeps it in its data directory.
    pub fn releases(&self) -> PathBuf {
        self.0.path().join("releases")
    }

    /// The stock backups, as the server keeps them in its data directory.
    pub fn stock(&self) -> PathBuf {
        self.0.path().join("stock")
    }

    /// The backups, as the server keeps them in its data directory.
    pub fn backups(&self) -> PathBuf {
        self.0.path().join("backups")
    }

    /// The data directory itself.
    pub fn dir(&self) -> &std::path::Path {
        self.0.path()
    }

    /// Open the database, as the server does at the fixed time.
    pub async fn open(&self) -> Store {
        Store::open(&self.path(), FixedClock.now())
            .await
            .expect("open the test database")
    }
}

impl Default for TempStore {
    fn default() -> Self {
        Self::new()
    }
}

/// The service, running on its own temporary store.
#[derive(Debug)]
pub struct TestService {
    store: TempStore,
    service: Service,
}

impl TestService {
    /// The service with [`config`] on a fresh store.
    pub async fn start() -> Self {
        Self::start_with(config()).await
    }

    pub async fn start_with(config: Config) -> Self {
        Self::open(TempStore::new(), config).await
    }

    /// The service with [`config`] on a fresh store, reading `clock`.
    pub async fn start_with_clock(clock: Arc<dyn Clock>) -> Self {
        Self::open_with_clock(TempStore::new(), config(), clock).await
    }

    /// The service with `config` on a fresh store, reading `clock`.
    pub async fn start_with_config_and_clock(config: Config, clock: Arc<dyn Clock>) -> Self {
        Self::open_with_clock(TempStore::new(), config, clock).await
    }

    /// The service with [`config`] on a fresh store, drawing from `random`.
    pub async fn start_with_random(random: Arc<dyn Random>) -> Self {
        Self::open_with(TempStore::new(), config(), Arc::new(FixedClock), random).await
    }

    /// The service with `config` on `store`, which may hold a previous
    /// run's data, its release cache included. Each start draws from a
    /// seed of its own.
    pub async fn open(store: TempStore, config: Config) -> Self {
        Self::open_with_clock(store, config, Arc::new(FixedClock)).await
    }

    async fn open_with_clock(store: TempStore, config: Config, clock: Arc<dyn Clock>) -> Self {
        let random = Arc::new(SeededRandom::fresh());
        Self::open_with(store, config, clock, random).await
    }

    async fn open_with(
        store: TempStore,
        mut config: Config,
        clock: Arc<dyn Clock>,
        random: Arc<dyn Random>,
    ) -> Self {
        config.releases.dir = store.releases();
        config.releases.stock.dir = store.stock();
        config.backups.dir = store.backups();
        let service = Service::start(store.open().await, config, clock, random)
            .await
            .expect("start the service");
        Self { store, service }
    }

    /// Write the home setup would have made, unless there is one: a
    /// device belongs to the home, and a test may adopt without a setup.
    /// A setup after it takes it on.
    pub async fn ensure_home(&self) {
        let store = self.store.open().await;
        if store.home().await.expect("read the home").is_none() {
            store
                .create_home(&crate::home(), FixedClock.now())
                .await
                .expect("write the home");
        }
        store.close().await;
    }

    /// Stop, keeping the database for [`Self::reopen`].
    pub async fn stop(self) -> TempStore {
        self.service.shutdown().await;
        self.store
    }

    /// The service again on a stopped one's database, with [`config`].
    pub async fn reopen(store: TempStore) -> Self {
        Self::open(store, config()).await
    }

    /// As [`Self::reopen`], reading `clock`.
    pub async fn reopen_with_clock(store: TempStore, clock: Arc<dyn Clock>) -> Self {
        Self::open_with_clock(store, config(), clock).await
    }

    pub fn service(&self) -> &Service {
        &self.service
    }

    pub fn store(&self) -> &TempStore {
        &self.store
    }

    /// Adopt `sim` by an invite for the server, at its address.
    pub async fn adopt(&self, sim: &Running) -> FeederId {
        self.adopt_at(sim, &sim.address.to_string()).await
    }

    /// Adopt `sim` by an invite for the server, reaching it at `address`
    /// (a relay in front of it, say).
    pub async fn adopt_at(&self, sim: &Running, address: &str) -> FeederId {
        self.adopt_as(sim, address, ClientRole::Server).await
    }

    /// Adopt `sim` at `address` by an invite for `role`: an owner's key is
    /// one the feeder lets set its time settings.
    pub async fn adopt_as(&self, sim: &Running, address: &str, role: ClientRole) -> FeederId {
        self.ensure_home().await;
        let feeder = self
            .service
            .adopt(
                &crate::ada().into(),
                AdoptFeeder {
                    invite: feeder::invite(sim, role).await,
                    name: "kitchen".into(),
                    address: Some(address.into()),
                    phone_key: None,
                },
            )
            .await
            .expect("the service adopts the simulator");
        assert_eq!(feeder.role, role);
        // The session at adoption names the simulator's model.
        assert_eq!(feeder.model, wellfed_api::Model::Plaf103.into());
        assert_eq!(feeder.kind, wellfed_api::DeviceKind::Feeder.into());
        feeder.id
    }

    /// Wait until the feeder's link is `want`.
    pub async fn link(&self, id: FeederId, want: Connection) {
        until(&format!("the feeder's link is {want:?}"), || async {
            let feeders = self.service.feeders().await.expect("list the feeders");
            feeders.iter().any(|f| f.id == id && f.connection == want)
        })
        .await;
    }

    /// Wait until the server holds the feeder's events through `seq`, in
    /// whichever history epoch it holds. A test that wipes the feeder's
    /// flash between two waits crosses into a new epoch, whose numbers start
    /// again, and this would find the old epoch's `seq` at once: it must
    /// wait on what it expects of the new history instead.
    pub async fn synced(&self, id: FeederId, seq: u64) {
        until(
            &format!("the server holds events through {seq}"),
            || async {
                let detail = self.service.feeder(id).await.expect("read the feeder");
                detail.feeder.synced.is_some_and(|synced| synced >= seq)
            },
        )
        .await;
    }

    /// Wait until the server holds every event the feeder had when its
    /// status was read live. Online alone is not enough: the history is
    /// subscribed during the connect sequence, but its backlog is stored
    /// one event at a time after the feeder shows online. The status names
    /// no epoch, so this holds only within one, as [`Self::synced`] does.
    pub async fn caught_up(&self, id: FeederId) {
        let newest = std::cell::Cell::new(None);
        until("the feeder's status is read live", || async {
            let detail = self.service.feeder(id).await.expect("read the feeder");
            let live = detail.status.filter(|snapshot| snapshot.live);
            let read = live.is_some();
            newest.set(live.map(|snapshot| snapshot.status.last_event));
            read
        })
        .await;
        if let Some(Some(seq)) = newest.get() {
            self.synced(id, seq).await;
        }
    }

    /// The bodies of the first page of the feeder's history.
    pub async fn bodies(&self, id: FeederId) -> Vec<EventBody> {
        let page = self
            .service
            .events(id, EventQuery::default())
            .await
            .expect("read the feeder's history");
        page.events.into_iter().map(|event| event.body).collect()
    }

    /// Stop the service and its tasks.
    pub async fn shutdown(self) {
        self.service.shutdown().await;
    }
}
