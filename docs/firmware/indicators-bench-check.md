# Indicators bench check

This guide checks the [indicator lights](indicators.md) on the [relay bench](../devices/plaf103/bench.md) with the owner watching. Only step 7 moves the motor, under the bench rules: hopper empty, D cells out, at most 1 portion.

## Steps

1. Install a `feeder` build through the reviewed update path and confirm it. Keep a UART capture (or `wellfed logs --follow`) running for the `PLIND1` lines.
2. **Boot.** You should see `show light=lock state=locked` and, once joined, `show light=connection state=online`. Lock and Wi-Fi are lit steady; food and warning are dark. Note each lit icon's color.
3. **Lock.** Run `wellfed lock off`, then `wellfed lock on`. The lock icon goes dark, then lights again, logging `state=none`, then `state=locked`.
4. **Lights setting.** Run `wellfed settings --lights off`: all four go dark. `wellfed settings --lights on` brings back lock and Wi-Fi.
5. **Pairing offer.** Unlock (`wellfed lock off`), press Lock first, then add Dispense, and hold both to open the recovery window. Dispense alone, even a moment early, starts a feed on an unlocked panel. Then start `wellfed recover --address …`. The lock icon blinks about twice a second until the code is keyed in or the offer times out.
6. **Setup mode.** On an unlocked panel, hold Lock and Mic. The Wi-Fi icon blinks once a second until setup mode reboots after 15 minutes with no phone; `wellfed reboot` ends it sooner.
7. **Feed.** Run `wellfed feed --portions 1`. The food icon blinks once a second while the feed runs and goes dark after its result. The feed's result and trace must not change.
8. **Refused press.** On an unlocked panel, press Dispense to start a feed, then again about 3 s later while it runs. The second press is refused `busy`, and the warning icon flashes quickly for about 3 s.
9. **Steady fault.** With the `sensor-fault` variant installed ([jams and sensor faults](jams-and-sensor-faults.md#fault-injection-variants)), a 3-portion feed holds a fault. The warning icon is lit steady and red.

Watch especially for `fail light=fault`: PB22 is the only pad with input readback, so only its check can catch an input mismatch. A `PLIND1 fail` line means the lights stayed off for that boot, the designed response, not a pass.

## Next steps

Put the normal `feeder` back if you installed the variant. Record which steps passed, each icon's color and any `PLIND1 fail` line in the [indicators evidence](indicators.md#evidence).
