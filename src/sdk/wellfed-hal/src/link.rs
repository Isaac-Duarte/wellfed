//! The Wi-Fi task: the driver that runs its flow on the vendor's calls.
//!
//! The task is the C shim's (`wellfed_wifi` in `example_entry.c`), so its
//! stack stays sized for the vendor driver's own calls, but its body is
//! [`run`]: every decision is `wellfed_core::wifi_task`'s machine, host
//! tested there, and this driver performs each [`Action`] through the
//! shim's thin wrappers (`sdk::wifi_*`) and hands back what it got. The
//! machine decides on the time and the two wishes held here, a restart and
//! battery mode's radio, which it reads before every step, so every wait,
//! a run of 50 ms delays, sees a change within one of them. After every
//! step the driver publishes whether the join is live to [`net_status`],
//! which trial confirmation and the broker session read.
//!
//! **The product's half.** Which credentials an attempt uses, and what its
//! outcome means for the fallback, are the product's stores
//! (`wellfed-device-runtime`'s `network`), which this crate may not depend
//! on: the product passes them in as a [`Join`] and exports the task's
//! entry itself (`wellfed_wifi_run`), so every call stays a direct one the
//! stack audit follows.
//!
//! **Before an intentional restart** the runtime calls
//! [`leave_for_restart`]. An intentional restart sends the access point
//! nothing, so it still holds the old boot's protected association and
//! rejects the next boot's with status 30, and the vendor's `wifi_connect`
//! can then hang the whole KM4 into a silent watchdog reset (observed on
//! the bench). So the task leaves a joined link whatever it shows, and
//! never joins again. The restart itself never waits for this: the watchdog
//! feeder stops at once, and the leave runs inside the watchdog's expiry or
//! not at all.
//!
//! **Battery mode** asks for the radio off ([`set_radio`]): the task leaves
//! as it does for a restart, turns the radio off (`wifi_off`) and waits
//! until it is asked on again: then `wifi_on` and a fresh join. Between
//! attempts it checks the wish too, so it neither scans nor joins while the
//! radio should be off. A restart wins over both.
//!
//! **A radio change takes the join's turn.** `wifi_off` (the driver's
//! deinit) and `wifi_on` (its RF bring-up) stall every task in the vendor
//! driver as an association does, which would break a moving feed's beat.
//! So the task brackets the leave and `wifi_off`, and each `wifi_on`, with
//! [`radio_begin`] and [`radio_end`]: the interlock's join turn
//! ([`crate::interlock`]). A claimed feed defers the change (up to the
//! claim's bound, as it defers a join), and a feed asked for during one is
//! refused as during a join. A `wifi_on` that keeps failing ends in a
//! restart ([`radio_failed`]), asked for with the turn still held, so no
//! feed starts into the reset.
//!
//! **The scan** reports each access point from the driver's context to
//! [`wellfed_wifi_scan_seen`], which keeps the strongest that names the
//! network (`wifi_task::Sighting`), against its own copy of the SSID.
//!
//! **The gateway probe** is one ICMP echo ([`echo`]): the request and the
//! match are `wellfed_core::echo`'s, and the shim only opens the raw socket,
//! sends, receives one datagram with a timeout, and closes it.

use core::sync::atomic::{AtomicBool, Ordering};

use wellfed_core::echo::{self as icmp, Outcome};
use wellfed_core::liveness::Radio;
use wellfed_core::wifi_task::{Action, Attempt, Done, Event, SSID_MAX, Sighting, WifiTask, Wishes};

use crate::cell::TaskCell;
use crate::interlock::INTERLOCK;
use crate::sdk::{IrqGuard, JoinCredentials, WifiEvent};
use crate::{net_status, sdk};

/// Set once, by the restart; nothing clears it but the reset.
static RESTARTING: AtomicBool = AtomicBool::new(false);

/// The feeder is about to restart: leave the network for good.
pub fn leave_for_restart() {
    RESTARTING.store(true, Ordering::Release);
}

/// Whether a restart asked the link to leave.
pub fn restarting() -> bool {
    RESTARTING.load(Ordering::Acquire)
}

/// Set while battery mode wants the radio off; the boot starts on.
static RADIO_OFF: AtomicBool = AtomicBool::new(false);

/// The loop's wish for the radio, every pass; the Wi-Fi task acts on it.
pub fn set_radio(radio: Radio) {
    RADIO_OFF.store(radio == Radio::Off, Ordering::Release);
}

/// Whether battery mode wants the radio off.
pub fn radio_off() -> bool {
    RADIO_OFF.load(Ordering::Acquire)
}

/// Both wishes, as the Wi-Fi task reads them before each step.
fn wishes() -> Wishes {
    Wishes {
        restarting: restarting(),
        radio: if radio_off() { Radio::Off } else { Radio::On },
    }
}

/// Before a radio change (the leave and `wifi_off`, or one `wifi_on`):
/// take the join's turn, waiting while a feed holds the outputs.
pub fn radio_begin() {
    let _start = INTERLOCK.begin_join(sdk::uptime_ms, sdk::delay_ms);
}

/// After the radio change: give the turn back.
pub fn radio_end() {
    INTERLOCK.end_join();
}

/// `wifi_on` failed too many times in a row: restart, with the turn the
/// task still holds keeping every feed out until the reset. An intentional
/// restart with its grace, so the writer saves first.
pub fn radio_failed() {
    crate::supervision::begin_restart_grace(sdk::uptime_ms());
}

/// Host tests only: back to the boot state.
#[cfg(not(target_os = "none"))]
pub fn reset() {
    RESTARTING.store(false, Ordering::Release);
    RADIO_OFF.store(false, Ordering::Release);
    let _masked = IrqGuard::lock();
    // SAFETY: interrupts are masked, which serializes every access.
    unsafe { *SCAN.get() = Scan::NEW };
}

/// One ICMP echo to `gateway` (host order), numbered `seq`: whether it
/// answered within [`icmp::TIMEOUT_MS`], or why nothing was asked of it.
pub fn echo(gateway: u32, seq: u16) -> Outcome {
    let Some(socket) = sdk::icmp_open() else {
        return Outcome::NoSocket;
    };
    let outcome = exchange(socket, gateway, seq);
    sdk::tcp_close(socket);
    outcome
}

/// Send the request, then read datagrams until the gateway's reply, an
/// error, or the timeout over all of them: the raw socket also hands over
/// every other ICMP datagram to this host, each skipped.
fn exchange(socket: i32, gateway: u32, seq: u16) -> Outcome {
    if !sdk::icmp_send(socket, &icmp::request(seq), gateway) {
        return Outcome::NotSent;
    }

    let sent = sdk::uptime_ms();
    let mut datagram = [0; icmp::RECEIVE_BYTES];
    loop {
        let waited = sdk::uptime_ms().wrapping_sub(sent);
        let left = icmp::TIMEOUT_MS.saturating_sub(waited);
        if left == 0 {
            return Outcome::Silent;
        }
        let Some(len) = sdk::icmp_recv(socket, &mut datagram, left) else {
            return Outcome::Silent;
        };
        if datagram
            .get(..len)
            .is_some_and(|got| icmp::answers(got, gateway, seq))
        {
            return Outcome::Answered;
        }
    }
}

/// The scan under way: the SSID it looks for and what it has seen.
struct Scan {
    wanted: [u8; SSID_MAX],
    /// `None` for credentials no access point can name (longer than
    /// [`SSID_MAX`]).
    wanted_len: Option<usize>,
    sighting: Sighting,
}

impl Scan {
    const NEW: Self = Self {
        wanted: [0; SSID_MAX],
        wanted_len: None,
        sighting: Sighting {
            done: false,
            found: false,
            rssi: 0,
            security: 0,
        },
    };
}

/// Written by the Wi-Fi task before each scan and by the scan's callbacks
/// in the driver's context, each with interrupts masked. A callback that
/// comes late, after the task gave up on its scan, finds only this copy,
/// never the product's credentials a later select may be rewriting.
static SCAN: TaskCell<Scan> = TaskCell::new(Scan::NEW);

/// A scan for `ssid` starts: neither complete nor found yet.
fn scan_begin(ssid: &[u8]) {
    let _masked = IrqGuard::lock();
    // SAFETY: interrupts are masked, which serializes every access.
    let scan = unsafe { &mut *SCAN.get() };
    scan.wanted_len = scan.wanted.get_mut(..ssid.len()).map(|wanted| {
        wanted.copy_from_slice(ssid);
        ssid.len()
    });
    scan.sighting.restart();
}

fn sighting() -> Sighting {
    let _masked = IrqGuard::lock();
    // SAFETY: interrupts are masked, which serializes every access.
    unsafe { (*SCAN.get()).sighting }
}

/// The scan's callback, from the driver's context: one access point it
/// saw, by its SSID (`len` bytes at `ssid`), signal strength (dBm) and
/// `rtw_security_t`.
///
/// # Safety
/// `ssid` names `len` bytes, live for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wellfed_wifi_scan_seen(
    ssid: *const u8,
    len: u32,
    rssi: i32,
    security: u32,
) {
    // An SSID no longer than 802.11's, which a longer one cannot be.
    let Some(len) = usize::try_from(len).ok().filter(|&len| len <= SSID_MAX) else {
        return;
    };
    let ssid = if len == 0 || ssid.is_null() {
        &[][..]
    } else {
        // SAFETY: the caller's contract.
        unsafe { core::slice::from_raw_parts(ssid, len) }
    };

    let _masked = IrqGuard::lock();
    // SAFETY: interrupts are masked, which serializes every access.
    let scan = unsafe { &mut *SCAN.get() };
    if let Some(wanted) = scan.wanted_len.and_then(|len| scan.wanted.get(..len)) {
        scan.sighting.offer(wanted, ssid, rssi, security);
    }
}

/// The scan's callback, from the driver's context: the scan is complete.
#[unsafe(no_mangle)]
pub extern "C" fn wellfed_wifi_scan_complete() {
    let _masked = IrqGuard::lock();
    // SAFETY: interrupts are masked, which serializes every access.
    unsafe { (*SCAN.get()).sighting.complete() };
}

/// The product's half of each join attempt.
pub trait Join {
    /// Before each attempt: take the join's turn (waiting while a feed
    /// holds the outputs), then choose the credentials. `out` holds the
    /// build's network, or nothing without one; the product replaces it
    /// with stored credentials, or empties it when there is nothing to
    /// join. Pointers it leaves name bytes that live until the next call.
    ///
    /// # Safety
    /// Called only from the Wi-Fi task.
    unsafe fn select(&mut self, out: &mut JoinCredentials);

    /// After each attempt, nothing to join included: ends the join's turn.
    ///
    /// # Safety
    /// Called only from the Wi-Fi task, never while a select runs.
    unsafe fn outcome(&mut self, attempt: Attempt);
}

/// The Wi-Fi task's machine, the product's half and the credentials of
/// the attempt under way.
pub struct Driver<J> {
    task: WifiTask,
    join: J,
    credentials: JoinCredentials,
    done: Done,
}

impl<J: Join> Driver<J> {
    pub const fn new(join: J) -> Self {
        Self {
            task: WifiTask::new(),
            join,
            credentials: JoinCredentials::EMPTY,
            done: Done::Nothing,
        }
    }

    /// The product's half.
    pub const fn join(&self) -> &J {
        &self.join
    }

    /// One step: the machine's next action, given the time and the wishes
    /// now, performed. [`Action::Park`] and [`Action::Stop`] are left to the
    /// caller.
    ///
    /// # Safety
    /// Called only from the Wi-Fi task.
    pub unsafe fn turn(&mut self) -> Action {
        let action = self.task.step(sdk::uptime_ms(), wishes(), self.done);
        net_status::set_live(self.task.live());
        // SAFETY: the caller's contract.
        self.done = unsafe { self.perform(action) };
        action
    }

    /// # Safety
    /// Called only from the Wi-Fi task.
    unsafe fn perform(&mut self, action: Action) -> Done {
        match action {
            Action::NetInit => sdk::wifi_net_init(),
            Action::RadioOn => return Done::Status(sdk::wifi_on()),
            Action::PowerSaveOff => sdk::wifi_power_save_off(),
            Action::RadioOff => return Done::Status(sdk::wifi_off()),
            Action::ReleaseIp => sdk::wifi_release_ip(),
            Action::DhcpStop => sdk::wifi_dhcp_stop(),
            Action::DhcpStart => {
                let (state, ip) = sdk::wifi_dhcp_start();
                return Done::Dhcp { state, ip };
            }
            Action::Disconnect => return Done::Status(sdk::wifi_disconnect()),
            Action::Associated => return Done::Associated(sdk::wifi_associated()),
            Action::Select => {
                self.credentials = sdk::wifi_build_credentials();
                // SAFETY: the caller's contract: the Wi-Fi task.
                unsafe { self.join.select(&mut self.credentials) };
                let chosen = self.credentials.ssid_len != 0;
                return Done::Selected(chosen.then_some(self.credentials.source));
            }
            // SAFETY: the caller's contract: the Wi-Fi task, after its select.
            Action::Outcome(attempt) => unsafe { self.join.outcome(attempt) },
            Action::Scan => {
                // SAFETY: the select left the pointers naming live bytes.
                scan_begin(unsafe { self.credentials.ssid() });
                return Done::Status(sdk::wifi_scan());
            }
            Action::Sighting => return Done::Sighting(sighting()),
            Action::Connect { attempt, security } => {
                // SAFETY: the select left the pointers naming live bytes.
                let status = unsafe { sdk::wifi_connect(&self.credentials, security, attempt) };
                return Done::Status(status);
            }
            Action::AccessPoint => {
                return Done::AccessPoint {
                    channel: sdk::wifi_channel(),
                    bssid: sdk::wifi_bssid(),
                };
            }
            Action::Lease => {
                let (used, bound) = sdk::wifi_lease();
                return Done::Lease { used, bound };
            }
            // The lease holds while lwIP's client is bound, renewing or
            // rebinding: it gives the address up only once the lease ran
            // out with renew and rebind unanswered.
            Action::Link => {
                return Done::Link {
                    associated: sdk::wifi_associated(),
                    leased: sdk::ipv4().is_some(),
                };
            }
            Action::Gateway => return Done::Gateway(sdk::netif_gateway()),
            Action::Echo { gateway, seq } => return Done::Echo(echo(gateway, seq)),
            Action::TurnBegin => radio_begin(),
            Action::TurnEnd => radio_end(),
            Action::RadioFailed => radio_failed(),
            Action::Record(event) => record(&event),
            Action::Delay(ms) => sdk::delay_ms(ms),
            Action::Park | Action::Stop => {}
        }
        Done::Nothing
    }
}

/// Put `event` in the shim's ring, which numbers it.
fn record(event: &Event) {
    sdk::wifi_record(&WifiEvent {
        seq: 0,
        kind: event.kind.code(),
        attempt: event.attempt,
        result: event.result,
        value: event.value,
        rssi: event.rssi,
        security: event.security,
        ms: event.ms,
    });
}

/// The Wi-Fi task's body: returns only when the radio never came up at
/// boot, and the task then ends; after a restart's leave it blocks for
/// good.
///
/// # Safety
/// Called once, from the C Wi-Fi task.
pub unsafe fn run<J: Join>(join: J) {
    let mut driver = Driver::new(join);
    loop {
        // SAFETY: the caller's contract.
        match unsafe { driver.turn() } {
            Action::Park => loop {
                sdk::delay_ms(u32::MAX);
            },
            Action::Stop => return,
            _ => {}
        }
    }
}
