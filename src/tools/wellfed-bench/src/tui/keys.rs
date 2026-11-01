//! The screen's keys, read from the terminal directly.
//!
//! Crossterm's event source is not used for this because of what it does on
//! a terminal that has hung up: `event::poll` loops on the read returning 0
//! or EIO and never returns, whatever its timeout, so the calling thread
//! spins a core, possibly during a UART write, and can never be joined. No
//! check before the call can help, as the screen sits inside it nearly all
//! the time. So this waits with `poll(2)`, which always honours its timeout,
//! and takes a hangup as an answer.
//!
//! Only the few keys the screen uses are understood; anything else is
//! dropped. A resize needs nothing here: each frame's draw takes the
//! terminal's size as it is then.

use std::fs::File;
use std::io::{self, ErrorKind, Read};
use std::time::Duration;

use rustix::event::{PollFd, PollFlags, Timespec, poll};

/// The longest escape sequence kept while waiting for its end: longer than
/// any the screen understands, so a stray one cannot grow without bound.
const LONGEST_ESCAPE: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Key {
    /// `q`, or Ctrl+C, which raw mode delivers as a byte.
    Quit,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
}

/// What came of waiting on the terminal.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Input {
    Keys(Vec<Key>),
    /// The wait ran out, or what came was not a key.
    Nothing,
    /// The terminal is gone, and nothing more will come from it.
    HungUp,
}

pub(super) struct Keys {
    tty: File,
    /// The start of an escape sequence whose end has not arrived.
    pending: Vec<u8>,
}

impl Keys {
    pub(super) fn new(tty: File) -> Self {
        Self {
            tty,
            pending: Vec::new(),
        }
    }

    /// Wait up to `wait` for keys. It never blocks past that: the read only
    /// follows a `poll` that found input, and this thread is the terminal's
    /// only reader.
    pub(super) fn wait(&mut self, wait: Duration) -> io::Result<Input> {
        match self.ready(wait)? {
            Ready::No => return Ok(Input::Nothing),
            Ready::HungUp => return Ok(Input::HungUp),
            Ready::Input => {}
        }

        let mut chunk = [0; 64];
        let read = match self.tty.read(&mut chunk) {
            Ok(0) => return Ok(Input::HungUp),
            Ok(read) => read,
            Err(error) if error.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error()) => {
                return Ok(Input::HungUp);
            }
            Err(error)
                if matches!(error.kind(), ErrorKind::Interrupted | ErrorKind::WouldBlock) =>
            {
                return Ok(Input::Nothing);
            }
            Err(error) => return Err(error),
        };

        self.pending.extend_from_slice(&chunk[..read]);
        let keys = parse(&mut self.pending);
        Ok(if keys.is_empty() {
            Input::Nothing
        } else {
            Input::Keys(keys)
        })
    }

    /// Whether the terminal has gone, without taking any input.
    pub(super) fn hung_up(&self) -> bool {
        matches!(self.ready(Duration::ZERO), Ok(Ready::HungUp))
    }

    fn ready(&self, wait: Duration) -> io::Result<Ready> {
        let timeout = Timespec {
            tv_sec: i64::try_from(wait.as_secs()).unwrap_or(i64::MAX),
            tv_nsec: i64::from(wait.subsec_nanos()),
        };
        let mut fds = [PollFd::new(&self.tty, PollFlags::IN)];

        match poll(&mut fds, Some(&timeout)) {
            Ok(0) => Ok(Ready::No),
            Ok(_) => {
                let events = fds[0].revents();
                Ok(
                    if events.intersects(PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL) {
                        Ready::HungUp
                    } else {
                        Ready::Input
                    },
                )
            }
            Err(rustix::io::Errno::INTR) => Ok(Ready::No),
            Err(error) => Err(error.into()),
        }
    }
}

enum Ready {
    No,
    Input,
    HungUp,
}

/// Take the keys from the front of `bytes`, leaving an escape sequence that
/// has not ended for the next read.
fn parse(bytes: &mut Vec<u8>) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut at = 0;

    while let Some(first) = bytes.get(at) {
        match first {
            b'q' | 0x03 => {
                keys.push(Key::Quit);
                at += 1;
            }
            0x1b => match escape(&bytes[at..]) {
                Escape::Incomplete if bytes.len() - at <= LONGEST_ESCAPE => break,
                Escape::Incomplete => at += 1,
                Escape::Ended { length, key } => {
                    keys.extend(key);
                    at += length;
                }
            },
            _ => at += 1,
        }
    }

    bytes.drain(..at);
    keys
}

enum Escape {
    Incomplete,
    Ended { length: usize, key: Option<Key> },
}

/// The sequence `bytes` starts with, ESC first: `ESC [ … final` or `ESC O
/// final`. A bare ESC before anything else is dropped alone, so what follows
/// still counts: Alt+q is a `q`, as it was under crossterm.
///
/// A control byte is never part of a sequence, so it ends one early and is
/// read for itself: Ctrl+C after a stray `ESC [` still quits.
fn escape(bytes: &[u8]) -> Escape {
    const fn control(byte: u8) -> bool {
        byte < 0x20
    }
    const fn abandoned(length: usize) -> Escape {
        Escape::Ended { length, key: None }
    }

    match bytes.get(1) {
        None => Escape::Incomplete,
        Some(b'[') => {
            let Some(end) = bytes[2..]
                .iter()
                .position(|byte| control(*byte) || (0x40..=0x7e).contains(byte))
                .map(|end| 2 + end)
            else {
                return Escape::Incomplete;
            };
            if control(bytes[end]) {
                return abandoned(end);
            }

            let (parameters, last) = (&bytes[2..end], bytes[end]);
            let length = end + 1;
            if last == b'M' && parameters.is_empty() {
                return mouse_report(bytes, length);
            }
            Escape::Ended {
                length,
                key: csi(parameters, last),
            }
        }
        Some(b'O') => match bytes.get(2) {
            None => Escape::Incomplete,
            Some(last) if control(*last) => abandoned(2),
            Some(last) => Escape::Ended {
                length: 3,
                key: letter(*last),
            },
        },
        Some(_) => abandoned(1),
    }
}

/// `ESC [ M` and three raw bytes, a legacy mouse report, with its header
/// `length` long. Those bytes must not read as keys: a click at column or row
/// 81 sends `q`, which outside a write stops the server. Each is at least
/// 0x20, so a control byte among them is no report.
fn mouse_report(bytes: &[u8], length: usize) -> Escape {
    let report = &bytes[length..];
    if let Some(control) = report.iter().take(3).position(|byte| *byte < 0x20) {
        return Escape::Ended {
            length: length + control,
            key: None,
        };
    }
    if report.len() < 3 {
        return Escape::Incomplete;
    }
    Escape::Ended {
        length: length + 3,
        key: None,
    }
}

/// A control sequence's key: a final letter, whatever modifiers it carries,
/// or a number and `~`.
fn csi(parameters: &[u8], last: u8) -> Option<Key> {
    if last != b'~' {
        return letter(last);
    }
    let number = parameters.split(|byte| *byte == b';').next()?;
    match number {
        b"1" | b"7" => Some(Key::Home),
        b"4" | b"8" => Some(Key::End),
        b"5" => Some(Key::PageUp),
        b"6" => Some(Key::PageDown),
        _ => None,
    }
}

fn letter(last: u8) -> Option<Key> {
    match last {
        b'A' => Some(Key::Up),
        b'B' => Some(Key::Down),
        b'H' => Some(Key::Home),
        b'F' => Some(Key::End),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::OpenOptionsExt;

    use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};

    use super::*;

    fn keys_of(bytes: &[u8]) -> Vec<Key> {
        let mut bytes = bytes.to_vec();
        let keys = parse(&mut bytes);
        assert!(bytes.is_empty(), "{bytes:?} left over");
        keys
    }

    #[test]
    fn the_screens_keys_read_as_terminals_send_them() {
        assert_eq!(keys_of(b"q"), [Key::Quit]);
        assert_eq!(keys_of(b"\x03"), [Key::Quit]);
        assert_eq!(keys_of(b"\x1bq"), [Key::Quit]);

        assert_eq!(keys_of(b"\x1b[A"), [Key::Up]);
        assert_eq!(keys_of(b"\x1b[B"), [Key::Down]);
        assert_eq!(keys_of(b"\x1b[5~"), [Key::PageUp]);
        assert_eq!(keys_of(b"\x1b[6~"), [Key::PageDown]);

        // Home and End differ between terminals.
        for home in [&b"\x1b[H"[..], b"\x1bOH", b"\x1b[1~", b"\x1b[7~"] {
            assert_eq!(keys_of(home), [Key::Home], "{home:?}");
        }
        for end in [&b"\x1b[F"[..], b"\x1bOF", b"\x1b[4~", b"\x1b[8~"] {
            assert_eq!(keys_of(end), [Key::End], "{end:?}");
        }

        // With a modifier, and in application mode.
        assert_eq!(keys_of(b"\x1b[1;2A"), [Key::Up]);
        assert_eq!(keys_of(b"\x1b[5;5~"), [Key::PageUp]);
        assert_eq!(keys_of(b"\x1bOB"), [Key::Down]);
    }

    #[test]
    fn several_keys_come_in_order_and_anything_else_is_dropped() {
        assert_eq!(
            keys_of(b"x\x1b[Ay\x1b[Zq\xc3\xa9\x1b[99~\x1b[B"),
            [Key::Up, Key::Quit, Key::Down]
        );
        assert_eq!(keys_of(b"Q"), []);
    }

    #[test]
    fn a_sequence_split_across_reads_waits_for_its_end() {
        let mut bytes = b"\x1b[5".to_vec();
        assert_eq!(parse(&mut bytes), []);
        assert_eq!(bytes, b"\x1b[5");

        bytes.extend_from_slice(b"~q");
        assert_eq!(parse(&mut bytes), [Key::PageUp, Key::Quit]);
        assert!(bytes.is_empty());
    }

    #[test]
    fn a_sequence_that_never_ends_cannot_grow_without_bound() {
        let mut bytes = b"\x1b[".to_vec();
        bytes.extend_from_slice(&[b'1'; 100]);

        assert_eq!(parse(&mut bytes), []);

        assert!(bytes.len() <= LONGEST_ESCAPE, "{bytes:?}");
    }

    #[test]
    fn a_mouse_report_is_no_key_even_where_it_spells_a_q() {
        assert_eq!(keys_of(b"\x1b[M\x20\x71\x71"), []);
        assert_eq!(keys_of(b"\x1b[<0;81;81M"), []);
        assert_eq!(keys_of(b"\x1b[M\x20\x71\x71\x1b[A"), [Key::Up]);

        // Split across reads, it still waits for its last byte.
        let mut bytes = b"\x1b[M\x20\x71".to_vec();
        assert_eq!(parse(&mut bytes), []);
        assert_eq!(bytes, b"\x1b[M\x20\x71");
        bytes.push(0x71);
        assert_eq!(parse(&mut bytes), []);
        assert!(bytes.is_empty());
    }

    #[test]
    fn a_control_byte_ends_a_sequence_and_ctrl_c_still_quits() {
        for stray in [
            &b"\x1b[\x03"[..],
            b"\x1b[1;\x03",
            b"\x1bO\x03",
            b"\x1b\x03",
            b"\x1b[M\x03",
            b"\x1b[M\x20\x03",
        ] {
            assert_eq!(keys_of(stray), [Key::Quit], "{stray:?}");
        }

        // Another escape starts afresh.
        assert_eq!(keys_of(b"\x1b[5;\x1b[A"), [Key::Up]);
        assert_eq!(keys_of(b"\x1bO\x1b[B"), [Key::Down]);
    }

    #[test]
    fn a_q_that_ends_a_sequence_is_no_key() {
        assert_eq!(keys_of(b"\x1b[0q"), []);
        assert_eq!(keys_of(b"\x1bOq"), []);
    }

    /// The two ends of a pseudo-terminal in raw enough use: bytes from the
    /// master arrive at the slave as they are read here.
    fn pair() -> (File, File) {
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).unwrap();
        grantpt(&master).unwrap();
        unlockpt(&master).unwrap();
        let path = ptsname(&master, Vec::new()).unwrap();

        let path = std::path::PathBuf::from(path.to_str().unwrap());
        let slave = File::options()
            .read(true)
            .write(true)
            .custom_flags(rustix::fs::OFlags::NOCTTY.bits().cast_signed())
            .open(path)
            .unwrap();
        (File::from(master), slave)
    }

    #[test]
    fn a_wait_honours_its_timeout_and_a_key_ends_it() {
        use std::io::Write;
        use std::time::Instant;

        let (mut master, slave) = pair();
        let mut keys = Keys::new(slave);

        let start = Instant::now();
        assert_eq!(
            keys.wait(Duration::from_millis(100)).unwrap(),
            Input::Nothing
        );
        assert!(start.elapsed() >= Duration::from_millis(90));
        assert!(!keys.hung_up());

        // The line discipline hands a line over whole.
        master.write_all(b"q\n").unwrap();
        assert_eq!(
            keys.wait(Duration::from_secs(5)).unwrap(),
            Input::Keys(vec![Key::Quit])
        );
    }

    #[test]
    fn a_terminal_that_goes_is_an_answer_at_once_every_time() {
        use std::time::Instant;

        let (master, slave) = pair();
        let mut keys = Keys::new(slave);

        drop(master);

        let start = Instant::now();
        assert_eq!(keys.wait(Duration::from_secs(30)).unwrap(), Input::HungUp);
        assert_eq!(keys.wait(Duration::from_secs(30)).unwrap(), Input::HungUp);
        assert!(keys.hung_up());
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
