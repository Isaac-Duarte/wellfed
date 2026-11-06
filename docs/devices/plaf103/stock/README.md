# Stock firmware 1.1.15

What the PLAF103's stock firmware does and how it talks to the cloud: Wellfed's parity target, and the protocol its stock server speaks. Everything here is stock **1.1.15** on our research unit; other models and releases may differ. Most findings are recovered from the firmware image and the vendor app, and each section says which. The [board page](../board.md) separates instruction-level behavior from what hardware confirmed.

## Network protocol

Stock talks plaintext MQTT on TCP 1883 with device-specific credentials.

- **Connect:** the first CONNECT uses legacy MQTT 3.1 (`MQIsdp`). Our server normalizes it for an MQTT 3.1.1 broker and forces a clean session, keeping the credentials, client identifier, keepalive and later packets as they are.
- **Topics:** `dl/PLAF103/<device-id>/<group>/<event>/post` or `/sub`, kept within the configured device namespace.
- **Our stock server** handles startup, attributes, heartbeat, clock, binding and captured plan replies ([stock server](../../../development/stock-server.md)). The Rust protocol tests hold complete synthetic wire examples.

### Where stock connects

Stock's broker is a DNS name. Its settings sector at `0x3e8000` holds the port as a little-endian word at its start and the NUL-terminated host name at `0x0c`. `Set_Dev_Info` writes that record, and stock loads it at every boot (static analysis; the offsets match the feeder's flash reads).

- **Plaintext, no pinning:** no TLS and no certificate or address pinning (static analysis; the stored port and a capture agree).
- **Looked up at each connect,** as the MQTT client starts, through the resolvers its network handed it. Its diagnostic report names two: the network's resolver, then a public one (observed). A resolver that doesn't answer most likely lets stock fall back to the public one, which answers with the vendor's address (inference).
- **So DNS moves it.** When the network's resolver answers the name with our server, a restarted stock connects there with its own credentials. Nothing is written to the feeder, so its plans and settings stay.
- **What rewrites it.** Once connected, a `DEVICE_CONFIG_SYNC` carrying an `mqttAddr` list of host and port rewrites only `0x3e8000`, and `WIFI_CHANGE_SERVICE` only the Wi-Fi settings. Neither erases the plans (static analysis). Our server sends neither.
- **First messages.** A stock set up before sends no `BINDING`. It sends a diagnostic report, `NTP`, `GET_FEEDING_PLAN_EVENT`, a heartbeat, `DEVICE_START_EVENT` and `ATTR_PUSH_EVENT` (observed). A stock fresh from setup sends `BINDING` first (observed, in a capture of the vendor app's setup).

Our broker is authenticated, with no upstream bridge and no kept sessions. The server sends only two commands unasked, `ATTR_GET_SERVICE` and `OTA_UPGRADE` (`protocol::Sent`), on QoS 0 and never retained. `ATTR_GET_SERVICE` gets a fresh UUID. `OTA_UPGRADE` carries the caller's durable request ID, and a reused ID returns the existing record without sending again. A reply must match its request ID, and an uncertain operation is never republished automatically. Stock's limited raw-payload and result caching is no substitute for durable no-replay handling.

## Feeding and device features

Stock's feeding code tracks feedback events, logical direction, single- and double-bowl progress, direction pauses and reverse recovery. Wellfed's controller uses bounded deadlines and fault stops instead, and doesn't copy every stock branch. Physical quantities and the mechanical mapping are uncalibrated.

We also traced stock's optical filtering for the food and outlet sensors, power-dependent emitter duty, current estimates, battery lookup, the four button roles, indicator selection and audio completion. A traced branch establishes no physical polarity or real timing, and no Wellfed support.

## Panel buttons and child lock

Stock's event task runs its button handler once per loop, with a 50-tick delay at 1 kHz. The boundaries below count those visits, so durations are nominal: handler and scheduling time add to them, and none was timed on the feeder.

| Behavior | Stock boundary | Nominal |
| --- | --- | --- |
| Lock (PB7) held toggles the panel lock, once per hold | 100th pressed visit | About 5 s |
| An unlocked panel relocks | 600th idle visit | About 30 s |
| Reset (PB4) held on an unlocked panel starts the factory restore | 100th pressed visit | About 5 s |
| Dispense (PB6) asks for one feed, once per press | The pressed visit | — |

- **Locked:** Dispense, Mic (PB5) and the Reset hold do nothing. Lock still toggles, so you can always unlock.
- **Idle:** activity on Mic, Reset or Dispense restarts the relock count.
- **Indicator:** PA4 is HIGH while locked and LOW while unlocked. With the `lights` setting off, every indicator is LOW.
- **App setting:** the PLAF103 has no `childLock*` fields; the vendor app reports `childLockSwitch` and both lock durations as `null` for it. Its setting is `disableHardwareButton` (configuration `+0x91`), which disables the panel except for a recovery sequence: hold Lock about 3 s, then Reset about 3 s, to start the factory restore.
- **Unknown:** the lock state at boot. Its flag's address falls inside the RAM image's code, so its initial value was not recovered.

### Wellfed's panel

`wellfed-feeder`'s `panel` keeps these rules over the SDK button service, with one 5 s hold threshold, and the `feeder` firmware shows the lock on PA4. Where stock is unknown or we chose differently:

- The panel starts locked at every boot, so an unattended reboot never leaves Dispense live.
- Any gesture, Lock included, restarts the relock count, and a held button keeps it off until release. Stock counts only Mic, Reset and Dispense.
- A `SetLock` request over the [device API](../../../firmware/device-api.md) locks or unlocks; a remote unlock relocks on the same 30 s timer.
- `disableHardwareButton` and its recovery sequence are not implemented. Wellfed's own Reset hold clears clients, Wi-Fi, settings and plans, and keeps the calibration, the feed journal and the flash wear counts ([roadmap](../../../roadmap.md)).

## Bluetooth LE

Stock links the full Realtek BLE stack, with its embedded controller patch and Wi-Fi coexistence, and runs as a GATT peripheral for two jobs:

- **Setup:** the app writes Wi-Fi and MQTT settings and binds the device, then reads the result, over 16-bit service `ffee` with characteristics `ffe1`–`ffe4`.
- **Offline diagnostics:** after about two minutes offline, the feeder advertises and sends a compact report by notification, until it is back online.

Advertising runs only on mains power. Feeding, schedules and settings all go over MQTT, never BLE. Pairing appears only as SDK plumbing; setup seems to rely on plain GATT with encryption in the app. Bonds are stored through the SDK's FTL flash layer, whose sector was not located. Wellfed's design is in [Bluetooth](../../../firmware/bluetooth.md).

## Clock and schedules

Stock keeps up to ten plans in flash ([plan records](#plan-records)) and asks for them with `GET_FEEDING_PLAN_EVENT` when it connects; any reply with a `plans` key replaces the whole table ([how stock receives plans](#how-stock-receives-plans)). Our stock server leaves the plans alone, and sets stock's clock only when no meal can come due. Nonempty plans are unvalidated for this release. Before that change our server answered an installer's config with `plans: []`, so installs run before then may have erased stock's table ([an installer's server](../../../development/stock-server.md#an-installers-server-trust-on-first-use)).

Stock's `NTP` request carries its own clock in `ts` (observed). With its clock held two days behind, stock refused an install with error 1214 (observed once; the clock as the cause is likely, not confirmed statically). Responses use the configured IANA time zone, including DST transitions. Stock's full time zone and timer activation, and how the cloud builds plans, are unresolved. Wellfed's own schedules are in [feeding schedules](../../../firmware/schedules.md).

## Feature limits and defaults

What stock does at each feature's edges, and the defaults it writes when its settings sector is blank or invalid. Evidence: **static** (static analysis), **emulation** (instruction-level emulation of stock code, no hardware) and **observed** (values the vendor app logged for the research unit; one unit, not proven factory defaults). Durations counted in loop visits or timer ticks are nominal.

**Stock has no local control API.** Settings and actions arrive as MQTT attribute and service messages from the cloud (static).

### Schedules

| Fact | Value | Evidence |
| --- | --- | --- |
| Stored plans | At most 10, an 11th dropped; 44 bytes each | emulation |
| App plan limit | The app's copy says 10 schedules per day | static (app) |
| Field validation | None: hour, minute, portions, `audioTimes` and weekday bytes store unchecked (256 wraps to 0, -1 to 255; "99:99" stores as given) | emulation |
| Due check | Only second 0 of the plan's minute, on a listed weekday. A minute that passes while the feeder is off is not caught up | emulation |
| Busy and restart | A plan due during a feed waits and runs afterwards. A waiting plan survives restart and runs late, with no age limit | emulation |
| Time zone | Stored as whole hours (5.5 becomes 5), so half-hour zones shift meals by 30 min | emulation |
| Stored plan layout | [Plan records](#plan-records). Non-repeating plans are deleted after they run | emulation, static |

#### Plan records

Stock keeps its plans in two flash sectors and rereads them at every boot. Wellfed neither reads nor writes them, so a rollback finds them as stock left them.

- **0x7d0000, the count.** One byte. Stock's boot reads an erased byte (0xff) as no plans and doesn't bound the count; its setter never writes more than 10.
- **0x7d1000, the entries.** Count × 44 bytes, entry *i* in plan slot *i*. Bytes past the count are not read.
- **Writes.** Each save erases and writes the count sector, then the entries sector. Nothing makes the pair atomic:
  - A cut after the count's erase leaves an erased count over the old entries. An empty save writes count 0, and a feeder that never saved has both sectors erased, so an erased count over written entries can only be such a cut.
  - A cut between the two writes leaves the new count over the old or erased entries.
  - The same cut can bring back a one-off plan stock had just deleted, with no trace in the sectors.
- **Deletes.** After a one-off plan's feed, stock zeroes its 44 bytes and keeps the count, so an all-zero entry is a deleted plan.

| Offset | Bytes | Field | Meaning |
| --- | --- | --- | --- |
| 0x00 | 4 | `planId` | Little-endian; the cloud's ID |
| 0x04 | 1 | Minute | From `executionTime` characters 3–4, unchecked |
| 0x05 | 1 | Hour | From `executionTime` characters 0–1, unchecked |
| 0x06 | 1 | One-off | Nonzero: runs at its next minute on any day, then is deleted. Set for a plan sent with no repeat days and a `syncTime` |
| 0x07 | 7 | Repeat days | One weekday per byte, 1 Monday to 7 Sunday; 0 is unused, and other values match no day. A plan with no day and no one-off flag never runs |
| 0x0e | 1 | `enableAudio` | Meal call before the feed |
| 0x0f | 1 | `audioTimes` | Plays of the call |
| 0x10 | 1 | `grainNum` | Portions, unchecked. Ten repeat days overflow into it |
| 0x11 | 1 | Run state | 0 idle, 8 due, 10 waiting behind another feed, 11 resumed from waiting |
| 0x12 | 8 | `syncTime` | Unix milliseconds, little-endian, unaligned; read only on arrival |
| 0x1a | 8 | `skipEndTime` | Unix milliseconds; the plan doesn't run before it. Compared unsigned, so a negative value never lapses. Zero when absent |
| 0x22 | 10 | — | Never written: zero |

The hour, minute and weekday are local: stock compares them with its clock shifted by the zone offset the cloud last sent, a signed byte of whole hours. No zone is stored with the plans.

#### How stock receives plans

Stock's reply handler for `GET_FEEDING_PLAN_EVENT` reads a root `plans` array (static analysis of 1.1.15, with one UART capture of a stock install).

- **The table is replaced.** Any reply with a `plans` key erases both sectors and writes the new table from the first entry, ten at most, later ones dropped. So `plans: []` erases the table.
- **No checks.** It checks neither the reply's code nor its message ID, takes the message unasked, and stores values as given ([field validation](#schedules)).
- **No key, no reply.** A reply without the key writes nothing, and stock then publishes a `FEEDING_PLAN_SERVICE` with code 2099. With no reply stock keeps its table: it sends the request once and does not retry.
- **Numbers.** `planId`, `grainNum`, `audioTimes` and each `repeatDay` value are read as integers, and `enableAudio` as an integer or a boolean (true reads as 1). `syncTime` and `skipEndTime` are read as JSON numbers and stored as 64-bit integers, so millisecond times are exact.

| Field | Type | Stored as |
| --- | --- | --- |
| `planId` | Integer | 0x00, the plan's ID |
| `executionTime` | `"HH:MM"`, local | 0x05 hour and 0x04 minute, from the characters as written |
| `grainNum` | Integer | 0x10, portions |
| `repeatDay` | Array of weekdays, 1 Monday to 7 Sunday | 0x07, one byte each. An empty array with a `syncTime` sets the one-off flag (0x06) |
| `enableAudio` | Boolean, or 0 or 1 | 0x0e, the meal call |
| `audioTimes` | Integer | 0x0f, plays of the call |
| `syncTime` | Unix milliseconds | 0x12 |
| `skipEndTime` | Unix milliseconds | 0x1a: the plan doesn't run before it |

Always send `repeatDay`: without it stock records an error, and the one-off flag depends on the plan before it in the list. A plan whose `skipEndTime` lies far in the future is stored but never runs.

### Feeding

| Fact | Value | Evidence |
| --- | --- | --- |
| Remote portions (`grainNum`) | One unchecked byte, 0–255; 256 wraps to 0. A request without `grainNum` gets ack 2000 | emulation |
| Dispense button | One portion per press. A press during a feed starts nothing and is logged (record code 2022, or 2023 when the outlet reads blocked) | static, emulation |
| Feed source in `GRAIN_OUTPUT_EVENT` | 1 schedule, 2 remote, 3 button | static, emulation (remote) |
| App portion cap | A per-device cloud field, 24 when absent. The cloud sent `maxFeedingCup` 48 | static (app); observed |
| Portion size | Unknown: no mass per portion is recorded. Weighing a counted run of portions settles it | — |
| On-device records | Two flash rings (feeds, errors) of 51 slots, 50 usable, 78-byte records. A feed record is deleted once the cloud acks its `GRAIN_END` with code 0: an upload queue, not a history | static |

### Food level and faults

| Fact | Value | Evidence |
| --- | --- | --- |
| `surplusGrain` | Boolean from the food beam: set as soon as the input reads low, cleared after 20 consecutive high readings. Physical polarity not verified | static |
| Empty hopper | No food check in the schedule or button feed paths. The panel shows a food warning LED | static, emulation |
| Motor jam | Motor current over 300 (ADC counts) reverses the motor. After the third reversal the feed ends with record code 2031 and counts as done (no retry) | static |
| Outlet blocked (`grainOutletState` false) | The feed pauses, and resumes if the outlet clears within 600 one-second ticks (about 10 min), else ends with record code 2008 | static |
| Other record codes | 2003 a plan came due while busy; 2022 and 2023 Dispense pressed while busy | static |
| `barnDoorError` | Not a firmware field; the cloud supplies it | static |

### Reminders

| Fact | Value | Evidence |
| --- | --- | --- |
| Desiccant | No firmware support. The cloud keeps the period and the count, and resets it through its own endpoint. `changeDesiccantFrequency` 14, `remainingDesiccantDays` 14 and -52 (keeps counting below zero). App text recommends 14 days | static; observed |
| Desiccant period range | Unknown: the app's cycle picker bounds were not recovered | — |
| Meal reminder | No firmware support. `feedingPlanAdvanceNoticeTime` 5 or null; its unit and allowed values are unknown | static; observed |

### Panel, lights and sound

| Fact | Value | Evidence |
| --- | --- | --- |
| Child lock | 5 s Lock hold toggles, 30 s idle relock ([panel](#panel-buttons-and-child-lock)) | static, emulation |
| Defaults | `volume` 50, `lightSwitch` and `soundSwitch` on, both aging types 1 (always on), windows 00:00–00:00, `disableHardwareButton` off. The app logged the same values | static; observed |
| `volume` | One unchecked byte; 0 silences every sound. App slider 0–100. The mapping to codec gain is unknown | static; static (app) |
| Quiet windows | Aging type 1 is always on, type 2 only inside start–end inclusive (a window may wrap midnight), any other value off. Compared with the device's local clock, though the fields are named `…Utc`; whether the app sends local or UTC times is unknown. `lightingTimes` and `soundTimes` are stored but not read by the window checks | static |
| Lights off | Every indicator LOW | emulation |
| Meal call clip | None until the app sets `enableAudio`: a URL naming the built-in call selects it, and any other URL up to 256 bytes is downloaded (at most 321,024 bytes) | static |
| When it plays | Before a scheduled feed only if the plan enables audio, a clip is set and sound is allowed (switch, window, volume above 0). It repeats `audioTimes` times, then the motor runs. A non-scheduled feed plays it twice under the same conditions. With sound blocked the feed starts at once | static, emulation |

### Power and battery

| Fact | Value | Evidence |
| --- | --- | --- |
| `powerType` | 1 mains with the battery reading 0 % (or absent), 3 mains with battery charge, 2 battery only. The research unit reported 1 with `electricQuantity` 0 | static; observed |
| Battery curve | ADC 1650 → 0 %, then 10 % per 60 counts up to 2250 → 100 %. A reading reports the first step whose threshold is at or above it | static |
| Low battery | Warning LED at 20 % or less whenever a battery reads (on battery or mains); none on mains with no battery reading. Below ADC 1650 on battery, stock stops motors, emitters and LEDs and polls for mains every second, restarting when it returns | emulation; static |
| Battery mode radios | A configured feeder shuts Wi-Fi and MQTT 5 s after switching to battery; an unconfigured one closes its AP and BLE. With no clock sync on battery it gives up joining after 60 s; with sync it does not retry | static, emulation |
| Battery mode schedules | The meal dispatch has no power-type check, so meals run on battery from the running clock. Beam emitters are duty-cycled (60 visits on, 200 off) | static |
| OTA | Refused on battery only (code 1110) and during a feed (1120) | static |
| `enableBatteryPercent`, `autoChangeMode`, `autoThreshold` | The first is cloud-only (`false` observed). The others are firmware settings (defaults 1 and 20) of unknown effect | static; observed |

### Reset and the cloud

| Fact | Value | Evidence |
| --- | --- | --- |
| Reset holds | One: Reset about 5 s on an unlocked panel starts the factory restore. The panel handler has no Wi-Fi-only hold | static, emulation |
| What the restore clears | Everything stock keeps: it erases `0x3e8000`–`0x3ea000` (the broker, Wi-Fi and binding records), `0x7d0000`–`0x7d6000` (the plans and the record rings) and `0x7da000`, then restarts. The replacement starts with no meals, since the installer doesn't read stock's plans ([consumer installation](../../../firmware/consumer-installation.md)) | static |
| Setup mode | Only through the Reset hold. A set-up stock that can't reach its Wi-Fi retries forever and never opens its setup access point on its own | static |
| MQTT `RESTORE` | Erases the same plan sectors, but keeps the Wi-Fi settings | static |
| Home Assistant, history, remote control and the app | None on the device: history and remote control live in the cloud, the vendor app is the client, and there is no Home Assistant | static |

**App and cloud, not firmware:** the desiccant reminder, the meal reminder, the low-battery and other notifications (`enableLowBatteryNotice` and siblings), `enableBatteryPercent`, `barnDoorError`, feed history (the device only queues unsent records), the per-model portion cap and the plan editor's limits. The firmware has no strings or fields for any of these.

## Reliability limits

Restarting only the stock server reproduced repeated startups and a KM4 watchdog reset indication; the stall's trigger is unresolved. WAN isolation, sustained reconnect, cold boot, time recovery, battery behavior and portion weight each need their own physical acceptance.

## Evidence

- **Recovered, October 18, 2026:** the panel handlers, by static analysis and offline emulation. Nothing ran on the feeder.
- **Observed, October 23, 2026:** Wellfed's panel rules, with the owner pressing the buttons; also host-tested. Before the device API, the deleted MQTT transport's signed `lock` request did what `SetLock` does now.
- **Recovered, October 24, 2026:** the feature limits and defaults, from a static pass over the 1.1.15 image and the vendor app, with the earlier Unicorn emulations of the plan, manual-feed, panel, audio and Wi-Fi handlers.
- **Recovered, October 28, 2026:** the plan records, from a static pass over stock's plan boot load, setter, due check and feed-end delete in the 1.1.15 KM4 image. The weekday numbering, stored byte widths and millisecond times come from the October 18 Unicorn emulations of the setter and the calendar.
- **Observed, October 28, 2026:** every full flash read of our feeder (October 18 to 26) holds count 0 and an erased entries sector: what stock's setter leaves for an empty plan list, as our server's `plans: []` replies wrote it then. Stock's boot reads them as no plans. No read held a stored plan, so the entry layout is unchecked against real bytes.
- **Recovered, October 28, 2026 (evening),** from a read-only static pass over the 1.1.15 KM4 image: the factory restore's erase list and restart; setup mode entered only through it; the broker kept as a DNS name in `0x3e8000` on plaintext 1883; `DEVICE_CONFIG_SYNC` and `WIFI_CHANGE_SERVICE` touching only their own settings; and the plan reply handler's fields and their mapping onto the 44-byte entry.
- **Observed, October 23, 2026:** stock's boot through our bench server (`fw40-stock-ota-07-uart-01`, UART): the name looked up through the network's resolver and a public one, and a set-up stock's first messages, with no `BINDING`. The settings record's offsets match every full flash read of our feeder.
- **Recovered:** Bluetooth LE, from static analysis of the 1.1.15 KM4 image and the vendor app.
- **Observed:** the MQTT protocol, from the feeder's traffic to our server. Our stock server's manual feeding was removed on October 24, 2026.
