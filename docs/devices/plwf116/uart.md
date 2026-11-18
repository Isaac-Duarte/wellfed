# PLWF116 UART evidence

What the supplied OTA image establishes about diagnostics, for planning a read-only inspection of a cordless fountain. It does not establish a physical connection procedure.

## Console code

The image includes the AmebaD diagnostic framework and AT command strings for system, Wi-Fi, Bluetooth and sleep controls. Application logs cover startup, water control, radar, battery voltage, source changes and suspend/resume. The version printed by application startup is `0.3.62`.

These strings identify useful capture targets. They do not establish which commands are registered on a selected unit, its baud rate, or that the console is accessible through an external connector. Diagnostic commands can change outputs, settings, radio state and flash; their presence is not a receive-only interface contract.

## Physical access remains unknown

| Item | PLWF116 evidence |
| --- | --- |
| Header position and labels | **Unknown** |
| Header-to-SoC continuity | **Unknown** |
| Console voltage and baud | **Unknown** |
| ROM download strap and its electrical effect | **Unknown** on a PLWF116 |
| Battery isolation and possible back-power paths | **Unknown** |

The chip family uses PA7/PA8 for LOGUART ([pin reference](../../chips/rtl8721dm/pins.md)). That is a lead for continuity testing, not a verified PLWF116 header assignment. The [PLWF106 UART procedure](../plwf106/uart.md) records what was observed on the corded unit only.

The cordless board adds a battery, source sensing and boost control. A USB power cut does not establish that the board is unpowered. Read the [bench hardware rules](../../development/bench-hardware.md) before any physical work, and never connect the feeder header's 3V3 to a target.

## Useful receive-only evidence

A future capture should identify model, build, source classification and reset reason before relying on the firmware analysis. It should distinguish DC-only, DC plus battery and battery-only operation, then correlate reported voltage with a meter. Receiving a boot log does not validate pump shutdown or backup recovery.

Stock diagnostic output may expose identity and network secrets. Keep raw captures private and publish only redacted findings. Backup and loader evidence needed for recovery is listed on the [recovery page](recovery.md).

## Evidence

- **Recovered, 2026-11-16:** diagnostic and application log strings in OTA 000362, with startup and GPIO code review. No UART connection, loader entry or physical operation was performed on a PLWF116.
