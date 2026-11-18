# PLWF116 power management

This reference describes the cordless fountain's source detection, battery estimate, charging indications and sleep policy in stock firmware 0.3.62. It is for reviewing a replacement firmware's power contract. All behavior below is recovered from the supplied OTA image; no battery voltage, current, runtime or charging behavior was measured.

Read it with the [board signals](board.md), [stock firmware](stock/README.md) and [communications](stock/communications.md). Addresses are KM4 runtime addresses unless stated otherwise.

## Power sources and source changes

The attributes object begins at `0x10023e14`. Its power fields are:

| Offset | Width | Meaning |
| --- | --- | --- |
| `+1` | Byte | Source: 1 DC only, 2 battery only, 3 DC plus battery |
| `+2` | Byte | Battery state: 0 `CHARGING`, 1 `CHARGED`, 2 `USING` |
| `+3` | Byte | Estimated percentage; unsupported source returns 255 |
| `+6` | Byte | Effective circulation mode: 0 continuous, 1 interval, 2 radar |
| `+0x35` | Byte | Original user circulation mode, retained across the battery override |

Source detection `0x0e018934` reads PA13 twice, with a 20,000-unit delay between reads. The delay chain reaches ROM `DelayUs` at `0x1010a949`, so this is a nominal 20 ms debounce. If the two levels disagree, the helper leaves the previous source unchanged.

| Debounced PA13 | Voltage result | Source selected |
| --- | --- | --- |
| HIGH | Not used for this branch | 2, battery only |
| LOW | Below 3,251 mV | 1, DC only; interpreted as battery disconnected |
| LOW | At least 3,251 mV | 3, DC plus battery |

PA13 is initialized as an input without an internal pull at `0x0e00849c`; reader `0x0e0084c0` uses that GPIO object. These are software meanings. They do not identify the external detection circuit, its voltage tolerance, or whether a depleted attached battery could be classified as disconnected.

GPIO event worker `0x0e019128` handles PA13 edges. It debounces for 20 ms and requests a reset after another 200 ms when the level crosses the remembered source:

- HIGH while the remembered source is not battery means USB unplugged.
- LOW while the remembered source is battery means USB plugged in.

The LOW branch acquires wake lock 24 before the debounce. The HIGH branch releases it. A reset reinitializes the source and circulation policy. This path is not a seamless live change of the pump's power source.

Attribute initialization `0x0e00e2c4` forces radar mode on battery and restores the original user mode on DC. When the original mode is erased, it chooses radar for battery and continuous for DC. The original mode and effective mode are separate fields; a replacement should preserve that distinction if exposing the same behavior.

This is a startup policy, not an immutable battery restriction. Remote settings handler `0x0e016558` writes both mode fields directly from `useWaterType`, with no source gate in that branch. A live remote request can therefore select continuous or interval circulation while on battery. A replacement must decide and enforce its own rule at every mode-change boundary.

## Battery ADC and voltage conversion

ADC initialization `0x0e008124` configures PB2, SDK pin 34. It reads two little-endian calibration words through ROM eFuse reads:

| eFuse bytes | Value | Fallback when the word is `0xffff` |
| --- | --- | --- |
| `0x1d0..0x1d1` | Offset | 2,480 |
| `0x1d2..0x1d3` | Gain divisor | 12,050 |

Offset is held at `0x10020dba`; the divisor is at `0x10020db8`. The image contains the fallback values, not this unit's eFuse contents.

Voltage reader `0x0e0081cc` takes 16 ADC samples, with a 5,000-unit delay after each. It averages by shifting the sum right four bits, then computes:

```text
average = sum_of_16_samples >> 4
reported_millivolts = 2 * (((average * 10 - offset) * 1000) / gain_divisor) + 95
```

The instruction sequence at `0x0e00820e..0x0e00822a` uses unsigned division and integer truncation. It applies the factor of two after the division. The added 95 mV is an explicit software correction, not a measured drop or proof of the divider ratio.

The ADC helper at `0x0e001ce4` starts a conversion, polls readiness without a timeout, stops conversion, and masks the result to 12 bits. Consequently the nominal sampling delay is 80 ms plus conversion and scheduling time. A stuck conversion can prevent the voltage reader from returning.

The reviewed path does not clamp a raw value below the offset before unsigned subtraction, or reject a zero calibration divisor. Erased calibration words get fallbacks; arbitrary damaged words are a different case. A replacement needs bounds and failure handling before using voltage to make a power decision.

## Percentage estimate

`0x0e0187e4` selects a source-specific lookup table. It returns the percentage of the first upper bound that includes the reported voltage. Above the last bound it returns 100. The tables are discrete bins, without interpolation or coulomb counting.

| Percentage | Battery-only upper bound, mV | DC-plus-battery upper bound, mV |
| --- | --- | --- |
| 0 | 3,373 | 3,460 |
| 10 | 3,467 | 3,600 |
| 20 | 3,542 | 3,678 |
| 30 | 3,579 | 3,708 |
| 40 | 3,608 | 3,736 |
| 50 | 3,650 | 3,782 |
| 60 | 3,730 | 3,845 |
| 70 | 3,819 | 3,928 |
| 80 | 3,921 | 4,028 |
| 90 | 4,032 | 4,070 |
| 100 | 4,150 | 4,160 |

The battery-only table starts at `0x0e0b1662`, the DC-plus-battery table at `0x0e0b1542`. Each entry is four bytes: percentage byte, padding byte and little-endian voltage bound.

DC-only source 1 is not mapped and returns 255. A client must not display that as 255% or silently turn it into a full battery.

The Bluetooth Battery Service is separate: callback `0x0e026fd0` supplies a fixed 90, rather than reading these attributes. Its value is not battery-measurement evidence.

The percentage does not establish cell chemistry, pack capacity, usable runtime, voltage accuracy or a safe charging limit. The two tables alone do not establish a hardware charger or battery-protection design.

### Sampling and report cadence

Power service `0x0e0189ec` updates source and percentage. Worker `0x0e0196d8` calls it at startup and every 1,200 visits, with a 50 ms delay per visit: nominally once a minute while that worker runs. Resume callback `0x0e018160` also calls it. Sleep and other work change the effective cadence.

The service creates three relevant timers:

| Timer | Period / delay | Callback behavior |
| --- | --- | --- |
| Periodic percentage report | 43,200,000 ms, 12 hours; repeating | `0x0e0188d0` resamples percentage and requests an attribute report |
| Percentage-change confirmation | 900,000 ms, 15 minutes; one shot | `0x0e0185d8` compares current percentage with last reported percentage and reports a difference |
| Charge-complete update | 300,000 ms, 5 minutes; one shot | `0x0e018560` checks completion input and voltage before changing battery state |

The change timer starts when sampled percentage differs from the last reported value and the timer is idle. It is not restarted at every change. Its callback does not resample voltage; it checks the current stored percentage. This is delayed reporting, not a demonstrated 15-minute stability requirement or voltage hysteresis.

## Charging state and indication

PA12 is initialized as an input without a pull at `0x0e0084d0`. Reader `0x0e00853c` treats HIGH as the charging-complete input.

The power service sets `USING` on battery and DC-only sources. With DC plus battery, it sets `CHARGING` while the completion-seen byte at `0x1002d180` is zero. A HIGH completion input sets that byte and starts the five-minute timer.

At expiry, callback `0x0e018560` requires both PA12 HIGH and voltage above 4,050 mV. It then sets `CHARGED` and requests a report. The callback does not force the percentage to 100. In the reviewed callback there is no failure branch that clears the completion-seen byte or rearms a failed check, so one failed check can leave the state charging until another path resets that state, such as reboot.

These are reporting decisions. The reviewed application does not use this callback to program charging current, charger voltage or a hardware cutoff. The charger IC, pack protection, cell temperature sensing and actual charge termination are **Unknown**.

Battery LED policy `0x0e018648` uses PB23 for white and PB26 for red, with one-second repeating toggle timers at `0x0e018508` and `0x0e018534`. The stock names establish intended colors; physical colors were not checked.

With normal indication enabled and outside factory/sleep-test overrides:

| Stored percentage | Battery only | DC plus battery |
| --- | --- | --- |
| 0–10 | Red steady | Red flashing |
| 11–80 | Both battery lamps off | White flashing |
| 81–100 | Both battery lamps off | White steady when charged, otherwise flashing |

DC-only takes a separate early return in the power service: it starts the red toggle timer and clears white. Unsupported values take the off branch in ordinary LED policy. Factory test and light policy can suppress or supersede the normal policy.

## Low-battery deep-sleep request

On battery only, the power service compares its initial voltage sample against 3,350 mV. A lower result increments the byte at `0x1002d182`; a sample at or above the threshold clears it. It takes the deep-sleep branch when the incremented byte equals 2.

These are two low service visits, not two consecutive ADC samples or a fixed elapsed duration. The service also runs during resume. Its first voltage sample is separate from the later sample used for the percentage update.

The sequence at `0x0e018abe..0x0e018ae0` is:

1. Disable the watchdog through `0x0e036bfc` and `0x0e02d3dc(0)`.
2. Request pump off through `0x0e008558`.
3. Request scale sleep through `0x0e036d74`.
4. Set the deep-sleep flag at the first byte of `0x1002d164` to 1 and its duration word at offset 4 to zero.
5. Disable interrupts, send the structure on IPC channel 31 through `0x0e02b640`, execute two `WFE` instructions, then enable interrupts if execution continues.

KM0 receives channel 31 and selects the deep-sleep path from the request flag. The [flash and boot reference](flash-layout.md) covers that cross-core handoff.

This is a software deep-sleep request, not a demonstrated disconnect of the battery or a guaranteed hardware undervoltage cutoff. The analog detector and pack protection may impose other limits that the OTA file cannot show.

The scale sleep helper tries its mutex without waiting. If busy, it returns without establishing that the scale slept. If acquired, it has an unbounded wait for data-ready LOW. The watchdog has already been disabled when this wait is reached. Thus a failed scale can block the request indefinitely. Pump-off itself uses a software shadow before clearing PA30 and PB4; it is not an unconditional pad-clear operation. These are constraints a replacement should address explicitly.

## Ordinary suspend and resume

Power initialization `0x0e01842c` selects SDK sleep type 1, acquires OS wake lock 0, and registers application callbacks in slot 16:

| Callback | Entry |
| --- | --- |
| Suspend | `0x0e0182f4` |
| Resume | `0x0e018160` |

The explicit idle-sleep entry `0x0e018474` configures Wi-Fi power-save parameters through `0x0e037eb0(10)` and `0x0e037e84(0, 1)`, clears a task-related handle, sets the maximum sleep budget, then releases OS lock 0. Those Wi-Fi helpers construct the driver's `pm_set` requests. They are not GPIO wake-pin configuration calls.

The ordinary maximum sleep budget is **119,000 ms** (`0x1d0d8`), even though the log says 120 seconds. A configured Wi-Fi retry interval replaces it with `interval_ms - 1000`. Sleep-test mode uses its supplied interval times 1,000. These are requested upper budgets; interrupts, pending work and wake locks can shorten or prevent sleep.

### Suspend ordering

The suspend callback first requests scale sleep, then processes pending radar/pump timing. It clears PA26, PA28, PB23, PB22 and PB7. If effective mode is radar, it enables radar through PA16 LOW.

Normal suspend does not unconditionally switch off the pump. The radar/pump timing helper may stop an expired run; the explicit pump-off call is in the sleep-test branch. A held pump-run wake lock is therefore part of the normal policy's correctness. If scale data-ready stays HIGH, the callback can block before it reaches radar timing or lamp changes.

The callback sets the next maximum sleep budget and returns success. It does not read back the pump latch or prove that each external peripheral reached its intended electrical state.

### Resume ordering

Resume acquires lock 16, updates the power service, processes radar/pump timing, requests overdue water sampling, and checks heartbeat reporting. It then updates the maximum sleep budget, runs the Wi-Fi resume/reconnect helper, releases lock 16, and returns success. Sleep-test mode has a separate branch that starts the pump and extends active time.

A radar event can start the pump, set its stop time to `now + sensing_duration + 2`, and extend active time by `sensing_duration * 1000 + 100` ms. Helper `0x0e00d484` holds wake lock 31 while that run is retained. [Stock circulation](stock/README.md) describes its maximum-run and cooling logic.

## Wake locks and work that delays sleep

Acquisition `0x0e02b51c` sets a bit in a mask; release `0x0e02b530` clears it. These are bit flags, not nesting counters. Two independent users of one bit would not get independent ownership from this mechanism. For example, unbinding also acquires bit 16 at `0x0e0161c0`; the label below identifies the reviewed resume use, not an exclusive owner.

The reviewed application uses these identifiable locks:

| Bit / ID | Work held awake | Anchor |
| --- | --- | --- |
| 0 | OS / explicit initial awake period | `0x0e01842c`, `0x0e018474` |
| 16 | Application resume processing | `0x0e018160` |
| 17 | Due water-report sampling | `0x0e00d8ec`, `0x0e00d200` |
| 18 | Due heartbeat | `0x0e019610` and communications worker |
| 19 | MQTT connection | `0x0e01349c` |
| 20 | Scale zero/calibration work | `0x0e036f40` |
| 21 | Interval circulation on phase | `0x0e00d578` |
| 22 | Reset button held | `0x0e019128` |
| 23 | Radar detect active | `0x0e019128` |
| 24 | USB present / plug-in transition | `0x0e019128` |
| 25 | Factory test | `0x0e016558` |
| 26 | BLE provisioning connection | `0x0e026d64` |
| 27 | Water sampling/report work | `0x0e00d200` |
| 28 | OTA transfer | `0x0e0265fc` |
| 29 | Factory aging | `0x0e017388` |
| 30 | Radar configuration retry | `0x0e015a34`, `0x0e016558` |
| 31 | Retained radar pump run | `0x0e00d484` |

This is an application inventory, not proof that all SDK locks, indirect paths or error-path releases are complete. Releasing OS lock alone does not prove sleep occurred. A lack of cloud connectivity does not prove the radio or either CPU is asleep. The [communications reference](stock/communications.md) covers idle timing and serialized periodic/radar reconnect attempts.

## What an implementation must establish

A cordless port needs its own measured power and failure contract:

- Confirm PA13/PA12 polarity and voltage, PB2 divider and calibration, PB4 boost routing, and which rails PA16 and scale sleep actually remove.
- Bound ADC and scale-ready waits. A failed sensor must not prevent pump stop or low-battery handling, especially after watchdog disable.
- Make every stopped state clear both pump and boost unconditionally, with readback where available. Include startup, panic, updates and sleep entry.
- Define source-change behavior, charger-state failure handling, unknown percentage and a stable low-battery policy independently of report timers.
- Measure active, suspended and deep-sleep current, wake latency, battery cutoff and charging behavior on the selected board and pack.
- Establish that pump-run locks, radar events, network work, flash operations and sleep callbacks cannot leave outputs active without a live owner.

No measurement here establishes battery lifetime or electrical safety. The OTA image supplies application policy and SDK machinery, not a schematic, battery datasheet or unit-specific recovery image.

## Evidence

- **Recovered, 2026-11-16:** static analysis of supplied PLWF116 OTA 000362, internal version 0.3.62. Source detection, GPIO event reset, ADC initialization and conversion, lookup-table bytes, timer arguments, charge-state logic, LED policy and low-voltage sequencing were checked against Thumb instruction listings and image bytes.
- **Recovered, 2026-11-16:** application suspend/resume callbacks, wake-lock operations and the Wi-Fi `pm_set` helper chain. The 119,000 ms budget was checked as the image's literal, rather than inferred from its 120-second log.
- **Recovered, 2026-11-16:** scale sleep's unbounded readiness wait and watchdog register writes. No hardware operation, simulation of electrical behavior, current measurement, eFuse read or charging test was performed.
