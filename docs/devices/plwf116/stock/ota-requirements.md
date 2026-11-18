# PLWF116 stock OTA contract

Stock 0.3.62's update request, HTTP transfer and activation, for reviewing a PLWF116 image or installer. This is a static reference, not an installation procedure. Read the [image and flash reference](../flash-layout.md) before assigning write bounds.

## Request and version check

MQTT dispatcher `0x0e017bdc` routes `OTA_UPGRADE` to handler `0x0e016ea8`. It requires `upgradeType`, `url`, `targetSoftwareVersion` and `msgId`. A missing field records failure state 3 and requests a response.

Types 2 and 3, logged as enforce and silence, bypass the version comparison. Other values reject an exact string match with `0.3.62`. This is not semantic version ordering and does not establish downgrade protection.

The handler parses URLs at most 256 bytes long. A longer URL skips that assignment without necessarily refusing the request, so stale URL state is possible. Parser `0x0e019b28` copies only 128 bytes into its local working buffer. Neither the outer limit nor its success response guarantees the complete URL reached the downloader.

The URL parser recognizes five scheme prefixes, including HTTP and HTTPS, and gives them default ports. The downstream OTA transport still opens a plain TCP socket. Recognizing `https://` therefore does not establish TLS support; it can select port 443 and send plaintext there. A reviewed server needs a short ASCII HTTP URL, an explicit ordinary decimal port where needed, and no redirect.

## Power and task ownership

Wake helper `0x0e02655c` signals the OTA task and resumes it if needed. Worker `0x0e0265fc` waits for usable Wi-Fi, polling at one-second intervals, then acquires wake lock **28** before calling the downloader.

No adapter-presence, battery-percentage or pump-off admission check appears in this request-to-worker chain. The wake lock prevents the normal sleep path while downloading; it is not an energy-reserve check. The separate low-battery policy is on the [power page](../power.md).

On failure, the worker records state 3, requests reporting, waits 2,000 ms and releases lock 28. On success it records state 2, requests reporting, waits 3,000 ms and calls the reset helper. If reset returns, it calls it again at one-second intervals, up to 100 times. The success path relies on reset rather than releasing the lock.

## HTTP transfer

Connector `0x0e02c4a8` resolves the host and opens an IPv4 stream socket. Downloader `0x0e02c8d4` sends a raw HTTP/1.1 GET. Its `Host` field contains the host alone, without a non-default port. The resource is prefixed with `/` in the request template.

Response parser `0x0e02c1b0` accepts status 200 and has a separate 302 redirect path. It recognizes `Content-Length` and `CONTENT-LENGTH`; the caller rejects a zero length. The parser searches for the header terminator and retains a 32-byte suffix across receives. These implementation choices do not establish support for every valid HTTP header split.

The redirect path searches for `Location` or `LOCATION`, parses another host, port and resource, and restarts the connection. Its presence does not make redirect handling a suitable installer contract. A conservative transfer is a direct 200 response with an exact positive length and unchanged image bytes. Chunking, compression and lowercase header names are outside that contract.

## Supplied envelope

The supplied `PLWF116_OTA_000362.bin` has one descriptor, an eight-byte prefix and a 24-byte descriptor. Words are little-endian:

| File offset | Value | Meaning |
| --- | --- | --- |
| `0x00` | `0xffffffff` | Outer version word |
| `0x04` | 1 | Descriptor count |
| `0x08` | `OTA1` | Identifier bytes |
| `0x0c` | 24 | Descriptor size / stride |
| `0x10` | `0x066f5c35` | Unsigned sum of payload bytes |
| `0x14` | `0x109000` | Payload length, 1,085,440 bytes |
| `0x18` | 32 | Payload file offset |
| `0x1c` | `0xffffffff` | Destination placeholder |

The complete file is `0x109020` bytes. Summing its payload reproduces `0x066f5c35`. This checks file consistency, not authenticity or bootability.

Header receiver `0x0e02c544` copies the outer version and count and calculates the header length from the count and descriptor stride. Descriptor parser `0x0e02b9dc` compares only the first three identifier bytes with `OTA`, copies 24 bytes per descriptor, and replaces its destination with the chosen bank. No outer-version comparison was found in this transfer chain.

The supplied one-descriptor form fits the reviewed path. Do not infer safe handling of arbitrary descriptor counts or malformed sizes from these checks.

## Destination, programming and checksum

Bank selector `0x0e02b900` reads the flash-mapping registers. The destination table contains CPU flash addresses `0x08006000` and `0x08186000`, corresponding to physical offsets `0x006000` and `0x186000`. The downloader selects the opposite application bank.

Streaming writer `0x0e02c614` receives in 512-byte blocks, erases in 4 KiB units as it advances, and withholds each image's first eight bytes. It programs the remaining bytes through the flash wrapper. A descriptor's size and offset control the transfer; an OTA file is not proof that every byte outside its payload is available for a replacement.

Checksum helper `0x0e02bca4` reads the staged image back in blocks of at most 512 bytes. It adds every unsigned payload byte, including the eight held marker bytes, and compares the result with the descriptor's byte-sum. This is an unkeyed checksum, not a signature.

## Activation and reset

Activation helper `0x0e02bd6c` first writes the new image's eight held bytes. It then programs four bytes over the old bank's marker. Its local buffer explicitly initializes only the first byte to zero. Do not assume the other three bytes become zero, or use that pattern to identify a successful activation.

The order keeps the new marker absent during transfer, but does not establish power-loss recovery at every erase or program boundary. The OTA file contains no original unit bootloader. Its behavior after a bad image or interrupted activation therefore needs separate loader evidence.

A progress report, matching byte-sum, valid marker or success report does not prove the new application booted. No first-boot confirmation or automatic rollback contract was established by this OTA analysis.

## What an installer still needs

A PLWF116 installer needs its own preservation map, size limits, loader and secure-boot review, and recovery evidence. The PLWF106 and PLAF103 writer allowances do not authorize writes to this model. Factory identity, calibration, battery hardware and erase behavior must come from the selected unit, not the OTA payload.

## Evidence

- **Recovered, 2026-11-16:** static analysis of supplied stock 0.3.62 KM4 request dispatch, URL parsing, OTA task, TCP/HTTP receive, descriptor parsing, bank selection, streaming writes, readback checksum and marker activation. Key lengths, waits, lock number and marker writes were checked in instruction listings.
- **Recovered, 2026-11-16:** local parsing of the supplied envelope and independent recomputation of its unsigned payload byte-sum. No device was contacted, no update offered and no flash written.
