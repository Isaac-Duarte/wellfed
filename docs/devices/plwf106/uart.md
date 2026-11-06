# UART and loader entry

How we reach the PLWF106's console and the ROM's download mode on our unit, for developers who want backups. The wiring and tools are the PLAF103's, so this page lists only what differs; the [PLAF103 UART page](../plaf103/uart.md) has the full steps.

## The console header

The main board has a four-pin header labelled **GND, TX, RX, 3V3**. The table is for a bare USB serial adapter. On the Wellfed Bench board, wire label to label (TX to TX, RX to RX, GND to GND) and leave 3V3 unconnected ([bench hardware](../../development/bench-hardware.md#plwf106)).

| Fountain | Receive-only | Two-way |
| --- | --- | --- |
| GND | Adapter GND | Adapter GND |
| TX | Adapter RXD | Adapter RXD |
| RX | Not connected | Adapter TXD |
| 3V3 | Not connected | Not connected |

> [!WARNING]
> Leave 3V3 unconnected. The label doesn't mean an adapter may power the board, and we haven't measured what it tolerates. Power the fountain from its own supply, and change wiring only with that supply off.

The logic level and baud rate match the PLAF103's settings, which our bench uses for both models. The fountain's idle TX voltage is unmeasured.

## Enter download mode

Hold the fountain's TX low through a **4.7 kΩ resistor** while you power it on from cold, as on the [PLAF103](../plaf103/uart.md#enter-download-mode). On our unit you should see repeated NAK bytes, with the fountain drawing about 14 to 23 mA. The [relay bench](../plaf103/bench.md) does this with its strap relay.

The ROM answers like the PLAF103's: its sampled words match the same pinned C-cut ROM, so the same pinned RAM read helper runs.

## The stock console

The stock console answers `?` with a KM4 command table at `0e0ba120` (the PLAF103's is at `0e11df78`). `wellfed-flash read inspect-rom` checks for one of the two known tables before it trusts the console.

Stock logs a burst of output, including Wi-Fi retries, about every 12 seconds, so a console query must end when its own reply is complete, not after a fixed wait.

The boot log holds the unit's serial number, product secret and saved Wi-Fi password. Keep captures under `private/`.

## Evidence

- **Observed, 2026-11-03:** on our unit, the header labels, readable stock boot output through our relay bench, download mode by a held TX strap with 14 to 23 mA drawn, and a passing `inspect-rom` against the pinned C-cut ROM.
- **Observed, 2026-11-03:** the KM4 command table at `0e0ba120`, and the stock log bursts that delayed fixed-window console replies.
