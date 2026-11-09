//! The device API in the loop: [`Serve`] owns the sockets and steps
//! [`wellfed_device_api`]'s server once per pass, the fifth stage of
//! [`super::Feeder::pass`]. Every socket call is non-blocking, so the loop
//! never waits on a client.
//!
//! # One pass, in order
//!
//! [`Serve::step`] takes the pass's [`Inputs`] (what the owner, the supply,
//! the schedule and the call stages decided) and runs:
//!
//! 1. **Before the server** (`before_server`, out of line):
//!    1. the boot's facts ([`Serve::boot`], read once a boot; the pass's
//!       local stage asks first, so its supply hold is armed before the
//!       first readiness);
//!    2. the records the event queue dropped since the last pass, as a
//!       `Lost` event, and the enrollments and revocations the settings
//!       writer saved, handed to the alert rules
//!       ([`super::events::written`]);
//!    3. the statistics' share of the pass: the link, the food beam and the
//!       battery's history (`telemetry`), so a `Stats` answer this pass
//!       sees them;
//!    4. the client table, copied from the writer when its generation
//!       moved, which closes a revoked key's sessions before they run
//!       another request;
//!    5. the address: the mDNS responder is made once the identity is
//!       loaded and told the DHCP address; a new address is logged;
//!    6. once there is an address, the listener and the mDNS socket opened
//!       or retried (`net`); the mDNS socket waits for the responder;
//!    7. mDNS's announcements and a bounded number of queries (`mdns`),
//!       after the open so a socket opened this pass announces at once;
//!    8. the time settings and the NTP exchange ([`super::time`]), so the
//!       clock state and the NTP outcome the server reports are this pass's;
//!    9. a waiting connection accepted into a free slot, then each slot's
//!       bytes read into the server's buffer (`net`), so a request that
//!       arrived is handled in this pass's step.
//! 2. **The server's step**, with a [`DeviceView`] over the rest of the
//!    feeder (`device_view`, `feeder_view`): handshakes, then each
//!    session's requests and events. It borrows the statistics, the task
//!    cache and the answer page, so the sessions it opened are counted
//!    only once it lets go.
//! 3. **After the server** (`after_server`, out of line):
//!    1. the bytes the step sealed sent, and the slots it ended closed
//!       (`net`), so an answer leaves in the pass that made it;
//!    2. the live samples queued for subscribers and, every 30 s, the
//!       counts handed to the writer (`telemetry`); a session seals a
//!       queued sample in a later step, so it goes out a pass later;
//!    3. once a minute, the heap and mDNS's counts as a `Stats` record.
//!
//! The delay `step` returns is 1 ms while the server has work, else the
//! owner's poll.
//!
//! Both halves around the server's step are out of line because the
//! server's handshakes and requests are the loop's deepest stack paths:
//! whatever a half keeps in its frame would otherwise sit under every one
//! of them in the stack audit.
//!
//! Every record the pass makes is in `records`; none carries a key, a
//! secret or a credential.

mod device_view;
mod feeder_view;
mod mdns;
mod net;
mod records;
mod telemetry;

use wellfed_core::mdns::{self as mdns_core, Responder};
use wellfed_proto::api::{BANK1_PAGE_MAX, EventBody, PORT, Supply};
use wellfed_proto::trace::PAGE_MAX;
use wellfed_session::noise::Keypair;

use super::Feeds;
use super::events;
use super::lock::LockBox;
use super::owner::POLL_MS;
use super::platform::{Boot, Platform};
use super::time::Time;
use crate::stats::Keeper;
use wellfed_device_api::mailbox::NtpResult;
use wellfed_device_api::pairing::{self, Pairing};
use wellfed_device_api::{self as server, Pass, SESSIONS, Server};

pub use device_view::DeviceView;
pub(super) use device_view::desiccant_now;
pub use records::{Record, describe};
pub use telemetry::TaskCache;

/// The answer page buffer: a trace's page or a bank-1 page, whichever is
/// larger, or stock's plan entries.
pub(super) const PAGE_BYTES: usize = if PAGE_MAX > BANK1_PAGE_MAX {
    PAGE_MAX
} else {
    BANK1_PAGE_MAX
};

// Stock's plan entries borrow the same buffer.

/// A listener or mDNS socket that failed is opened again after this.
const RETRY_MS: u32 = 2_000;
const HEAP_MS: u32 = 60_000;

/// What the loop hands the device API each pass.
pub struct Inputs<'a, F> {
    pub now_ms: u32,
    /// BLE setup mode runs: no feed starts.
    pub setup: bool,
    pub pairing: &'a mut Pairing,
    pub identity: Option<&'a Keypair>,
    /// The panel lock, once the owner has read it.
    pub locked: Option<bool>,
    pub recovery: Option<pairing::Event>,
    /// Pairing's decision on an update key's offer, this step.
    pub signer: Option<pairing::Event>,
    /// The supply this pass judged.
    pub supply: Supply,
    /// A brownout holds feeding until the adapter is seen.
    pub battery_hold: bool,
    /// A plan's claimed feed must be the next one.
    pub meal: bool,
    /// A factory reset is queued or marked: no install starts.
    pub reset: bool,
    /// A plan's claim is in flight: its meal is the next feed.
    pub claim: bool,
    /// When the next meal call's lead begins (Unix ms), if one is to come.
    pub call_lead: Option<u64>,
    pub feeds: &'a mut F,
}

pub struct Serve<S> {
    server: Server,
    listener: Option<S>,
    sockets: [Option<S>; SESSIONS],
    mdns_socket: Option<S>,
    responder: Option<Responder>,
    /// The address last logged.
    advertised: Option<u32>,
    /// The writer's client table generation this copy holds.
    clients_seen: u32,
    boot: Option<Boot>,
    retry_at: u32,
    next_power: u32,
    next_rssi: u32,
    next_heap: u32,
    /// mDNS datagrams received and answers sent, for the stats record.
    queries: u32,
    answers: u32,
    failed: u32,
    stats: Keeper,
    /// The task stacks a memory answer borrows its names from.
    tasks: TaskCache,
    /// The trace or bank-1 page an answer borrows its bytes from.
    page: [u8; PAGE_BYTES],
    packet: [u8; mdns_core::PACKET],
    reply: [u8; mdns_core::PACKET],
    time: Time<S>,
}

impl<S: Copy> Serve<S> {
    /// Built at compile time: the server's table is larger than the loop's
    /// stack.
    pub const fn new() -> Self {
        Self {
            server: Server::new(),
            listener: None,
            sockets: [None; SESSIONS],
            mdns_socket: None,
            responder: None,
            advertised: None,
            clients_seen: 0,
            boot: None,
            retry_at: 0,
            next_power: 0,
            next_rssi: 0,
            next_heap: 0,
            queries: 0,
            answers: 0,
            failed: 0,
            stats: Keeper::new(),
            tasks: TaskCache::new(),
            page: [0; PAGE_BYTES],
            packet: [0; mdns_core::PACKET],
            reply: [0; mdns_core::PACKET],
            time: Time::new(),
        }
    }

    /// The server, for a host test to look into.
    pub fn server(&mut self) -> &mut Server {
        &mut self.server
    }

    /// What the last NTP exchange came to.
    pub const fn ntp_last(&self) -> Option<NtpResult> {
        self.time.last()
    }

    /// The statistics, which the rest of the loop feeds.
    pub fn stats(&mut self) -> &mut Keeper {
        &mut self.stats
    }

    /// The statistics and the buffers a memory or trace answer borrows
    /// from.
    pub(super) fn stats_parts(&mut self) -> (&mut Keeper, &mut TaskCache, &mut [u8; PAGE_BYTES]) {
        (&mut self.stats, &mut self.tasks, &mut self.page)
    }

    /// This boot's `Boot` has been recorded.
    pub const fn booted(&self) -> bool {
        self.boot.is_some()
    }

    /// This boot's facts, read once.
    pub fn boot<P: Platform<Socket = S>>(&mut self, platform: &mut P, now_ms: u32) -> Boot {
        *self.boot.get_or_insert_with(|| {
            let boot = platform.boot();
            let body = EventBody::Boot {
                bank: boot.bank,
                reset_flags: boot.reset_flags,
            };
            events::record(platform, now_ms, body);
            self.stats.boot(boot.reset_flags);
            boot
        })
    }

    /// One pass; the delay until the next.
    pub fn step<P: Platform<Socket = S>, F: Feeds>(
        &mut self,
        platform: &mut P,
        lock: &mut LockBox,
        inputs: Inputs<'_, F>,
        mut record: impl FnMut(Record),
    ) -> u32 {
        let Inputs {
            now_ms,
            setup,
            pairing,
            identity,
            locked,
            recovery,
            signer,
            supply,
            battery_hold,
            meal,
            reset,
            claim,
            call_lead,
            feeds,
        } = inputs;
        let boot = self.before_server(platform, &*feeds, now_ms, identity, &mut record);

        // While the outputs are claimed, not only while they move, only IK
        // session handshakes go on (until one is refused): an invite's join or a
        // recovery enrolls a key, a store write that would wait out the
        // feed, so the invite is redeemed (and the recovery code taken) only
        // once it can be written. Held for at most the claim's bound, as
        // joins are. A feed still waiting to claim them counts: an enrollment
        // now would queue the very write it would then wait for.
        let feeding =
            feeds.feeding() || feeds.claiming().is_some() || platform.claim_in_bound(now_ms);
        // The sessions this pass opened, counted once the device lets go of
        // the statistics.
        let mut opened = [None; SESSIONS];
        let mut device = DeviceView {
            platform: &mut *platform,
            lock,
            feeds,
            boot,
            locked,
            setup,
            now_ms,
            ntp_last: self.time.last(),
            stats: &mut self.stats,
            tasks: &mut self.tasks,
            page: &mut self.page,
            battery_hold,
            supply,
            meal,
            reset,
            claim,
            call_lead,
        };
        let busy = self.server.step(
            Pass {
                now_ms,
                device: &mut device,
                pairing,
                identity,
                feeding,
                recovery,
                signer,
            },
            |r| {
                if let server::Record::Opened { handshake_ms, .. } = r
                    && let Some(free) = opened.iter_mut().find(|o| o.is_none())
                {
                    *free = Some(handshake_ms);
                }
                record(Record::Server(r));
            },
        );
        for handshake_ms in opened.into_iter().flatten() {
            self.stats.opened(handshake_ms);
        }
        self.after_server(platform, now_ms, supply, &mut record);

        if busy { 1 } else { POLL_MS }
    }

    /// The pass's work behind the server's step: the sockets' answers, the
    /// live samples and the minute's record. Out of line for the same reason
    /// as [`Self::before_server`].
    #[inline(never)]
    fn after_server<P: Platform<Socket = S>>(
        &mut self,
        platform: &mut P,
        now_ms: u32,
        supply: Supply,
        record: &mut impl FnMut(Record),
    ) {
        self.send(platform, record);
        self.samples(platform, now_ms, supply);

        if wellfed_core::deadline_passed(now_ms, self.next_heap) {
            self.next_heap = now_ms.wrapping_add(HEAP_MS);
            let (free, min) = platform.heap();
            record(Record::Stats {
                free,
                min,
                queries: self.queries,
                answers: self.answers,
                failed: self.failed,
            });
        }
    }

    /// The pass's work ahead of the server's step: the boot's facts, lost
    /// records, the statistics, the client table, mDNS, the clock and what
    /// the sockets hold. Out of line because its frame would otherwise stay
    /// live under every request and handshake, the loop's deepest paths.
    #[inline(never)]
    fn before_server<P: Platform<Socket = S>, F: Feeds>(
        &mut self,
        platform: &mut P,
        feeds: &F,
        now_ms: u32,
        identity: Option<&Keypair>,
        record: &mut impl FnMut(Record),
    ) -> Boot {
        let boot = self.boot(platform, now_ms);
        let lost = platform.take_lost();
        if lost > 0 {
            let count = u16::try_from(lost).unwrap_or(u16::MAX);
            events::record(platform, now_ms, EventBody::Lost { count });
        }
        let unheard = events::written(platform, now_ms);
        if unheard > 0 {
            record(Record::Unheard { count: unheard });
        }
        self.keep_stats(platform, feeds, now_ms);

        let (table, seen) = self.server.clients_table();
        if let Some(generation) = platform.clients_since(seen.max(self.clients_seen), table) {
            self.clients_seen = generation;
            self.server
                .clients_changed(generation, |r| record(Record::Server(r)));
        }

        let ip = platform.ipv4();
        if self.responder.is_none()
            && let Some(identity) = identity
        {
            self.responder = Some(Responder::new(identity.public(), PORT));
        }
        if let Some(responder) = self.responder.as_mut() {
            responder.set_ip(ip, now_ms);
        }
        if ip != self.advertised {
            self.advertised = ip;
            if let Some(ip) = ip {
                record(Record::Address { ip });
            }
        }
        if ip.is_some() {
            self.open_sockets(platform, now_ms, record);
        }
        self.mdns(platform, now_ms);
        let clock_ms = platform.clock_ms(now_ms);
        self.time
            .step(platform, clock_ms, ip, |r| record(Record::Time(r)));
        self.accept(platform, now_ms, record);
        self.receive(platform, record);
        boot
    }
}

impl<S: Copy> Default for Serve<S> {
    fn default() -> Self {
        Self::new()
    }
}
