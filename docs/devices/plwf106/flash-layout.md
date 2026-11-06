# PLWF106 flash and boot reference

The stock image and preservation constraints of an 8 MiB PLWF106 backup, for reviewing an image or continuing loader analysis. It is not an installation procedure. The machine-readable copy is [`layout.json`](../../../src/firmware/devices/plwf106/layout.json), and [what the writers allow](#what-the-writers-allow) says how `wellfed-flash` uses it. Ranges are half-open, and offsets are physical flash offsets unless a table marks them as CPU addresses.

## Application segments

Each segment body follows a 32-byte header. KM0 is the low-power core and KM4 the application core; XIP code executes through a flash mapping ([chip](../../chips/rtl8721dm/README.md#memory-map)).

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

Both groups also have an empty PSRAM header (marker `81958711`, length 0, load address `0x02000020`) at `0x0f0e90` and `0x251a30`. Preserve this fifth header in the image format. Both KM0 payloads match byte for byte, padding included; the KM4 payloads differ.

For a bank-0 KM4 XIP address, the matching byte is at `0x1d020 + (address - 0x0e000020)`. RAM bytes use their own segment mapping. Don't search for a runtime address as a physical offset.

## Loader selection and failure behavior

The loader table at CPU address `0x00082898` holds `0x08006000` and `0x08186000`. Loader `0x08000308` tries index 1, then index 0. Helper `0x08000154` checks the KM0 marker, derives the aligned KM4 position from the preceding segment sizes, and checks KM4.

In the reviewed backup, bank 0's leading marker is valid and bank 1's is altered, so the ordinary path predicts bank 0. That is a prediction, not a record of the last bank that ran.

If the KM0 marker is valid but KM4 is invalid, stock enters a warning loop instead of trying the other bank. A marked-valid image that crashes later is not rolled back either, so a replacement needs an explicit trial and recovery policy.

The KM4 loader checks RAM address and size and `RTKWin` structures. It also has a hardware-controlled secure-boot gate whose backing register or eFuse state a flash backup doesn't show, so static analysis can't prove it accepts an unsigned custom image. Keep the original pre-application loader and system region intact.

That whole region, `0x000000..0x006000`, matches a stock PLAF103 backup byte for byte: both loaders, the bank table and the system data. So the fountain runs the [PLAF103's factory loader](../plaf103/flash-layout.md#how-it-boots), whose bank order and fallback the feeder's bank policy relies on. A shared loader can't settle the secure-boot gate, which each chip's eFuse sets. Reading it over UART, or booting a harmless test image, decides whether this unit accepts a custom image.

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

Outside the wider ranges, preserve at least the containing 4 KiB sector. The clock writer calls the block-erase helper, so preserving only its checkpoint sector is not enough.

FTL initialization reads page count 3 and base `0x182000`, then manages pages at 4 KiB strides. Its other two pages being erased in the backup doesn't make them application padding.

A legacy Bluetooth patch-address branch reads flash word `0x3028`. When its low bit is clear, it selects `0x0f8000` or `0x1f8000` by active bank. The reviewed word is erased, so that branch is not selected. These conditional addresses are not separately allocated partitions; one lies within bank 1's image.

## Bounds for an image review

The rounded bank-0 image footprint is `0xeb000` bytes, ending at `0x0f1000`; from bank 1's start the same length ends at `0x271000`, the writers' window below. It bounds a first build's review, and is neither the bank's capacity nor proof that a complete firmware fits. Review any expansion against the actual segments, FTL, radio material and persistent records. MMU window sizes don't allocate flash. The feeder's store at `0x340000` and its payload limit don't transfer to the fountain, and no store allocation is established here.

## What the writers allow

With `--device plwf106`, the `wellfed-flash` writers write the fountain only within these bounds. Bank 0 is never written, so the stock image the loader falls back to stays intact.

| Range | Writers |
| --- | --- |
| `0x186000..0x271000`, bank 1 | `update` installs a candidate here, `give-way` clears its first byte, `stock-recovery` puts the backup's bytes back |
| `0x006000..0x0f1000`, bank 0 | Read only. A give-way requires it to equal the backup's byte for byte |
| Every row of the [minimum preservation map](#minimum-preservation-map) | Never written. A frame or sector that touches one is refused |
| `0x180000..0x186000`, bank 1's first block below the bank, with the FTL | Read before any write, with its hash bound into the plan. Read back after every erase in that block and at every region check, and required to match that hash, in case the part erases whole blocks |
| Everything else | Never written |

A candidate's payload is therefore at most `0xeb000` bytes. In our backup, the window past bank 1's stock image (`0x252000`) is erased. `layout.json` and the board crate's stock spans merge adjacent rows of the map and widen one: `0x3e8000..0x3f0000` also covers `0x3ec000`, which holds data of unknown role, and the erased `0x3ef000`. A test holds the two to the same sectors.

Every writer is pinned by SHA-256 to our unit's backup and to the backup's own bank 0 and bank 1, so a backup of another unit is refused, even another fountain's. The steps are on the [recovery page](recovery.md).

## Still unknown

Whether the loader accepts an unsigned custom image, how a failed boot recovers on its own, and where a full PLWF106 image or store may go.

## Evidence

- **Recovered, 2026-11-03:** both application groups and the mapped KM0 and KM4 loaders, analysed in Ghidra. Extracted bodies, empty headers, the bank table and initialized memory-block hashes matched the backup.
- **Recovered, 2026-11-03:** the storage readers, writers and reset callers, FTL configuration and garbage collection, and the clock's block-erase paths. Generic console writes mean the fixed writer addresses are not an exhaustive partition map.
- **Recovered, 2026-11-04:** a sector-by-sector comparison of `0x000000..0x006000` between the PLWF106 backup and a stock PLAF103 backup. All six 4 KiB sectors are identical.
- **Recovered, 2026-11-04:** the backup's written ranges. `0x3ec000` holds data, and `0xf1000..0x182000` and `0x252000..0x3e8000` are erased.
