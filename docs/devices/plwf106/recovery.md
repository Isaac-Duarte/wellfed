# UART backup and recovery

Backing up the PLWF106 over UART, writing a test image into its bank 1, and putting stock back, for developers running a boot test on the fountain. The writers are pinned to our unit's backup, so **they are not general PLWF106 installers**: a backup of another fountain fails their hash checks.

## What has been shown

On our unit:

- entering download mode without the stock application ([UART](uart.md#enter-download-mode));
- `wellfed-flash read inspect-rom` against the pinned C-cut ROM;
- the first 4 KiB, read three times, matching;
- a full 8 MiB read, twice, matching byte for byte, with no stock boot in between.

Not shown on the fountain: any write, a give-way, a restore, or recovery from a power cut. The fountain writers have run only against a modeled helper on the host.

## Take a backup

The steps are the PLAF103's ([recovery](../plaf103/recovery.md), [relay bench](../plaf103/bench.md)): run `read inspect-rom` while stock runs, then enter download mode and run `read small --device plwf106`, then `read full --device plwf106`. A full read takes about 16 minutes. `read small`, `full` and `status` accept only a ROM report that names the fountain's stock console, so take a fresh `inspect-rom` while stock runs.

> [!NOTE]
> Keep your hands off the board during a full read. A bump to the wiring can stop it with "Changed or excessive duplicate packet". The read then refuses rather than retrying, and you start it again.

Take two full reads with no stock boot in between, and compare them. Stock rewrites its settings and log sectors while it runs, so reads with a boot between them differ there and only there: on our unit, the sectors at `0x3e8000`, `0x3e9000`, `0x3ea000` and `0x3ff000`.

## How a boot test stays recoverable

The loader tries bank 1, then bank 0 ([flash layout](flash-layout.md#loader-selection-and-failure-behavior)). Bank 1's marker is altered on our unit, so stock runs from bank 0. A boot test uses that order:

- **Install.** The test image goes into bank 1 only, marker last, so a stop part-way leaves bank 1 unbootable and stock still boots.
- **Give way.** If the test image boots but misbehaves, clearing bank 1's first byte sends the loader back to stock bank 0: one 1 KiB program, no erase.
- **Restore.** Stock recovery then puts the backup's bank 1 back exactly, altered marker included.

Nothing falls back on its own ([loader failure behavior](flash-layout.md#loader-selection-and-failure-behavior)): recovering is the manual path of download mode, a give-way and a restore. The ROM enters download mode before the flash loader runs, so the fountain stays reachable over UART. Nothing outside bank 1 is ever written ([what the writers allow](flash-layout.md#what-the-writers-allow)).

The writers keep every PLAF103 rule, with the fountain's own pins:

| Pin | The fountain's |
| --- | --- |
| Baseline | Our unit's 8 MiB backup, by SHA-256, and its bank 0 and bank 1 by their own hashes |
| ROM inspection | A report naming the PLWF106's stock console |
| Writable bytes | `0x186000..0x271000`, clear of every preserved range |
| Fallback for a give-way | The backup's bank 0, byte for byte. A `--fallback` review is refused |
| Plan kinds | `plwf106_bank1_development_update`, `plwf106_bank1_give_way`, `plwf106_original_bank1_recovery` |
| Image review | `plwf106_development_image_review` of a `plwf106` firstboot build |

Marker-last sequencing, the preservation checks and the no-retry stops are the PLAF103's code paths, unchanged. Stock recovery still refuses a bank 1 that can boot.

The status check before each erase and program decodes the registers as the PLAF103's GD25Q64E. The fountain's flash part is **Unknown**, so run `read status --device plwf106` first. A status that doesn't decode as unprotected refuses the write, and nothing clears it. The helper session has no reviewed way to read the part's JEDEC ID, so nothing pins the part itself.

An unknown part might also erase a whole 64 KiB block where a 4 KiB sector was asked for. Bank 1's first block, `0x180000..0x190000`, also holds the FTL pages at `0x182000..0x185000`. So the writers read `0x180000..0x186000` before any write: an update's inspection reads it and its plan binds the hash, and stock recovery takes it from the reviewed full read. After every erase in that block, and at every region check of an update, they read it back. It must match that hash, or the session stops there without retrying. The journal records each pass as `neighbors_verified`. The backup is no reference here: stock's FTL is live, and stock runs between the backup and a test.

To finish a stopped fountain update with `--interrupted`, also pass `--stopped-session` with the stopped session's output directory. Its reviewed plan must be this update's (same baseline, source and target), and its journal must open with that plan and not have verified. The new inspection must still read the bytes that plan bound, and the finish's plan records which plan it was checked against. If stock booted in between, its FTL has moved on and the finish refuses; go back to stock with a full read and stock recovery instead.

> [!WARNING]
> A stop naming bytes beside bank 1 (`NeighborsDiffer`) means the FTL may be damaged. No writer here rewrites it, so restoring it needs its own reviewed plan. A finish of that session refuses, and so does stock recovery while the FTL pages read erased. Don't boot stock until that review, since stock would act on a damaged FTL.

## Run a boot test

Every command takes `--device plwf106` and one helper session's evidence: `--baseline` (the pinned backup), `--rom-report`, `--small-report` and `--port`. Each writer runs `inspect`, `prepare`, then `execute` with the plan's SHA-256 ([Rust flash client](../../development/rust-flash.md#how-a-write-works)). The candidate is a fountain review from `wellfed-build review` ([boot test](boot-test.md#build-and-review-it)).

1. Run `wellfed-flash update --device plwf106 --help` with no device attached.
2. While stock runs, run `read inspect-rom`. Then enter download mode and run `read small --device plwf106`.
3. Run `wellfed-flash update --device plwf106 inspect --target <review>`. With no `--source`, the original bytes come from the backup, and bank 1 must still equal them.
4. Run `update prepare`, and check the plan: kind `plwf106_bank1_development_update`, bank 1, and sectors from `0x186000` up.

   > [!WARNING]
   > The next step rewrites bank 1. Keep independent copies of both full backups, and keep the fountain powered until the run ends.

5. Run `update execute` with that plan hash.
6. Release the strap and power the fountain on. Watch the console.

To replace one test image with another, pass the installed one's review as `--source`.

## Go back to stock

1. Enter download mode, then run `read small --device plwf106` with the ROM report taken while stock ran. The test image's console is not stock's, so `inspect-rom` cannot run on it.
2. Run `wellfed-flash give-way --device plwf106 inspect`, with no `--fallback`. It checks that bank 0 is exactly the backup's. If an install stopped before its marker, bank 1 cannot boot and there is nothing to give way: skip to step 5.
3. Run `give-way prepare`, check the plan, then `give-way execute`.
4. Power the fountain on with the strap released. Stock should boot from bank 0.
5. To restore bank 1 exactly, enter download mode again and run `read small --device plwf106`, then `read full --device plwf106`.
6. Run `wellfed-flash stock-recovery --device plwf106 prepare --current <full read>`. Check the plan: kind `plwf106_original_bank1_recovery`, sectors inside `0x186000..0x271000`, bank 1's first sector last.

   > [!WARNING]
   > The next step rewrites bank 1. Keep the fountain powered until the run ends.

7. Run `stock-recovery execute` with that plan hash. It reads all 8 MiB three times, so allow about an hour.

The boot log holds the unit's serial number, product secret and saved Wi-Fi password. Keep captures under `private/`.

## Common mistakes

- **Naming `--bank 0`.** The fountain has no bank-0 writes, and the refusal comes before the port opens.
- **Restoring while bank 1 can still boot.** Stock recovery refuses it. Give way first.
- **Passing the feeder's backup or plans.** They hash to the PLAF103's pins and are refused.

## Next steps

- [Flash layout](flash-layout.md): the banks and what the writers preserve.
- [Rust flash client](../../development/rust-flash.md): the plan, prepare and execute flow.

## Evidence

- **Observed, 2026-11-03:** on our unit, two full 8 MiB reads that match, three matching reads of the first 4 KiB, and a passing `inspect-rom`. One full read refused at 7.3 MB with a changed duplicate packet after the wiring was disturbed. A read taken after stock had run differed from the matching pair only in the sectors named above.
- **Modeled, 2026-11-04:** `cargo test -p wellfed-flash` installed a synthetic image into a synthetic fountain's bank 1, gave way and restored bank 1, changing nothing outside `0x186000..0x271000`. It refused bank 0, an image past the cap, a frame in a preserved range, another backup and the feeder's layouts. Simulation only.
