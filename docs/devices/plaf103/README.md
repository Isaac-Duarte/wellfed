# PETLIBRO Granary PLAF103

The 5 L Granary feeder is the model Wellfed is built on: every Wellfed feature is developed and tested on one PLAF103 on our bench.

## Sold as

PETLIBRO Granary automatic feeder, 5 L, model **PLAF103**, FCC ID **2A3DEPLAF103S**. Our unit shipped with stock firmware 1.1.15.

## How to identify it

- **Model and FCC ID:** PLAF103 and 2A3DEPLAF103S, on the label.
- **Board:** the [FCC internal photos](https://fccid.io/2A3DEPLAF103S/Internal-Photos/Internal-Photos-6465982.pdf) show a board marked `PLAF103 MB V1.0`. Our unit's PCB revision is not established, so compare your own board's markings.
- **Flash ID:** `C8 40 17` over UART.

> [!WARNING]
> Don't assume a PLAF108 is compatible. It reportedly uses a different SoC family (ESP8266/ESP32), and nothing here was checked on one. [Supported devices](../README.md) lists the other PETLIBRO models we know of.

## SoC and flash

| Part | What it is |
| --- | --- |
| SoC | Realtek [RTL8721DM](../../chips/rtl8721dm/README.md) (AmebaD) |
| Flash | GigaDevice GD25Q64ESIG, 64 Mbit (8 MiB) SPI NOR ([datasheet](https://download.gigadevice.com/Datasheet/DS-00484-GD25Q64E-Rev1.6.pdf)) |
| Motor driver | TC118AH single-channel H-bridge, U7, beside CON3 ([motor driver](motor-driver.md)) |
| Console | Three-hole `CON5` header: RX, TX, GND ([UART](uart.md)) |

The [board page](board.md) shows how the food path and the parts reach the chip, and maps every SoC pin the feeder uses.

## UART access

The console is a 3.3 V UART at 115200 baud, 8N1, on `CON5`. Holding the feeder's TX low through a 4.7 kΩ resistor at power-on enters the ROM's download mode. The [UART page](uart.md) has the wiring and steps, and the [relay bench](bench.md) automates them.

## Recovery

The whole 8 MiB flash reads over UART, and repeated reads match. A pinned writer has restored stock 1.1.15 to bank 1 and booted it. It accepts only our unit's exact images, so it is not a general installer. Full-bank restores and recovery from a power cut mid-write are unverified. See [recovery](recovery.md) and the [flash layout](flash-layout.md).

Owners don't need UART: Wellfed installs through stock's own updater and goes back to stock over the air ([consumer installation](../../firmware/consumer-installation.md)).

## Hardware blocks

The Stock column summarizes [stock 1.1.15](stock/README.md), and the [roadmap](../../roadmap.md) says where each capability stands. All of it holds for our unit and the builds we tested, not as a general reliability claim ([evidence levels](../../evidence.md)).

| Block | Stock 1.1.15 | Wellfed |
| --- | --- | --- |
| Console, loader and backups | ROM loader over [UART](uart.md) | Reads, writes and recovers the feeder over the same UART. |
| Startup and build identity | — | Starts with the motor stopped and reports its build. |
| Flash, journal and settings | Factory loader picks the bank | See [flash layout](flash-layout.md). |
| Stop timer and watchdog | — | Both run under Wellfed. |
| Clock and time zones | Synced by the cloud | Needs a time source after each power-up. |
| Wi-Fi | Cloud MQTT | DHCP-server loss and lease expiry are untested. |
| Local device API | None | See [device API](../../firmware/device-api.md). |
| Bluetooth LE | Wi-Fi and broker setup | Setup and pairing. |
| Updates and boot recovery | Own updater, no rollback ([requirements](stock/ota-requirements.md)) | Signed updates with rollback, and both ways between stock and Wellfed. |
| Motor and feeding | Yes | Supervised, and a power cut never repeats a feed. |
| Food, outlet and feedback sensors | Yes | The status and the low-food light show the latest food reading. |
| Power and battery | Yes | Stock's battery mode. The charge, the floor and the brownout hold are tested only in simulation. |
| Buttons and child lock | Yes ([stock](stock/README.md#panel-buttons-and-child-lock)) | The reset button clears clients, Wi-Fi, settings, plans, history and statistics. |
| Indicators | Yes | Which states show is not settled. |
| Speaker | Yes | Which sounds play when is not settled. |
| Schedules | Yes ([stock](stock/README.md#clock-and-schedules)) | Up to ten plans, run on the feeder with no server. |
| Home Assistant | Through the vendor cloud | Direct over the LAN, with no server; tested in simulation only. |

## Notes

- Every finding comes from one research unit. Compare your own board before trusting a pin or an offset.
- There is no schematic and no complete connector pinout. The routes from the SoC to the motor driver are not traced.
- Unknown flash ranges are not free space.

## Pages

| Topic | Page |
| --- | --- |
| Signal map and connectors | [Board](board.md), [chip pin table](../../chips/rtl8721dm/pins.md) |
| Motor driver and electrical checks | [Motor driver](motor-driver.md) |
| Console and loader entry | [UART](uart.md) |
| Flash layout and boot | [Flash layout](flash-layout.md) |
| Backup and recovery | [Recovery](recovery.md) |
| Relay bench automation | [Bench](bench.md), [bench server](bench-server.md), [integration suite](integration-suite.md) |
| Stock 1.1.15 behavior and protocol | [Stock firmware](stock/README.md), [stock OTA requirements](stock/ota-requirements.md) |

Machine-readable device data lives in [`src/firmware/devices/plaf103/`](../../../src/firmware/devices/plaf103/).

## Evidence

- **Observed:** board photographs of the research unit identify the RTL8721DM and the GD25Q64ESIG. UART reports flash ID `C8 40 17`, which matches the GD25Q64E's nominal 8 MiB. Two separately acquired 8 MiB reads matched.
- **External:** the FCC internal-photo exhibit is a comparison reference only. The research unit's own PCB revision is not established.
