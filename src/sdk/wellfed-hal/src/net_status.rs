//! Wi-Fi state shared between its writers and the tasks that read it, as
//! atomics: no lock, and no reference from a reader into a writer. The
//! probe, folding in the shim's events, writes the counters and the
//! generation; the Wi-Fi task's watch ([`crate::link`]) alone writes
//! whether the link is live, so a loss clears it before the event saying so
//! is even drained.

use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicU32, Ordering};

use crate::sdk::WifiEvent;

/// `LwIP_DHCP` state for an assigned address, in a DHCP event's `result`.
const DHCP_ASSIGNED: i32 = 2;

static RSSI: AtomicI32 = AtomicI32::new(0);
static ATTEMPT: AtomicU32 = AtomicU32::new(0);
/// Every loss of a joined link, whatever noticed it. An access point that
/// goes silent usually shows first as an unanswered gateway, 11-16 s in and
/// before the driver gives up on its beacons, so counting only the
/// association would read 0 after a real outage.
static LINK_LOSSES: AtomicU32 = AtomicU32::new(0);
/// Those of [`LINK_LOSSES`] the watch noticed (the lease or the gateway)
/// rather than the association.
static CHECK_LOSSES: AtomicU32 = AtomicU32::new(0);
/// [`LossCause::code`] of the latest loss.
static LAST_LOSS: AtomicU8 = AtomicU8::new(LossCause::None.code());
/// [`wellfed_core::liveness::Liveness::live`] of the current join.
static LIVE: AtomicBool = AtomicBool::new(false);
/// Bumped at every join and loss: a connection opened in an older generation
/// rides on a link that has since gone away.
static GENERATION: AtomicU32 = AtomicU32::new(0);
/// For the statistics: leases renewed without a rejoin, gateway probes
/// answered and the latest one's round trip, the joined access point's
/// channel (0 while not joined) and BSSID tail, and how often a join
/// landed on another access point than the one before.
static RENEWALS: AtomicU32 = AtomicU32::new(0);
static GATEWAY_ANSWERS: AtomicU32 = AtomicU32::new(0);
static GATEWAY_MS: AtomicU32 = AtomicU32::new(0);
static CHANNEL: AtomicU32 = AtomicU32::new(0);
static ACCESS_POINT: AtomicU32 = AtomicU32::new(0);
static AP_CHANGES: AtomicU32 = AtomicU32::new(0);

/// What noticed the latest loss of a joined link.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum LossCause {
    /// No joined link has been lost this boot.
    #[default]
    None = 0,
    /// The association: the driver gave up on the access point.
    Link = 1,
    /// The watch's check: the lease or the gateway.
    Check = 2,
}

impl LossCause {
    /// The variant's number, as the shared atomic keeps it.
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// The variant `byte` numbers, if any.
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::None),
            1 => Some(Self::Link),
            2 => Some(Self::Check),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub rssi: i32,
    pub attempt: u32,
    /// Every loss of a joined link.
    pub link_losses: u32,
    /// Those the watch noticed, the lease or the gateway, rather than the
    /// association.
    pub check_losses: u32,
    pub last_loss: LossCause,
    /// The link is known good: associated, leased and, once its gateway
    /// has answered this join, answering still. No internet is involved.
    pub live: bool,
    pub generation: u32,
    pub dhcp_renewals: u32,
    pub gateway_answers: u32,
    pub gateway_ms: u32,
    /// The joined access point's channel, `None` while not joined.
    pub channel: Option<u8>,
    pub ap_changes: u32,
}

/// Fold one shim event in (Wi-Fi probe task only).
pub fn observe(event: &WifiEvent) {
    match event.kind {
        WifiEvent::SCAN if event.result == 0 && event.value == 1 => {
            RSSI.store(event.rssi, Ordering::Relaxed);
            ATTEMPT.store(event.attempt, Ordering::Relaxed);
        }
        WifiEvent::DHCP if event.result == DHCP_ASSIGNED => {
            GENERATION.fetch_add(1, Ordering::Release);
        }
        WifiEvent::LOST => {
            LINK_LOSSES.fetch_add(1, Ordering::Relaxed);

            // Any cause but the association's is the watch's check.
            let cause = if event.result != 0 {
                CHECK_LOSSES.fetch_add(1, Ordering::Relaxed);
                LossCause::Check
            } else {
                LossCause::Link
            };
            LAST_LOSS.store(cause.code(), Ordering::Relaxed);
            CHANNEL.store(0, Ordering::Relaxed);
            GENERATION.fetch_add(1, Ordering::Release);
        }
        WifiEvent::GATEWAY if event.result == 0 => {
            GATEWAY_MS.store(event.ms, Ordering::Relaxed);
            GATEWAY_ANSWERS.fetch_add(1, Ordering::Relaxed);
        }
        WifiEvent::AP => {
            CHANNEL.store(channel(event.result), Ordering::Relaxed);
            // An unread BSSID (0) says nothing about which one it was.
            if event.value != 0 {
                let before = ACCESS_POINT.swap(event.value, Ordering::Relaxed);
                if before != 0 && before != event.value {
                    AP_CHANGES.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        WifiEvent::RENEW => {
            RENEWALS.fetch_add(1, Ordering::Relaxed);
        }
        _ => {}
    }
}

/// An access point event's channel, 0 for none. The Wi-Fi task checks it
/// too, but this is the decode boundary: the SDK's `wifi_get_channel` writes
/// one byte of its `int`, so a reader that misses that sends -250 for
/// channel 6 over a -1. Anything outside the 2.4 GHz (1-14) and 5 GHz (36-177) channels of a
/// dual-band AmebaD reads as none rather than as a channel.
fn channel(result: i32) -> u32 {
    u32::try_from(result)
        .ok()
        .filter(|channel| matches!(channel, 1..=14 | 36..=177))
        .unwrap_or(0)
}

/// This boot has no network to join: the image has no build network, and
/// the credential store read as holding none (not one that could not be
/// read, nor one a factory reset or handover has yet to clear).
static UNPROVISIONED: AtomicBool = AtomicBool::new(false);

/// The product's join half, before each attempt: whether this boot has
/// no network to join, as [`UNPROVISIONED`] says.
pub fn set_unprovisioned(unprovisioned: bool) {
    UNPROVISIONED.store(unprovisioned, Ordering::Release);
}

/// Whether the last join attempt found no network to join, neither the
/// build's nor a stored one. A trial build can never confirm then: only the
/// settings writer saves credentials, and it starts once the trial has.
pub fn unprovisioned() -> bool {
    UNPROVISIONED.load(Ordering::Acquire)
}

/// The current join's liveness (the Wi-Fi task's watch only).
pub(crate) fn set_live(live: bool) {
    LIVE.store(live, Ordering::Release);
}

pub fn read() -> Status {
    Status {
        generation: GENERATION.load(Ordering::Acquire),
        live: LIVE.load(Ordering::Acquire),
        rssi: RSSI.load(Ordering::Relaxed),
        attempt: ATTEMPT.load(Ordering::Relaxed),
        link_losses: LINK_LOSSES.load(Ordering::Relaxed),
        check_losses: CHECK_LOSSES.load(Ordering::Relaxed),
        // Only `observe` stores it, and only a variant's code.
        last_loss: LossCause::from_byte(LAST_LOSS.load(Ordering::Relaxed)).unwrap_or_default(),
        dhcp_renewals: RENEWALS.load(Ordering::Relaxed),
        gateway_answers: GATEWAY_ANSWERS.load(Ordering::Relaxed),
        gateway_ms: GATEWAY_MS.load(Ordering::Relaxed),
        channel: u8::try_from(CHANNEL.load(Ordering::Relaxed))
            .ok()
            .filter(|&channel| channel != 0),
        ap_changes: AP_CHANGES.load(Ordering::Relaxed),
    }
}

/// Host tests only: back to the boot state.
#[cfg(not(target_os = "none"))]
pub fn reset() {
    for counter in [
        &ATTEMPT,
        &LINK_LOSSES,
        &CHECK_LOSSES,
        &GENERATION,
        &RENEWALS,
        &GATEWAY_ANSWERS,
        &GATEWAY_MS,
        &CHANNEL,
        &ACCESS_POINT,
        &AP_CHANGES,
    ] {
        counter.store(0, Ordering::Relaxed);
    }
    LAST_LOSS.store(LossCause::None.code(), Ordering::Relaxed);
    RSSI.store(0, Ordering::Relaxed);
    LIVE.store(false, Ordering::Relaxed);
    UNPROVISIONED.store(false, Ordering::Relaxed);
}
