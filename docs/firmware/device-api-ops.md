# Device API operations

This reference lists what each [device API](device-api.md) operation does, who may send it and what refuses it, plus what a feed records and the live samples. With each op it covers the firmware behind it: the settings writer and its record layout, Wi-Fi join and fallback, the lock's box and the hopper beam's filter. It is for contributors writing a client or changing the firmware. The types are in [`wellfed-proto`](../../src/product/wellfed-proto/src/api.rs). Every write is remembered by its id and checked against its role first ([roles](device-api.md#writes-roles-and-generations)), and a client's store write during a feed is refused `Busy` ([why](device-api.md#no-flash-write-while-the-motor-may-move)).

## The shared core

Every kind of device serves these.

| Op | Notes |
| --- | --- |
| `Status` | Answered `Status`: the shared fields and the kind's part. A feeder's is `DeviceStatus::Feeder(FeederStatus)`: the lock, the settings' generation, the food beam, the last feed fault, the desiccant count, what blocks feeding, and the judged supply. Every kind's carries `alerts_generation`, which changes whenever its alert table does |
| `Alive` | Answered `Done`, to any role. The keepalive: frozen, so a client keeps a session with a feeder of any build alive ([evolution](device-api.md#evolution)) |
| `PutWifi { base, wifi }` | `None` clears. Owners only |
| `TimeSync` | An owner's may step the clock. Anyone else's may only narrow it, and is refused `Time` when it disagrees with a valid clock beyond both error bounds |
| `CreateInvite { role }`, `CreateBoundInvite { role, key }`, `ListClients`, `RevokeClient { key }` | `RevokeClient` is owners only. `CreateBoundInvite` (revision 2) is an invite only the static key `key` can redeem |
| `Reboot` | Owners only. Refused `Busy` while a feed is in flight |
| `GetTime`, `PutTime { base, changes }` | `GetTime` is answered `Time(TimeView)`. `PutTime` is owners only |
| `Update`: `Offer`, `Chunk`, `State`, `Cancel`, `EnrollKey`, `KeyStatement` | Owners only. Refused `Unavailable` by a build without an installer. Frozen, with their answers, so any later client updates a bank 0 of any build ([evolution](device-api.md#evolution)) |
| `Stats { section }` | One section of the statistics. The food, feeds and portions sections are a feeder's ([statistics](device-api-diagnostics.md#statistics)) |
| `ReadBank1 { offset }` | Answered `Bank1(Bank1Page)`. Owners only, served only from bank 0 ([stock backup](device-api-diagnostics.md#stock-backup)) |
| `TailLog { after }` | Owners only, answered with `Log` batches ([debug log](device-api-diagnostics.md#debug-log)) |
| `GetAlerts` | Answered `Alerts(AlertsView)`: the device's table of open alerts, whole, or `Unavailable` from a kind that keeps none. Owners, members and the server may read it. `Status::alerts_generation` changes whenever it does ([alerts](alerts.md)) |

## The feeder's module

`Op::Feeder(FeederOp)`, answered `Outcome::Feeder(FeederOutcome)` or a refusal. Another kind refuses it `Unavailable`.

| Op | Notes |
| --- | --- |
| `GetSettings`, `PutSettings { base, changes }` | A stale `base` is refused `Stale { current }` |
| `SetLock`, `Play` | |
| `Feed { portions }` | 1 to 48 portions, any other number refused `Portions { asked }`. Answered `Feeding { feed }` once the owner has reserved it; the result is a `Fed` event ([feeds](device-api.md#feeds)) |
| `CancelFeed { feed }` | `Done` while that feed runs, `NotFound` once it has ended |
| `FeedTrace { feed, offset }` | Answered `FeedTrace(TracePage)` ([feed traces](device-api-diagnostics.md#feed-traces)) |
| `GetCalibration`, `PutCalibration` | `PutCalibration` is owners and members only |
| `GetPlans`, `PutPlans { base, plans }` | `GetPlans` is answered `Plans { generation, writable, plans, skipped }`, where `skipped` names each slot's skipped meal (Unix seconds) while its skip stands. `PutPlans` replaces all ten slots; a plan whose extension tail the build can't read is refused `Invalid` ([whole-replace writes](device-api.md#everything-else)). A plan's `call` is how many times the meal call plays before it, 0 to 5 |
| `SkipMeal { slot, due_s }`, `UnskipMeal { slot, due_s }` | Skip one of today's meals still to come, or undo the skip until it is due |
| `ResetDesiccant` | Records a desiccant replacement today by the feeder's clock |
| `GetCall`, `StartCall { bytes, crc }`, `CallChunk { offset, data }`, `DeleteCall`, `PlayCall` | The meal call's clip. `GetCall` is answered `Call(CallView)`. The writes are owners only; `PlayCall` is any client's |

## What a feed records

`Fed` carries the feed's number, the portions asked for (`requested`) and those delivered (`completed`, one per rotor mark the feedback sensor confirmed), an outcome, a reason, the hopper reading and who asked (`by`). The outlet beam's breaks are a lower bound on food passing, not a count, so they stay in the log and the statistics ([calibration](calibration.md#delivered-food-estimate)).

The **outcome** is `Done`, `Cancelled`, `Expired` (the feeder couldn't get ready in time), `Failed`, or `Uncertain`: the owner stopped without a durable result, or a reset interrupted the feed and the next boot resolved it.

The **reason** names what ended it ([jams and sensor faults](jams-and-sensor-faults.md)). No failure is retried. The full set is `FeedReason`. Every reason other than `Done` and `Cancelled`:

| Reason | Means |
| --- | --- |
| `Jammed` | A jam three reversals didn't free (stock's code 2031), or one that held while backing off |
| `Overcurrent` | One reading over the ceiling, or the limit held while the motor is stopped |
| `OutletBlocked` | The outlet beam read blocked. Mid-feed the feed ends with the portions delivered so far and is never resumed, where stock pauses up to about 10 minutes. At the start it ends with no motion |
| `NoFeedback` | No rotor mark for too long |
| `MarkStuck` | The mark sensor stuck at a mark |
| `FeedbackStream` | The feedback sensor's edges couldn't be trusted |
| `Sensor` | The ADC or the beams couldn't be read, or current at rest was over the jam limit |
| `Output` | An output failed to apply or verify |
| `ObservationGap` | The owner wasn't stepped often enough ([why](device-api.md#handshakes-and-handshakes-during-a-feed)) |
| `Timeout` | The whole feed ran past its limit |
| `Clock` | The feeder's clock jumped |
| `Power` | The pack fell below its floor ([power](power.md)) |
| `Interrupted` | A reset cut the feed short; always `Uncertain` ([feeds](device-api.md#feeds)) |

`Status.feed_fault` names the last feed this boot that ended `Failed` or `Uncertain` (number, outcome, reason) until a later feed completes.

**The hopper.** `hopper` is what the food beam read during the feed: `Low` once it read no food, `Present` if it read only food, `Unread` if the feed ended before the beam settled or a reset interrupted it.

- A feed runs whatever the beam reads, as stock's does, so a misreading beam never skips a meal: a low reading is recorded, not refused.
- The beam is filtered as stock filters its `surplusGrain`: food shows at once, and low only after a second of readings without. Stock counts 20 readings. Its poll rate isn't recovered, so the second assumes the nominal 50 ms its outlet filter implies.
- The beam is also read in a short [window at rest](optical-inputs.md#reading-the-food-beam-at-rest) on the adapter. `Status.food` is the latest reading of either, `Unread` before the first of a boot.
- A `Food { level }` event records each change once no feed is in flight: at most one a feed or a window. A boot's first reading records only if a feed took it, so a restart adds nothing to the history.

## Samples

A subscriber that asked for samples (`Samples::On`) gets every kind, each on its own cadence.

| Metric | Cadence | Holds |
| --- | --- | --- |
| `Power`, `Supply` | Every supply sample, 5 s | The adapter and the pack; the supply as judged |
| `Rssi` | 30 s | The Wi-Fi signal |
| `Heap { free, min }` | 30 s | Free bytes now and the fewest since boot |
| `Loop { passes, max_us, p99_us }` | 30 s | The loop's passes since the last sample, timed on the 32 kHz system timer around the owner's step and the pass. The percentile comes from buckets an eighth of an octave wide |
| `Gateway { round_trip_ms }` | 30 s | The latest answered gateway probe since the last sample, `None` if none |
| `Wifi { channel, ap_changes }` | 30 s | The channel, and how often a join landed on another access point |

**Supply.** `Metric::Supply { adapter, battery, below_floor }` names an unread, stale or implausible reading rather than passing it off as a value. A plausible reading is `Battery::Reading { mv, percent }`, its charge on stock's curve. On its batteries the feeder turns Wi-Fi off 5 s after the adapter goes, so sessions end until it returns, and it refuses an update offer `Update(Battery)` ([battery mode](power.md#battery-mode)). `EventBody::Supply` records the adapter's changes and the pack crossing its floor, at most one a minute.

## Settings, Wi-Fi and the lock

**Device settings** are lights, sound, volume (0–100) and a window each for lights and sound, with a record generation.

- **Windows.** A window is `Always` or `Between { start, end }`: minutes of the local day, both included, wrapping past midnight when the end is before the start, as stock's "aging type" windows compare them. Lights show while `lights` is on and inside the lights window. A sound plays while `sound` is on, the volume is above 0 and the time is inside the sound window. While the clock is invalid or has no zone, every window is open, so a lost clock never hides a fault light. `wellfed settings` takes them as `--lights-window` and `--sound-window` (`always` or `HH:MM-HH:MM`).
- **Desiccant reminder.** The period (`desiccant_days`, 1–90, `--desiccant-days`) and the local day of the last replacement (`desiccant_reset`), which only `ResetDesiccant` sets. The status reports the count (`Status::desiccant`: `Unset`, `Unknown` without a clock, or `Remaining { days }`, due at 0 or below) ([desiccant reminder](desiccant.md)).
- **Meal reminder.** `Off` or `Before { minutes }` with 1–60 minutes (`--meal-reminder off|N`), which sends a `MealSoon` event ahead of each scheduled meal ([schedules](schedules.md#meal-reminder)).
- **The records.** The settings are two records, saved from one request: the device's (`PLFSET01` major 5, storage sectors 2 and 3: lights, sound, volume and the windows; `src/product/wellfed-device/src/store/settings.rs`) and the feeder's (`PLFFDS01` major 1, sectors 48 and 49: bowl mode, meal reminder and desiccant reminder; `src/product/wellfed-feeder/src/store/settings.rs`). A newer minor of either reads, its unknown bytes kept through this build's saves; a newer major or an unreadable record opens read-only. A record of an older major reads as erased, so the store opens writable on defaults. Either record read-only makes the whole view read-only, even for a change to the other's fields: a record opens read-only only when it is a newer major or corrupt, and no client offers a feeder an older build.
- **The writer.** The settings writer (`src/product/wellfed-device-runtime/src/writer.rs`), which the runtime's service task steps, owns both records. It starts after the boot is confirmed, and arms the flash-lock hooks if the confirmation didn't. A change saves both records, whichever fields it touches, under one write turn and attempt marker: the device's and then the feeder's, both at the same new generation, one past the view's. If the device's save fails, the feeder's is not tried. If either fails, the writer reads both records again from flash (a write can land and still report failure) and publishes what they hold, records no event, and refuses the client's write `NotWritable`, never `Done`: the writer leaves each change's outcome where the client's waiting write reads it (`wellfed_core::settings::Saved`).
- **The generation.** A client's write names the generation it read, so a generation must name one state of the settings. The view reports one only when both records read their newest slot cleanly at the same generation. Otherwise (a save stopped between them by a power cut or a failure, or a newest slot lost) it reports `4294967295` (`u32::MAX`, which no record reaches) and is read-only, and the lights and speaker go by what the records hold. Once per boot, before it takes a change, the writer repairs records it finds apart: it saves both again with what each holds, two past the larger generation, the record that read cleanly first, and logs `repaired generation=…`. A save that fails and leaves the records apart gets one more repair 10 s later (`REPAIR_RETRY_MS`), so a transient failure does not wait for a restart; if that repair fails too, or a boot's does, the view stays read-only until a later boot's repair lands. Two past, and the clean record first, keep a generation from returning with other settings when newest slots are lost and repairs fail; a brute-force model of the stores checks every sequence of saves, repairs, partial writes, failed reads, power cuts and lost slots through 10 events (`wellfed_core::settings` gives the argument and its limit). Each repair spends two generations of each record's wear limit: a record whose saves keep failing makes every boot rewrite the other at two past, 2 of its 100,000 generations each time, until it reads `Worn` and refuses saves.
- **Records:** `PLCFG1 start`, `open status=… generation=… lights=… sound=… volume=… lights_window=… sound_window=… feeder_status=… feeder_generation=… meal_reminder=… desiccant_days=… desiccant_reset=…` (`off` or the minutes; a day number, or `none`), the arming records, `saved …`, `repaired generation=…` and `error stage=unarmed|save|repair|apart store=device|kind status=…` (the store whose save or repair failed, with its status before the records were read again; `apart` a change taken while the records share no generation).

**Wi-Fi credentials** are stored for the next join: the next boot, or the next attempt while the feeder is offline. A clear returns to the build's credentials.

- **Store:** `wellfed-device/src/store/net.rs`, two 128-byte records in storage sectors 5 and 6, written by the settings writer inside a marked attempt. An SSID is 1–32 bytes, a passphrase 8–63 printable ASCII.
- **Join:** before each attempt the C shim takes the build's credentials, and Rust substitutes the stored ones. After 5 failed attempts in a row with no success, the build's credentials take over until the next boot or the next save.
- **Saving while offline:** the save is read at the next attempt, with a fresh count of 5. BLE setup relies on this. While the feeder is online, nothing rejoins.
- The connect event names the source: `build`, `stored` or `fallback`. Nothing ever reports the SSID or passphrase.
- **Without build Wi-Fi** (a consumer image, [firmware builds](../development/firmware-builds.md#bench-and-consumer-images)), stored credentials never fall back. With none stored the feeder joins nothing and retries every 30 s without scanning. Its status still reads `build`, since no source value stands for "nothing to join".

**The child lock.** The device API reaches the lock only through a one-slot box the loop's panel owner reads (`wellfed_feeder::feeder::lock`). An unlock from the network relocks after the same 30 s idle time as a button unlock.

**The speaker.** `Play` queues one built-in tune (`chime`, `meal`, `coin`, `alert`, `test` or `twinkle`) through `audio::request`, which reads the settings and fills a one-slot mailbox ([audio](audio.md)).

- It is refused `Busy` while a feed is queued or running, the outputs are claimed or a plan's meal is being claimed; during BLE setup mode, an install or a restart; and when the tune would still sound as a meal call's lead begins.
- Those checks come first, so `sound` off can still answer `Busy`. After them it is refused `Muted` when `sound` is off, the volume is 0 or the time is outside the sound window, and `Busy` while a sound plays or the settings aren't loaded yet.
- Neither refusal changes anything, and a store write in progress holds nothing back.
- `Done` means queued, not heard: a codec that won't open logs `PLAUD1 error stage=open` after the client has its answer.

## Time and calibration

**Time** ([clock](clock.md#ntp)). `GetTime` answers the time view: the zone's rule, the NTP server, the clock with its error bound (or `None` while the feeder needs the time), the learned rate, the last source and the last NTP result. `PutTime` queues the zone's rule and the NTP server for the settings writer (storage sectors 27 and 28). It is answered `Done` once the store holds them (or already did), and refused `NotWritable` when the writer couldn't write them, with no event. The loop applies a new generation on its next pass, and runs the NTP exchange on one more UDP socket.

**Plans** ([schedules](schedules.md)). `PutPlans` is checked against the plans' generation. Before anything is queued, a plan asking for other than 1 to 48 portions is refused `Portions { asked }`, and one with another field outside its limits (a call of more than 5 plays among them) `Invalid`. It is refused `Busy` while the outputs are claimed or the schedule is taking a meal, since the meal's claim, feed and release act on the slot they took. It is answered `Done { generation }` once the store holds the table, and records a `Plans` event with its client. The writer saves the table in storage sectors 11 and 12. The schedule's own events are `MealSoon`, `Missed`, and a plan's `Fed` with its `Schedule` source.

**Skipping a meal** ([skipping a meal](schedules.md#skipping-a-meal)). Owners, members and the server may send these, as they may change the plans. Neither takes a base: each names its meal, and the store's own checks apply when the writer takes it.

- `SkipMeal { slot, due_s }` names the plan's slot and the Unix second its meal is due. That must be the plan's next occurrence, still to come, on today's date by the feeder's clock and zone. It moves the plan's mark up to the meal and stores the slot's skip, so the meal never feeds, and disables a plan that runs once.
- `UnskipMeal` with the same two moves the mark back to the second before the meal and clears the skip, enabling a run-once plan again, while the meal is still to come and the plan's next.
- Each is answered `Done` once the store holds it, and records `Skipped { plan, due, by }` or `Unskipped { plan, due, by }`. The answer is decided by the stored skip rather than the mark, which the schedule's claim moves as soon as an undone meal is due.
- `GetPlans` names a skip until the plan's next meal is taken or passed, so after the meal's time a client compares it with the clock.

| Refusal | `SkipMeal` | `UnskipMeal` |
| --- | --- | --- |
| `NoClock` | No valid clock and zone offset | The same |
| `NotFound` | An empty or disabled slot | An empty slot only, since a run-once plan is disabled by its skip |
| `Invalid` | The meal isn't the plan's next today, is already due or is already skipped | The store holds no skip of that meal, or the meal is already due or no longer the plan's next. A meal that ran or was missed leaves the same mark and is never undone, even after the clock moves back before it |
| `Busy` | Another plans change is queued, the outputs are claimed, the schedule is taking a meal, or the plan's previous meal hasn't yet run or been passed | Another plans change is queued, the outputs are claimed or the schedule is taking a meal |
| `NotWritable` | The store can't be written, or the writer couldn't save it | The same |

**Meal call** ([meal call](schedules.md#meal-call)). One clip, which a client converts to 8 kHz 4-bit IMA ADPCM and uploads: at most 32,256 bytes, about 8 s. The writes are owners only, since the clip is the owner's voice.

- `StartCall { bytes, crc }` erases the stored clip and is answered `Done` once the store waits for the first chunk. It is refused `Invalid` for 0 bytes or more than the store holds.
- `CallChunk { offset, data }` sends the stream from where the upload stands, 512 bytes a chunk (the last may be shorter). It is answered `Call(CallView)` once queued, `Busy` while the chunk before is still being written, and `Invalid` for any other offset or length. A chunk already received is answered without a write, so a retry is safe.
- After the last chunk the writer reads the stream back against the CRC and stores the clip (`Stored`), or keeps none (`None`). `GetCall` shows which. A restart forgets an upload in progress.
- `DeleteCall` is answered `Done` once no clip is stored.
- Each write is refused `Busy` while the outputs are claimed, and waits on the writer while a call plays, so the speaker never reads a clip being rewritten.
- `PlayCall` plays the stored clip once so its owner can hear it, answered `Done` once queued. It is refused `NotFound` with no clip, `Muted` as a test tune is, and `Busy` under a test tune's rules, including when the clip would still sound as a meal's call begins.

**Desiccant** ([desiccant reminder](desiccant.md)). `ResetDesiccant` takes no base: a replacement is a fact about today, whatever the settings hold. Owners, members and the server may send it. It is refused `NoClock` while the clock is invalid, `Busy` while a settings change is queued or the outputs are claimed, and `NotWritable` for a store that can't be written. Otherwise it queues today's local day for the settings writer, is answered `Done { generation }` once stored, and records a `Settings` event with its client.

**Calibration** ([calibration](calibration.md)). `GetCalibration` answers the stored weight of one portion, the bowl mode the build runs and the rotor's phase. `PutCalibration` (owners and members, not the server's key) queues a new weight for the settings writer (storage sectors 34 and 35). It is answered `Done` once stored, refused `Invalid` outside 1–50 g, and `Unverified` for a bowl mode the build wasn't verified for. The statistics' `Portions` section reports what was delivered. There is no event, since the calibration has its own generation.

## Updates, invites and recovery

**Updates** (owner sessions only) are `Op::Update`'s `Offer`, `Chunk`, `State`, `Cancel`, `EnrollKey` and `KeyStatement`, answered `Outcome::Update`, `Outcome::Done` or `Refusal::Update`. They are frozen ([evolution](device-api.md#evolution)).

- `UpdateView.bank0` is bank 0 as bank 1 last checked it: `Unchecked`, `Match`, or `Blocked { reason }`. `UpdateRefusal::Fallback { reason }` names the check an offer failed: `Marker`, `Unrecorded`, `Key`, `Extent`, `Changed` or `Read` ([OTA design](ota-design.md#bootstrap)).
- The loop never verifies a signature. It copies the request into the update mailbox and answers from what the service task's update stage publishes ([OTA design](ota-design.md#updates-over-the-device-api)).
- An offer is refused `Busy` while a feed is queued or driven, and a feed while an install holds the flash.
- A chunk is idempotent by its offset, so it isn't a remembered write, and chunks aren't logged one by one.
- An update key is enrolled only once someone at the feeder presses Dispense within 60 s. The session's answer waits for the press (`wellfed-client` waits 75 s for this op, not its usual 10 s), and a session that leaves cancels the offer. A locked panel still takes this press, and the history records the enrollment. `wellfed update-key enroll` sends it ([OTA design](ota-design.md#keys-and-anti-rollback)).

**Invites and recovery** follow [pairing](pairing.md#pairing), and are frozen with the update path ([evolution](device-api.md#the-frozen-part)). `CreateInvite` mints a single-use secret, held in RAM for 10 minutes, which is the PSK of the new client's XXpsk3 handshake. The third handshake message carries the label, and the key enrolls through the keys mailbox. The sealed answer, `Joined::Enrolled { role, model }`, names the model as a welcome does, so the client knows what it joined without opening a session. `CreateBoundInvite { role, key }` mints the same invite for one static key: whoever asks names the key the new client will join with, the redemption checks it once the third message reveals the joiner's key, and a joiner with another key is refused with no answer, the invite kept for the right one. So the key enrolled is the one the asker recorded. It is answered `Invite` like the frozen op and redeemed through the same frozen handshake, but it isn't frozen itself: a feeder before revision 2 refuses it `Unavailable`. The recovery window runs BLE's setup exchange, code proof included, over the stream, closes at the first offer, and takes no Wi-Fi.

## Evidence

- **Observed** (October 22, 2026), settings: a change from generation 2978 to 2979 (lights on, volume 40) was saved and read back after a cold boot.
- **Observed** (October 22), Wi-Fi credentials: stored credentials joined after a cold boot with `source: stored`. With a wrong passphrase, stored attempts 1 to 5 failed, the build's credentials took over from attempt 6, and the feeder came online on attempt 7 (`source: fallback`). A clear returned to the build's credentials. Neither passphrase appeared in any capture.
- **Observed** (October 24), a live change to a different network: with a second hotspot on the bench host, a stored change took effect at the next join after the lab hotspot went away (`source: stored`, 15 s). A clear, then the lab hotspot's return, rejoined with the build's credentials (`source: build`, 20 s), all without a reboot.
- **Observed** (October 31), an update key's enrollment: one real Dispense press on a locked panel confirmed it, and the history recorded the enrollment.
- **Modeled:** the Wi-Fi store under a power cut at every mutation, reopening to the old or the new value and staying writable. Invites, recovery and the update ops are in the [device API's evidence](device-api.md#evidence).
