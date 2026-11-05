//! The Wi-Fi task's flow against a scripted driver: the order of its vendor
//! calls, its waits and events, and where it looks at the restart and
//! battery mode's wishes.

use std::collections::VecDeque;

use wellfed_core::echo::Outcome as Echo;
use wellfed_core::liveness::{Cause, Radio};
use wellfed_core::wifi_task::{
    Action, Attempt, BACKOFF_MAX_MS, BACKOFF_MIN_MS, Done, Event, Kind, POLL_MS, SCAN_TIMEOUT,
    Sighting, WifiTask, Wishes,
};

const SSID: &[u8] = b"home";
const GATEWAY: u32 = 0xc0a8_0101;
const IP: u32 = 0xc0a8_0123;
/// The scripted access point's security (`RTW_SECURITY_WPA2_AES_PSK`).
const SECURITY: u32 = 0x0040_0004;

/// What the scripted driver answers. The defaults are a network that
/// joins, leases and answers its gateway.
struct Net {
    boot_on: i32,
    /// `wifi_on` after battery mode, in order; 0 once they run out.
    radio_on: VecDeque<i32>,
    /// The credentials' source, `None` for nothing to join.
    select: Option<u32>,
    scan_start: i32,
    /// Sighting reads before the scan completes.
    scan_polls: u32,
    /// What the scan sees: SSID, RSSI and security.
    aps: Vec<(&'static [u8], i32, u32)>,
    connect: i32,
    dhcp: u8,
    channel: Option<i32>,
    bssid: Option<[u8; 6]>,
    /// lwIP's lease clock and bound, read in order; then the last again.
    lease: VecDeque<(u32, bool)>,
    associated: bool,
    leased: bool,
    gateway: u32,
    /// Echo outcomes in order; answered once they run out.
    echoes: VecDeque<Echo>,
    /// Association reads after a disconnect that still find it up.
    lingers: u32,
}

impl Default for Net {
    fn default() -> Self {
        Self {
            boot_on: 0,
            radio_on: VecDeque::new(),
            select: Some(1),
            scan_start: 0,
            scan_polls: 2,
            aps: vec![(SSID, -60, SECURITY)],
            connect: 0,
            dhcp: 2,
            channel: Some(6),
            bssid: Some([0x02, 0x11, 0x22, 0x33, 0x44, 0x55]),
            lease: VecDeque::from([(10, true)]),
            associated: true,
            leased: true,
            gateway: GATEWAY,
            echoes: VecDeque::new(),
            lingers: 0,
        }
    }
}

/// The machine, a clock and the scripted driver, with every action taken.
struct Task {
    machine: WifiTask,
    now: u32,
    wishes: Wishes,
    done: Done,
    net: Net,
    sighting: Sighting,
    polls: u32,
    trace: Vec<Action>,
    /// When each action in `trace` was asked for.
    times: Vec<u32>,
}

impl Task {
    fn new(net: Net) -> Self {
        Self {
            machine: WifiTask::new(),
            now: 1_000,
            wishes: Wishes::NONE,
            done: Done::Nothing,
            net,
            sighting: Sighting::default(),
            polls: 0,
            trace: Vec::new(),
            times: Vec::new(),
        }
    }

    /// One step, answered as the driver would.
    fn next(&mut self) -> Action {
        let action = self.machine.step(self.now, self.wishes, self.done);
        self.trace.push(action);
        self.times.push(self.now);
        self.done = self.answer(action);
        action
    }

    fn answer(&mut self, action: Action) -> Done {
        let net = &mut self.net;
        match action {
            Action::RadioOn if self.trace.len() <= 2 => Done::Status(net.boot_on),
            Action::RadioOn => Done::Status(net.radio_on.pop_front().unwrap_or(0)),
            Action::RadioOff => Done::Status(0),
            Action::Disconnect => {
                net.associated = false;
                Done::Status(0)
            }
            Action::Associated => {
                let still = net.lingers > 0;
                net.lingers = net.lingers.saturating_sub(1);
                Done::Associated(still)
            }
            Action::Select => Done::Selected(net.select),
            Action::Scan => {
                self.sighting.restart();
                self.polls = 0;
                Done::Status(net.scan_start)
            }
            Action::Sighting => {
                self.polls += 1;
                if self.polls > net.scan_polls && !self.sighting.done {
                    for (ssid, rssi, security) in &net.aps {
                        self.sighting.offer(SSID, ssid, *rssi, *security);
                    }
                    self.sighting.complete();
                }
                Done::Sighting(self.sighting)
            }
            Action::Connect { .. } => {
                net.associated = net.connect == 0;
                Done::Status(net.connect)
            }
            Action::DhcpStart => Done::Dhcp {
                state: net.dhcp,
                ip: if net.dhcp == 2 { IP } else { 0 },
            },
            Action::AccessPoint => Done::AccessPoint {
                channel: net.channel,
                bssid: net.bssid,
            },
            Action::Lease => {
                let (used, bound) = if net.lease.len() > 1 {
                    net.lease.pop_front().unwrap()
                } else {
                    net.lease[0]
                };
                Done::Lease { used, bound }
            }
            Action::Link => Done::Link {
                associated: net.associated,
                leased: net.leased,
            },
            Action::Gateway => Done::Gateway(net.gateway),
            Action::Echo { .. } => {
                let outcome = net.echoes.pop_front().unwrap_or(Echo::Answered);
                self.now += if outcome == Echo::Silent { 1_000 } else { 3 };
                Done::Echo(outcome)
            }
            Action::Delay(ms) => {
                self.now += ms;
                Done::Nothing
            }
            _ => Done::Nothing,
        }
    }

    /// Steps until `found` holds for an action, which is returned.
    fn until(&mut self, found: impl Fn(&Action) -> bool) -> Action {
        for _ in 0..100_000 {
            let action = self.next();
            if found(&action) {
                return action;
            }
        }
        panic!(
            "never reached; last {:?}",
            &self.trace[self.trace.len() - 5..]
        );
    }

    /// Steps until the next event of `kind`.
    fn until_event(&mut self, kind: Kind) -> Event {
        match self.until(|a| matches!(a, Action::Record(e) if e.kind == kind)) {
            Action::Record(event) => event,
            _ => unreachable!(),
        }
    }

    /// The actions from the next one up to and including the first that
    /// `found` holds for.
    fn take_until(&mut self, found: impl Fn(&Action) -> bool) -> Vec<Action> {
        let from = self.trace.len();
        self.until(found);
        self.trace[from..].to_vec()
    }

    fn events(&self, kind: Kind) -> Vec<Event> {
        self.trace
            .iter()
            .filter_map(|a| match a {
                Action::Record(e) if e.kind == kind => Some(*e),
                _ => None,
            })
            .collect()
    }

    /// Where the last `action` is in the trace.
    fn at(&self, action: Action) -> usize {
        self.trace.iter().rposition(|a| *a == action).unwrap()
    }

    fn count(&self, action: Action) -> usize {
        self.trace.iter().filter(|a| **a == action).count()
    }

    /// Joined, its gateway answered once and the join settled.
    fn settled(net: Net) -> Self {
        let mut task = Self::new(net);
        task.until(|a| *a == Action::Outcome(Attempt::Joined));
        task.until_event(Kind::Gateway);
        task.until(|a| *a == Action::Delay(POLL_MS));
        assert!(task.machine.live());
        task
    }
}

fn delays(actions: &[Action]) -> u32 {
    actions
        .iter()
        .map(|a| match a {
            Action::Delay(ms) => *ms,
            _ => 0,
        })
        .sum()
}

fn is_event(kind: Kind) -> impl Fn(&Action) -> bool {
    move |a| matches!(a, Action::Record(e) if e.kind == kind)
}

#[test]
fn the_boot_brings_the_radio_up_then_joins_and_watches() {
    let mut task = Task::new(Net::default());

    let boot = task.take_until(|a| *a == Action::Select);
    assert_eq!(boot[..2], [Action::NetInit, Action::RadioOn]);
    let Action::Record(on) = boot[2] else {
        panic!("{boot:?}")
    };
    assert_eq!(
        (on.kind, on.attempt, on.result, on.value),
        (Kind::On, 0, 0, 0)
    );
    assert_eq!(boot[3..], [Action::PowerSaveOff, Action::Select]);

    let join = task.take_until(|a| matches!(a, Action::Outcome(_)));
    assert_eq!(join[0], Action::Scan);
    let kinds: Vec<_> = join
        .iter()
        .filter(|a| !matches!(a, Action::Record(_) | Action::Sighting | Action::Delay(_)))
        .copied()
        .collect();
    assert_eq!(
        kinds,
        [
            Action::Scan,
            Action::Connect {
                attempt: 1,
                security: SECURITY
            },
            Action::DhcpStart,
            Action::AccessPoint,
            Action::Outcome(Attempt::Joined),
        ]
    );

    let scan = task.events(Kind::Scan)[0];
    assert_eq!(
        (
            scan.attempt,
            scan.result,
            scan.value,
            scan.rssi,
            scan.security
        ),
        (1, 0, 1, -60, SECURITY)
    );
    assert_eq!(scan.ms, 2 * POLL_MS, "two polls before the scan completed");
    let connect = task.events(Kind::Connect)[0];
    assert_eq!(
        (connect.result, connect.value),
        (0, 1),
        "the source, never the credentials"
    );
    let dhcp = task.events(Kind::Dhcp)[0];
    assert_eq!((dhcp.result, dhcp.value), (2, IP));
    let ap = task.events(Kind::AccessPoint)[0];
    assert_eq!((ap.result, ap.value, ap.ms), (6, 0x2233_4455, 0));
    assert!(!task.machine.live(), "not live until it settles");

    // The watch: the lease clock, then the link, then the echo the first
    // tick asks for, and the tick after it at once.
    let watch = task.take_until(|a| matches!(a, Action::Delay(_)));
    assert_eq!(
        watch,
        [
            Action::Lease,
            Action::Lease,
            Action::Link,
            Action::Gateway,
            Action::Echo {
                gateway: GATEWAY,
                seq: 1
            },
            watch[5],
            Action::Lease,
            Action::Link,
            Action::Delay(POLL_MS),
        ]
    );
    let gateway = task.events(Kind::Gateway)[0];
    assert_eq!((gateway.result, gateway.value, gateway.ms), (0, GATEWAY, 3));
    assert!(task.machine.live(), "settled on the answer");

    // A watch tick every 500 ms, in polls.
    let tick = task.take_until(|a| *a == Action::Link);
    assert_eq!(POLL_MS + delays(&tick), 500);
}

#[test]
fn a_radio_that_never_comes_up_ends_the_task() {
    let mut task = Task::new(Net {
        boot_on: -1,
        ..Net::default()
    });

    assert_eq!(task.take_until(|a| *a == Action::Stop).len(), 4);
    assert_eq!(task.events(Kind::On)[0].result, -1);
    for _ in 0..3 {
        assert_eq!(task.next(), Action::Stop);
    }
}

#[test]
fn failed_attempts_back_off_doubling_to_the_longest() {
    let mut task = Task::new(Net {
        aps: vec![(b"other", -40, 4)],
        ..Net::default()
    });

    for (attempt, backoff) in [1000, 2000, 4000, 8000, 16000, 30000, 30000]
        .into_iter()
        .enumerate()
    {
        let event = task.until_event(Kind::Backoff);
        assert_eq!((event.attempt, event.value), (attempt as u32 + 1, backoff));
        let waited = task.take_until(|a| *a == Action::Select);
        assert_eq!(delays(&waited), backoff, "attempt {}", attempt + 1);
    }
    assert_eq!(task.count(Action::Outcome(Attempt::Failed)), 7);
    assert!(
        !task
            .trace
            .iter()
            .any(|a| matches!(a, Action::Connect { .. })),
        "never associates with a network the scan did not see"
    );
    assert!(task.events(Kind::Scan).iter().all(|e| e.value == 0));
}

#[test]
fn with_nothing_to_join_it_neither_scans_nor_associates() {
    let mut task = Task::new(Net {
        select: None,
        ..Net::default()
    });

    task.until(|a| *a == Action::Select);
    assert_eq!(task.next(), Action::Outcome(Attempt::Failed));
    for _ in 0..3 {
        let event = task.until_event(Kind::Backoff);
        assert_eq!(event.value, BACKOFF_MAX_MS);
    }
    assert!(!task.trace.contains(&Action::Scan));
}

#[test]
fn the_strongest_access_point_naming_the_network_is_joined() {
    let mut task = Task::new(Net {
        aps: vec![
            (SSID, -70, 2),
            (b"other", -30, 7),
            (b"hom", -20, 7),
            (SSID, -50, 5),
            (SSID, -65, 6),
        ],
        ..Net::default()
    });

    let connect = task.until(|a| matches!(a, Action::Connect { .. }));
    assert_eq!(
        connect,
        Action::Connect {
            attempt: 1,
            security: 5
        }
    );
    assert_eq!(task.events(Kind::Scan)[0].rssi, -50);
}

#[test]
fn a_scan_that_never_completes_times_out() {
    let mut task = Task::new(Net {
        scan_polls: u32::MAX,
        ..Net::default()
    });

    task.until(|a| *a == Action::Scan);
    let scan = task.take_until(is_event(Kind::Scan));
    assert_eq!(delays(&scan), 10_000);
    let Some(Action::Record(event)) = scan.last() else {
        panic!()
    };
    assert_eq!(
        (event.result, event.value, event.ms),
        (SCAN_TIMEOUT, 0, 10_000)
    );
    assert_eq!(task.next(), Action::Outcome(Attempt::Failed));

    // One that fails to start is not waited for.
    let mut task = Task::new(Net {
        scan_start: -1,
        ..Net::default()
    });
    task.until(|a| *a == Action::Scan);
    let scan = task.take_until(is_event(Kind::Scan));
    assert_eq!(delays(&scan), 0);
    assert_eq!(task.events(Kind::Scan)[0].result, -1);
}

/// A scan that finds nothing still reports the strength and security the
/// last one found, as the shim's event always did.
#[test]
fn a_scan_event_keeps_the_last_sighting_when_not_found() {
    let mut task = Task::new(Net {
        connect: -1,
        ..Net::default()
    });
    task.until_event(Kind::Scan);
    task.net.aps.clear();

    let event = task.until_event(Kind::Scan);
    assert_eq!(
        (event.value, event.rssi, event.security),
        (0, -60, SECURITY)
    );
}

#[test]
fn the_static_fallback_address_is_refused() {
    let mut task = Task::new(Net {
        dhcp: 1,
        ..Net::default()
    });

    task.until(|a| *a == Action::DhcpStart);
    let refused = task.take_until(|a| matches!(a, Action::Outcome(_)));
    assert_eq!(
        refused[1..],
        [
            Action::DhcpStop,
            Action::ReleaseIp,
            Action::Disconnect,
            Action::Outcome(Attempt::Failed),
        ]
    );
    let dhcp = task.events(Kind::Dhcp)[0];
    assert_eq!((dhcp.result, dhcp.value), (1, 0));
    assert!(task.events(Kind::AccessPoint).is_empty());
}

#[test]
fn a_failed_connect_goes_straight_to_the_backoff() {
    let mut task = Task::new(Net {
        connect: -1,
        ..Net::default()
    });

    task.until(|a| matches!(a, Action::Connect { .. }));
    let failed = task.take_until(is_event(Kind::Backoff));
    assert!(matches!(failed[0], Action::Record(e) if e.kind == Kind::Connect && e.result == -1));
    assert_eq!(failed[1], Action::Outcome(Attempt::Failed));
    assert_eq!(failed.len(), 3);
}

#[test]
fn the_access_point_reports_a_channel_only_when_it_is_one() {
    for (read, result) in [
        (Some(1), 1),
        (Some(14), 14),
        (Some(36), 36),
        (Some(177), 177),
        (Some(0), -1),
        (Some(15), -1),
        (Some(35), -1),
        (Some(178), -1),
        (Some(-250), -1),
        (None, -1),
    ] {
        let mut task = Task::new(Net {
            channel: read,
            bssid: None,
            ..Net::default()
        });
        let event = task.until_event(Kind::AccessPoint);
        assert_eq!((event.result, event.value), (result, 0), "{read:?}");
    }
}

/// A settled join resets the backoff; a join lost before it settled does
/// not.
#[test]
fn a_settled_join_resets_the_backoff() {
    let mut task = Task::new(Net {
        connect: -1,
        ..Net::default()
    });
    for _ in 0..3 {
        task.until_event(Kind::Backoff);
    }
    task.net.connect = 0;
    task.until_event(Kind::Gateway);
    task.until(|a| *a == Action::Delay(POLL_MS));

    task.net.associated = false;
    let lost = task.until_event(Kind::Lost);
    assert_eq!(lost.result, i32::from(Cause::Link.code()));
    // From the step after the outcome to the loss.
    let joined = task.at(Action::Outcome(Attempt::Joined)) + 1;
    assert_eq!(lost.value, task.now - task.times[joined]);
    assert!(lost.value > 500);
    let after = task.take_until(is_event(Kind::Backoff));
    assert_eq!(after[..2], [Action::DhcpStop, Action::ReleaseIp]);
    assert!(
        !after.contains(&Action::Disconnect),
        "the association is already gone"
    );
    assert_eq!(
        task.events(Kind::Backoff).last().unwrap().value,
        BACKOFF_MIN_MS
    );

    // Lost before it settled: the doubling goes on.
    let mut task = Task::new(Net {
        connect: -1,
        ..Net::default()
    });
    task.until_event(Kind::Backoff);
    task.net.connect = 0;
    task.net.leased = false;
    let lost = task.until_event(Kind::Lost);
    assert_eq!(lost.result, i32::from(Cause::Lease.code()));
    let after = task.take_until(is_event(Kind::Backoff));
    assert_eq!(
        after[..3],
        [Action::DhcpStop, Action::ReleaseIp, Action::Disconnect],
        "still associated: leave, so the next attempt joins afresh"
    );
    assert_eq!(
        task.events(Kind::Backoff).last().unwrap().value,
        2 * BACKOFF_MIN_MS
    );
}

#[test]
fn a_silent_gateway_is_left_once_it_has_answered() {
    let mut task = Task::settled(Net::default());
    task.net.echoes = VecDeque::from([Echo::Silent, Echo::Silent, Echo::Silent]);

    let lost = task.until_event(Kind::Lost);
    assert_eq!(lost.result, i32::from(Cause::Gateway.code()));
    let silent = task.events(Kind::Gateway);
    assert_eq!(
        silent[1..]
            .iter()
            .map(|e| (e.result, e.ms))
            .collect::<Vec<_>>(),
        [(-4, 1000); 3]
    );
    assert!(!task.machine.live());
}

#[test]
fn a_lease_without_a_gateway_settles_without_an_echo() {
    let mut task = Task::new(Net {
        gateway: 0,
        ..Net::default()
    });

    task.until(|a| *a == Action::Outcome(Attempt::Joined));
    for _ in 0..3 {
        let event = task.until_event(Kind::Gateway);
        assert_eq!((event.result, event.value, event.ms), (-1, 0, 0));
    }
    task.until(|a| matches!(a, Action::Delay(_)));
    assert!(task.machine.live());
    assert!(!task.trace.iter().any(|a| matches!(a, Action::Echo { .. })));
}

/// An echo that never left asks nothing of the gateway, and the sequence
/// runs on across joins.
#[test]
fn echoes_number_on_across_joins() {
    let mut task = Task::settled(Net::default());
    task.net.echoes = VecDeque::from([Echo::NoSocket, Echo::NotSent]);
    for _ in 0..2 {
        task.until_event(Kind::Gateway);
    }
    assert_eq!(
        task.events(Kind::Gateway)
            .iter()
            .map(|e| e.result)
            .collect::<Vec<_>>(),
        [0, -2, -3]
    );
    assert!(task.machine.live(), "an unsent probe is no miss");

    task.net.associated = false;
    task.until_event(Kind::Lost);
    let echo = task.until(|a| matches!(a, Action::Echo { .. }));
    assert_eq!(
        echo,
        Action::Echo {
            gateway: GATEWAY,
            seq: 4
        }
    );
}

#[test]
fn a_renewal_is_the_lease_clock_going_back_while_bound() {
    let mut task = Task::new(Net {
        lease: VecDeque::from([
            (40, true),
            (41, true),
            (42, true),
            (0, true),
            (1, true),
            (30, true),
            (0, false),
            (0, false),
            (5, true),
        ]),
        ..Net::default()
    });

    task.until(|a| *a == Action::Outcome(Attempt::Joined));
    for _ in 0..9 {
        task.until(|a| *a == Action::Link);
    }
    let renewals = task.events(Kind::Renew);
    assert_eq!(renewals.len(), 1, "a release that zeroes the clock is none");
    assert_eq!(
        (renewals[0].result, renewals[0].value, renewals[0].ms),
        (0, 0, 0)
    );
    let at = task
        .trace
        .iter()
        .position(|a| matches!(a, Action::Record(e) if e.kind == Kind::Renew))
        .unwrap();
    assert_eq!(task.trace[at - 1], Action::Lease);
    assert_eq!(task.trace[at + 1], Action::Link);
}

fn restarting() -> Wishes {
    Wishes {
        restarting: true,
        radio: Radio::On,
    }
}

fn radio_off() -> Wishes {
    Wishes {
        restarting: false,
        radio: Radio::Off,
    }
}

#[test]
fn a_restart_leaves_a_joined_link_then_parks() {
    let mut task = Task::settled(Net {
        lingers: 3,
        ..Net::default()
    });
    task.wishes = restarting();

    let leave = task.take_until(is_event(Kind::Leave));
    let from = leave.iter().position(|a| *a == Action::DhcpStop).unwrap();
    assert_eq!(delays(&leave[..from]), 0, "the watch's poll ends for it");
    assert_eq!(leave[from - 1], Action::Link);
    assert_eq!(
        leave[from..leave.len() - 1],
        [
            Action::DhcpStop,
            Action::ReleaseIp,
            Action::Disconnect,
            Action::Associated,
            Action::Delay(POLL_MS),
            Action::Associated,
            Action::Delay(POLL_MS),
            Action::Associated,
            Action::Delay(POLL_MS),
            Action::Associated,
            Action::Associated,
        ],
        "waits while the driver still reports the association, then reads it for the event"
    );
    let event = task.events(Kind::Leave)[0];
    assert_eq!((event.result, event.value, event.ms), (0, 0, 3 * POLL_MS));
    assert!(task.events(Kind::Lost).is_empty(), "a restart is no loss");
    assert!(!task.machine.live());
    for _ in 0..3 {
        assert_eq!(task.next(), Action::Park);
    }
}

#[test]
fn a_leave_waits_for_the_association_at_most_a_second() {
    let mut task = Task::settled(Net {
        lingers: u32::MAX,
        ..Net::default()
    });
    task.wishes = restarting();

    task.until(|a| *a == Action::Disconnect);
    let leave = task.take_until(is_event(Kind::Leave));
    assert_eq!(delays(&leave), 1_000);
    let event = task.events(Kind::Leave)[0];
    assert_eq!((event.value, event.ms), (1, 1_000));
    assert_eq!(task.next(), Action::Park);
}

/// A restart wins wherever the task looks: no new scan, association or
/// radio change starts once one is asked for.
#[test]
fn a_restart_is_seen_between_and_inside_attempts() {
    // Between attempts: the backoff ends at once and the task parks.
    let mut task = Task::new(Net {
        connect: -1,
        ..Net::default()
    });
    task.until_event(Kind::Backoff);
    task.next();
    task.wishes = Wishes {
        restarting: true,
        radio: Radio::Off,
    };
    assert_eq!(task.next(), Action::Park);

    // During the scan: no association.
    let mut task = Task::new(Net::default());
    task.until(|a| *a == Action::Scan);
    task.wishes = restarting();
    let rest = task.take_until(|a| *a == Action::Park);
    assert!(!rest.iter().any(|a| matches!(a, Action::Connect { .. })));
    assert!(rest.contains(&Action::Outcome(Attempt::Failed)));
    assert_eq!(
        delays(&rest[rest.len() - 3..]),
        0,
        "the backoff ends at once"
    );

    // While it associated: leave the fresh association at once.
    let mut task = Task::new(Net::default());
    task.until(|a| matches!(a, Action::Connect { .. }));
    task.wishes = restarting();
    let rest = task.take_until(|a| *a == Action::Park);
    assert_eq!(rest[1], Action::DhcpStop);
    assert!(!rest.contains(&Action::DhcpStart));
    assert!(matches!(rest[6], Action::Record(e) if e.kind == Kind::Leave));
    assert_eq!(rest[7], Action::Outcome(Attempt::Failed));

    // While the turn waited: the select returns into a restart.
    let mut task = Task::new(Net::default());
    task.until(|a| *a == Action::Select);
    task.wishes = restarting();
    assert_eq!(task.next(), Action::Outcome(Attempt::Failed));
    assert!(
        !task
            .take_until(|a| *a == Action::Park)
            .contains(&Action::Scan)
    );
}

#[test]
fn the_radio_off_between_attempts_settles_then_turns_it_off() {
    let mut task = Task::new(Net {
        connect: -1,
        ..Net::default()
    });
    task.until_event(Kind::Backoff);
    task.wishes = radio_off();

    let off = task.take_until(is_event(Kind::Off));
    assert_eq!(
        delays(&off[..off.len() - 5]),
        2_000,
        "settles outside the turn"
    );
    assert_eq!(
        off[off.len() - 5..off.len() - 1],
        [
            Action::TurnBegin,
            Action::ReleaseIp,
            Action::RadioOff,
            Action::TurnEnd
        ]
    );
    let event = task.events(Kind::Off)[0];
    assert_eq!((event.attempt, event.result), (1, 0));

    // Off until asked on again.
    for _ in 0..100 {
        assert_eq!(task.next(), Action::Delay(POLL_MS));
    }
    task.wishes = Wishes::NONE;
    let on = task.take_until(|a| *a == Action::Select);
    assert_eq!(
        on[..4],
        [
            Action::TurnBegin,
            Action::RadioOn,
            Action::PowerSaveOff,
            Action::TurnEnd
        ]
    );
    assert!(
        matches!(on[4], Action::Record(e) if e.kind == Kind::On && e.result == 0 && e.value == 0 && e.attempt == 1)
    );
    assert_eq!(on.len(), 6, "straight to the next attempt");
    task.until_event(Kind::Backoff);
    assert_eq!(
        task.events(Kind::Backoff).last().unwrap().value,
        BACKOFF_MIN_MS,
        "the backoff starts over"
    );
}

#[test]
fn the_radio_back_during_the_settle_never_goes_off() {
    let mut task = Task::new(Net {
        connect: -1,
        ..Net::default()
    });
    task.until_event(Kind::Backoff);
    task.wishes = radio_off();
    task.until(|a| matches!(a, Action::Delay(_)));
    for _ in 0..10 {
        task.next();
    }
    // A hold ends early only for a restart, so the settle runs out first.
    task.wishes = Wishes::NONE;
    let rest = task.take_until(|a| *a == Action::Select);
    assert_eq!(delays(&rest), 2_000 - 11 * POLL_MS);
    assert!(!rest.contains(&Action::TurnBegin) && !rest.contains(&Action::RadioOff));
}

#[test]
fn the_radio_off_while_joined_leaves_inside_the_turn() {
    let mut task = Task::settled(Net::default());
    task.wishes = radio_off();

    let off = task.take_until(is_event(Kind::Off));
    let first = off.iter().position(|a| *a == Action::TurnBegin).unwrap();
    assert!(
        delays(&off[..first]) <= POLL_MS,
        "no settle before the leave"
    );
    let calls: Vec<_> = off[first..]
        .iter()
        .filter(|a| !matches!(a, Action::Delay(_) | Action::Record(_)))
        .copied()
        .collect();
    assert_eq!(
        calls,
        [
            Action::TurnBegin,
            Action::DhcpStop,
            Action::ReleaseIp,
            Action::Disconnect,
            Action::Associated,
            Action::Associated,
            Action::ReleaseIp,
            Action::RadioOff,
            Action::TurnEnd,
        ]
    );
    assert_eq!(
        delays(&off[first..]),
        500,
        "the leave's settle, inside the turn"
    );
    assert_eq!(task.events(Kind::Leave).len(), 1);
    assert!(
        task.events(Kind::Lost).is_empty(),
        "battery mode is no loss"
    );
    assert!(!task.machine.live());

    task.wishes = Wishes::NONE;
    let on = task.take_until(|a| *a == Action::Select);
    assert!(on.contains(&Action::RadioOn));
    let attempts: Vec<_> = task.events(Kind::Scan).iter().map(|e| e.attempt).collect();
    task.until_event(Kind::Scan);
    assert_eq!(
        task.events(Kind::Scan).last().unwrap().attempt,
        attempts[0] + 1
    );
}

#[test]
fn a_radio_that_will_not_come_back_restarts_with_the_turn_held() {
    let mut task = Task::new(Net {
        connect: -1,
        radio_on: VecDeque::from([-1; 6]),
        ..Net::default()
    });
    task.until_event(Kind::Backoff);
    task.wishes = radio_off();
    task.until_event(Kind::Off);
    task.wishes = Wishes::NONE;

    for failed in 1..=5 {
        let tries = task.take_until(is_event(Kind::On));
        assert_eq!(
            tries[tries.len() - 3..tries.len() - 1],
            [Action::RadioOn, Action::TurnEnd],
            "no power save call after a failure"
        );
        let event = task.events(Kind::On).last().copied().unwrap();
        assert_eq!((event.result, event.value), (-1, failed));
        let retry = task.take_until(|a| *a == Action::TurnBegin);
        assert_eq!(delays(&retry), 5_000);
    }
    assert_eq!(task.next(), Action::RadioOn);
    assert_eq!(task.next(), Action::RadioFailed);
    let event = task.until_event(Kind::On);
    assert_eq!((event.result, event.value), (-1, 6));
    assert_eq!(task.next(), Action::Park);
    assert_eq!(
        task.count(Action::TurnEnd),
        6,
        "the off's, then five tries'"
    );
}

#[test]
fn a_radio_back_after_failures_counts_from_zero() {
    let mut task = Task::new(Net {
        connect: -1,
        radio_on: VecDeque::from([-1, -1, 1]),
        ..Net::default()
    });
    task.until_event(Kind::Backoff);
    task.wishes = radio_off();
    task.until_event(Kind::Off);
    task.wishes = Wishes::NONE;

    let values: Vec<_> = (0..3).map(|_| task.until_event(Kind::On)).collect();
    assert_eq!(
        values
            .iter()
            .map(|e| (e.result, e.value))
            .collect::<Vec<_>>(),
        [(-1, 1), (-1, 2), (1, 0)],
        "1 is already running, which is up"
    );
    assert_eq!(task.next(), Action::Select);
}

#[test]
fn a_restart_while_the_radio_is_off_parks() {
    let mut task = Task::new(Net {
        connect: -1,
        ..Net::default()
    });
    task.until_event(Kind::Backoff);
    task.wishes = radio_off();
    task.until_event(Kind::Off);
    task.next();
    task.wishes = Wishes {
        restarting: true,
        radio: Radio::Off,
    };
    assert_eq!(task.next(), Action::Park);

    // During the settle: the hold ends at once.
    let mut task = Task::new(Net {
        connect: -1,
        ..Net::default()
    });
    task.until_event(Kind::Backoff);
    task.wishes = radio_off();
    task.until(|a| matches!(a, Action::Delay(_)));
    task.next();
    task.wishes = restarting();
    assert_eq!(task.next(), Action::Park);
    assert!(!task.trace.contains(&Action::TurnBegin));

    // During a retry: the hold ends at once, the radio still off.
    let mut task = Task::new(Net {
        connect: -1,
        radio_on: VecDeque::from([-1]),
        ..Net::default()
    });
    task.until_event(Kind::Backoff);
    task.wishes = radio_off();
    task.until_event(Kind::Off);
    task.wishes = Wishes::NONE;
    task.until_event(Kind::On);
    task.next();
    task.wishes = restarting();
    assert_eq!(task.next(), Action::Park);
}

/// At the watch, a restart wins over battery mode, and either wins over
/// whatever the link shows.
#[test]
fn a_restart_wins_over_the_radio_at_the_watch() {
    let mut task = Task::settled(Net::default());
    task.wishes = Wishes {
        restarting: true,
        radio: Radio::Off,
    };
    task.net.associated = false;

    let rest = task.take_until(|a| *a == Action::Park);
    assert!(!rest.contains(&Action::TurnBegin) && !rest.contains(&Action::RadioOff));
    assert_eq!(task.events(Kind::Leave).len(), 1);
    assert!(task.events(Kind::Lost).is_empty());
}

/// Every join starts a fresh watch: not live until it settles, and probing
/// at once rather than a period after the last join's probe.
#[test]
fn every_join_is_watched_from_scratch() {
    let mut task = Task::settled(Net::default());
    task.net.associated = false;
    task.until_event(Kind::Lost);
    assert!(!task.machine.live(), "a loss is not live");

    task.until(|a| *a == Action::Outcome(Attempt::Joined));
    assert!(!task.machine.live());
    let first = task.take_until(|a| matches!(a, Action::Echo { .. }));
    assert_eq!(
        first,
        [
            Action::Lease,
            Action::Lease,
            Action::Link,
            Action::Gateway,
            first[4]
        ]
    );
    task.until(|a| matches!(a, Action::Delay(_)));
    assert!(task.machine.live());
}
