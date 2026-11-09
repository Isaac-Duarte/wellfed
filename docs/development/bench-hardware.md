# Bench hardware

This is the bench's physical reference, for contributors and agents who touch it: what plugs into what, which pin is which, how to check the wiring, and the order of a flash session. The commands are on the [relay bench](../devices/plaf103/bench.md) page and the board's design on the [Wellfed Bench board](../../src/hardware/bench/README.md) page. Everything here beyond reading is a physical operation on the device under test.

## Wire TX to TX

> [!NOTE]
> The Wellfed Bench board labels its **FEEDER UART** header with the feeder's pin names. Wire label to label: feeder **TX** to the board's **TX**, **RX** to **RX**, **GND** to **GND**. The board crosses the lines inside itself. Do not cross them again in the cable.

The usual "TX goes to RX" applies only to a bare USB serial adapter, whose labels name its own pins. The bench no longer uses one, but older pages and captures describe it.

| You connect the feeder to | Feeder TX goes to | Feeder RX goes to | Feeder GND goes to |
| --- | --- | --- | --- |
| Wellfed Bench board, FEEDER UART (J2) | **TX** (pin 2) | **RX** (pin 1) | **GND** (pin 3) |
| A bare 3.3 V USB serial adapter (the old FT232R) | RXD | TXD | GND |

Nothing ever connects to a feeder's 3V3 or VCC pin. The feeder runs from its own supply.

## What is on the bench

| Part | Job |
| --- | --- |
| A host computer | Runs `wellfed-bench serve` and every other tool |
| Wellfed Bench board, Rev B | One USB cable to the host. Switches feeder 5 V, holds the loader strap, bridges the feeder's UART and isolates it while the feeder is off |
| The feeder (PLAF103) or the fountain (PLWF106) | The device under test, one at a time |

The board replaced an SH-UR04A USB relay board and an FT232R adapter, and speaks the relay board's `AT+CHn` protocol, so `wellfed-bench` drives it the same way. The [relay bench](../devices/plaf103/bench.md#wiring) page keeps the relay board's wiring for comparison.

## The board's connectors

| Silkscreen | Ref | Connector | Connects to |
| --- | --- | --- | --- |
| TO PC | J1 | USB-C | The host. Data and the board's power |
| AUX 5V | J5 | USB-C | An optional second 5 V supply. The board switches the feeder to it when the host port cannot supply enough. Power only |
| FEEDER 5V | J4 | USB-A | The feeder's USB power cable. It carries 5 V and GND only: its D+ and D− are not connected |
| FEEDER UART | J2 | JST PH, 3 pins, 2.0 mm | The feeder's console header ([below](#feeder-uart-j2)) |
| EXPANSION | J6 | 2×8, 2.54 mm | Unused by the firmware |
| SWD | J7 | 2×5, 1.27 mm | A debug probe for the RP2040 |
| RESET, BOOT, RECOVER | SW1–SW3 | Buttons | RP2040 reset, its UF2 bootloader, and a feeder power cycle or loader entry ([LEDs and buttons](../../src/firmware/bench/README.md#leds-and-buttons)) |
| 3V3, STAT, UART, FDR | D1, D3, D4, D2 | LEDs | Board powered, firmware state, UART traffic, feeder 5 V on |

### FEEDER UART (J2)

Each pin's silkscreen label and net name is the feeder's signal on that pin.

| Pin | Silkscreen | Net | Feeder pin | Path inside the board |
| --- | --- | --- | --- | --- |
| 1, square pad | RX | `FDR_RX` | Feeder RX | RP2040 GPIO0 (UART0 TX) → buffer, enabled by CH3 → J2 pin 1 |
| 2 | TX | `FDR_TX` | Feeder TX | J2 pin 2 → buffer, enabled by CH4 → RP2040 GPIO1 (UART0 RX). Also the loader strap and the TX level sense |
| 3 | GND | `GND` | Feeder GND | Board ground |

- **The strap is on feeder TX** because the chip's download strap is its log UART TX, PA7 ([chip page](../chips/rtl8721dm/README.md#boot-and-download-mode)). CH2 pulls `FDR_TX` to ground through 4.7 kΩ while the feeder powers on.
- **The TX sense reads the wire, not the buffer.** GPIO28 reads `FDR_TX` on the header side, so `AT+TXV?` and `AT+ALIVE?` show whether feeder TX reaches pin 2 even while CH4 is off.
- **The header carries no power.** Feeder power goes only through FEEDER 5V.
- **Both directions are buffered** by a 74LVC2G125, so a board left powered cannot back-power a feeder that is off through its UART pins. Each line also has ESD protection.

## Wire a feeder

> [!WARNING]
> Change wiring only with every power source removed: unplug FEEDER 5V and take the D cells out. Leave every 3V3 or VCC pin on the feeder unconnected.

### PLAF103

The console header `CON5` is labelled **RX, TX, GND** ([UART page](../devices/plaf103/uart.md#the-console-header)).

| CON5 pad | Label | Bench J2 |
| --- | --- | --- |
| Square pad | RX | Pin 1, RX |
| Middle round pad | TX | Pin 2, TX |
| Round pad nearest the mounting screw | GND | Pin 3, GND |

Pin 1 is RX on both ends, so a three-wire cable runs straight across, pin 1 to pin 1. Pad positions can differ between units, so go by the silkscreen.

Plug the feeder's USB power cable into FEEDER 5V. Keep the D cells out, so FEEDER 5V is the only supply and cutting it is the kill switch. With the cells in, the board's RECOVER refuses and the bench cannot power the feeder off.

### PLWF106

The fountain's header is labelled **GND, TX, RX, 3V3** ([UART page](../devices/plwf106/uart.md)): another order, and a fourth pin, so wire by label, not position:

| Fountain | Bench J2 |
| --- | --- |
| GND | Pin 3, GND |
| TX | Pin 2, TX |
| RX | Pin 1, RX |
| 3V3 | Not connected |

Run the fountain's 5 V supply through FEEDER 5V, so the bench can switch it.

## Find the ports

The board shows up as two USB serial ports. `private/bench.json` names both.

| USB interface | Port | `bench.json` key | Answers `AT` |
| --- | --- | --- | --- |
| 0 | The feeder UART bridge, `…-if00` | `uart_port` | No |
| 2 | The relay port, `…-if02` | `relay_port` | Yes, with `OK` |

Name each port by a path that survives a replug, such as `/dev/serial/by-id/usb-Wellfed_Wellfed_Bench_<serial>-if00`. Unsure which is which? Send `AT`: only the relay port answers. The [bench firmware](../../src/firmware/bench/README.md#use-it-with-the-bench) page has the full `bench.json` and the port names on each host.

## Check the wiring

Check these before blaming the firmware or the tools. Steps 1 and 2 run with no server; steps 3 and 4 run as jobs of `wellfed-bench serve`, which owns both ports.

1. Run `wellfed-bench status` with no server.

   You should see the lease free, all four channels off and both ports present.

2. Open the relay port in a serial terminal and send `AT+CH1=1`, then `AT+STAT?`, then `AT+CH1=0`.

   You should see `ALIVE:1` and `TXV` near 3290 mV in the `AT+STAT?` reply, proving feeder TX reaches pin 2 and the grounds are shared. CH3 and CH4 stay off; this check does not need them.

3. Start the server, then run `wellfed-bench boot NAME`.

   You should see stock's or Wellfed's boot log in the capture, proving the feeder-to-host direction through the CH4 buffer.

4. Run `wellfed-bench loader NAME`.

   You should see `#Flash Download Start` and a run of NAK bytes, proving the strap reaches feeder TX. The loader sends unasked, so a host-to-feeder fault does not show here; it shows only when the feeder must answer: `rom-inspect`, and the helper upload at the start of `dump` and every prepare.

| Symptom | Most likely cause |
| --- | --- |
| `TXV` near 0 and `ALIVE:0` with the feeder on | Feeder TX is not on pin 2: lines crossed, a loose wire, or no shared ground |
| No boot output, but `ALIVE:1` | `uart_port` names the relay port, or another program holds the UART |
| Boot output arrives, but the console or the loader never answers the host | Feeder RX is not on pin 1: lines crossed, or a loose RX wire |
| Garbled output | Missing ground, or a baud rate other than 115200 |
| A normal boot where the loader was asked for | The D cells are in, or the strap does not reach feeder TX |

## Flash a feeder

The order of a session that writes a feeder's flash over UART, each step linking to its detail:

1. Build the bench binaries together, from the repository root:

   ```sh
   cargo build --locked --release -p wellfed-bench -p wellfed-flash -p wellfed-stock-server -p wellfed
   ```

   Don't rebuild `wellfed-flash` until the session ends: a plan binds the hash of the binary that prepared it.

2. Freeze the reviews you need, offline, with `wellfed-build review` ([development updates](uart-development.md#freeze-the-images)): the target, and the source each bank holds now.

3. Wire the feeder as above, with the D cells out and the hopper empty, then run `wellfed-bench status`.

   You should see the lease free, no server, and both ports present.

4. Start the server in a terminal of its own, naming the model:

   ```sh
   target/release/wellfed-bench serve --device plaf103
   ```

   Use `--device plwf106` for the fountain. Run every other command from a second terminal.

5. For a unit the bench has never read, inspect its ROM, then take its first backup:

   ```sh
   target/release/wellfed-bench rom-inspect NAME --candidate-rom <SDK km0_irom.bin>
   target/release/wellfed-bench dump NAME
   ```

   Point `rom_report` and `baseline` in `private/bench.json` at the two results ([bench firmware](../../src/firmware/bench/README.md#use-it-with-the-bench)). A unit the bench knows has both.

6. Take a fresh backup with `wellfed-bench dump NAME`, and keep an independent copy.

7. Prepare the update:

   ```sh
   target/release/wellfed-bench flash-prepare NAME --bank 1 \
     --source private/verification/<review of what bank 1 holds> \
     --target private/verification/<target review>
   ```

   You should see the plan summary and the exact `flash-execute` line to run next. The updater's inspection refuses a `--source` that does not match the bank.

8. Check the plan: the bank, the sectors and erases, the source and target hashes, and that step 6's backup exists. Keep the feeder powered from here on.

   > [!WARNING]
   > The next step erases and programs flash. Don't run heavy host work, such as `make check`, beside it.

9. Run the printed `flash-execute` line exactly as printed.

   You should see the final verification pass and the feeder boot.

10. For the other bank, repeat from step 6 with `--bank 0` and its own source. Write one bank at a time: the other must hold a valid image, except that bank 1 may also be written over a bank 0 still at the pinned baseline ([what the updater writes](uart-development.md#what-the-updater-writes)). `wellfed-bench banks` shows what the bench last saw in each.

11. Finish: `wellfed-bench off`, stop the server with `q`, put back any network or service you changed, and record what ran.

If a step stops, nothing is retried. Don't run `flash-execute` again: start a fresh `flash-prepare … --interrupted` session ([finish an interrupted update](uart-development.md#finish-an-interrupted-update)). Going back to stock has its own commands ([stock and back](../devices/plaf103/bench.md#stock-and-back)).

## Flash the bench board

The board's own firmware is separate from the feeder's. Stop `wellfed-bench serve`, which holds the relay port, then send `AT+BOOTSEL` on the relay port (it cuts feeder power first), or hold BOOT and tap RESET, and copy the UF2 onto the `RPI-RP2` drive ([bench firmware](../../src/firmware/bench/README.md#flash)).

## Common mistakes

- **Crossing TX and RX on the bench board.** Its labels already name the feeder's pins ([wire TX to TX](#wire-tx-to-tx)).
- **Connecting a 3V3 or VCC pin.** The feeder powers itself, and the bench header has no power pin.
- **Leaving the D cells in.** The feeder stays powered when FEEDER 5V is cut, so power-off, loader entry and RECOVER don't work.
- **Swapping `uart_port` and `relay_port`.** Only the relay port answers `AT`.
- **Rewiring with power on.** Unplug FEEDER 5V first.
- **Rebuilding `wellfed-flash` between prepare and execute**, or re-running `flash-execute` after a stop.

## Evidence

- **Recovered, November 4, 2026:** the J2 pinout, silkscreen, nets and internal paths, and the J4 and J5 connections, from the Rev B KiCad schematic and board in `src/hardware/bench/kicad/`.
- **Observed, November 2, 2026:** with a stock PLAF103 on the board, `wellfed-bench` drove `status`, `on`, `off` and `loader` through it, RECOVER reached the ROM loader, and `wellfed-flash` read the whole 8 MiB flash through its bridge, wired label to label.
- **Observed, November 3, 2026:** the fountain's stock boot output, loader entry and a passing `inspect-rom` through the bench.
- **Observed:** the PLAF103's `CON5` labels and pad positions on the research unit ([UART page](../devices/plaf103/uart.md#evidence)).
