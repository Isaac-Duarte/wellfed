# UART and loader entry

This page shows how we reach the PLWF106's console and the ROM's download mode on our unit. It is for developers who want backups. The wiring and tools are the same as the PLAF103's, so this page lists only what differs. Read the [PLAF103 UART page](../plaf103/uart.md) for the full steps.

## The console header

The main board has a four-pin header labelled **GND, TX, RX, 3V3**.

| Fountain | Receive-only | Two-way |
| --- | --- | --- |
| GND | Adapter GND | Adapter GND |
| TX | Adapter RXD | Adapter RXD |
| RX | Not connected | Adapter TXD |
| 3V3 | Not connected | Not connected |

> [!WARNING]
> Leave 3V3 unconnected. The label does not mean the board may be powered from an adapter, and we have not measured what it tolerates. Power the fountain from its own supply, and change wiring only with that supply off.

The logic level and baud rate match the PLAF103's settings, which our bench uses for both models. We have not measured the fountain's idle TX voltage.

## Enter download mode

Hold the fountain's TX low through a **4.7 kΩ resistor** while you power it on from cold, as on the [PLAF103](../plaf103/uart.md#enter-download-mode). On our unit you should see repeated NAK bytes, with the fountain drawing about 14 to 23 mA. The [relay bench](../plaf103/bench.md) does this with its strap relay.

The ROM answers like the PLAF103's. Its sampled words match the same pinned C-cut ROM, so the same pinned RAM read helper runs.

## The stock console

The stock console answers `?` with a KM4 command table at `0e0ba120`. The PLAF103's sits at `0e11df78`. `wellfed-flash read inspect-rom` checks for one of the two known tables before it trusts the console.

Stock logs a burst of output, including Wi-Fi retries, about every 12 seconds. A console query must therefore end when its own reply is complete, not after a fixed wait.

The boot log holds the unit's serial number, product secret and saved Wi-Fi password. Keep captures under `private/`.

## Evidence

- **Observed, 2026-11-03:** on our unit, the header labels, readable stock boot output through our relay bench, download mode by a held TX strap with 14 to 23 mA drawn, and a passing `inspect-rom` against the pinned C-cut ROM.
- **Observed, 2026-11-03:** the KM4 command table at `0e0ba120`, and the stock log bursts that delayed fixed-window console replies.
