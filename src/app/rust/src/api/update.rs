//! Updating a Wellfed feeder from the image this APK carries: the driver the
//! install's promotion and the CLI share, on a thread of its own, with what it
//! reports on one stream. A failure is a value on that stream, for the reason
//! [`crate::api::install`] gives.

use std::cell::Cell;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use flutter_rust_bridge::frb;
use wellfed_client::offer::{self, Candidate, Confirmation, Gate, Offer, Restarted, confirmation};
use wellfed_client::update::{Image, InstallError};
use wellfed_connect::Connection;
use wellfed_connect::update::Pushed;
use wellfed_install::event::Event;
use wellfed_install::push::push;
use wellfed_install::{Discovery, Review};
use wellfed_model::{Answer, Bank0Blocked, Op, Readiness, Skew, UpdatePhase, UpdateView};
use wellfed_session::hex;
use wellfed_session::noise::Keypair;

use crate::api::error::{BridgeError, ErrorKind};
use crate::api::feeder::{FeederRefusal, UpdateInfo};
use crate::api::install::{ImageCheck, InstallAnswer, InstallAsk, InstallUpdate};
use crate::api::keys::ClientKey;
use crate::api::model::UpdateStop;
use crate::frb_generated::StreamSink;
use crate::install::{AppDiscovery, Asks, UPDATE_MANIFEST, check_image, guarded, pin};
use crate::requests::refusal_of;

/// The driver's future is deep, as an install's is.
const STACK: usize = 16 * 1024 * 1024;
/// From activation until the restart has begun.
const RESTART: Duration = Duration::from_secs(20);
/// As long as an install waits for its build to confirm.
const CONFIRM_WAIT: Duration = Duration::from_secs(6 * 60);
const POLL: Duration = Duration::from_secs(5);

pub enum UpdateProgress {
    /// Bank 0 took this much of the image.
    Sending {
        written: u32,
        length: u32,
    },
    /// The connection dropped; chunks resume where the feeder says.
    Reconnecting,
    /// Where the feeder is: answer by id with [`FirmwareUpdate::answer`].
    Ask {
        id: u32,
        ask: InstallAsk,
    },
    /// The feeder restarts onto the new version and tries it.
    Activated,
    Ended {
        end: UpdateEnd,
    },
}

/// How an update ended.
pub enum UpdateEnd {
    /// The new version runs, and has confirmed itself.
    Confirmed,
    /// It already ran this build; nothing was sent.
    AlreadyRunning,
    Refused {
        refusal: FeederRefusal,
    },
    /// Bank 0 stopped the install before activating it.
    Stopped {
        stop: UpdateStop,
    },
    /// Bank 0 is not installing this update any more.
    Lost,
    /// The offer went out and the feeder stopped answering: only the feeder
    /// can tell what happened, and the offer is not sent again.
    Uncertain,
    /// The feeder never answered, so nothing was offered.
    NotOffered,
    /// The new version was activated, and has not confirmed in the time an
    /// install gives it.
    Unconfirmed,
    /// The new version was activated, and the feeder runs bank 0's copy, which
    /// may be older than what it ran before the update: a trial that failed.
    WentBack,
    /// The new version was activated, and bank 1 runs some other build: an
    /// update from elsewhere came between.
    WrongBuild,
    /// Something of this phone's before any offer: the image, or its address.
    Failed {
        detail: String,
    },
}

#[frb(opaque)]
pub struct FirmwareUpdate {
    keys: Keypair,
    feeder: [u8; 32],
    address: Option<String>,
    image_dir: String,
    asks: Arc<Asks>,
    /// The image is offered once, by the first [`Self::run`].
    started: AtomicBool,
}

impl FirmwareUpdate {
    #[frb(sync)]
    pub fn new(
        key: &ClientKey,
        feeder: Vec<u8>,
        address: Option<String>,
        image_dir: String,
    ) -> Result<Self, BridgeError> {
        let feeder = <[u8; 32]>::try_from(feeder)
            .map_err(|_| BridgeError::new(ErrorKind::Key, "a feeder's key is 32 bytes"))?;
        Ok(Self {
            keys: key.keypair().clone(),
            feeder,
            address,
            image_dir,
            asks: Arc::new(Asks::new()),
            started: AtomicBool::new(false),
        })
    }

    /// Answer the ask `id`. An answer nobody waits for any more is dropped.
    #[frb(sync)]
    pub fn answer(&self, id: u32, answer: InstallAnswer) {
        self.asks.answer(id, answer);
    }

    /// Offer the image once, and report until the update ends. The stream
    /// closes with the run. A second call offers nothing and leaves the
    /// first run's asks where they go: its own stream hears that it ran.
    pub fn run(&self, updates: StreamSink<UpdateProgress>) {
        if !self.first_run() {
            let _ = updates.add(UpdateProgress::Ended {
                end: UpdateEnd::Failed {
                    detail: "this update already ran".to_string(),
                },
            });
            return;
        }

        let keys = self.keys.clone();
        let (feeder, address) = (self.feeder, self.address.clone());
        let image_dir = self.image_dir.clone();
        let asks = self.asks.clone();

        // Held by the thread itself, through the give-way and bank 0's
        // restart, which outlive an engine swiped away.
        let held = crate::alerts::RunThread::hold();
        // The driver's futures are not Send, so it keeps a thread.
        let thread = std::thread::Builder::new()
            .name("wellfed-update".into())
            .stack_size(STACK)
            .spawn(move || {
                let _held = held;
                let updates = Arc::new(updates);
                let heard = updates.clone();
                *asks.deliver() = Some(Box::new(move |update| match update {
                    InstallUpdate::Ask { id, ask } => {
                        heard.add(UpdateProgress::Ask { id, ask }).is_ok()
                    }
                    _ => false,
                }));
                let report = |progress| {
                    // Nobody listening: the screen went, and the feeder still
                    // wants its chunks.
                    let _ = updates.add(progress);
                };
                // A panic says nothing of an offer that may be out.
                let end = guarded(
                    || match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(runtime) => runtime.block_on(update(
                            &keys,
                            feeder,
                            address.as_deref(),
                            &image_dir,
                            asks.clone(),
                            &report,
                        )),
                        Err(error) => UpdateEnd::Failed {
                            detail: error.to_string(),
                        },
                    },
                    |_| UpdateEnd::Uncertain,
                );
                report(UpdateProgress::Ended { end });
                *asks.deliver() = None;
            });
        // With no thread the sink dropped with the closure, which closes Dart's
        // stream: the screen reads that as a run with no end.
        drop(thread);
    }
}

impl FirmwareUpdate {
    /// Whether this is the first run, which alone may offer the image.
    fn first_run(&self) -> bool {
        !self.started.swap(true, Ordering::SeqCst)
    }
}

async fn update(
    keys: &Keypair,
    feeder: [u8; 32],
    address: Option<&str>,
    image_dir: &str,
    asks: Arc<Asks>,
    report: &impl Fn(UpdateProgress),
) -> UpdateEnd {
    match try_update(keys, feeder, address, image_dir, asks, report).await {
        Ok(end) => end,
        Err(failure) => UpdateEnd::Failed {
            detail: crate::install::text(&failure),
        },
    }
}

/// What of this phone's stopped an update before any offer.
#[derive(Debug, thiserror::Error)]
enum NotOffered {
    #[error("this app's image did not pass its check")]
    ImageUnchecked,
    #[error("read the image's review")]
    Review(#[source] wellfed_install::Error),
    #[error("read the image's update manifest")]
    Manifest(#[source] std::io::Error),
}

async fn try_update(
    keys: &Keypair,
    feeder: [u8; 32],
    address: Option<&str>,
    image_dir: &str,
    asks: Arc<Asks>,
    report: &impl Fn(UpdateProgress),
) -> Result<UpdateEnd, NotOffered> {
    // The image is the one this build pinned, or nothing goes out.
    let ImageCheck::Ready { version, .. } = check_image(Path::new(image_dir), pin().as_ref())
    else {
        return Err(NotOffered::ImageUnchecked);
    };
    let last = Cell::new(address.and_then(|address| wellfed_connect::parse_address(address).ok()));
    let discovery = AppDiscovery(asks);
    let review = Review::open(Path::new(image_dir)).map_err(NotOffered::Review)?;
    let manifest =
        std::fs::read(Path::new(image_dir).join(UPDATE_MANIFEST)).map_err(NotOffered::Manifest)?;
    let payload = review.payload().map_err(NotOffered::Review)?;
    let served = review.build().to_owned();

    // Where it last answered, then where NSD says: a restart can move it.
    let connect = async || -> Result<Connection, wellfed_install::Error> {
        if let Some(address) = last.get() {
            let ephemeral = wellfed_install::session::random()?;
            if let Ok(session) = Connection::open(keys, feeder, address, ephemeral).await {
                return Ok(session);
            }
        }
        let found = discovery.locate(feeder).await.ok_or_else(|| {
            wellfed_connect::Error::NotFound(hex::Lower(&feeder[..8]).to_string())
        })?;
        let ephemeral = wellfed_install::session::random()?;
        let session = Connection::open(keys, feeder, found, ephemeral).await?;
        last.set(Some(found));
        Ok(session)
    };
    let events = |event: Event| match event {
        Event::Written { written, length } => report(UpdateProgress::Sending { written, length }),
        Event::SessionDropped { .. } => report(UpdateProgress::Reconnecting),
        Event::Activated | Event::AlreadyActivated => report(UpdateProgress::Activated),
        _ => {}
    };

    // The driver's own failure may come after the offer went out, and it does
    // not say which, so it cannot be called nothing.
    let Ok(pushed) = push(
        Image {
            manifest: &manifest,
            downgrade: None,
            payload,
        },
        connect,
        |bank, build| bank == 1 && build.eq_ignore_ascii_case(&served),
        &events,
    )
    .await
    else {
        return Ok(UpdateEnd::Uncertain);
    };

    Ok(match pushed {
        Pushed::Activated => confirm(&connect, &served, version).await,
        Pushed::AlreadyRunning => UpdateEnd::AlreadyRunning,
        Pushed::Failed(InstallError::Refused(refusal)) => UpdateEnd::Refused {
            refusal: refusal_of(refusal),
        },
        Pushed::Failed(InstallError::Stopped(stop)) => UpdateEnd::Stopped { stop },
        Pushed::Failed(InstallError::Lost) => UpdateEnd::Lost,
        Pushed::Failed(InstallError::Uncertain(_)) | Pushed::Gone { offered: true } => {
            UpdateEnd::Uncertain
        }
        Pushed::Gone { offered: false } => UpdateEnd::NotOffered,
    })
}

/// Until the new version runs and has confirmed itself, or the feeder shows it
/// gave way to bank 0. These are reads, so looking again sends nothing it could act on.
async fn confirm(
    connect: &impl AsyncFn() -> Result<Connection, wellfed_install::Error>,
    served: &str,
    version: u32,
) -> UpdateEnd {
    tokio::time::sleep(RESTART).await;
    after_restart(connect, served, version).await
}

/// [`confirm`] once the restart has begun: read until the shared rule
/// settles the end, or [`CONFIRM_WAIT`] passes.
async fn after_restart(
    connect: &impl AsyncFn() -> Result<Connection, wellfed_install::Error>,
    served: &str,
    version: u32,
) -> UpdateEnd {
    let deadline = Instant::now() + CONFIRM_WAIT;
    while Instant::now() < deadline {
        if let Some(read) = reading(connect).await {
            let restarted = Restarted {
                build: &read.build,
                bank: read.view.bank,
                confirmed: read.view.min_version,
                ready: read.ready,
            };
            match confirmation(served, version, restarted) {
                Confirmation::Confirmed => return UpdateEnd::Confirmed,
                Confirmation::FellBack => return UpdateEnd::WentBack,
                Confirmation::WrongBuild => return UpdateEnd::WrongBuild,
                Confirmation::Pending => {}
            }
        }
        tokio::time::sleep(POLL).await;
    }
    UpdateEnd::Unconfirmed
}

/// What a fresh session says it runs.
struct Reading {
    /// The build its welcome names.
    build: String,
    view: UpdateView,
    /// Whether it reports itself ready to feed.
    ready: bool,
}

/// What a fresh session says it runs, or `None` while it cannot say. The
/// bank comes from the update state and the build from the welcome, both
/// frozen, so a trial that fell back to an older bank 0 reads as having
/// gone back. A status this app cannot read, or a pre-revision build's,
/// leaves it not ready.
async fn reading(
    connect: &impl AsyncFn() -> Result<Connection, wellfed_install::Error>,
) -> Option<Reading> {
    let mut session = connect().await.ok()?;
    let build = session.build().to_owned();
    let id = wellfed_install::session::random().ok()?;
    let Answer::Update(view) = session.ask(id, &Op::UpdateState).await.ok()? else {
        return None;
    };
    // A pre-revision build's status is no longer this app's to read: it is
    // never asked, and it is not the served build anyway.
    let ready = if session.skew() == Skew::Legacy {
        false
    } else {
        let id = wellfed_install::session::random().ok()?;
        match session.ask(id, &Op::Status).await.ok()? {
            Answer::Status(status) => status
                .feeder()
                .is_some_and(|feeder| feeder.readiness == Readiness::Ready),
            _ => false,
        }
    };
    Some(Reading { build, view, ready })
}

/// Whether this app offers its image to a feeder.
pub enum UpdateOffer {
    /// Newer than what the feeder confirmed, and bank 0 allows it: offer
    /// it. The only value an update starts from.
    Newer { build: String, version: u32 },
    /// The feeder runs this build already.
    Current,
    /// The feeder runs newer firmware than this app carries: update the app.
    Older,
    /// Bank 1 refuses every update until bank 0 checks itself in.
    Blocked { reason: Bank0Blocked },
    /// The feeder's update state is not read, or its read failed: not this
    /// build and not ahead of this app, but nothing is offered until the
    /// state says what it confirmed.
    Unread,
}

/// Whether to offer the image `build` at `version` to a feeder that runs
/// `running` (`None` before its status is read) at `skew`, with `update`
/// its update state (`None` before it is read, or when its read failed):
/// the rule every client shares.
#[frb(sync)]
pub fn update_offer(
    build: String,
    version: u32,
    running: Option<String>,
    skew: Skew,
    update: Option<UpdateInfo>,
) -> UpdateOffer {
    let candidate = Candidate {
        build: &build,
        version,
    };
    let gate = update.map(|update| Gate {
        confirmed: update.min_version,
        bank0: update.bank0,
    });
    match offer::offer(candidate, running.as_deref(), skew, gate) {
        Offer::Newer => UpdateOffer::Newer { build, version },
        Offer::Current => UpdateOffer::Current,
        Offer::Older => UpdateOffer::Older,
        Offer::Blocked(reason) => UpdateOffer::Blocked { reason },
        Offer::Unread => UpdateOffer::Unread,
    }
}

/// What a read settles of an update whose end was unknown.
pub enum UnknownEnd {
    /// Nothing is under way and the new build does not run: the owner may
    /// decide again.
    Decide,
    /// The new build runs and has confirmed itself.
    Confirmed,
    /// Still unknown: look again later.
    Unknown,
}

/// What `update` (the update state a fresh read gave) and the feeder's
/// current status settle of the update that sent `served` at `version`
/// (`None` when this app does not know what it sent): `running` is the
/// status's build and `ready` whether it can feed, `None` and `false` while
/// the status is not current. Idle alone settles nothing: a bank 1 on trial
/// reads idle too, before its verdict.
#[frb(sync)]
pub fn unknown_end(
    served: Option<String>,
    version: u32,
    update: UpdateInfo,
    running: Option<String>,
    ready: bool,
) -> UnknownEnd {
    if update.phase != UpdatePhase::Idle {
        return UnknownEnd::Unknown;
    }
    if update.bank == 0 {
        return UnknownEnd::Decide;
    }
    let (Some(served), Some(running)) = (served, running) else {
        return UnknownEnd::Unknown;
    };

    let restarted = Restarted {
        build: &running,
        bank: update.bank,
        confirmed: update.min_version,
        ready,
    };
    match confirmation(&served, version, restarted) {
        Confirmation::Confirmed => UnknownEnd::Confirmed,
        // Bank 0 answered above; another build is not this update's to
        // settle.
        Confirmation::FellBack | Confirmation::WrongBuild | Confirmation::Pending => {
            UnknownEnd::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use wellfed_model::Bank0Check;

    use super::*;

    /// The check comes before the first connection and the first ask.
    #[test]
    fn an_update_without_a_pinned_image_sends_nothing() {
        let empty = tempfile::tempdir().unwrap();
        let keys = Keypair::from_secret([7; 32]);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let end = runtime.block_on(update(
            &keys,
            [0; 32],
            None,
            &empty.path().to_string_lossy(),
            Arc::new(Asks::new()),
            &|_| panic!("nothing to report"),
        ));

        assert!(matches!(end, UpdateEnd::Failed { .. }));
    }

    /// A second run of one update offers nothing: only the first may.
    #[test]
    fn an_update_offers_once() {
        let key = ClientKey::from_secret([7; 32]);
        let update = FirmwareUpdate::new(&key, vec![0; 32], None, String::new()).unwrap();

        assert!(update.first_run());
        assert!(!update.first_run());
    }

    /// An update state read idle on `bank`, with floor `floor` and bank 0's
    /// check `bank0`.
    fn state(bank: u8, floor: u32, bank0: Bank0Check) -> UpdateInfo {
        UpdateInfo {
            phase: UpdatePhase::Idle,
            bank,
            min_version: floor,
            version: 0,
            image_len: 0,
            written: 0,
            bank0,
        }
    }

    /// The bridge hands Dart the shared rule's verdict, the build and version
    /// with an offer, and decides what runs and the skew say before the
    /// update state is read.
    #[test]
    fn the_offer_is_the_shared_rules() {
        let offer = |running: Option<&str>, skew, update| {
            update_offer("BB22".into(), 5, running.map(str::to_owned), skew, update)
        };
        let running = Some("aa11");

        assert!(matches!(
            offer(running, Skew::Same, Some(state(1, 4, Bank0Check::Match))),
            UpdateOffer::Newer { build, version: 5 } if build == "BB22"
        ));
        assert!(matches!(
            offer(
                Some("bb22"),
                Skew::Same,
                Some(state(1, 4, Bank0Check::Match))
            ),
            UpdateOffer::Current
        ));
        assert!(matches!(
            offer(running, Skew::Same, Some(state(1, 5, Bank0Check::Match))),
            UpdateOffer::Older
        ));
        assert!(matches!(
            offer(
                running,
                Skew::FeederAhead,
                Some(state(1, 0, Bank0Check::Match))
            ),
            UpdateOffer::Older
        ));
        let unrecorded = Bank0Check::Blocked(Bank0Blocked::Unrecorded);
        assert!(matches!(
            offer(running, Skew::Same, Some(state(1, 4, unrecorded))),
            UpdateOffer::Blocked {
                reason: Bank0Blocked::Unrecorded
            }
        ));
        assert!(matches!(
            offer(running, Skew::Same, Some(state(1, 4, Bank0Check::Unknown))),
            UpdateOffer::Blocked {
                reason: Bank0Blocked::Unknown
            }
        ));

        // Before the update state reads, or when its read failed.
        assert!(matches!(
            offer(running, Skew::Same, None),
            UpdateOffer::Unread
        ));
        assert!(matches!(offer(None, Skew::Same, None), UpdateOffer::Unread));
        assert!(matches!(
            offer(Some("bb22"), Skew::Same, None),
            UpdateOffer::Current
        ));
        assert!(matches!(
            offer(running, Skew::FeederAhead, None),
            UpdateOffer::Older
        ));
    }

    /// An unknown end settles only on bank 0 idle (decide again), or on the
    /// served build running on bank 1 with its version confirmed or ready to
    /// feed; anything else stays unknown.
    #[test]
    fn an_unknown_end_settles_by_the_shared_rule() {
        let end = |served: Option<&str>, update, running: Option<&str>, ready| {
            unknown_end(
                served.map(str::to_owned),
                5,
                update,
                running.map(str::to_owned),
                ready,
            )
        };
        let served = Some("bb22");
        let unchecked = Bank0Check::Unchecked;

        let mut receiving = state(1, 5, unchecked);
        receiving.phase = UpdatePhase::Receiving;
        assert!(matches!(
            end(served, receiving, served, true),
            UnknownEnd::Unknown
        ));
        for known in [served, None] {
            assert!(matches!(
                end(known, state(0, 4, unchecked), None, false),
                UnknownEnd::Decide
            ));
        }

        assert!(matches!(
            end(served, state(1, 5, unchecked), Some("BB22"), false),
            UnknownEnd::Confirmed
        ));
        assert!(
            matches!(
                end(served, state(1, 4, unchecked), served, true),
                UnknownEnd::Confirmed
            ),
            "ready, its floor below: a reinstall the feeder confirmed before"
        );
        assert!(matches!(
            end(served, state(1, 4, unchecked), served, false),
            UnknownEnd::Unknown
        ));

        assert!(
            matches!(
                end(served, state(1, 5, unchecked), Some("aa11"), true),
                UnknownEnd::Unknown
            ),
            "another build is not this update's to settle"
        );
        assert!(matches!(
            end(None, state(1, 5, unchecked), served, true),
            UnknownEnd::Unknown
        ));
        assert!(matches!(
            end(served, state(1, 5, unchecked), None, true),
            UnknownEnd::Unknown
        ));
    }

    /// Bank 1 running some other build than the one activated is not a
    /// fallback: another update came between, and the end says so rather
    /// than that the trial failed. The simulator's bank 1 runs its own build;
    /// the same reads of that build confirm it.
    #[test]
    fn another_build_on_bank_1_ends_on_the_wrong_build() {
        use wellfed_feeder_sim::banks::BANK1_BUILD;
        use wellfed_feeder_sim::flash::Flash;
        use wellfed_feeder_sim::harness::{Running, local};

        const OWNER: [u8; 32] = [7; 32];
        local(async {
            let keys = Keypair::from_secret(OWNER);
            let sim = Running::start(&Flash::memory(), Some(*keys.public())).await;
            let (identity, address) = (sim.identity, sim.address);
            let connect = async || -> Result<Connection, wellfed_install::Error> {
                let ephemeral = wellfed_install::session::random()?;
                Ok(Connection::open(&keys, identity, address, ephemeral).await?)
            };

            let another = hex::Lower(&[0xcd; 32]).to_string();
            let end = after_restart(&connect, &another, 1).await;
            assert!(matches!(end, UpdateEnd::WrongBuild));
            let running = hex::Lower(&BANK1_BUILD).to_string();
            let end = after_restart(&connect, &running, 0).await;
            assert!(matches!(end, UpdateEnd::Confirmed));

            sim.stop().await;
        });
    }
}
