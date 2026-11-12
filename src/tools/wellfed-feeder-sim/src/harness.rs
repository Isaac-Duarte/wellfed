//! A simulator running on the current [`tokio::task::LocalSet`], for the
//! tests of every crate that talks to a feeder: this one's, and the
//! server's service and end-to-end tests. Its randomness is a counter, so
//! a test's identities and invite secrets repeat from run to run. The
//! runtime, the relay and the generator are every device's
//! (`wellfed_device_sim::harness`).

pub mod newer;
mod peer;
pub mod pre_revision;

use std::cell::RefCell;
use std::net::{Ipv4Addr, SocketAddr};
use std::rc::Rc;
use std::sync::atomic::{AtomicU8, Ordering};

use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use wellfed_core::clock::wall::{Origin, Sync};
use wellfed_device_api::clients::{Label, Role};
use wellfed_device_api::setup::{MAX_MESSAGE, Received, Responder};

use wellfed_board::storage;
use wellfed_device::store::events::Read;
use wellfed_device::store::traces::{TraceError, TraceRing};
use wellfed_feeder::feeder::Platform;
use wellfed_proto::api::{EventBody, TraceSummary};
use wellfed_proto::setup::{Enrolled, Reply, WifiSaved};
use wellfed_session::noise::Keypair;
use wellfed_session::setup_code::Code;

use wellfed_device_sim::harness::Alone;
// The device sim's test helpers, as this simulator's tests name them.
pub use wellfed_device_sim::harness::{Relay, counter, local};

/// Make every program into the feeder's own settings record on `flash`
/// fail from now on (`true`), or none: a save of the settings then lands
/// the device's part alone and leaves the two records apart, so the
/// feeder's settings are read-only with no generation.
pub fn fail_feeder_settings(flash: &Flash, failing: bool) {
    flash.fail_programs(failing.then_some(storage::FEEDER_SETTINGS));
}

/// Make every program into the meal call's store on `flash` fail from now
/// on (`true`), or none: the next chunk an upload writes is lost with the
/// clip it was part of, and the store refuses later chunks `NotWritable`
/// until the feeder restarts.
pub fn fail_call(flash: &Flash, failing: bool) {
    flash.fail_programs(failing.then_some(storage::CALL));
}

use crate::banks::Banks;
use crate::console::{self, ConsoleError};
use crate::flash::{Flash, Region};
use crate::serve::{self, Control, Event, Feeder};
use crate::sim::{Record, Sim};

/// Run `sim`'s loop in virtual time for `ms` from the uptime `now`, a pass at
/// least every second, handing `record` the uptime and each record.
pub fn run_loop(sim: &mut Sim, now: &mut u32, ms: u64, mut record: impl FnMut(u32, Record)) {
    let end = u64::from(*now) + ms;
    while u64::from(*now) < end {
        let at = *now;
        let delay = sim.pass(at, |logged| record(at, logged));
        let step = u64::from(delay.clamp(1, 1000)).min(end - u64::from(at));
        *now += u32::try_from(step.max(1)).expect("a step is at most a second");
    }
}

/// An owner's sync at the uptime `now`: the wall clock reads `unix_ms`, at UTC
/// unless a zone rule is set.
pub fn owner_sync(sim: &mut Sim, now: u32, unix_ms: u64) {
    let sync = Sync {
        unix_ms,
        offset_s: 0,
        next_change_s: 0,
        next_offset_s: 0,
    };
    sim.platform()
        .time_sync(now, sync, Origin::Owner)
        .expect("a test's sync is a valid time");
}

/// Every event `sim`'s ring holds, oldest first.
pub fn events(sim: &mut Sim) -> Vec<EventBody> {
    let mut bodies = Vec::new();
    let mut after = None;
    while let Read::Record(record) = sim.platform().ring_after(after) {
        after = Some(record.seq);
        bodies.extend(wellfed_proto::decode_padded::<EventBody>(&record.body));
    }
    bodies
}

/// Append a feed's sealed trace to `flash`'s trace ring, as the feeder's
/// settings writer does once the feed is over: the simulator has no motor
/// to trace, but serves what its ring holds.
pub fn store_trace(
    flash: &Rc<Flash>,
    summary: &TraceSummary,
    body: &[u8],
) -> Result<u32, TraceError> {
    let mut ring = TraceRing::default();
    ring.open(
        Region::new(flash, storage::TRACES),
        storage::TRACES.count as usize,
    );
    ring.append(summary, body)
}

/// A running simulator.
#[derive(Debug)]
pub struct Running {
    pub address: SocketAddr,
    /// The feeder's identity key, as its first boot made it.
    pub identity: [u8; 32],
    /// Everything the simulator logs, from its start.
    pub events: mpsc::UnboundedReceiver<Event>,
    control: mpsc::UnboundedSender<Control>,
    done: JoinHandle<()>,
    _alone: Alone,
}

impl Running {
    /// The simulator on `flash` at a free localhost port, with `owner`'s
    /// public key enrolled as an owner at every boot when given.
    pub async fn start(flash: &Rc<Flash>, owner: Option<[u8; 32]>) -> Self {
        Self::start_at(flash, owner, SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await
    }

    /// As [`Self::start`], at `address`: a restart on the port a client
    /// already knows.
    pub async fn start_at(flash: &Rc<Flash>, owner: Option<[u8; 32]>, address: SocketAddr) -> Self {
        Self::start_with(flash, &Banks::new(), None, owner, address).await
    }

    /// As [`Self::start_at`], on `banks` (kept across starts) and trusting
    /// `update_key` for updates.
    pub async fn start_with(
        flash: &Rc<Flash>,
        banks: &Rc<RefCell<Banks>>,
        update_key: Option<[u8; 32]>,
        owner: Option<[u8; 32]>,
        address: SocketAddr,
    ) -> Self {
        Self::start_ruled(flash, banks, update_key, owner, address, true).await
    }

    /// As [`Self::start_with`], with the alert rules on or off: a control for
    /// the rules changing nothing about a meal.
    pub async fn start_ruled(
        flash: &Rc<Flash>,
        banks: &Rc<RefCell<Banks>>,
        update_key: Option<[u8; 32]>,
        owner: Option<[u8; 32]>,
        address: SocketAddr,
        rules: bool,
    ) -> Self {
        let alone = Alone::take();
        let listener = TcpListener::bind(address)
            .await
            .expect("bind the simulator's port");
        let address = listener
            .local_addr()
            .expect("a bound listener has an address");
        // Each start its own seed, so two simulators in one test have two
        // identities; a restart on the same flash keeps the stored one.
        static SEED: AtomicU8 = AtomicU8::new(0x5a);
        let seed = SEED.fetch_add(1, Ordering::Relaxed);
        let feeder = Feeder {
            flash: Rc::clone(flash),
            banks: Rc::clone(banks),
            update_key,
            random: Box::new(counter(seed)),
            prepare: Box::new(move |sim| {
                sim.platform().set_alerts_on(rules);
                match owner {
                    Some(key) => {
                        let label = Label::new(b"test-owner").expect("a 10-byte label");
                        sim.platform().enroll(key, label, Role::Owner)
                    }
                    None => Ok(()),
                }
            }),
        };
        let (control, commands) = mpsc::unbounded_channel();
        let (sink, mut events) = mpsc::unbounded_channel();
        let done = tokio::task::spawn_local(async move {
            serve::serve(feeder, listener, None, commands, |event| {
                let _ = sink.send(event);
            })
            .await
            .expect("the simulator serves until told to quit");
        });

        let identity = loop {
            match events.recv().await {
                Some(Event::Started { identity }) => break identity,
                Some(_) => {}
                None => panic!("the simulator stopped before it started"),
            }
        };
        Self {
            address,
            identity,
            events,
            control,
            done,
            _alone: alone,
        }
    }

    pub fn control(&self, control: Control) {
        // The simulator only stops on `Quit`, which takes `self`.
        let _ = self.control.send(control);
    }

    /// A console line, as the operator would type it.
    pub fn type_line(&self, line: &str) -> Result<(), ConsoleError> {
        for control in console::parse(line)? {
            self.control(control);
        }
        Ok(())
    }

    /// Quit, and wait for the sockets to close.
    pub async fn stop(self) {
        self.control(Control::Quit);
        self.done.await.expect("the simulator task ends cleanly");
    }
}

/// A simulator on a thread of its own, in real time, for a test whose
/// client runs on a multi-threaded runtime or a blocking thread. Its
/// thread holds the process, as any simulator's does, until it is dropped,
/// which stops it.
#[derive(Debug)]
pub struct Threaded {
    pub address: SocketAddr,
    pub identity: [u8; 32],
    control: std::sync::mpsc::Sender<Control>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Threaded {
    /// A fresh simulator with `owner`'s public key enrolled as an owner.
    pub fn start(owner: [u8; 32]) -> Self {
        let (started, starting) = std::sync::mpsc::channel();
        let (control, controls) = std::sync::mpsc::channel::<Control>();
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a current-thread runtime builds");
            runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
                let sim = Running::start(&Flash::memory(), Some(owner)).await;
                let _ = started.send((sim.address, sim.identity));
                loop {
                    match controls.try_recv() {
                        Ok(Control::Quit) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            break;
                        }
                        Ok(other) => sim.control(other),
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                        }
                    }
                }
                sim.stop().await;
            }));
        });
        let (address, identity) = starting
            .recv()
            .expect("the simulator thread reports its address");

        Self {
            address,
            identity,
            control,
            thread: Some(thread),
        }
    }

    pub fn control(&self, control: Control) {
        let _ = self.control.send(control);
    }
}

impl Drop for Threaded {
    fn drop(&mut self) {
        // Stopped and joined, so its thread's hold is gone before the next
        // test's simulator starts.
        let _ = self.control.send(Control::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A feeder in setup mode with no radio around it, for a client's test to play
/// over whatever link it fakes: its own responder, which asks for the code,
/// takes it, and answers that it saved the Wi-Fi and already holds the owner.
pub struct SetupFeeder {
    responder: Responder,
    out: Box<[u8; MAX_MESSAGE]>,
}

impl SetupFeeder {
    /// A feeder answering as `identity`.
    pub fn new(identity: &Keypair) -> Self {
        Self {
            responder: Responder::new(identity, [4; 32]).expect("a fixed ephemeral is a valid key"),
            out: Box::new([0; MAX_MESSAGE]),
        }
    }

    /// The feeder's answer to one whole message from the client, unframed. A
    /// client that breaks the protocol fails the test here.
    pub fn hear(&mut self, message: &[u8]) -> Vec<u8> {
        let len = match self
            .responder
            .receive(message, &mut self.out[..])
            .expect("the client keeps to the setup protocol")
        {
            Received::Send(len) => len,
            Received::Offer(_) => self.reply(&Reply::EnterCode { seconds: 60 }),
            Received::Proven { .. } => self.reply(&Reply::Done {
                enrolled: Enrolled::Already,
                wifi: WifiSaved::Saved,
            }),
        };
        self.out[..len].to_vec()
    }

    /// The feeder's proof of `code`, as if someone had pressed it on its
    /// panel, unframed.
    pub fn prove(&mut self, code: Code) -> Vec<u8> {
        let len = self
            .responder
            .prove(code, [6; 32], &mut self.out[..])
            .expect("the feeder has asked for the code");
        self.out[..len].to_vec()
    }

    fn reply(&mut self, reply: &Reply) -> usize {
        self.responder
            .reply(reply, &mut self.out[..])
            .expect("a reply fits a setup message")
    }
}
