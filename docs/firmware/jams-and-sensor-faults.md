# Jams, missing feedback and sensor faults

A feed can go wrong: kibble jams the rotor, the mark sensor stops reporting, the outlet is blocked, or the ADC stops converting. In every case the feeding owner stops the motor, writes an explicit outcome and reason to the journal and the `Fed` event, and never restarts the feed by itself. A client never resends an uncertain feed, and the feeder never re-runs one.

Every case has run on hardware through fault-injection variants, and a taped outlet beam has been tried by hand. A jam with real food and a current reference are still open ([left for the owner](#left-for-the-owner)). Status is on the [roadmap](../roadmap.md), and the bench procedure is in [motor bench checks](motor-bench-checks.md#2-jams-and-sensor-faults).

The limits are in `feeding::CONFIG` (`src/firmware/wellfed/feeder/src/feeding.rs`). The behavior is the controller's (`wellfed_core_feeder::feed::controller`) and the owner's (`wellfed_hal_feeder::owner`). The drive, its marks and its current are in [motor and feeds](motor-pulse.md#how-a-feed-drives-the-motor).

## Jams: bounded reversals

A jam is motor current held over the limit: two consecutive polls, 20 ms apart, over 300 scaled, outside the 150 ms after any change of drive. The controller backs off: 1 s the other way, a 0.5 s stop, and forward again. As in stock, it does this at most three times, and the next jam ends the feed `Failed`, `Jammed`. Other ends:

- A jam that holds while backing off ends the feed `Jammed` at once.
- The limit held during a pause, with the motor stopped, ends it `Overcurrent`: nothing turns, so it is a reading at rest, not a jam.
- One reading over the 1000 ceiling ends it `Overcurrent`, with no reversal.

| Limit | Value | Why |
| --- | --- | --- |
| `current_limit_ma` | 300 scaled | Stock's limit, in stock's unit. 1.8 times the highest drive peak seen |
| `surge_ms` | 150 | Covers the driver's 50 ms deadtime and the start-up surge at every start, reversal, stop and resume |
| `current_max_ma` | 1000 scaled | One reading over it stops the drive at once, even in the surge window |
| `reverse_ms` | 1000 | About 13 degrees of rotor back, a ninth of a compartment: meant to unseat a kibble without dispensing. Provisional |
| `direction_pause_ms` | 500 | The stop between directions |
| `max_reverse_attempts` | 3 | Stock's count |

- **Measured current.** A drive reads a mean of 59–64 scaled and peaks of 164–168 empty, a mean of 82–87 with food, and 15–16 stopped. A start-up surge read 333 on one poll 17 ms in, and up to 447 at 10 ms polling. Full scale reads about 3192, so 1000 is a third of the channel's range and more than twice the highest surge.
- **Units.** Milliamps need a reference. R36 (1.00 Ω) is the current sense, so the scaled values are about milliamps if the scale is pin millivolts. That is unconfirmed.
- **Reversal length.** Stock's is not recovered, so 1 s stays provisional until a real jam shows what frees it. A turn is about 27 s of drive.

**Threshold risk.** The 300 and 1000 come from empty and light-food drives on the adapter. On November 1, four full-hopper feeds peaked at 236 to 283, one feed on batteries at 268, and empty feeds at 259 and 273. Older feeds went higher: 289 on food, and 324 on an empty feed that still finished. A real stall's current is not measured. Peaks near 300 would make sound feeds back off, and end them after three reversals. A stall reading over 1000 would stop every jam at once with no reversal: safe, but never freeing one.

Rules the controller keeps (host-tested in `wellfed-core-feeder`'s `feed_controller`):

- **Bounded time.** Each reversal adds what it costs, 2.5 s (back, pause, and forward to win the ground back), to the feed's deadline and its feedback window. Three add at most 7.5 s, and the deadline never passes `max_duration_ms`. A jam that clears in time completes its portions. A slower rotor or a jam freed late can still end the feed on its deadline.
- **A jam both ways ends it.** The limit held while backing off, or during the pause, ends the feed at once, with no more motion either way.
- **No start over the limit.** Current over the limit with the motor stopped is a faulty reading, so the feed does not start (`Sensor`, [below](#outlet-and-food-sensors)).

**Each mark counted once across a reversal.** The 5 s inhibit window stays anchored to the last credited portion (or the start), not to the resume. The rotor wins its 1 s back in about a second, so a window restarted at the resume would hide a real mark just past the jam: about 44 % of jam positions would lose a mark and under-report food.

- Edges captured before the resume are dropped.
- Backing off recrosses at most one boundary, since 1 s is shorter than a mark (1.2–1.4 s). That boundary is the rise the rotor passed just before the jam. If the last forward edge was a rise (credited or inside the inhibit window) and PA18 read LOW while backing off, the next forward rise is that same one and is skipped, with a fresh inhibit window for its bounces.
- A double bowl's first direction counts the fall into a mark instead, so there the recross is the input reading HIGH while backing off. Any non-boundary edge first (the rotor never left the mark) cancels the skip (`a_freed_jam_in_a_double_bowl_d1_credits_each_fall_once`).
- `a_freed_jam_anywhere_in_a_mark_period_credits_each_mark_once` sweeps jam positions over two mark periods with one to three reversals. The simulator's `a_jam_next_to_a_mark_credits_each_mark_once` puts a jam just before a mark and just past one.
- A recross whose LOW lasts less than one 20 ms poll is not seen. The inhibit window covers it only for a single reversal (recovered).

Stock counts a feed ended by a jam as done (record 2031). Wellfed records it failed, `Jammed`, with what it completed: it never claims food it cannot count. Like stock, it never retries the feed. The status names it (`feed_fault`) until a later feed completes, and the trace keeps the reversal count.

## Missing or stuck feedback

The rotor's mark sensor (PA18) rests HIGH and reads LOW for 1.2–1.4 s at each mark, one mark a portion. Marks come 9.1 s of drive apart empty and 9.8 s with food, so a turning rotor shows an edge at least every 8.5 s.

| Fault | Detection | Ends as |
| --- | --- | --- |
| Marks lost: sensor gone, PA18 stuck HIGH, or a rotor stalled with no current to show it | No edge of either level for 11 s (`mark_gap_ms`) of forward drive, counted afresh at each resume; or no portion within the 15 s feedback timeout | `Failed`, `FeedbackTimeout` (`NoFeedback` on the wire) |
| Mark input stuck LOW | PA18 LOW for 4 s (`mark_low_ms`) of forward drive; a reversal or pause restarts the count | `Failed`, `MarkStuck` |

Missing feedback never triggers a reversal. Without marks the controller cannot tell a jammed rotor from a lost sensor, and backing off blind is motion it cannot account for. A one-portion feed with no marks ends at 11 s, inside its 12 s deadline, so the reason says what went wrong rather than `Timeout`.

The level comes from the owner's own acquisitions (one port read with PA16, PA17 and PA19), not from the edge interrupts, so a stuck input with no edges is still seen.

## Outlet and food sensors

**Blocked outlet.** A beam blocked for 1 s (the stock filter) during a drive ends the feed at once: the motor stops, and the result is `Failed`, `Outlet` (`OutletBlocked` on the wire) with the portions credited so far. The outlet clearing resumes nothing, and the feed is never retried.

Stock instead pauses the feed and resumes it if the outlet clears within about 10 min, else ends it (record 2008). Ending at once is the owner's decision (October 28), so a blocked chute never turns into a meal delivered minutes late, or partly into a blocked outlet.

**A feed that never starts** because of an input ends with that input's reason. A blocked outlet, an unsettled or unreadable beam, low power, or current over the limit at rest holds it until its 2 s prepare window runs out. It then ends `Failed` with `Outlet`, `Sensor` or `Power`, with no motion (`motion_possible` false). A feed that only ran out of time still ends `Expired`, `TotalTimeout`: the journal's format holds an expired result to that reason, and bank 0 reads it with the same rule, so any other reason takes the failed outcome.

**Food beam.** The owner reads the food beam while a feed's emitters are on and in a [window at rest](optical-inputs.md#reading-the-food-beam-at-rest), and publishes its latest qualified reading (`OwnerStatus::food`).

- Food shows at once, and low only after 1 s of readings without. Stock's `surplusGrain` filter counts 20 readings. Its poll rate is not recovered, so the 1 s assumes the nominal 50 ms its outlet filter implies (to confirm against stock).
- `Feeds::food()` returns the reading, so the status's `food`, the `Food` change event and the statistics' Food section (present, ran low, refilled, feeds since refill) follow it.
- `present` comes from the first reading after a boot, which records a `Food` event only if a feed took it. A refill between two feeds shows at the next reading, within about a minute on the adapter.
- Each feed's trace keeps what the beam read during that feed (`FeedTrace::hopper`: low once it read low). Its `Fed` event carries it as `hopper`, and `PLOWN1 current … hopper=` logs it.
- As in stock, a feed never waits on the beam and is never refused for it. A beam stuck on no food, or a hopper that really is low, still feeds, and the history says the food read low. A food beam stuck either way reads as a hopper that never changes.

**Implausible beams.** A receiver that reads lit with its emitter off would mean a stuck beam. The owner does not check that yet: the fake hardware does not model receivers that follow their emitters, and ambient light on the receivers is not measured. The risk it would catch is a stuck-clear outlet beam, a blocked outlet read as clear; a jam then still ends the drive.

## Lost sensors: held, not reset on

An ADC conversion that never completes trips the board. The owner then holds the fault rather than resetting:

| Where | What happens |
| --- | --- |
| At rest | Fault `Sensor` (code 7), held. The owner verifies STOP and the emitters off on every pass and keeps beating, so the platform stays up, serves the device API and OTA, and reports `Blocked::Fault`, a lasting reason recorded as `FeedingBlocked`. A restart clears it: the device API's `Reboot`, power, or the one recovery restart below |
| During a feed | The feed fails `Sensor`, durably, then the same hold. A feed cancelled in the same acquisition that lost the sensors also records `Failed`, `Sensor`, since the journal holds a failed result to a fault's reason |

**Why hold.** On a confirmed build, a reset only meets the same converter again. When the owner faulted `Hardware` and stopped beating instead, a converter that stayed lost reset the board every few seconds. A trial build is different. Its owner must prove it can feed before the build confirms, and a faulted owner reports `Feeding::Failed`, so the runtime resets the trial at once, KM0 counts it, and the update falls back to bank 0. So the hold only ever lasts on a confirmed build. The conversion's bound counts readiness polls, not time, so preemption cannot exhaust it: only a converter that stopped converting fails it.

**One recovery restart.** A held feeder never reads the ADC again, so on its own it would never feed again. After the owner has held lost sensors at rest for 10 min (`RECOVERY_HOLD_MS`), with no feed in flight and no install running, the loop logs `PLOWN1 recover restart=1 held_ms=` and asks for a restart once.

- It takes the deliberate-restart path every reboot takes (`request_reset`), which settles the trial register as asked-for, so it does not count toward a reset storm.
- It happens only from the first boot of a power cycle. If the sensors are lost again after it, the owner holds them until a `Reboot` or a power cut.
- The first boot is told from the KM4 boot count in backup register 3, incremented at each arming and cleared by power loss. It is read and never written, since no backup register bit is free to count recovery restarts (1 is rewritten at every arming, 2 is the crumb, 3 and 4 the cores' boot counts and the trial, 5 KM0's heartbeat). So a power cycle that has already had any reset, such as an update or a watchdog reset, gives up its recovery restart.
- While a reset storm holds the feeder (`wellfed_sdk::held`), the owner stays closed and the loop never reaches the recovery.

Only lost sensors are held. A lost supervisor (the timer, its clock, the owner's lease with the emitters on) or a board faulted by an output failure still lapses the beat into a watchdog reset. The supervision itself is in doubt there, and the reset is the independent path ([motor and feeds](motor-pulse.md#a-stalled-loop-and-a-failed-stop-during-a-feed)).

**The watchdog clock's bounds stay 24–54 kHz.** No stop here depends on the watchdog, since the controller and the TIM2 lease end every drive, and the wide bounds only make the fault variants' ceilings more conservative. Narrowing them would change the audited supervision constants for no stop that needs it.

## Records

- `PLOWN1 current … samples=N reversals=N`: the reversals the drive made.
- `PLOWN1 state=7 fault=7`: the owner holds lost sensors.
- `PLOWN1 recover restart=1 held_ms=`: the one recovery restart, about to happen.

The journal's v1 record predates `Jammed` and `MarkStuck`, and stays byte-identical for bank 0 and the pinned tools. So it stores them as `Overcurrent` and `Sensor` (`FeedReason::journaled`), reasons bank 0 decodes. The owner's reported result, the `Fed` event and the `PLOWN1 fed` line keep the specific reason (codes 13 and 14). The device API's `FeedReason` names them `Jammed` and `MarkStuck`, and calls the outlet `OutletBlocked`. `Status.feed_fault` names the last feed that ended on a fault until one completes. Owner fault 7 appears only in UART records and the simulator's reports.

## Fault-injection variants

Two opt-in variants of `feeder`, `feed-jam` and `sensor-fault`, are never in the normal image. They enable `wellfed-hal-feeder`'s `feed-faults` feature, without which nothing below is compiled. While a fault is armed, the board's input capture reads what that fault would show: the current channel's samples, the GPIO word, an ADC conversion, or the feedback interrupt. The motor really turns and the marks really pass, and everything after the capture is the product's own handling. The pin watch samples the pads directly, so a capture still shows PA18 toggling under a masked sensor, which tells an injected fault from a real one.

The requested portions choose the case (`feed_faults::Planner`, host-tested):

| Variant | Portions | Shown | Expected, empty hopper |
| --- | --- | --- | --- |
| `feed-jam` | 1 | A jam the first reversal frees, 2 s into the drive (forward reads 1000 raw, about 620 scaled, until the motor first drives back) | One reversal, then `Done`, `completed=1`, `reversals=1`, about 11.5 s of drive |
| `feed-jam` | 2 | A jam no reversal frees, 2 s into the drive | Three reversals, then `Failed`, `Jammed`, `reversals=3`, about 7 s from the start |
| `feed-jam` | 3 | The mark input stuck LOW, 2 s into the drive | `Failed`, `MarkStuck`, about 6 s of drive |
| `sensor-fault` | 1 | Marks lost from the drive's start (PA18 reads HIGH, its interrupts dropped) | `Failed`, `FeedbackTimeout`, at about 11 s |
| `sensor-fault` | 2 | The outlet blocked before the feed starts | `Failed`, `Outlet` (`OutletBlocked` on the wire), no motion, about 2 s |
| `sensor-fault` | 3 | The ADC lost 2 s into the drive | `Failed`, `Sensor`; owner fault 7, readiness `Fault`, no reset until a `Reboot` |

Records:

- `PLINJ1 show id= fault=` when a feed is shown its fault, with the `Injected` code: 1 jam that frees, 2 jam that holds, 3 marks missing, 4 mark stuck LOW, 5 outlet blocked, 6 ADC lost.
- `PLINJ1 clear id= armed= readings= edges=` once that feed has ended: whether the fault is still armed (0 once a freed jam cleared itself), the captures it changed, and the feedback interrupts it dropped.

> [!WARNING]
> Every feed is shown its fault, whatever starts it. Never leave a variant installed.

Host coverage: `wellfed-hal-feeder`'s `tests/feed_faults.rs` (built with `--features feed-faults`, which `make check-rust` runs) runs each case through the owner. The simulator models the same faults on the measured plant (`wedge` for a jam that frees or holds, `jam` with `edge` for lost or stuck marks, `inputs` for the outlet, `adc_stuck` for the ADC), in `wellfed-sim`'s `virtual_feeder` tests and the `jam-freed` and `jam-held` scenarios under `src/firmware/vendor/sim/scenarios`.

## Left for the owner

- **A real jam.** A kibble or a wedge in the rotor, with food. Kibble pressed into the rotor by hand did not jam it ([Evidence](#evidence)). The stall current the 300 limit should catch is not measured (the simulator's 1000 raw is a guess), nor whether a 1 s reversal frees a real jam.
- **Taped sensors.** PA18 covered (lost marks), and a stuck mark.
- **The rotor on a tired pack.** One feed on fresh D cells has run: 10,189 ms of drive and a peak of 268, inside both the 11 s `mark_gap_ms` and the 300 limit ([Evidence](#evidence)). A low pack hasn't, and could need a battery case for both limits.
- **A current reference.** A USB power meter in series, or a confirmed sense-resistor scaling, to put 300, 1000 and the surge in milliamps.
- **Food readings.** The food beam's reading across a refill, so the statistics' low and refilled counts have a physical check.

## Evidence

- **A full hopper and batteries.** Observed (November 1, the same day and food). Feeds of 3, 2, 5 and 5 portions, none reversing, peaked at 236, 239, 283 and 275. A feed on three fresh D cells peaked at 268. A taped outlet again ended a feed `outlet_blocked` with no drive, and readiness read ready before it.
- **A taped outlet, and kibble by hand.** Observed (October 29, owner present, hopper empty). The outlet beam taped before a feed ended it `outlet_blocked` with no drive, and the rotor stayed still. Covered mid-feed, a 3-portion feed ended `outlet_blocked` at 1 of 3, with the motor stopped 1.0 s after the beam was blocked and no resume. Kibbles pressed into the rotor gap did not jam it: the feed finished 1 of 1 with no reversal, at a peak current of 260 against 281–289 on normal food feeds.
- **The renamed reasons.** Observed (October 28, unattended, hopper empty, `feed-jam` variant): 1 portion done after a reversal, 2 portions `jammed`, 3 portions `mark_stuck`, and the status named the last fault.
- **The variants on hardware.** Observed (October 26, `feed-jam` v233 and `sensor-fault` v234, hopper empty, feeds 48–53, UART captures `fw19-{jam,sensor}-20261026-feed{1,2,3}-uart`):
  - A jam that frees credited its portion after one reversal (feed 48, `Done`, 11.5 s).
  - A jam that persists ended `Failed`, `Overcurrent` after three reversals (49, 7.2 s); that reason is now `Jammed`.
  - A mark input stuck LOW ended `Sensor` (50, 6.1 s, no reversal); that reason is now `MarkStuck`.
  - Dropped mark interrupts ended `NoFeedback` at 11.0 s while the pin watch saw PA18 toggle (51).
  - A blocked outlet refused with no drive (52).
  - A lost ADC held owner fault 7 with readiness blocked `Fault` and no reset in 180 s, cleared by a reboot (53).
  - Every feed ended stopped and durable, with no reset.
- **In a long run.** Observed (October 27, the first sustained bench run): the `feed-jam` cases repeated as above (Done after one reversal, Overcurrent after three, Sensor). The `sensor-fault` cases gave `FeedbackTimeout`, `Outlet` with no drive, and `Sensor` with the owner going to its fault. The owner logs that fault state as the supervisor stops the drive, before the feed's result.
- **Current figures.** Observed: drive means and peaks, the 333 surge on v166 and the 447 surge on the first pulse ([motor and feeds](motor-pulse.md#evidence)).
- **Stock.** Recovered: stock reverses over 300 and ends the feed after the third reversal; it counts a jam-ended feed as done (record 2031) and pauses on a blocked outlet (record 2008).
- **Watchdog clock.** Observed: nine watchdog resets on October 25 and 26 came 3.97–4.05 s after the last refresh, about 32.4–32.8 kHz, the 32.768 kHz SDM clock. The supervision probe's 43 kHz was most likely its 1 s stall reporting (recovered).
- **Host and simulation.** Modeled: the controller rules above, and every jam position swept, single and double bowl (45,360 + 490 simulated runs clean).
