//! The KM4 side of the park handshake with KM0, for every KM4 caller that
//! binds a flash authority: the arming sequence, the probes that drive the
//! outputs on their own, and the settings stress.
//!
//! A handshake bumps the boot count, opens a session and says hello; then
//! KM0's reply is awaited for at most [`READY_TIMEOUT_MS`]; and the
//! authority is bound only with motor STOP and infrared OFF verified by
//! readback, so a lease never starts with outputs that might move. None of
//! it starts while the runtime holds writes for a reset it decided. One
//! copy, so the callers cannot drift apart again.

use wellfed_core::flash_coord::{FlashCoord, FlashReply};

use crate::flash::park::{self, Mmio, Reply};
use crate::flash_lock::provider::{BOOT_COUNT_REGISTER, READY_TIMEOUT_MS};
use crate::sdk;
use crate::stopped;
use crate::supervision::config::FLASH_LIMITS;

/// Why a handshake did not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unstarted {
    /// The runtime holds writes for a reset it decided: nothing is bound
    /// into it. Try again on a later poll.
    Held,
    /// KM0 did not take the hello.
    Refused,
}

/// Where a handshake stands after a poll.
#[derive(Clone, Copy, Debug)]
pub enum Handshaking {
    /// KM0 has not answered yet. `foreign` when its reply register holds
    /// a READY for another session, or a malformed one, which a bench report
    /// may name; it is waited past like no answer.
    Waiting { foreign: bool },
    /// KM0 never answered within [`READY_TIMEOUT_MS`].
    TimedOut,
    /// KM0 answered, and its session was bound into a coordinator.
    Ready(Answer),
}

/// KM0's answer, bound under the handshake's session. The coordinator was
/// told whether STOP and infrared OFF read back verified, and refuses the
/// bind when they did not; [`Answer::accepted`] hands it out only when it
/// accepted, so a caller cannot attach a refused one.
#[derive(Clone, Copy, Debug)]
pub struct Answer {
    peer: u64,
    inactive: bool,
    /// `None` when the clock gave no coordinator.
    bound: Option<(FlashCoord, FlashReply)>,
}

impl Answer {
    /// KM0's session, which it echoed with this handshake's.
    pub const fn peer(&self) -> u64 {
        self.peer
    }

    /// Whether STOP and infrared OFF read back verified.
    pub const fn inactive(&self) -> bool {
        self.inactive
    }

    /// The coordinator's reply to the bind, for a report; `None` when the
    /// clock gave no coordinator.
    pub fn reply(&self) -> Option<FlashReply> {
        self.bound.map(|(_, reply)| reply)
    }

    /// The coordinator to attach or arm: only one that accepted the bind.
    pub fn accepted(self) -> Option<FlashCoord> {
        self.bound
            .filter(|&(_, reply)| reply == FlashReply::Accepted)
            .map(|(coord, _)| coord)
    }

    /// The coordinator whatever it replied, for a caller that keeps even a
    /// refused one in its provider: arming, whose provider the feeding owner
    /// reads its outputs' leases from, and the settings stress, which reports
    /// its phase and fault. A refused coordinator is never idle, so it grants
    /// no lease, and its phase and fault say why it refused. Arm nothing
    /// with it unless its reply is [`FlashReply::Accepted`].
    pub fn with_any_reply(self) -> Option<(FlashCoord, FlashReply)> {
        self.bound
    }
}

/// One handshake, from its hello until KM0's reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handshake {
    authority: u64,
    count: u32,
}

impl Handshake {
    /// Bump the boot count, open a session at raw systimer `ticks` and say
    /// hello.
    pub fn start<M: Mmio>(ticks: u32) -> Result<Self, Unstarted> {
        if crate::supervision::writes_held() {
            return Err(Unstarted::Held);
        }
        let count = sdk::backup_read(BOOT_COUNT_REGISTER).wrapping_add(1);
        sdk::backup_write(BOOT_COUNT_REGISTER, count);

        let authority = park::session(count, ticks, false);
        if !park::hello::<M>(authority) {
            return Err(Unstarted::Refused);
        }
        Ok(Self { authority, count })
    }

    /// The session this handshake opened, which the coordinator is bound
    /// under.
    pub const fn authority(&self) -> u64 {
        self.authority
    }

    /// The boot count this handshake raised, which its session carries.
    pub const fn count(&self) -> u32 {
        self.count
    }

    /// One poll, `now_ms` after the handshake started.
    pub fn step<M: Mmio>(&self, now_ms: u32) -> Handshaking {
        let peer = match park::reply::<M>(self.authority) {
            Reply::Ready(peer) => peer,
            _ if now_ms >= READY_TIMEOUT_MS => return Handshaking::TimedOut,
            reply => {
                return Handshaking::Waiting {
                    foreign: reply == Reply::Foreign,
                };
            }
        };

        // Bind only with verified STOP and infrared OFF readback.
        let inactive = stopped::observe().verified();
        let bound = FlashCoord::new(&FLASH_LIMITS, self.authority, now_ms).map(|mut coord| {
            let reply = coord.ready(self.authority, peer, inactive, now_ms);
            (coord, reply)
        });
        Handshaking::Ready(Answer {
            peer,
            inactive,
            bound,
        })
    }
}
