//! Versioned startup records, formatted without allocation or device writes.

use core::ffi::CStr;
use core::fmt::{self, Write};

pub const BUILD_ID: &str = env!("WELLFED_BUILD_ID");

/// Raw live mapping registers and the factory-preserved reset flags.
#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub control: u32,
    pub start: u32,
    pub offset: u32,
    pub reset: u32,
}

/// Read the live KM0 image mapping and the factory-preserved ROM reset flags.
#[cfg(target_os = "none")]
pub fn snapshot() -> Snapshot {
    let entry = crate::sdk::flash_mmu_entry0();
    Snapshot {
        control: entry.control,
        start: entry.start,
        offset: entry.offset,
        reset: crate::sdk::boot_reason(),
    }
}

impl Snapshot {
    /// Zero-based research-board bank, or unknown for an unrecognized mapping.
    pub fn bank(self) -> Option<u8> {
        crate::sdk::FlashMmuEntry {
            control: self.control,
            start: self.start,
            offset: self.offset,
        }
        .bank()
    }

    /// The bank as a record names it: `0`, `1` or `unknown`.
    pub fn bank_name(self) -> &'static str {
        match self.bank() {
            Some(0) => "0",
            Some(1) => "1",
            _ => "unknown",
        }
    }
}

/// The default record buffer: enough for the fixed `PLFW1 start` schema
/// with a full SHA-256 build ID and raw registers, and for every record
/// line a task logs through [`log`]. A task with longer records sizes its
/// own ([`Report::sized`]), its worst case pinned by a host test.
pub const LINE: usize = 192;

/// What an overflowing record is cut to: its leading words, then this.
const TRUNCATED: &str = " truncated=1\r\n";

/// A record line formatted into a fixed buffer of `N` bytes, at most
/// `N - 1` of text and the NUL.
pub struct Report<const N: usize = LINE> {
    bytes: [u8; N],
    used: usize,
    /// The last record did not fit and was cut.
    cut: bool,
}

impl Default for Report {
    fn default() -> Self {
        Self::new()
    }
}

impl Report {
    pub const fn new() -> Self {
        Self::sized()
    }
}

impl<const N: usize> Report<N> {
    /// A buffer of `N` bytes, for a task whose records outgrow [`LINE`].
    /// Every size must leave room for a NUL, and its longest line must fit
    /// the log tail, or `wellfed logs` would miss the lines the UART shows.
    pub const fn sized() -> Self {
        const { assert!(N >= 1 && N - 1 <= wellfed_core::log_tail::LINE_MAX) };
        Self {
            bytes: [0; N],
            used: 0,
            cut: false,
        }
    }

    pub fn start(&mut self, snapshot: Snapshot) -> &CStr {
        let bank = snapshot.bank_name();
        self.record(format_args!(
            concat!(
                "PLFW1 start build=",
                env!("WELLFED_BUILD_ID"),
                " bank={} reset={:08x} mmu={:08x},{:08x},{:08x}\r\n"
            ),
            bank, snapshot.reset, snapshot.control, snapshot.start, snapshot.offset
        ))
    }

    /// Format a complete record, replacing any previous buffer contents.
    ///
    /// A record that does not fit (or whose formatting fails) is never
    /// dropped or cut mid-value: it becomes its leading words, up to the
    /// first `key=value`, and `truncated=1` (`PLCFG1 saved truncated=1`),
    /// so the bench still sees that it happened and a parser never reads a
    /// half value as a whole one.
    pub fn record(&mut self, args: fmt::Arguments<'_>) -> &CStr {
        self.used = 0;
        self.cut = self.write_fmt(args).is_err();
        if self.cut {
            self.truncate();
        }
        self.terminated()
    }

    /// Whether the last [`record`](Self::record) had to be cut.
    pub fn truncated(&self) -> bool {
        self.cut
    }

    /// Cuts the partial text to its complete leading words without an `=`
    /// and appends [`TRUNCATED`]. Out of line, as is [`Self::terminated`]:
    /// [`Self::record`]'s frame sits under `core::fmt::write`, on a task's
    /// deepest path, and the stack audit counts every byte of it.
    #[cold]
    #[inline(never)]
    fn truncate(&mut self) {
        let room = N.saturating_sub(TRUNCATED.len() + 1);
        let partial = self.bytes.get(..self.used.min(room)).unwrap_or(&[]);
        let mut keep = 0;
        for (at, &byte) in partial.iter().enumerate() {
            match byte {
                b'=' | b'\r' | b'\n' => break,
                b' ' => keep = at,
                _ => {}
            }
        }

        self.used = keep;
        if keep.saturating_add(TRUNCATED.len()) < N {
            self.copy(TRUNCATED.as_bytes());
        }
    }

    #[inline(never)]
    fn terminated(&mut self) -> &CStr {
        if let Some(nul) = self.bytes.get_mut(self.used) {
            *nul = 0;
        }
        // The text is formatted from `str`s, which a record never gives a
        // NUL; one that did ends the line there.
        let text = self.bytes.get(..=self.used).unwrap_or(&[]);
        CStr::from_bytes_until_nul(text).unwrap_or(c"")
    }

    /// Appends what of `bytes` fits before the NUL's byte; how much did.
    ///
    /// No indexing that could panic: [`Write::write_str`] is a formatting
    /// method, which the stack audit charges at its deepest, and a panic's
    /// path through the handler is far deeper than the copy.
    fn copy(&mut self, bytes: &[u8]) -> usize {
        let free = self.bytes.get_mut(self.used..).unwrap_or(&mut []);
        let room = free.len().saturating_sub(1);
        let mut taken = 0;
        for (slot, &byte) in free.iter_mut().zip(bytes.iter().take(room)) {
            *slot = byte;
            taken += 1;
        }
        self.used = self.used.saturating_add(taken);
        taken
    }
}

impl<const N: usize> Write for Report<N> {
    /// Writes what fits, leaving room for the NUL, and fails if that was
    /// not all of it: the record's leading words survive a cut even when
    /// the piece that overflows is one long literal.
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self.copy(value.as_bytes()) < value.len() {
            return Err(fmt::Error);
        }
        Ok(())
    }
}

/// Logs one record line, `PLxxx1 …\r\n`, through a [`LINE`] buffer. A
/// line longer than that is logged cut, as [`Report::record`] says, so a
/// parser never sees half a value.
pub fn log(line: fmt::Arguments<'_>) {
    let mut report = Report::new();
    crate::sdk::log(report.record(line));
}

/// A record a task logs as one `PLxxx1 …` line: its `Display` is the body
/// after the tag. `#[derive(Record)]` with `#[record(tag = "…")]` implements
/// it.
pub trait LogRecord: fmt::Display {
    /// The line's tag, such as `PLBLK1`.
    const TAG: &'static str;

    /// Logs the whole line through [`log`]. The derive writes it with the
    /// tag in the format string, so the line costs no extra argument.
    fn log(&self);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actual_factory_mappings_and_unknown_cases() {
        let mut s = Snapshot {
            control: 3,
            start: 0x0c00_0000,
            offset: 0x3ff_a000,
            reset: 0x10_0000,
        };

        assert_eq!(s.bank(), Some(0));
        s.offset = 0x3e7_a000;
        assert_eq!(s.bank(), Some(1));
        s.control = 0;
        assert_eq!(s.bank(), None);

        s.control = 1;
        s.offset = u32::MAX;
        assert_eq!(s.bank(), None);
        s.control = 3;
        assert_eq!(s.bank(), None);
        s.offset = 0;
        assert_eq!(s.bank(), None);
        s.start = 0;
        assert_eq!(s.bank(), None);
    }

    #[test]
    fn report_retains_raw_reason_and_reuses_buffer_without_stale_bytes() {
        let mut report = Report::new();
        let mut s = Snapshot {
            control: 0,
            start: 0x0c00_0000,
            offset: 0x3e7_a000,
            reset: 0x10_0000,
        };

        let line = report.start(s).to_str().unwrap();
        assert!(line.contains(" bank=unknown reset=00100000 mmu=00000000,0c000000,03e7a000\r\n"));

        s.control = 3;
        s.reset = 0;
        let line = report.start(s).to_str().unwrap();
        assert!(line.contains(" bank=1 reset=00000000 mmu=00000003,0c000000,03e7a000\r\n"));
        assert!(line.starts_with(&format!("PLFW1 start build={BUILD_ID} ")));
    }

    #[test]
    fn a_record_that_fits_is_logged_whole() {
        let mut report = Report::<32>::sized();

        let line = report.record(format_args!("PLCFG1 saved generation={}\r\n", 7));
        assert_eq!(line.to_str().unwrap(), "PLCFG1 saved generation=7\r\n");
        assert!(!report.truncated());
    }

    #[test]
    fn an_overflowing_record_keeps_its_kind_and_says_it_was_cut() {
        let mut report = Report::<40>::sized();

        let line = report.record(format_args!(
            "PLCFG1 net saved generation={} stored=1 event=appended\r\n",
            u64::MAX
        ));
        assert_eq!(line.to_str().unwrap(), "PLCFG1 net saved truncated=1\r\n");
        assert!(report.truncated());

        // The buffer is reused without the cut line's bytes.
        let line = report.record(format_args!("PLCFG1 repaired generation=1\r\n"));
        assert_eq!(line.to_str().unwrap(), "PLCFG1 repaired generation=1\r\n");
    }

    #[test]
    fn a_record_with_no_room_for_its_words_keeps_the_tag() {
        let mut report = Report::<24>::sized();

        let line = report.record(format_args!("PLSUP1 trail_mark_long_word_here mark=a\r\n"));
        assert_eq!(line.to_str().unwrap(), "PLSUP1 truncated=1\r\n");

        let mut tiny = Report::<8>::sized();
        let line = tiny.record(format_args!("PLSUP1 hang x=1\r\n"));
        assert_eq!(line.to_str().unwrap(), "");
        assert!(tiny.truncated());
    }
}
