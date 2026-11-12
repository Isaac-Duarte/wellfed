//! The KM4 side of the park handshake with KM0, for every KM4 caller that
//! binds a flash authority: the arming sequence, and the probes that drive
//! the outputs on their own.
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
    /// KM0 has not answered yet.
    Waiting,
    /// KM0 never answered within [`READY_TIMEOUT_MS`].
    TimedOut,
    /// KM0 answered. `inactive` is whether STOP and infrared OFF read back
    /// verified, which the coordinator was told; `bound` is the coordinator
    /// and its reply, `None` when the clock gave none. Only an
    /// [`FlashReply::Accepted`] reply may be installed.
    Ready {
        inactive: bool,
        bound: Option<(FlashCoord, FlashReply)>,
    },
}

/// One handshake, from its hello until KM0's reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handshake {
    authority: u64,
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
        Ok(Self { authority })
    }

    /// The session this handshake opened, which the coordinator is bound
    /// under.
    pub const fn authority(&self) -> u64 {
        self.authority
    }

    /// One poll, `now_ms` after the handshake started.
    pub fn step<M: Mmio>(&self, now_ms: u32) -> Handshaking {
        let peer = match park::reply::<M>(self.authority) {
            Reply::Ready(peer) => peer,
            _ if now_ms >= READY_TIMEOUT_MS => return Handshaking::TimedOut,
            _ => return Handshaking::Waiting,
        };

        // Bind only with verified STOP and infrared OFF readback.
        let inactive = stopped::observe().verified();
        let bound = FlashCoord::new(&FLASH_LIMITS, self.authority, now_ms).map(|mut coord| {
            let reply = coord.ready(self.authority, peer, inactive, now_ms);
            (coord, reply)
        });
        Handshaking::Ready { inactive, bound }
    }
}
