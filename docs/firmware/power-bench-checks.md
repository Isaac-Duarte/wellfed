# Power bench checks

This guide checks the feeder's [power](power.md) handling on the [relay bench](../devices/plaf103/bench.md): readings, the floor, the brownout hold and battery mode. Most checks need cells or a bench supply at the pack terminals, which is the owner's call.

Motion stays out of the early checks. Check 3 feeds under load; check 5 presses Dispense once on an empty hopper and lets a scheduled meal run.

## 1. Readings at rest

1. Fit three fresh D cells, adapter in.
2. `wellfed stats --section power` and a subscriber's `Supply` sample read `Reading` near the meter, with `below_floor: false`.
3. Pull the adapter with relay 1: one `Supply` event (`adapter: false`), readiness stays `Ready`.
4. Restore it: one more event within a minute.

## 2. The floor without motion

1. Swap the cells for a bench supply at the pack terminals, adapter out, at 4.5 V.
2. Step down. Readiness turns `Battery` within a poll of the pin reading under 1650 mV for 300 ms (about 3.40 V).
3. Step up. It returns to `Ready` only at about 3.53 V.
4. Step down past the floor again. Note the voltage of any reset and whether the next boot's cause is `Brownout`: that is the margin the floor rests on.

## 3. Under load

> [!NOTE]
> This needs an authorization covering motion with cells or a supply in.

1. Put the analyzer on the motor and log the supply voltage.
2. Feed one portion on fresh cells to measure the real surge and drive sag.
3. Repeat just above the floor.

The figures replace the simulator's zero sags and confirm or move `FLOOR`.

## 4. Brownout hold

1. With the adapter out, lower the supply until a brownout reset. Readiness stays `Battery`; the UART shows `PLBAT2 hold brownout`.
2. Leave the supply there 3 minutes and note the pin reading. It must stay under 1890 mV (50 %), or the release's premise fails.
3. Raise it to 4.5 V. The hold ends 3 minutes later (`PLBAT2 hold released by=pack`).
4. Repeat steps 1–2, then fit the adapter: it ends at once (`by=adapter`).

## 5. Battery mode

1. Fresh cells, adapter in, `wellfed events --follow` running, UART captured.
2. Pull the adapter with relay 1. Within 5 s, `wellfed status` shows `Power: on batteries` with the charge, and an update offer is refused `battery`.
3. After 5 s: `PLBAT2 radio off` and `PLWIF3 … kind=off`, the session drops, and the feeder leaves the access point's client list.
4. Empty hopper, unlocked panel: Dispense feeds one portion.
5. Restore the adapter: `PLBAT2 radio on`, `kind=on`, a join, and the CLI reconnects. Events show the `Supply` changes and the feed.
6. Stay on batteries across a scheduled meal: it runs from the running clock.

Variations:

- **Short episode.** Pull for about 20 s, under the access point's idle timeout. `kind=leave` reads 0 (not still associated), and the next join isn't rejected with status 30.
- **During a join.** Pull while joining, and again during a rejected join (wrong password via the hotspot). The task settles 2 s, then logs `kind=off`, with no watchdog reset.
- **Cycles.** about 20 pulls, a minute or more apart. Every `wifi_off` and `wifi_on` returns, no hang or reset, and the heap's low water (`wellfed stats`) doesn't fall cycle to cycle.
- **Discovery.** Afterwards `wellfed` finds the feeder over mDNS without its address.

## 6. The brownout bit is fresh

1. After check 4's brownout, fit the adapter.
2. Force a watchdog reset with a build that stalls the watchdog, such as `owner-stall`. `wellfed reboot` is deliberate and doesn't count.
3. The next boot must not log `PLTRL1 brownout=1`, and its `Boot` flags must not carry bit 18.

If they do, the bit is stale here. The [guards on a stale bit](power.md#brownout) still hold; record it.

## 7. No cells

1. No cells, adapter in: `wellfed status` reads `Power: adapter; no batteries`, with nothing about a floor.
2. Fit cells: the charge shows within 5 s.
3. Remove them, cut relay 1, restore it.
4. The history's `Supply` event at the cut reads `no batteries`, not a voltage.

## Next steps

Record results in the [power evidence](power.md#evidence) with their level and date. If the floor or release moves, change `wellfed_hal::power::FLOOR` or the hold's constants and the power page together.
