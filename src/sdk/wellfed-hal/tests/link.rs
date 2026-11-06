//! The Wi-Fi task's driver against the SDK fake: each step through the
//! shim's wrappers, the scan's callbacks, liveness published to
//! `net_status`, radio changes in the join's turn, the gateway echo, and
//! the shim's events counted by cause.

use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use wellfed_core::echo::Outcome;
use wellfed_core::liveness::{Cause, Radio};
use wellfed_core::wifi_task::{Action, Attempt, Kind};
use wellfed_hal::interlock::{Held, INTERLOCK, Turn};
use wellfed_hal::link::{
    self, Driver, Join, radio_begin, radio_end, radio_failed, wellfed_wifi_scan_complete,
    wellfed_wifi_scan_seen,
};
use wellfed_hal::sdk::{self, JoinCredentials, WifiEvent, fake};
use wellfed_hal::{net_status, supervision};

/// `net_status`, the wishes and the scan are process-wide.
static SERIAL: Mutex<()> = Mutex::new(());

const IP: u32 = 0xc0a8_0123;
const GATEWAY: u32 = 0xc0a8_0101;
const STORED_SSID: &[u8] = b"stored-net";
const STORED_PSK: &[u8] = b"stored-pass";

fn start() -> MutexGuard<'static, ()> {
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    fake::reset_hardware();
    net_status::reset();
    link::reset();
    INTERLOCK.reset();
    guard
}

fn lost(cause: Cause) {
    net_status::observe(&WifiEvent {
        kind: WifiEvent::LOST,
        result: i32::from(cause.code()),
        value: 30_000,
        ..WifiEvent::default()
    });
}

/// The product's half: the build's network unless it has stored
/// credentials, or nothing when told so; every outcome kept.
#[derive(Default)]
struct Product {
    stored: bool,
    nothing: bool,
    selects: u32,
    outcomes: Vec<Attempt>,
}

impl Join for Product {
    unsafe fn select(&mut self, out: &mut JoinCredentials) {
        self.selects += 1;
        if self.nothing {
            *out = JoinCredentials::EMPTY;
        } else if self.stored {
            *out = JoinCredentials {
                ssid: STORED_SSID.as_ptr(),
                ssid_len: STORED_SSID.len() as u32,
                psk: STORED_PSK.as_ptr(),
                psk_len: STORED_PSK.len() as u32,
                source: 1,
            };
        }
    }

    unsafe fn outcome(&mut self, attempt: Attempt) {
        self.outcomes.push(attempt);
    }
}

/// A bench image on a network that joins and leases, whose gateway
/// answers every echo at once.
fn network() {
    fake::with(|h| {
        h.wifi.build = Some((GATEWAY_NET, b"build-pass"));
        h.wifi.dhcp = (2, IP);
        h.wifi.channel = Some(11);
        h.wifi.bssid = Some([0, 0, 0xaa, 0xbb, 0xcc, 0xdd]);
        h.wifi.lease = (5, true);
        h.ipv4 = IP;
        h.gateway = GATEWAY;
    });
}

const GATEWAY_NET: &[u8] = b"home";

/// Turns until `found` holds for the action, which is returned. A scan
/// reports `aps` from the driver's context once it starts.
fn until(
    driver: &mut Driver<Product>,
    aps: &[(&[u8], i32, u32)],
    found: impl Fn(&Action) -> bool,
) -> Action {
    for _ in 0..10_000 {
        // Each echo is answered: the fake queues the reply ahead of it.
        fake::with(|h| {
            if h.icmp_rx.is_empty() {
                let seq = u16::try_from(h.icmp_tx.len() + 1).unwrap();
                h.icmp_rx.push_back((2, reply(GATEWAY, seq)));
            }
        });
        // SAFETY: the test is the only Wi-Fi task.
        let action = unsafe { driver.turn() };
        if action == Action::Scan {
            for (ssid, rssi, security) in aps {
                // SAFETY: the SSID is live for the call.
                unsafe {
                    wellfed_wifi_scan_seen(ssid.as_ptr(), ssid.len() as u32, *rssi, *security)
                };
            }
            wellfed_wifi_scan_complete();
        }
        if found(&action) {
            return action;
        }
    }
    panic!("never reached");
}

fn kinds() -> Vec<u32> {
    fake::with(|h| h.wifi_events.iter().map(|e| e.kind).collect())
}

#[test]
fn the_event_kinds_are_the_rings() {
    for (kind, code) in [
        (Kind::On, WifiEvent::ON),
        (Kind::Scan, WifiEvent::SCAN),
        (Kind::Connect, WifiEvent::CONNECT),
        (Kind::Dhcp, WifiEvent::DHCP),
        (Kind::Lost, WifiEvent::LOST),
        (Kind::Backoff, WifiEvent::BACKOFF),
        (Kind::Gateway, WifiEvent::GATEWAY),
        (Kind::Leave, WifiEvent::LEAVE),
        (Kind::AccessPoint, WifiEvent::AP),
        (Kind::Renew, WifiEvent::RENEW),
        (Kind::Off, WifiEvent::OFF),
    ] {
        assert_eq!(kind.code(), code, "{kind:?}");
    }
}

/// From boot to a live link: every vendor call through the shim, the
/// strongest access point naming the network, and the events in the ring
/// in order, numbered from 1.
#[test]
fn a_join_runs_through_the_shim_and_goes_live() {
    let _guard = start();
    network();
    let mut driver = Driver::new(Product::default());
    let aps: &[(&[u8], i32, u32)] = &[
        (GATEWAY_NET, -70, 4),
        (b"elsewhere", -20, 6),
        (GATEWAY_NET, -40, 5),
        (b"home-5g", -30, 7),
    ];

    until(&mut driver, aps, |a| *a == Action::Outcome(Attempt::Joined));
    assert!(!net_status::read().live, "not live until it settles");
    until(&mut driver, aps, |a| matches!(a, Action::Delay(_)));
    assert!(net_status::read().live);

    fake::with(|h| {
        assert_eq!(
            h.wifi.calls[..9],
            [
                "LwIP_Init",
                "wifi_on",
                "wifi_set_power_mode",
                "wifi_scan_networks",
                "wifi_connect",
                "DHCP_START",
                "wifi_get_channel",
                "wifi_get_ap_bssid",
                "wifi_is_connected_to_ap",
            ]
        );
        assert_eq!(
            h.wifi.connected_with,
            [(GATEWAY_NET.to_vec(), b"build-pass".to_vec(), 5)]
        );
        assert_eq!(h.icmp_tx[0].1, GATEWAY);
    });
    assert_eq!(
        kinds(),
        [
            WifiEvent::ON,
            WifiEvent::SCAN,
            WifiEvent::CONNECT,
            WifiEvent::DHCP,
            WifiEvent::AP,
            WifiEvent::GATEWAY
        ]
    );
    let events = fake::with(|h| h.wifi_events.clone());
    assert_eq!(
        events.iter().map(|e| e.seq).collect::<Vec<_>>(),
        [1, 2, 3, 4, 5, 6]
    );
    assert_eq!(
        (events[1].value, events[1].rssi, events[1].security),
        (1, -40, 5)
    );
    assert_eq!(
        (events[2].value, events[3].value),
        (0, IP),
        "the build's source"
    );
    assert_eq!((events[4].result, events[4].value), (11, 0xaabb_ccdd));
    assert_eq!(events[5].result, 0);
    assert_eq!(driver_product(&driver).selects, 1);

    // The probe drains them as before.
    assert_eq!(sdk::wifi_event(2), sdk::WifiPoll::Event(events[1]));
}

fn driver_product(driver: &Driver<Product>) -> &Product {
    driver.join()
}

/// Stored credentials replace the build's, and the connect event names
/// their source.
#[test]
fn the_products_credentials_are_the_ones_joined() {
    let _guard = start();
    network();
    let mut driver = Driver::new(Product {
        stored: true,
        ..Product::default()
    });

    until(
        &mut driver,
        &[(STORED_SSID, -50, 4), (GATEWAY_NET, -10, 5)],
        |a| matches!(a, Action::Outcome(_)),
    );
    fake::with(|h| {
        assert_eq!(
            h.wifi.connected_with,
            [(STORED_SSID.to_vec(), STORED_PSK.to_vec(), 4)]
        );
        assert_eq!(h.wifi_events[2].value, 1);
    });
    assert_eq!(driver_product(&driver).outcomes, [Attempt::Joined]);
}

#[test]
fn with_nothing_to_join_nothing_scans() {
    let _guard = start();
    network();
    fake::with(|h| h.wifi.build = None);
    let mut driver = Driver::new(Product {
        nothing: true,
        ..Product::default()
    });

    until(
        &mut driver,
        &[],
        |a| matches!(a, Action::Record(e) if e.kind == Kind::Backoff),
    );
    assert_eq!(driver_product(&driver).outcomes, [Attempt::Failed]);
    fake::with(|h| {
        assert!(!h.wifi.calls.contains(&"wifi_scan_networks"));
        assert_eq!(h.wifi_events.last().unwrap().value, 30_000);
    });
}

/// A scan keeps its own copy of the SSID: no access point is matched
/// against credentials the product may be rewriting, and none whose SSID
/// 802.11 would not allow.
#[test]
fn the_scan_matches_its_own_copy_of_the_ssid() {
    let _guard = start();
    network();
    let mut driver = Driver::new(Product::default());
    until(&mut driver, &[], |a| *a == Action::Scan);

    let long = [b'h'; 40];
    // SAFETY: the SSIDs are live for the calls.
    unsafe {
        wellfed_wifi_scan_seen(long.as_ptr(), 40, -10, 9);
        wellfed_wifi_scan_seen(GATEWAY_NET.as_ptr(), 4, -60, 4);
        wellfed_wifi_scan_seen(core::ptr::null(), 0, -5, 9);
    }
    wellfed_wifi_scan_complete();
    let connect = until(&mut driver, &[], |a| matches!(a, Action::Connect { .. }));
    assert_eq!(
        connect,
        Action::Connect {
            attempt: 1,
            security: 4
        }
    );
}

#[test]
fn a_restart_leaves_a_live_link_and_parks() {
    let _guard = start();
    network();
    let mut driver = Driver::new(Product::default());
    until(&mut driver, &[(GATEWAY_NET, -40, 5)], |a| {
        matches!(a, Action::Delay(_))
    });
    assert!(net_status::read().live);

    link::leave_for_restart();
    assert!(link::restarting());
    until(&mut driver, &[], |a| *a == Action::Park);
    assert!(!net_status::read().live);
    fake::with(|h| {
        let at = h
            .wifi
            .calls
            .iter()
            .rposition(|c| *c == "DHCP_STOP")
            .unwrap();
        assert_eq!(
            h.wifi.calls[at..],
            [
                "DHCP_STOP",
                "LwIP_ReleaseIP",
                "wifi_disconnect",
                "wifi_is_connected_to_ap",
                "wifi_is_connected_to_ap",
            ]
        );
        let leave = h.wifi_events.last().unwrap();
        assert_eq!((leave.kind, leave.value), (WifiEvent::LEAVE, 0));
    });
}

/// Battery mode: the radio goes off inside the join's turn, which a feed
/// is refused during, and comes back the same way.
#[test]
fn the_radio_changes_inside_the_joins_turn() {
    let _guard = start();
    network();
    let mut driver = Driver::new(Product::default());
    until(&mut driver, &[(GATEWAY_NET, -40, 5)], |a| {
        matches!(a, Action::Delay(_))
    });

    link::set_radio(Radio::Off);
    until(&mut driver, &[], |a| *a == Action::RadioOff);
    assert_eq!(INTERLOCK.turn(), Turn::Joining);
    assert_eq!(INTERLOCK.claim_feed(u32::MAX), Err(Held::Joining));
    until(&mut driver, &[], |a| *a == Action::TurnEnd);
    assert_eq!(INTERLOCK.turn(), Turn::Free);
    assert!(!net_status::read().live);
    until(
        &mut driver,
        &[],
        |a| matches!(a, Action::Record(e) if e.kind == Kind::Off),
    );
    let off = kinds();
    assert_eq!(off[off.len() - 2..], [WifiEvent::LEAVE, WifiEvent::OFF]);

    link::set_radio(Radio::On);
    until(&mut driver, &[], |a| *a == Action::RadioOn);
    assert_eq!(INTERLOCK.turn(), Turn::Joining);
    until(&mut driver, &[], |a| *a == Action::TurnEnd);
    assert_eq!(INTERLOCK.turn(), Turn::Free);
    until(&mut driver, &[(GATEWAY_NET, -40, 5)], |a| {
        *a == Action::Outcome(Attempt::Joined)
    });
}

#[test]
fn losses_count_by_cause() {
    let _guard = start();

    lost(Cause::Link);
    let status = net_status::read();
    assert_eq!((status.link_losses, status.check_losses), (1, 0));
    assert_eq!(status.last_loss, 1);

    lost(Cause::Gateway);
    lost(Cause::Lease);
    let status = net_status::read();
    assert_eq!((status.link_losses, status.check_losses), (3, 2));
    assert_eq!(status.last_loss, 2);
    assert_eq!(status.generation, 3, "every loss is a new generation");
}

fn event(kind: u32, result: i32, value: u32, ms: u32) {
    net_status::observe(&WifiEvent {
        kind,
        result,
        value,
        ms,
        ..WifiEvent::default()
    });
}

/// The statistics' link figures: answered probes and their round trip,
/// renewals, the access point's channel and how often it changed.
#[test]
fn the_link_figures_follow_the_shim_events() {
    let _guard = start();

    event(WifiEvent::GATEWAY, -4, 0x0a00_0001, 1000);
    let status = net_status::read();
    assert_eq!((status.gateway_answers, status.gateway_ms), (0, 0));
    event(WifiEvent::GATEWAY, 0, 0x0a00_0001, 4);
    let status = net_status::read();
    assert_eq!((status.gateway_answers, status.gateway_ms), (1, 4));

    event(WifiEvent::RENEW, 0, 0, 0);
    assert_eq!(net_status::read().dhcp_renewals, 1);

    event(WifiEvent::AP, 6, 0x1122_3344, 0);
    let status = net_status::read();
    assert_eq!((status.channel, status.ap_changes), (Some(6), 0));
    // The same access point again, then an unread BSSID: no change.
    event(WifiEvent::AP, 6, 0x1122_3344, 0);
    event(WifiEvent::AP, -1, 0, 0);
    let status = net_status::read();
    assert_eq!((status.channel, status.ap_changes), (None, 0));
    event(WifiEvent::AP, 11, 0x5566_7788, 0);
    let status = net_status::read();
    assert_eq!((status.channel, status.ap_changes), (Some(11), 1));

    // A 5 GHz channel is a channel too.
    event(WifiEvent::AP, 36, 0x5566_7788, 0);
    assert_eq!(net_status::read().channel, Some(36));

    // Channel 6 written over the low byte of a -1 (0xFFFFFF06), and values
    // that are no channel, all read as none.
    for result in [-250, 0, 15, 35, 178, 255, 256 + 6] {
        event(WifiEvent::AP, result, 0x5566_7788, 0);
        assert_eq!(net_status::read().channel, None, "result {result}");
    }
    event(WifiEvent::AP, 14, 0x5566_7788, 0);
    assert_eq!(net_status::read().channel, Some(14));

    lost(Cause::Link);
    assert_eq!(
        net_status::read().channel,
        None,
        "no channel while not joined"
    );
}

/// Battery mode: the radio asked off leaves a live link with its own
/// cause, and the shim sees the wish between its waits; asked on again,
/// the next join is watched as before. A restart wins.
#[test]
fn a_radio_change_takes_turns_with_feeds() {
    let _guard = start();
    INTERLOCK.reset();

    radio_begin();
    assert_eq!(INTERLOCK.turn(), Turn::Joining);
    assert_eq!(INTERLOCK.claim_feed(u32::MAX), Err(Held::Joining));
    radio_end();
    assert_eq!(INTERLOCK.turn(), Turn::Free);

    // A feed holds the outputs: the change waits for its release.
    INTERLOCK.claim_feed(u32::MAX).unwrap();
    let change = thread::spawn(|| {
        radio_begin();
        INTERLOCK.turn()
    });
    thread::sleep(Duration::from_millis(50));
    assert!(
        !change.is_finished(),
        "no radio change while a feed is claimed"
    );
    assert_eq!(
        INTERLOCK.turn(),
        Turn::FeedingJoinWaits,
        "no new feed meanwhile"
    );
    INTERLOCK.release_feed();
    assert_eq!(change.join().unwrap(), Turn::Joining);
    let status = INTERLOCK.status();
    assert_eq!((status.deferred, status.forced), (1, 0));
    radio_end();
    INTERLOCK.reset();
}

/// A `wifi_on` that keeps failing asks for an intentional restart.
#[test]
fn a_radio_that_will_not_come_back_restarts_the_feeder() {
    let _guard = start();
    supervision::reset();
    assert!(!supervision::restart_grace());

    radio_failed();
    assert!(supervision::restart_grace());
    supervision::reset();
}

/// `from`'s echo reply to request `seq`, behind a 20-byte IPv4 header.
fn reply(from: u32, seq: u16) -> Vec<u8> {
    let mut datagram = vec![0; 20];
    datagram[0] = 0x45;
    datagram[12..16].copy_from_slice(&from.to_be_bytes());
    let mut echo = wellfed_core::echo::request(seq).to_vec();
    echo[0] = 0;
    datagram.extend_from_slice(&echo);
    datagram
}

/// The probe sends one request to the gateway and takes its reply, then
/// closes the socket.
#[test]
fn an_echo_the_gateway_answers() {
    let _guard = start();
    fake::with(|h| h.icmp_rx.push_back((4, reply(GATEWAY, 9))));

    assert_eq!(link::echo(GATEWAY, 9), Outcome::Answered);
    fake::with(|h| {
        assert_eq!(
            h.icmp_tx,
            [(wellfed_core::echo::request(9).to_vec(), GATEWAY)]
        );
        assert_eq!(h.icmp_waits, [1000]);
        assert_eq!(h.tcp_closed.len(), 1, "the socket is closed");
        assert_eq!(h.uptime_ms, 4);
    });
}

/// Other ICMP datagrams to this host are skipped, and the probe's second
/// is shared by every receive.
#[test]
fn an_echo_skips_other_icmp_within_one_budget() {
    let _guard = start();
    fake::with(|h| {
        h.icmp_rx.push_back((300, reply(0xc0a8_0102, 9)));
        h.icmp_rx.push_back((300, reply(GATEWAY, 8)));
        h.icmp_rx.push_back((100, vec![0x45; 12]));
        h.icmp_rx.push_back((200, reply(GATEWAY, 9)));
    });
    assert_eq!(link::echo(GATEWAY, 9), Outcome::Answered);
    fake::with(|h| assert_eq!(h.icmp_waits, [1000, 700, 400, 300]));
}

#[test]
fn an_echo_answered_past_its_budget_is_silent() {
    let _guard = start();
    fake::with(|h| {
        h.icmp_rx.push_back((300, reply(0xc0a8_0102, 9)));
        h.icmp_rx.push_back((300, reply(GATEWAY, 8)));
        h.icmp_rx.push_back((300, reply(GATEWAY, 8)));
        h.icmp_rx.push_back((300, reply(GATEWAY, 9)));
    });
    assert_eq!(link::echo(GATEWAY, 9), Outcome::Silent);
    fake::with(|h| {
        assert_eq!(h.icmp_waits, [1000, 700, 400, 100]);
        assert_eq!(h.uptime_ms, 1000, "the budget, and no more");
        assert_eq!(h.tcp_closed.len(), 1);
    });
}

#[test]
fn an_echo_that_never_left_says_why() {
    let _guard = start();
    fake::with(|h| h.icmp_open_fails = true);
    assert_eq!(link::echo(GATEWAY, 1), Outcome::NoSocket);
    fake::with(|h| assert!(h.icmp_tx.is_empty() && h.tcp_closed.is_empty()));

    fake::with(|h| {
        h.icmp_open_fails = false;
        h.icmp_send_fails = true;
    });
    assert_eq!(link::echo(GATEWAY, 1), Outcome::NotSent);
    fake::with(|h| {
        assert!(h.icmp_waits.is_empty(), "nothing waited for");
        assert_eq!(h.tcp_closed.len(), 1);
    });
}
