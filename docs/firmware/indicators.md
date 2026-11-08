# Indicator lights

The panel has four single-color lights, one behind each icon. The indicator service decides what each light shows when several things want it at once, and checks every change it makes. If a light cannot be trusted, it turns them all off rather than show a wrong state.

The service is host-tested, and its colors and transitions passed the [bench check](indicators-bench-check.md) with the owner watching (October 28). Which states show is still provisional.

## The lights

From the front, left to right: low food, lock, Wi-Fi, error/warning. Each light has its own pad, lit HIGH.

| Light (`wellfed_core::indicator::Light`) | Icon | Color | Pad | Input readback |
| --- | --- | --- | --- | --- |
| `connection` | Wi-Fi | White | PA0 | None |
| `food` | Low food | Red | PA2 | None |
| `lock` | Lock | White | PA4 | None |
| `fault` | Error/warning | Red | PB22 | Yes |

Port A's low pads read LOW whatever is driven on our unit, so only PB22 can be read back ([startup](startup-diagnostics.md#shared-pad-power)). The pads come from the board's `wellfed_board::pins` ([device layer](device-layer.md)).

## How the service decides

`wellfed_core::indicator` is `no_std` and allocation-free, with no clock or task of its own.

- **Requests** name a light, a pattern (`Off`, `Steady` or `Blink { on_ms, off_ms }`), a priority, an optional duration and the requester's label. `Indicators::show` returns a `Handle`, and `cancel` takes it.
- **Who wins.** Per light, the highest priority, then the newer request. The rest wait underneath, so a cancelled or expired winner reveals the next. An `Off` request holds a light dark over lower ones.
- **Fixed table.** `Indicators<S, SLOTS>` is an array, and a full table refuses (`Refused::Full`). So are a blink phase under 50 ms (`MIN_PHASE_MS`) and a duration of 0 or half the clock's range.
- **Handles** carry a 64-bit sequence number that never repeats in a boot, so a stale handle cancels nothing, even after its slot is reused.
- **Frames.** The loop calls `frame(now)`: expired requests end, and the loop gets each light's level and the ms until the next change. Blinks start lit and re-anchor at whole periods, so their phase survives the millisecond clock's wrap.
- **Claims.** `Claim` holds one request while a condition lasts: shown when it starts, unchanged (same handle and phase) while it holds, cancelled when it ends, replaced when what it wants changes.
- **Driver.** `Drive` writes a light only when its level changes, waits the board's 10 ms settle, then checks it (`Lights::check`). A refused write or a level that does not hold turns every light off for the rest of the boot. The lock, feeding and the device API are unaffected.

## Never in a feed's way

The service runs in the loop task and adds no task: the product firmware uses two of the four a firmware gets. A step is a few comparisons over at most a dozen slots; each light that changes costs one masked port write under the interrupt lock and, a settle later, one readback. It takes no motor or flash lease.

The board's indicator writes (`wellfed_hal::leds::show`, checked by `leds::shown`) do not require stopped outputs, unlike the probe's `leds::request`, because the lights must work while a feed runs the motor and emitters. What keeps that safe:

- The write is one masked write of indicator bits under the interrupt lock, so it cannot interleave with the supervision timer's motor writes.
- A compile-time assertion keeps the indicator masks disjoint from the motor and emitter masks.
- A host test drives every light during a simulated feed and checks that no motor or emitter bit moves.

The check runs up to once per blink edge per light, so it holds the interrupt lock only for five register reads: both ports' directions, the latch, one input sample and the pad. It keeps the latch, pad, direction and (on PB22) input checks, but takes no stopped-output snapshot and none of the probe's 64 extra samples. The write holds the lock for the direction reads and one port write, the longest the supervision timer can wait (static analysis; each read is an SDK or ROM call of a few instructions).

A steady light that drifts is noticed only at its next change, and only some drifts: a lost direction refuses the next write and a reconfigured pad fails the next check, darkening the lights, but a flipped latch is just overwritten and checks clean, so it is never reported.

Startup still enables the indicators only from verified stopped outputs and leaves them LOW.

## What the feeder shows

`wellfed_feeder::feeder::lights` maps the feeder's states to the lights, in a provisional mapping that exercises the service:

| State | When | Light | Pattern | Priority |
| --- | --- | --- | --- | --- |
| `fault` | The feeding owner holds a fault, or its journal failed | fault | Steady | 90 |
| `updating` | An install runs | connection | Blink 150/150 ms | 80 |
| `refused` | A Dispense press was refused (3 s) | fault | Blink 150/150 ms | 70 |
| `setup` | BLE setup mode runs | connection | Blink 500/500 ms | 70 |
| `pairing` | An offer waits for a Dispense press or its code | lock | Blink 250/250 ms | 70 |
| `battery` | The battery holds feeds | fault | Blink 1000/1000 ms | 60 |
| `low-food` | The food beam's latest reading is low | food | Steady | 10 |
| `locked` | The panel is locked | lock | Steady | 10 |
| `online` | The feeder has an address | connection | Steady | 10 |

- **Settings, as in stock.** With `lights` off, or outside the lights window on the local clock, every light is dark, faults included. The requests still hold, so the lights come back as they were. While the clock is invalid the window is open.
- **Low food** shows the beam's latest reading, from a feed or a [window at rest](optical-inputs.md#reading-the-food-beam-at-rest). On the adapter it lights within seconds of a boot and clears within a minute of a refill. On batteries no window runs, so it holds the last reading until a feed.
- **A feed** shows nothing of its own. The food light is the panel's red low-food light, so blinking it through a feed would read as a low-food warning.
- **Missing time** has no indication yet; it waits for the light policy ([product architecture](product-architecture.md)).

The loop steps the lights after readiness each pass. A change in what a light shows is logged once, not per blink edge: `PLIND1 show light=lock state=pairing`, or `state=none`. A failure is `PLIND1 fail light=… check=refused|input-mismatch|abort`.

## Evidence

- **The bench check.** Observed (October 28, owner watching, `feeder` `f1e4655c…` on bank 1, then the `sensor-fault` variant for the steady fault): every step passed, with no `PLIND1 fail`. Lock and Wi-Fi lit steady at boot; the lock went dark on unlock and back on lock; the `lights` setting darkened all four and restored them; the lock blinked through a recovery offer that timed out; Wi-Fi blinked in setup mode; low food blinked through each 1-portion feed (the feed's blink, removed since) and went dark after its result; a second Dispense press during a feed was refused `busy` and flashed the warning for 3 s; a held sensor fault lit the warning steady, and a later press changed nothing. Low food and warning are red, lock and Wi-Fi white.
- **Pins and polarity.** Observed: each light on its pad, lit HIGH, from the startup indicator probe with the operator watching, and the owner's observation of the icon order (October 24). PB22 is red.
- **The service.** Modeled (October 26): `wellfed-core/tests/indicator.rs` (priority, ties, cancellation, stale handles, expiry and blink phase across the clock's wrap, refusals, claims, the driver's settle-and-check and failure), `wellfed-hal/tests/leds.rs` (writes with the motor and an emitter on, latch and direction failures) and `wellfed-fw-feeder/tests/lights.rs` (the mapping against the fake GPIO: each state's pad, priorities, the 3 s refused flash, the setting, low food steady through a feed with motor bits untouched, a failure darkening all four).
- **Builds.** Fresh `device` and `firstboot` `feeder` builds passed their link, RAM-park and stack audits (loop stack 5,412 of 8,192 B).
