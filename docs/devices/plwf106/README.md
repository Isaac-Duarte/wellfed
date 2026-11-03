# PLWF106 (Dockstream 2 Smart Fountain)

This reference describes the corded PETLIBRO PLWF106 and its stock firmware for someone starting a fountain port. Start with [stock behavior](stock/README.md), then use the board and flash references. These findings do not establish that a Wellfed feeder image can run on this board. See [supported devices](../README.md) for support coverage.

## Sold as

PETLIBRO Dockstream 2 Smart Fountain, model PLWF106, corded 5 V / 1 A version. Reservoir capacity is not established by this analysis. The related PLWF116 shares PCB markings, but its populated components and firmware are outside this reference.

## How to identify it

The inspected unit's main board reads `PLWF106/116 MB Board V0.5`, dated `2025.04.24`. Its weight/LED daughterboard reads `WF106/116 Weith_LED Board V0.3`, dated `2025.03.01`. The shared marking does not make the variants electrically interchangeable.

The stock KM4 image contains version `0.0.17`, build date `Aug 22 2025`, time `19:28:23`. The other retained bank differs despite carrying the same version. Function addresses in these pages refer to bank 0 unless stated otherwise.

## SoC and flash

Stock uses the [RTL8721DM](../../chips/rtl8721dm/README.md), with KM0 and KM4 images. The acquired external-flash image covers 8 MiB; the flash manufacturer's part number is not established here. Pump enable is PA30. A clocked external weight ADC uses PB5/PB6. Its U1 package marking is unreadable, so its silicon identity remains unknown.

The [board reference](board.md) separates firmware pin assignments from photographed component population. The RADAR-labelled connector footprints are unpopulated on the photographed unit.

## UART access

The photographed header is labelled GND, TX, RX and 3V3. These labels do not authorize powering the board from an adapter. This reference supplies no new wiring, baud-rate or ROM-entry procedure. Use a reviewed device-specific bench procedure before connecting or operating hardware.

## Recovery

Full flash backups and retained UART logs exist for the inspected unit. Backup acquisition does not prove a tested custom-image recovery path. The [flash reference](flash-layout.md) explains the loader's asymmetric fallback behavior. A marked-valid image that crashes does not automatically cause stock to retry the other bank.

No machine-readable PLWF106 install profile or persistent-store allocation is supplied by these docs. The PLAF103 profile is unsuitable unchanged: this fountain has additional calibration sectors and different stopped-output requirements.

## Hardware blocks

| Block | Stock | Requirement for a fountain port |
| --- | --- | --- |
| Flash and boot | Two core-image groups; bank 1 preferred | Fountain-specific layout, trial policy and recovery review |
| Wi-Fi and cloud | Starts communications alongside local watering tasks | Keep circulation independent of a vendor cloud |
| Bluetooth LE | SDK components and FTL code present | Fountain setup behavior needs separate protocol analysis |
| OTA | HTTP download, byte-sum verification, opposite-bank activation | Preserve loader and marker invariants |
| Pump | PA30 active high; continuous and interval modes | One owner with bounded faults and sensor-freshness checks |
| Weight | External clock/data ADC; stored zero/scale | Preserve factory calibration and bound readiness waits |
| Button | Active-low PA15; nominal three-second reset hold | Explicit debounce and reset scope |
| Indicators | Six identified software roles; three additional startup outputs | Verify physical roles before naming colors |
| Radar | Telemetry field exists; connector footprints unpopulated | Leave unknown radar pins unconfigured |
| Clock | Local worker and persistent checkpoint | Define behavior with unknown wall time |
| Power and battery | Main board includes an unpopulated BAT-labelled footprint | Do not import the cordless model's power assumptions |

## Notes

Stock factory commands can suppress normal pump shutdown. The sensor's ready wait can also block indefinitely while an unrelated watchdog timer keeps running. These behaviors explain stock; they are unsuitable safety policies for a replacement.

A fresh agent should read all five pages below before editing a driver or allocating flash. The [implementation boundaries](stock/README.md#requirements-for-a-replacement) identify which existing feeder paths must change. Hardware tests, new firmware builds, installation and pump operation are separate work from this static reference.

## Pages

| Topic | Page |
| --- | --- |
| Board signals and physical unknowns | [Board](board.md) |
| Images, storage and loader behavior | [Flash layout](flash-layout.md) |
| Runtime tasks, pump, scale, reset and telemetry | [Stock firmware](stock/README.md) |
| Request, transport and activation contract | [Stock OTA](stock/ota.md) |

## Evidence

- **Recovered, 2026-11-03:** direct Ghidra analysis of both retained KM4 banks, bank-0 KM0 and the flash-resident boot loaders. The source acquisition digest, extracted segments and initialized Ghidra memory maps were checked against the original 8 MiB backup.
- **Observed, 2026-11-03:** owner photographs establish the PCB markings, white WEIGH ribbon and unpopulated RADAR footprints. U1 has no legible part marking in the supplied close-up.
- **Observed, reviewed 2026-11-03:** retained stock boot logs corroborate stored weight settings. No new device operation or Wellfed fountain execution was performed for this analysis.
