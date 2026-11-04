# UART backup and recovery

This page says what backing up the PLWF106 over UART has shown, and what a restore still needs. It is for developers. **Nothing on this page writes flash, and no writer accepts this model.**

## What has been shown

On our unit:

- entering download mode without the stock application ([UART](uart.md#enter-download-mode));
- `wellfed-flash read inspect-rom` against the pinned C-cut ROM;
- the first 4 KiB, read three times, matching;
- a full 8 MiB read, twice, matching byte for byte, with no stock boot in between.

Not shown: any write, a restore, a marker change, or recovery from a power cut.

## Take a backup

The steps are the PLAF103's ([recovery](../plaf103/recovery.md), [relay bench](../plaf103/bench.md)): enter download mode, run `read inspect-rom`, then `read small`, then `read full`. A full read takes about 16 minutes.

> [!NOTE]
> Keep your hands off the board during a full read. A bump to the wiring can stop it with "Changed or excessive duplicate packet". The read then refuses rather than retrying, and you start it again.

Take two full reads with no stock boot in between, and compare them. Stock rewrites its settings and log sectors while it runs, so reads with a boot between them differ there and only there. On our unit those were the sectors at `0x3e8000`, `0x3e9000`, `0x3ea000` and `0x3ff000`.

## What a restore needs

The PLAF103's writers are pinned to that feeder's baseline, so they refuse this model. A PLWF106 restore needs its own reviewed plan:

- a fountain flash layout and preservation map, starting from the [minimum preservation map](flash-layout.md#minimum-preservation-map);
- the factory weight calibration at `0x7ce000` and `0x7cf000` preserved, with nothing recalibrating it;
- a recovery story for stock's loader, which does not fall back from a crashing bank ([flash layout](flash-layout.md#loader-selection-and-failure-behavior));
- the pinned writer checks unchanged: marker sequencing, preservation and no automatic retry.

## Evidence

- **Observed, 2026-11-03:** on our unit, two full 8 MiB reads that match, three matching reads of the first 4 KiB, and a passing `inspect-rom`. One full read refused at 7.3 MB with a changed duplicate packet after the wiring was disturbed. A read taken after stock had run differed from the matching pair only in the sectors named above.
