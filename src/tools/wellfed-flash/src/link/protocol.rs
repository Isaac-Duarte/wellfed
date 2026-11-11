//! The ROM loader and pinned vendor helper's UART protocol.
//!
//! Port of `src/tools/flash/uart-flash-read.py`. The helper executes in RAM and
//! changes controller state, including OTF decryption; speaking its protocol
//! here does not certify ROM or recovery safety. There are no baud changes,
//! cancellations, retries or fallback images.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use wellfed_proto::device::Model;

use serde::Serialize;

use crate::error::Result;
use wellfed_formats::canonical::sha256;
use wellfed_formats::require;

/// The loader, helper and console protocol's refusals. Each stops the
/// session with the transport left as it is; none is retried.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("Interrupted by the operator; no retry")]
    Interrupted,
    #[error("Helper does not match the reviewed vendor binary")]
    HelperMismatch { bytes: usize },
    #[error("Unexpected helper entry pointer")]
    HelperEntry { entry: Option<u32> },
    #[error("Only the pinned helper may be uploaded")]
    UnpinnedHelper { bytes: usize },
    #[error("Invalid helper packet index")]
    HelperPacket { index: usize },

    #[error("Read range must be positive and aligned to 4 KiB")]
    ReadUnaligned { offset: usize, length: usize },
    #[error("Read exceeds the identified flash capacity")]
    ReadBeyondCapacity { offset: usize, length: usize },
    #[error("Read exceeds the helper's sector count")]
    ReadSectorCount { length: usize },
    #[error("Address outside the 8 MiB array")]
    AddressOutside { offset: usize },

    #[error("Serial read deadline expired")]
    SerialDeadline,
    #[error("Partial serial write; aborting without replay")]
    PartialWrite { written: usize, length: usize },
    #[error("Unexpected protocol byte {actual:#04x}; expected {expected:#04x}")]
    UnexpectedByte { actual: u8, expected: u8 },
    #[error("Loader NAK deadline expired")]
    NakDeadline,
    #[error("Unexpected {0} output")]
    UnexpectedOutput(Pending),
    #[error("BAUDCHK acknowledgement deadline expired")]
    BaudcheckDeadline,
    #[error("Unexpected BAUDCHK response")]
    BaudcheckResponse { byte: u8 },

    #[error("Overall read deadline expired")]
    ReadDeadline,
    #[error("Premature EOT; incomplete read")]
    PrematureEot { received: usize, length: usize },
    #[error("Unexpected read control {control:#04x}; no fallback")]
    ReadControl { control: u8 },
    #[error("Packet number/complement mismatch")]
    PacketComplement { number: u8, complement: u8 },
    #[error("Read packet checksum mismatch")]
    PacketChecksum { number: u8 },
    #[error("Changed or excessive duplicate packet")]
    ChangedDuplicate { number: u8 },
    #[error("Out-of-order read packet")]
    OutOfOrder { number: u8, expected: u8 },
    #[error("Read exceeded requested length")]
    ReadOverrun { length: usize },
    #[error("Incomplete region read")]
    IncompleteRegion { received: usize, length: usize },

    #[error("Unexpectedly large console response")]
    ConsoleOverflow { bytes: usize },
    #[error("Observation must be 1-180 seconds")]
    ObservationWindow { seconds: f64 },
    #[error("Console command is outside the fixed ROM-inspection profile")]
    ConsoleCommand,
    /// The press line this tool built does not read back as that press.
    #[error("The press line does not read back as the press asked for")]
    PressLine,
    /// No `PLKEY1 virtual` line came back: the running build is not the
    /// `virtual-panel` variant, or it is not running. Never resent.
    #[error(
        "The feeder did not echo the press within {window_ms} ms: is a virtual-panel build running?"
    )]
    PressUnanswered { window_ms: u128 },
    #[error(
        "The feeder refused the press line (bytes were lost) and echoed no press; check the capture"
    )]
    PressRefused,
    /// Bytes were lost into another valid line, which the build pressed.
    #[error("The feeder pressed something else ({echoed}): bytes were lost; check the capture")]
    PressMismatch { echoed: String },
    #[error("The feeder echoed the press but reported no release of its chord")]
    PressNotReleased,
    #[error("Fault or startup observed during console query; stop and inspect capture")]
    ConsoleFault,
    #[error("Unexpected pre-existing fault output")]
    PreexistingFault,
    #[error("Expected stock KM4 command table not observed; no core switch attempted")]
    Km4Table,
    #[error("Expected stock KM0 help table not observed; no ROM read attempted")]
    Km0Table,
    #[error("Expected exactly one matching ROM word; no retry")]
    RomWordCount { address: u32, found: usize },
    #[error("ROM differs from candidate at {address:#x}; helper upload blocked")]
    RomDiffers { address: u32 },
    #[error("KM4 console return not verified")]
    Km4Return,
}

/// Where unexpected bytes were already buffered: before a helper command,
/// before the helper upload, or before a read request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    Command,
    Upload,
    Read,
}

impl fmt::Display for Pending {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Command => "buffered helper",
            Self::Upload => "pending loader",
            Self::Read => "pending helper",
        })
    }
}

/// The reviewed vendor flash helper, which runs from RAM; no other bytes are
/// ever uploaded.
pub const HELPER_SHA256: &str = "9307121385cb390dfd2da64da2c6c515f17b5a9556b3d04021487c9b9f220b55";
pub const HELPER_BYTES: usize = 4688;
pub const HELPER_ADDRESS: u32 = 0x82000;

pub const CAPACITY: usize = 8 * 1024 * 1024;
pub const SECTOR: usize = 4096;

pub const ACK: u8 = 6;
pub const NAK: u8 = 21;
pub const EOT: u8 = 4;
pub const STX: u8 = 2;

/// The SDK ROM words sampled before any helper upload.
pub const ROM_POINTS: [u32; 17] = [
    0x1298, 0x0474, 0x1434, 0x14C0, 0x1504, 0x2E9C, 0x3594, 0x40B0, 0x43D4, 0x61A0, 0x61D4, 0x6220,
    0x6250, 0x6274, 0x628C, 0x6590, 0x66AC,
];
pub const ROM_BYTES: usize = 54144;
pub const ROM_SHA256: [&str; 2] = [
    "301be574cc338a0bb0a3516f691ec4409064be39fc4144ebfa46488ede9e1612",
    "db0fdd8b2232a0d9edaa403715a9992ae36944fbda4dd936e017c1fa531e66d6",
];

/// Set by the operator's Ctrl-C. Every transport wait checks it and stops
/// terminally, as Python's `KeyboardInterrupt` does; nothing is retried.
pub static INTERRUPTED: AtomicBool = AtomicBool::new(false);

pub fn interrupted() -> Result<()> {
    require(!INTERRUPTED.load(Ordering::SeqCst), || {
        ProtocolError::Interrupted.into()
    })
}

/// A serial-shaped byte transport. `read` returns 0 when nothing arrives
/// within the transport's short poll interval.
pub trait Port {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize>;
    fn write(&mut self, data: &[u8]) -> io::Result<usize>;
    fn flush(&mut self) -> io::Result<()>;
    /// Bytes already received and not yet read.
    fn in_waiting(&mut self) -> io::Result<usize>;
    /// How this session waits for the helper to take a command.
    fn readiness(&self) -> Readiness {
        Readiness::Settled
    }
}

/// How a session waits for the helper's idle-ready NAKs, which it sends
/// every 50 ms while it waits for a command and never mid-reply.
///
/// A command must go right after a NAK arrives: a NAK still on the wire
/// when the command reaches the helper is read as its reply, and the
/// session stops (the loader `0x15` stops after host pauses, when queued
/// NAKs were read in one burst and the command went at a random point of
/// the helper's 50 ms cycle).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    /// Three NAKs in a row, buffered ones included, then the rest drained:
    /// 100 to 150 ms. The stock, recovery and give-way writers keep it, as
    /// every write was first proven with.
    Settled,
    /// Every byte already buffered must be an idle NAK and is dropped, and
    /// the command goes on the next NAK, just sent: half a cycle on
    /// average, and never on a stale NAK whatever the host paused for.
    Fresh,
}

/// A session that waits [`Readiness::Fresh`]. A development update waits
/// before each of thousands of commands; a settled wait each time left
/// the line idle for more than half of it.
pub struct Prompt<P>(pub P);

impl<P: Port> Port for Prompt<P> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }

    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.0.write(data)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }

    fn in_waiting(&mut self) -> io::Result<usize> {
        self.0.in_waiting()
    }

    fn readiness(&self) -> Readiness {
        Readiness::Fresh
    }
}

pub fn load_helper(data: Vec<u8>) -> Result<Vec<u8>> {
    require(
        data.len() == HELPER_BYTES && sha256(&data) == HELPER_SHA256,
        || ProtocolError::HelperMismatch { bytes: data.len() },
    )?;
    let entry = data.first_chunk().map(|word| u32::from_le_bytes(*word));
    require(entry == Some(HELPER_ADDRESS + 0x21), || {
        ProtocolError::HelperEntry { entry }
    })?;
    Ok(data)
}

pub fn read_request(offset: usize, length: usize) -> Result<[u8; 6]> {
    require(
        length > 0 && offset.is_multiple_of(SECTOR) && length.is_multiple_of(SECTOR),
        || ProtocolError::ReadUnaligned { offset, length },
    )?;
    require(offset + length <= CAPACITY, || {
        ProtocolError::ReadBeyondCapacity { offset, length }
    })?;
    let [low, middle, high] = address(offset)?;
    let Ok(sectors) = u16::try_from(length / SECTOR) else {
        return Err(ProtocolError::ReadSectorCount { length }.into());
    };

    let [count_low, count_high] = sectors.to_le_bytes();
    Ok([0x20, low, middle, high, count_low, count_high])
}

/// The three address bytes requests carry; the 8 MiB array needs no more.
fn address(offset: usize) -> Result<[u8; 3]> {
    require(offset < CAPACITY, || ProtocolError::AddressOutside {
        offset,
    })?;
    let Ok(word) = u32::try_from(offset) else {
        return Err(ProtocolError::AddressOutside { offset }.into());
    };

    let [low, middle, high, _] = word.to_le_bytes();
    Ok([low, middle, high])
}

pub fn erase_request(offset: usize) -> Result<[u8; 6]> {
    let [low, middle, high] = address(offset)?;
    Ok([0x17, low, middle, high, 0x01, 0x00])
}

fn checksum(body: &[u8]) -> u8 {
    body.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte))
}

/// One STX frame: packet number, its complement, a 4-byte address, 1 KiB
/// and an 8-bit sum. Callers bound the address before framing.
pub fn frame(number: u8, address: u32, payload: &[u8; 1024]) -> Vec<u8> {
    let mut body = Vec::with_capacity(1028);
    body.extend_from_slice(&address.to_le_bytes());
    body.extend_from_slice(payload);
    let mut out = vec![STX, number, number ^ 0xff];
    let sum = checksum(&body);
    out.extend_from_slice(&body);
    out.push(sum);
    out
}

fn pinned_helper(helper: &[u8]) -> Result<()> {
    require(
        helper.len() == HELPER_BYTES && sha256(helper) == HELPER_SHA256,
        || {
            ProtocolError::UnpinnedHelper {
                bytes: helper.len(),
            }
            .into()
        },
    )
}

pub fn upload_frame(helper: &[u8], index: usize) -> Result<Vec<u8>> {
    pinned_helper(helper)?;
    require(index < helper.len().div_ceil(1024), || {
        ProtocolError::HelperPacket { index }
    })?;

    let mut payload = [0xffu8; 1024];
    let chunk = &helper[index * 1024..helper.len().min((index + 1) * 1024)];
    payload[..chunk.len()].copy_from_slice(chunk);

    // The pinned helper is five packets, so both always fit.
    let (Ok(number), Ok(at)) = (u8::try_from(index + 1), u32::try_from(index * 1024)) else {
        return Err(ProtocolError::HelperPacket { index }.into());
    };
    Ok(frame(number, HELPER_ADDRESS + at, &payload))
}

pub fn exact_read(port: &mut dyn Port, count: usize, timeout: Duration) -> Result<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut data = vec![0; count];
    let mut filled = 0;
    while filled < count {
        require(Instant::now() < deadline, || ProtocolError::SerialDeadline)?;
        interrupted()?;
        filled += port.read(&mut data[filled..])?;
    }
    Ok(data)
}

pub fn send(port: &mut dyn Port, data: &[u8]) -> Result<()> {
    // Checked after the intent's fsync and before the bytes, as Python's
    // KeyboardInterrupt lands before `send` runs.
    interrupted()?;
    let written = port.write(data)?;
    require(written == data.len(), || ProtocolError::PartialWrite {
        written,
        length: data.len(),
    })?;
    Ok(port.flush()?)
}

pub fn expect(port: &mut dyn Port, value: u8) -> Result<()> {
    let actual = exact_read(port, 1, Duration::from_secs(3))?[0];
    require(actual == value, || {
        ProtocolError::UnexpectedByte {
            actual,
            expected: value,
        }
        .into()
    })
}

/// Require consecutive NAKs; any preceding banner stays in the transport log.
pub fn wait_naks(port: &mut dyn Port, count: usize, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    let mut seen = 0;
    let mut byte = [0u8; 1];
    while seen < count {
        require(Instant::now() < deadline, || ProtocolError::NakDeadline)?;
        interrupted()?;
        if port.read(&mut byte)? == 0 {
            continue;
        }
        seen = if byte[0] == NAK { seen + 1 } else { 0 };
    }
    Ok(())
}

/// Drain only idle-ready NAK bytes that are already buffered.
pub fn drain_naks(port: &mut dyn Port, before: Pending) -> Result<()> {
    let pending = port.in_waiting()?;
    if pending > 0 {
        let data = exact_read(port, pending, Duration::from_secs(1))?;
        require(data.iter().all(|byte| *byte == NAK), || {
            ProtocolError::UnexpectedOutput(before)
        })?;
    }
    Ok(())
}

/// Helper idle-ready, by the session's [`Readiness`]; then nothing else
/// buffered.
pub fn command_ready(port: &mut dyn Port) -> Result<()> {
    ready(port, Pending::Command)
}

fn ready(port: &mut dyn Port, before: Pending) -> Result<()> {
    if port.readiness() == Readiness::Fresh {
        drain_naks(port, before)?;
        wait_naks(port, 1, Duration::from_secs(5))?;
    } else {
        wait_naks(port, 3, Duration::from_secs(5))?;
    }
    drain_naks(port, before)
}

/// One request; only already-in-flight NAKs may precede its ACK.
pub fn baudcheck(port: &mut dyn Port) -> Result<()> {
    send(port, &[0x07])?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        require(Instant::now() < deadline, || {
            ProtocolError::BaudcheckDeadline
        })?;
        match exact_read(port, 1, Duration::from_secs(1))?[0] {
            ACK => return Ok(()),
            NAK => {}
            byte => return Err(ProtocolError::BaudcheckResponse { byte }.into()),
        }
    }
}

/// Upload the pinned helper to an already-observed ROM loader.
pub fn upload_helper(port: &mut dyn Port, helper: &[u8]) -> Result<()> {
    pinned_helper(helper)?;
    wait_naks(port, 3, Duration::from_secs(5))?;
    drain_naks(port, Pending::Upload)?;
    baudcheck(port)?;

    for index in 0..helper.len().div_ceil(1024) {
        send(port, &upload_frame(helper, index)?)?;
        expect(port, ACK)?;
    }

    send(port, &[EOT])?;
    expect(port, ACK)?;
    wait_naks(port, 3, Duration::from_secs(5))
}

/// A completed read as reports record it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Transfer {
    pub bytes: usize,
    pub packets: usize,
    pub duplicate_packets: usize,
}

/// Exact checksum-framed read; malformed, partial or cancelled reads fail closed.
///
/// A bounded duplicate of the previous packet is accepted only with an
/// identical payload (a lost ACK). Neither requests nor damaged packets are
/// retried.
pub fn receive_flash(
    port: &mut dyn Port,
    offset: usize,
    length: usize,
    output: &mut dyn Write,
    mut progress: Option<&mut dyn FnMut(usize, usize)>,
) -> Result<Transfer> {
    let request = read_request(offset, length)?;
    ready(port, Pending::Read)?;
    send(port, &request)?;

    let mut total = 0;
    let mut sequence: usize = 1;
    let mut previous: Option<Vec<u8>> = None;
    let mut duplicates = 0;
    // 30 s plus 1/4000 s per byte for the whole read; read_request
    // has already bounded the length by the 8 MiB capacity.
    let per_byte =
        Duration::from_micros(250).saturating_mul(u32::try_from(length).unwrap_or(u32::MAX));
    let deadline = Instant::now() + Duration::from_secs(30) + per_byte;

    loop {
        require(Instant::now() < deadline, || ProtocolError::ReadDeadline)?;
        interrupted()?;
        let control = exact_read(port, 1, Duration::from_secs(5))?[0];
        if control == EOT {
            require(total == length, || ProtocolError::PrematureEot {
                received: total,
                length,
            })?;
            // The helper is idle again. A fresh session's next command
            // waits for its own NAK, so one is enough here.
            let naks = match port.readiness() {
                Readiness::Settled => 3,
                Readiness::Fresh => 1,
            };
            send(port, &[ACK])?;
            wait_naks(port, naks, Duration::from_secs(5))?;
            return Ok(Transfer {
                bytes: total,
                packets: total / 1024,
                duplicate_packets: duplicates,
            });
        }

        require(control == STX, || ProtocolError::ReadControl { control })?;

        let frame = exact_read(port, 1027, Duration::from_secs(3))?;
        let (number, complement) = (frame[0], frame[1]);
        let payload = &frame[2..1026];
        require(number ^ complement == 0xff, || {
            ProtocolError::PacketComplement { number, complement }
        })?;
        require(checksum(payload) == frame[1026], || {
            ProtocolError::PacketChecksum { number }
        })?;

        if previous.is_some() && number == (sequence - 1).to_le_bytes()[0] {
            require(
                previous.as_deref() == Some(payload) && duplicates < 8,
                || ProtocolError::ChangedDuplicate { number },
            )?;
            duplicates += 1;
            send(port, &[ACK])?;
            continue;
        }

        let expected = sequence.to_le_bytes()[0];
        require(number == expected, || ProtocolError::OutOfOrder {
            number,
            expected,
        })?;
        require(total + payload.len() <= length, || {
            ProtocolError::ReadOverrun { length }
        })?;

        output.write_all(payload)?;
        total += payload.len();
        previous = Some(payload.to_vec());
        sequence += 1;
        send(port, &[ACK])?;

        if let Some(progress) = progress.as_mut()
            && (total % (256 * 1024) == 0 || total == length)
        {
            progress(total, length);
        }
    }
}

/// Read one region into memory.
pub fn read_region(port: &mut dyn Port, offset: usize, length: usize) -> Result<Vec<u8>> {
    let mut output = Vec::with_capacity(length);
    receive_flash(port, offset, length, &mut output, None)?;
    require(output.len() == length, || ProtocolError::IncompleteRegion {
        received: output.len(),
        length,
    })?;
    Ok(output)
}

pub fn collect(port: &mut dyn Port, timeout: Duration) -> Result<Vec<u8>> {
    collect_until(port, timeout, |_| false, Duration::ZERO)
}

/// What `port` sends until `timeout`, or until `done` holds for all of it
/// so far, whichever comes first; then `tail` more of listening, so what the
/// console prints right after the reply (a fault, a reboot's banner) is in
/// it too.
fn collect_until(
    port: &mut dyn Port,
    timeout: Duration,
    done: impl Fn(&[u8]) -> bool,
    tail: Duration,
) -> Result<Vec<u8>> {
    let mut deadline = Instant::now() + timeout;
    let mut ended = false;
    let mut result = Vec::new();
    let mut buffer = [0u8; 4096];
    while Instant::now() < deadline {
        interrupted()?;
        let count = port.read(&mut buffer)?;
        result.extend_from_slice(&buffer[..count]);
        require(result.len() <= 512 * 1024, || {
            ProtocolError::ConsoleOverflow {
                bytes: result.len(),
            }
        })?;
        if !ended && count > 0 && done(&result) {
            ended = true;
            deadline = Instant::now() + tail;
        }
    }
    Ok(result)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    let mut found = 0;
    let mut at = 0;
    while at + needle.len() <= haystack.len() {
        if &haystack[at..at + needle.len()] == needle {
            found += 1;
            at += needle.len();
        } else {
            at += 1;
        }
    }
    found
}

const FAULTS: [&[u8]; 3] = [b"Hard Fault", b"HardFault", b"Fault on thread"];

/// What a receive-only observation saw.
#[derive(Debug, Clone, Serialize)]
pub struct Observation {
    pub bytes: usize,
    pub transmitted_bytes: usize,
    pub nak_bytes: usize,
    pub longest_nak_run: usize,
    pub download_banner_seen: bool,
    pub stock_startup_markers: usize,
    pub fault_marker_seen: bool,
    pub limit: &'static str,
}

/// The bounded receive-only window, checked before the port opens. The range
/// is checked on the operator's fractional seconds before they become a span,
/// so rounding to whole nanoseconds can never admit a value just outside it.
pub fn observation_window(seconds: f64) -> Result<Duration, ProtocolError> {
    require((1.0..=180.0).contains(&seconds), || {
        ProtocolError::ObservationWindow { seconds }
    })?;
    Duration::try_from_secs_f64(seconds).map_err(|_| ProtocolError::ObservationWindow { seconds })
}

/// Receive only: NAKs are observations, never an automatic loader assertion.
/// The window is checked again here, after the port opens.
pub fn observe(port: &mut dyn Port, window: Duration) -> Result<Observation> {
    require(
        (Duration::from_secs(1)..=Duration::from_secs(180)).contains(&window),
        || ProtocolError::ObservationWindow {
            seconds: window.as_secs_f64(),
        },
    )?;

    let data = collect(port, window)?;
    let longest = data
        .split(|byte| *byte != NAK)
        .map(<[u8]>::len)
        .max()
        .unwrap_or(0);

    Ok(Observation {
        bytes: data.len(),
        transmitted_bytes: 0,
        nak_bytes: data.iter().filter(|byte| **byte == NAK).count(),
        longest_nak_run: longest,
        download_banner_seen: contains(&data, b"#Flash Download Start"),
        stock_startup_markers: count(&data, b"app_main:version:"),
        fault_marker_seen: FAULTS.iter().any(|fault| contains(&data, fault)),
        limit: "Serial output only; operator must establish power/strap state and loader identity",
    })
}

fn rom_command(command: &[u8]) -> bool {
    command == b"?\r\n"
        || command == b">>\r\n"
        || (command.len() == 13
            && command.starts_with(b"DW 0000")
            && command.ends_with(b"\r\n")
            && command[7..11]
                .iter()
                .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(b)))
}

pub fn console_query(port: &mut dyn Port, command: &[u8], window: Duration) -> Result<Vec<u8>> {
    console_query_until(port, command, window, |_| false, Duration::ZERO)
}

/// How long the inspection's last query listens after its reply has ended:
/// no later query would see a fault or a reboot printed right after it.
const QUIET_TAIL: Duration = Duration::from_millis(200);

/// A console query that returns as soon as `done` holds for the reply so far,
/// or at `window`. A stock app that logs every few seconds (the PLWF106's
/// Wi-Fi retries) leaves gaps too short for a fixed-window inspection, so the
/// replies the inspection can recognize end it early. What a reply must say
/// is unchanged: `done` only decides when to stop listening, `tail` after
/// it, and the fault checks cover the tail too.
fn console_query_until(
    port: &mut dyn Port,
    command: &[u8],
    window: Duration,
    done: impl Fn(&[u8]) -> bool,
    tail: Duration,
) -> Result<Vec<u8>> {
    require(rom_command(command), || ProtocolError::ConsoleCommand)?;

    send(port, command)?;
    let reply = collect_until(port, window, done, tail)?;
    require(
        !FAULTS
            .iter()
            .chain([&b"app_main:version:"[..]].iter())
            .any(|marker| contains(&reply, marker)),
        || ProtocolError::ConsoleFault,
    )?;

    Ok(reply)
}

pub fn is_km0_help(reply: &[u8]) -> bool {
    [
        &b"TEST COMMAND MODE HELP 12"[..],
        b"switch shell to KM4",
        b"HSSDM32K",
        b"CTC",
        b"FWDBG",
        b"TEST COMMAND MODE END  c",
    ]
    .iter()
    .all(|marker| contains(reply, marker))
        && !contains(reply, b"switch shell to KM0")
}

/// The command-table address each model's stock application prints in its
/// KM4 help, which ties a console to a stock build we have observed: the
/// PLAF103 feeder's (1.1.15) at 0e11df78, the PLWF106 fountain's (0.0.17) at
/// 0e0ba120. No wildcard, so a new model cannot land without its table. The
/// ROM words sampled afterwards stay the check that gates a helper upload.
const fn stock_km4_table(model: Model) -> &'static [u8] {
    match model {
        Model::Plaf103 => b"0e11df78",
        Model::Plwf106 => b"0e0ba120",
    }
}

/// The model whose stock KM4 help `reply` is, if it is one.
fn km4_help_model(reply: &[u8]) -> Option<Model> {
    if !contains(reply, b"switch shell to KM0") {
        return None;
    }
    Model::ALL
        .into_iter()
        .find(|&model| contains(reply, stock_km4_table(model)))
}

/// Every `ADDRESS:<spaces>XXXXXXXX` word in a `DW` reply.
fn dumped_words(reply: &[u8], address: u32) -> Vec<u32> {
    let label = format!("{address:08X}:");
    let label = label.as_bytes();
    let mut values = Vec::new();
    let mut at = 0;

    while at + label.len() <= reply.len() {
        if &reply[at..at + label.len()] != label {
            at += 1;
            continue;
        }

        let mut cursor = at + label.len();
        // Python's bytes `\s`, which also includes vertical tab.
        while cursor < reply.len()
            && matches!(reply[cursor], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
        {
            cursor += 1;
        }
        let digits = &reply[cursor..reply.len().min(cursor + 8)];
        let word = std::str::from_utf8(digits)
            .ok()
            .filter(|digits| digits.len() == 8 && digits.bytes().all(|b| b.is_ascii_hexdigit()))
            .and_then(|digits| u32::from_str_radix(digits, 16).ok());
        match word {
            Some(word) => {
                values.push(word);
                at = cursor + 8;
            }
            None => at += 1,
        }
    }
    values
}

/// A help table has printed its closing line.
fn help_ended(reply: &[u8]) -> bool {
    let marker = b"TEST COMMAND MODE END";
    reply
        .windows(marker.len())
        .position(|window| window == marker)
        .is_some_and(|at| reply[at..].contains(&b'\n'))
}

/// A `DW` reply holds its one word and the line it is on has ended.
fn word_ended(reply: &[u8], address: u32) -> bool {
    reply.ends_with(b"\n") && dumped_words(reply, address).len() == 1
}

pub fn rom_sample_addresses() -> Vec<u32> {
    ROM_POINTS
        .iter()
        .flat_map(|base| (0..4).map(move |word| (base & !3) + word * 4))
        .collect()
}

/// The core that owns the stock console when ROM inspection starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Core {
    Km4,
    Km0,
}

#[derive(Debug, Clone, Serialize)]
pub struct Sample {
    pub address: String,
    pub value: String,
}

/// The sampled ROM words and the pinned candidates they still match.
#[derive(Debug, Clone, Serialize)]
pub struct RomInspection {
    pub sampled_words: Vec<Sample>,
    /// Candidate path to its SHA-256.
    pub matching_candidates: BTreeMap<String, String>,
    pub returned_to_km4: bool,
    /// The model whose stock KM4 command table the console printed.
    pub stock_model: Model,
    pub limit: &'static str,
}

/// Enter KM0 only after verifying both actual help-table identities.
pub fn inspect_rom(
    port: &mut dyn Port,
    candidates: &[(String, Vec<u8>)],
    start: Core,
) -> Result<RomInspection> {
    let initial = collect(port, Duration::from_millis(500))?;
    require(!contains(&initial, b"Hard Fault"), || {
        ProtocolError::PreexistingFault
    })?;

    let mut opened = None;
    if start == Core::Km4 {
        let high = console_query_until(
            port,
            b"?\r\n",
            Duration::from_secs(1),
            help_ended,
            Duration::ZERO,
        )?;
        opened = Some(km4_help_model(&high).ok_or(ProtocolError::Km4Table)?);
        console_query(port, b">>\r\n", Duration::from_millis(500))?;
    }

    let low = console_query_until(
        port,
        b"?\r\n",
        Duration::from_secs(1),
        help_ended,
        Duration::ZERO,
    )?;
    require(is_km0_help(&low), || ProtocolError::Km0Table)?;

    let mut samples = Vec::new();
    let mut surviving: Vec<&(String, Vec<u8>)> = candidates.iter().collect();

    for address in rom_sample_addresses() {
        let command = format!("DW {address:08X}\r\n");
        let response = console_query_until(
            port,
            command.as_bytes(),
            Duration::from_millis(250),
            |reply| word_ended(reply, address),
            Duration::ZERO,
        )?;
        let values = dumped_words(&response, address);
        require(values.len() == 1, || ProtocolError::RomWordCount {
            address,
            found: values.len(),
        })?;

        let value = values[0];
        let at = address as usize;
        surviving.retain(|(_, rom)| rom.get(at..at + 4) == Some(&value.to_le_bytes()[..]));
        require(!surviving.is_empty(), || ProtocolError::RomDiffers {
            address,
        })?;

        samples.push(Sample {
            address: format!("0x{address:08x}"),
            value: format!("0x{value:08x}"),
        });
    }

    console_query(port, b">>\r\n", Duration::from_millis(500))?;
    let high = console_query_until(
        port,
        b"?\r\n",
        Duration::from_secs(1),
        help_ended,
        QUIET_TAIL,
    )?;
    let stock_model = km4_help_model(&high).ok_or(ProtocolError::Km4Return)?;
    // A console that came back as another model's is not the one inspected.
    require(opened.is_none_or(|model| model == stock_model), || {
        ProtocolError::Km4Return
    })?;

    Ok(RomInspection {
        sampled_words: samples,
        matching_candidates: surviving
            .iter()
            .map(|(name, rom)| (name.clone(), sha256(rom)))
            .collect(),
        returned_to_km4: true,
        stock_model,
        limit: "Sampled ROM identity only; not whole-ROM or all-side-effect verification",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn km4_help_names_the_model_whose_stock_table_it_lists() {
        let help = |shell: &str, table: &str| {
            format!(
                "\t>> \r\n\t\t switch shell to {shell} \r\n--- TEST COMMAND MODE END  12 [{table}]---\r\n"
            )
        };
        let feeder = help("KM0", "0e11df78:0e11e038");
        let fountain = help("KM0", "0e0ba120:0e0ba1e0");
        let unknown = help("KM0", "0e000000:0e0000c0");
        let km0 = help("KM4", "0e0ba120:0e0ba1e0");
        let (feeder, fountain, unknown, km0) = (
            feeder.as_bytes(),
            fountain.as_bytes(),
            unknown.as_bytes(),
            km0.as_bytes(),
        );

        assert_eq!(km4_help_model(feeder), Some(Model::Plaf103));
        assert_eq!(km4_help_model(fountain), Some(Model::Plwf106));
        assert_eq!(km4_help_model(unknown), None);
        assert_eq!(km4_help_model(km0), None);
    }

    /// A line holding `buffered`, which `in_waiting` counts, and then
    /// `arriving`, a byte per read once the buffer is empty: what the
    /// helper sends after the session looks.
    struct Line {
        buffered: std::collections::VecDeque<u8>,
        arriving: std::collections::VecDeque<u8>,
    }

    fn line(buffered: &[u8], arriving: &[u8]) -> Line {
        Line {
            buffered: buffered.iter().copied().collect(),
            arriving: arriving.iter().copied().collect(),
        }
    }

    impl Port for Line {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let next = self
                .buffered
                .pop_front()
                .or_else(|| self.arriving.pop_front());
            match (next, buffer.first_mut()) {
                (Some(byte), Some(slot)) => {
                    *slot = byte;
                    Ok(1)
                }
                _ => Ok(0),
            }
        }

        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            Ok(data.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }

        fn in_waiting(&mut self) -> io::Result<usize> {
            Ok(self.buffered.len())
        }
    }

    /// A settled session counts buffered NAKs, a stray byte restarting the
    /// count, so after a pause it goes on NAKs queued long ago.
    #[test]
    fn a_settled_session_counts_three_naks_buffered_or_not() {
        let mut settled = line(&[NAK, b'A', NAK, NAK, NAK], &[NAK]);
        command_ready(&mut settled).unwrap();
        assert!(settled.buffered.is_empty());
        assert_eq!(settled.arriving.len(), 1, "went on buffered NAKs");
    }

    /// A fresh session drops the NAKs queued while it paused and goes on
    /// the next one, so the command leaves just after a NAK, never at a
    /// random point of the helper's cycle; stray output still stops it.
    #[test]
    fn a_fresh_session_goes_on_a_new_nak_and_refuses_stray_output() {
        let mut fresh = Prompt(line(&[NAK, NAK, NAK, NAK], &[NAK, NAK]));
        command_ready(&mut fresh).unwrap();
        assert!(fresh.0.buffered.is_empty());
        assert_eq!(fresh.0.arriving.len(), 1, "waited for one NAK, not more");

        let mut stray = Prompt(line(&[NAK, b'A'], &[NAK]));
        assert!(matches!(
            command_ready(&mut stray),
            Err(crate::Error::Protocol(ProtocolError::UnexpectedOutput(
                Pending::Command
            )))
        ));
    }

    /// Only the outer session decides: the evidence log and a box pass the
    /// readiness through, and every other session stays settled.
    #[test]
    fn the_readiness_is_the_outer_sessions() {
        let boxed: Box<dyn Port> = Box::new(line(&[], &[]));
        assert_eq!(boxed.readiness(), Readiness::Settled);
        assert_eq!(Prompt(boxed).readiness(), Readiness::Fresh);
        let inner: Box<dyn Port> = Box::new(Prompt(line(&[], &[])));
        assert_eq!(inner.readiness(), Readiness::Fresh);
    }

    /// A console that prints its reply in timed pieces.
    struct Scripted {
        start: Option<Instant>,
        pieces: Vec<(Duration, &'static [u8])>,
    }

    impl Port for Scripted {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let due = self.start.is_some_and(|start| {
                self.pieces
                    .first()
                    .is_some_and(|(after, _)| start.elapsed() >= *after)
            });
            if !due {
                std::thread::sleep(Duration::from_millis(5));
                return Ok(0);
            }
            let (_, piece) = self.pieces.remove(0);
            buffer[..piece.len()].copy_from_slice(piece);
            Ok(piece.len())
        }

        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.start = Some(Instant::now());
            Ok(data.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }

        fn in_waiting(&mut self) -> io::Result<usize> {
            Ok(0)
        }
    }

    /// The last help ends at its END line, and a fault printed right after
    /// it still refuses the inspection; without the tail it would be missed.
    #[test]
    fn the_last_reply_watches_its_tail_for_a_fault() {
        let script = || Scripted {
            start: None,
            pieces: vec![
                (
                    Duration::ZERO,
                    b"--- TEST COMMAND MODE END  12 [0e11df78:0e11e038]---\r\n",
                ),
                (Duration::from_millis(60), b"Hard Fault\r\n"),
            ],
        };
        let window = Duration::from_secs(1);

        let watched = console_query_until(&mut script(), b"?\r\n", window, help_ended, QUIET_TAIL);
        assert!(
            matches!(
                watched,
                Err(crate::Error::Protocol(ProtocolError::ConsoleFault))
            ),
            "{watched:?}"
        );
        let unwatched =
            console_query_until(&mut script(), b"?\r\n", window, help_ended, Duration::ZERO);
        assert!(unwatched.is_ok(), "{unwatched:?}");
    }

    #[test]
    fn console_replies_end_only_once_their_line_is_whole() {
        assert!(help_ended(
            b"--- TEST COMMAND MODE END  12 [0e0ba120:0e0ba1e0]---\r\n"
        ));
        assert!(!help_ended(b"--- TEST COMMAND MODE END  12 [0e0ba1"));
        assert!(!help_ended(b"\t>> \r\n\t\t switch shell to KM0 \r\n"));

        assert!(word_ended(
            b"DW 00001298\r\n00001298: 4B0FB40F \r\n",
            0x1298
        ));
        assert!(!word_ended(b"DW 00001298\r\n00001298: 4B0F", 0x1298));
        assert!(!word_ended(b"DW 00001298\r\n00001298: 4B0FB40F ", 0x1298));
        assert!(!word_ended(
            b"DW 00001298\r\n0000129C: 4B0FB40F \r\n",
            0x1298
        ));
    }

    #[test]
    fn requests_are_aligned_and_bounded() {
        assert_eq!(
            read_request(0x186000, 0x2000).unwrap(),
            [0x20, 0x00, 0x60, 0x18, 0x02, 0x00]
        );
        assert!(matches!(
            read_request(0x100, 4096),
            Err(crate::Error::Protocol(ProtocolError::ReadUnaligned {
                offset: 0x100,
                ..
            }))
        ));
        assert!(matches!(
            read_request(0, 0),
            Err(crate::Error::Protocol(ProtocolError::ReadUnaligned {
                length: 0,
                ..
            }))
        ));
        assert!(matches!(
            read_request(CAPACITY - 4096, 8192),
            Err(crate::Error::Protocol(
                ProtocolError::ReadBeyondCapacity { .. }
            ))
        ));
        assert_eq!(
            erase_request(0x186000).unwrap(),
            [0x17, 0x00, 0x60, 0x18, 1, 0]
        );
        assert!(matches!(
            erase_request(CAPACITY),
            Err(crate::Error::Protocol(ProtocolError::AddressOutside {
                offset: CAPACITY
            }))
        ));
        assert_eq!(
            ProtocolError::UnexpectedOutput(Pending::Upload).to_string(),
            "Unexpected pending loader output"
        );
        assert_eq!(
            ProtocolError::UnexpectedByte {
                actual: 0x15,
                expected: 6
            }
            .to_string(),
            "Unexpected protocol byte 0x15; expected 0x06"
        );
    }

    #[test]
    fn frames_carry_complement_address_and_sum() {
        let frame = frame(3, 0x0818_6000, &[1; 1024]);
        assert_eq!(&frame[..7], &[STX, 3, 0xfc, 0x00, 0x60, 0x18, 0x08]);
        assert_eq!(frame.len(), 1032);
        let sum = (0x60u32 + 0x18 + 0x08 + 1024) as u8;
        assert_eq!(frame[1031], sum);
    }

    #[test]
    fn rom_profile_allows_only_fixed_commands() {
        assert!(rom_command(b"?\r\n"));
        assert!(rom_command(b"DW 00001298\r\n"));
        assert!(!rom_command(b"DW 00001298 4\r\n"));
        assert!(!rom_command(b"DW 0000129a\r\n"));
        assert!(!rom_command(b"EW 00001298\r\n"));
    }

    #[test]
    fn dumped_word_parsing_matches_the_regex() {
        let reply = b"DW 00001298\r\n00001298:  4B02B510\r\n";
        assert_eq!(dumped_words(reply, 0x1298), vec![0x4B02B510]);
        assert!(dumped_words(b"00001298: 4B02", 0x1298).is_empty());
        assert_eq!(rom_sample_addresses().len(), 68);
        assert_eq!(rom_sample_addresses()[..2], [0x1298, 0x129c]);
    }
}

/// A virtual-panel press and what the feeder said to it.
#[derive(Debug, Clone, Serialize)]
pub struct PressResult {
    pub chord: String,
    pub hold_ms: u32,
    /// The build echoed this exact press, then its chord's release.
    pub echoed: bool,
    pub released: bool,
    pub bytes: usize,
    pub limit: &'static str,
}

/// Gap between the press line's bytes: the build drains the UART's 16-byte
/// FIFO every 5 ms, so one byte every 2 ms keeps it from filling unless the
/// button task is held off (an overrun then refuses the line).
const PRESS_BYTE_GAP: Duration = Duration::from_millis(2);
/// How long past the hold the feeder has to echo the press and report the
/// release: the echo is at once, the release a debounce after the hold.
const PRESS_MARGIN: Duration = Duration::from_millis(1500);

/// A byte no press line holds, so any partial line it ends is refused.
const POISON: char = '!';

/// What the build said about a press line, from every `PLKEY1 virtual`
/// line in the reply: a stray byte before the line ends a refused line of
/// its own first, so the first echo is not always the press's.
#[derive(Debug, PartialEq, Eq)]
enum Echo {
    /// The exact echo, and whether its chord's release followed.
    Pressed { released: bool },
    /// The build pressed something else: bytes were lost into another
    /// valid line.
    Other { echoed: String },
    /// Only refused lines: the press line lost bytes.
    Refused,
    /// No echo at all: not a virtual-panel build, or not running.
    Silent,
}

impl Echo {
    fn read(reply: &[u8], press: wellfed_core::virtual_panel::Press) -> Self {
        let text = String::from_utf8_lossy(reply);
        let pressed = format!(
            "PLKEY1 virtual press={} hold_ms={}",
            press.chord, press.hold_ms
        );
        let released = format!("PLKEY1 up chord={} ", press.chord);
        // The build's log lines can start with a stray `\r` as well as end
        // with one, and the first line after the feeder restarts can start
        // with a NUL (both physical, October 29 and 30), so control
        // characters are trimmed from both ends with the whitespace.
        let lines: Vec<&str> = text
            .lines()
            .map(|line| line.trim_matches(|c: char| c.is_whitespace() || c.is_control()))
            .collect();

        // The first press the build echoes must be this one: an earlier one
        // pressed something nobody asked for in this job.
        if let Some(at) = lines
            .iter()
            .position(|line| line.starts_with("PLKEY1 virtual press="))
        {
            if lines[at] != pressed {
                return Self::Other {
                    echoed: lines[at].into(),
                };
            }
            return Self::Pressed {
                released: lines[at..].iter().any(|line| line.starts_with(&released)),
            };
        }
        if lines.contains(&"PLKEY1 virtual refused") {
            return Self::Refused;
        }
        Self::Silent
    }
}

/// Types one press line to a running `virtual-panel` build, paced, and
/// waits for its echo and its chord's release: it returns once both are in
/// whole lines, and otherwise reads until the hold plus [`PRESS_MARGIN`],
/// so a wrong answer is classified on everything the window brought. The line
/// is built here from the typed press and read back through the firmware's
/// own parser, so nothing else is ever sent. It starts with [`POISON`] and
/// a line end, which end any partial line the build holds as a refused one:
/// a bare line end would complete a cut-off `press dispense` into a press.
/// It is sent once, never again: a line the build read wrongly may already
/// have pressed something.
pub fn press(
    port: &mut dyn Port,
    press: wellfed_core::virtual_panel::Press,
) -> Result<PressResult> {
    use wellfed_core::virtual_panel::Line;

    let line = format!("press {} hold {}", press.chord, press.hold_ms);
    require(Line::parse(&line) == Some(Line::Press(press)), || {
        ProtocolError::PressLine
    })?;

    for byte in format!("{POISON}\n{line}\n").as_bytes() {
        send(port, std::slice::from_ref(byte))?;
        std::thread::sleep(PRESS_BYTE_GAP);
    }
    let window = Duration::from_millis(u64::from(press.hold_ms)) + PRESS_MARGIN;
    let reply = collect_until(
        port,
        window,
        |reply| {
            // Whole lines only: a release record cut mid-line is not read yet.
            let whole = reply
                .iter()
                .rposition(|&byte| byte == b'\n')
                .map_or(&reply[..0], |end| &reply[..=end]);
            Echo::read(whole, press) == Echo::Pressed { released: true }
        },
        Duration::ZERO,
    )?;

    let released = match Echo::read(&reply, press) {
        Echo::Pressed { released } => released,
        Echo::Refused => return Err(ProtocolError::PressRefused.into()),
        Echo::Other { echoed } => return Err(ProtocolError::PressMismatch { echoed }.into()),
        Echo::Silent => {
            return Err(ProtocolError::PressUnanswered {
                window_ms: window.as_millis(),
            }
            .into());
        }
    };
    require(released, || ProtocolError::PressNotReleased)?;

    Ok(PressResult {
        chord: press.chord.to_string(),
        hold_ms: press.hold_ms,
        echoed: true,
        released,
        bytes: reply.len(),
        limit: "The build read the line and its chord went down and up; what the gesture did is in the capture",
    })
}

#[cfg(test)]
mod press_tests {
    use super::*;
    use wellfed_core::buttons::{Button, Buttons};
    use wellfed_core::virtual_panel::Press;

    /// A feeder answering a whole press line with `reply`.
    struct Feeder {
        answer: &'static [u8],
        sent: Vec<u8>,
        reply: Vec<u8>,
    }

    impl Feeder {
        fn new(answer: &'static [u8]) -> Self {
            Self {
                answer,
                sent: Vec::new(),
                reply: Vec::new(),
            }
        }
    }

    impl Port for Feeder {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.reply.is_empty() {
                std::thread::sleep(Duration::from_millis(10));
                return Ok(0);
            }
            let count = self.reply.len().min(buffer.len());
            buffer[..count].copy_from_slice(&self.reply[..count]);
            self.reply.drain(..count);
            Ok(count)
        }

        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.sent.extend_from_slice(data);
            if self.sent.ends_with(b"150\n") {
                self.reply.extend_from_slice(self.answer);
            }
            Ok(data.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }

        fn in_waiting(&mut self) -> io::Result<usize> {
            Ok(self.reply.len())
        }
    }

    const DISPENSE: Press = Press {
        chord: Buttons::of(Button::Dispense),
        hold_ms: 150,
    };
    const LINE: &[u8] = b"!\npress dispense hold 150\n";

    #[test]
    fn a_press_sends_its_one_line_and_reads_the_echo_and_release() {
        let mut feeder = Feeder::new(
            b"PLKEY1 virtual press=dispense hold_ms=150\r\nPLKEY1 down chord=dispense dropped=0\r\n\
              PLKEY1 up chord=dispense level=0 held_ms=150 dropped=0\r\n",
        );
        let started = std::time::Instant::now();
        let result = press(&mut feeder, DISPENSE).unwrap();
        assert!(result.echoed && result.released);
        assert_eq!(feeder.sent, LINE);
        // It returned on the release, well before the window's end.
        assert!(started.elapsed() < PRESS_MARGIN, "{:?}", started.elapsed());
    }

    /// A release record still cut mid-line does not end the read early:
    /// the press waits out its window for the rest, and the outcome is
    /// then classified as before.
    #[test]
    fn a_release_cut_mid_line_waits_out_the_window() {
        let mut feeder = Feeder::new(
            b"PLKEY1 virtual press=dispense hold_ms=150\r\nPLKEY1 up chord=dispense lev",
        );
        let started = std::time::Instant::now();
        assert!(press(&mut feeder, DISPENSE).unwrap().released);
        assert!(started.elapsed() >= PRESS_MARGIN, "{:?}", started.elapsed());
        assert_eq!(feeder.sent, LINE);
    }

    #[test]
    fn records_framed_as_the_feeder_frames_them_still_match() {
        // The bytes the first hardware press returned.
        let mut feeder = Feeder::new(
            b"PLKEY1 virtual press=dispense hold_ms=150\r\n\rPLKEY1 down chord=dispense dropped=0\r\n\
              \rPLKEY1 up chord=dispense level=0 held_ms=143 dropped=0\r\n",
        );
        assert!(press(&mut feeder, DISPENSE).unwrap().released);
    }

    #[test]
    fn a_nul_before_the_echo_still_matches() {
        // The bytes the install's first press returned, on the feeder's
        // first log line since it restarted into the replacement.
        let mut feeder = Feeder::new(
            b"\0\rPLKEY1 virtual press=dispense hold_ms=150\r\n\rPLKEY1 down chord=dispense dropped=0\r\n\
              \rPLKEY1 up chord=dispense level=0 held_ms=143 dropped=0\r\n",
        );
        assert!(press(&mut feeder, DISPENSE).unwrap().released);
    }

    /// A feeder whose firmware line reader is the build's own, echoing each
    /// line it finishes as the build does, with every press it took.
    struct Panel {
        reader: wellfed_core::virtual_panel::Reader,
        pressed: Vec<Press>,
        reply: Vec<u8>,
    }

    impl Port for Panel {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.reply.is_empty() {
                std::thread::sleep(Duration::from_millis(10));
                return Ok(0);
            }
            let count = self.reply.len().min(buffer.len());
            buffer[..count].copy_from_slice(&self.reply[..count]);
            self.reply.drain(..count);
            Ok(count)
        }

        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            use wellfed_core::virtual_panel::Line;

            for &byte in data {
                let echo = match self.reader.byte(byte) {
                    Some(Line::Press(press)) => {
                        self.pressed.push(press);
                        format!(
                            "PLKEY1 virtual press={} hold_ms={}\r\n\
                             PLKEY1 up chord={} level=0 held_ms={} dropped=0\r\n",
                            press.chord, press.hold_ms, press.chord, press.hold_ms
                        )
                    }
                    Some(Line::Refused) => "PLKEY1 virtual refused\r\n".into(),
                    None => continue,
                };
                self.reply.extend_from_slice(echo.as_bytes());
            }
            Ok(data.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }

        fn in_waiting(&mut self) -> io::Result<usize> {
            Ok(self.reply.len())
        }
    }

    /// A press cut off before its line end stays in the build's line
    /// buffer; the next press ends it as a refused line, not as a press.
    #[test]
    fn a_partial_press_left_in_the_build_is_refused_not_pressed() {
        for partial in [
            &b"press dispense"[..],
            b"press dispense ",
            b"press dispense hold 15",
            b"press",
            b"",
        ] {
            let mut panel = Panel {
                reader: wellfed_core::virtual_panel::Reader::new(),
                pressed: Vec::new(),
                reply: Vec::new(),
            };
            for &byte in partial {
                assert_eq!(panel.reader.byte(byte), None);
            }

            let result = press(&mut panel, DISPENSE).unwrap();
            assert!(result.echoed && result.released);
            assert_eq!(
                panel.pressed,
                [DISPENSE],
                "{}",
                String::from_utf8_lossy(partial)
            );
        }
    }

    /// Another press echoed before this one's is a mismatch, even when this
    /// one's exact echo follows: the build pressed something unasked.
    #[test]
    fn an_earlier_press_echo_is_a_mismatch() {
        assert_eq!(
            Echo::read(
                b"PLKEY1 virtual press=dispense hold_ms=1500\r\n\
                  PLKEY1 virtual press=dispense hold_ms=150\r\n\
                  PLKEY1 up chord=dispense level=0 held_ms=150 dropped=0\r\n",
                DISPENSE
            ),
            Echo::Other {
                echoed: "PLKEY1 virtual press=dispense hold_ms=1500".into()
            }
        );
    }

    #[test]
    fn a_stray_line_refused_before_the_press_does_not_hide_it() {
        let mut feeder = Feeder::new(
            b"PLKEY1 virtual refused\r\nPLKEY1 virtual press=dispense hold_ms=150\r\n\
              PLKEY1 up chord=dispense level=0 held_ms=150 dropped=0\r\n",
        );
        assert!(press(&mut feeder, DISPENSE).unwrap().released);
    }

    /// Each wrong answer waits out the whole press window, so the cases
    /// run side by side, each against a feeder of its own.
    #[test]
    fn each_wrong_answer_is_its_own_refusal_and_is_never_resent() {
        let cases = [
            (&b""[..], "unanswered"),
            (b"PLKEY1 virtual refused\r\n", "refused"),
            (
                b"PLKEY1 virtual press=dispense hold_ms=1500\r\n",
                "mismatch",
            ),
            (
                b"PLKEY1 virtual press=dispense hold_ms=150\r\n",
                "not released",
            ),
            (
                b"PLKEY1 virtual refused\r\nPLKEY1 virtual press=mic hold_ms=150\r\n",
                "mismatch",
            ),
        ];

        std::thread::scope(|scope| {
            let cases: Vec<_> = cases
                .into_iter()
                .map(|(answer, refusal)| {
                    let pressed = scope.spawn(move || {
                        let mut feeder = Feeder::new(answer);
                        let error = press(&mut feeder, DISPENSE).unwrap_err();
                        (error, feeder.sent)
                    });
                    (pressed, refusal)
                })
                .collect();

            for (pressed, refusal) in cases {
                let (error, sent) = pressed.join().unwrap();
                let matched = match &error {
                    crate::Error::Protocol(ProtocolError::PressUnanswered { .. }) => "unanswered",
                    crate::Error::Protocol(ProtocolError::PressRefused) => "refused",
                    crate::Error::Protocol(ProtocolError::PressMismatch { .. }) => "mismatch",
                    crate::Error::Protocol(ProtocolError::PressNotReleased) => "not released",
                    _ => "other",
                };
                assert_eq!(matched, refusal, "{error}");
                assert_eq!(sent, LINE, "sent once");
            }
        });
    }
}
