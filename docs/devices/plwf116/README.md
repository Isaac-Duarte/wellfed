# PLWF116 (Dockstream 2 Cordless)

The PLWF116 is PETLIBRO's cordless Wi-Fi water fountain. These pages describe stock firmware 0.3.62 from its OTA package, for developers reviewing a fountain port. Wellfed does not run on this model.

## Sold as

PETLIBRO Dockstream 2 Cordless. Reservoir capacity, battery capacity and the supplied adapter's rating are **Unknown** in this analysis. The firmware's weight and voltage tables do not establish those specifications.

## How to identify it

| What | Evidence available |
| --- | --- |
| Package | `PLWF116_OTA_000362.bin` |
| Application model | `PLWF116`, used in network topics and setup naming |
| Application version | `0.3.62` |
| Application build | `Oct 15 2026 16:55:09` |
| FCC ID | `2A3DE-PLWF116`, from the external device inventory |
| Board revision and fitted components | **Unknown** for the unit this package targets |
| Unit flash ID and factory configuration | **Unknown**; no full backup supplied |

The [PLWF106](../plwf106/README.md) unit's boards carry shared PLWF106/116 markings. That observation does not establish the PLWF116's component population or make the images interchangeable. Its cordless firmware has different GPIO roles, settings and power policy.

## SoC and flash

The package contains [RTL8721DM](../../chips/rtl8721dm/README.md) KM0 and KM4 application images. Their segment headers and memory addresses match the AmebaD format. The flash part and physical capacity remain **Unknown**; accesses near 8 MiB establish stock address use, not a measured chip size.

The application controls a pump enable, a separate boost request, radar detection and configuration lines, an external scale ADC, battery ADC and power-source inputs. The [board reference](board.md) distinguishes recovered software roles from unknown electronics.

## UART access

No PLWF116 header, console voltage, baud rate or loader-entry wiring was physically checked. The [UART reference](uart.md) records what the image establishes and what needs observation. Do not transfer a neighboring model's wiring procedure without checking this board.

## Recovery

An OTA package is not a full-flash backup. It lacks the unit's original loaders, credentials, calibration and radio configuration. No PLWF116 write, restore or custom-image boot was performed. Read [recovery](recovery.md) and the [image and flash reference](flash-layout.md) before considering a writer or storage allocation.

## Hardware blocks

| Block | Stock behavior recovered from this package | Wellfed |
| --- | --- | --- |
| Console and backups | Diagnostic command framework; no unit backup | No model-specific recovery contract |
| Flash and boot | KM0/KM4 segments; opposite-bank update selection | No board crate, layout or install profile |
| Wi-Fi | Local workers plus cloud connectivity; battery retry and sleep policy | No fountain transport implementation |
| Bluetooth LE | Setup frames and battery-service callback | No model-specific setup implementation |
| Over-the-air updates | Plain HTTP, payload byte-sum, marker activation | No PLWF116 installer |
| Power and battery | Three source classifications, ADC estimate, charging indication, sleep and low-battery shutdown | No cordless power policy |
| Button | Active-LOW reset input, nominal three-second hold | No model-specific input layer |
| Indicators | Network, water, filter and battery roles | No model-specific indication |
| Clock | RTC timestamps, checkpoint and timed watering | No fountain timing policy |
| Pump | PA30 enable and PB4 boost; continuous, interval and radar modes | No fountain pump owner |
| Water level and scale | PB5/PB6 serial ADC; stored zero and coefficient | No model-specific calibration or sensing layer |
| Radar | Detection interrupt and separate left/right configuration endpoints | No radar driver |

## Notes

- **Power matters throughout the application.** Battery operation selects radar watering at startup, changes telemetry and reconnect behavior, and permits sleep. Remote settings can change the watering mode afterwards. The [power reference](power.md) follows those paths and their failure cases.
- **Static analysis is the evidence boundary.** No voltage, current, runtime, pump timing, sensor response or recovery reliability was measured.
- **This is a package-specific reference.** Stored defaults, ROM implementations, eFuse calibration and external circuitry are not all contained in the OTA file.
- **Stock behavior needs its own safety review.** Scale waits can block, pump requests have several callers, and low-battery shutdown disables the watchdog before a potentially unbounded wait.

## Pages

| Topic | Page |
| --- | --- |
| Signal assignments and initialization | [Board signals](board.md) |
| Source detection, battery, charging and sleep | [Power management](power.md) |
| Console and loader evidence | [UART](uart.md) |
| Container, segments and persistent records | [Image and flash reference](flash-layout.md) |
| Backup and restore limits | [Recovery](recovery.md) |
| Pump, scale, radar, settings and reset | [Stock firmware](stock/README.md) |
| Setup, MQTT and network sleep | [Communications](stock/communications.md) |
| Download, staging and activation | [Stock OTA contract](stock/ota-requirements.md) |

## Evidence

- **Recovered, 2026-11-16:** static analysis of the [supplied OTA package](https://oss.us.petlibro.com/dlcloud/platform/device_package/PLWF116/000362.bin/2026-10-27/03:31:14/PLWF116_OTA_000362.bin), SHA-256 `66167f09e0abda5749d2c5e6e0a0c0afa651560b11dce00a36ca8f23b4fe0f3b`. Its payload byte-sum, segment boundaries, model, version and build strings were checked locally.
- **External:** product name and FCC ID are the existing [device inventory](../README.md)'s external identification. No PLWF116 physical observation is claimed.
