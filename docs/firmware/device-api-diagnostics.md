# Diagnostics and the stock backup

An owner's feeder has no UART wired, so what an owner might inspect comes over the [device API](device-api.md): statistics, feed traces, the firmware's log lines and the stock firmware in bank 1. This page explains how the feeder keeps and serves each, for contributors and for owners curious about what `wellfed stats`, `trace`, `logs` and `backup` read. Anything stored goes through the settings writer under the write turn ([no flash write while the motor may move](device-api.md#no-flash-write-while-the-motor-may-move)).

## Statistics

The loop keeps the figures in RAM as it runs (`wellfed_feeder::stats::Keeper`): no task, no wait and no flash write, a few additions per pass. The lifetime totals are the settings writer's, in storage sectors 32–33.

**`Stats { section }`** answers one section in one frame, to any role that opens a session, except `Memory`, which is owners only. `wellfed stats [--section …]` prints each section as JSON. Without `--section` it asks for the sections the device's kind keeps.

| Section | Holds |
| --- | --- |
| `Counters` | Why this boot started (`ResetCause` from the ROM's reset flags: power-on, watchdog, software, brownout, other); this boot's uptime, counted past the millisecond clock's 49.7-day wrap; feeds, and those not done; confirmed portions; panel gestures; sessions; reconnects (link and check losses); DHCP renewals; outages and seconds on battery; the longest and mean handshake (accept to open) |
| `Memory` | The heap's free bytes and low-water mark, and each RTOS task's fewest free stack bytes (up to 16, with `tasks_total` the number the RTOS runs, so a truncated list shows) |
| `Flash` | Each storage sector's erases and bank 1's this boot, counted where every erase goes (`storage_erase` and `bank1_erase`), and the event ring's records kept against its capacity |
| `Power` | The adapter now, its changes and outages, time on battery, the battery's least and most, and its voltage at the end of each of the last 24 hours |
| `Food` | The food beam's last reading, its low and refill transitions, and feeds since a refill |
| `Feeds` | This boot's feeds as their traces measured them ([below](#feed-traces)) |
| `Portions` | Portions credited (one a mark), feeds that began with an uncredited homing mark, outlet-beam breaks during feeds (a lower bound), and the calibration's portion weight with the weight delivered by it ([calibration](calibration.md)) |

- **Memory's walk** (`uxTaskGetSystemState` and the name copies, with the scheduler paused) runs at most every 5 s, its result reused meanwhile. It never runs during a feed: the request is refused `Busy` while one is queued or runs.
- **Food is the latest reading.** The beam is read only while the emitters are on, during a feed or a [window at rest](optical-inputs.md#reading-the-food-beam-at-rest). So `Food::present` is `None` until the first reading of a boot (`Status.food` reads `Unread`), and then the latest. A window never blocks a feed or changes readiness.
- **Lifetime figures** (`Counters::lifetime`, `resets`, `Flash::lifetime`) are the stored totals plus this boot's figures.

**The store** (`wellfed_feeder::store::stats`, sectors 32–33 of the 48-sector region) is one 288-byte dual-slot record (`PLFSTA01`). It holds the ten counts, boots by reset cause, erases of every storage sector and of bank 1, and the early-save budget. It keeps all 48 sectors' erases, though the wire's `Erases` lists the first 32. Newest valid wins, so a cut save loses only that save. Its layout covers the region as it is, and growing the region again takes a new record version.

**At boot** the writer loads it, adds this boot to its reset cause (from the ROM's flags, as `Counters::reset`) and publishes it for the loop (`PLCFG1 stats open status= generation= early= missed=`). The loop hands this boot's counts to the writer every 30 s beside the signal sample, again as each feed ends, and just before it asks for a restart. The loop never writes.

**Saves** take the write turn like every store write, but never one forced past a claim's bound, so they never run while a feed holds the outputs. They come only when nothing else is queued, never per event (`PLCFG1 stats saved generation= early=`):

- **Early:** the boot's own as it starts, so a reset is counted however short the boot, and one in an intentional restart's grace (below). Each takes one of 6 tokens kept in the record. The save that takes the last sets a sticky flag (`missed=1`: a boot may have been missed), and later boots go uncounted until a settled save gives a token back.
- **Settled:** every hour of a boot's uptime, the first an hour in. Each gives one token back. A power cut loses at most the last hour of a boot.
- **After a feed:** once a feed has ended, at once for a boot's first and then at most every 10 minutes (`FEED_SAVE_MS`). It takes no token and gives none back, so a spent budget never costs a short boot its feeds.

**Wear.** Early saves never outnumber settled ones by more than 6, so the store erases at most 6 times plus twice per hour of uptime, whatever the crash pattern: about 17,500 erases a year, 8,800 of each sector. A save after a feed adds at most one erase per finished feed, and within a boot at most 6 an hour. That limit is per boot, because the time of the last save is in RAM: each boot's first counted feed saves at once. So a feed every 10 minutes all year adds about 26,300 of each sector (about 35,100 with the hourly bound), and three meals a day add about 550 of each sector a year. A cancelled or expired feed counts as a feed too, so a save can follow no motion.

**A restart's save** happens in the restart's grace. `wellfed_sdk::request_reset` (the device API's reboot, setup's restart) opens it (`supervision::begin_restart_grace`). The watchdog is still fed and writes still run. The writer saves (an early save) and says it is done, or has nothing to save. The runtime restarts then, or after 2 s (`RESTART_GRACE_MS`) if the save is held, say by a feed.

- Once the feeding stops, no write turn is granted, so no save starts into the reset. A cut write there would leave the arming marker set and the next boot without its stores.
- An update's activation or give-way restarts at once, without a grace, holding the write turn until the reset anyway.
- A supervision stop (a hung task, unverified outputs) saves nothing.
- Each save counts its own erase. A failed save isn't retried that boot.

**Lower bounds.** The lifetime counts and `resets` never exceed the truth. A power cut loses up to an hour of a boot's counts and 10 minutes of its feeds. While no token is left, a short boot's reset goes uncounted, and so does anything else it counted unless a feed's save took it along. The feed history records every feed, so the lifetime feed count can read under it. The `missed` flag that says so is in the log, not on the wire.

**`None`** comes from a build that keeps no totals, and from a store this build can't write (a newer record version, or a read error).

**The Wi-Fi task** reports two more events in `PLWIF3`: `kind=ap`, after each join's DHCP, with the channel and the BSSID's last four bytes; and `kind=renew`, a lease renewal. A renewal is seen as lwIP's lease clock going back while the client is bound. A release on expiry also zeroes it, and isn't counted.

The live metrics that go beside the statistics are in [samples](device-api-ops.md#samples).

## Feed traces

The feeder keeps what each feed's drive measured, poll by poll, in RAM and then in its own storage, so an owner can read it without a UART or a server: `wellfed trace <feed>` (`--json` for the whole trace). It is a feeder op (`FeederOp::FeedTrace`).

**Recorded after each owner step, never inside it.** The feeding owner keeps one `Observation` (`wellfed_core::feed::observe`) per poll of a feed being prepared or driven:

- the current channel (PB1, ADC channel 4);
- the battery pack in millivolts;
- the outlet and food beams, read only while the emitters are on;
- PA18's edge counts, and the latest edge's time, from the feedback interrupt;
- the controller's motor command.

The loop takes it once the step is over (`Owner::take_observation`, only from the task that steps the owner) and adds it to the trace (`wellfed_feeder::trace::Recorder`, held in `wellfed_feeder_runtime::traces`). So recording runs after the step's stop and motor decisions, with no task, no wait and no flash, and the motor lease and supervision timer never depend on it. The poll is the owner's own, so a reading comes about every 30 ms on the PLAF103 (903 polls in a 27.2 s drive), not every 10 ms.

**Format** (`wellfed_proto::trace`). A body of entries, each timed in milliseconds from the drive's start and coded against the one before:

- current readings, two bytes at a steady poll;
- marks: PA18 edges at the interrupt's millisecond, with the level after;
- beam changes;
- battery moves of 20 mV or more;
- motor command changes (`Drive0`, `Drive1`, `Stop`), which line up with the logic analyzer's D2 and D3.

The body is at most 864 bytes, and the whole recorder is under 1 KiB of RAM, a compile-time check.

**Overflow: decimated, then truncated.** A full body halves its current readings in place: the second of each pair goes, and the first reading, a drive's start-up surge, stays. From then on it keeps every second poll's reading (`stride` doubles). Marks, beams, battery and motor entries are always kept. Only once they alone fill the body does it end there (`truncated`). A one-portion feed keeps every poll, three portions every 4th (about 120 ms), and 48 portions every 64th or so, with every mark. The summary never depends on the body.

**Summary** (`TraceSummary`):

- the feed's result (requested, completed, outcome, reason);
- the owner's own figures, the same that `PLOWN1 fed` and `PLOWN1 current` log: drive time, polls, peak and mean current, rising and falling edges, the first edge's time;
- beam breaks: polls that found the outlet beam blocked after finding it clear;
- reversals: the owner's count of its reverse attempts on a jam, as `PLOWN1 current … reversals=` logs. A double bowl's turn between portions counts as none;
- the battery just before the drive, and its lowest during it;
- the stride and `truncated`.

Current is in stock's scaled unit (`CurrentUnit::Scaled`) until its scaling to milliamps is calibrated. A later build switches the unit without a wire change.

**Stored once the feed is over.**

- The loop seals the trace when it records the feed's `Fed` event.
- The settings writer appends it to the ring under the write turn, so never while a feed claims the outputs, and under the arming attempt marker like every store write (`PLCFG1 trace saved feed= seq=`, or `trace error feed=`).
- A sealed trace counts as a queued write, so the next feed's claim lets it go first (at most `WRITE_WAIT_MS`).
- The next feed takes the RAM trace back. One that never reached storage is counted (`unsaved`). A feed that starts while the writer is still appending goes untraced rather than wait.
- A feed a reset interrupted loses its trace with the RAM; its `Uncertain` `Fed` event remains.

**The ring** (`wellfed_device::store::traces`, storage sectors 29–31) holds four 1 KiB slots a sector.

- Each slot is a 96-byte header (`PLTR`, version, the lengths, a sequence number, the summary, a CRC-32 over header and body) programmed first, then the body. A cut before both are in leaves a slot that fails its checksum and is skipped.
- When the newest sector is full, the oldest is erased. So the ring keeps the last 8 to 12 traces, and each sector is erased once every 12 feeds.
- A power cut at any mutation of an append, or of its erase, loses at most the trace being written.

**`FeedTrace { feed, offset }`** (op `0x12 0x06`, the feeder module's seventh) answers `FeederOutcome::FeedTrace` (`0x09 0x02`): `TracePage { summary, stored, offset, total, bytes }`, with at most 432 bytes of body, so a three-portion trace is two pages.

- Any role may read it.
- The loop answers from the latest feed's trace in RAM, otherwise from the ring through mapped flash. A read takes no lease, and a lease holds every other task off the flash.
- `Busy` while that feed is still recorded, `NotFound` when neither holds it, `Invalid` for an offset past the body.
- Each page repeats the summary, and `wellfed_client::trace::Pages` refuses a trace that changed between pages rather than splice two.

**Statistics.** `Stats { section: Feeds }` (section `5`) sums this boot's traces: traced feeds, drive time, polls, peak and mean current, edges, beam breaks and reversals. It adds the ring's fill, the unsaved count and the latest summary.

**Cost.** About 1.4 KB of `.bss`: the recorder, under 1 KiB, and the page an answer borrows. Recording costs a few comparisons and a two-byte append a poll, and a halving one pass over under a kilobyte. A page read scans twelve slot headers and checks one trace's CRC from mapped flash.

## Debug log

An owner can read what the feeder's UART prints without opening the feeder: `wellfed logs` (`--follow` for new lines, `--after N` to resume).

**The tail.** Every record line goes through `wellfed_amebad_sys::log` (`wellfed_sdk::log`, the `Record` derive and the runtime's own lines). It prints the line as before, then copies it into a 16 KiB RAM ring, `LOG_TAIL` (`wellfed_core::log_tail`): about 280 lines of the usual 56 bytes.

- A Wi-Fi rejoin logs about 3 KB in its first 90 s, so a 4 KiB ring lost the lines from before an outage. 16 KiB holds several minutes of a rejoin's lines, so a feed's result lines survive a typical outage.
- The copy runs inside the print's scheduler suspension, after it, so the UART timing is unchanged and tasks never contend for the ring.
- Vendor C prints (`_rtl_printf`, `DiagPrintf`, the panic record) don't go through `log` and aren't in the tail.
- Nothing is written to flash. The tail starts empty each boot and numbers its lines from 1.

**Safe from an interrupt, never waiting.** A task logs with the scheduler suspended, so only an interrupt can find another writer mid-copy. It drops its line from the tail (the UART still prints it) and counts it, rather than wait for a writer that can't run until it returns. The dropped line still takes a number. No interrupt logs today.

- Readers never block writers. A reader copies a line, then checks the ring's oldest line hasn't moved past it (a seqlock), starting over a few times at most.
- A line too long for the ring (over 255 bytes; the record buffer is 192) is counted the same way.

**Overrun drops the oldest.** A line that needs room drops whole lines from the old end. A session whose cursor fell behind learns how many it missed: each batch's `dropped` counts the numbers after its cursor that the batch doesn't hold.

**Batches fit the frame, only when the session has room.** `TailLog { after }` is answered `Done`. `None` starts from the oldest line kept, and asking again moves the cursor. The session then pushes `Log { seq, dropped, text }`: as many whole lines as fit `LOG_TEXT_MAX` (992 bytes), each ending in `\n`, with `seq` the last line's number.

- Like events or a sample, a batch goes only when the session's last frame is out, after any events and sample waiting, one frame per pass. So a session that can't keep up costs only its own lines.
- Text is printable ASCII; any other byte reads `?`.
- A cursor past the newest line can only be an earlier boot's, and reads from the oldest. One below it can't be told from this boot's own, so a client resumes with a cursor only within one boot (the status's `boot_count`).
- Lines dropped after the newest written one show in `dropped` only once the next line is written.

**Ships in release images.** The `feeder` firmware keeps the tail in every build, not only behind a debug feature.

- The op's contract is text with no schema, so any later build can keep serving it from whatever it logs.
- An owner's feeder has no UART wired, so the tail is the only way to see a field fault's lines.
- It is owner-only, and no record carries a key, a secret or a credential (each record type's rule). The stream shows an owner nothing they couldn't already change.
- The other firmwares (`blink`, `bringup`, `qa`) serve no API and build without the `log-tail` feature, so they keep no tail.

**Cost.** `.bss` grows by 16,400 bytes (the ring and its state), which comes out of the heap, and flash-resident code by 1,920 bytes. The copy runs in its own 56-byte frame under `log`, and a batch reuses the server's buffers, so every task's stack bound stays within noise.

- Reading a batch walks the ring's line headers from the oldest line, copies once, then seals like any frame (about 0.9 ms a kilobyte). A pass with nothing new returns before the walk.
- The walk is about 280 headers in a full ring of usual lines, some tens of microseconds. Its worst case, a ring of the shortest lines, is about 3,300 headers, roughly 1 ms by static estimate, as long as the seal. Real records are longer, so a full ring holds far fewer. The reader runs with interrupts on, so the walk delays no writer.

## Stock backup

After stock installs the replacement into bank 0, bank 1 still holds stock, with only its marker's first word overwritten. Promotion would overwrite it, so the installer reads it first and keeps the owner's own copy ([rollback](consumer-installation.md#rollback)).

- **`ReadBank1 { offset }`** (op `0x11`) answers `Outcome::Bank1` (`0x08`): `Bank1Page { offset, total, bytes }`, with at most `BANK1_PAGE_MAX` (768) bytes. The bank's 1,376,256 bytes (`BANK1_BYTES`, stock's payload size) are 1,792 pages. A build that isn't the bank-0 role refuses it `Unavailable`.
- **Who and when.**
  - Owners only; a member or the server is refused `Forbidden`. The copy is the owner's to keep.
  - Read-only and not a remembered write, so any page may be asked again.
  - Served only from bank 0 (`Unavailable` from bank 1, whose own image is the running build). The SDK's read also refuses unless the flash MMU maps bank 0.
  - `Busy` while an install holds the flash, since it rewrites the bank under the read. `Invalid` at or past the bank's end.
- **How.** The loop copies the page through the mapped flash window (`sdk::bank1_read`, a sector at a time, taking no lease) into the 768-byte page buffer, which trace answers also borrow. Page answers aren't logged one by one, like update chunks.
- **Nothing vouches for the bytes.** `wellfed_client::install::StockBackup::from_bank` restores the marker's first word, checks the payload against the pinned 1.1.15 hash, and rebuilds the vendor's container, whose hash must match its own pin.

## Evidence

- **Modeled** (October 26, 2026): statistics, lifetime figures, feed traces, the debug log and the stock backup are host-tested. The trace ring and the stats store are tested under a power cut at every mutation. In simulation, `wellfed-feeder-sim`'s bank 0 served an owner the whole of bank 1 byte for byte over TCP, once bank 1 was left as stock leaves it, and bank 1 refused the read. The device conformance suite checks both platforms refuse it from bank 1.
- **Modeled** (October 26), build costs, from the `feeder` firstboot builds with every audit passing:
  - the debug log, against the same build without it: payload unchanged at 946,176 bytes; the loop's bound 5,140 → 5,108 of 8,192 (5,116 once rebased onto the SysTimer clock work); `wellfed` 2,808 → 2,808 of 4,096, `wellfed-key` 1,272 → 1,248 of 2,048, `wellfed-svc` 11,744 → 11,744 of 16,384, `wellfed-wdg` 1,452 → 1,420 of 2,048;
  - traces: `.bss` against `.ram_image2.bss` of 61,668 bytes in all, payload 995,328 bytes, the loop's bound 5,360 of 8,192 (its deepest path a feed's submission, not the trace), `wellfed-svc` at 11,744 of 16,384;
  - the stock backup: the page buffer grew from 432 to 768 bytes (336 bytes of `.bss`), payload 1,015,808 bytes, the loop's bound 5,412 of 8,192 (5,460 before the clock moved to the SysTimer).
- **Observed** (October 30), the stock backup: a replacement on bank 0 served bank 1 to the installer on the research unit, and the kept copy matched the pinned stock image.
