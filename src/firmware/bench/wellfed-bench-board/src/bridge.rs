//! Feeder UART bridge: UART0 (GPIO0/1, through the 74LVC2G125) to CDC#0.
//!
//! `wellfed-flash` opens and closes this port many times per session with
//! DTR/RTS forced low, while the feeder's loader session runs on. So the bridge
//! never resets or gates on DTR. When the host (re)opens the port it sends
//! SET_CONTROL_LINE_STATE and SET_LINE_CODING; on either we drop UART bytes
//! that queued up while nobody was reading and apply the requested baud
//! rate. A new process sees fresh traffic apart from at most one USB packet
//! (64 bytes) already handed to the controller, which embassy's driver
//! cannot take back; the flash writer refuses such bytes rather than act on
//! them.

use portable_atomic::{AtomicBool, AtomicU32, Ordering};

use embassy_futures::join::join;
use embassy_futures::select::{Either, select};
use embassy_rp::uart::{BufferedUart, BufferedUartRx, BufferedUartTx};
use embassy_usb::class::cdc_acm::{ControlChanged, Receiver, Sender};
use embedded_io_async::{Read, ReadReady, Write};

use crate::usb::{Cdc, PACKET, UsbDriver};

/// Set on every byte moved in either direction; the UI flashes the UART LED.
pub(crate) static TRAFFIC: AtomicBool = AtomicBool::new(false);

#[embassy_executor::task]
pub(crate) async fn run(mut uart: BufferedUart, class: Cdc) -> ! {
    let (mut sender, mut receiver, control) = class.split_with_control();
    let (tx, rx) = uart.split_ref();
    join(
        uart_to_usb(rx, &mut sender, &control),
        usb_to_uart(&mut receiver, tx),
    )
    .await;
    unreachable!()
}

async fn uart_to_usb(
    rx: &mut BufferedUartRx,
    sender: &mut Sender<'static, UsbDriver>,
    control: &ControlChanged<'static>,
) -> ! {
    let mut buf = [0u8; PACKET as usize];
    loop {
        sender.wait_connection().await;
        let n = match select(rx.read(&mut buf), control.control_changed()).await {
            Either::First(Ok(n)) => n,
            // Framing/overrun errors: the bytes are gone; keep going.
            Either::First(Err(_)) => continue,
            Either::Second(()) => {
                host_reopened(rx, sender.line_coding().data_rate()).await;
                continue;
            }
        };
        TRAFFIC.store(true, Ordering::Relaxed);
        // Blocks while the host is not reading (port closed). A control
        // change abandons the packet, since it is stale by then.
        match select(sender.write_packet(&buf[..n]), control.control_changed()).await {
            Either::First(_) => {}
            Either::Second(()) => {
                host_reopened(rx, sender.line_coding().data_rate()).await;
                continue;
            }
        }

        // A full packet does not end a bulk transfer: the host holds it until
        // a short one arrives. With nothing more queued, end the transfer now,
        // or a loader reply that fills the packet exactly would stall.
        if n == usize::from(PACKET) && !rx.read_ready().unwrap_or(false) {
            match select(sender.write_packet(&[]), control.control_changed()).await {
                Either::First(_) => {}
                Either::Second(()) => host_reopened(rx, sender.line_coding().data_rate()).await,
            }
        }
    }
}

async fn usb_to_uart(receiver: &mut Receiver<'static, UsbDriver>, tx: &mut BufferedUartTx) -> ! {
    let mut buf = [0u8; PACKET as usize];
    loop {
        receiver.wait_connection().await;
        let Ok(n) = receiver.read_packet(&mut buf).await else {
            continue;
        };
        if n > 0 {
            TRAFFIC.store(true, Ordering::Relaxed);
            // BufferedUartTx writes never fail.
            let _ = tx.write_all(&buf[..n]).await;
        }
    }
}

/// Drops queued UART input and applies the host's baud rate.
async fn host_reopened(rx: &mut BufferedUartRx, baud: u32) {
    let mut scratch = [0u8; 64];
    while rx.read_ready().unwrap_or(false) {
        if rx.read(&mut scratch).await.is_err() {
            break;
        }
    }
    set_baud(baud);
}

/// The feeder's rate, which the UART starts at.
pub(crate) const START_BAUD: u32 = 115_200;

/// The rate the UART runs at, as `set_baud` last applied it.
static APPLIED_BAUD: AtomicU32 = AtomicU32::new(START_BAUD);

/// Only the rate is honoured: the feeder link is always 8N1, and
/// `BufferedUart` cannot change framing after a split. The PL011's divisor
/// and LCR_H must not change mid-character, and every open and close asks
/// again for the same rate, so an unchanged rate writes nothing.
fn set_baud(baud: u32) {
    // Linux sends 9600 or 0 before the application's termios; ignore
    // nonsense and keep the current divisor.
    if !(300..=3_000_000).contains(&baud) || APPLIED_BAUD.swap(baud, Ordering::Relaxed) == baud {
        return;
    }
    let clk = embassy_rp::clocks::clk_peri_freq();
    let div = 8 * clk / baud;
    let (ibrd, fbrd) = match div >> 7 {
        0 => (1, 0),
        i if i >= 65535 => (65535, 0),
        i => (i, (div & 0x7f).div_ceil(2)),
    };
    let uart = embassy_rp::pac::UART0;
    uart.uartibrd()
        .write_value(embassy_rp::pac::uart::regs::Uartibrd(ibrd));
    uart.uartfbrd()
        .write_value(embassy_rp::pac::uart::regs::Uartfbrd(fbrd));
    // The PL011 latches the divisor on a write to LCR_H.
    uart.uartlcr_h().modify(|_| {});
}
