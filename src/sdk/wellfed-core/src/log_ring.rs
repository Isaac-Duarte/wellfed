//! The lines on their way to the UART.
//!
//! At 115200 baud a record line takes up to 16 ms to send. Waiting for it
//! with the scheduler held stopped every task, so a line is queued here
//! instead and a low-priority task sends the queue while the UART has room.
//!
//! - **Whole lines.** A line goes in whole or not at all, so bytes come out
//!   in the order the lines were logged and no line is split by another's.
//! - **A full ring drops the new line and counts it.** What is queued is
//!   older, and may be half sent. The next line that fits, or an empty ring,
//!   brings a `PLLOG1 dropped=N` line first, at the place the hole is.
//! - **Callers serialize.** Every call runs inside one critical section (the
//!   SDK masks interrupts around it), so the ring has no waiting of its own.
//!   Its fields are atomics only so that it can be a safe `static`; a read
//!   outside that section (`room`, `mid_line`) is a hint a sender checks again.

use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering::Relaxed};

/// The start of the line that reports dropped lines.
const DROPPED: &[u8] = b"PLLOG1 dropped=";
/// A `u32` in decimal.
const DIGITS_MAX: usize = 10;
const ENDING: &[u8] = b"\r\n";
/// The longest dropped-lines report: a whole `u32` count.
const REPORT_MAX: usize = DROPPED.len() + DIGITS_MAX + ENDING.len();

/// Whether a line can go in now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Room {
    Fits,
    /// It fits once the ring has drained.
    Full,
    /// It is too long to fit even an empty ring.
    Never,
}

/// Lines that may be lost to a full ring rather than wait for room: only
/// the device API's, the one record a client's request rate can drive.
/// Every other record is the firmware's own evidence (feeds, supervision,
/// updates, measurements) and comes at a rate the firmware sets.
pub fn may_drop(line: &[u8]) -> bool {
    line.starts_with(b"PLAPI1 ")
}

/// A ring of `N` bytes (a power of two).
pub struct LogRing<const N: usize> {
    bytes: [AtomicU8; N],
    /// The stream position (wrapping) just past the newest line.
    head: AtomicU32,
    /// The stream position of the next byte to send.
    tail: AtomicU32,
    /// Lines dropped since the last report.
    dropped: AtomicU32,
    /// The last byte sent did not end a line.
    mid_line: AtomicBool,
}

impl<const N: usize> LogRing<N> {
    pub const fn new() -> Self {
        // The size divides the wrapping `u32` positions, so a byte's index
        // is its position modulo `N` across the wrap.
        const { assert!(N.is_power_of_two() && N > REPORT_MAX && N <= 1 << 30) };
        Self {
            bytes: [const { AtomicU8::new(0) }; N],
            head: AtomicU32::new(0),
            tail: AtomicU32::new(0),
            dropped: AtomicU32::new(0),
            mid_line: AtomicBool::new(false),
        }
    }

    /// Whether a line of `len` bytes can go in, with the report of dropped
    /// lines that must go ahead of it.
    pub fn room(&self, len: usize) -> Room {
        if len > N - REPORT_MAX {
            return Room::Never;
        }
        let report = if self.dropped.load(Relaxed) == 0 {
            0
        } else {
            REPORT_MAX
        };
        if len + report <= self.free() {
            Room::Fits
        } else {
            Room::Full
        }
    }

    /// Queue one whole line and answer [`Room::Fits`], or leave the ring as it
    /// was and say why not, for a caller to wait or to [`Self::count_dropped`].
    pub fn try_push(&self, line: &[u8]) -> Room {
        let room = self.room(line.len());
        if room == Room::Fits {
            self.report_dropped();
            self.write(line);
        }
        room
    }

    /// Count a line that did not go in, for the next report.
    pub fn count_dropped(&self) {
        let dropped = self.dropped.load(Relaxed);
        self.dropped.store(dropped.saturating_add(1), Relaxed);
    }

    /// Queue one whole line, or count it dropped if it does not fit.
    pub fn push(&self, line: &[u8]) {
        if self.try_push(line) != Room::Fits {
            self.count_dropped();
        }
    }

    /// Send up to `max` bytes through `send`, stopping after a line's end,
    /// and return how many. Stopping there lets `send` add to a line ending
    /// and stay within `max` plus what it adds to one. Inlined so that a send
    /// on the panic path, which every task's stack bound counts, stays shallow.
    #[inline(always)]
    pub fn pop(&self, max: usize, mut send: impl FnMut(u8)) -> usize {
        // Nothing else is coming to carry the report: it goes out alone.
        if self.head.load(Relaxed) == self.tail.load(Relaxed) {
            self.report_dropped();
        }

        let head = self.head.load(Relaxed);
        let mut tail = self.tail.load(Relaxed);
        let mut sent = 0;
        while sent < max && tail != head {
            let byte = self.bytes[index::<N>(tail)].load(Relaxed);
            send(byte);
            tail = tail.wrapping_add(1);
            sent += 1;
            self.mid_line.store(byte != b'\n', Relaxed);
            if byte == b'\n' {
                break;
            }
        }
        self.tail.store(tail, Relaxed);
        sent
    }

    /// Something waits to be sent, a report of dropped lines included.
    pub fn pending(&self) -> bool {
        self.used() != 0 || self.dropped.load(Relaxed) != 0
    }

    /// A line has started out and the rest of it is still here.
    pub fn mid_line(&self) -> bool {
        self.mid_line.load(Relaxed) && self.used() != 0
    }

    fn report_dropped(&self) {
        let dropped = self.dropped.load(Relaxed);
        if dropped == 0 || self.free() < REPORT_MAX {
            return;
        }
        self.dropped.store(0, Relaxed);

        self.write(DROPPED);
        // Digit by digit into the ring: a panic reaches this path through the
        // flush, so it holds no buffer and no index that could panic again.
        let mut place = 1_000_000_000;
        let mut started = false;
        while place != 0 {
            let digit = dropped / place % 10;
            started |= digit != 0;
            if started {
                // A digit, which fits a byte.
                self.write(&[b'0' + u8::try_from(digit).unwrap_or(0)]);
            }
            place /= 10;
        }
        self.write(ENDING);
    }

    fn write(&self, bytes: &[u8]) {
        let mut head = self.head.load(Relaxed);
        for &byte in bytes {
            self.bytes[index::<N>(head)].store(byte, Relaxed);
            head = head.wrapping_add(1);
        }
        self.head.store(head, Relaxed);
    }

    fn used(&self) -> usize {
        let used = self
            .head
            .load(Relaxed)
            .wrapping_sub(self.tail.load(Relaxed));
        usize::try_from(used).unwrap_or(N)
    }

    fn free(&self) -> usize {
        N.saturating_sub(self.used())
    }
}

impl<const N: usize> Default for LogRing<N> {
    fn default() -> Self {
        Self::new()
    }
}

fn index<const N: usize>(at: u32) -> usize {
    // N is a power of two no larger than 2^30, so the mask fits a u32.
    usize::try_from(at).unwrap_or(0) & (N - 1)
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::string::String;
    use std::vec::Vec;
    use std::{format, vec};

    use super::*;

    const RING: usize = 128;

    fn drain(ring: &LogRing<RING>) -> String {
        let mut out = Vec::new();
        while ring.pending() {
            ring.pop(8, |byte| out.push(byte));
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn lines_come_out_whole_and_in_order() {
        let ring = LogRing::<RING>::new();
        assert!(!ring.pending());

        ring.push(b"PLONE1 a\r\n");
        ring.push(b"PLTWO1 b\r\n");
        ring.push(b"\r\nWellfed\r\n");

        assert_eq!(drain(&ring), "PLONE1 a\r\nPLTWO1 b\r\n\r\nWellfed\r\n");
        assert!(!ring.pending());
    }

    #[test]
    fn a_pop_takes_at_most_its_limit_and_stops_at_a_line_end() {
        let ring = LogRing::<RING>::new();
        ring.push(b"123456789\n");
        ring.push(b"ab\n");
        let mut out = vec![];

        assert_eq!(ring.pop(4, |b| out.push(b)), 4);
        assert_eq!(out, b"1234");
        assert_eq!(ring.pop(100, |b| out.push(b)), 6);
        assert_eq!(out, b"123456789\n");
        assert_eq!(ring.pop(100, |b| out.push(b)), 3);
        assert_eq!(out, b"123456789\nab\n");
        assert_eq!(ring.pop(100, |b| out.push(b)), 0);
    }

    #[test]
    fn mid_line_lasts_from_the_first_byte_to_the_last() {
        let ring = LogRing::<RING>::new();
        ring.push(b"abc\n");
        assert!(!ring.mid_line(), "queued, not started");

        ring.pop(2, |_| {});
        assert!(ring.mid_line());
        ring.pop(2, |_| {});
        assert!(!ring.mid_line());

        // A print with no line end leaves nothing to wait for.
        ring.push(b"abc");
        ring.pop(3, |_| {});
        assert!(!ring.mid_line());
    }

    #[test]
    fn bytes_and_positions_carry_across_the_wrap() {
        let ring = LogRing::<RING>::new();
        ring.head.store(u32::MAX - 20, Relaxed);
        ring.tail.store(u32::MAX - 20, Relaxed);
        let mut expected = String::new();
        for i in 0..50 {
            let line = format!("PLTST1 line={i:03}\r\n");
            ring.push(line.as_bytes());
            expected.push_str(&line);
            if i % 3 == 0 {
                assert!(expected.starts_with(&drain(&ring)));
                expected.clear();
            }
        }
        assert_eq!(drain(&ring), expected);
    }

    #[test]
    fn a_full_ring_drops_whole_new_lines_and_reports_them_in_their_place() {
        let ring = LogRing::<RING>::new();
        let line = |i: usize| format!("PLTST1 line={i:03} {}\r\n", "x".repeat(10));
        let mut kept = 0;
        while ring.room(line(kept).len()) == Room::Fits {
            ring.push(line(kept).as_bytes());
            kept += 1;
        }

        for i in 0..3 {
            ring.push(line(100 + i).as_bytes());
        }
        assert_eq!(ring.room(line(0).len()), Room::Full);

        let mut before = String::new();
        for i in 0..kept {
            before.push_str(&line(i));
        }
        let mut out = vec![];
        for _ in 0..kept {
            while ring.pop(100, |b| out.push(b)) == 0 {}
        }
        assert_eq!(String::from_utf8(out).unwrap(), before);

        ring.push(line(7).as_bytes());
        let text = drain(&ring);
        assert_eq!(text, format!("PLLOG1 dropped=3\r\n{}", line(7)));
    }

    #[test]
    fn the_report_goes_out_alone_once_the_ring_is_empty() {
        let ring = LogRing::<RING>::new();
        ring.push(&[b'x'; RING]);
        assert!(ring.pending(), "the count has yet to go out");

        assert_eq!(drain(&ring), "PLLOG1 dropped=1\r\n");
        assert!(!ring.pending());
    }

    #[test]
    fn a_line_longer_than_the_ring_can_ever_hold_is_counted() {
        let ring = LogRing::<RING>::new();
        let longest = RING - REPORT_MAX;
        assert_eq!(ring.room(longest), Room::Fits);
        assert_eq!(ring.room(longest + 1), Room::Never);

        ring.push(&[b'y'; RING + 1]);
        ring.push(&vec![b'y'; longest + 1]);
        ring.push(b"next\n");

        assert_eq!(drain(&ring), "PLLOG1 dropped=2\r\nnext\n");
    }

    #[test]
    fn the_longest_line_fits_with_a_report_ahead_of_it() {
        let ring = LogRing::<RING>::new();
        ring.dropped.store(u32::MAX, Relaxed);
        let longest = vec![b'z'; RING - REPORT_MAX];

        assert_eq!(ring.room(longest.len()), Room::Fits);
        ring.push(&longest);
        let text = drain(&ring);

        let report = "PLLOG1 dropped=4294967295\r\n";
        assert_eq!(report.len(), REPORT_MAX);
        assert!(text.starts_with(report));
        assert_eq!(text.len(), RING);
    }

    #[test]
    fn the_count_stops_at_its_limit_rather_than_wrap_to_none() {
        let ring = LogRing::<RING>::new();
        ring.dropped.store(u32::MAX, Relaxed);
        ring.push(&[b'y'; RING]);
        assert_eq!(drain(&ring), "PLLOG1 dropped=4294967295\r\n");
    }

    /// A line that waits for room is not a dropped one, even when another
    /// logger takes the room that had opened before the line's turn came.
    #[test]
    fn a_refused_try_push_counts_nothing_and_goes_in_once_there_is_room() {
        let ring = LogRing::<RING>::new();
        let filler = [b'x'; RING - REPORT_MAX];
        assert_eq!(ring.try_push(&filler), Room::Fits);

        let line = [b'w'; 60];
        assert_eq!(ring.try_push(&line), Room::Full);
        assert_eq!(ring.try_push(&[b'y'; RING]), Room::Never);
        ring.pop(RING, |_| {});
        // A logger above this one takes the room first.
        assert_eq!(ring.try_push(&filler), Room::Fits);
        assert_eq!(ring.try_push(&line), Room::Full);
        ring.pop(RING, |_| {});
        assert_eq!(ring.try_push(&line), Room::Fits);

        let text = drain(&ring);
        assert!(text.starts_with("www"), "no report ahead of it: {text}");
        assert_eq!(text.len(), line.len());
        assert!(!ring.pending(), "nothing was counted");
    }

    #[test]
    fn a_half_sent_line_survives_a_full_ring() {
        let ring = LogRing::<RING>::new();
        let first = format!("{}\r\n", "a".repeat(90));
        ring.push(first.as_bytes());
        let mut out = vec![];
        ring.pop(10, |b| out.push(b));

        ring.push(&[b'b'; 90]);
        ring.pop(200, |b| out.push(b));

        assert_eq!(String::from_utf8(out).unwrap(), first);
        assert_eq!(drain(&ring), "PLLOG1 dropped=1\r\n");
    }

    #[test]
    fn only_the_device_apis_lines_may_drop() {
        assert!(may_drop(
            b"PLAPI1 request slot=0 op=status result=status\r\n"
        ));
        for line in [
            &b"PLOWN1 fed id=1\r\n"[..],
            b"PLCNF1 confirmed=1\r\n",
            b"PLSUP1 stop\r\n",
            b"PLAUD1 start\r\n",
            b"PLOTA1 progress sectors=1/2\r\n",
            b"PLFW1 start\r\n",
            b"PLAPI2 other\r\n",
        ] {
            assert!(!may_drop(line));
        }
    }
}
