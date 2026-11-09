//! [`Sim`], one simulated feeder: the product's loop
//! ([`wellfed_feeder::feeder::Feeder`]) on the simulator's platform
//! ([`SimPlatform`]), then the tasks the firmware runs beside its loop: the
//! settings writer's turn and the update stage's.
//!
//! A boot first runs the loader's choice and KM0's trial, and the update
//! stage takes its turn after each pass, as every device's do
//! ([`Updates`]): a new build in trial confirms itself at once, unless the
//! banks say it never will, in which case it resets after [`TRIAL_MS`] as
//! the supervisor's deadline makes it. Button presses come from the caller ([`Sim::gesture`]) and wait
//! for the next pass, as the button service's queued gestures do.
//!
//! Nothing here reads a clock: every call takes the time, so tests can run
//! virtual time and the driver can run real time through the same code.

use std::cell::RefCell;
use std::convert::Infallible;
use std::fmt;
use std::net::Ipv4Addr;
use std::rc::Rc;

use tokio::net::{TcpStream, UdpSocket};
use wellfed_core::buttons::Gesture;
use wellfed_core::tone::{PlayResult, Tune};
use wellfed_core_feeder::feed::controller::FeedReason;
use wellfed_device_api::FeederDevice;
use wellfed_device_api::mailbox::{Asked, KeyResult, MealSkip};
pub use wellfed_device_sim::updates::TRIAL_MS;
use wellfed_device_sim::updates::{Booted, Updates};
use wellfed_feeder::feeder::{self, Feeder};
use wellfed_feeder::panel;
use wellfed_hal::ota::update::{self as update, Floor};
use wellfed_hal::trial::Storm;
use wellfed_session::noise::Keypair;

use crate::banks::Banks;
use crate::feeding::SimFeeds;
use crate::flash::Flash;
use crate::platform::{DeviceError, Random, SimPlatform};

/// What one step did, for the simulator's log. No record carries a key or
/// a secret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Record {
    /// The feeder's loop, as its firmware logs it.
    Feeder(feeder::Record<Infallible>),
    /// The boot: the bank the loader chose, whether its build is in trial
    /// and confirmed itself, and what a reset storm left it.
    Boot {
        bank: u8,
        trial: bool,
        confirmed: Option<Floor>,
        storm: Storm,
    },
    /// The writer's turn applied a client store change.
    Keys(KeyResult),
    Update(update::Record),
}

impl Record {
    /// A panel event the loop logged.
    pub fn panel(&self) -> Option<panel::Event> {
        match self {
            Self::Feeder(feeder::Record::Owner(feeder::owner::Record::Panel(event))) => {
                Some(*event)
            }
            _ => None,
        }
    }
}

/// The record lines the feeder would log, and the simulator's own.
impl fmt::Display for Record {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Feeder(record) => write!(f, "{} {record}", record.tag()),
            Self::Boot {
                bank,
                trial,
                confirmed,
                storm,
            } => write!(
                f,
                "boot bank={bank} trial={} confirmed={confirmed:?} storm={}",
                u8::from(*trial),
                storm.name()
            ),
            Self::Keys(result) => write!(f, "keys result={result}"),
            Self::Update(record) => {
                write!(f, "{} ", record.tag())?;
                update::describe(*record, f)
            }
        }
    }
}

pub struct Sim {
    feeder: Box<Feeder<SimPlatform>>,
    /// The feeding owner, stepped before each pass as the firmware's is.
    feeds: SimFeeds,
    booted: bool,
    /// The boot's bank and trial, and the update stage's turns.
    updates: Updates,
    /// What the boot did, for the first pass's log.
    boot: Record,
}

impl fmt::Debug for Sim {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sim")
            .field("platform", self.feeder.platform())
            .finish_non_exhaustive()
    }
}

impl Sim {
    /// The feeder as it boots from `flash` at `now_ms`, for the
    /// `boot_count`th time: the panel locked, the clock invalid, no sessions.
    /// With `mdns`, it advertises the device API there as the given address.
    pub fn open(
        flash: &Rc<Flash>,
        banks: &Rc<RefCell<Banks>>,
        update_key: Option<[u8; 32]>,
        random: Random,
        now_ms: u32,
        boot_count: u32,
        mdns: Option<(UdpSocket, Ipv4Addr)>,
    ) -> Result<Self, DeviceError> {
        let (updates, booted) = Updates::open(flash, banks, update_key, now_ms);
        let Booted {
            bank,
            trial,
            confirmed,
            storm,
            build,
        } = booted;

        let mut platform = SimPlatform::open(flash, random, boot_count, bank, build, mdns)?;
        platform.attach_banks(banks);
        Ok(Self {
            feeder: Box::new(Feeder::new(platform)),
            feeds: SimFeeds::open(flash),
            booted: false,
            updates,
            boot: Record::Boot {
                bank,
                trial,
                confirmed,
                storm,
            },
        })
    }

    /// The feeder's identity public key, as the client store holds it.
    pub fn identity(&self) -> Result<[u8; 32], DeviceError> {
        let secret = self
            .feeder
            .platform()
            .identity()
            .ok_or(DeviceError::NoIdentity)?;
        Ok(*Keypair::from_secret(*secret).public())
    }

    pub fn platform(&mut self) -> &mut SimPlatform {
        self.feeder.platform_mut()
    }

    /// A client's `play`, as the device API asks it at `now_ms`. The
    /// simulator has no speaker, so one that reaches it answers `Muted`, and
    /// `Busy` means the loop refused it first.
    pub fn play(&mut self, now_ms: u32, tune: Tune) -> PlayResult {
        self.feeder.device(now_ms, &mut self.feeds).play(tune)
    }

    /// The client `by`'s skip of slot `slot`'s meal due at `due_s`, or its
    /// undo, as the device API asks it at `now_ms`: checked and queued for
    /// the writer, or refused.
    pub fn skip_meal(
        &mut self,
        now_ms: u32,
        slot: u8,
        due_s: u32,
        skip: Asked<MealSkip>,
    ) -> Result<(), wellfed_proto::api::Refusal> {
        self.feeder
            .device(now_ms, &mut self.feeds)
            .skip_meal(slot, due_s, skip, now_ms)
    }

    /// The epoch a session connecting at `now_ms` is welcomed under, as the
    /// device API asks it: `None` while none is welcomed.
    pub fn welcome_epoch(&mut self, now_ms: u32) -> Option<u32> {
        let device = self.feeder.device(now_ms, &mut self.feeds);
        wellfed_device_api::Device::epoch(&device)
    }

    /// Whether the feeder asked for a restart, taken once: the driver drops
    /// every connection and opens the feeder again from its flash, after
    /// the lifetime totals are saved.
    pub fn take_reboot(&mut self) -> bool {
        let reboot = self.platform().take_reboot();
        if reboot {
            self.updates.restarting();
            self.platform().save_stats(true);
        }
        reboot
    }

    /// One gesture from the buttons, for the next pass: pairing takes it
    /// first (a code being keyed in, the recovery or setup hold), otherwise
    /// the panel.
    pub fn gesture(&mut self, gesture: Gesture) {
        self.platform().gesture_in(gesture);
    }

    /// Hold a feed in flight where it is, or let it go on: a motor that
    /// stopped moving, for a test to act while a feed runs. A restart
    /// lets go.
    pub fn stall_feeds(&mut self, stalled: bool) {
        self.feeds.stall(stalled);
    }

    /// End the next feed on `reason` once it has driven a portion.
    pub fn fault_feed(&mut self, reason: FeedReason) {
        self.feeds.fault(reason);
    }

    /// Fill the hopper (`true`) or empty it, as the food beam next reads it.
    /// A restart fills it again.
    pub fn fill_hopper(&mut self, food: bool) {
        self.feeds.fill(food);
    }

    /// The food beam's latest reading (`true` with food), once one is taken.
    pub fn food(&self) -> Option<bool> {
        wellfed_feeder::feeder::Feeds::food(&self.feeds)
    }

    /// A client's cancel of the feed numbered `feed`: whether it found it.
    pub fn cancel_feed(&self, feed: u32) -> bool {
        wellfed_feeder::feeder::Feeds::cancel(&self.feeds, feed)
    }

    /// Hold a storage write in the way of a feed's claim, or end it.
    pub fn hold_write(&mut self, held: bool) {
        self.feeds.hold_write(held);
    }

    /// Whether a feed's claim on the outputs was taken and has not ended.
    pub fn feed_outputs_claimed(&self) -> bool {
        self.feeds.outputs_claimed()
    }

    /// Refuse every feed `Busy` though readiness says ready, as an owner
    /// that lost a race does, or stop.
    pub fn refuse_feeds(&mut self, refusing: bool) {
        self.feeds.refuse(refusing);
    }

    /// Whether the serve loop should accept connections.
    pub(crate) fn listening(&self) -> bool {
        self.feeder.platform().listening()
    }

    /// A connection the serve loop accepted, for the next pass.
    pub(crate) fn accepted(&mut self, stream: TcpStream) {
        self.platform().accepted(stream);
    }

    /// The serve loop's accept failed.
    pub(crate) fn accept_failed(&mut self) {
        self.platform().accept_failed();
    }

    /// One pass of the loop at `now_ms`, then the writer's turn and the
    /// update stage's. The delay until the next pass.
    pub fn pass(&mut self, now_ms: u32, mut record: impl FnMut(Record)) -> u32 {
        // Every record line goes into the log tail too, as the firmware's
        // `log` copies what it prints.
        let tail = self.feeder.platform().log_tail();
        let mut record = |r: Record| {
            tail.push(r.to_string().as_bytes());
            record(r);
        };
        if !self.booted {
            self.booted = true;
            record(self.boot);
        }

        let supply = self.feeder.platform().sim_supply();
        self.feeds.sense(now_ms, supply);
        let (fed, feed_delay) = self.feeds.step(now_ms);
        if let Some(body) = fed {
            self.feeder.record_outcome(now_ms, body);
        }
        let delay = self
            .feeder
            .pass(now_ms, &mut self.feeds, |r| record(Record::Feeder(r)));
        self.platform().answer_read();
        let delay = feed_delay.map_or(delay, |feed| delay.min(feed));
        if let Some(result) = self.platform().write() {
            record(Record::Keys(result));
        }
        let updating = self.update(now_ms, &mut record);
        if updating { 1 } else { delay }
    }

    /// The update stage's turn, as the service task gives it; whether it
    /// has work waiting.
    fn update(&mut self, now_ms: u32, record: &mut impl FnMut(Record)) -> bool {
        let turn = self.updates.turn(now_ms);
        if let Some(event) = turn.record {
            record(Record::Update(event));
        }
        if turn.restart {
            self.platform().request_reboot();
        }
        turn.busy
    }
}
