# Desiccant reminder

The hopper's desiccant pack keeps the food dry and needs replacing every few weeks. The feeder counts the days since you last replaced it and reports when it is due, with no server and no cloud. This page is for owners and anyone changing the reminder. The count has run on our feeder; status is on the [roadmap](../roadmap.md).

## Using it

```sh
wellfed desiccant --reset              # after you put in a fresh pack
wellfed desiccant                      # where the count stands
wellfed settings --desiccant-days 21   # the period, 1 to 90 days, 14 by default
```

Through the server: `wellfed server desiccant [--reset]` and `wellfed server settings --desiccant-days`. Owners, members and the server may record a replacement, as they may change the plans, and the period is a setting like any other: a change names the generation it read.

## The count

The feeder keeps the period and the last replacement's date in its settings, never a running count, so the reminder survives any restart and costs no flash write as days pass.

Days are the feeder's local calendar days. A replacement on day D with a period of P days leaves P days that day and P − n days n days later. It is due on day D + P, at 0, and keeps counting below zero, as stock's cloud did. A daylight change makes one day 23 or 25 hours long, but the count still moves by one at each local midnight.

The status reports one of:

| State | Meaning |
| --- | --- |
| `unset` | No replacement recorded yet. Record one once the pack is fresh. |
| `unknown` | The clock is invalid, so today's date is unknown. |
| `remaining` | `days` left of the period, and `due` from 0 on. |

After a cold boot the count is `unknown` until a time source sets the [clock](clock.md). With no zone and no offset from a sync, local time is UTC, so a reset made before the zone is set may count one day off once it is.

## Recording a replacement

A replacement is dated by the feeder's clock, never the client's. While the clock is invalid the reset is refused `NoClock` and nothing is recorded, since a guessed date would make the count wrong for weeks: sync the time (`wellfed time`) and try again. A reset is answered `Done` with the settings' new generation and records a `Settings` event. If the clock moved back after a reset, so the recorded day is after today, the count shows the whole period until that day passes.

## How it differs from stock

| | Stock | Wellfed |
| --- | --- | --- |
| Where it lives | The vendor's cloud | The feeder |
| Works with the app alone | No | Yes |
| Period | 14 days observed; range unknown | 1 to 90 days, 14 by default |
| Below zero | Keeps counting | Keeps counting |
| Notification | Phone push from the cloud | Due state in the status. Alerts are planned ([deferred](product-architecture.md#deferred)) |

## Storage

The period and the replacement day are fields of the settings record, version 4, in storage sectors 2 and 3 ([device API](device-api-ops.md#settings-wi-fi-and-the-lock)):

| Bytes | Field |
| --- | --- |
| 44 | The period in days, 1–90 |
| 45 | 1 when a replacement is recorded, else 0 |
| 46–47 | Zero |
| 48–51 | The replacement's local day, days since 1970-01-01 (i32), zero when none |

A record of an older version reads as erased: the settings open on their defaults and the next save replaces it.

## Evidence

- **Observed, October 28, 2026** (evening, unattended, hopper empty): the reminder read `not started` after an install. A reset was refused before the clock was set and accepted after it, with 14 days left. A period of 1 day then read 1 day left, and a reboot kept the period and the date. No server ran.
- **Modeled, October 28, 2026.** `wellfed-core/tests/desiccant.rs` (the count through zero, a reset after today, the local day with and without an offset, Denver's daylight changes); the settings store's tests (fields saved and reopened, a period out of range refused, a bad field corrupt, a power cut at every byte); and `wellfed-client`'s tests, the conformance suite on both platforms, `wellfed-feeder-sim` across a restart and the server through its journal (a replacement recorded, refused without a clock, the count reported).
