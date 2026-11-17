//! Wellfed Bench rev B: RP2040 firmware for the feeder recovery board.
//!
//! The board enumerates as a CDC-ACM port that bridges the feeder UART and a
//! vendor interface that answers the bench's postcard-rpc control requests
//! (`wellfed-bench-icd`).

#![no_std]
#![no_main]

mod bridge;
mod outputs;
mod rpc;
mod telemetry;
mod ui;
mod usb;
mod wire;

use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::peripherals::{UART0, USB};
use embassy_rp::uart::{BufferedInterruptHandler, BufferedUart};
use panic_reset as _;
use static_cell::StaticCell;

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => embassy_rp::usb::InterruptHandler<USB>;
    UART0_IRQ => BufferedInterruptHandler<UART0>;
    ADC_IRQ_FIFO => embassy_rp::adc::InterruptHandler;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // Feeder off, strap released, UART buffers isolated: the same state the
    // board's pull resistors hold during reset, and what a de-energised relay
    // board looked like.
    outputs::init(
        outputs::Outputs {
            power: Output::new(p.PIN_2, Level::Low),
            strap: Output::new(p.PIN_4, Level::Low),
            tx_oe_n: Output::new(p.PIN_5, Level::High),
            rx_oe_n: Output::new(p.PIN_6, Level::High),
        },
        embassy_rp::clocks::RoscRng.next_u32(),
    )
    .await;

    // Read before USB starts: the unique-ID command briefly stops XIP.
    let serial = usb::serial_number(p.FLASH);

    static UART_TX: StaticCell<[u8; 1024]> = StaticCell::new();
    static UART_RX: StaticCell<[u8; 4096]> = StaticCell::new();
    let uart = BufferedUart::new(
        p.UART0,
        p.PIN_0,
        p.PIN_1,
        Irqs,
        UART_TX.init([0; 1024]),
        UART_RX.init([0; 4096]),
        {
            let mut config = embassy_rp::uart::Config::default();
            config.baudrate = bridge::START_BAUD;
            config
        },
    );
    // ADP_RXD floats while the feeder-TX buffer is disabled; hold it idle-high
    // so the UART does not see framing noise.
    embassy_rp::pac::PADS_BANK0.gpio(1).modify(|w| {
        w.set_pde(false);
        w.set_pue(true);
    });

    let (device, uart_class, ep_in, ep_out) = usb::build(p.USB, Irqs, serial);

    let sensors = telemetry::Sensors {
        adc: embassy_rp::adc::Adc::new(p.ADC, Irqs, Default::default()),
        vbus: embassy_rp::adc::Channel::new_pin(p.PIN_26, Pull::None),
        current: embassy_rp::adc::Channel::new_pin(p.PIN_27, Pull::None),
        tx: embassy_rp::adc::Channel::new_pin(p.PIN_28, Pull::None),
        fault_n: Input::new(p.PIN_3, Pull::None),
        aux: Input::new(p.PIN_7, Pull::None),
    };

    let panel = ui::Panel {
        status_led: Output::new(p.PIN_16, Level::Low),
        uart_led: Output::new(p.PIN_17, Level::Low),
        recover_n: Input::new(p.PIN_8, Pull::Up),
        watchdog: embassy_rp::watchdog::Watchdog::new(p.WATCHDOG),
    };

    // Each task has one pool slot and is spawned exactly once, here.
    const ONCE: &str = "each task is spawned once";
    spawner.spawn(usb::run(device).expect(ONCE));
    spawner.spawn(bridge::run(uart, uart_class).expect(ONCE));
    spawner.spawn(rpc::run(ep_in, ep_out).expect(ONCE));
    spawner.spawn(rpc::reboot().expect(ONCE));
    spawner.spawn(telemetry::run(sensors).expect(ONCE));
    spawner.spawn(ui::run(panel).expect(ONCE));
    spawner.spawn(ui::recover().expect(ONCE));
}
