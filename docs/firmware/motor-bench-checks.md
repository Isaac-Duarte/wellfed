# Motor bench checks

This guide reruns the motor checks on the [relay bench](../devices/plaf103/bench.md): the stalled loop and failed stop from [motor and feeds](motor-and-feeds.md), and the jam and sensor-fault variants from [jams and sensor faults](jams-and-sensor-faults.md). Every check moves the motor, and they run unattended under an authorization covering motion.

## Before any variant check

The stall, failed-stop, jam and sensor-fault checks share these conditions and this setup:

- Hopper empty, D cells out, at most 3 portions a feed.
- The logic analyzer at the motor (D2, D3), recording all eight channels at 1 MHz.
- Installs go over the air to bank 1, with no bank-0 write.

1. Run `wellfed-bench status`. You should see relay 1 on, the UART relays closed, no capture or server running, the hotspot up, and the normal `feeder` build confirmed on bank 1.
2. Note the newest feed's number from the last line of `wellfed feeds`.

For each feed in the checks below:

1. `wellfed-bench capture NAME-uart --seconds 180` starts a UART capture.
2. `sigrok-cli --driver fx2lafw --config samplerate=1M --time 60000 --channels D0,D1,D2,D3,D4,D5,D6,D7 --output-format binary --output-file private/captures/NAME-la.bin` starts the analyzer.
3. `wellfed feed --portions N --address <feeder>` runs the feed.
4. Watch the UART for the expected records, then read the `Fed` event with `wellfed events --address <feeder>`. In a long case the board resets mid-feed, and the client loses its session without a `Fed` event: the feed is uncertain and is never sent again.
5. After a reset, check that the next boot shows `PLOWN1 open` before anything else runs, and before the next feed. If arming was skipped because a store write was in progress at the reset, power-cycle with `wellfed-bench boot` so the owner opens and the pending feed resolves.

A flat analyzer capture through a drive the firmware reports is a failed capture, not evidence.

## 1. Stalled loop and failed stop

About 45 minutes: three over-the-air installs and four feeds, each driving at most about 10 s. The expected results are in the [variants table](motor-and-feeds.md#a-stalled-loop-and-a-failed-stop-during-a-feed).

1. Run `wellfed-bench ota NAME --build --firmware feeder --variant feed-stall` from the main checkout's root (or with `--root`). Wait for the build to confirm. A reset during the trial would count against it, so run no feed before the confirmation.
2. Feed 1 portion, then 2.
3. Install `stop-fail` the same way, wait for the confirmation, then feed 1 portion, then 2.
4. Restore the normal build at once (see [Restore](#restore)). A variant injects into every feed, whatever starts it.

> [!WARNING]
> Stop if a drive is still running 1 s after it should have ended: power off at once with `wellfed-bench off` (or `boot`, which cuts power itself), then report. That is 3 s after `PLOWN1 state=4` for the stalls and the short failed stop, and 11 s for the long failed stop. Any motion after a reset is the same.

You should see one drive per feed on the analyzer (D3 HIGH, clockwise), ending about 2.2 s in for both stalls, about 2.3 s for the short failed stop, and at the reset for the long one (2 s plus at most about 7 s). The UART shows the injection record, the stop or reset, and the expected `fed` record on the right boot. There is no second drive after a reset, and no `PLINJ1 stall … end` or `hold lifted` after a long case.

## 2. Jams and sensor faults

About an hour: three over-the-air installs and six feeds, the longest driving about 12 s. The expected results are in the [variants table](jams-and-sensor-faults.md#fault-injection-variants).

1. Run `wellfed-bench ota NAME --build --firmware feeder --variant feed-jam`, and wait for it to confirm. Run no feed before the confirmation.
2. Feed 1, 2 and 3 portions. Check each outcome, reason and `reversals=` against the table. On the analyzer you should see forward (D3 HIGH), back (D2 HIGH) for 1.0 s and stopped 0.5 s, plus the 50 ms deadtime each way.
3. Install `sensor-fault`, wait for the confirmation, then feed 1, 2 and 3 portions:
   - For 1, `PLOWN1 pins` shows PA18 toggling (the real marks) while `PLINJ1 clear … edges=` counts the dropped interrupts.
   - For 2, the analyzer shows no drive.
   - For 3, after the `Fed` event, `wellfed status` reports readiness blocked `Fault`, `wellfed events` shows a `FeedingBlocked`, and the UART shows no `PLSUP1 stop` for 2 minutes. Then `wellfed reboot`: the next boot is Idle and ready, with `FeedingBlocked` cleared. The variant came in by an over-the-air restart, so this power cycle has had a reset and its recovery restart stays off: left alone, the hold must last past 10 min.
4. Restore the normal build at once (see [Restore](#restore)).

> [!WARNING]
> Stop if the motor drives 1 s past its expected end, or at all after a reset: power off (`wellfed-bench off`, or `boot`), then report.

**What it measures.** The independent stop while the owner stalls is already measured: the TIM2 lease ended both stalled drives 0.18–0.2 s after the stall began ([motor and feeds](motor-and-feeds.md#evidence)). This check measures the controller's own stop:

- In the 2-portion jam, every forward drive after a pause reads the jam from its first sample. Its length at the motor, less the 150 ms surge window, is the time from detection to the drive's end: two 20 ms polls and the step that applies STOP, expected 190–250 ms in all.
- The reversal and pause lengths check the controller's timing against the analyzer's clock.
- In the ADC case the drive ends within one poll (about 20 ms) of the variant's arming, which the UART's `PLINJ1 show` dates only to the host's timestamps.

## Restore

After each variant check, and before anything else:

1. Install the normal `feeder` with `wellfed-bench ota NAME --build --firmware feeder`, and wait for its confirmation.
2. Run `wellfed-bench status`. You should see the normal build confirmed on bank 1, the hotspot up, and no capture or server running.

## Next steps

- Record results in the [motor evidence](motor-and-feeds.md#evidence) or the [jams evidence](jams-and-sensor-faults.md#evidence), with their level and date.
