# Wellfed Bench firmware

This is the RP2040 firmware for the [Wellfed Bench board](../../hardware/bench/README.md). Over USB the board enumerates as two serial ports that stand in for the relay bench's FT232R UART adapter and SH-UR04A relay board. This page covers building and flashing the firmware, the relay port's protocol, and the checks to run on the first boards.

> [!NOTE]
> The firmware has run on one Rev B board (November 2, 2026). Checklist steps 1–4 below passed with nothing attached to the feeder side. That run found replies of exactly 64 bytes held back by the host until the next packet, now ended with a zero-length packet on both ports. The same day, with a stock feeder attached, steps 5–7 passed apart from the D-cell refusal, and `wellfed-flash` read the whole flash through the bridge. The bench tools drive it as the relay board. How to wire a feeder to it, TX to TX, is on the [bench hardware](../../../docs/development/bench-hardware.md) page.

| USB interface | by-id suffix | Replaces | Job |
| --- | --- | --- | --- |
| 0 | `-if00` | FT232R | Feeder UART bridge (115200 8N1 to the feeder's UART header) |
| 2 | `-if02` | SH-UR04A | `AT+CHn` relay protocol plus telemetry |

## Crates

| Crate | What it holds |
| --- | --- |
| [`wellfed-bench-at`](wellfed-bench-at) | The relay port's protocol: line framing, the `AT` parser, the replies and the telemetry scaling. Pure `no_std` code in the main workspace, so `make check` tests it on the host |
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

**Updates:** send `AT+BOOTSEL` on the relay port. The board cuts feeder power and reboots into the UF2 drive:

```sh
printf 'AT+BOOTSEL\r\n' > /dev/serial/by-id/usb-Wellfed_Wellfed_Bench_<serial>-if02
```

## Use it with the bench

1. Find the ports. On Linux, `ls /dev/serial/by-id/ | grep Wellfed`. On macOS, `ls /dev/cu.usbmodem*`: macOS names each port after the USB location and the interface number plus one, so a board in one socket shows `cu.usbmodem2101` (the UART bridge) and `cu.usbmodem2103` (the relay port). The names change with the socket or hub, so recheck them after moving the cable. Use the `cu.` path, not `tty.`, which waits for carrier detect. On Linux the by-id names carry the serial, the flash chip's unique ID, so each board keeps stable paths there.
2. Point `private/bench.json` at them:

   ```json
   {
     "relay_port": "/dev/serial/by-id/usb-Wellfed_Wellfed_Bench_<serial>-if02",
     "uart_port": "/dev/serial/by-id/usb-Wellfed_Wellfed_Bench_<serial>-if00",
     "baseline": "private/flash-backups/<full-read>/flash-read.bin",
     "rom_report": "private/captures/<rom-inspection>/report.json"
   }
   ```

   `baseline` is this feeder's full flash read and `rom_report` its ROM inspection. A new feeder gets both by hand first: `wellfed-flash read inspect-rom` while stock runs (it fails if a stock log line lands inside a reply, so run it while the feeder is quiet), then RECOVER's long press into the loader, `wellfed-flash read small --device plaf103` and `read full --device plaf103`. Those two refuse a ROM report of another model's console.

3. On Linux your user needs serial access: the `uucp` group on Arch, `dialout` on Debian and Ubuntu. macOS needs no group.

## Relay port

Send one line ending in CR, LF or CRLF, and you get exactly one line back ending in CRLF. Commands are case-insensitive. A line that does not parse, or is longer than 32 bytes, gets `ERROR`.

**SH-UR04A compatible.** These replies are what `wellfed-relay` checks, byte for byte:

| Command | Reply | Hardware |
| --- | --- | --- |
| `AT` | `OK` | |
| `AT+CH1=1` / `=0` / `=?` | `OK+CH1=1` / `0` | Feeder 5 V on or off (AP2171W, about 1.1 A limit) |
| `AT+CH2=…` | `OK+CH2=…` | Strap: feeder TX to GND through 4.7 kΩ (ROM loader entry) |
| `AT+CH3=…` | `OK+CH3=…` | Bridge TX → feeder RX buffer enabled |
| `AT+CH4=…` | `OK+CH4=…` | Feeder TX → bridge RX buffer enabled |

**Board additions.** The bench tools never send these:

| Command | Reply | Meaning |
| --- | --- | --- |
| `AT+VER?` | `OK+VER=0.1.0 revB` | Firmware version and board revision |
| `AT+VBUS?` | `OK+VBUS=5012` | Feeder 5 V, in mV |
| `AT+CUR?` | `OK+CUR=180` | Feeder current, in mA (INA181 on 50 mΩ) |
| `AT+SRC?` | `OK+SRC=HOST` / `AUX` | Which USB-C input feeds the feeder |
| `AT+FLT?` | `OK+FLT=0` / `1` | Overcurrent flag |
| `AT+TXV?` | `OK+TXV=3290` | Feeder TX line level, in mV |
| `AT+ALIVE?` | `OK+ALIVE=1` | The feeder MCU is powered (its TX idles high). `1` with CH1 off means the D-cells are still in |
| `AT+STAT?` | `OK+STAT=CH:1011;VBUS:…;CUR:…;SRC:…;FLT:…;TXV:…;ALIVE:…` | Everything above at once |
| `AT+CUTS?` | `OK+CUTS=2914730551` | Feeder power cuts the board has made, from either port or RECOVER, counted from a random start at each board boot. `wellfed-bench` records it when it leaves the feeder in the loader and refuses to write if it moved, and enters no session on a board whose firmware does not answer it |
| `AT+BOOTSEL` | `OK` | Cuts feeder power, then reboots into the UF2 bootloader |

At power-up, and after any reset including the 2 s watchdog, all four channels are off. That matches an unpowered relay board. The board's pull resistors hold the same state while the RP2040 is in reset.

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

While a RECOVER sequence runs, a host command that switches a relay (`AT+CHn=0|1`, `AT+BOOTSEL`) is answered `ERROR` at once and never runs later; queries answer with the current state.

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

1. The UF2 flashes, and `ls /dev/serial/by-id/` shows `-if00` and `-if02`.
2. `AT+STAT?` with nothing attached: all channels 0, VBUS about 0, ALIVE 0. Read `AT+TXV?` a few times too. Nothing loads the feeder TX net and the firmware reads it with no pull, so a floating line can read high, and a high reading refuses RECOVER as if the D-cells were in.
3. `AT+CH1=1`: the FDR LED lights and `AT+VBUS?` reads about 5000. Check it against a meter.
4. Plug in the aux USB-C: `AT+SRC?` reads `AUX`. If it reads `HOST`, the ST polarity is inverted; flip it in `wellfed-bench-board/src/telemetry.rs`.
5. With a feeder connected and powered, `AT+ALIVE?` is 1 and `AT+CUR?` matches a USB meter.
6. Point `private/bench.json` at the board, then run `wellfed-bench status`, `on`, `off` and `loader`. The loader must see the `#Flash Download Start` banner.
7. RECOVER long press with the D-cells in: refused (triple blink). With them out: the feeder enters the loader.

> [!WARNING]
> Steps 3 and later power a feeder. Use the reviewed bench procedure for the feeder you connect.

## Before boards ship

- Replace the pid.codes test VID:PID `1209:0001` in `wellfed-bench-board/src/usb.rs` with an allocated PID, from pid.codes (open hardware) or a Raspberry Pi `2E8A` sub-PID.
- Standalone recovery from the board's own flash (upload the helper, then a sector-by-sector bank write and marker commit) waits until the bridge is proven on real hardware.
