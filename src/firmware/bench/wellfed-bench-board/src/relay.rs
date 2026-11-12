//! CDC#1: the SH-UR04A relay protocol plus telemetry queries.
//!
//! One reply per command line, nothing unprompted: `wellfed-relay` flushes
//! its input, sends a line and reads exactly one reply line within 1 s. A
//! switch while a RECOVER sequence holds the relays is answered `ERROR` at
//! once, so a command the host saw refused never runs later.

use embassy_time::{Duration, Timer};
use embassy_usb::class::cdc_acm::Sender;
use wellfed_bench_at::{Command, Line, LineBuffer, Relays};

use crate::usb::{Cdc, PACKET, UsbDriver};
use crate::{outputs, telemetry};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[embassy_executor::task]
pub(crate) async fn run(class: Cdc) -> ! {
    let (mut sender, mut receiver) = class.split();
    let mut lines = LineBuffer::default();
    let mut buf = [0u8; PACKET as usize];
    loop {
        receiver.wait_connection().await;
        let Ok(n) = receiver.read_packet(&mut buf).await else {
            // Bus reset or unplug: drop any half line.
            lines = LineBuffer::default();
            continue;
        };
        for &byte in &buf[..n] {
            match lines.push(byte) {
                None => {}
                Some(Line::TooLong) => send(&mut sender, wellfed_bench_at::ERROR).await,
                Some(Line::Complete(line)) => execute(&mut sender, &line).await,
            }
        }
    }
}

async fn execute(sender: &mut Sender<'static, UsbDriver>, line: &[u8]) {
    let Some(command) = wellfed_bench_at::parse(line) else {
        send(sender, wellfed_bench_at::ERROR).await;
        return;
    };

    // A switch takes the relays now or is refused: the host gives up after
    // 1 s, and a RECOVER sequence holds them for seconds. A query reads the
    // last state and never waits.
    let channels = if command.switches() {
        let guard = outputs::try_lock();
        let held = if guard.is_some() {
            Relays::Free
        } else {
            Relays::Held
        };
        let Some(mut relays) = guard.filter(|_| !wellfed_bench_at::refused(command, held)) else {
            send(sender, wellfed_bench_at::ERROR).await;
            return;
        };
        if let Command::Set(channel, on) = command {
            relays.set(channel, on);
        }
        relays.state()
    } else {
        outputs::snapshot()
    };
    let reply = wellfed_bench_at::reply(
        command,
        channels,
        outputs::cuts(),
        &telemetry::latest(),
        VERSION,
    );
    send(sender, &reply).await;

    if command == Command::Bootsel {
        // Let the host read the OK before the device drops off the bus.
        Timer::after(Duration::from_millis(100)).await;
        outputs::lock().await.cut_power();
        embassy_rp::rom_data::reset_to_usb_boot(0, 0);
    }
}

async fn send(sender: &mut Sender<'static, UsbDriver>, text: &str) {
    let mut out = heapless::Vec::<u8, { wellfed_bench_at::REPLY_MAX + 2 }>::new();
    // REPLY_MAX bounds every reply, so the CRLF always fits.
    let _ = out.extend_from_slice(text.as_bytes());
    let _ = out.extend_from_slice(b"\r\n");
    for chunk in out.chunks(usize::from(PACKET)) {
        if sender.write_packet(chunk).await.is_err() {
            return;
        }
    }

    // A full packet does not end a bulk transfer: Linux's cdc-acm holds it
    // until a short one arrives, so a reply of exactly 64 bytes (`AT+STAT?`
    // with a four-digit VBUS) would wait for the next command's reply.
    if out.len().is_multiple_of(usize::from(PACKET)) {
        let _ = sender.write_packet(&[]).await;
    }
}
