# Motor and feeds

One motor turns the feeder's rotor, and only the [feeding owner](../glossary.md) may drive it. Every feed, from the app, the CLI or the Dispense button, is a request to that owner. This page explains what bounds a drive, what a feed records, what a power cut mid-feed does, and the fault-injection variants that prove the motor stops when the loop or the stop path fails. It ends with the first-pulse probe that brought the motor up. The bench procedures are in [motor bench checks](motor-bench-checks.md).

Feeds through the owner, stopping on time, and surviving a power cut without feeding twice are observed on our unit.

## How a feed drives the motor

`wellfed feed --portions N` runs a feed through the `feeder` firmware's feeding owner:

1. The feed journal reserves the feed before the motor may start.
2. The motor drives clockwise (`Drive0`, PA13 HIGH and PA14 LOW) under the supervisor's 200 ms motor lease.
3. The owner credits portions from the rotor's mark sensor (PA18), and stops when the feed is done or a limit ends it.
4. Once STOP is verified, the result goes to the journal and becomes a `Fed` event.

Nothing else moves the motor. The image has no startup motion, and the bench never sends a feed. An allowed Dispense press asks the same owner for one portion, through the same readiness check ([device API](device-api.md)).

What bounds a drive (limits in `feeding.rs`):

| Bound | Rule |
| --- | --- |
| Portions | In single-bowl mode, as in stock, a portion is one rising PA18 edge (the end of a compartment's mark), at least 5 s (`feedback_inhibit_ms`) after the drive starts or after the last portion. After a feed that stopped between marks, the next feed's first rise inside those 5 s is an uncredited homing mark, and that feed's deadline is 5 s longer ([calibration](calibration.md#phase)) |
| Feedback | No credited portion within 15 s (`feedback_timeout_ms`) fails the feed `NoFeedback`. No PA18 edge for 11 s, or PA18 LOW for 4 s, also ends the drive ([missing feedback](jams-and-sensor-faults.md#missing-or-stuck-feedback)) |
| Whole feed | 12 s a portion (`portion_ms`), capped at 48 portions' worth |
| Current | A jam (over 300 scaled, held past the 150 ms after any change of drive) backs off 1 s and drives on, at most three times, then ends `Jammed`. One reading over 1000 stops at once (`Overcurrent`) ([jams](jams-and-sensor-faults.md#jams-bounded-reversals)) |
| Inputs | A blocked outlet, an unreadable beam, low power, or current over the limit at rest keeps a feed from starting, and it fails with that input's reason |
| Loop stalls | The supervisor stops the motor 200 ms after the owner's last renewal. An owner beat older than 1 s while driving ends in a watchdog reset |

### What a feed records

Each feed logs, in order:

1. `PLOWN1 feed id= portions= submit=0`.
2. The state changes: `state=3` preparing, `4` feeding, `5` reporting, `6` terminal, `1` idle.
3. `PLOWN1 fed id= requested= completed= outcome= reason= durable= drive_ms= rising= falling=`: the result and the drive's length. `rising` and `falling` count every PA18 edge while capture was on, qualified or not.
4. `PLOWN1 current id= first_edge_ms= current_max= current_mean= samples= reversals= hopper=`: the rest of what the drive measured. `reversals` counts the jam handling's reverse attempts. `hopper` is what the food beam read (`present`, `low` or `unread`). It is a separate line because the log drops a line longer than 191 bytes with its CRLF.
5. `PLOWN1 edges id=…`, with each edge's time.
6. `PLOWN1 steps id= max_gap_ms= handshakes=`: the longest gap between two owner steps during the feed, which must stay under the 100 ms observation gap, and the handshake operations the loop ran meanwhile.
7. `PLOWN1 marks id=N homed_ms=… outlet_breaks=… phase=at_mark|unknown` ([calibration](calibration.md)).
8. `PLOWN1 pins … changed= toggles=`, from a pin watch that samples both GPIO ports on the supervision tick while capture is on.

Outcomes are 1 done, 2 cancelled, 3 expired, 4 failed and 5 uncertain (an interrupted feed, resolved at the next boot). Reasons are the `FeedReason` codes in `wellfed_core_feeder::feed::controller`, such as 5 timeout, 6 no feedback, 8 sensor, 9 outlet, 10 overcurrent, 12 output, 13 jammed and 14 mark stuck.

### A power cut mid-feed

A power cut or reset mid-feed leaves the journal's reservation pending. The next boot resolves it:

1. The owner writes an uncertain result to the journal under the same id: outcome 5, `completed=0`.
2. It sits Terminal until the loop has logged `PLOWN1 fed id=N … outcome=5 … durable=1` and recorded a `Fed` event with outcome `Uncertain`, by the feeder.
3. It goes Idle.

The feed is never retried, and its id is never reused: the next feed takes the id after it. The edge cases resolve the same way:

- **A cut during the resolution's own journal write** leaves the reservation pending, so the next boot resolves it, once.
- **A cut while the reservation itself is being written**, a few milliseconds before the motor may start, leaves a cut record beside the one at rest. It can only be the next reservation, so the next boot resolves it as uncertain under the next id. Usually that reservation never committed: the motor never ran and no client was answered `Feeding`, so the uncertain result is conservative.
- **A cut late in that resolution's own erase** returns the journal to the record at rest, and the id is reserved again. That is harmless for an uncommitted reservation. Only a committed reservation that also decayed could be forgotten this way.
- **A cut while the first boot creates the journal** leaves the start of its first record beside an erased sector. The next boot reads that as the empty journal it was, erases the cut record and creates the journal again, so the feeder feeds. No other state reads as empty: a later record that decayed still faults. A second cut during that erase can leave bits the erase raised, which cannot be told from a decayed record, and faults the journal.
- **A journal that does not decode** still faults the owner closed (`fault=2`, recovery).

A reset the runtime decides never lands in a store write it let start. From the first feeder pass that does not feed the watchdog, no write turn is granted, no arming starts and an install goes no further (`supervision::writes_held`). A write already running finishes in tens of milliseconds, well inside the watchdog's expiry, and clears its attempt marker itself. So only a reset the runtime did not decide (a hang, a fault or a halted lease) leaves the marker set, which is the case it exists for ([OTA design](ota-design.md)).

## A stalled loop and a failed stop during a feed

Two opt-in variants of `feeder` inject one fault into each feed, 2 s into its drive (`INJECT_AFTER_MS`), when the motor is past its start-up surge. They show, with real motion, the supervision that depends on neither the loop nor the stop path: the TIM2 interrupt's motor lease and its stop retry, the owner's beat, and the runtime's watchdog.

The normal `feeder` image has neither. `feed-stall` is a feature of the firmware crate only. `stop-fail` also enables `wellfed-hal-feeder`'s `stop-fail` feature, without which the injection is not compiled. Nothing else changes: no startup motion, feeds only from the device API, the same limits, journal and no-retry rule. The requested portions pick the case:

| Variant, portions | Fault | What ends the drive | Then |
| --- | --- | --- | --- |
| `feed-stall`, 1 | The loop spins 600 ms, interrupts enabled | The motor lease: the timer interrupt stops the motor 200 ms after the owner's last renewal (`MotorGateFault::Expired`) | The owner finds its supervisor faulted and records the feed failed (`outcome=4 reason=12`) durably. It never beats again, so 1 s later the runtime withholds the watchdog and the board resets. The next boot is Idle with nothing pending |
| `feed-stall`, 2–3 | The loop spins 8 s | The motor lease, as above | The beat lapses and the watchdog resets the board inside the spin. The next boot resolves the feed as uncertain (`outcome=5`) and never re-runs it |
| `stop-fail`, 1 | STOP writes held 300 ms, then the feed cancelled | The timer interrupt's stop retry, every 10 ms: the first one after the hold lifts | The cancellation is recorded durably (`outcome=2`, owner fault hardware), then the lapsed beat resets the board |
| `stop-fail`, 2–3 | STOP writes held 8 s, then the feed cancelled | The watchdog reset: the owner stops beating once its stop fails, the runtime withholds the refresh after the 1 s gap, and the reset puts the motor pins in their reset state (the driver's standby) | The next boot resolves the feed as uncertain |

The lengths are pinned against the supervision bounds at compile time (`faults.rs`). The short ones fall between the 200 ms motor lease and the 1 s beat. The long ones pass the beat plus the 6 s ceiling on the watchdog's expiry, whose bounds are 2.4–5.5 s (measured about 4 s).

**Where the failed stop is injected.** While the board is open, only the board's pin write (`BoardPins::write_pair`) writes the motor pins: it writes the pair and reads it back. With a hold armed (`wellfed_hal_feeder::inject`), that write skips a STOP and leaves the pins driving, so the readback fails exactly as a pad that did not follow its latch would. Everything after that is the product's own failure handling:

- the gate latches an I/O fault with the applied state unknown;
- the supervisor faults its outputs, and the timer interrupt retries the stop every tick;
- the owner faults hardware, keeps its result back until a stop verifies, and stops beating;
- the runtime's watchdog check reads the pins (`stopped::observe`) and the beat.

Other injection points were rejected. Skipping the owner's stop request (the controller or `quiet`) would exercise only the owner, since the gate would still stop the motor at its lease. Re-driving the pins behind the board's back bypasses the readback the board relies on, and models a second writer the board does not have. Changing pin modes or masking the timer puts the driver or the supervision in a state the product never sees, and could leave the motor in an unknown mode.

The hold only skips STOP: a drive write is untouched, and the faulted gate issues none. It is bounded twice: the variant asks for 300 ms or 8 s, and the SDK caps any hold at 10 s (`MAX_HOLD_MS`). After it lifts, the timer's next retry stops the motor even if the watchdog never reset, so the long case drives at most about 10 s, whatever else fails. The hold is a RAM static, disarmed at every boot.

The stall spins in the loop task with interrupts enabled, like `owner-stall`, so the TIM2 interrupt and the runtime's watchdog feeder keep running while the owner is not stepped.

> [!WARNING]
> Both variants inject into every feed they run, whatever starts it: the device API, the Dispense button or a schedule. Never leave one installed. Put the normal `feeder` back right after the run.

### Records

Besides the usual `PLOWN1` lines, and the runtime's `PLSUP1 stop … owner_age= owner_gap= supervised=` when it withholds the watchdog:

| Record | Meaning |
| --- | --- |
| `PLINJ1 inject id= fault= severity= ms=` | Fault 1 stall, 2 failed stop. Severity 1 short, 2 long. `ms` is the spin or the hold |
| `PLINJ1 stall id= end ms=` | The loop came back from its spin. After a long stall it means the watchdog did not reset the board: a failure |
| `PLINJ1 stop id= cancel=` | The hold is armed and the feed cancelled |
| `PLINJ1 hold lifted= skipped=` | The hold ended by itself, having skipped that many STOP writes. After a long hold it is a failure too |

## The first-pulse probe

Before feeds existed, one supervised 100 ms pulse brought up the motor. `wellfed-build build --device plaf103 --profile firstboot --firmware bringup --variant pulse` builds it. The manifest records `startup_motion_policy: fw16-pulse-v1` (declared in `src/firmware/wellfed/bringup/Cargo.toml`) and `variant: pulse`, and the build gets its own build ID. The builder refuses a motion policy outside a variant, and the bring-up firmware's other images start no motor task, so nothing else can drive the motor. The bench never builds it.

After startup, the pulse probe (`src/sdk/wellfed-probes/src/pulse.rs`):

1. Skips everything unless this is a clean cold boot: reset reason 0 and its backup-register marker clear.
2. Opens the board (motor STOP, emitters OFF, sensors), binds a fresh flash authority to the KM0 peer, and opens the supervisor: TIM2 at 10 ms, owner and motor lease 200 ms. It enables feedback capture.
3. Logs `armed`, then waits 60 s, renewing its owner lease every 2 ms, so the operator can get ready.
4. Sets the marker, then drives once: `DRIVE_0` (PA13 HIGH, PA14 LOW) for 100 ms, then STOP. It counts PA18 edges and samples motor current on ADC channel 4 every 10 ms.
5. Watches 1 s more, then verifies STOP: the gate state, the motor pins, the supervisor closing and the stopped-output readback.

The timer interrupt enforces the motor gate's 200 ms command lease and the owner lease, so both hold even if the task stalls. A host test stalls the task mid-pulse and requires STOP within the lease. A drive applies only through `Events::motor`, which needs a running timer, a live owner and enabled capture.

> [!WARNING]
> Every clean cold boot pulses once. The marker survives a watchdog reset but not power loss, so the image must not stay installed after the session.

Records (`PLMOT1`), in order:

- `start created=`
- `open board= ready= attached= events= capture= ms=`
- `armed delay_ms= pulse_ms= lease_ms=`
- `pulse dir= applied_ms= on_ms= edges= first_edge_ms= current=min,max,mean samples=`
- `after ms= edges= current=… stopped=`
- `done`

A `skip`, `timeout` or `error stage= fault= stopped=` ends the probe stopped.

Guards:

- `wellfed-bench flash-prepare` and `flash-execute` refuse a target whose manifest names a motion policy unless `--attended-motion` is given.
- The startup audit binds `fw16-pulse-v1` to the `PLMOT1 start created=` format both ways, and refuses motion and optics policies together.
- The electrical output checks and the current channel were prerequisites.

## Evidence

All observed on the research unit with the hopper empty, unless marked. The full dated record is kept in the project's work history.

- **First pulse** (October 24, owner present, `fw16-pulse-01`, image `073940ec…`, capture `fw16-pulse-01-run`). One pulse of 106 ms (the 10 ms timer tick), `stopped=1`, no error, and the owner heard the motor. The analyzer at the motor (1 MHz) saw one drive in 130 s, 106.07 ms with the white-lead side HIGH, 60.5 s after power-on; `dir=1` is `MotorCommand::Drive0`, PA13 HIGH and PA14 LOW. Opening the board before arming showed a 212 µs coast. Current read raw 64–447, mean 147 over 10 samples while driving, 15–16 stopped. No PA18 edge in the pulse or the second after it. `feeder` v154 was restored over UART (`fw16-restore-01`). The owner ran this before the power-on flicker at the motor was classified, since the ROM loader shows it too.
- **Direction** (October 25, `fw16-feed-02`, v164). The first feed through the owner: the owner saw the rotor turn clockwise (`Drive0`, PA13 HIGH). It stopped by itself on the then 3 s feedback timeout (`outcome=4 reason=6 durable=1 drive_ms=3018`, current max 166 and mean 64 scaled), with no `PLFLT`, and the client got the `Fed` event.
- **Mark sensor** (October 25, v165–v167). The pin watch saw only PA18 change among the GPIO pads. PA18 rests HIGH and reads LOW for about 1.17 s at a mark. The portion is credited on the rise and the motor stopped 18 ms later (`drive_ms=9072`, edges at 7,888 ms LOW and 9,054 ms HIGH). A portion is about 9.05 s of drive, empty. A start-up surge read 333 on one poll and tripped a feed 17 ms in, so a jam now needs two consecutive polls over the limit (`OVERCURRENT_POLLS`); a feedback edge reuses its poll's sample and does not count.
- **Food and a full turn** (October 25). With food (v167, `fw16-food-01`, `-02`), three 1-portion feeds ended `Done` at 9,787–9,857 ms, mark LOW about 1.38 s, mean current 82–87. PA17 read food present, the outlet beam changed 0, 4 and 8 times, and food came out, at first a little less than stock's portion. A 3-portion feed (`fw16-turn-01`, v168 `0c955e25…`) took 27,241 ms, marks at 9.0, 18.2 and 27.2 s, stopping 20 ms after the third rise, and the owner's tape mark returned to its start: three compartments a turn, as owner photos of the rotor over one drop hole show. These figures are the virtual feeder's plant (`wellfed_sim::plant`, [development guide](../development/README.md)).
- **Stock** (recovered, PA18 interrupt `0x0e01a55c`): single-bowl mode accepts PA18 only when it reads HIGH, the end of a mark, then disarms it for 5 s; the drive's start also arms that timer. Its missing-feedback protection is about 30 s.
- **Cancellation** (October 25, `feedwrites-20261025-01`, v173): feed 18, cancelled 2 s into its drive, ended `outcome=2 reason=2` with outputs stopped.
- **Power cut mid-drive** (October 25 and 26): an interrupted feed resolved as uncertain at the next boot and was not re-run.
- **Stall and failed stop** (October 26, unattended, adapter only, analyzer at the motor). `feed-stall` v228 (`7e367b6a…`) and `stop-fail` v229 (`5166022b…`) each installed to bank 1 and confirmed before any feed; `feeder` v230 (`aa3c7f3f…`) was restored afterwards.

  | Feed | Case | Drive at the motor | Result |
  | --- | --- | --- | --- |
  | 26 (`fw16-stall-20261026-02`) | Stall 600 ms | 2,210 ms, about 0.18 s after the injection, by the lease | Failed, output, durable; reset 3.8 s after `PLSUP1 stop owner_age=1116`; next boot Idle |
  | 27 (`fw16-stall-20261026-03`) | Stall 8 s | 2,227 ms, by the lease | Watchdog reset inside the spin; next boot `outcome=5` |
  | 28 (`fw16-stopfail-20261026-02`) | Failed stop 300 ms | 2,317 ms, at the first retry after the hold (`hold lifted=1 skipped=60`) | Cancelled, durable, owner fault hardware; then reset |
  | 29 (`fw16-stopfail-20261026-03`) | Failed stop 8 s | 6,995 ms, until the reset | `PLSUP1 stop outputs=0 owner_age=1172`; next boot `outcome=5` |

  Each capture shows one drive, none after a reset, and no boot skipped arming. The four resets came 3.80–3.83 s after the runtime withheld the refresh, about 4.05 s from the last refresh: about 32.8 kHz. Of the 60 skipped STOP writes, about 29 were the timer's retries (one per 10 ms tick) and the rest the owner's own attempts (recovered).
- **Reset mid-drive** (October 26, `fw16-stopfail-20261026-03-la.bin`). At feed 29's reset the long drive ended, then the two leads read differently three more times within 0.2 ms: 38 µs with D2 HIGH and D3 LOW (opposite to the drive), then 38 µs and 110 µs with the drive's polarity. No drive followed. Back-EMF from the coasting motor would keep the drive's polarity, so the opposite read does not fit it, and outputs-only captures cannot tell drive from noise near the threshold ([motor driver](../devices/plaf103/motor-driver.md)). This transition is still open.
- **Host and simulation.** Modeled: `wellfed-hal-feeder`'s `tests/inject.rs` (built with `--features stop-fail`, which `make check-rust` runs) covers a held stop driving on while the owner holds its result and stops beating, the first retry after the hold stopping the motor and recording the cancellation, and a hold capped and never touching a drive write. The virtual feeder (`wellfed-sim`, its `stop_fails` command) stops a 600 ms stall by the lease within 220 ms, stops a 300 ms failed stop at the first retry, and drives a never-taking stop until the watchdog reset, 1 s plus an expiry inside the bounds after the cancel. `stalled_owner_independent_timer_and_watchdog_reset` covers the 8 s stall. `faults.rs` injects each feed once, 2 s into its own drive.
- **A cut first journal creation.** Modeled, November 11, 2026, not observed on hardware: `wellfed-core-feeder`'s `tests/feed_journal.rs` cuts the first creation at every byte, for a single bowl and for double bowls of either direction and phase, and each reads as empty and is created again; cuts of that second creation read as empty or created, or, only within its erase, as corrupt; and a later record decayed beside an erased sector still faults. `wellfed-hal-feeder`'s `tests/owner/journal.rs` cuts the first boot's creation at every byte, then boots the owner, which opens Idle and feeds id 1.
- **Current in milliamps.** Not measured. Scaling the channel needs a reference measurement of the motor's current, such as a meter or USB power meter in series.
