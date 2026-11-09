# Alerts on the feeder

This page explains how the feeder decides what is wrong and how a client reads it. It is for contributors. An alert only reports: no rule feeds, retries or changes anything.

The feeder is the only thing that knows what is wrong, so it decides. It keeps a small table of open alerts in RAM, and every client reads that table, so a phone, the CLI and a server can't disagree. `wellfed alerts` prints it. The Android app catches up with the table at home and notifies each alert once, clearing the notification when the feeder clears the alert ([alerts at home](../../src/app/README.md#alerts-at-home)); the server doesn't read it yet ([roadmap](../roadmap.md)). Neither judges an alert: the phone's one judgement of its own is that it can't reach the feeder.

## One failed meal

Say a meal jams at 08:00.

1. The feed ends `Failed`, as it always would. The plan, the owner and the journal never look at alerts.
2. The loop records the `Fed` event. The recorder tells the rules as it queues the event, so the rules see it even if the queue is full.
3. The rules open `MealFailed` with the feed's number, plan, portions and reason, and the table's generation moves.
4. `Status.alerts_generation` carries the new generation, so a session sees the change on its next status and reads the table with `GetAlerts`.
5. 48 hours after the event's own time, the report lapses from the table.

## Two kinds of alert

A **condition** opens once its state has held past its threshold, and closes when the state ends. A **report** opens because something happened, and lapses 48 hours after its own time, or sooner if room runs out. A report with no time stays until room runs out. A later good meal doesn't undo a failed one.

| Alert | Kind | Source | After a restart |
| --- | --- | --- | --- |
| `MealFailed`, `MealUncertain` | Report | `Fed` at the recorder, `Failed`, `Expired` or `Uncertain` | The boot read |
| `MealMissed` | Report | `Missed` at the recorder | The boot read |
| `MealNoRecord` | Report | A plan's latest due time against the schedule's mark, 25 minutes on, with a valid clock. A skipped meal is at the mark, so it never opens this | Worked out again |
| `NoClock` | Condition | No local time for 1 hour of uptime: no valid clock, or a valid one with no zone offset | The timer starts again |
| `CannotFeed` | Condition | Readiness, or the reason the schedule holds a meal, for 30 minutes. A plan's claim the writer hasn't finished after 30 s opens it sooner, with the reason it blocks | The timer starts again |
| `PlansStuck` | Condition | With local time, for 25 minutes, plans still loading or a change to them never finishing, so no meal can be judged. Readiness takes nothing from this | The timer starts again |
| `FoodLow` | Condition | The food beam's latest reading | Live |
| `FoodLikelyOut` | Condition | Five portions confirmed since the beam read low. There is no empty reading. A feed whose own reading went low counts none of its portions, since the beam may have read low at its end | The count comes back from the boot read |
| `DesiccantDue` | Condition | The desiccant count at 0 or below | Live |
| `BatteryBelowFloor` | Condition | The owner's verdict on the pack | Live |
| `PowerWasOut` | Report | A `Supply` event with the adapter out, once an outage. A feeder with no pack writes no such event, so it reads as unreachable | The boot read |
| `RolledBack` | Report | `UpdateAtBoot`, against the boots around it ([below](#two-kinds-of-alert)) | The boot read |
| `UpdateFailed` | Report | `UpdateStop`, except a cancel | The boot read |
| `RepeatedRestarts` | Tracked like a condition | Three boots that count ([below](#two-kinds-of-alert)) within 6 hours, over after a quiet day. A boot with no time counts as recent | The boot read |
| `ClientJoined`, `ClientRevoked` | Report | `Enrolled`, `Revoked` | The boot read |

**How a boot got there.** A restart loses RAM, and an activation or a give-way keeps the write turn until the reset, so nothing queued before it could reach the ring. The update store's record does get there: `Pending` is written before a give-way clears bank 1's marker, and `Activated` before an activation programs its marker. So once in every boot, the loop records what the store held, as `UpdateAtBoot`, and the rules judge the boot from it. Time plays no part, so a slow install is no rollback, and a later stop takes nothing back.

| This boot | The boot before | Means |
| --- | --- | --- |
| Bank 0, `None` | Bank 1 | A fall with nothing pending, such as a reset storm: `RolledBack` |
| Bank 0, `Activated` | Anything but bank 0 with `Activated` | The activated build gave way, or never ran: `RolledBack`, once |
| Bank 0, `Pending` | Bank 1 | An install giving way |
| Bank 1, `Activated` | Bank 0 | An install's activation |

- A store that can't be read shows `None`, so a fall then reports `RolledBack`: a false alert costs less than silence.
- A boot with no earlier boot in the read judges nothing.
- A boot whose record never reached the ring has no verdict of its own. The next boot's record judges from what the store holds then, and the earlier boot counts as a restart by its flags.
- A boot on bank 0 with no record waits for the next boot to show what bank 0 runs. A record from bank 0 says its build records, and the boot lost its record as above. Otherwise bank 0 runs an older build, from before `UpdateAtBoot`, and the judgement is reduced ([below](#what-clears-an-older-format)).
- A marker changed by the bench rather than by the update stage reads as a fall.

**What counts as a restart.** A boot counts towards `RepeatedRestarts` if `BOOT_Reason` has its system, watchdog, brownout, KM4 system or KM4 watchdog bit set, except:

- an install's give-way and activation boots, as in the table;
- the boot after a `Restarting` event, though never the boot that recorded it. The loop records the event as it asks for a reboot over the device API, setup mode's end or the factory reset's restart. The writer appends what is queued in the restart's grace, before it saves the totals. If the event still doesn't land, the boot counts.

So faults count, a stale brownout bit counts, a restart after Wi-Fi fails to start counts, and so does a reset in the middle of an install. A power-on with no flags set doesn't.

**No local time.** Without a valid clock, or with one but no zone offset, the feeder works out no due times and holds every plan ([schedules](schedules.md)), so a meal it isn't running stays silent until `NoClock` opens at 1 hour. The second case is a cold boot that NTP sets with no zone rule stored: NTP carries no offset, and a client's sync kept its offset in RAM. The app stores the phone's zone on a feeder it has just paired, but a feeder paired from the CLI, or whose owner cleared the zone, has none. Once local time returns, a meal held past its grace is reported missed.

**A skipped meal.** A client's `SkipMeal` moves the plan's mark up to the meal it skips ([schedules](schedules.md)), so the meal never runs and is never reported missed. The rules judge it as one the schedule already passed, so it opens no meal alert of any kind. The `Skipped` and `Unskipped` events open nothing either.

## Identity

A client needs to tell whether two reads show the same alert. A report's key comes from the event's own fields, as the rules see an event before the writer numbers it, and a dropped event never gets a number.

| Alert | Key |
| --- | --- |
| A finished feed | The read's epoch and the feed's number, whatever its outcome. An uncertain feed can record a second `Fed`: a failed or uncertain one replaces the alert, and a clean one leaves it |
| Missed, or no record | Plan and due time, so a `Missed` replaces no record of the same occurrence |
| Client joined, revoked | Kind, client and the event's time |
| Power out, update failed, rolled back | Kind and the event's time |
| A condition, repeated restarts | Kind alone |

The epoch holds for a boot, so a read carries it once rather than in each entry.

## Reading the table

`GetAlerts`, in the device API's shared core, is answered `Alerts(AlertsView)`: the whole table in one frame, with `epoch`, `generation`, `more`, `settling` and up to 12 entries, oldest first. Owner, member and server sessions may read it. An update-signer session is refused `Forbidden`, as it is for every op outside the update path. The status carries `alerts_generation` (`Status`), so a session sees a change on its next status. Each entry goes on the wire behind its own length, so a client of an older build skips an alert kind it doesn't know and counts it (`unread` in the client's view) rather than failing the whole table. `wellfed alerts` prints the table as JSON.

- **`generation`** changes whenever anything in the view does. It counts from 0 at each boot, and a session doesn't outlive a boot.
- **`more`** says a report left for want of room, or the boot read didn't reach all of the ring, and what is missing could still have been listed.
- **`settling`** is set until the boot read has ended and the thresholds that began at boot have run out, and while a reading a condition needs is unknown: the desiccant's days with no clock, the food beam before its first reading, the verdict on a fitted pack, or local time (no valid clock, or one with no zone offset). With no local time the feeder closes `PlansStuck` because its threshold can't run, not because the plans settled, so the table doesn't vouch for that absence; the feeder counts it as an unknown reading rather than keep the alert open, since a restart empties the table anyway.

While either flag is set, a client doesn't take an absent alert to mean it ended, though not for good. `settling` is one flag for the whole table, so it can't say which condition it holds back, and a reading that stays unknown (the desiccant's days with no clock) keeps it set as long as the clock is missing. The app takes an absent condition as ended once tables have left it out for 2 hours, twice the longest threshold that starts again at a boot, so a cleared `FoodLow` doesn't stay posted while the clock is unknown. `DesiccantDue` and `PlansStuck` wait for the clock (the desiccant's days, and the plans' threshold that runs from local time's arrival), so while the feeder has no local time (its status shows no valid clock, or the table lists `NoClock`, which also covers a clock with no zone offset) they aren't ended this way, and their 2 hours count from the first read with it. A condition held absent longer than 2 hours by another unknown reading (the food beam before its first reading, a fitted pack's verdict) does come off, and posts again if the feeder reopens it. An absent report ends once it is past the feeder's own 48 hours: by its time, or, made with no valid clock, by how long tables have left it out.

**When room runs out.** There are at most 7 conditions in 12 slots, so a condition always fits. A report pushes out the oldest of the least urgent reports: `Soon`, then `Security`, then `Urgent`. A report less urgent than all of them is dropped itself.

## After a restart

The table is RAM. Once the ring is open, the loop passes the newest 128 records it held at that moment to the rules, oldest first, up to 32 a pass. Later records are this boot's, and reached the rules through the recorder. On bank 1 the device API welcomes no session until the read ends, or for at most the boot's first 10 s, so the read shares few turns with them. Bank 0 holds no session back: it is the recovery bank, there for installs, and an installer reconnecting after a give-way is welcomed as soon as the ring opens. There the read takes its turns among the sessions'. The read gets 10 s of its own from the ring's opening, and the rules go on without it if the ring hasn't opened after 60 s.

A read that didn't reach everything leaves `more` set for as long as what it missed could still have been listed. If it gave up, or found no time, that is a whole 48 hours. If the 128 records were all it read of a longer ring, that is 48 hours less the age of the oldest time it found, since everything it missed is older.

History in the ring is mostly over, and the clock is seldom valid yet at this point. So the rules judge it against the newest time the read has shown:

- A report already past its life stays out, and one that lapses leaves without counting as room running out.
- Restarts the times show were followed by a quiet day are dropped. With no times, the false alert stands: it costs less than silence.
- What this boot already said stands. A report the read finds with an older time doesn't replace one with the same key, and a `Supply` event the read finds changes nothing once this boot has recorded one. A live report with no time can't be told apart, so an older timed one still replaces it.
- A time more than a day past a valid wall clock, or with none valid more than a year past the newest time seen, or past any time a clock can be set to, doesn't count as the newest. The report it came on still opens.
- This boot's own `Boot` and `UpdateAtBoot` reach the recorder first, and wait for the read to end, because a bank and a run of restarts only mean something in order.

## What clears an older format

`alerts_generation` moved from the feeder's status to the shared one, and a server's store keeps the last status it read as JSON in `feeder_state.status`. A row written before won't read back. On a server that has already held a status, reset its store, or clear that column. The ring needs nothing: the new events come after the old ones, old records read as they are, and old boots have no update record, so they aren't judged. Flash the firmware and build the clients from one source together.

Boot judgement reads bank 0's own `Boot` and `UpdateAtBoot` records, and bank 0 keeps the build it was written with: the update path never rewrites it. A bank 0 older than this build records `Boot` alone, so the rules can't tell a give-way from a fall. They judge it from the boots around it instead. A boot on that bank 0 counts as no restart. If the boot before it on bank 1 was an activation that never recorded `UpdateConfirmed`, the new build never confirmed, so the bank 0 boot is a fall and reports `RolledBack`, once. Any other is an install giving way, so the activation boot after it is an install. A build confirms within its activation boot, with no restart, so the loop records that as `UpdateConfirmed`: a second install from the same boot then gives way from a confirmed build, which is no fall. So one install counts as no restart, and so do two in one boot. This judgement misses:

- a fall from a build that had already confirmed;
- faults while bank 0 runs, which count as no restart;
- a confirmation whose `UpdateConfirmed` never reached the ring before the next give-way, which reads as a fall. Rewriting bank 0 with this build over UART, the bench step, restores the full judgement.

## Why feeding is unaffected

The rules read, and open or close RAM entries. No stage that decides or runs a meal reads the table, and readiness takes nothing from it. While a feed is queued or driven or a plan's claim is in flight, the boot read and the look at each plan's meal wait, so a drive's gaps see no flash read. The conditions go on being looked at, once a second, or a claim that never finishes would freeze them. No alert writes flash. The new ring records are `UpdateStop`, `UpdateAtBoot`, `Restarting` and `UpdateConfirmed`, which the loop writes whatever the rules are doing. They go through the event queue like any other, so the writer appends them under the write turn, never during a drive.

The rules live behind `Platform::alerts` so the one recorder can reach them, which leaves them within reach of every stage that holds the platform. Only the rule stage and the device API's read may use it. Review enforces that, not the compiler.

## Evidence

- **Modeled, November 7, 2026.** On the simulator's loop, a meal skipped through the device's own skip path runs 2 hours past its time with no feed, no missed meal and no alert, and stays so after a restart (`a_skipped_meal_opens_no_meal_alert`). A boot on bank 1 welcomes no session while its read runs, and a boot on bank 0 welcomes one at once (`only_bank_1_holds_sessions_for_the_boot_read`).
- **Modeled, November 1, 2026.** On the `wellfed-feeder-sim` loop, the same seeded scripts of meals, presses, faults, an emptied hopper and restarts record identical rings, `Fed` events included, with the rules and without (`the_same_script_records_the_same_history_with_the_rules_and_without`). Those scripts run no client session. A separate run with a client that reboots the feeder and connects straight back, following the history through the boot read, ends in the same meals with the rules and without (`a_client_across_a_restart_changes_no_meal`). The simulator's ring serves every reader at once, so that doesn't exercise the writer's one read slot.
- **Modeled, November 1, 2026.** The rules take 16 B an entry and 336 B in all on the host (`an_entry_and_the_whole_state_stay_small`). What they cost in stack and code is in each image's audits.

None of this has run on a feeder, and what the boot read costs at boot has not been measured.
