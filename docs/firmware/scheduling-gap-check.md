# Scheduling gap bench check

This guide measures how long a Wi-Fi rejoin holds the feeder's tasks, with the `gap-probe` variant on the [relay bench](../devices/plaf103/bench.md). It repeats the measurement behind [scheduling gaps](resource-budget.md#scheduling-gaps-across-wi-fi-rejoins). No motor moves.

## The `gap-probe` variant

`wellfed-build build --device plaf103 --profile firstboot --firmware feeder --variant gap-probe` builds the normal `feeder` firmware with two additions (`src/firmware/wellfed/feeder/src/gaps.rs`, wired in `app.rs`):

- **A probe task,** `wellfed-gap`, at priority 4 with a 256-word stack. It wakes every 10 ms, reads the 32 kHz system timer, and records the gap since its last wake into atomics. It never touches the owner, the board, flash or the watchdog. It is the firmware's third task of four. The stack audit bounds it at 304 B of 1 KiB, and the loop at 5,056 B of 8 KiB in this variant.
- **The loop measures itself** the same way: each pass's start is a wake, and the delay the previous pass asked for is the expected gap.

Gaps are timed on the system timer rather than the FreeRTOS tick, so a driver section that masks interrupts (and with them the tick) still shows its full length.

**No feeds.** The variant's `Feeds::submit` returns `Unavailable` without calling `Feeding::submit`, which is compiled out of its path. The device API's `Feed` is refused, and nothing else submits a feed. The feeding owner still opens at rest, so the loop carries its usual load and the beat runs as in the product.

**Rejoins come from the host.** The firmware cannot force one: the Wi-Fi shim in `example_entry.c` owns the connection, and its C interface has only start and an event ring. A disconnect call would change the C interface for a measurement. Taking the bench hotspot down and back up makes the shim lose the link, back off, and run a full scan, connect (the WPA3 SAE handshake) and DHCP when it returns. The first join after each boot is another sample.

## Records

Every 10 s (`WINDOW_MS`), at the end of a loop pass, the loop logs three lines with the same `t`:

```
PLGAP1 p4 t=<ms> w=<ms> n=<wakes> max=<ms> late=<ms> p99=<ms> over=<a>/<b>/<c> worst=<ms>
PLGAP1 loop t=<ms> w=<ms> n=<passes> max=<ms> late=<ms> p99=<ms> over=<a>/<b>/<c> worst=<ms>
PLGAP1 net t=<ms> gen=<n> att=<n> lost=<n> dns=<n> rejoin=<0|1>
```

At startup it logs `PLGAP1 task created=1`, or `=0` if `spawn` refused the task.

| Field | Meaning |
| --- | --- |
| `t` | Uptime when the window closed |
| `w` | How long the window ran, by the loop's clock: more than 10,000 when the loop itself was held |
| `n` | Gaps measured in the window (wakes after the first) |
| `max` | The longest gap between two wakes, the asked delay included |
| `late` | The longest time past the asked delay. For `p4` that is 10 ms. For the loop it is what the previous pass returned, and lateness includes the pass's own run time, since the owner's beat sees that too |
| `p99` | The 99th percentile of lateness, rounded up to its bucket's top (exact below 32 ms, then powers of two: 63, 127, 255, …) |
| `over` | Wakes later than 50, 200 and 1,000 ms |
| `worst` | The longest `max` since boot, so a gap between captures loses nothing |
| `gen` | The Wi-Fi generation, bumped by every join (DHCP address) and every loss |
| `att` | The join attempt of the last successful scan |
| `lost`, `dns` | Losses since boot: every loss, and those of them the link check noticed (lease or gateway) |
| `rejoin` | 1 when `gen` changed during the window: the link was lost or joined |

The shim's own `PLWIF3` records (scan, connect, DHCP, loss, backoff, with their durations) fall between the `PLGAP1` lines and show where in a window a join ran.

**Reading it.** For each rejoin window, compare `p4 late` with `loop late`. If the priority-4 task stays far under 1 s while the loop's lateness reaches hundreds of milliseconds, priority helps. If both are held about as long, the hold is in a driver section above priority 4 or with interrupts masked, and joins must be held during a feed instead.

## Repeating the measurement

The feeder must be powered and on the replacement firmware, with the hotspot up for the install, which goes over the device API. `wellfed-stock serve --device plaf103` is not needed.

1. Install over the air: `target/release/wellfed-bench ota NAME-install --build --firmware feeder --variant gap-probe`. It builds and freezes the review, offers it, and captures until the build confirms. Over UART instead, freeze the same build with `wellfed-build review build`, then use `wellfed-bench flash-prepare` and `flash-execute` as the [development updater](../development/uart-development.md) describes.
2. Capture in back-to-back 180 s segments: `target/release/wellfed-bench capture NAME-01 --seconds 180`, then `-02`, and so on. Each segment holds about two rejoin cycles.
3. Force 10 rejoins from another shell, with the hotspot connection named in `private/hotspot.json`:
   ```
   HOTSPOT=$(jq -r .connection private/hotspot.json)
   trap 'nmcli connection up "$HOTSPOT"' EXIT   # an interrupted loop leaves the hotspot up
   for i in $(seq 10); do sleep 60; nmcli connection down "$HOTSPOT"; sleep 15; nmcli connection up "$HOTSPOT"; done
   ```
   After a loss the shim backs off from 1 s, doubling up to 30 s between attempts, and a scan waits up to 10 s. So a rejoin lands 15–45 s after the hotspot returns. The 60 s quiet time leaves at least one quiet window between rejoins.
4. Add first-join samples with cold boots: `wellfed-bench boot NAME-boot-01 --seconds 180`, with the hotspot up.
5. Read each `PLGAP1 net … rejoin=1` window, the `p4` and `loop` lines before it, and the `worst` values at the end. Count rejoins from `gen`, which a loss and a join each bump. You should see `gen` rise by about 20, and both lines in every rejoin window.

## Next steps

- Restore. Check the hotspot is up (`nmcli connection show --active`). Reinstall the build that was on bank 1 before: its frozen review (`wellfed-bench ota NAME-restore --review private/verification/<prior review>`), or `--build --firmware feeder` from the same `master` commit, so the restored identity is the recorded one. Put the hotspot and `serve` back as you found them.
- Record what ran and the restored build id, and add the results to the [resource budget's evidence](resource-budget.md#evidence) with their level and date.
