# The test log

This page is what we did to the feeder and what happened, including what broke. It's a curated log, not every run. It's for people who own a feeder or might run Wellfed, and want to see the record before trusting it.

Everything in the log is **Observed**: it happened on real hardware, by our [evidence levels](../../evidence.md). Where an entry leans on a host test or a simulation, it says so and labels that part **Modeled**.

## What broke

Each row links to its entry, which has the numbers and what the fix did and didn't show.

| Entry | What broke | Result |
| --- | --- | --- |
| [Forty power cuts](#forty-power-cuts-to-catch-a-hang) | A rejected Wi-Fi join after a power cut hung the chip, in 9 of 19 rejections. | **Fixed** |
| [The app during a power cut](#cut-the-power-mid-feed-with-the-app-watching) | The app showed Feeding and a live Stop button about 2 minutes after the cut. | **Fixed** |
| [Motor leads at boot](#watched-the-motor-leads-at-every-boot) | 49 of 49 boots commanded a 105 µs forward pulse. | **Fixed** |
| [First feeds](#first-feeds-through-the-feeding-owner) | A start-up surge tripped a feed 17 ms in. | **Fixed** |
| [Repeated installs](#installed-over-and-over-through-bank-0) | Crashes during downloads: 5 of 26 installs passed. | **Partly fixed** |
| [A test build's trial](#installed-a-test-build-which-rolled-itself-back) | A Wi-Fi retry starved the supervising task, and the build rolled itself back. | **Fixed** |
| [Installing from stock](#installed-from-stock-on-a-route-weve-since-removed) | Stock refused the install, then our backup check refused a correct copy. | **Fixed** |
| [The clock across an install](#compared-the-clock-with-the-bench-computer) | The clock read 12.2 s behind while claiming 5.5 s of error. | **Fixed** |
| [A 40 s hotspot drop](#dropped-the-hotspot-for-40-s) | The feeder took 64 to 68 s to answer again after the hotspot came back. | **Fixed** |
| [Two history readers](#two-clients-reading-history-at-once) | With the server's stream open, a second client got one backlog event in 25 s and no live ones. | **Fixed** |
| [Power cuts with no cells](#cut-the-power-with-no-cells-fitted) | The record read a 3,861 mV battery with no batteries fitted. | **Partly fixed** |
| [72 hours on schedules alone](#72-hours-on-schedules-alone) | Offline during a feed, the device log lost that meal's result lines, 3 times in 3. | **Passed with gaps** |

These entries are **Open**:

- [The 8-hour fault run](#the-8-hour-fault-run-six-tries)
- [The motor leads through a reset mid-drive](#looked-at-the-motor-leads-through-a-reset-mid-drive)
- [A real jam](#tried-and-failed-to-jam-it-on-purpose)

## How far this goes

- **One feeder.** Every entry ran on one research PLAF103. That feeder had already run Wellfed when its installs ran, so the install hasn't been proven on a feeder that has never run it.
- **Mostly an empty hopper.** Most motion runs had no food in them. An entry says so when food was in.
- **That unit, that build.** A pass says something about this feeder and the build on it that day, not about every feeder. Many of these ran once. The dates matter too, because the firmware changed under them, and not every result has been repeated on the current build.
- **A bench, not a home.** Power cuts are a relay cutting the adapter, with no batteries fitted unless the entry says so. Most network tests used a hotspot on the bench computer, not a home router.

None of this makes the feeder safe to rely on. [How feeding stays safe](feeding-safety.md) lists what it doesn't protect against, and the [roadmap](../../roadmap.md) says where each piece stands.

Each entry gives its dates and conditions, then opens its result with one of these verdicts:

| Verdict | Meaning |
| --- | --- |
| **Passed** | It met what we set out to check, in the runs the entry names, and no further. |
| **Passed with gaps** | It met the check, and the entry names what it missed or what went wrong along the way. |
| **Fixed** | It failed, and the fix passed in the runs the entry names. |
| **Partly fixed** | It failed, and the fix is only partly checked or doesn't cover every fault. |
| **Open** | There's no result yet, the test didn't finish, or we can't explain what we saw. |
| **Measured** | We took a measurement or found a limit, with nothing to pass. |

## Long runs

### 72 hours on schedules alone

*Nov 1 to Nov 4 · Hopper empty*

Ten daily 1-portion meals. Each day brings a 20 s power cut mid-feed, a 180 s cut across a meal, a 40 s hotspot drop mid-feed and a 40 s drop at idle. A bench computer sets the clock every four hours and after each fault, because the test hotspot's time server doesn't answer.

To pass, every meal must run once and within 60 s, with no restarts but the ones we cause, heap and stack steady, and no lost log lines. A meal the 180 s cut crosses is due to run late, under the 15-minute rule.

**Passed with gaps.** All 30 meals ran at most once, and the run lost log lines.

| Check | Result |
| --- | --- |
| Meals | 27 done and 3 uncertain, none run twice. The 3 uncertain meals are the ones the 20 s cuts stopped mid-feed. Each was recorded as uncertain at the next boot and never re-run. |
| Timing | Every other meal ran within 60 s of its time, except the three that the 180 s cuts crossed. Those ran at power-on, 2 to 2½ minutes late, as the 15-minute rule allows. |
| Restarts | 6, one for each power cut. |
| Heap | Free heap sat at 124,448 B and dipped to 124,064 B after a reconnect, then came back. The least free was 122,400 B. |
| Stack | Read only at the end, for the boot after the last cut: the feeder's own tasks kept 1,280 B or more free. |
| Hotspot drops | The feeder's log reached the bench computer again 42 to 47 s after each of the six drops ended. |
| Clock | 16 syncs, about 4 hours apart, found it 8 to 18 ppm slow (see [the clock](../../firmware/clock.md)). |

The gaps:

- **Lost log lines.** In all three hotspot drops mid-feed, the device log lost that meal's result lines. Its other lines from the feed survived, and the feed history recorded each meal as done. The feeder keeps its recent log lines in a small buffer, and while it was offline, rejoining Wi-Fi logged enough to push them out. The buffer is now four times larger, and the bench's device log marks where lines are missing. The same 40 s drop during a meal on the new build (November 4) kept all of the meal's result lines: the outage logged 4,300 B before the bench reconnected, more than the old buffer held.
- **Uncertain meals without a time.** The feed history shows each uncertain meal as "clock not set". The boot records the meal before the bench computer sets the clock, and the hotspot's time server doesn't answer.
- **The first day's faults came late.** The bench script scheduled nothing on November 1, because of the clock change that day. A second script cut the power across the 17:00 meal and dropped the hotspot at 19:54, so that day had no mid-feed faults.

This is one feeder on DC power at room temperature, on the build from November 1. Builds since then, including the clock's new limit, haven't had a long run.

### The 8-hour fault run, six tries

*Oct 27 and 28 · Hopper empty*

The plan was 16 half-hour cycles, with feeds every 10 minutes and one fault a cycle. Any failed check cuts the power and ends the run.

**Open.** It never ran the full 8 hours.

| Try | How far it got | What ended it |
| --- | --- | --- |
| 1 | 1 cycle | A check of ours that was wrong |
| 2 | 5 cycles (22 feeds, least free heap 133,440 B and flat) | A check of ours that was wrong |
| 3 | Never started | The UART adapter failed |
| 4 | 3 cycles | A real crash during a download (see [the download crashes](#installing-updating-and-rollback)) |
| 5 | Failed at once | Out-of-date bench tools |
| 6 | 4 cycles in 2 hours (26 feeds, least free heap 133,376 B) | A real Wi-Fi hang (see [power cuts](#power-cuts)) |

The 72-hour run takes its place as the long run, but only with power cuts and hotspot drops. It doesn't repeat the injected faults or installs.

## Power cuts

### Cut the power during a feed

*Oct 26 and 27 · Hopper empty*

We cut it mid-drive, and with a feed reserved but not yet driving.

**Passed.** Each cut feed was recorded as uncertain once at the next boot and never re-run.

- After one mid-drive cut, the next feed passed an uncounted homing mark at 3,301 ms, then counted 1 portion.
- After another, the next feed saw two mark edges for one counted portion, the first at 3.1 s against the usual 7.9 s. That's likely the rotor finishing the cut portion, and we haven't weighed that case.

A cut between a meal's claim and its feed is **Modeled** only.

### Forty power cuts to catch a hang

*Oct 28*

After a cut, the access point still held the old connection and rejected the first join (status 30).

**Fixed.**

- **The first 20 cuts:** 19 met that rejection, and 9 of those hung the chip until the watchdog reset it. The trail pointed at the Wi-Fi driver powering down under a write, so we turned its idle power save off.
- **The next 20 cuts:** all 20 met the rejection and none hung.
- **Nov 6, after the Wi-Fi task moved from C to Rust:** 20 more cuts, 18 met the rejection, none hung, and every boot joined.

### Cut the power mid-feed with the app watching

*Oct 31 · Twice · Hopper empty*

**Fixed.**

- **The first time:** the app still showed Feeding with a live Stop button about 2 minutes on, and said the feeder would finish by itself, which was untrue.
- **After the fix:** the app asked whether the feeder had fed at about 20 s. Once power returned, History read "Fed between 0 and 3", not re-run.

### Watched the motor leads at every boot

*Oct 24 to 27 · With a logic analyzer*

We looked for motion at power-on.

**Fixed.**

- **Oct 26:** 49 of 49 boots commanded a 105 µs forward pulse about 100 ms in. The analyzer's own bias on the pins probably helped cause it, but the fix holds either way.
- **Oct 27:** 50 cold boots with the fix showed no drive when the board opened, and all ended in brake.

Skews of up to 4.5 µs remain in the first millisecond of the supply ramp, before any code runs.

### Looked at the motor leads through a reset mid-drive

*Oct 26*

**Open.** The drive ended, then the leads flickered three times within 0.2 ms, once in the opposite direction. No drive followed. We can't yet tell a real command from noise.

## Feeding, sensors and faults

Motor current below is in the feeder's own scale, which we haven't confirmed as milliamps. The jam limit is 300 on that scale, held for two readings in a row.

### First feeds through the feeding owner

*Oct 25 · Hopper empty*

**Fixed.** A start-up surge read 333 on one reading and tripped a feed 17 ms in. A limit now needs two readings in a row. The first full portion then took 9,072 ms of drive and stopped 18 ms after the mark.

### Broke the motor control on purpose

*Oct 26 · Once per case · Hopper empty*

We broke it mid-drive, with a test build and an analyzer at the motor. Each fault came 2 s in.

**Passed.** No drive followed any reset, and nothing re-ran.

| Fault | Motor stopped at | How |
| --- | --- | --- |
| A 600 ms stalled loop | 2.21 s | |
| An 8 s stalled loop | 2.23 s | |
| A failed stop held 300 ms | 2.32 s | At the first retry |
| A failed stop held 8 s | 7.0 s | Only the watchdog reset stopped it |

### Injected jams and sensor faults

*Oct 26 and 28 · Hopper empty*

We used test builds that fake the readings.

**Passed.**

| Fault | What happened |
| --- | --- |
| A jam that frees | Counted its portion after one reversal (11.5 s) |
| A jam that holds | Ended after three reversals (7.2 s) |
| A stuck mark | Ended in 6.1 s |
| Dropped marks | Ended in 11.0 s |
| A lost sensor | Held the feeder with no reset for 180 s, and a reboot cleared it |

A sweep of every jam position is **Modeled**, 45,360 + 490 simulated runs.

### Tried and failed to jam it on purpose

*Oct 29 and Nov 1 · Full hopper on Nov 1*

On Oct 29 we pressed kibble into the rotor gap by hand. On Nov 1 we looked at it again at the bench.

**Open.** A real jam has never been reproduced.

- **Oct 29:** the feed gave 1 of 1 with no reversal, at a peak of 260 against 281 to 289 on normal food feeds.
- **Nov 1:** a person at the bench judged that the rotor could not practically be wedged by hand, and we closed the test.

So the jam handling rests on the injected run above, and we don't know whether its 1 s reversal frees a real one.

### Blocked the outlet

*Oct 29 and Nov 1 · Full hopper on Nov 1*

On Oct 29 we covered the beam mid-feed. On Oct 29, and again on Nov 1, we taped it before a feed.

**Passed with gaps.**

- **Covered mid-feed:** a 3-portion feed stopped 1.0 s later at 1 of 3 and didn't resume.
- **Taped:** the feed ended `outlet_blocked` with no drive and no fault.
- **The gap:** on Nov 1 readiness read ready beforehand, because the outlet beam isn't read at idle. So a blocked outlet shows only when a feed tries, and that meal fails.

### Fed from food and weighed it

*Oct 29 and Nov 1 · Food in the hopper*

**Passed.**

| Date | Food | Weighed |
| --- | --- | --- |
| Oct 29 | One dry food | 7.11 g a portion over 10 portions, and 7.10 g over 3 |
| Nov 1 | A different dry food, full hopper | 15 of 15 portions over four feeds, no reversals, with 40.9 g and 40.4 g per 5 portions, or 8.13 g a portion |

On Nov 1, peaks read 236, 239, 283 and 275 with a mean of 81 to 83, against 259 and 273 with a mean of 57 empty the same day. The highest of these sits 17 under the jam limit. An older food feed peaked at 289, and an empty feed read 324 on Oct 27 and still finished. We haven't measured a real stall to compare.

### Ran a hand-emptied hopper down

*Oct 29 · Eight 1-portion feeds*

**Measured.** A usual feed drops 7 g.

| Feed | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Dropped (g) | 1.7 | 11.4 | 6.5 | 7.0 | 6.0 | 3.0 | 1.9 | 0 |

The food beam read low from the first feed, with about 5 portions still left. So it's a low warning, and we reworded everything that called it empty. The motor turning past a mark counts a portion, whatever fell.

### Fed under load

*Oct 26 and 31 · Hopper empty on Oct 26*

**Passed.** The limit on the gap between motor checks is 100 ms.

| Date | Load | Largest gap |
| --- | --- | --- |
| Oct 26 | Ten 3-portion feeds, all completed, with a client reconnecting back to back, four unknown keys at once and a tune playing | 52 ms, across 336 to 369 handshake operations a feed |
| Oct 31 | Tunes ending mid-drive | 50, 39 and 54 ms, with no audio underruns |
| Oct 31 | A 6-portion feed under status reads every 5 s | 41 ms |

## Installing, updating and rollback

The feeder has two firmware slots, called banks. A new build runs on trial until it confirms itself. If it doesn't, it gives way to the other bank.

### Installed builds broken on purpose

*Oct 22*

One build fails at start, one hangs, and one never confirms because its server was stopped.

**Passed.** Each was counted over three trial boots and gave way. The one that never confirmed hit its 10-minute deadline each boot. A power cut 10 s into a trial restarted the count, and that build then confirmed. The trial also caught a real stack overflow of ours.

### Sent bad downloads

*Oct 22 and 24*

A corrupted chunk, a truncated offer, a stalled server, a restarted server and a power cut mid-download.

**Passed with gaps.** Each one stopped before activation, or resumed and activated. The next boot after a cut downloaded again from the start. The activation marker was never written early. One Oct 24 run of this suite failed when bank 0 itself crashed mid-download, and the rerun passed (see the Oct 27 entry).

### Reset storms

*Oct 26 and 27*

A build whose feeding check fails, and a build that confirms and then resets about 190 s into every boot.

**Passed.** The first gave way after three trial boots. The second gave way after five resets. On the fallback bank, with nowhere to go, it held at the sixth boot with feeding blocked and the API answering, then opened itself 10 minutes later. The longer halt that follows is **Modeled** only.

### Installed over and over through bank 0

*Oct 27*

This followed crashes during downloads.

**Partly fixed.** The crashes stopped in the runs since, but earlier faults stay open.

| Runs | Installs that passed |
| --- | --- |
| First set | 2 of 10 |
| After a full power-off | 2 of 8 |
| With the bench idle | 1 of 8 |
| With the flash clock at 50 MHz | 8 of 8 |

The crashes landed somewhere new each time. One install failed on a single-bit read error, where the flash held the right word and one read returned a bit wrong. Lowering the flash clock from 80 to 50 MHz widened the boot's read window from 11–17 to 29–31. That's one evening's runs, and thin read margin is a working explanation.

The earlier faults are 3 crashes and a hang in 157 bank-0 boots (Oct 24, none in 199 bank-1 boots), and 2 failures in about 9 installs (Oct 25). We haven't shown the clock change fixed those.

### Installed a test build, which rolled itself back

*Oct 29*

Nobody asked it to.

**Fixed.** Each of its three trial boots stopped at about 10.4 s, when a Wi-Fi retry starved the supervising task past its 1 s limit. The feeder gave way, as designed. The failing boots reached 1,088–1,093 ms. Moving the watchdog's task above the Wi-Fi driver gave a 250 ms trial gap on the one build we ran then, against 797–819 ms at the old priority, and it confirmed.

## Resetting and going back to stock

### The round trip with no UART write

*Oct 23*

To stock over our updater, then back over stock's, with nobody at the bench. Also a stock recovery over UART.

**Passed.** The round trip took 6 min 31 s, and stock's download about 40 s. The UART recovery rewrote 171 sectors and read them back, and stock booted. We skipped its final full read.

### Installed from stock, on a route we've since removed

*Oct 28 and 29*

**Fixed.** Stock refused at once with error 1214, likely because its clock was held back. Set its clock (it was 275,721 s behind) and it accepted. Then our backup check refused a correct copy: stock changes a whole 4-byte word of the marker, and we expected one byte. With that fixed, the backup matched the pinned hash and the build confirmed.

### A factory reset

*Oct 29*

By a held Reset on an unlocked panel.

**Passed.** The feeder erased 33 sectors and took a new identity, and it was paired again over Bluetooth. Settings, plans and the clip were gone, and the portion weight was kept. One gap: the wear counts it keeps miss that boot's last erases and the reset's own.

### Round trip from a wiped phone app

*Oct 30*

A real Reset, the install, the restore, another real Reset, and the same version again.

**Passed.** The final run took about 27 minutes and ended confirmed. Earlier that day an app crash at the end of setup was recovered by Pick up. One run also stalled until the phone was unlocked, and we didn't find out why.

### Back to the vendor's own app

*Oct 31 · Once*

After a restore, then an install from that feeder.

**Passed.** Setup in the vendor's app reached the vendor's cloud, after a try on Oct 30 that hadn't joined. The install from there took about 7 minutes. The phone's picker found no feeder until the saved network was forgotten in Wi-Fi settings, seen once.

### Installed from stock by the CLI

*Oct 31 · Twice*

With a person at the terminal.

**Passed with gaps.** In both runs the first setup code timed out. The first run had to be resumed, and the second took a second code. In the second, the code step also sat for some minutes, and we don't know why it moved on. The CLI's own hop onto the feeder's setup network didn't run, and the app's search for its feeder changed after its last run from stock. That installer has since been removed, and the app is the only one.

## Clock and schedules

### Compared the clock with the bench computer

*Oct 26 and 27*

Across an install.

**Fixed.**

- **Oct 26:** the clock read 12.2 s behind while claiming 5.5 s of error, about 12 s lost over 249 flash erases. It now counts on a timer that runs through erases.
- **Oct 27:** 121 ms off after the same kind of install, claiming 2.6 s.

Against a time server, three polls were used and the third agreed to about 2 ppm, but that was only 45 minutes.

### Ran meal plans through restarts

*Oct 28 · Once per case · Hopper empty*

**Passed.**

- A daily plan fed once.
- A reboot within the meal's minute ran nothing again.
- A once-only plan fed and turned itself off.
- A plan due during a cold boot with no clock held, and the sync 16 minutes later recorded it missed and unfed.

Daylight saving changes, overlapping plans and retries after a refused feed are **Modeled** only.

### Let the clock run out

*Oct 31 and Nov 1 · Hopper empty*

We also carried it across restarts.

**Passed.** Three update restarts each carried the clock over gaps of 1 to 3 s, and two power cuts lost it, as designed. With no time source and about 15 hours up, the clock expired about 6.6 hours after its last sync. A 09:30 meal held, a manual feed still ran, and a sync at 09:32:50 ran the meal late, once. Expiry with a working time server hasn't run.

## Network and Wi-Fi

### Flooded it

*Oct 24 · Once*

112 connections of 4,000 random bytes in about 35 s.

**Passed.** Each was closed as malformed. An owner's status kept answering in about 0.1 s, nothing stopped, and the heap didn't change. The Wi-Fi driver ran short of buffers four times. We don't claim it stands up to a determined flood.

### Cycled the hotspot ten times

*Oct 25 · With a timing probe*

We measured what a rejoin does to the firmware.

**Measured.** An association stalled every task for about 0.45 s, and a rejected one for about 0.72 s. That's why feeds and joins now take turns. The loop also crashed once, on what looked like a bad read from flash. That came before the flash clock change, and we haven't shown the change fixed it.

### Pointed the optional server at it

*Oct 28 · No motion*

Through a restart, a power cut and a 38 s Wi-Fi drop.

**Passed with gaps.** The server's history matched the feeder's event for event, 1 to 1,049, with no gaps or duplicates. The backlog of 1,039 events took 216 s. A dead feeder still showed online for 90 s. Feeds through the server haven't run on hardware. The server has since been restructured, and this run hasn't been repeated on it.

### Two clients reading history at once

*Oct 28, 31 and Nov 6*

**Fixed.**

- **Oct 28:** with the server's stream open, a second client got one backlog event in 25 s and no live ones.
- **Oct 31, after the fix:** two readers from different points, five times, each got every event once, in about 0.6 s.

- **Nov 6:** beside a client following live with samples on, as the server's stream does, a second client caught up 674 events in 0.46 s, in order. Both got two new events at once, and the first client's samples kept their 5 s pace. The first client was the CLI, not the server itself.

### Turned Wi-Fi leisure power save off

*Nov 1 · 120 pings 5 s apart each way, no loss*

The before and after are two builds, so other small changes sit between them.

**Measured.**

| Measure | Power save on | Power save off |
| --- | --- | --- |
| Median ping | About 50 ms | 1.25 ms |
| p90 ping | 103 ms | 3.07 ms |
| Maximum ping | 131 ms | 13 ms |
| TCP connect after a quiet minute | 84–102 ms | 3–6 ms |
| Connection time after a drop | 5.1 s, on one drop, on an earlier build | 5.2 to 5.4 s |

Current draw wasn't measured, and a Bluetooth install on this setting hasn't run.

### Dropped the hotspot for 40 s

*Nov 1 and 6 · 28 times*

**Fixed.**

- **Nov 1, three drops:** with power save off, the feeder took 64 to 68 s to answer again. The first join failed and the next waited out a 30 s backoff. One earlier drop with power save on, on another build, showed the same, untimed.
- **Why the first join fails:** the Wi-Fi driver offers the key it cached from the last join. The restarted hotspot no longer has it and rejects the join (status 53), and the driver then drops it, so the next join runs the full handshake and succeeds.
- **Why the wait was 30 s:** the wait between tries doubled the whole time the network was gone, and the rejected join waited the longest.
- **Nov 6, one drop:** the feeder now starts the waits over when the network comes back. The rejected join was retried after 1 s, and the feeder had its address 34.6 s after the hotspot returned. Most of what's left is the wait the outage grew: the try before it just missed the hotspot's return, and the next came 20 s later.
- **Nov 6, twenty more drops on that build:** each rejoin took 36 to 40 s, with no fault or restart.
- **Nov 6, three drops on the next build:** while the network is gone the wait between tries now stops growing at 10 s. The feeder had its address 29, 34 and 33 s after the hotspot returned, with no fault. What's left is the scans themselves (4 s each), the rejected join and the join that succeeds (5 s each).

## Batteries

### Pulled the adapter 20 times

*Oct 28 · Three D cells fitted · Hopper empty*

Each pull ran 20 s on the cells, a minute apart.

**Passed.** Wi-Fi went off after 5 s. Every cycle rejoined in 12 to 13 s in the same boot, with the heap's low water steady at 129,440 B and no hang.

### Cut the power with no cells fitted

*Oct 28 and 31*

Then we read what the feeder recorded.

**Partly fixed.** The record read a 3,861 mV battery, which was likely the 5 V rail decaying. The fix is **Modeled**. On Oct 31 the status read no batteries, and a cut recorded no supply event at all, because the board appears to lose power first.

### Fed on batteries by Dispense

*Nov 1 · Once · Three fresh D cells · Full hopper*

One feed with the adapter cut.

**Passed.** It fed 1 of 1 with no reset, in 10,189 ms of drive, at a peak of 268 and a mean of 86. The pack read 4,839 mV at rest with the adapter in, and 4,586 mV at its lowest by the feeder's own samples. Wi-Fi went off 35 to 46 s after the cut, not 5 s, likely held by its 60 s minimum on time after a boot. We didn't confirm that.

## Speed

### Timed reading the event history

*Oct 29 and 31*

**Measured.** A full history of 1,408 hasn't been timed.

| When | Events | Time |
| --- | --- | --- |
| Oct 29 | About 1,140 | About 6 minutes |
| By the end of Oct 29 | About 1,140 | 16.5 s |
| Oct 31, with events in batches | 440 | 334 to 345 ms, five reads |

### Let the main loop rest when idle

*Oct 31 · One run of about 10 minutes each way · Hopper empty*

We also woke the writer only for work.

**Measured.**

| Measure | Before | After |
| --- | --- | --- |
| Loop passes per 30 s at rest | 917 | About 1,266 |
| Longest pass at rest | 16–18 ms | 10–11 ms |
| A meal call upload | 4.8–5.6 s | 0.8–1.3 s |
| A feed's step gap | 35 ms | 35 ms |

The step gap comes from the one feed record the log kept each way.

### Two speedups we measured and threw out

*Oct 31*

A faster checksum, and caching the time zone rules.

**Measured.** Neither gave a real gain, and we left both out.

- **The checksum:** a history read took 325–416 ms with the faster checksum and 343–408 ms without.
- **The zone cache:** it gave about 1.6 % more passes at rest, and the longest pass at rest didn't fall. History reads were maybe 10 % faster with it, on only three reads each.

## What hasn't run

These need a person, more equipment or a change to the test network. None of them has a result on this page.

- **A real jam**, and a real stall's current.
- **A taped mark sensor.** We skipped it at the bench, so that fault rests on the injected run.
- **A real brownout, and the battery floor.** Both are **Modeled** only. Neither has run on a bench supply, so whether a real brownout sets the flag the firmware reads is unknown.
- **A flat pack.**
- **A DHCP server lost, or a lease expired.** A lease renewal has been seen, once.
- **A client that stops reading.** We have no way yet to stall one on purpose, so what happens to its unsent data is open.
- **An install on a feeder that has never run Wellfed.**

## Evidence

- **Observed, October 22 to November 1, 2026, on one research feeder, unless an entry says otherwise.** Much of the detail is on the pages for [motor and feeds](../../firmware/motor-pulse.md#evidence), [jams and sensor faults](../../firmware/jams-and-sensor-faults.md#evidence), [calibration](../../firmware/calibration.md#evidence), [power](../../firmware/power.md#evidence), [the clock](../../firmware/clock.md#evidence), [the OTA design](../../firmware/ota-design.md#evidence), [consumer installation](../../firmware/consumer-installation.md#evidence), [the device API](../../firmware/device-api.md#evidence) and [the resource budget](../../firmware/resource-budget.md#evidence).
