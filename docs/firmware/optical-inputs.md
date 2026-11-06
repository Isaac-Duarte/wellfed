# Optical inputs

The feeder watches its food with infrared beams. One looks across the hopper and says whether food is there. It sits above the rotor, so it is a low warning that reads low with about five portions left. Another looks across the outlet chute and says whether it is clear. A third input, the rotor's mark sensor, tells the feeder when a portion has turned past. This page covers how the firmware drives the emitters, reads the receivers and keeps both bounded, and the optics probe that proved them.

| Input | Pin | Emitter | Reads |
| --- | --- | --- | --- |
| Food beam | PA17 | PA15 | LOW is food present |
| Outlet beam | PA16 | PB3 | HIGH is clear, LOW is blocked |
| Mark sensor | PA18 | none | Rests HIGH, LOW at each rotor mark ([calibration](calibration.md#what-a-portion-is)) |
| Adapter | PA19 | none | HIGH while the adapter is in ([power](power.md)) |

As in stock, a blocked beam reads LOW on both receivers, the same as with its emitter off.

## How readings are qualified

Beam states come from the stock-derived `SensorInputs` filter, and only while both emitters are on. A reading needs 100 ms of optical settling, and a bad state 1 s of continuous bad readings. Each receiver changes within one 2 ms poll of its emitter turning on and within 1–3 ms of it turning off, so the 100 ms settle is pure margin. Receiver levels show beam state only, not beam geometry or how much food reads "present".

While the emitters are on under supervision, the owner publishes the states, and health reports `outputs: "sensing"` instead of a stopped-readback fault: motor STOP read back and the emitters exactly as published.

## Reading the food beam at rest

A feed reads the food beam as it runs. So that the low-food light and `Status.food` also follow a restart or a refill, the feeder reads it with no feed running:

| Constant | Value | Why |
| --- | --- | --- |
| First window | 5 s after the feeder is first free to | |
| `FOOD_READ_EVERY_MS` | 60 s | Then every minute |
| `FOOD_READ_RETRY_MS` | 35 s | After a window that reached no level. Longer than the 30 s of verified-stopped outputs the [confirmation](ota-design.md#what-confirmation-proves) waits for, so retries never starve it |
| `FOOD_READ_MS` | 1.2 s | Both emitters lit: the filter's 100 ms settle, 1 s of bad readings, and a few samples of lag |

The owner judges the level at the window's end, so a flicker inside it changes nothing, and a window that ends between levels settles nothing. The level goes to `Status.food` and the light. A later change is one `Food` event. A boot's first window records no event and writes nothing, since it can't tell whether anything happened to the hopper: a refill made while the feeder was off shows in the status and the light, not in History.

A window never moves the motor, and stays out of everything else's way:

| Against | What happens |
| --- | --- |
| A feed or a Dispense press | The feed goes first: the loop turns the emitters off before it claims the outputs, so the feed starts as from rest. The owner stays `Idle`, so readiness never reports a window |
| A store write | Lit emitters would fail a flash lease's stopped check, which resets the board. So a window holds a flag on the [interlock](../../src/sdk/wellfed-hal/src/interlock.rs) and starts only with no write running, waiting or queued for a writer. A writer that finds the flag set waits and counts itself, and the loop ends the window within a step |
| An install | No window starts or runs while a request or an install holds the flash |
| A Wi-Fi join | The join waits for the window's end, at most 2.4 s. Joins retry often with no network, so a window that gave way to them might never finish |
| A client | Nothing changes. The window claims no outputs, so a settings change is not refused `Busy`, and handshakes go on |
| Bluetooth setup | No window runs in setup mode, and setup's start ends one before Bluetooth starts, since lit emitters would fail the stopped check if a prebuilt Bluetooth or Wi-Fi library wrote flash. Setup ends in a restart, and the first window comes 5 s into that boot. A feeder with no Wi-Fi and no enabled plan runs setup every boot, so it reads only during feeds |
| A trial boot | No window runs until the build has confirmed, since its [confirmation](ota-design.md#what-confirmation-proves) requires the emitters read back off. The light stays dark until then |
| Batteries | No window runs, since the emitters' current is unmeasured, and the level stays as last read. A window ends if the adapter leaves |

**Supervision.** The emitters are under the same timer as a feed's: a non-renewable 1.4 s budget and the 200 ms owner lease. The window publishes them as sensing, so the watchdog's feeder keeps feeding. A feed's stale lease latches a fault, since the motor may be moving. A window's cuts the emitters from the timer interrupt and latches nothing: the loop runs at low priority, the Wi-Fi driver can hold it off past the lease, and a reset is a high price for one reading.

## PB3 is SWD_CLK

The outlet emitter, PB3, is also the SWD clock (`SWD_CLK` in the SDK pin map). `REG_SWD_PMUX_EN` bit 0 resets to 1, giving the pad to SWD. Stock's sensor setup calls the SDK's `sys_jtag_off()`, which clears it, just before configuring PB3.

Stopped-output startup and `Board::open_outputs` release SWD first and read the bit back, and stopped readback requires it clear, so an unreleased PB3 fails startup. The host fake keeps PB3 undriven until release, reproducing the failure. Flashing uses UART, so losing SWD costs nothing.

## The optics probe

The optics probe (`src/sdk/wellfed-probes/src/optics.rs`) proves the emitters, receivers and their supervision with the motor stopped. It runs as its own KM4 task (priority 3), in the bring-up firmware's `optics` variant and as the [QA firmware](qa.md)'s first check:

1. Opens the board: motor STOP, emitters OFF, sensor inputs PA16–PA19.
2. After stopped-output readback, binds a fresh flash authority to the KM0 peer ([boot handshake](bring-up-probes.md#boot-handshake)) and attaches it to the board. No flash hook provider is installed, so no lease can occur; the attachment satisfies the board's rule that an emitter needs an authority.
3. Opens the supervisor: TIM2 every 10 ms, owner lease 300 ms (the motor lease).
4. Runs its plan's emitter windows. Each on window arms a non-renewable budget of its length plus 500 ms. Each 2 ms poll reads PA16–PA19 in one port access and renews the owner lease through `Events::observe`.
5. Ends with a deliberate owner stall: both emitters on, then 1 s without renewing. The timer interrupt must cut them.

| Plan | Policy | Windows |
| --- | --- | --- |
| 1 | `fw13-optics-v1` | A 2 s baseline off, then six cycles of PA15 alone (2 s), PB3 alone (2 s) and both (4 s), each followed by 4 s off: 37 windows |
| 2 | `fw13-optics-v2` | Every window 5 s, three cycles, about 95 s, then the same stall |

The image runs plan 2, whose longer windows suit meter readings. Its start record announces it (`PLOPT1 start created=1 plan=2`), which the startup policy binds both ways. Each policy has its own schedule, so plan 1 evidence still audits.

### Records

`PLOPT1` lines:

- `open board= ready= attached= events= inputs= ms=`
- `window n= emitters=off|a|b|both start= len= samples= gap= edges= dropped= food=unknown|present|low outlet=unknown|clear|blocked stopped=`
- `inputs n= highs=food,outlet,feedback begin= end= first= low_min=`: high counts, levels at the window's start and end, ms to each input's first change, and the shortest complete LOW run
- `edge n= input= level= ms=`: up to 16 per window, the rest counted as dropped
- `stall cut= fault= off= stopped= closed=`, then `done windows=`
- `timeout ms=` or `error kind=supervisor|clock|capture|observe|optics n= fault= stall=`, which end the probe with emitters off. `stall=` is the ms since the last good sample (`-` without a clock); past 300 ms, the task was starved longer than the owner lease.

A window's records are logged at least 300 ms into the next window, so logging never delays settling samples.

A pass has the planned sequence and timing, every gap within the filter's 100 ms, `stopped=1` throughout, a cut 300–340 ms after the last renewal with the OWNER fault, and the supervisor closed. An unattended run also needs the same inputs lit by each selection in every cycle. A blocking run needs the operator's blocking seen through the filter: food both present and low, the outlet both clear and blocked.

## Limits

- **Food passing the outlet.** Stock latches food passage through the outlet beam. The probe resolves LOW runs to about 2 ms, but a real pulse needs motion. Feeds count outlet-beam breaks as a lower bound ([calibration](calibration.md#delivered-food-estimate)).
- **Emitter current, thermal duty and electrical OFF levels** are unmeasured. The probe's duty (40 % over two minutes, each window budgeted) is a bound, not a characterization. Windows at rest add 2 % continuously, about 3.3 % at worst while retried (1.2 s lit, 35 s dark).
- **Real food in the hopper at rest** is unobserved. Windows at rest have run on the research unit: with the beam clear they read low food, and with it blocked by hand the next window cleared the light, with no feed and no fault.
- **Ambient light** on the receivers is unmeasured, and the owner does not yet check for a receiver lit with its emitter off ([jams and sensor faults](jams-and-sensor-faults.md#outlet-and-food-sensors)).

## Evidence

Observed on the research unit, October 21, 2026, over the UART, hopper empty, motor stopped, on the adapter with no batteries. Both runs used plan 1 and passed the probe's auditor, since removed; its reports remain.

- **Unattended run.** PA15 lights the food receiver (PA17) and PB3 the outlet receiver (PA16), in all six cycles. Settling within one 2 ms poll on, 1–3 ms off. The lit food beam read "low" through the filter and the clear chute "clear". PA18 read HIGH throughout, so neither emitter lights it. The longest gap between polls was 21 ms, with Wi-Fi and the transport running. The stall cut came 305 ms after the last renewal, fault 7 (OWNER), with emitters off, the motor at STOP and the supervisor closed.
- **Blocking run.** The operator held paper across each beam on a timed script. The outlet's both-on window inside the block read "blocked", the windows around it "clear". The food line dropped when the block began, and the both-on window inside it read "present". The cut came 300 ms after the last renewal.
- **PB3.** Observed: the first bench run stopped when PB3 failed its enable readback. Startup had not released SWD, so the latch never reached the pad, which read its pull-down. Every earlier "PB3 off" readback reflected the pull-down, not a driven LOW; PB3's OFF level now comes from a driven pin.
- **Food beam with food.** Observed October 29, owner weighing: a feed with food read "present". From a hand-emptied hopper, eight 1-portion feeds read "low" from their start yet dropped 36.8 g, about five portions at 7.11 g ([calibration](calibration.md#evidence)).
- **Mark sensor.** Observed later, with motion: PA18 is the rotor's mark sensor ([motor pulse](motor-pulse.md#evidence)).
- **Optics fault on trial boots.** Observed: the first WPA3 join starved the probe past its owner lease (`stall=703`). The radio now waits for startup ([QA](qa.md#known-issues)).
- **Windows at rest.** Modeled, October 30: `wellfed-sim`'s virtual feeder and `wellfed-hal-feeder`'s owner tests run the real owner, supervisor and interlock against the fake board: a boot's window, a refill, a write, a feed, batteries, a held trial and a stalled loop.
