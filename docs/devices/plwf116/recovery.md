# PLWF116 backup and recovery limits

What is required before a PLWF116 replacement image can be installed or recovered. The supplied stock OTA package supports static analysis; it does not provide a verified restore path.

## What the OTA package cannot restore

The package contains application segments for KM0 and KM4, with a container checksum. It does not contain the selected unit's original bootloaders, boot/system data, radio material, identity, Wi-Fi settings or scale calibration. A stock OTA reinstall cannot be assumed to reconstruct any of those.

The [persistent-address inventory](flash-layout.md#minimum-persistent-address-inventory) identifies minimum preservation requirements. Unexplained sectors and erased space must also remain intact until an allocation is reviewed. The [power reference](power.md) explains why cutting USB alone may leave the unit powered.

## Evidence needed for a recovery procedure

Before reviewing a writer, establish these on the selected board:

1. Identify its revision, SoC, flash part, power isolation and UART wiring. Establish receive-only access before changing it.
2. Obtain matching full-flash reads and record their hashes privately. Include both application banks, original loaders, calibration and all persistent regions.
3. Analyze that backup's boot selection, marker handling and secure-boot gate. The corded unit's loader does not establish the cordless unit's eFuse state.
4. Define device-specific write allowances, sector and block erase footprints, marker order and preservation checks. Pins for another model are not transferable.
5. Review a bounded stopped-output boot image and its recovery path before allowing an install. Its stop state must include both PA30 and PB4.

These are evidence requirements, not executable flashing steps. No PLWF116 backup pin, approved write window, recovery journal, install profile or custom-image acceptance result exists in this analysis.

## Update failure behavior

Stock writes the inactive bank while holding back its first eight bytes, verifies an unsigned byte-sum, then writes the new marker and invalidates the old marker. That ordering comes from the [OTA writer](stock/ota-requirements.md#activation-and-reset), not a demonstrated recovery test.

It does not establish what the original loader does after a power cut, a valid marker with a bad KM4 image, or a new application that crashes. A replacement needs an explicit trial and rollback policy plus evidence that its recovery image remains bootable. Neither a successful download nor a success report proves that.

## Evidence

- **Recovered, 2026-11-16:** package inventory and stock OTA marker sequence. No full-flash backup was supplied; no PLWF116 flash was read or written, and no restore or custom-image boot was performed.
