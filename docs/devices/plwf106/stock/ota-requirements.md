# PLWF106 stock OTA contract

This reference describes stock 0.0.17's update request, HTTP transfer and activation. It supports an image/profile review; it is not an installation recipe. Read the [flash layout](../flash-layout.md) before reasoning about payload bounds or fallback.

## Request precheck

Handler `0x0e010fd4` requires `upgradeType`, `url`, `targetSoftwareVersion` and `msgId`. Types 2 and 3 bypass the same-version check. Other types reject an exact match to `0.0.17`; this is string comparison rather than semantic version ordering. Missing fields set OTA failure state.

The outer handler admits URL parsing for lengths at most 256. Inner parser `0x0e012cf4` copies at most 128 bytes into its working buffer. The outer limit is therefore not a safe transfer profile. A conservative reviewed candidate should use a whole ASCII URL shorter than 128 bytes, a short IPv4 host/path and an ordinary decimal port. The handler does not propagate every parsing failure as a refusal; do not probe malformed URLs on hardware.

## HTTP behavior

Despite the `https_ota` log tag, this transfer path opens a plain TCP socket and writes an HTTP request directly. A conservative profile uses HTTP, not inferred TLS support. Request formatting omits a non-default port from the `Host` field.

Parser `0x0e01ac50` accepts status 200 and has a separate 302 branch. It recognizes `Content-Length` and `CONTENT-LENGTH`; the caller rejects zero length. Use direct 200 with an exact length and unchanged image bytes when reviewing a future profile. Chunking, lowercase header names, compression and redirects are outside that conservative profile.

## Envelope and bank choice

Bank selector `0x0e01a3a0` inspects mapping registers; the destination is the opposite application bank. Descriptor parser `0x0e01a47c` replaces the envelope's destination with that selected address.

The recovered single-descriptor envelope has an eight-byte prefix and a 24-byte descriptor. Words are little-endian:

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

The parser compares only the first three identifier bytes with `OTA`, copies 24 descriptor bytes, and uses the first descriptor's size as its stride. Outer version is copied by `0x0e01afe4`; no comparison was found in the reviewed parse/write/checksum/activation chain.

A conservative proposed profile uses version `0xffffffff`, count 1, identifier `OTA1`, size 24, payload offset 32 and destination `0xffffffff`. These are review constraints, not a claim that stock enforces every value. Shared Realtek container structure does not authorize the PLAF103 payload limit or preservation list.

## Write, checksum and activation

Streaming writer `0x0e01b0b4` withholds the first eight payload bytes. Checksum helper `0x0e01a744` sums staged bytes read back from flash plus the held bytes, then compares with the descriptor's unsigned byte-sum.

Activation helper `0x0e01a80c` writes the new eight-byte marker, then programs four bytes over the old bank's marker. Only the first source byte is explicitly initialized to zero locally. Do not require an exact four-byte zero pattern as evidence of this stock operation.

OTA worker `0x0e018990` reports success, waits 3,000 ms and requests reset after the updater succeeds. Neither a progress report nor a valid marker proves first boot or rollback. The stock loader can loop on a valid KM0/invalid KM4 pair and does not retry another bank after a marked-valid application crashes.

## Unestablished installation requirements

A candidate needs a fountain-specific size/erase/preservation review, custom-image acceptance evidence and verified recovery. Preserve original loaders and calibration. Keep the existing reviewed bench tools' writer pins and marker/preservation checks exact. Rebuilding a diagnostic does not make it compatible with a pinned writer.

The reference footprint in the flash page is a starting bound for reviewing an actual build, not an approved install range. No stock server was started, no update offered and no flash written for this analysis.

## Still unknown

Whether the fountain accepts a custom image, how large a payload it can take, how an interrupted update recovers, and where a PLWF106 install and store may go.

## Evidence

- **Recovered, 2026-11-03:** Ghidra review of request precheck, URL parser, HTTP parser, descriptor parser, mapping-based bank selection, streaming writer, checksum, marker activation and reset owner in stock 0.0.17.
