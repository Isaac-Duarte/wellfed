//! One connection's framed bytes each way.

use crate::{BUFFER, Reason, STALL_MS};
use wellfed_core::deadline_passed;
use wellfed_proto::api::FRAME_MAX;
use wellfed_session::frame;

/// One connection's bytes each way, apart from its phase so a step can
/// hold both.
pub(super) struct Wire {
    pub(super) rx: [u8; BUFFER],
    pub(super) rx_len: usize,
    pub(super) tx: [u8; BUFFER],
    pub(super) tx_len: usize,
    pub(super) tx_sent: usize,
    /// When queued bytes last moved, as a pass saw it: the first pass after
    /// they were queued, or after the driver sent some. `None` until then, and
    /// while nothing is queued.
    tx_moved_ms: Option<u32>,
}

impl Wire {
    pub(super) const EMPTY: Self = Self {
        rx: [0; BUFFER],
        rx_len: 0,
        tx: [0; BUFFER],
        tx_len: 0,
        tx_sent: 0,
        tx_moved_ms: None,
    };

    pub(super) const fn tx_idle(&self) -> bool {
        self.tx_sent >= self.tx_len
    }

    /// The driver sent `len` of the queued bytes.
    pub(super) fn sent(&mut self, len: usize) {
        self.tx_sent = (self.tx_sent + len).min(self.tx_len);
        if len != 0 {
            self.tx_moved_ms = None;
        }
    }

    /// Whether queued bytes have not moved for [`STALL_MS`] by `now_ms`.
    pub(super) fn stalled(&mut self, now_ms: u32) -> bool {
        if self.tx_idle() {
            self.tx_moved_ms = None;
            return false;
        }
        let moved = *self.tx_moved_ms.get_or_insert(now_ms);
        deadline_passed(now_ms, moved.wrapping_add(STALL_MS))
    }

    /// Give up on the queued bytes, so the driver can close the connection.
    pub(super) fn discard(&mut self) {
        self.tx_sent = self.tx_len;
        self.tx_moved_ms = None;
    }

    /// The first complete frame's length, if one is in.
    pub(super) fn frame(&self) -> Result<Option<usize>, Reason> {
        if self.rx_len < frame::PREFIX {
            return Ok(None);
        }
        let prefix = [self.rx[0], self.rx[1]];
        let len = frame::parse_prefix(prefix, FRAME_MAX).ok_or(Reason::Malformed)?;
        Ok((self.rx_len >= frame::PREFIX + len).then_some(len))
    }

    /// The first frame's bytes, `len` after its prefix.
    pub(super) fn message(&self, len: usize) -> &[u8] {
        &self.rx[frame::PREFIX..frame::PREFIX + len]
    }

    /// Drop the first frame, `len` bytes after its prefix.
    pub(super) fn consume(&mut self, len: usize) {
        let used = frame::PREFIX + len;
        self.rx.copy_within(used..self.rx_len, 0);
        self.rx_len -= used;
    }

    /// Queue `bytes` as one frame; the caller checked [`Self::tx_idle`].
    pub(super) fn send(&mut self, bytes: &[u8]) -> Result<(), Reason> {
        let prefix = frame::prefix(bytes.len(), FRAME_MAX).ok_or(Reason::Malformed)?;
        self.tx[..frame::PREFIX].copy_from_slice(&prefix);
        self.tx[frame::PREFIX..frame::PREFIX + bytes.len()].copy_from_slice(bytes);
        self.tx_len = frame::PREFIX + bytes.len();
        self.tx_sent = 0;
        self.tx_moved_ms = None;
        Ok(())
    }
}
