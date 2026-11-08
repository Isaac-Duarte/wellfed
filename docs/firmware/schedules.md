# Feeding schedules

The feeder keeps up to ten feeding plans and runs them itself, with no server. This page explains what a plan is, when one runs, and how the feeder makes sure a meal runs at most once through restarts, clock jumps and daylight changes. It is for owners who want to know what to expect, and for anyone changing the schedule code.

Plans, the meal reminder and the meal call have all run on our feeder. So has the autumn daylight change: on November 1, 2026 a 01:30 meal ran once, in daylight time, and not again in standard time. The spring change, power cuts between a claim and its feed, and skipping a meal are tested in simulation only. Status is on the [roadmap](../roadmap.md).

## Plans

A plan feeds a number of portions at a time of the feeder's local day, on the weekdays it lists. A plan that lists no weekday runs once, at its next occurrence, then disables itself. It stays in its slot so you can enable it again.

| Field | Limits |
| --- | --- |
| Time | 00:00 to 23:59, local time |
| Portions | 1 to 48, as a manual feed |
| Days | Any of Monday to Sunday, or none for once |
| Enabled | On or off |
| Meal call | 0 to 5 plays of your clip before the meal; 0 for none ([below](#meal-call)) |

The feeder has ten slots, as stock keeps at most ten plans. The slot number is the plan's name in the history and in `wellfed plans`.

```sh
wellfed plans list
wellfed plans add --at 07:30 --portions 2 --days daily
wellfed plans edit 0 --days mon,wed,fri --call 2
wellfed plans disable 0
wellfed plans delete 0
```

Owners and members may change the plans, as they may change the settings. Each change reads the plans, edits that table and writes it back against the generation it read. If another client changed the plans meanwhile, or a one-off plan disabled itself, the write is refused `Stale` and nothing changes: read them again and retry. The command never retries by itself.

A plan you add or enable again, or whose time or days you change, runs only from its next occurrence after the save. Saving a plan at 07:35 for 07:30 does not feed until tomorrow. Changing only a plan's portions or meal call keeps its occurrences as they were.

## When a plan runs

The feeder needs a valid clock with a zone offset to run plans. Until a time source sets the clock after a cold boot, plans hold and manual feeds still work ([clock](clock.md)). An NTP fix carries no offset, and an offset from a client's sync lives in RAM. So a feeder with no zone set holds every plan after a power cut, even once NTP has the time right. The app stores the phone's zone at pairing, gives a feeder with no offset its own at each connection, and says the meals hold until then.

Each occurrence goes through the [missed-meal rule](clock.md#missed-meal-rule):

- **On time.** It runs at its minute, through the same feeding owner, readiness and journal as a Dispense press.
- **Late, within 15 minutes.** After a restart, a forward clock jump, or a wait for the feeder to be ready, it still runs once.
- **Later than 15 minutes.** It never runs. The history records it as `Missed`, with the reason.
- **Already run.** A backward clock jump or a restart never runs it again.

A plan due while the feeder cannot feed waits: during another feed, a Wi-Fi join, an install, BLE setup or a battery hold. It runs as soon as the feeder is ready, within the 15 minutes. On batteries the feeder keeps running plans from its running clock, with Wi-Fi off ([battery mode](power.md#battery-mode)). Plans due at the same minute run one after another, by slot.

A feed a plan started is a `Fed` event by `schedule`, with the plan's slot. A missed occurrence is a `Missed` event:

| Reason | Meaning |
| --- | --- |
| `late` | The feeder was off or had no time, or the clock moved past it |
| `blocked` | It waited past 15 minutes; the event names what blocked it |
| `interrupted` | The feeder stopped after taking the occurrence and before the feed was reserved |

A plan saved while the feeder had no time has no starting point, so it starts once the clock returns. An occurrence before then, late or not, passes without a `Missed` event, since it may have fallen before the save.

## Skipping a meal

You can skip one of today's meals that hasn't come due yet, and undo the skip until it does. A skipped meal doesn't feed, isn't reminded, plays no call and isn't reported missed. The plan itself doesn't change, so tomorrow's meal runs as usual. Owners, members and the server's session may skip and undo, as they may change the plans.

```sh
wellfed plans skip 2      # skip plan 2's next meal, if it falls today
wellfed plans unskip 2    # bring it back, until its time
```

The CLI finds the meal from the plans, the feeder's clock and its zone, so you never type a time. `wellfed plans list` shows a skip beside its plan. `wellfed server plans skip` and `unskip` do the same through the server (`POST /feeders/{id}/meals/skip` and `/meals/unskip`), and the server's next meal passes over a skipped one.

A client sends `SkipMeal { slot, due_s }`, naming the plan's slot and the meal's due instant, and `UnskipMeal` with the same two ([device API](device-api-ops.md#time-and-calibration)). Each is answered once stored, and recorded as a `Skipped` or `Unskipped` event with its client. `GetPlans` names each slot's skipped meal while its skip stands, until the plan's next meal is taken or passed, so a skip in the past is spent.

- **Only today's next meal**, still to come by the feeder's clock and zone. A meal already due belongs to the schedule, so an undo at or after its minute is refused and the meal stays skipped.
- **One meal at a time.** Skipping a meal twice, or undoing one that wasn't skipped, is refused.
- **After the last one settles.** While the plan's previous meal is still running or waiting (up to its 15 minutes), a skip is refused `Busy`, since it would spend that meal unseen.
- **A plan that runs once** is disabled by the skip and enabled again by the undo.
- **Changing the plan drops the skip** when the change starts it afresh: a new time or days, or enabling it again. Changing only the portions or meal call, or disabling the plan, keeps it. Undoing a disabled plan's skip leaves the plan off.
- **Only a skip can be undone.** A meal that ran or was missed can't be brought back, even if the clock moves back before it.

**How the undo stays safe.** A skip moves the plan's [mark](#at-most-once) up to the skipped meal, so the schedule, reminder and call treat it as already run, and sets the slot's skip bit. The undo needs the bit and the mark still at the meal. It moves the mark back to the second before the meal and clears the bit. Any other change that moves the mark (taking a meal, passing a missed one, a save that starts the plan afresh) clears the bit too. Without the bit, a meal that ran or was missed would look like a skip once the clock moved back, and undoing it would feed it twice. Consecutive occurrences are at least 23 hours apart, even across a daylight change, so no occurrence lies between the old mark and the meal, and an undo never lets a meal run that wouldn't have run without the skip.

**A skip never races its meal.** While a skip, or any plans change, waits for the writer, the schedule, reminder and call wait too, so a meal that comes due meanwhile is judged against the new mark. That is usually one write, but a change queued behind a feed waits for the feed. A reminder or call whose lead passes meanwhile is not given for that meal, which still feeds once, within its grace. A call already playing when a skip lands plays to its end, and the meal doesn't feed.

## Meal reminder

The feeder can announce each scheduled meal ahead of time, so the app, the server or a home automation can tell you food is coming. It records a `MealSoon` event on the event stream. It plays no sound and sends no phone notification itself. The reminder is off until you set it:

```sh
wellfed settings --meal-reminder 10    # 10 minutes before each meal
wellfed settings --meal-reminder off
```

The lead is 1 to 60 minutes, one setting for every plan. Stock kept a similar field in its cloud (`feedingPlanAdvanceNoticeTime`, 5 or unset) with no unit we could find, so the range is ours.

A `MealSoon` event names the plan's slot, the time the meal is due (and its minute of the feeder's local day) and the portions it will ask for. The feeder sends one for each occurrence:

- **When the lead starts.** With a 10-minute lead, a 07:30 meal is announced at 07:20. A plan you save, or move, inside the lead is announced at once.
- **Only for a meal the feeder expects to run.** A disabled plan, a feeder without a valid clock, or a plans store it cannot write gets no reminder.
- **Once.** Changing only a plan's portions keeps the reminder already sent, and the meal feeds the new portions. A change of time or days is a new occurrence, with its own reminder.
- **Across a daylight change**, the lead counts real minutes to the meal's instant. A 03:10 meal on the spring change day, with a 30-minute lead, is announced at 01:40, just before the clock jumps.
- **Never withdrawn.** A meal can still be missed after its reminder: blocked past the grace, held, or passed by a forward jump. Disabling, deleting or moving a plan inside the lead leaves its `MealSoon`, and a move adds a second. A forward jump across the lead gives no reminder at all. So a client acting on a reminder should check the plans, or the `Fed` or `Missed` event that follows for the same plan and due time.
- **Again after a restart.** It keeps nothing on flash, so a restart inside the lead announces the same meal again once the clock is back, with the same plan and due time. A client can drop the repeat.

The reminder only reads the plans. It never claims, delays or blocks a meal, so the meal runs exactly as it would with the reminder off.

## Meal call

The feeder can call your pet before a scheduled meal with a clip you record: your voice, a bell, anything up to about 8 seconds. You upload one clip, and each plan says how many times it plays before its meal. Stock did the same with a clip recorded in its app; stock's own clips are not reused.

```sh
wellfed call upload dinner.wav     # convert and upload the clip
wellfed plans edit 0 --call 2      # play it twice before plan 0's meal
wellfed call show
wellfed call play                  # hear it once on the feeder
wellfed call delete
```

- **The clip.** The CLI converts a WAV file (integer PCM or 32-bit float, any rate, mono or stereo) on your computer. It mixes it to mono, resamples it to 8 kHz, trims the silence at either end and sets its peak just under full scale. Convert another format to WAV first, for example with `ffmpeg -i dinner.m4a dinner.wav`. A clip longer than about 8 seconds once trimmed is refused before anything is sent. An upload replaces the stored clip, which is gone as soon as the upload starts. Only owners may upload or delete it.
- **When it plays.** The call starts its own length ahead of the meal (every play, plus a second), so it ends as the meal is due and the meal runs on time. Stock played its call first and then ran the motor, so its meals ran late by the call's length.
- **Never in the way.** Like the reminder, the call only reads the plans, and never claims, delays or blocks a meal. With sound off, outside the sound window, at volume 0, with no clip, or with the speaker busy, the meal runs silently. A call not started by the meal's minute is dropped. A client's test sound, or hearing the clip on request (`wellfed call play`, or **Play on the feeder** in the app, which any client may ask for), is refused `Busy` if it would still sound as the call's lead begins.
- **Which meals.** Those the reminder announces, and again after a restart inside the lead. No call plays during BLE setup, or while the feeder cannot feed at all this boot (its feeding owner stopped on a fault, or its feed journal did not open). A feed you start yourself plays no call.

The clip lives in storage sectors 40 to 47 (`wellfed_feeder::store::call`): a header and the stream. The header is written only once the whole stream reads back against its CRC, so a power cut during an upload leaves no clip, never a damaged one. Each boot checks the stream again. The [factory reset](pairing.md#factory-reset) erases the clip.

## Daylight saving and time zones

Plans keep local time across daylight changes, from the zone's rule or a client's sync ([clock](clock.md#time-settings)):

- **A skipped time runs as far past the change as it was meant to be.** In spring, 02:30 runs at 03:30 when the clock jumps from 02:00 to 03:00.
- **A repeated time runs on its first pass.** In autumn, 01:30 runs once, in daylight time.

This is the rule RFC 5545 and JavaScript's Temporal call "compatible". Half-hour and quarter-hour zones work; stock rounded its zone to whole hours. The feeder works out each plan's occurrence once a minute, so a new zone applies to plans within a minute.

## At most once

The feeder never runs an occurrence twice, even if power fails at any moment. Each plan keeps a mark: the latest occurrence already run, missed, skipped or before the plan was saved. Nothing at or before the mark runs.

Running an occurrence takes two steps:

1. **The claim.** The writer saves the occurrence, the plan's slot and the feed journal's newest number. The claimed occurrence counts as the plan's mark from then on.
2. **The feed.** Only once the claim is on flash does the loop hand the feed to the feeding owner. Every other feed waits meanwhile, so the claimed feed is the very next number the journal reserves. A storage write still in its way delays the feed for at most 3 s. Refused past that, the occurrence is given back and tried again, like any refused feed.

A cut between the two leaves a claim whose feed was never reserved. The next boot finds the journal where the claim left it, records the occurrence `interrupted`, and does not feed. A cut during the feed is the journal's usual case: the next boot resolves it `uncertain` and never runs it again. The event names the plan, since the claim says the feed was the plan's, and carries the occurrence's due time. The boot resolves the feed before any source has set the clock after a power cut, so the claim's due time is the only time it has; the feed began within the grace after it.

If the feeding owner refuses the claimed feed (a join began in between, say), the loop gives the occurrence back and claims it again 5 seconds later, until the 15 minutes run out.

A plans store the feeder cannot write runs nothing, since it could not keep the marks.

## How it differs from stock

| | Stock | Wellfed |
| --- | --- | --- |
| A plan due during another feed | Waits, with no limit, even across a restart | Waits up to 15 minutes, then is missed |
| A meal missed while off | Not caught up, not reported | Run if at most 15 minutes late, otherwise reported |
| Field checks | None ("99:99" is stored) | Every field checked |
| Time zone | Whole hours | Any offset, with daylight rules |
| A one-off plan after it runs | Deleted | Kept, disabled |
| A sound before the meal | Per plan; plays first, then the motor runs | Per plan; ends as the meal is due, never delaying it |
| A reminder before the meal | The vendor's cloud and app; nothing in the firmware | A `MealSoon` event on the feeder's event stream, 1 to 60 minutes ahead |
| Skipping one meal | Not offered | Today's next meal of a plan, undone until it is due |

## Storage

The plans live in storage sectors 11 and 12 (`wellfed_feeder::store::plans`): one 256-byte record, major 4, holding the ten plans (a plan's meal call and its skip bit in its flag byte), each plan's mark and the last claim, and 128 spare bytes for per-plan grams and more plans ([record versions](pairing.md#record-versions)). Only the settings writer writes it, inside a marked attempt, after a feed has released the outputs. A save happens for each change you make and for each occurrence run or missed. A record of a newer major makes the store read-only with no plans, so a build runs nothing it can't read. A record this build can't read at all (an older major, or one that fails its checks) leaves it with no plans too, and is never written over; the [factory reset](pairing.md#factory-reset) erases it.

> [!NOTE]
> Major 4 gave the record its spare room. A build of major 4 finds an older plans record unreadable: it opens corrupt with no plans and never writes over it, so no meal runs until a [factory reset](pairing.md#factory-reset) erases the record and the owner adds the plans again (app or `wellfed plans add`). The marks and the last claim go with the old record, so a plan added again runs from its next occurrence after the save, and the feed journal still stops a claimed feed from running twice. A build from before major 4 goes read-only on a major 4 record rather than misread it.

The [factory reset](pairing.md#factory-reset) erases the plans, last claim included. The feed journal it keeps still stops a claimed feed from running twice.

## Evidence

- **Simulated, November 5, 2026: skipping a meal.** `wellfed-feeder-sim`'s `tests/schedules.rs` on the loop: a skipped meal neither fed, reminded, called nor was missed, and an undo restored all three; undos refused at the meal's minute, and after the clock moved back past a meal that ran, was missed or disabled its once plan; a skip surviving a restart; a skip being written holding the schedule, reminder and call. `tests/skip_api.rs` over TCP: answered once stored, named in the plans and the history, and an undo landing a second after the meal answered `Done` with the meal fed once. `wellfed-service`'s `tests/plans.rs` through the server: the next meal moved, a retry answered from the journal, a second skip refused, another client's undo seen. `wellfed-feeder`'s `tests/skip.rs` and `tests/plans_store.rs`: the skip bit cleared by a claim, a settled claim, a pass and a fresh save, kept by a save keeping the times, and reloaded; the checks across both 2026 Central European daylight changes, a weekday plan, a plan saved without a clock and a once plan.
- **Observed, October 29, 2026** (owner listening, hopper empty): the meal call at volumes 10, 50 and 100, each played before its meal, which then fed on time ([speaker](audio.md#evidence)). A settings write made during a feed was refused `Busy`.
- **Observed, October 28, 2026** (evening, unattended, hopper empty): a meal reminder set 5 minutes ahead of a plan arrived on time, a reboot inside the lead repeated it once, and the meal fed on time. A plan calling a clip twice played it and then fed on time. With sound off, the next plan logged `result=muted` and fed on time.
- **Observed, October 28, 2026** (`fp03-ota-20261028`, hopper empty): a daily plan fed once at its minute; a reboot within that minute, with the clock carried across it, ran nothing again; a plan set to run once fed and disabled itself; a plan due while the feeder had no clock after a cold boot held, and the sync 16 minutes later recorded it missed, unfed.
- **Simulated, October 28, 2026.** `wellfed-feeder-sim`'s `tests/schedules.rs` runs the product loop in virtual time over the real stores and feed journal: plans through restarts, jumps, a cold boot without time, plans due together, a wait past the grace, cuts before and during the feed, refused feeds, one-off and timeless plans, and Denver's daylight changes; the reminder (off by default, edits and restarts inside the lead); and the call (once between its lead and a second before the meal, none without a clip or call). `wellfed-feeder`'s `tests/call_store.rs` covers uploads chunk by chunk, a bad CRC, the longest clip, deletes, a changed byte and a power cut anywhere. `wellfed-feeder-sim`'s `tests/conformance.rs` uploads and deletes a clip on both platforms.
