//! The Wi-Fi task's whole flow: joining the network, keeping it, leaving
//! it, and battery mode's radio off and on, as a machine that does no IO.
//!
//! The task runs on the shim's `wellfed_wifi` task (its stack sized for
//! the vendor driver's calls). Its driver ([`step`](WifiTask::step) in a
//! loop, in wellfed-hal's `link`) performs each [`Action`] with the shim's
//! vendor calls and hands back what it got ([`Done`]), with the time and
//! the two wishes the rest of the firmware holds ([`Wishes`]): a restart
//! (leave for good) and battery mode's radio. Every wait is a run of
//! [`POLL_MS`] delays the machine itself decides on, so it sees a wish
//! within one poll wherever it waits.
//!
//! **Boot.** `LwIP_Init`, `wifi_on` in station mode and power save off; a
//! radio that does not come up ends the task. Power save stays off: the
//! chip never sleeps on the adapter, so holding frames until a beacon wake
//! saves nothing, and inactive power save must stay off for its own reason.
//! After a rejected join the driver tears down in the timer task, and its
//! key-table writes (`rtw_clearstakey_cmd`, then the CAM writes) race
//! `ips_enter`, which powers the MAC down while unassociated. A store to
//! the powered-down MAC stalled the bus and the tick with it, until the
//! watchdog reset the board: the call trail caught it in every such hang.
//!
//! **Attempts.** Each takes credentials from the product ([`Action::Select`],
//! which first takes the join's turn from feeds), scans for the network to
//! learn its security, picking the strongest access point that names it
//! ([`Sighting`]), connects and runs DHCP, then reports the outcome, which
//! ends the turn. With nothing to join it neither scans nor associates. The
//! SDK's static-address DHCP fallback counts as a failure and is cleared. A
//! failed attempt waits [`BACKOFF_MIN_MS`], doubling to [`BACKOFF_MAX_MS`];
//! with nothing to join the wait is the longest, which still picks up
//! credentials BLE setup stores within one (the radio stays on, since BLE
//! needs it). While scans complete without the network the wait stops at
//! [`BACKOFF_ABSENT_MAX_MS`], so a network back from an outage is found
//! within about that; the longest wait is for a network that is there and
//! refuses the feeder (a wrong password, a full access point). A scan that
//! finds the network after one that completed without it starts the
//! doubling over: a network back from an outage has not refused the feeder
//! yet. Its first join often fails all the same, since the driver offers
//! the cached PMKSA and an access point that restarted rejects it (status
//! 53) before the driver drops it, so the retry that follows at full SAE
//! comes after a second, not after the longest wait.
//!
//! **Watch.** A joined link is sampled every [`WATCH_MS`] and judged by
//! [`Liveness`], which asks for a gateway echo when one is due. A settled
//! join resets the backoff, so a network that keeps dropping the feeder
//! right after DHCP backs off rather than rejoining every second. A lease
//! clock gone back while bound is a renewal (no lwIP event says so). On a
//! loss the address is dropped, and an association still up is left so the
//! next attempt joins afresh.
//!
//! **Restart.** Before an intentional restart the firmware asks the task to
//! leave for good: the access point otherwise holds the old boot's
//! protected association and rejects the next one with status 30. A joined
//! link is left with a bounded wait for the driver to report it gone
//! ([`LEAVE_WAIT_MS`]); then the task parks. A restart wins over battery
//! mode, and no attempt associates once one is asked for.
//!
//! **Battery mode.** With the radio wished off the task leaves (when
//! joined), turns the radio off and waits for the wish to turn; then
//! `wifi_on` as at boot and a fresh attempt. `wifi_off` and `wifi_on` stall
//! every task in the driver as an association does, so each runs inside the
//! join's turn ([`Action::TurnBegin`]). Before `wifi_off` a failed or
//! rejected join's teardown may still run in the driver's timer task, and
//! powering the MAC down under it stalled the bus into a watchdog reset: so
//! [`RADIO_SETTLE_MS`] outside the turn with no association, and
//! [`RADIO_LEAVE_SETTLE_MS`] inside it after a leave (provisional: the
//! teardown has finished well within both on the bench). A `wifi_on` that
//! fails is tried every [`RADIO_RETRY_MS`]; after [`RADIO_TRIES`] in a row
//! the feeder restarts with the turn still held, so no feed starts into the
//! reset.
//!
//! **Events.** Each step the host tools see is an [`Event`] for the shim's
//! ring, whose kinds, results and values they parse.

use crate::echo::Outcome as Echo;
use crate::liveness::{Cause, Liveness, Probe, Radio, Verdict};

/// The first wait after a failed attempt, and the wait once a join settles.
pub const BACKOFF_MIN_MS: u32 = 1_000;
/// The longest wait between attempts.
pub const BACKOFF_MAX_MS: u32 = 30_000;
/// The longest wait while the last scan did not see the network. Each
/// attempt holds feeds for its scan (about 4 s), so a due meal waits that
/// much more often during an outage; well within a meal's 60 s.
pub const BACKOFF_ABSENT_MAX_MS: u32 = 10_000;
/// How long a scan may run before it counts as timed out.
pub const SCAN_WAIT_MS: u32 = 10_000;
/// Between watch ticks of a joined link.
pub const WATCH_MS: u32 = 500;
/// Every wait is a run of these, each followed by a look at the wishes.
pub const POLL_MS: u32 = 50;
/// How long a leave waits for the driver to report the association gone:
/// inside the watchdog's shortest expiry (2.4 s at its clock's bound) with
/// a gateway probe's second before it.
pub const LEAVE_WAIT_MS: u32 = 1_000;
/// Between `wifi_on` tries when the radio comes back.
pub const RADIO_RETRY_MS: u32 = 5_000;
/// `wifi_on` failures in a row, [`RADIO_RETRY_MS`] apart, before the
/// feeder restarts: about 30 s without Wi-Fi on the adapter.
pub const RADIO_TRIES: u32 = 6;
/// Before `wifi_off` with no association held, outside the join's turn.
pub const RADIO_SETTLE_MS: u32 = 2_000;
/// After a leave, before `wifi_off`, inside the join's turn.
pub const RADIO_LEAVE_SETTLE_MS: u32 = 500;

/// The vendor's `RTW_SUCCESS`.
pub const SUCCESS: i32 = 0;
/// The vendor's `RTW_TIMEOUT`: a scan event's result when the scan never
/// completed within [`SCAN_WAIT_MS`]. The shim checks it against the SDK.
pub const SCAN_TIMEOUT: i32 = 2;
/// lwIP's `DHCP_ADDRESS_ASSIGNED`; the shim checks it against the SDK.
pub const DHCP_ASSIGNED: u8 = 2;
/// The longest SSID 802.11 allows.
pub const SSID_MAX: usize = 32;

/// An event's kind, as the shim's ring and the host tools number it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Kind {
    /// `wifi_on`: `result` its return, `value` battery mode's failures in
    /// a row, `ms` its duration.
    On = 1,
    /// `result` the scan's, `value` 1 when the network was seen, with its
    /// RSSI and security.
    Scan = 2,
    /// `wifi_connect`: `value` the credentials' source, never the
    /// credentials.
    Connect = 3,
    /// `result` the DHCP state, `value` the address (host order) once one
    /// is assigned.
    Dhcp = 4,
    /// A joined link lost: `result` its [`Cause`], `value` ms joined.
    Lost = 5,
    /// `value` ms until the next attempt.
    Backoff = 6,
    // 7 was the internet DNS check; never reused.
    /// A gateway probe: `result` [`Echo::code`], `value` the gateway.
    Gateway = 8,
    /// A clean leave: `result` `wifi_disconnect`'s, `value` 1 while still
    /// associated at [`LEAVE_WAIT_MS`].
    Leave = 9,
    /// After an address: `result` the channel (1-14, 36-177) or -1,
    /// `value` the BSSID's last 4 bytes.
    AccessPoint = 10,
    /// lwIP's DHCP client took a new ACK without a rejoin.
    Renew = 11,
    /// Battery mode: `result` `wifi_off`'s.
    Off = 12,
}

impl Kind {
    pub const fn code(self) -> u32 {
        self as u32
    }
}

/// One event for the shim's ring, which numbers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub kind: Kind,
    pub attempt: u32,
    pub result: i32,
    pub value: u32,
    /// Scan events only; 0 on the rest.
    pub rssi: i32,
    pub security: u32,
    pub ms: u32,
}

/// What a scan has seen of the network being joined. The scan's callback
/// fills it from the driver's context while the task polls it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sighting {
    /// The driver reported the scan complete.
    pub done: bool,
    pub found: bool,
    /// The strongest matching access point's, kept from an earlier scan
    /// until this one finds the network: the scan event reports them either
    /// way.
    pub rssi: i32,
    pub security: u32,
}

impl Sighting {
    /// A scan starts: neither complete nor found yet.
    pub fn restart(&mut self) {
        self.done = false;
        self.found = false;
    }

    /// One access point the scan saw: kept when it names `wanted` and is
    /// the strongest of those so far.
    pub fn offer(&mut self, wanted: &[u8], ssid: &[u8], rssi: i32, security: u32) {
        if ssid == wanted && (!self.found || rssi > self.rssi) {
            self.rssi = rssi;
            self.security = security;
            self.found = true;
        }
    }

    pub fn complete(&mut self) {
        self.done = true;
    }
}

/// What the rest of the firmware wants of the task, read before each step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wishes {
    /// A restart is coming: leave for good. Set once; only the reset
    /// clears it.
    pub restarting: bool,
    /// Battery mode's wish for the radio.
    pub radio: Radio,
}

impl Wishes {
    /// Nothing asked: no restart, the radio on.
    pub const NONE: Self = Self {
        restarting: false,
        radio: Radio::On,
    };

    /// A restart or battery mode wants the task's attention.
    const fn interrupting(self) -> bool {
        self.restarting || matches!(self.radio, Radio::Off)
    }
}

/// How a join attempt ended, as the product counts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attempt {
    /// An address was assigned: the link is watched from here.
    Joined,
    /// Anything else, nothing to join included.
    Failed,
}

/// What the task does next. Each names one shim call; the driver answers
/// with the [`Done`] the action's documentation names, or
/// [`Done::Nothing`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// `LwIP_Init`.
    NetInit,
    /// `wifi_on` in station mode: [`Done::Status`].
    RadioOn,
    /// Inactive and leisure power save off (`wifi_set_power_mode(0, 0)`).
    PowerSaveOff,
    /// `wifi_off`: [`Done::Status`].
    RadioOff,
    /// `LwIP_ReleaseIP`.
    ReleaseIp,
    /// Stop lwIP's DHCP client.
    DhcpStop,
    /// Run DHCP: [`Done::Dhcp`].
    DhcpStart,
    /// `wifi_disconnect`: [`Done::Status`].
    Disconnect,
    /// Whether the driver holds an association: [`Done::Associated`].
    Associated,
    /// Take the join's turn and choose credentials: [`Done::Selected`].
    Select,
    /// Report the attempt, which ends the join's turn.
    Outcome(Attempt),
    /// Start a scan for the selected network, its sighting restarted:
    /// [`Done::Status`].
    Scan,
    /// Read the scan's sighting: [`Done::Sighting`].
    Sighting,
    /// `wifi_connect` with the selected credentials and the security the
    /// scan saw: [`Done::Status`].
    Connect { attempt: u32, security: u32 },
    /// The joined access point's channel and BSSID: [`Done::AccessPoint`].
    AccessPoint,
    /// lwIP's lease clock and whether its client is bound:
    /// [`Done::Lease`].
    Lease,
    /// The link as the watch judges it: [`Done::Link`].
    Link,
    /// The interface's gateway: [`Done::Gateway`].
    Gateway,
    /// One echo to the gateway: [`Done::Echo`].
    Echo { gateway: u32, seq: u16 },
    /// Take the join's turn for a radio change, waiting while a feed holds
    /// the outputs.
    TurnBegin,
    /// Give the turn back.
    TurnEnd,
    /// `wifi_on` keeps failing: restart, the turn still held.
    RadioFailed,
    /// Put an event in the shim's ring.
    Record(Event),
    /// Block the task this long.
    Delay(u32),
    /// Nothing more to do until the reset: block for good.
    Park,
    /// The radio never came up at boot: end the task.
    Stop,
}

/// What the driver got from the last action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Done {
    /// The first step, or an action with nothing to report.
    Nothing,
    /// A vendor call's return.
    Status(i32),
    Associated(bool),
    /// The credentials' source code, or `None` with nothing to join.
    Selected(Option<u32>),
    Sighting(Sighting),
    /// The DHCP state, and the address (host order) once assigned.
    Dhcp {
        state: u8,
        ip: u32,
    },
    /// The channel as `wifi_get_channel` left it, and the BSSID; `None`
    /// where the call failed.
    AccessPoint {
        channel: Option<i32>,
        bssid: Option<[u8; 6]>,
    },
    /// lwIP's lease clock (seconds since the last ACK) and whether its
    /// client is bound.
    Lease {
        used: u32,
        bound: bool,
    },
    /// Whether the driver holds the association, and lwIP the leased
    /// address.
    Link {
        associated: bool,
        leased: bool,
    },
    /// The gateway (host order), 0 for none.
    Gateway(u32),
    Echo(Echo),
}

impl Done {
    // Each reads the answer its action names. Another can only come from a
    // driver bug; it reads as a failure, never as success.

    fn status(self) -> i32 {
        match self {
            Self::Status(code) => code,
            _ => -1,
        }
    }

    fn associated(self) -> bool {
        matches!(self, Self::Associated(true))
    }

    fn sighting(self) -> Sighting {
        match self {
            Self::Sighting(sighting) => sighting,
            _ => Sighting::default(),
        }
    }
}

/// Where a radio change began: between attempts with nothing joined, or
/// from a joined link the watch left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    Unassociated,
    Associated,
}

/// Where a leave goes once its event is recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfterLeave {
    /// An attempt a wish cut short after it associated.
    AttemptFailed,
    Park,
    /// Battery mode's leave, inside the turn: settle, then `wifi_off`.
    RadioSettle,
}

/// Where a hold goes once it ran out (or a restart ended it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfterHold {
    /// The unassociated settle before `wifi_off`.
    Settled,
    /// The settle after a leave, inside the turn.
    RadioOff,
    /// Between `wifi_on` tries.
    RadioRetry,
}

/// Each point of the flow where the task resumes after an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum At {
    Start,
    BootNet {
        begin: u32,
    },
    BootRadio {
        begin: u32,
    },
    BootReported {
        on: i32,
    },
    Top,
    Attempt,
    Selected,
    Scan,
    ScanStarted {
        begin: u32,
    },
    ScanPolled {
        begin: u32,
        result: i32,
    },
    ScanWaited {
        begin: u32,
        result: i32,
    },
    Scanned {
        result: i32,
        sighting: Sighting,
    },
    Connected {
        begin: u32,
    },
    ConnectReported {
        result: i32,
    },
    DhcpDone {
        begin: u32,
    },
    DhcpReported {
        assigned: bool,
    },
    FallbackStopped,
    FallbackReleased,
    AccessPointRead,
    JoinFailed,
    JoinSucceeded,
    /// The outcome was reported; `None` when there was nothing to join.
    Reported(Option<Attempt>),
    WatchStarted,
    WatchTick,
    WatchLeased,
    WatchRenewed,
    WatchSampled,
    WatchWait {
        waited: u32,
    },
    GatewayRead,
    Echoed {
        gateway: u32,
        begin: u32,
    },
    GatewayReported {
        probe: Probe,
    },
    LostReported {
        cause: Cause,
    },
    LostStopped {
        cause: Cause,
    },
    LostReleased,
    Backoff,
    BackoffWait {
        waited: u32,
    },
    Leave {
        then: AfterLeave,
    },
    LeaveStopped {
        then: AfterLeave,
        begin: u32,
    },
    LeaveReleased {
        then: AfterLeave,
        begin: u32,
    },
    LeaveDisconnected {
        then: AfterLeave,
        begin: u32,
    },
    LeavePolled {
        then: AfterLeave,
        begin: u32,
        result: i32,
    },
    LeaveWaited {
        then: AfterLeave,
        begin: u32,
        result: i32,
    },
    LeaveChecked {
        then: AfterLeave,
        begin: u32,
        result: i32,
    },
    RadioDown,
    Hold {
        ms: u32,
        waited: u32,
        then: AfterHold,
    },
    RadioSettled,
    RadioTurn,
    RadioOff,
    RadioReleased {
        begin: u32,
    },
    RadioOffDone {
        begin: u32,
    },
    RadioOffEnded {
        begin: u32,
        off: i32,
    },
    RadioWait,
    RadioOnTurn,
    RadioOnDone {
        begin: u32,
    },
    RadioCounted {
        begin: u32,
        on: i32,
    },
    RadioTurnEnded {
        begin: u32,
        on: i32,
    },
    RadioReported {
        on: i32,
    },
    Parked,
    Stopped,
}

/// The Wi-Fi task, from boot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WifiTask {
    at: At,
    attempt: u32,
    backoff: u32,
    /// The gateway echo's sequence, kept across joins.
    seq: u16,
    /// The selected credentials' source, for the connect event.
    source: u32,
    /// The current join's watch.
    watch: Liveness,
    live: bool,
    /// The gateway probe's result, for the next watch tick.
    probe: Probe,
    /// lwIP's lease clock at the last watch tick.
    lease_used: u32,
    joined_at: u32,
    /// Where the radio change under way began.
    entry: Entry,
    /// `wifi_on` failures in a row since the radio went off.
    failed: u32,
    /// Whether the last scan that completed saw the network.
    seen: bool,
}

impl Default for WifiTask {
    fn default() -> Self {
        Self::new()
    }
}

/// The next point, and the action that leads there; without one, the
/// machine goes on at once in the same step.
type Next = (At, Option<Action>);

const fn go(at: At) -> Next {
    (at, None)
}

const fn act(at: At, action: Action) -> Next {
    (at, Some(action))
}

/// A channel `wifi_get_channel` read, or -1: AmebaD is dual band, so a
/// 2.4 GHz (1-14) or 5 GHz (36-177) channel, and anything else is none.
fn channel(read: Option<i32>) -> i32 {
    read.filter(|channel| matches!(channel, 1..=14 | 36..=177))
        .unwrap_or(-1)
}

/// The BSSID's last four bytes, which tell a home's few access points
/// apart; 0 unread.
fn bssid_tail(bssid: Option<[u8; 6]>) -> u32 {
    bssid.map_or(0, |b| u32::from_be_bytes([b[2], b[3], b[4], b[5]]))
}

impl WifiTask {
    pub const fn new() -> Self {
        Self {
            at: At::Start,
            attempt: 0,
            backoff: BACKOFF_MIN_MS,
            seq: 0,
            source: 0,
            watch: Liveness::new(),
            live: false,
            probe: Probe::None,
            lease_used: 0,
            joined_at: 0,
            entry: Entry::Unassociated,
            failed: 0,
            seen: false,
        }
    }

    /// Whether the current join is known good (`Liveness::live`): trial
    /// confirmation and the broker session wait for it. False from a
    /// restart's or battery mode's leave on.
    pub const fn live(&self) -> bool {
        self.live
    }

    /// The next action, given the time after the last one, the wishes read
    /// now and what the last action got.
    pub fn step(&mut self, now: u32, wishes: Wishes, done: Done) -> Action {
        let mut done = done;
        loop {
            let (at, action) = self.advance(now, wishes, done);
            self.at = at;
            if let Some(action) = action {
                return action;
            }
            done = Done::Nothing;
        }
    }

    fn event(&self, kind: Kind, result: i32, value: u32, ms: u32) -> Action {
        Action::Record(Event {
            kind,
            attempt: self.attempt,
            result,
            value,
            rssi: 0,
            security: 0,
            ms,
        })
    }

    /// The radio is back on (or never went off): a fresh attempt.
    fn radio_back(&mut self) -> Next {
        self.backoff = BACKOFF_MIN_MS;
        match self.entry {
            // Between attempts: straight to the next one.
            Entry::Unassociated => go(At::Attempt),
            // From a watch: round the loop, its wishes first.
            Entry::Associated => go(At::Top),
        }
    }

    fn advance(&mut self, now: u32, wishes: Wishes, done: Done) -> Next {
        let since = |begin: u32| now.wrapping_sub(begin);
        match self.at {
            At::Start => act(At::BootNet { begin: now }, Action::NetInit),
            At::BootNet { begin } => act(At::BootRadio { begin }, Action::RadioOn),
            At::BootRadio { begin } => {
                let on = done.status();
                let event = self.event(Kind::On, on, 0, since(begin));
                act(At::BootReported { on }, event)
            }
            // 0 started, 1 already running (something brought the radio up
            // first), as battery mode reads it; below 0 the driver failed.
            At::BootReported { on } if on < 0 => act(At::Stopped, Action::Stop),
            At::BootReported { .. } => act(At::Top, Action::PowerSaveOff),

            // Between attempts no join holds the network: one that got no
            // address disconnected.
            At::Top if wishes.restarting => act(At::Parked, Action::Park),
            At::Top if wishes.radio == Radio::Off => {
                self.entry = Entry::Unassociated;
                go(At::RadioDown)
            }
            At::Top => go(At::Attempt),
            At::Attempt => {
                self.attempt = self.attempt.wrapping_add(1);
                act(At::Selected, Action::Select)
            }
            At::Selected => match done {
                Done::Selected(Some(source)) => {
                    self.source = source;
                    // A restart asked for while the turn waited: join no
                    // more; nor while battery mode wants the radio off.
                    if wishes.interrupting() {
                        go(At::JoinFailed)
                    } else {
                        go(At::Scan)
                    }
                }
                _ => act(At::Reported(None), Action::Outcome(Attempt::Failed)),
            },

            At::Scan => act(At::ScanStarted { begin: now }, Action::Scan),
            At::ScanStarted { begin } => {
                let result = done.status();
                act(At::ScanPolled { begin, result }, Action::Sighting)
            }
            At::ScanPolled { begin, result } => {
                let sighting = done.sighting();
                if result == SUCCESS && !sighting.done && since(begin) < SCAN_WAIT_MS {
                    return act(At::ScanWaited { begin, result }, Action::Delay(POLL_MS));
                }

                let result = if result == SUCCESS && !sighting.done {
                    SCAN_TIMEOUT
                } else {
                    result
                };
                let event = Action::Record(Event {
                    kind: Kind::Scan,
                    attempt: self.attempt,
                    result,
                    value: u32::from(sighting.found),
                    rssi: sighting.rssi,
                    security: sighting.security,
                    ms: since(begin),
                });
                act(At::Scanned { result, sighting }, event)
            }
            At::ScanWaited { begin, result } => {
                act(At::ScanPolled { begin, result }, Action::Sighting)
            }
            // A restart asked for during the scan (up to 10 s): no new
            // association, which the access point would hold against the
            // next boot. Nor one battery mode would only leave again.
            At::Scanned { result, sighting } => {
                if result == SUCCESS {
                    if sighting.found && !self.seen {
                        self.backoff = BACKOFF_MIN_MS;
                    }
                    self.seen = sighting.found;
                    if !sighting.found {
                        self.backoff = self.backoff.min(BACKOFF_ABSENT_MAX_MS);
                    }
                }
                if result != SUCCESS || !sighting.found || wishes.interrupting() {
                    return go(At::JoinFailed);
                }
                let connect = Action::Connect {
                    attempt: self.attempt,
                    security: sighting.security,
                };
                act(At::Connected { begin: now }, connect)
            }
            At::Connected { begin } => {
                let result = done.status();
                let event = self.event(Kind::Connect, result, self.source, since(begin));
                act(At::ConnectReported { result }, event)
            }
            At::ConnectReported { result } if result != SUCCESS => go(At::JoinFailed),
            // Asked for while it associated: leave the fresh association at
            // once.
            At::ConnectReported { .. } if wishes.interrupting() => go(At::Leave {
                then: AfterLeave::AttemptFailed,
            }),
            At::ConnectReported { .. } => act(At::DhcpDone { begin: now }, Action::DhcpStart),
            At::DhcpDone { begin } => {
                let (state, ip) = match done {
                    Done::Dhcp { state, ip } => (state, ip),
                    _ => (0, 0),
                };
                let assigned = state == DHCP_ASSIGNED;
                let ip = if assigned { ip } else { 0 };
                let event = self.event(Kind::Dhcp, i32::from(state), ip, since(begin));
                act(At::DhcpReported { assigned }, event)
            }
            // Never keep the SDK's static fallback address.
            At::DhcpReported { assigned: false } => act(At::FallbackStopped, Action::DhcpStop),
            At::DhcpReported { assigned: true } => act(At::AccessPointRead, Action::AccessPoint),
            At::FallbackStopped => act(At::FallbackReleased, Action::ReleaseIp),
            At::FallbackReleased => act(At::JoinFailed, Action::Disconnect),
            At::AccessPointRead => {
                let (read, bssid) = match done {
                    Done::AccessPoint { channel, bssid } => (channel, bssid),
                    _ => (None, None),
                };
                let event = self.event(Kind::AccessPoint, channel(read), bssid_tail(bssid), 0);
                act(At::JoinSucceeded, event)
            }
            At::JoinFailed => act(
                At::Reported(Some(Attempt::Failed)),
                Action::Outcome(Attempt::Failed),
            ),
            At::JoinSucceeded => {
                // Watched from scratch, and not live until it settles.
                self.watch = Liveness::new();
                self.live = false;
                act(
                    At::Reported(Some(Attempt::Joined)),
                    Action::Outcome(Attempt::Joined),
                )
            }
            At::Reported(None) => {
                self.backoff = BACKOFF_MAX_MS;
                go(At::Backoff)
            }
            At::Reported(Some(Attempt::Failed)) => go(At::Backoff),
            At::Reported(Some(Attempt::Joined)) => {
                self.joined_at = now;
                self.probe = Probe::None;
                act(At::WatchStarted, Action::Lease)
            }

            At::WatchStarted => {
                if let Done::Lease { used, .. } = done {
                    self.lease_used = used;
                }
                go(At::WatchTick)
            }
            At::WatchTick => act(At::WatchLeased, Action::Lease),
            At::WatchLeased => {
                let (used, bound) = match done {
                    Done::Lease { used, bound } => (used, bound),
                    _ => (self.lease_used, false),
                };
                // lwIP's client restarts the clock at every ACK; a release
                // (an expiry) zeroes it too, but is not bound.
                let renewed = used < self.lease_used && bound;
                self.lease_used = used;
                if renewed {
                    act(At::WatchRenewed, self.event(Kind::Renew, 0, used, 0))
                } else {
                    act(At::WatchSampled, Action::Link)
                }
            }
            At::WatchRenewed => act(At::WatchSampled, Action::Link),
            At::WatchSampled => {
                let (associated, leased) = match done {
                    Done::Link { associated, leased } => (associated, leased),
                    _ => (false, false),
                };
                let verdict = self.judge(now, wishes, associated, leased);
                self.probe = Probe::None;
                match verdict {
                    Verdict::Leave(Cause::Restart) => go(At::Leave {
                        then: AfterLeave::Park,
                    }),
                    // Battery mode: no loss to count.
                    Verdict::Leave(Cause::Radio) => {
                        self.entry = Entry::Associated;
                        go(At::RadioDown)
                    }
                    Verdict::Leave(cause) => {
                        let event = self.event(
                            Kind::Lost,
                            i32::from(cause.code()),
                            since(self.joined_at),
                            0,
                        );
                        act(At::LostReported { cause }, event)
                    }
                    Verdict::Probe => act(At::GatewayRead, Action::Gateway),
                    Verdict::Settled => {
                        self.backoff = BACKOFF_MIN_MS;
                        go(At::WatchWait { waited: 0 })
                    }
                    Verdict::Hold => go(At::WatchWait { waited: 0 }),
                }
            }
            // In polls, so a restart's leave (or battery mode's) starts
            // within one rather than at the next watch tick.
            At::WatchWait { waited } if waited < WATCH_MS && !wishes.interrupting() => act(
                At::WatchWait {
                    waited: waited + POLL_MS,
                },
                Action::Delay(POLL_MS),
            ),
            At::WatchWait { .. } => go(At::WatchTick),

            At::GatewayRead => match done {
                Done::Gateway(gateway) if gateway != 0 => {
                    self.seq = self.seq.wrapping_add(1);
                    let echo = Action::Echo {
                        gateway,
                        seq: self.seq,
                    };
                    act(
                        At::Echoed {
                            gateway,
                            begin: now,
                        },
                        echo,
                    )
                }
                _ => {
                    let event = self.event(Kind::Gateway, Echo::NoGateway.code(), 0, 0);
                    act(
                        At::GatewayReported {
                            probe: Echo::NoGateway.probe(),
                        },
                        event,
                    )
                }
            },
            At::Echoed { gateway, begin } => {
                // The machine asked for an echo, so anything else is one
                // that never left.
                let outcome = match done {
                    Done::Echo(outcome) => outcome,
                    _ => Echo::NoSocket,
                };
                let event = self.event(Kind::Gateway, outcome.code(), gateway, since(begin));
                act(
                    At::GatewayReported {
                        probe: outcome.probe(),
                    },
                    event,
                )
            }
            At::GatewayReported { probe } => {
                self.probe = probe;
                go(At::WatchTick)
            }

            At::LostReported { cause } => act(At::LostStopped { cause }, Action::DhcpStop),
            At::LostStopped { cause: Cause::Link } => act(At::Backoff, Action::ReleaseIp),
            // Still associated: leave, so the next attempt joins afresh.
            At::LostStopped { .. } => act(At::LostReleased, Action::ReleaseIp),
            At::LostReleased => act(At::Backoff, Action::Disconnect),

            At::Backoff => {
                let event = self.event(Kind::Backoff, 0, self.backoff, 0);
                act(At::BackoffWait { waited: 0 }, event)
            }
            At::BackoffWait { waited } if waited < self.backoff && !wishes.interrupting() => act(
                At::BackoffWait {
                    waited: waited + POLL_MS,
                },
                Action::Delay(POLL_MS),
            ),
            At::BackoffWait { .. } => {
                self.backoff = self.backoff.saturating_mul(2).min(BACKOFF_MAX_MS);
                go(At::Top)
            }

            // The disconnect sends the access point a (protected)
            // deauthentication from the driver's command thread; the wait
            // lets it go before the reset or the radio does. Before a
            // restart the watchdog feeder has already stopped: if anything
            // here hangs, the reset comes all the same.
            At::Leave { then } => act(At::LeaveStopped { then, begin: now }, Action::DhcpStop),
            At::LeaveStopped { then, begin } => {
                act(At::LeaveReleased { then, begin }, Action::ReleaseIp)
            }
            At::LeaveReleased { then, begin } => {
                act(At::LeaveDisconnected { then, begin }, Action::Disconnect)
            }
            At::LeaveDisconnected { then, begin } => {
                let result = done.status();
                act(
                    At::LeavePolled {
                        then,
                        begin,
                        result,
                    },
                    Action::Associated,
                )
            }
            At::LeavePolled {
                then,
                begin,
                result,
            } => {
                if done.associated() && since(begin) < LEAVE_WAIT_MS {
                    let waited = At::LeaveWaited {
                        then,
                        begin,
                        result,
                    };
                    return act(waited, Action::Delay(POLL_MS));
                }
                let checked = At::LeaveChecked {
                    then,
                    begin,
                    result,
                };
                act(checked, Action::Associated)
            }
            At::LeaveWaited {
                then,
                begin,
                result,
            } => act(
                At::LeavePolled {
                    then,
                    begin,
                    result,
                },
                Action::Associated,
            ),
            At::LeaveChecked {
                then,
                begin,
                result,
            } => {
                let still = u32::from(done.associated());
                let event = self.event(Kind::Leave, result, still, since(begin));
                let next = match then {
                    AfterLeave::AttemptFailed => At::JoinFailed,
                    AfterLeave::Park => At::Parked,
                    AfterLeave::RadioSettle => At::Hold {
                        ms: RADIO_LEAVE_SETTLE_MS,
                        waited: 0,
                        then: AfterHold::RadioOff,
                    },
                };
                act(next, event)
            }

            At::RadioDown => match self.entry {
                Entry::Unassociated => go(At::Hold {
                    ms: RADIO_SETTLE_MS,
                    waited: 0,
                    then: AfterHold::Settled,
                }),
                Entry::Associated => act(At::RadioTurn, Action::TurnBegin),
            },
            // A hold ends early only for a restart.
            At::Hold { ms, waited, then } if waited < ms && !wishes.restarting => act(
                At::Hold {
                    ms,
                    waited: waited + POLL_MS,
                    then,
                },
                Action::Delay(POLL_MS),
            ),
            At::Hold { then, .. } => go(match then {
                AfterHold::Settled => At::RadioSettled,
                AfterHold::RadioOff => At::RadioOff,
                AfterHold::RadioRetry => At::RadioWait,
            }),
            At::RadioSettled if wishes.restarting => act(At::Parked, Action::Park),
            // The adapter came back meanwhile: the radio is still on.
            At::RadioSettled if wishes.radio == Radio::On => self.radio_back(),
            At::RadioSettled => act(At::RadioTurn, Action::TurnBegin),
            At::RadioTurn => match self.entry {
                Entry::Associated => go(At::Leave {
                    then: AfterLeave::RadioSettle,
                }),
                Entry::Unassociated => go(At::RadioOff),
            },
            // `wifi_off` stops lwIP's DHCP client and takes the interface
            // down, so the device API's address goes with it.
            At::RadioOff => act(At::RadioReleased { begin: now }, Action::ReleaseIp),
            At::RadioReleased { begin } => act(At::RadioOffDone { begin }, Action::RadioOff),
            At::RadioOffDone { begin } => {
                let off = done.status();
                act(At::RadioOffEnded { begin, off }, Action::TurnEnd)
            }
            At::RadioOffEnded { begin, off } => {
                let event = self.event(Kind::Off, off, 0, since(begin));
                self.failed = 0;
                act(At::RadioWait, event)
            }
            At::RadioWait if wishes.radio == Radio::Off && !wishes.restarting => {
                act(At::RadioWait, Action::Delay(POLL_MS))
            }
            // A restart asked for meanwhile parks the task with the radio
            // off.
            At::RadioWait if wishes.restarting => act(At::Parked, Action::Park),
            At::RadioWait => act(At::RadioOnTurn, Action::TurnBegin),
            At::RadioOnTurn => act(At::RadioOnDone { begin: now }, Action::RadioOn),
            // 0 started, 1 already running; below 0 the driver failed.
            At::RadioOnDone { begin } => match done.status() {
                on if on >= 0 => act(At::RadioCounted { begin, on }, Action::PowerSaveOff),
                on => go(At::RadioCounted { begin, on }),
            },
            At::RadioCounted { begin, on } => {
                self.failed = if on >= 0 { 0 } else { self.failed + 1 };
                let turn = if self.failed >= RADIO_TRIES {
                    Action::RadioFailed
                } else {
                    Action::TurnEnd
                };
                act(At::RadioTurnEnded { begin, on }, turn)
            }
            At::RadioTurnEnded { begin, on } => {
                let event = self.event(Kind::On, on, self.failed, since(begin));
                act(At::RadioReported { on }, event)
            }
            At::RadioReported { on } if on >= 0 => self.radio_back(),
            At::RadioReported { .. } if self.failed >= RADIO_TRIES => act(At::Parked, Action::Park),
            At::RadioReported { .. } => go(At::Hold {
                ms: RADIO_RETRY_MS,
                waited: 0,
                then: AfterHold::RadioRetry,
            }),

            At::Parked => act(At::Parked, Action::Park),
            At::Stopped => act(At::Stopped, Action::Stop),
        }
    }

    /// One watch tick's verdict. Once a restart was asked for, always a
    /// leave for [`Cause::Restart`], and while the radio should be off, one
    /// for [`Cause::Radio`]; otherwise the join's [`Liveness`], with the
    /// probe the last tick asked for.
    fn judge(&mut self, now: u32, wishes: Wishes, associated: bool, leased: bool) -> Verdict {
        if wishes.restarting {
            self.live = false;
            return Verdict::Leave(Cause::Restart);
        }
        if wishes.radio == Radio::Off {
            self.live = false;
            return Verdict::Leave(Cause::Radio);
        }

        let verdict = self.watch.step(now, associated, leased, self.probe);
        self.live = self.watch.live();
        verdict
    }
}
