//! How the loop ends the device API's connections on the board's platform,
//! against the fake lwIP sockets: a connection whose last bytes never move is
//! reset (`wellfed_tcp_abort`), one the client hung up on or whose send
//! failed is closed, and every 30 s the loop sweeps the API port for closed
//! connections stuck behind a shut window (`wellfed_tcp_sweep`).

use std::sync::{Mutex, MutexGuard};

use wellfed_client::api::Session;
use wellfed_device::io::Reset;
use wellfed_device_api::STALL_MS;
use wellfed_device_runtime::{events, keys};
use wellfed_feeder::feeder::serve::{self, ResetBy};
use wellfed_feeder::feeder::{Feeder, NoFeeds, Record as PassRecord};
use wellfed_feeder_runtime::settings::SETTINGS;
use wellfed_fw_feeder::platform::{Driver, Plaf103};
use wellfed_proto::api::PORT;
use wellfed_sdk::hal::sdk::{TcpReset, fake};
use wellfed_sdk::hal::{buttons, clock};
use wellfed_session::noise::Keypair;

/// The mailboxes are process-wide; the fake hardware is per thread.
static SERIAL: Mutex<()> = Mutex::new(());

/// The feeder's identity, and a client key it has never enrolled.
const FEEDER: [u8; 32] = [0x11; 32];
const STRANGER: [u8; 32] = [0x22; 32];
/// Far enough apart that a test of minutes runs in milliseconds.
const PASS_MS: u32 = 100;

struct Loop {
    _serial: MutexGuard<'static, ()>,
    feeder: Box<Feeder<Plaf103>>,
    now: u32,
    resets: Vec<(ResetBy, Reset)>,
    lines: Vec<String>,
}

impl Loop {
    /// The loop on a network, its identity published, with no client yet.
    fn start() -> Self {
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        fake::reset_hardware();
        SETTINGS.reset();
        clock::reset();
        buttons::reset();
        keys::reset();
        keys::publish_identity(&FEEDER);
        // A handshake is answered once the event ring has opened.
        events::reset();
        events::publish_open(1, None);
        wellfed_sdk::hal::rng::reset();
        fake::with(|h| {
            h.adc_noise = 0x1357_9bdf;
            h.ipv4 = 0x0a2a_6702;
        });
        assert_eq!(wellfed_sdk::hal::rng::start().result, Ok(()));

        Self {
            _serial: serial,
            feeder: Box::new(Feeder::new(Plaf103::new())),
            now: 0,
            resets: Vec::new(),
            lines: Vec::new(),
        }
    }

    /// Passes until `until_ms`.
    fn run(&mut self, until_ms: u32) {
        while self.now < until_ms {
            self.now += PASS_MS;
            let now = self.now;
            fake::with(|h| h.uptime_ms = now);
            let (resets, lines) = (&mut self.resets, &mut self.lines);
            self.feeder
                .pass(now, &mut NoFeeds, |record: PassRecord<Driver>| {
                    if let PassRecord::Serve(record) = record {
                        if let serve::Record::Reset { by, reset } = record {
                            resets.push((by, reset));
                        }
                        let mut line = String::new();
                        serve::describe(record, &mut line).unwrap();
                        lines.push(line);
                    }
                });
        }
    }

    /// A client connects on `socket` and sends an IK session's first
    /// message under a key the feeder doesn't know: the feeder queues its
    /// refusal.
    fn connect(&mut self, socket: i32) {
        let feeder = *Keypair::from_secret(FEEDER).public();
        let (_, hello) =
            Session::connect(&Keypair::from_secret(STRANGER), feeder, [3; 32]).unwrap();
        fake::with(|h| {
            h.api_waiting.push_back(socket);
            h.api_rx.entry(socket).or_default().extend(hello);
        });
    }
}

fn closed() -> Vec<i32> {
    fake::with(|h| h.tcp_closed.clone())
}

fn aborted() -> Vec<i32> {
    fake::with(|h| h.tcp_aborted.clone())
}

/// A client that never takes the refusal: its bytes stall, and at the limit
/// the connection is reset, not closed, and the reset is logged.
#[test]
fn a_connection_whose_last_bytes_stall_is_reset() {
    let mut test = Loop::start();
    fake::with(|h| h.api_send_room = 0);
    test.connect(7);

    test.run(STALL_MS - 1_000);
    assert!(aborted().is_empty(), "inside the limit");

    test.run(STALL_MS + 2_000);
    assert_eq!(aborted(), [7]);
    assert!(!closed().contains(&7), "reset rather than closed");
    assert_eq!(test.resets, [(ResetBy::Abort, Reset::Done(1))]);
    assert!(test.lines.iter().any(|l| l == "tcp reset by=abort reset=1"));
}

/// A reset that never reached lwIP's thread is logged, so the bench sees
/// it; the socket is closed by the shim either way.
#[test]
fn a_reset_that_never_reached_the_stack_is_logged() {
    let mut test = Loop::start();
    fake::with(|h| {
        h.api_send_room = 0;
        h.tcp_reset = TcpReset::Unreachable;
    });
    test.connect(7);

    test.run(STALL_MS + 2_000);
    assert_eq!(aborted(), [7]);
    assert_eq!(test.resets, [(ResetBy::Abort, Reset::Unreachable)]);
    assert!(
        test.lines
            .iter()
            .any(|l| l == "tcp reset by=abort unreachable=1")
    );
}

/// A client that hung up is closed: nothing of ours is stuck behind it.
#[test]
fn a_connection_the_client_hung_up_is_closed() {
    let mut test = Loop::start();
    fake::with(|h| {
        h.api_waiting.push_back(8);
        h.api_hung_up.push(8);
    });

    test.run(1_000);
    assert_eq!(closed(), [8]);
    assert!(aborted().is_empty());
}

/// A send the stack refused is closed: the socket already failed.
#[test]
fn a_connection_whose_send_failed_is_closed() {
    let mut test = Loop::start();
    fake::with(|h| h.tcp_send_fails = true);
    test.connect(9);

    test.run(1_000);
    assert_eq!(closed(), [9]);
    assert!(aborted().is_empty());
}

/// The sweep runs on the API port once every stall limit, and is logged only
/// when it reset something or never reached the stack.
#[test]
fn the_loop_sweeps_the_api_port_every_stall_limit() {
    let mut test = Loop::start();

    test.run(STALL_MS - PASS_MS);
    assert!(fake::with(|h| h.tcp_sweeps.is_empty()), "not before");
    test.run(3 * STALL_MS + PASS_MS);
    assert_eq!(fake::with(|h| h.tcp_sweeps.clone()), [PORT; 3]);
    assert!(test.resets.is_empty(), "nothing reset, nothing logged");

    fake::with(|h| h.tcp_swept = TcpReset::Reset(2));
    test.run(4 * STALL_MS + PASS_MS);
    fake::with(|h| h.tcp_swept = TcpReset::Unreachable);
    test.run(5 * STALL_MS + PASS_MS);
    assert_eq!(fake::with(|h| h.tcp_sweeps.len()), 5);
    assert_eq!(
        test.resets,
        [
            (ResetBy::Sweep, Reset::Done(2)),
            (ResetBy::Sweep, Reset::Unreachable)
        ]
    );
    assert!(test.lines.iter().any(|l| l == "tcp reset by=sweep reset=2"));
    assert!(
        test.lines
            .iter()
            .any(|l| l == "tcp reset by=sweep unreachable=1")
    );
}
