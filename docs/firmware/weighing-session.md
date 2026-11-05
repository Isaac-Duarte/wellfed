# Weighing session

This guide measures what one portion weighs on your feeder and stores it, so the feeder can estimate delivered food ([calibration](calibration.md)). It is the thorough version of the owner guide's [weighing a portion](../guide/everyday-use.md#weighing-a-portion).

You need food, a kitchen scale reading to 0.1 g (1 g works over more portions) and the feeder on its latest build. Each step is a supervised feed through the device API. Keep hands away from the outlet. You can cut power at any time; the feed then reads as uncertain and is never repeated.

## Steps

1. **Prime.** With the hopper full, run `wellfed feed --portions 1` and discard what falls. This fills the compartments and leaves the rotor at a mark. `wellfed calibrate` should show `"phase": "at_mark"`.
2. **Full hopper.** Tare the empty bowl on the scale, or note its weight. Run `wellfed feed --portions 5` twice. Each `Fed` event should say `completed: 5`. Note the sum of `completed` and the bowl's net weight.
3. **Low hopper.** Feed until the hopper is near empty: `wellfed stats --section food` reports `present: false`, or the level is just above the rotor. Repeat step 2's 10 portions into the tared bowl.
4. **Phase effect (optional).** Run `wellfed feed --portions 3`, and about 5 s into its drive run `wellfed cancel-feed N` from a second terminal, with the number `wellfed feed` printed. Discard what falls. Then run `wellfed feed --portions 1` into the tared bowl. `PLOWN1 marks` (or `wellfed stats --section portions`) shows the homing, and the weight shows what a homed feed drops against a normal portion.
5. **Enter the result.** Use the full-hopper run, unless the two runs differ by more than about 10 %; then enter their mean and report both. `wellfed calibrate --weighed 98.4 --portions 10` stores the mean, here 9,840 mg.
6. **Check.** Run `wellfed calibrate`. It should show the weight you stored, here `"portion_mg": 9840`.

Other forms: `wellfed calibrate --portion 9.8` enters one portion directly, `--clear` forgets it, and `wellfed calibrate` alone prints the calibration, the phase and the portions statistics.

Record per run the hopper level, each feed's `completed`, `outlet_breaks` and `drive_ms`, and the net weight. A spread of more than about 10 % between runs at the same level means the compartments fill unevenly, and the estimate is only as good as that spread.

## Still to check

1. After an install, `wellfed calibrate` answers `"portion_mg": null`, `"bowl": "single"` and a phase.
2. A ten-portion low-hopper run, if the owner wants one. The run-down in the [evidence](calibration.md#evidence) already shows portions near 7 g until the last few.
3. Bank 0's role install leaves sectors 34–35 alone. This is still open. A give-way and an install over the air already kept the weight (October 29).

## Next steps

Record results in the [calibration evidence](calibration.md#evidence) with their level and date.
