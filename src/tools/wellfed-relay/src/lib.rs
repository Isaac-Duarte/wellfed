//! The Wellfed Bench board's switched outputs: feeder power, the loader
//! strap and the UART lines.
//!
//! The bench commands drive them through [`Relay`]. The board answers the
//! postcard-rpc requests of `wellfed-bench-icd` on a vendor USB interface,
//! found by its serial string ([`RpcBoard`]). A set returns only once the
//! board confirmed the new state, and a refusal is an error, so a sequence
//! stops rather than assuming an output moved. `FakeBoard` (the `fake`
//! feature) stands in for the board in tests.
//!
//! The crate knows the board and the channel wiring, nothing of the bench's
//! sequences: those belong to `wellfed-bench`.

#![forbid(unsafe_code)]

use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc as sync_mpsc;
use std::time::Duration;

use postcard_rpc::header::VarSeqKind;
use postcard_rpc::host_client::{HostClient, HostErr};
use postcard_rpc::standard_icd::{ERROR_PATH, WireError};
use postcard_rpc::{Endpoint, postcard_schema::Schema};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::mpsc;
use wellfed_bench_icd::{Bootsel, Channel, GetStatus, Refusal, Set, SetChannel};
pub use wellfed_bench_icd::{Source, Status, Telemetry};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No Wellfed Bench board with this serial is on the bus.
    #[error("No Wellfed Bench board with serial {serial} is connected")]
    NotFound { serial: String },
    /// The board is there but its control interface would not open: held
    /// by another process, or no permission (the udev rule).
    #[error("Opening the Wellfed Bench board {serial}: {reason}")]
    Open { serial: String, reason: String },
    /// A RECOVER sequence holds the outputs; the request did not run.
    #[error("The bench board's RECOVER sequence holds the outputs; nothing was switched")]
    Held,
    /// A channel number outside 1–4.
    #[error("No relay channel {channel}")]
    Channel { channel: u8 },
    /// The board did not answer in time. The request may still have run,
    /// so a set's outcome is unknown until the status is read again.
    #[error(
        "The bench board did not answer within {} ms; a switch may still have happened, so read its status",
        TIMEOUT.as_millis()
    )]
    Timeout,
    /// The board acknowledged a set but reported the channel in the other
    /// state: firmware that disagrees with this protocol.
    #[error("The bench board acknowledged relay {channel} but reports it {}", if *.on { "on" } else { "off" })]
    Unconfirmed { channel: u8, on: bool },
    /// The board answered with an error, or the link failed. An
    /// `UnknownKey` wire error is firmware built from another protocol.
    #[error("Bench board request failed: {0:?}")]
    Rpc(#[from] HostErr<WireError>),
    /// The link to the board is gone (unplugged, or the worker ended).
    #[error("The bench board link is closed")]
    Closed,
    /// [`Relay::detached`]: this command does not open the board.
    #[error("This command does not open the bench board")]
    Detached,
    #[error("Listing USB devices: {0}")]
    Usb(#[source] std::io::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Switches the feeder's USB 5V.
pub const POWER: u8 = Channel::Power as u8;
/// Holds feeder TX to GND through the 4.7 kΩ strap.
pub const STRAP: u8 = Channel::Strap as u8;
/// The UART bridge's two lines, each through one buffer: 3 carries the
/// bridge's TXD to the feeder's RX, 4 the feeder's TX to the bridge (the
/// strap stays on the feeder side). A powered bridge holds both lines high,
/// which can back-power a feeder that is off, so they open while it is.
pub const UART: [u8; 2] = [Channel::UartTx as u8, Channel::UartRx as u8];

/// What a set asks of a channel's relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    On,
    Off,
}

impl Switch {
    pub fn is_on(self) -> bool {
        self == Self::On
    }
}

/// A board with the bench's four switches.
pub trait Board: Send {
    /// Switch `channel`, returning only once the board acknowledged it.
    fn set(&mut self, channel: u8, switch: Switch) -> Result<()>;
    /// Everything the board reports in one answer: the channels, its power
    /// cut count and the feeder's power telemetry.
    fn status(&mut self) -> Result<Status>;

    /// The channel's state as the board reports it.
    fn get(&mut self, number: u8) -> Result<bool> {
        let channel = channel(number)?;
        Ok(self.status()?.channels.get(channel))
    }

    /// The feeder power cuts the board has made, whoever asked for them.
    /// The RECOVER button cycles the power by itself, which the channels
    /// alone cannot show once the feeder is back on.
    fn power_cuts(&mut self) -> Result<u32> {
        Ok(self.status()?.cuts)
    }
}

/// The relay board a sequence drives.
pub struct Relay {
    board: Box<dyn Board>,
}

impl Relay {
    pub fn of(board: Box<dyn Board>) -> Self {
        Self { board }
    }

    /// The board with this USB serial string.
    pub fn open(serial: &str) -> Result<Self> {
        Ok(Self::of(Box::new(RpcBoard::open(serial)?)))
    }

    /// No board: every command fails. For a command that never switches a
    /// relay, so it does not hold the board while it runs.
    pub fn detached() -> Self {
        Self::of(Box::new(Detached))
    }

    pub fn set(&mut self, channel: u8, switch: Switch) -> Result<()> {
        self.board.set(channel, switch)
    }

    pub fn get(&mut self, channel: u8) -> Result<bool> {
        self.board.get(channel)
    }

    pub fn power_cuts(&mut self) -> Result<u32> {
        self.board.power_cuts()
    }

    pub fn status(&mut self) -> Result<Status> {
        self.board.status()
    }
}

struct Detached;

impl Board for Detached {
    fn set(&mut self, _: u8, _: Switch) -> Result<()> {
        Err(Error::Detached)
    }

    fn status(&mut self) -> Result<Status> {
        Err(Error::Detached)
    }
}

/// The board's USB IDs: pid.codes' test VID:PID, which other prototypes
/// share, so the manufacturer string is matched too.
const VID: u16 = 0x1209;
const PID: u16 = 0x0001;
const MANUFACTURER: &str = "Wellfed";

/// How long a request may wait for its reply. The board answers within a
/// millisecond; a switch that is refused is refused at once.
const TIMEOUT: Duration = Duration::from_secs(2);

fn is_bench(device: &nusb::DeviceInfo) -> bool {
    device.vendor_id() == VID
        && device.product_id() == PID
        && device.manufacturer_string() == Some(MANUFACTURER)
}

/// Whether a board with this serial is on the bus, without opening it.
pub fn connected(serial: &str) -> Result<bool> {
    let mut devices = nusb::list_devices().map_err(Error::Usb)?;
    Ok(devices.any(|device| is_bench(&device) && device.serial_number() == Some(serial)))
}

type Job =
    Box<dyn FnOnce(HostClient<WireError>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

/// The Wellfed Bench board over postcard-rpc.
///
/// postcard-rpc's client is async; the bench's sequences are not. One
/// worker thread owns the client and its runtime, and each call waits for
/// its reply, so a call works the same from a plain thread or from inside
/// the bench server's runtime.
pub struct RpcBoard {
    jobs: mpsc::UnboundedSender<Job>,
}

impl RpcBoard {
    /// Opens the board with this USB serial string. Its control interface
    /// is claimed for as long as this lives, so a second process cannot
    /// share the board.
    pub fn open(serial: &str) -> Result<Self> {
        let serial = serial.to_owned();
        Self::spawn(move || {
            let devices = nusb::list_devices().map_err(Error::Usb)?;
            let device = devices
                .filter(is_bench)
                .find(|device| device.serial_number() == Some(serial.as_str()))
                .ok_or_else(|| Error::NotFound {
                    serial: serial.clone(),
                })?;
            // The only vendor-class interface is the control interface.
            let interface = device
                .interfaces()
                .find(|interface| interface.class() == 0xFF)
                .map(|interface| usize::from(interface.interface_number()))
                .ok_or_else(|| Error::Open {
                    serial: serial.clone(),
                    reason: "no control interface: reflash the bench board".into(),
                })?;
            HostClient::try_from_nusb_and_interface(
                &device,
                interface,
                ERROR_PATH,
                8,
                VarSeqKind::Seq2,
            )
            .map_err(|reason| Error::Open { serial, reason })
        })
    }

    /// Starts the worker, which builds its client with `connect` inside its
    /// runtime: the client spawns its I/O tasks there.
    pub fn spawn(
        connect: impl FnOnce() -> Result<HostClient<WireError>> + Send + 'static,
    ) -> Result<Self> {
        let (jobs, mut queue) = mpsc::unbounded_channel::<Job>();
        let (ready, opened) = sync_mpsc::sync_channel(1);

        std::thread::Builder::new()
            .name("bench-board".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready.send(Err(Error::Usb(error)));
                        return;
                    }
                };
                runtime.block_on(async move {
                    let client = match connect() {
                        Ok(client) => client,
                        Err(error) => {
                            let _ = ready.send(Err(error));
                            return;
                        }
                    };
                    let _ = ready.send(Ok(()));

                    while let Some(job) = queue.recv().await {
                        job(client.clone()).await;
                    }
                    client.close();
                });
            })
            .map_err(Error::Usb)?;

        opened.recv().map_err(|_| Error::Closed)??;
        Ok(Self { jobs })
    }

    fn call<E>(&self, request: E::Request) -> Result<E::Response>
    where
        E: Endpoint,
        E::Request: Serialize + Schema + Send + Sync + 'static,
        E::Response: DeserializeOwned + Schema + Send + 'static,
    {
        let (reply, replied) = sync_mpsc::sync_channel(1);
        let job: Job = Box::new(move |client| {
            Box::pin(async move {
                let answer = tokio::time::timeout(TIMEOUT, client.send_resp::<E>(&request)).await;
                let _ = reply.send(answer);
            })
        });
        self.jobs.send(job).map_err(|_| Error::Closed)?;

        match replied.recv() {
            Ok(Ok(Ok(response))) => Ok(response),
            Ok(Ok(Err(HostErr::Closed))) | Err(_) => Err(Error::Closed),
            Ok(Ok(Err(error))) => Err(Error::Rpc(error)),
            Ok(Err(_elapsed)) => Err(Error::Timeout),
        }
    }

    /// Cuts the feeder's power and reboots the board into its UF2
    /// bootloader, 100 ms after it answers.
    pub fn bootsel(&mut self) -> Result<()> {
        self.call::<Bootsel>(())?.map_err(refused)
    }
}

fn refused(refusal: Refusal) -> Error {
    match refusal {
        Refusal::Held => Error::Held,
    }
}

fn channel(number: u8) -> Result<Channel> {
    Channel::from_number(number).ok_or(Error::Channel { channel: number })
}

impl Board for RpcBoard {
    fn set(&mut self, number: u8, switch: Switch) -> Result<()> {
        let request = Set {
            channel: channel(number)?,
            on: switch.is_on(),
        };
        let channels = self.call::<SetChannel>(request)?.map_err(refused)?;
        wellfed_formats::require(channels.get(request.channel) == request.on, || {
            Error::Unconfirmed {
                channel: number,
                on: channels.get(request.channel),
            }
        })
    }

    fn status(&mut self) -> Result<Status> {
        self.call::<GetStatus>(())
    }
}

/// A board in memory, for tests: it switches, counts cuts and refuses as
/// told.
#[cfg(any(test, feature = "fake"))]
#[derive(Clone, Default)]
pub struct FakeBoard {
    pub inner: std::sync::Arc<std::sync::Mutex<FakeState>>,
}

#[cfg(any(test, feature = "fake"))]
#[derive(Default)]
pub struct FakeState {
    /// Indexed by channel number; slot 0 is unused.
    pub on: [bool; 5],
    /// Every set, in order: (channel, on).
    pub log: Vec<(u8, bool)>,
    /// Refuse every set, as a RECOVER sequence holding the outputs does.
    pub refuse: bool,
    /// Refuse this one channel only, as a failed relay would.
    pub refuse_channel: Option<u8>,
    /// The power-cut count. Switching CH1 off from on adds one.
    pub cuts: u32,
    /// What the board reports of the feeder's power.
    pub telemetry: Telemetry,
}

#[cfg(any(test, feature = "fake"))]
impl FakeBoard {
    /// A test that panicked holding the lock leaves plain data behind.
    pub fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn state(&self) -> [bool; 4] {
        let s = self.lock();
        [s.on[1], s.on[2], s.on[3], s.on[4]]
    }

    pub fn log(&self) -> Vec<(u8, bool)> {
        self.lock().log.clone()
    }
}

#[cfg(any(test, feature = "fake"))]
impl Board for FakeBoard {
    fn set(&mut self, number: u8, switch: Switch) -> Result<()> {
        channel(number)?;
        let mut s = self.lock();
        if s.refuse || s.refuse_channel == Some(number) {
            return Err(Error::Held);
        }

        let slot = usize::from(number);
        if number == POWER && s.on[slot] && !switch.is_on() {
            s.cuts = s.cuts.wrapping_add(1);
        }
        s.on[slot] = switch.is_on();
        s.log.push((number, switch.is_on()));
        Ok(())
    }

    fn status(&mut self) -> Result<Status> {
        let s = self.lock();
        let mut channels = wellfed_bench_icd::Channels::default();
        for channel in Channel::ALL {
            channels.set(channel, s.on[usize::from(channel.number())]);
        }
        Ok(Status {
            channels,
            cuts: s.cuts,
            telemetry: s.telemetry,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use postcard_rpc::header::VarHeader;
    use postcard_rpc::host_client::test_channels::new_from_channels;
    use postcard_rpc::server::SpawnContext;
    use postcard_rpc::server::impls::test_channels::dispatch_impl::{
        Settings, WireSpawnImpl, WireTxImpl, new_server,
    };
    use postcard_rpc::server::impls::test_channels::{
        ChannelWireRx, ChannelWireSpawn, ChannelWireTx,
    };
    use postcard_rpc::{define_dispatch, server::Dispatch};
    use wellfed_bench_icd::{
        BootselReply, Channels, ENDPOINT_LIST, SetReply, TOPICS_IN_LIST, TOPICS_OUT_LIST, Telemetry,
    };

    use super::*;

    /// The board's side of the protocol, as its firmware answers it.
    #[derive(Default)]
    struct Panel {
        channels: Channels,
        cuts: u32,
        held: bool,
        /// Acknowledge a set without making it, as firmware that disagrees
        /// with the protocol would.
        ignores_sets: bool,
    }

    struct Context(Arc<Mutex<Panel>>);

    impl SpawnContext for Context {
        type SpawnCtxt = ();
        fn spawn_ctxt(&mut self) {}
    }

    define_dispatch! {
        app: PanelDispatcher;
        spawn_fn: spawn_fn;
        tx_impl: WireTxImpl;
        spawn_impl: WireSpawnImpl;
        context: Context;

        endpoints: {
            list: ENDPOINT_LIST;

            | EndpointTy | kind     | handler    |
            | ---------- | ----     | -------    |
            | SetChannel | blocking | set        |
            | GetStatus  | blocking | get_status |
            | Bootsel    | blocking | bootsel    |
        };
        topics_in: {
            list: TOPICS_IN_LIST;

            | TopicTy | kind | handler |
            | ------- | ---- | ------- |
        };
        topics_out: {
            list: TOPICS_OUT_LIST;
        };
    }

    fn set(context: &mut Context, _: VarHeader, request: Set) -> SetReply {
        let mut panel = context.0.lock().unwrap();
        if panel.held {
            return Err(Refusal::Held);
        }
        if panel.channels.cuts_power(request.channel, request.on) {
            panel.cuts = panel.cuts.wrapping_add(1);
        }
        if !panel.ignores_sets {
            panel.channels.set(request.channel, request.on);
        }
        Ok(panel.channels)
    }

    fn get_status(context: &mut Context, _: VarHeader, (): ()) -> Status {
        let panel = context.0.lock().unwrap();
        Status {
            channels: panel.channels,
            cuts: panel.cuts,
            telemetry: Telemetry::new(),
        }
    }

    fn bootsel(context: &mut Context, _: VarHeader, (): ()) -> BootselReply {
        if context.0.lock().unwrap().held {
            return Err(Refusal::Held);
        }
        Ok(())
    }

    /// An [`RpcBoard`] talking postcard-rpc to [`Panel`] over channels.
    fn board(panel: Arc<Mutex<Panel>>) -> RpcBoard {
        RpcBoard::spawn(move || {
            let (to_server, server_rx) = tokio::sync::mpsc::channel(8);
            let (server_tx, from_server) = tokio::sync::mpsc::channel(8);
            let dispatcher = PanelDispatcher::new(Context(panel), ChannelWireSpawn);
            let kkind = dispatcher.min_key_len();
            let mut server = new_server(
                dispatcher,
                Settings {
                    tx: ChannelWireTx::new(server_tx),
                    rx: ChannelWireRx::new(server_rx),
                    buf: 256,
                    kkind,
                },
            );
            tokio::spawn(async move { server.run().await });
            Ok(new_from_channels(to_server, from_server, VarSeqKind::Seq2))
        })
        .unwrap()
    }

    #[test]
    fn a_set_is_read_back_and_a_power_cut_is_counted() {
        let panel = Arc::new(Mutex::new(Panel {
            cuts: u32::MAX,
            ..Panel::default()
        }));
        let mut relay = Relay::of(Box::new(board(panel)));

        relay.set(POWER, Switch::On).unwrap();
        assert!(relay.get(POWER).unwrap());
        assert!(!relay.get(STRAP).unwrap());
        assert_eq!(relay.power_cuts().unwrap(), u32::MAX, "nothing was cut");

        relay.set(POWER, Switch::Off).unwrap();
        assert_eq!(relay.power_cuts().unwrap(), 0, "the count wraps");
    }

    /// While RECOVER holds the outputs a switch is refused, and the state
    /// still reads.
    #[test]
    fn a_held_board_refuses_switches_and_still_answers_status() {
        let panel = Arc::new(Mutex::new(Panel {
            held: true,
            ..Panel::default()
        }));
        let mut board = board(panel);

        assert!(matches!(board.set(POWER, Switch::On), Err(Error::Held)));
        assert!(matches!(board.bootsel(), Err(Error::Held)));
        assert!(!board.get(POWER).unwrap());
    }

    #[test]
    fn a_set_the_board_reports_undone_is_an_error() {
        let panel = Arc::new(Mutex::new(Panel {
            ignores_sets: true,
            ..Panel::default()
        }));
        let mut board = board(panel);

        assert!(matches!(
            board.set(POWER, Switch::On),
            Err(Error::Unconfirmed {
                channel: POWER,
                on: false
            })
        ));
    }

    #[test]
    fn a_channel_outside_one_to_four_is_refused_before_sending() {
        let mut board = board(Arc::default());
        assert!(matches!(
            board.set(5, Switch::On),
            Err(Error::Channel { channel: 5 })
        ));
        assert!(matches!(board.get(0), Err(Error::Channel { channel: 0 })));
    }

    #[test]
    fn a_board_that_never_opens_reports_why() {
        let error = RpcBoard::spawn(|| Err(Error::NotFound { serial: "X".into() }))
            .err()
            .unwrap();
        assert!(matches!(error, Error::NotFound { .. }), "{error:?}");
    }

    #[test]
    fn the_fake_counts_cuts_and_refuses_as_told() {
        let board = FakeBoard::default();
        let mut relay = Relay::of(Box::new(board.clone()));

        relay.set(POWER, Switch::On).unwrap();
        relay.set(POWER, Switch::Off).unwrap();
        assert_eq!(relay.power_cuts().unwrap(), 1);
        assert_eq!(board.log(), [(POWER, true), (POWER, false)]);

        board.lock().refuse = true;
        assert!(matches!(relay.set(STRAP, Switch::On), Err(Error::Held)));
    }

    #[test]
    fn a_detached_relay_switches_nothing() {
        assert!(matches!(
            Relay::detached().set(1, Switch::Off),
            Err(Error::Detached)
        ));
    }
}
