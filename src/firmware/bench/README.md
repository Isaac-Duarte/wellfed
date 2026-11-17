# Wellfed Bench firmware

This is the RP2040 firmware for the [Wellfed Bench board](../../hardware/bench/README.md). Over USB the board enumerates as a serial port that bridges the feeder's UART, and a control interface that `wellfed-bench` drives over [postcard-rpc](https://github.com/jamesmunns/postcard-rpc). This page covers building and flashing the firmware, the control interface, and the checks to run on the first boards.

> [!NOTE]
> The firmware has run on one Rev B board (November 2, 2026). Checklist steps 1–4 below passed with nothing attached to the feeder side. That run found replies of exactly 64 bytes held back by the host until the next packet, now ended with a zero-length packet on both ports. The same day, with a stock feeder attached, steps 5–7 passed apart from the D-cell refusal, and `wellfed-flash` read the whole flash through the bridge. Those runs drove the board's former `AT` relay protocol. The postcard-rpc control interface that replaced it has not run on a board yet. How to wire a feeder to it, TX to TX, is on the [bench hardware](../../../docs/development/bench-hardware.md) page.

| USB interface | Class | Job |
| --- | --- | --- |
| 0 and 1, by-id suffix `-if00` | CDC-ACM serial port | Feeder UART bridge (115200 8N1 to the feeder's UART header) |
| 2 | Vendor (`0xFF`), one bulk endpoint each way | The [control interface](#control-interface): switched outputs, telemetry and BOOTSEL |

## Crates

| Crate | What it holds |
| --- | --- |
| [`wellfed-bench-icd`](wellfed-bench-icd) | The control interface's endpoints and message types, which the firmware and `wellfed-relay` both build from, plus the telemetry scaling and the RECOVER press classifier. Pure `no_std` code in the main workspace, so `make check` tests it on the host |
| [`wellfed-bench-board`](wellfed-bench-board) | The embassy firmware that owns the pins, USB and the button. It builds only for `thumbv6m-none-eabi`, so it is its own workspace with its own lockfile and [dependency policy](wellfed-bench-board/deny.toml) |

## Build

```sh
make check-bench-board
```

That runs Clippy and a release build for the RP2040. It touches no hardware. The `thumbv6m-none-eabi` target comes with the repository's pinned toolchain. To build the UF2 image by hand, you also need `cargo install elf2uf2-rs --locked`:

```sh
cd src/firmware/bench/wellfed-bench-board
cargo build --locked --release
elf2uf2-rs target/thumbv6m-none-eabi/release/wellfed-bench-board wellfed-bench-board.uf2
```

## Flash

**First time, or if the board does not answer:**

1. Hold **BOOT** while you plug in USB, or hold BOOT and tap RESET. An `RPI-RP2` drive appears.
2. Copy the `.uf2` onto that drive. With the board in this mode, `cargo run --locked --release` builds and copies in one step.

**Updates:** stop `wellfed-bench serve`, which holds the board, then run `wellfed-bench bootsel`. The board cuts feeder power and reboots into the UF2 drive. Holding BOOT and tapping RESET does the same.

A board still on the `AT` firmware has no control interface. Update it once with BOOT and RESET, or send `AT+BOOTSEL` on its second serial port:

```sh
printf 'AT+BOOTSEL\r\n' > /dev/serial/by-id/usb-Wellfed_Wellfed_Bench_<serial>-if02
```

## Use it with the bench

1. Find the board's serial and its UART port. The serial is the flash chip's unique ID, 16 hex digits, so it stays the same for each board.

   On Linux, run `ls /dev/serial/by-id/ | grep Wellfed`. You should see `usb-Wellfed_Wellfed_Bench_<serial>-if00`, which names the serial and is the UART port.

   On macOS, run `ioreg -p IOUSB -l -n "Wellfed Bench" | grep '"USB Serial Number"'` for the serial, and `ls /dev/cu.usbmodem*` for the port. macOS names the port after the USB location, so a board in one socket shows `cu.usbmodem2101`. The name changes with the socket or hub, so recheck it after moving the cable. Use the `cu.` path, not `tty.`, which waits for carrier detect.
2. Point `private/bench.json` at them:

   ```json
   {
     "board_serial": "<serial>",
     "uart_port": "/dev/serial/by-id/usb-Wellfed_Wellfed_Bench_<serial>-if00",
     "baseline": "private/flash-backups/<full-read>/flash-read.bin",
     "rom_report": "private/captures/<rom-inspection>/report.json"
   }
   ```

   `baseline` is this feeder's full flash read and `rom_report` its ROM inspection. A new feeder gets both by hand first: `wellfed-flash read inspect-rom` while stock runs (it fails if a stock log line lands inside a reply, so run it while the feeder is quiet), then RECOVER's long press into the loader, `wellfed-flash read small --device plaf103` and `read full --device plaf103`. Those two refuse a ROM report of another model's console.

3. On Linux your user needs two kinds of access. Install [`70-wellfed-bench.rules`](70-wellfed-bench.rules) for the control interface:

   ```sh
   sudo cp src/firmware/bench/70-wellfed-bench.rules /etc/udev/rules.d/
   sudo udevadm control --reload && sudo udevadm trigger
   ```

   The UART port needs the serial group: `uucp` on Arch, `dialout` on Debian and Ubuntu. macOS needs neither.
4. Run `wellfed-bench status` with no server. You should see all four relays off, the board's telemetry, and `board_serial: present`.

## Control interface

The board answers [postcard-rpc](https://github.com/jamesmunns/postcard-rpc) requests on interface 2. The host finds it by VID:PID `1209:0001`, the manufacturer string `Wellfed` and the serial string. Opening it claims the interface, so only one process drives the board at a time. The endpoints and their types live in [`wellfed-bench-icd`](wellfed-bench-icd/src/lib.rs):

| Endpoint | Request | Reply |
| --- | --- | --- |
| `bench/set` | A channel and on or off | The four channels after the set, or `Held` |
| `bench/status` | None | The four channels, the power-cut count and the telemetry |
| `bench/bootsel` | None | `Ok` or `Held`. The board cuts feeder power, answers, and 100 ms later reboots into the UF2 bootloader |

| Channel | Hardware |
| --- | --- |
| 1 | Feeder 5 V on or off (AP2171W, about 1.1 A limit) |
| 2 | Strap: feeder TX to GND through 4.7 kΩ (ROM loader entry) |
| 3 | Bridge TX → feeder RX buffer enabled |
| 4 | Feeder TX → bridge RX buffer enabled |

| Status field | Meaning |
| --- | --- |
| `vbus_mv` | Feeder 5 V, in mV |
| `current_ma` | Feeder current, in mA (INA181 on 50 mΩ) |
| `source` | Which USB-C input feeds the feeder, `Host` or `Aux` |
| `fault` | Overcurrent flag |
| `tx_mv` | Feeder TX line level, in mV. 2000 mV or more counts as alive: the feeder MCU is powered. Alive with channel 1 off means the D-cells are still in |
| `cuts` | Feeder power cuts the board has made, from a request or RECOVER, counted from a random start at each board boot. `wellfed-bench` records it when it leaves the feeder in the loader, and refuses to write if it moved |

The board also answers postcard-rpc's standard endpoints, such as the schema report. A request built from another version of `wellfed-bench-icd` gets an `UnknownKey` error, not a wrong answer.

At power-up, and after any reset including the 2 s watchdog, all four channels are off. The board's pull resistors hold the same state while the RP2040 is in reset.

## Feeder UART bridge

- The default is 115200 8N1. The baud rate follows the host's setting; data bits, parity and stop bits are always 8N1.
- The bridge never resets or gates on DTR or RTS. `wellfed-flash` opens and closes the port many times in one session while the feeder keeps running.
- When the host opens the port, UART bytes that queued up while nobody was reading are dropped, apart from at most one 64-byte USB packet already handed to the controller. The flash writer refuses unexpected bytes, so a stale packet fails a step rather than corrupt it.

## LEDs and buttons

| | Meaning |
| --- | --- |
| **3V3** | Board powered (hardware) |
| **STAT** | Fast blink: no USB host. A short blip every second: ready. Solid: strap held. Triple blinks: RECOVER refused |
| **UART** | Bridge traffic |
| **FDR** | Feeder 5 V present (hardware) |
| **RECOVER, hold 1 s** | Power-cycles the feeder into its ROM loader: power off 2 s, strap on, power on, hold 2 s, strap off. If the feeder is still alive with its power cut (batteries in), it stays off and STAT triple-blinks |
| **RECOVER, short press** | A plain feeder power cycle |
| **RESET / BOOT** | RP2040 reset and BOOTSEL |

While a RECOVER sequence runs, a request that switches an output (`bench/set`, `bench/bootsel`) is answered `Held` at once and never runs later. A status request answers with the current state.

## Pin map

| GPIO | Net | Notes |
| --- | --- | --- |
| 0 / 1 | ADP_TXD / ADP_RXD | UART0 through the 74LVC2G125; RX has an internal pull-up |
| 2 | FDR_PWR_EN | CH1, active high |
| 3 | FDR_PWR_FLT_N | Overcurrent flag, active low |
| 4 | STRAP_EN | CH2, active high |
| 5 / 6 | UART_TX_OE_N / UART_RX_OE_N | CH3 / CH4, active low |
| 7 | AUX_ACTIVE | TPS2116 ST |
| 8 | RECOVER_N | SW3 |
| 16 / 17 | LED_STATUS / LED_UART | |
| 26 / 27 / 28 | VBUS_FDR_SENSE / FDR_ISENSE / FDR_TX_SENSE | ADC0–2 |
| 9–15, 18–23, 29 | SWD probe header, expansion | Unused, left as inputs |

The code uses the first 2 MiB of the 16 MiB flash. The rest is reserved for stored feeder images.

## Bring-up checklist

Run these on the first boards, in order:

1. The UF2 flashes, `ls /dev/serial/by-id/` shows `-if00`, and `lsusb -v -d 1209:0001` lists a vendor-class interface 2.
2. `wellfed-bench status` with no server and nothing attached: all relays off, VBUS about 0, not alive. Run it a few times. Nothing loads the feeder TX net and the firmware reads it with no pull, so a floating line can read high, and a high reading refuses RECOVER as if the D-cells were in.
3. Start `wellfed-bench serve` and run `wellfed-bench on`: the FDR LED lights. Stop the server and run `wellfed-bench status`: VBUS reads about 5000 mV. Check it against a meter.
4. Plug in the aux USB-C: `status` shows the source as `Aux`. If it shows `Host`, the ST polarity is inverted; flip it in `wellfed-bench-board/src/telemetry.rs`.
5. With a feeder connected and powered, `status` shows it alive, and its current matches a USB meter.
6. With the server running, run `wellfed-bench status`, `on`, `off` and `loader`. The loader must see the `#Flash Download Start` banner.
7. RECOVER long press with the D-cells in: refused (triple blink). With them out: the feeder enters the loader.

> [!WARNING]
> Steps 3 and later power a feeder. Use the reviewed bench procedure for the feeder you connect.

## Before boards ship

- Replace the pid.codes test VID:PID `1209:0001` in `wellfed-bench-board/src/usb.rs` with an allocated PID, from pid.codes (open hardware) or a Raspberry Pi `2E8A` sub-PID.
- Standalone recovery from the board's own flash (upload the helper, then a sector-by-sector bank write and marker commit) waits until the bridge is proven on real hardware.
