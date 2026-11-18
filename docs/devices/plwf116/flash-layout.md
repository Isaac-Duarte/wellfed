# PLWF116 image and flash reference

The stock 0.3.62 OTA container, application mappings and recovered persistent addresses, for reviewing a PLWF116 port. This is not an approved flash layout or write procedure. Unknown ranges are not free space.

## Package identity

The complete package is `0x109020` bytes, or 1,085,472 bytes. Its SHA-256 is `66167f09e0abda5749d2c5e6e0a0c0afa651560b11dce00a36ca8f23b4fe0f3b`.

One `OTA1` descriptor follows an eight-byte prefix. It names a `0x109000`-byte payload starting at file offset `0x20`. The unsigned sum of those payload bytes is `0x066f5c35`, matching the descriptor. The destination word is `0xffffffff`; stock replaces it with the inactive bank ([OTA contract](stock/ota-requirements.md#supplied-envelope)).

This establishes consistency of the downloaded bytes, not a vendor signature or a safe installation.

## Application segments

Each segment has a 32-byte header beginning with the ASCII marker `81958711`. Offsets below distinguish the complete OTA file from its payload. Body ranges are half-open CPU ranges.

| Segment | Header in OTA file | Header in payload | Body length | CPU body range |
| --- | --- | --- | --- | --- |
| KM0 XIP | `0x000020` | `0x000000` | `0x14850` | `0x0c000020..0x0c014870` |
| KM0 RAM | `0x014890` | `0x014870` | `0x24d0` | `0x00083000..0x000854d0` |
| KM4 XIP | `0x017020` | `0x017000` | `0xd69c0` | `0x0e000020..0x0e0d69e0` |
| KM4 RAM | `0x0eda00` | `0x0ed9e0` | `0x1b250` | `0x10005000..0x10020250` |
| Empty PSRAM header | `0x108c70` | `0x108c50` | 0 | Declares `0x02000020`, with no body |

Padding separates the KM0 RAM body from the aligned KM4 header and follows the empty PSRAM header. Preserve the empty header when reconstructing the image. A marker-shaped byte sequence inside code or strings is not another segment: validate lengths, addresses and the surrounding chain.

For KM4 XIP, OTA file offset is `0x17040 + (address - 0x0e000020)`. For KM4 RAM, it is `0xeda20 + (address - 0x10005000)`. Physical flash offsets additionally depend on the selected payload destination.

Import each core separately as little-endian ARMv8-M Thumb code. Map its RAM body alongside XIP before following cross-segment calls. Leave unread ROM, peripherals and BSS uninitialized. A pointer to an address does not supply bytes for that address. Automatic function seeds and decompiler types need review against calls and instruction listings.

## Startup and low-power core

The first KM0 RAM word points to Thumb entry `0x0c002839`; the KM4 word points to `0x0e02d7f1`. These are image entry pointers, not ordinary Cortex vector tables. KM4 startup `0x0e02d7f0` clears BSS at `0x10020260..0x1003a1b4`, configures the runtime and reaches the application startup described in [stock firmware](stock/README.md#startup-and-workers).

KM0 is part of the sleep contract. Its handler `0x0c00352c` receives channel 31's shared sleep structure through `0x0c001d94`. The first byte enables deep sleep, the second selects ordinary sleep type, and the word at offset 4 supplies the requested sleep time.

In the deep-sleep branch it disables the AON timer, optionally installs a nonzero timeout, takes the flash lock, enters the RAM deep-sleep path and releases the lock if that path returns. The relevant thunks are `0x0c011c58`, `0x0c011e28`, `0x0c011e98`, `0x0c011ba8` and `0x0c011ed8`; the deep-sleep thunk targets RAM `0x0008494c`.

Ordinary sleep reaches `0x0c0032f0`, which records a nonzero requested duration against the KM0 tick count. It selects clock gating through `0x0c00329c` for type 1 and power gating through `0x0c003258` otherwise. Both wait for the KM4 sleep flag at `0x4000000c` before the transition. This recovers the cross-core mechanism, not measured wake latency or proof that every wake source is electrically connected. The application-side ordering and battery cutoff are on the [power page](power.md).

## Banks and bootloader boundary

Stock's OTA bank selector names physical starts `0x006000` and `0x186000`. Placing this payload at either start would occupy these ranges:

| Destination | Payload footprint |
| --- | --- |
| Bank 0 | `0x006000..0x10f000` |
| Bank 1 | `0x186000..0x28f000` |

Those are arithmetic footprints of this package, not bank capacities or writer allowances. The FTL lies immediately below bank 1. A replacement's erased sectors, padding and growth need a separate allocation review.

The OTA file contains neither the original boot/system region nor a unit's bank-selection table. It cannot establish loader fallback, secure-boot eFuse state, unsigned-image acceptance or recovery from a marked-valid crash. The PLWF106's matching loader is evidence about that model's backup, not this PLWF116.

## Minimum persistent-address inventory

These addresses come from readers, writers and reset callers. They do not describe the actual contents of a selected unit. Preserve at least the containing erase unit and any wider erase footprint.

| Physical offset or range | Recovered use | KM4 anchor |
| --- | --- | --- |
| `0x182000..0x185000` | FTL, three 4 KiB pages | `0x0e020f00`, configuration at `0x0e0b9549` and `0x0e0b954c` |
| `0x3e8000` | Network/setup record, `0x481` bytes | `0x0e006770`, `0x0e007010` |
| `0x3e9000` | Wi-Fi settings, `0x60` bytes | `0x0e005ecc` |
| `0x3ec000` | One-byte network initialization/retry flag | `0x0e006034` |
| `0x3ed000` | Identity and broker configuration, `0x96` bytes | `0x0e012e82`, `0x0e010ee4` |
| `0x3ef000` | Attributes, `0x6b` bytes | `0x0e00e2c4`, `0x0e016558` |
| `0x3f0000..0x400000` | Whole erase block containing checkpoint word at `0x3ff000` | `0x0e012dcc`, `0x0e013058`, `0x0e0130b4` |
| `0x400000` | Binding counter / identifier, four bytes | `0x0e012e82`, `0x0e013a54` |
| `0x7cb000` | Device log list, `0x1f1` bytes | `0x0e00e2c4`, `0x0e00e870` |
| `0x7cc000` | System information, 200 bytes | `0x0e033b0c`, `0x0e0126d0` |
| `0x7cd000` | Report log, `0x5e0` bytes | `0x0e01953c` |
| `0x7ce000` | Scale zero: state byte and unaligned `u32` | `0x0e036c40`, `0x0e036f40` |
| `0x7cf000` | Scale coefficient: state byte and unaligned `u32` | `0x0e036c40`, `0x0e036f40` |
| `0x7d3000` | Broker-address list, `0x209` bytes | `0x0e006770`, `0x0e0177f4` |
| `0x7d4000..0x7d6000` | Two diagnostic/error record sectors | `0x0e0109a0`, `0x0e0108d0`, `0x0e010b60` |

The FTL initializer reads page count 3 and base `0x182000`, then manages pages at 4 KiB strides. The clock writer `0x0e0120bc` calls block erase `0x0e01d35c`, whose cache maintenance covers 64 KiB. Preserving only the checkpoint's 4 KiB sector would miss the erase footprint.

The common sector writer `0x0e011fb8` erases before programming. It is not an append-only record writer. The sector erase wrapper is `0x0e012080`; flash reads use `0x0e012020`.

Settings reset erases `0x3e8000`, `0x3ef000`, `0x3e9000` and `0x7d3000`. Attributes-only reset erases `0x3ef000`. Neither establishes permission to erase identity or calibration. Unlike the corded reference, this package does not establish an identity backup at `0x3ee000`; preserve unknown sectors until a full backup and wider writer review explain them.

No machine-readable PLWF116 layout or write profile is supplied. An allocation that tools enforce needs full-device evidence and reviewed allowances; this minimum inventory cannot safely stand in for one.

## Evidence

- **Recovered, 2026-11-16:** local parsing, hashing and byte-sum recomputation of OTA 000362. Segment headers and first RAM entry pointers were checked against original bytes.
- **Recovered, 2026-11-16:** mapped KM0 and KM4 analysis of startup, IPC sleep handoff, FTL initialization, fixed-address storage readers, writers and reset callers. IPC structure and channel meanings were compared with the repository's preserved AmebaD SDK. No loader bytes, full-flash contents or eFuse state were supplied.
