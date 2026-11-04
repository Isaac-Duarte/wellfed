# PLWF106 flash and boot reference

This page records the stock image and preservation constraints for an 8 MiB PLWF106 backup. Use it to review an image or continue loader analysis. It is not a write allowance or an installation procedure, and `src/firmware/devices/` holds no machine-readable layout for this model. Ranges are half-open. Offsets are physical flash offsets unless a table marks them as CPU addresses.

## Application segments

Each segment body follows a 32-byte header. KM0 is the low-power core; KM4 is the application core. XIP means code executes through a flash mapping.

| Bank | Core / segment | Header offset | Body length | CPU load address |
| --- | --- | --- | --- | --- |
| 0 | KM0 XIP | `0x006000` | `0x14580` | `0x0c000020` |
| 0 | KM0 RAM | `0x01a5a0` | `0x2490` | `0x00083000` |
| 0 | KM4 XIP | `0x01d000` | `0xba1c0` | `0x0e000020` |
| 0 | KM4 RAM | `0x0d71e0` | `0x19c90` | `0x10005000` |
| 1 | KM0 XIP | `0x186000` | `0x14580` | `0x0c000020` |
| 1 | KM0 RAM | `0x19a5a0` | `0x2490` | `0x00083000` |
| 1 | KM4 XIP | `0x19d000` | `0x9bfa0` | `0x0e000020` |
| 1 | KM4 RAM | `0x238fc0` | `0x18a50` | `0x10005000` |

Both groups also have an empty PSRAM header: marker `81958711`, length 0, load address `0x02000020`. Its header offsets are `0x0f0e90` and `0x251a30`. Preserve this fifth header in the corresponding image format. Both KM0 payloads, including their padding, match byte-for-byte; KM4 payloads differ.

For a bank-0 KM4 XIP address, the matching byte is at `0x1d020 + (address - 0x0e000020)`. RAM bytes use their own segment mapping. Do not search a runtime address as a physical offset.

## Loader selection and failure behavior

The loader table at CPU address `0x00082898` contains `0x08006000` and `0x08186000`. Loader `0x08000308` tries index 1 first, then index 0. Loader helper `0x08000154` checks the KM0 marker, derives the aligned KM4 position from the preceding segment sizes, and checks KM4.

In the reviewed backup, bank 0's leading marker is valid and bank 1's is altered. The ordinary path therefore predicts bank 0. This is not a captured record of the last executed bank.

If the KM0 marker is valid but KM4 is invalid, stock enters a warning loop instead of trying the other bank. A later crash in a marked-valid image also has no automatic trial rollback. A replacement must provide an explicit trial/recovery policy.

The KM4 loader checks RAM address/size and `RTKWin` structures. It also has a hardware-controlled secure-boot gate. Its backing register/eFuse state is not established by a flash backup, so static analysis does not prove acceptance of an unsigned custom image. Keep the original pre-application loader/system region intact.

The whole pre-application region, `0x000000..0x006000`, matches a stock PLAF103 backup byte for byte: both loaders, the bank table and the system data. The fountain therefore runs the [PLAF103's factory loader](../plaf103/flash-layout.md#how-it-boots), whose bank order and fallback the feeder's bank policy already relies on. What a shared loader cannot settle is the secure-boot gate, which each chip's eFuse sets. Reading it over UART, or booting a harmless test image, decides whether this unit accepts a custom image.

## Minimum preservation map

Preserve complete overlapping erase units, including erased pages within an allocated region. The table is a minimum inventory, not proof that other addresses are free.

| Flash offset or range | Recovered stock use | KM4 anchor |
| --- | --- | --- |
| `0x000000..0x006000` | Boot/system material | Flash-resident loaders |
| `0x182000..0x185000` | FTL: three 4 KiB pages | `0x0e01701c`, config `0x0e09f74c` |
| `0x3e8000` | Auxiliary sector cleared on Wi-Fi reset; record role unresolved | `0x0e0050c0` |
| `0x3e9000` | Wi-Fi settings; 96-byte read | `0x0e004790` |
| `0x3ea000..0x3ec000` | Attributes; 111-byte record, two-sector reset | `0x0e00a1c8`, `0x0e005438` |
| `0x3ed000` | Identity/MQTT configuration; 150-byte read | `0x0e00e218` |
| `0x3ee000` | Identity/MQTT backup | `0x0e00e218` |
| `0x3f0000..0x400000` | Entire 64 KiB erase block containing clock checkpoint at `0x3ff000` | `0x0e00e144`, `0x0e00d9b8`, `0x0e0139a0` |
| `0x400000` | Binding counter / ID | `0x0e00e218` |
| `0x7cb000` | Device log list | `0x0e00a1c8` |
| `0x7cc000` | System info; 200-byte record | `0x0e021a78`, `0x0e00dce4` |
| `0x7cd000` | Report log; 1,504-byte record | `0x0e0126c4` |
| `0x7ce000` | Weight zero record | `0x0e024d34`, `0x0e024f88` |
| `0x7cf000` | Weight coefficient record | `0x0e024d34`, `0x0e024f88` |

Except for explicitly wider ranges, preserve at least the containing 4 KiB sector. The clock writer calls the block-erase helper, so preserving only its checkpoint sector is insufficient.

FTL initialization reads page count 3 and base `0x182000`, then scans/manages pages at 4 KiB strides. Its other two pages being erased in the backup does not make them application padding.

A legacy Bluetooth patch-address branch reads flash word `0x3028`. When its low bit is clear, it selects `0x0f8000` or `0x1f8000` by active bank. The reviewed word is erased, so that branch is not selected. These conditional addresses are not evidence of independently allocated partitions; one lies within bank 1's image.

## Bounds for a future image review

The rounded bank-0 image footprint is `0xeb000` bytes from its start, ending at `0x0f1000`. The same length from bank 1's start ends at `0x271000`. This can be a conservative reference for reviewing a first build. It is neither the maximum bank capacity nor proof that a complete new firmware fits.

Review any expansion against actual segments, FTL, radio material and persistent records. MMU window sizes do not allocate flash. The feeder's store approval at `0x340000` and its payload limit do not transfer to this fountain. No new store allocation is established here.

## Still unknown

Whether the loader accepts an unsigned custom image, how a failed boot recovers, and where a reviewed PLWF106 image or store may be written.

## Evidence

- **Recovered, 2026-11-03:** both application groups and mapped KM0/KM4 loaders analysed in Ghidra. Extracted bodies, empty headers, bank table and initialized memory-block hashes matched the acquired backup.
- **Recovered, 2026-11-03:** storage reader/writer/reset callers, FTL configuration and garbage collection, and clock block-erase paths traced. Generic console writes prevent treating fixed writer addresses as an exhaustive partition map.
- **Recovered, 2026-11-04:** a sector-by-sector comparison of `0x000000..0x006000` between the PLWF106 backup and a stock PLAF103 backup. All six 4 KiB sectors are identical.
