# Indicators bench check

This guide checks the [indicator lights](indicators.md) on the [relay bench](../devices/plaf103/bench.md) with the owner watching.

> [!WARNING]
> Steps 7, 8 and 10 move the motor. Keep the hopper empty and the D cells out. Step 7 feeds 1 portion. Step 10 feeds 3 portions under the `sensor-fault` variant, as in [the jams checks](motor-bench-checks.md#2-jams-and-sensor-faults).

## Steps

1. Run `wellfed-bench status`, and note the build confirmed on bank 1. You restore it in step 11.
2. Install a `feeder` build through the reviewed update path, and confirm it. Keep a UART capture (or `wellfed logs --follow`) running for the `PLIND1` lines.
3. **Boot.** You should see `show light=lock state=locked` and, once joined, `show light=connection state=online`. Lock and Wi-Fi are lit steady; food and warning are dark. Note each lit icon's color.
4. **Lock.** Run `wellfed lock off`, then `wellfed lock on`. The lock icon goes dark, then lights again, logging `state=none`, then `state=locked`.
5. **Lights setting.** Run `wellfed settings --lights off`: all four go dark. `wellfed settings --lights on` brings back lock and Wi-Fi.
6. **Pairing offer.** Unlock (`wellfed lock off`), press Lock first, then add Dispense, and hold both to open the recovery window. Dispense alone, even a moment early, starts a feed on an unlocked panel. Then start `wellfed recover --address …`. The lock icon blinks about twice a second until the code is keyed in or the offer times out, then returns to its state.
7. **Feed.** Run `wellfed feed --portions 1`. The food icon blinks once a second while the feed runs, and goes dark after its result. Check the feed's result and trace as usual: the lights must not change either.
8. **Refused press.** On an unlocked panel, press Dispense to start a feed, then again about 3 s later while it runs. The second press is refused `busy`, and the warning icon flashes quickly for about 3 s.
9. **Setup mode.** On an unlocked panel, hold Lock and Mic. The Wi-Fi icon blinks once a second until setup mode reboots after 15 minutes with no phone; `wellfed reboot` ends it sooner.
10. **Steady fault.** Install the `sensor-fault` variant ([jams and sensor faults](jams-and-sensor-faults.md#fault-injection-variants)) and wait for its confirmation. Feed 3 portions: the feed holds a fault, and the warning icon is lit steady and red.
11. **Restore.** Reinstall the build from step 1, wait for its confirmation, and lock the panel with `wellfed lock on`.

Watch especially for `fail light=fault`: PB22 is the only pad with input readback, so only its check can catch an input mismatch. A `PLIND1 fail` line means the lights stayed off for that boot. That is the designed response, not a pass.

## Next steps

Record which steps passed, each icon's color and any `PLIND1 fail` line in the [indicators evidence](indicators.md#evidence).
