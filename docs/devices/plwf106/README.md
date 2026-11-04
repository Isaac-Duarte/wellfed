# PLWF106 (Dockstream 2 Smart Fountain)

The PLWF106 is PETLIBRO's corded Wi-Fi water fountain, on the same RTL8721DM chip as the PLAF103 feeder. Wellfed does not run on it. These pages describe its board and stock firmware for anyone starting a fountain port.

## Sold as

PETLIBRO Dockstream 2 Smart Fountain, model PLWF106, the corded 5 V / 1 A version. The reservoir capacity is **Unknown**. The PLWF116 shares the PCB markings, but its parts and firmware are not covered here.

## How to identify it

| What | Our unit |
| --- | --- |
| Main board | `PLWF106/116 MB Board V0.5`, dated `2025.04.24` |
| Weight and LED board | `WF106/116 Weith_LED Board V0.3`, dated `2025.03.01` |
| Stock firmware | KM4 version `0.0.17`, built `Aug 22 2025 19:28:23` |
| Stock console | Its KM4 command table sits at `0e0ba120`, where the PLAF103's is at `0e11df78` |
| FCC ID | **Unknown** |

The shared PLWF106/116 marking does not make the two models interchangeable. Both firmware banks report version `0.0.17`, but their KM4 images differ. Addresses on these pages refer to bank 0 unless stated otherwise.

## SoC and flash

The fountain uses the [RTL8721DM](../../chips/rtl8721dm/README.md), with KM0 and KM4 images. The flash holds 8 MiB, read in full over UART. The flash part number is **Unknown**.

Other parts that matter:

- **Pump:** switched from PA30, HIGH for on.
- **Weight sensor:** an external clocked ADC on PB5 and PB6, on the weight and LED board. Its chip, U1, has no readable marking, so its identity is **Unknown**.
- **Antenna:** a U.FL (IPEX MHF1) connector on the main board.
- **Unfitted footprints:** RADAR and BAT are empty on our unit.

The signal map is on the [board page](board.md), and the fountain has a column in the [chip pin table](../../chips/rtl8721dm/pins.md).

## UART access

The board has a four-pin header labelled **GND, TX, RX, 3V3**. Our bench reads the stock console through it and enters the ROM's download mode by holding TX low through 4.7 kΩ at power-on, as on the PLAF103. The details are on the [UART page](uart.md).

## Recovery

We have two full 8 MiB reads of our unit that match byte for byte. `wellfed-flash` has fountain writers pinned to that backup: a test image into bank 1 only, a give-way back to stock bank 0, and an exact restore of bank 1. None has run on the fountain yet. See [backup and recovery](recovery.md) and the [flash layout](flash-layout.md).

The stock loader does not fall back to the other bank when a marked-valid image crashes. Over UART, a give-way does that by hand. A Wellfed image's trial boot would have to do it unattended. [`layout.json`](../../../src/firmware/devices/plwf106/layout.json) gives the fountain's banks and preserved ranges. There is no install profile for this model. The PLAF103 profile does not fit unchanged: this fountain keeps calibration records the feeder doesn't have, and different pins must be held off.

## Hardware blocks

| Block | Stock | Wellfed |
| --- | --- | --- |
| Console and backups | Stock console over the header; ROM download mode by strap | Bench reads, and bank-1 writes and restore pinned to our backup, same tools as the PLAF103 |
| Flash and boot | Two application banks; the loader tries bank 1 first | A layout and bank-1 write window; no trial policy for this model |
| Wi-Fi | Starts alongside the local watering tasks | Nothing for this model |
| Bluetooth LE | SDK components and FTL code present | Nothing; the setup protocol needs its own analysis |
| Over-the-air updates | HTTP download, byte-sum check, opposite-bank activation ([contract](stock/ota-requirements.md)) | Nothing for this model |
| Pump | PA30; continuous or interval circulation | Nothing; a port needs one pump owner with bounded faults |
| Weight and water level | External ADC, stored zero and scale | Nothing; a port must keep the factory calibration |
| Button | PA15, active LOW; three-second hold resets Wi-Fi | Nothing for this model |
| Indicators | Six known roles, plus three outputs set HIGH at startup | Nothing; physical colors are **Unknown** |
| Radar | A telemetry field only; footprints unfitted | Leave the unknown radar pins unconfigured |
| Clock | Local worker with a stored checkpoint | Nothing for this model |
| Power and battery | Corded; BAT footprint unfitted | Do not reuse the PLAF103's battery assumptions |

## Notes

- **Wi-Fi on our unit:** after we opened it, it no longer joined its network, and app setup did not complete. The antenna lead is the suspect, not yet confirmed. Its Wi-Fi findings may not reflect a healthy unit.
- **Stock's safety shortcuts:** its factory commands can suppress the normal pump shutdown, and its sensor wait can block forever while an unrelated watchdog keeps running. Both explain stock. Neither is a policy to copy.
- **The feeder's stops miss the pump:** the feeder's panic and flash-hold paths clear PA13/PA14 and the emitters, but not PA30. A fountain build must hold the pump off in every one of those paths. See [what a port must change](stock/README.md#what-a-port-must-change).
- No Wellfed code, simulation or pump operation has run on this model.

## Pages

| Topic | Page |
| --- | --- |
| Signal map and connectors | [Board](board.md) |
| Console and loader entry | [UART](uart.md) |
| Flash layout and boot | [Flash layout](flash-layout.md) |
| Backup and recovery | [Recovery](recovery.md) |
| Stock firmware: tasks, pump, scale, reset, telemetry | [Stock firmware](stock/README.md) |
| Stock update request, transfer and activation | [Stock OTA](stock/ota-requirements.md) |

## Evidence

- **Observed, 2026-11-03:** on our unit, the PCB markings, the WEIGH ribbon, the U.FL antenna connector and the unfitted RADAR and BAT footprints, from owner photographs. U1 has no readable marking in the close-up.
- **Observed, 2026-11-03:** on our bench, stock boot output over the header, ROM download mode by strap, the KM4 command table at `0e0ba120`, and two matching full reads.
- **Observed, 2026-11-03:** after reassembly, the unit did not join its Wi-Fi network in repeated boots.
- **Recovered, 2026-11-03:** static analysis of both KM4 banks, the bank-0 KM0 image and the flash-resident loaders from our backup. The stored weight settings match stock's own boot logs.
