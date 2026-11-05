# Resource budget

This page measures whether the Wellfed product fits on the PLAF103, and what gives way where it doesn't: flash, RAM, tasks, the loop's latency, how Wi-Fi stalls the scheduler, and the storage region. It is for contributors deciding where new work can go. The design it measures is in [product architecture](product-architecture.md).

## The short answer

It fits.

- **Flash:** the current `feeder` image uses about three quarters of a bank, with room for what's still planned.
- **Heap:** about 120 KB free in Bluetooth setup mode, the tightest case.
- **Tasks:** none spare, by design. Everything new joins the loop task.
- **Latency:** three rules keep the loop responsive: one handshake operation per pass, feeds and Wi-Fi joins take turns, and logging never holds the scheduler.
- **Storage:** a 48-sector region with 3 sectors still free; the meal call's clip takes the last 8.

## Flash

Each OTA bank holds 1,376,256 bytes, stock 1.1.15's own size (`layout.json`).

| Image | Payload | Of a bank |
| --- | ---: | ---: |
| `blink`, the smallest firmware | 684,032 | 50 % |
| `feeder` with the panel only | 684,032 | 50 % |
| `feeder` with Noise and the session table, BLE linked out | 720,896 | 52 % |
| `feeder` with BLE reachable | 868,352 | 63 % |
| `feeder` with the device API, mDNS and the event ring | 942,080 | 68 % |
| `feeder` with traces and the stock backup | 1,015,808 | 74 % |

- **BLE:** about 147 KB, which the product always carries, because any boot can enter setup mode.
- **Feeding owner:** about 12,288 bytes, 1 % of a bank, costed from the bring-up firmware's `motor` and `pulse` images before it joined the product.
- **mDNS:** the vendor library would have cost about 33,500 bytes of code. The product's own responder replaced it ([mDNS](#mdns)).
- **Crypto:** the largest pieces are SHA-512 for update signatures (21 KB) and curve25519's field arithmetic.
- **Headroom:** with BLE, the first estimate left about 508 KB, and put the rest of the product (feeding owner, API protocol, schedules, alerts, pairing, mDNS, NTP) at 80–140 KB.

## RAM

KM4 static sections, in bytes, from the first product image:

| Section | `blink` | `feeder` normal | `feeder` setup |
| --- | ---: | ---: | ---: |
| `.ram_image2.text` (code in RAM) | 98,472 | 98,488 | 98,608 |
| `.ram_image2.data` | 12,936 | 24,616 | 25,792 |
| `.ram_image2.bss` | 37,576 | 37,616 | 41,028 |
| `.ram_image2.nocache.data` | 23,928 | 23,920 | 23,932 |

The session table is part of the feeder's state, the `FEEDER` static in the firmware's `app.rs`. It is built at compile time (`Slot::filled`) because it is larger than the loop task's stack.

**Heap.** Free bytes of the FreeRTOS heap, from `PLAPI1 heap`. Task stacks come from this heap.

| Mode | At boot | Steady, free | Steady, least ever |
| --- | ---: | ---: | ---: |
| Normal | 271,776 | 146,464 | 141,792 |
| BLE setup mode | 267,040 | 131,904 | 123,136 |
| Normal, with the device API | — | 125,088 | — |

- **No leak:** the normal image held 146,464 free for three more minutes.
- **BLE:** setup mode costs about 15–19 KB of heap (`PLBLE1 ready heap_free=135136`).
- **Headroom:** about 120 KB as measured, with the four-client table already counted. Since then the [log ring](#logging) takes 4 KiB of it, and the device API's log tail growing from 4 KiB to 16 KiB takes 12 KiB more. lwIP's own pools are separate ([clients](#clients-request-ids-and-buffers)).
- **The device API's cost** (146,464 down to 125,088) is not yet attributed.

## Tasks

A firmware gets four tasks (`FIRMWARE_TASKS`). The product uses two:

| Task | Stack | Audited bound |
| --- | --- | --- |
| `wellfed-key`, the button service | 2 KiB | 1,264 B |
| `wellfed-loop`, the loop | 8 KiB (2,048 words) | 5,008 B in the latest `firstboot` feeder build ([history](#evidence)) |

- Both bounds include the formatting the task can reach. The audit charges it, since it bounds indirect calls by class.
- The loop's deepest path is setup mode's handshake over BLE, down to X25519: `Feeder::pass > owner_stage > Owner::step > Setup::step > Setup::message > Responder::receive > read_message`. The device API took the bound past a 6 KiB stack's 25 % margin; the feeding owner's step stays below it.
- **The loop's headroom.** The audit refuses a bound above 75 % of the stack: 6,144 B of 8 KiB. At 5,008 B the loop has 1,136 B left. A feature that deepens the deepest path spends that, and the build then fails the audit rather than overflowing, so it cannot slip through unnoticed.
- **`Stats` requests.** Below `Server::request`, `Device::stats` costs 960 B, down from 1,368 B. Lifetime erases add the two arrays directly, with no `Totals` sum, and the Memory, Flash and Counters sections each have a function of their own. Memory is the deepest at 904 B, through its panic path. Flash, 864 B, still holds the stored `Totals`.
- **Test variants** add a third task: `gap-probe` adds `wellfed-gap` (256 words at priority 4) for the [scheduling-gap measurement](scheduling-gap-check.md), and `held-stall` adds `wellfed-hld` (256 words at priority 3) for the reset-storm drill.

**What keeps the loop's bound down.** Each of these lowered it (static analysis of the linked image):

| Change | Effect |
| --- | --- |
| The owner's `feeding::Record` formats by reference | Formatting's charge falls from 1,812 B to 1,248 B, because a copied trace was in that method's frame at each of the audit's nesting levels |
| `Feeder::pass` becomes a tail over `owner_stage`, `local_stage` and `speaker_stage` | Its frame under the device API falls from 712 B to 192 B |
| `Serve::step` moves its work into `before_server` and `after_server` | Its frame falls from 384 B to 304 B |
| `answer` hands off to `seal_answer` | The client list and the request's line no longer sit under the 1,272 B frame that holds a wire message |
| Noise public keys come from the X25519 ladder, not `PublicKey::from` | Avoids that function's Edwards multiplication and its 1.9 KiB frame |

**What the audit reads.** Besides the usual frame patterns, it counts Thumb-2 `subw sp, sp, #imm`, which LLVM uses for frames up to 4 KiB. It takes conditional direct calls (`blne`, `bne.w`) as edges, and a prologue slot saved as `str.w fp, [sp, #-4]!` as frame. Every firmware and variant builds under it, the deepest being `entropy` at 5,152 B.

**Outside the audit.** The stack audit takes its roots from the firmware's `spawn` calls, so C-created tasks are outside it:

- The C Wi-Fi task (`wellfed_wifi`, 1,280 words, created by `example_entry.c`). The audit bounds neither the shim's C nor the Rust it reaches through `wellfed_wifi_select`, `wellfed_wifi_outcome` and `wellfed_wifi_watch`. That Rust is the credential choice, a flash-mapped read of the stored credentials, the interlock's atomics and `vTaskDelay` while a join waits, and the link's liveness machine. Nothing on that path formats.
- The BLE stack's own C tasks, which cost heap. The setup-mode heap figure includes them.
- The two print wraps and the fault handler's call into the log ring ([logging](#logging)). They run on the calling C task's stack or the handler's: 24 B for the wrap, 40 B for the send.

## Handshakes and loop latency

The loop runs one Noise operation (one side writing or reading one message) per step. Timed on the 32 kHz system timer, worst of three runs each:

| | XX (pairing) | KK (reconnect) |
| --- | ---: | ---: |
| Messages | 3 | 2 |
| Initiator, all operations | 42–55 ms | 56–122 ms |
| Responder, all operations | 42 ms | 56–72 ms |
| Longest single operation | 28 ms | 45–77 ms |
| 1 KiB frame, sealed and opened each way | 3.4 ms | 3.4 ms |

- **X25519:** one operation costs about 25–28 ms on the KM4. Generating each side's ephemeral key adds one more, inside `Handshake::new`, which the runs built outside the timing.
- **Spread:** the 77 ms outlier did the same work as a 45 ms run. Other tasks (Wi-Fi, and then the MQTT task) preempted the loop by tens of milliseconds.
- **Per handshake:** the feeder spends about 80–110 ms of CPU, spread over two or three loop steps.
- **Transport messages are cheap:** about 0.9 ms to seal or open a kilobyte.

Sessions now use Noise IK rather than KK ([device API](device-api.md)).

**One handshake operation per pass.** One operation blocks the loop for up to about 80 ms. So the loop does at most one per pass, across all clients, and four clients connecting at once cannot stack up to 320 ms.

During a feed, only IK session handshakes go on, one operation a pass, and the owner is stepped again right after each such pass. A refused one closes unanswered and holds the rest until the feed ends; invites and recovery wait. The owner's observation gap (100 ms) bounds a pass during a drive. An operation (one or two X25519s, about 55 ms with no preemption) fits it, with the owner stepped on both sides ([device API](device-api.md#in-the-firmware)). `PLOWN1 steps` measures the real gap per feed.

The motor's stop guarantees never depend on loop timing. They stay with the supervision and hardware timers.

**The owner at rest.** The owner polls every 20 ms in every state. Preparing and Feeding convert both ADC channels on every poll. `Idle` converts only the battery, every 100 ms (`rest_capture_ms`), and `Terminal` converts nothing. A capture of both channels takes up to 10.1 ms ([bring-up probes](bring-up-probes.md)), so at rest about one poll in five spends about 5 ms, where every poll once spent about 10 ms. That is an estimate: pass times before and after are unmeasured.

Nothing that protects the feeder waits for a conversion:

| What | Why it holds at rest |
| --- | --- |
| The owner's 200 ms lease, and its beat | Renewed by the poll, conversion or not |
| The sensors' `max_age_ms` and observation gap | Timed from the poll's GPIO read |
| The pack's `hold_ms` | Timed on the clock, not counted. A sag takes up to one conversion longer to judge at rest, and a poll with no conversion is not a failed one |
| Motor current | Judged only while a feed runs (`input_refusal`, `update_captured`), and a feed's every poll converts it. No check at rest: a feed's first polls catch a bad reading before any motion |
| A converter that stopped | Found by the next battery conversion, within 100 ms, and held as lost sensors |

A gap past the observation limit drops the battery's last reading, so the next poll converts it again whatever its period says.

**Waiting is not work.** The server reports work only when the next pass has some to do, and the loop then passes after 1 ms instead of 20 ms. A session waiting for its client to read, for the writer to reach a store write, or for a Dispense press reports none, so the 20 ms poll looks at it. Bytes that stop moving do not hold a slot: a connection whose queued bytes have not moved for 30 s (`STALL_MS`) is dropped, one already ending included, and its client hears what any lost connection means ([device API](device-api.md#sessions-that-stop-reading)). These figures are static analysis until the bench measures passes per second with a paused client.

## Logging

`sdk::log` copies a record line into a RAM ring (`wellfed_core::log_ring`) and returns, rather than holding the scheduler for the up to 16 ms a line takes at 115200 baud. In quiet windows that took the probe's `p4 max` from 12–19 ms to 10–13 ms against its 10 ms period (physical, [evidence](#evidence)). The other figures here are static analysis, or modeled from captures.

**Who sends.** The diagnostic task (priority 1, 4 KiB stack), between its Wi-Fi probe steps. Each pass puts up to 8 bytes in the LOGUART FIFO if it reads empty, then sleeps 1 ms while lines wait, up to 10 ms when none do. That is about 8 B/ms against the wire's 11.5, so a 180-byte line takes about 23 ms from first byte to last. The FIFO's depth of 16 comes from the vendor's Bluetooth UART driver, which fills the same UART block 16 bytes at a time, and is unmeasured on the log UART; a burst of 8 leaves room for a vendor print caught between its check and its write. The task sits under the loop and the service task, so sending costs them nothing, and the ring absorbs what they log while they hold it off. With the loop at 1 ms passes it gets one burst per pass, about 3–4 B/ms.

**Boot.** Until the task's first pass after the start hook, or for a whole boot that never reaches it (halted, or a failed startup), `log` sends its own line before returning, as a preemptible spin.

**Size.** 4 KiB of `.bss` on every firmware, taken from the heap; `.ram_image2.bss` went from 66,304 to 70,432 bytes in the first image with it. The feeder's 16 KiB log tail holds the same lines for the device API, but can't serve as the ring: it drops its oldest lines first, which could be the one being sent.

Replaying the timed UART bytes of 1,116 recent captures through a model of the ring, boot's synchronous part and crash dumps left out (modeled, October 31):

| Backlog | Median | 95 % | 99 % | Worst |
| --- | ---: | ---: | ---: | ---: |
| At the full rate, 8 B/ms | 112 B | 496 B | 514 B | 2,847 B |
| At a quarter of it, 2 B/ms | 124 B | 625 B | 771 B | 6,117 B |

The worst figures are a hang's call trail, which goes out before the scheduler starts, so the ring never queues it; the model cannot tell. The wire limits the arrivals, so a real burst is sharper than a capture shows. The steady rate is 272 B/s at most over any capture longer than a minute, and a feed's end is about 470 B.

**A full ring.** A line goes in whole or not at all, so lines stay whole and in order, and what drops is the new line.

- `PLAPI1` lines drop, being the one record a client's request rate can drive. So does a line logged from an interrupt or with interrupts masked, or one too long for the ring. The next line, or an empty ring, brings `PLLOG1 dropped=N` first, where the hole is. The log tail keeps them all.
- Every other line waits in its caller for room, sending as it goes: usually one line's time, with other tasks still running, so evidence costs its own task's time, never the system's.
- That wait ends after a ring's worth of bursts, about 0.4 s, in case another logger at the same priority or above keeps taking the room; the service task's 10 s liveness limit is why. The line then drops too, reported through `PLLOG1`.
- A `call-chunk` or `update-chunk` refused `Busy` logs no line, since a client sends it again at once.

**Crashes.** The lines still queued go out first, in order:

| Path | What sends them |
| --- | --- |
| Hard, bus, usage or memory fault | The vendor handler, patched to flush before its first print |
| Panic | The panic handler, after it stops the outputs and before `PLPNC1` |
| A certain reset: the runtime restarting, a stop that outlasted the progress gap, or a refused checkpoint | The watchdog feeder (priority 7), every pass until the reset |

A supervision stop shorter than the progress gap is none of these: feeding resumes, so its lines go to the drain. A flush there would hold every task, and its time would count towards the checkpoint gap. A certain reset has the time: certain at about 1 s, 0.36 s for a full ring, and an expiry of at least 2,426 ms. A send is one masked burst that leaves the ring consistent, so these paths can take over from one another with no lock to find held, and a fault mid-copy loses only that unpublished line.

Queued lines are still lost to a hang with interrupts masked or above priority 7 (which stops the feeder too), and to a power cut or brownout: usually under 25 ms of lines, where direct printing lost at most the line in progress. The ring is not kept across a reset. A hang's crumb and call trail bypass it ([startup diagnostics](startup-diagnostics.md)).

**Masked time** (estimates from the disassembly). Copying a line takes about 8 instructions a byte: about 1,500 instructions, or 8 µs at 200 MHz without XIP cache misses, for a 191-byte line, and about the same for the log tail's copy in its own section. A burst is a status read and at most nine register writes, about 2 µs.

**Vendor prints.** The Wi-Fi driver and the SDK print from their own tasks, about 34 lines per join, spinning on the UART. The firmware's own C ports print nothing on KM4. Above the drain's priority, such a print could land inside a partly sent line and spoil both, about once per join by a model of the same captures. So the KM4 link wraps `DiagPrintf` and `_rtl_printf`: a wrapped print first lets the line on the wire finish (at worst one line's time, spinning at its own priority). Lines queued behind it stay queued, so a vendor line can come out ahead of an earlier logged one. Some prints can still split a line:

- a print from inside the ROM, or from the object that defines the print, which is not wrapped;
- a print from an interrupt or with interrupts masked, which does not wait, since up to 16 ms masked costs more than a rare split line;
- a task at the drain's own priority, idle+1, such as the BLE task (`wellfed_ble.c`, setup mode), since a tick can switch to the drain mid-print.

KM0 prints only at boot, before KM4 logs through the ring.

**Stack.** Only `wellfed-halt`'s bound rose, by 8 B of a 2 KiB stack, through `log`'s frame. The audit charges a panic's depth under each nested formatting call, four times over, so the send is inlined into `log_flush`, and the panic line calls the ROM's print directly.

## Scheduling gaps across Wi-Fi rejoins

**A Wi-Fi association holds every task, not just the loop.** The feeding owner is stepped from the loop task (`wellfed-loop`, priority idle+2), and its beat is renewed only when the loop runs. The WPA3 SAE handshake once held the loop past the 1 s gap the beat promises while outputs may move. (At rest the beat now promises 10 s.) A feed's beat could stay inside 1 s two ways: move the owner to its own task at priority 4, above the Wi-Fi shim's task (idle+3), or hold Wi-Fi joins during a feed. A measurement with the [`gap-probe` variant](scheduling-gap-check.md), with no motor motion, settled it.

### The result

- In each window where a join's association ran, the priority-4 task was late by 444–450 ms, and the loop by 443–474 ms: the same stall, within a pass's run time.
- Windows with only a scan or DHCP stayed at 3–17 ms for the probe, and 28–29 ms for the loop (its own pass).
- An association rejected with status 53 (stale PMKID) held both for about 720 ms (`worst` 724 and 741 ms).

**So priority 4 does not shorten the gap.** A priority-4 feeding task would see the same 0.45–0.72 s. The probe sat below the vendor driver's threads, though: the receive and transmit tasklets run at 5, the interrupt and command threads at 6, and the SAE work runs in the receive tasklet with interrupts unmasked (static, October 29). So the measurement could not tell masked interrupts from a thread at 5. A task at 7 is not held: the watchdog feeder moved there and the stall stopped reaching it ([OTA design](ota-design.md#boot-confirmation-and-fallback)). The loop and the feeding owner stay at 2, so the design is the interlock below.

**One hard fault in the loop.** Right after the status-53 rejection and its 720 ms stall, the 8 KiB `wellfed-loop` faulted with UNDEFINSTR (`cfsr=10000`) at `0x0e010ffe`. That address holds a valid `mrs r0, IPSR` in `unscheduled` (from `keys::identity_since`), executing from XIP flash. The board reset and came back on bank 1. A valid instruction faulting as undefined points to a bad fetch from flash, not a code bug, but one capture cannot establish the cause. It is the same class as the bank-0 download faults that stopped once the flash clock was lowered to 50 MHz ([OTA design](ota-design.md#bank-0-download-faults)).

### The interlock

`wellfed_hal::interlock` holds whose turn it is, in one atomic the loop and the C Wi-Fi task share, with no change to the C shim.

- The loop claims the outputs before it submits a feed. It releases them once no feed is in flight and the owner is at rest, with motor STOP and the emitters off.
- The shim calls `wellfed_wifi_select` before each attempt and `wellfed_wifi_outcome` after it. The select takes the join's turn and the outcome ends it, so the turn covers the scan, the association and DHCP.
- Battery mode's radio changes take the same turn: the leave with `wifi_off`, and each `wifi_on` (`wellfed_wifi_radio_begin` and `wellfed_wifi_radio_end`). They stall the driver as an association does ([battery mode](power.md#battery-mode)).
- Watching a joined link takes no turn. That watch no longer leaves a network that has no internet, so such a network costs no rejoin, and no refused feed, every few seconds ([link liveness](startup-diagnostics.md#link-liveness-without-internet-dns)).

How each case plays out:

- **A feed during a join** is refused `Busy` (`PLOWN1 feed portions=… held=1`). Clients treat that as final for that write id.
- **A join during a feed** waits in the Wi-Fi task, looking again every 50 ms, and marks itself waiting. A second feed is then refused (`held=2`), and the release hands the turn straight to the join, so back-to-back feeds cannot starve it.
- **Joins cannot starve feeds.** An attempt ends within the shim's 10 s scan wait, plus one association and one DHCP run, and attempts back off at least 1 s. Only the vendor SDK's own timeouts inside `wifi_connect` and `LwIP_DHCP`, prebuilt and not in this repository, bound the association and DHCP runs. So the loop counts a join that holds its turn past 60 s (`LONG_JOIN_MS`, logged as `PLOWN1 joins … long_joins=`). Feeds stay refused while it does.
- **No deadlock.** A join waits at most the claim's bound, `FEED_BOUND_MS`: the owner's `optics_max_on_ms` plus 2 s, which is 580 s for 48 portions. (The owner's configuration check holds the prepare timeout and the longest drive inside `optics_max_on_ms`.) Past the bound the join goes ahead and is counted as forced (`PLOWN1 joins … forced=`). A loop that stops stepping during a feed resets through the beat long before then.
- **Storage writes take a turn too.** A flash lease while the outputs may move fails its stopped-outputs check and resets the board. So every store write outside the feed journal takes the write turn (`WriteTurn::begin`) for one record, granted only while no feed holds the outputs ([device API](device-api.md#no-flash-write-while-the-motor-may-move)). A claim waits at most `WRITE_WAIT_MS` for writes: 3 s, one record's three leases at their whole budget plus `WRITER_POLL_MS` (500 ms), the slowest writer poll in the service task. A write still running, or a client's change still in its mailbox, then refuses the feed (`held=3`). Writes and joins do not wait for each other.
- **The cost.** Feeds are refused for a whole attempt, scan and DHCP included, though only the association stalls. While the network is away, that is the scan (up to 10 s) every backoff (1–30 s). Narrowing the turn to the association alone would need a shim call between scan and connect.

## Clients, request IDs and buffers

- **Client limit: 4.** lwIP allows 10 TCP connections and 5 listeners (`lwipopts.h`). An update download and the API listener each hold one. Four clients leaves room to refuse a fifth.
- **Request-ID ring: 32 per client** (512 B). A client retries a lost answer within seconds, and 32 outstanding or recent requests cover any realistic burst. A feed's ID also goes into its journal reservation, so the ring is not what stops a double feed.
- **Frame: 1 KiB.** That covers every planned request and event batch. Firmware images move in 512-byte chunks.

## Event history (7 days)

- **Record:** 32 bytes: sequence, the ring's epoch, time, a 16-byte body (which carries the kind and source) and a CRC (`wellfed_device::store::events`).
- **Rate:** about 100 events a day in normal use: feeds, button gestures, lock changes, settings, boots and hourly clock-sync summaries. Samples are never stored. We plan for 200 a day.
- **Size:** 7 days at 200 a day is 1,400 records, 44.8 KB. That is 11 sectors, plus one being erased: **12 sectors**, holding 1,408 records.
- **Wear:** each sector is erased about every 8 days, about 50 times a year, far inside NOR endurance.

## The storage region

The replacement's storage lives at 0x340000–0x370000: 48 sectors of 4 KiB. The full sector map is in [pairing](pairing.md#storage-sectors).

**Why it grew.** The original region was 16 sectors (0x340000–0x350000). The journal (sectors 0–1), settings (2–3), confirmation (4) and Wi-Fi (5–6) left 9 free sectors, 36 KB. Client keys and schedules needed a few of those too, so a 12-sector event ring did not fit, and 16 sectors would have meant about 3 days of history. So the owner extended the region to 32 sectors (0x340000–0x360000), then to 48 (0x340000–0x370000) for the lifetime statistics.

**Why that space is safe to use.**

- A static pass over stock 1.1.15 found no fixed or bounded computed flash access in 0x306000–0x3e8000.
- All 15 full dumps of the research unit are blank there, nine of them from before the replacement ran.
- The area is headroom left by the vendor's shifted SDK layout. Only an oversized OTA image (over about 1.79 MB) or the UART console's `FLASH` command could reach it.
- 0x370000 is still 480 KiB below stock's settings at 0x3e8000.
- The region grows only upward, so bank 1's room is unchanged.

**Stock's clock block.** The same pass found that stock's clock checkpoint save block-erases **0x3f0000–0x400000**, 64 KiB at a time, at minute 30 and on save. It is in `layout.json`'s preserve ranges (`clock_checkpoint_block`). Nothing of ours is near it.

**Statistics wear.** The lifetime statistics in sectors 32–33 erase each sector about 8,800 times a year from their hourly and boot saves under any crash pattern, about 35,100 in the worst case with saves after feeds, and about 550 more at three meals a day ([statistics](device-api-diagnostics.md#statistics)).

## mDNS

The vendor responder is 33.5 KB of code and 2.3 KB of BSS, and creates its own FreeRTOS task, outside the stack audit. The feeder needs only its own advertisement: a PTR, SRV, TXT and A answer for one service. So the device API has its own responder in the loop task, on one UDP socket: a few KB and no task ([device API](device-api.md#mdns)).

## Evidence

- **Observed** (October 23, 2026), the first product budget: the `feeder` firmware installed over the air as `fw44-budget-02` (v95, normal) and `fw44-setup-01` (v96, BLE setup mode from boot), each confirmed, then the normal build restored (`fw44-budget-04`, v97). The steady heap held for three more minutes in `fw44-budget-03`. Sizes come from the builds' manifests and ELF sections; heap and handshake times from the feeder's `PLAPI1` records. Nothing was run by the owner. The feeding owner, the vendor mDNS library and the device API protocol were not linked then, and were costed from other builds.
- **Observed** (October 23): the first product build overflowed the loop's then 4 KiB stack (`cfsr=100000`, STKOF) on every trial boot, and KM0 rolled it back (`fw44-budget-01`, v94). The audit had passed it because its frame pattern skipped `subw sp`. Fixing that also needed the runtime's `wellfed-net` task raised to 2,048 words (bound 5,488 B), a task since deleted with the MQTT transport, and the X25519 ladder for Noise public keys.
- **Modeled** (October 25–26, image audits): the loop's stack bounds of 4,784 B (the feeding owner and the join interlock) and 5,412 B (traces and the stock backup, [device API](device-api-diagnostics.md#stock-backup)); 5,852 B (October 28, battery mode, on the desiccant reminder); and the payloads of the later builds. These bounds predate the audit's conditional-call and prologue readers.
- **Modeled** (October 31, image audits of the `firstboot` feeder, none of them installed): the shallower loop frames. `wellfed-loop` went from 5,816 B to 4,984 B and its formatting charge from 1,812 B to 1,196 B, read by the same audit. Counting conditional calls (17 `blne` and `bne.w` edges in the feeder image) and writeback prologue slots (4 B short each) gives 5,008 B and 1,248 B, and `entropy` 5,152 B. `wellfed-svc` stayed at 11,752 B throughout.
- **Observed** (October 24): the device API's figures, from the `feeder` builds' manifest audits and `PLAPI1` records ([device API](device-api.md#resources)).
- **Observed** (October 25), the scheduling gaps: `gap-probe` v170 (`2b0806ce…`) on bank 1, six 180 s captures (`ar9-gap-20261025-cap-01`…`06`) while the hotspot was cycled ten times (60 s up, 15 s down). That gave 86 complete windows, 20 with a rejoin, across two boots. The hard fault was in `cap-04`. Bank 1 was then restored to the reviewed `e43324be…` (v171, `ar9-restore-20261025-01`). These runs predate the log ring, when logging held the scheduler up to 16 ms a line, so a `p4 max` of 10–40 ms in quiet windows was logging, not the radio.
- **Observed** (October 25): the WPA3 SAE handshake holding the loop past 1 s, in `fw16-stall-01`.
- **Modeled** (October 31): the log ring's whole lines, wrap, drop count and report are host-tested; its bounds and sections come from the image audits, and its backlog from replaying captures.
- **Observed** (October 31): the log ring and the shallower feed claim on hardware, hopper empty, on the adapter. `gap-probe` before and after the ring, about 5 minutes of 10 s windows each: `p4 max` 12–19 ms to 10–13 ms, and the boot's worst gap 845 ms to 720 ms. A settings write with a feed 0.1 s behind it, five times each, before and after the ring and the claim together: the longest pass in those 30 s windows went from 76–129 ms to 48–53 ms, and every feed gave its portion. The cold boot's first 70 log lines match the earlier image's apart from values, with no line dropped.
- **Observed** (October 31): the owner at rest, a session that only waits, and a writer that wakes for work, before and after on the same procedure, hopper empty, on the adapter, no phone session, one run of about 10 minutes each.
  - At rest: 917 to about 1,266 passes per 30 s, the longest pass 16–18 ms to 10–11 ms, and its 99th percentile 15.4 ms to 10.2 ms.
  - A Dispense wait: up to 5,874 to about 1,439 passes per 30 s.
  - A meal call upload, five times each: 4.8–5.6 s to 0.8–1.3 s.
  - A settings write, eight times each: 230–450 ms to 173–270 ms.
  - A full history read, three times each: 465–583 ms to 360–493 ms.
  - A feed's step gap stayed 35 ms, and the loop's stack bound 5,008 B.
  - A client that stops reading has not been closed on hardware yet, so what `lwip_close` does with its unsent data is still open.
- **Modeled:** the interlock, including storage writes' turn, is host-tested. A feed with the hotspot cycling, showing that no association overlaps a feed, has not run on hardware.
- **Modeled:** the `gap-probe` variant was built and host-tested before its run.
- **Recovered:** the storage region's safety, from a static pass over stock 1.1.15 and the research unit's 15 full dumps. The owner chose 32 sectors on October 23 and 48 on October 26.
