# PLWF106 stock OTA contract

Stock 0.0.17's update request, HTTP transfer and activation, for reviewing an image or profile. It is not an installation recipe. Read the [flash layout](../flash-layout.md) before reasoning about payload bounds or fallback.

## Request precheck

Handler `0x0e010fd4` requires `upgradeType`, `url`, `targetSoftwareVersion` and `msgId`; a missing field sets OTA failure state. Types 2 and 3 bypass the same-version check. Other types reject an exact string match to `0.0.17`, not a semantic version ordering.

The outer handler admits URLs of at most 256 bytes, but inner parser `0x0e012cf4` copies at most 128 bytes into its working buffer. A reviewed candidate should use a whole ASCII URL under 128 bytes, a short IPv4 host and path, and an ordinary decimal port. The handler doesn't report every parsing failure as a refusal, so don't probe malformed URLs on hardware.

## HTTP behavior

Despite the `https_ota` log tag, the transfer opens a plain TCP socket and writes an HTTP request directly, so a profile uses HTTP, not inferred TLS support. The request's `Host` field omits a non-default port.

Parser `0x0e01ac50` accepts status 200 and has a separate 302 branch. It recognizes `Content-Length` and `CONTENT-LENGTH`, and the caller rejects zero length. Serve a direct 200 with an exact length and the unchanged image bytes. Chunking, lowercase header names, compression and redirects are outside the profile.

## Envelope and bank choice

Bank selector `0x0e01a3a0` reads the mapping registers, and the destination is the opposite application bank. Descriptor parser `0x0e01a47c` replaces the envelope's destination with that address.

The single-descriptor envelope is an eight-byte prefix and a 24-byte descriptor, in little-endian words:

| File byte offset | Bytes | Meaning |
| --- | --- | --- |
| 0 | 4 | Outer version word |
| 4 | 4 | Descriptor count |
| 8 | 4 | Identifier |
| 12 | 4 | Descriptor size / stride |
| 16 | 4 | Payload byte-sum |
| 20 | 4 | Payload size |
| 24 | 4 | Payload offset |
| 28 | 4 | Destination, replaced by selected bank |

The parser compares only the first three identifier bytes with `OTA`, copies 24 descriptor bytes, and uses the first descriptor's size as its stride. `0x0e01afe4` copies the outer version; no comparison was found in the parse, write, checksum and activation chain.

A proposed profile uses version `0xffffffff`, count 1, identifier `OTA1`, size 24, payload offset 32 and destination `0xffffffff`. These are review constraints, not values stock is shown to enforce. A shared Realtek container doesn't carry over the PLAF103's payload limit or preservation list.

## Write, checksum and activation

Streaming writer `0x0e01b0b4` withholds the first eight payload bytes. Checksum helper `0x0e01a744` sums the staged bytes read back from flash plus the held bytes, and compares with the descriptor's unsigned byte-sum.

Activation helper `0x0e01a80c` writes the new eight-byte marker, then programs four bytes over the old bank's marker. Only the first source byte is explicitly zeroed locally, so don't require an exact four-byte zero pattern as evidence of this operation.

OTA worker `0x0e018990` reports success, waits 3,000 ms and requests a reset. Neither a progress report nor a valid marker proves a first boot or a rollback. The stock loader can loop on a valid KM0 and invalid KM4 pair, and doesn't retry the other bank after a marked-valid application crashes.

## Before an install

A candidate needs a fountain-specific size, erase and preservation review, evidence that the fountain accepts a custom image, and a verified recovery. Preserve the original loaders and calibration, and keep the bench tools' writer pins and marker and preservation checks exact; a rebuilt diagnostic is not automatically compatible with a pinned writer. The flash page's reference footprint is a starting bound for reviewing a build, not an approved install range. How large a payload the fountain takes and how an interrupted update recovers are also unknown ([still unknown](../flash-layout.md#still-unknown)).

## Evidence

- **Recovered, 2026-11-03:** Ghidra review of stock 0.0.17's request precheck, URL parser, HTTP parser, descriptor parser, mapping-based bank selection, streaming writer, checksum, marker activation and reset owner. No stock server was started, no update offered and no flash written for this analysis.
