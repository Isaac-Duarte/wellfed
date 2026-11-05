# Power

The feeder runs from its adapter, with three D cells as a backup. This page explains how the firmware reads the supply, what it does on batteries, the floor below which it won't feed, brownouts, and sleep. It is for firmware developers; the physical checks are in [power bench checks](power-bench-checks.md).

Inputs and battery mode are observed on our unit. The charge, the floor and brownout handling are simulation only: the one sag measured on the pack is a single feed on fresh cells, and no tired pack has fed ([Evidence](#evidence)). Status is on the [roadmap](../roadmap.md).

## In short

The feeder never passes off an unknown reading as a value. Battery mode and the brownout hold wait for fresh readings; the floor treats a failed conversion as low.

| Situation | What the feeder does |
| --- | --- |
| Adapter in | Feeds normally. The floor never refuses or stops a feed. |
| On the pack | Feeds from its running clock. Wi-Fi off after 5 s, so nothing reaches it remotely. |
| On the pack, below the floor | Refuses and stops feeds. Buttons, lock and clock keep working. |
| Adapter plugged or pulled mid-feed | Fails that feed (`Failed`, `Power`). The next runs on whatever supply is there. |
| Boot after a brownout | Holds feeding until the adapter returns or the pack reads 50 % for 3 minutes. |

## Inputs

| Input | Where | Notes |
| --- | --- | --- |
| Pack | ADC 5, divider ≈2.063 (`power::DIVIDER_PER_MILLE`) | Fresh cells: 2357 mV at the pin on the adapter (4.85 V metered), 2306 mV without (4.77 V). No pack: ≈16 mV. |
| Adapter | PA19, HIGH when in | Both transitions ride through without a reset. |
| Motor current | ADC 4 | [Jams and sensor faults](jams-and-sensor-faults.md). |
| Chip supply | Internal ADC 10 | mV with stock's scaling ([bring-up probes](bring-up-probes.md#power-input-probe)). |

The runtime's power monitor samples every 5 s. In the `feeder` firmware the feeding owner holds the ADC instead and samples every 20 ms poll in a feed, every 100 ms at rest. Both publish the same sample (`wellfed_hal::power`).

**Charge** follows stock's curve in pin millivolts (`power::PACK.curve`, `wellfed_core::supply::Curve`): 0 % at 1650 mV (the floor, about 3.40 V pack), then 10 % per 60 mV up to 100 % at 2250 mV (about 4.64 V). A reading reports the first step at or above it, so 1651–1710 mV is 10 % and 2191 mV up is 100 %. Fresh cells (about 2306 mV) read 100 %.

**`PLBAT1` records** come from every firmware that doesn't claim the ADC:

| Record | When |
| --- | --- |
| `start created=<0\|1>` | The monitor starts |
| `open ok=<0\|1>` | The startup probe frees the ADC |
| `sample pin_mv=<n> adapter=<0\|1>` | The first sample, and each adapter change |
| `error adc` | A conversion fails; the monitor stops |
| `claimed` | The firmware keeps the ADC |

## The supply on the wire

`Supply { adapter, battery, below_floor }`, as defined in `wellfed-proto/src/api.rs`:

- `adapter`: `None` without a fresh sample.
- `battery`, one of:
  - `Absent`: no pack.
  - `Reading { mv, percent }`: millivolts and charge.
  - `Implausible { mv }`: above 5.5 V, or below 2.5 V on the pack alone, where the board couldn't run.
  - `Unread`: the monitor hasn't run, its ADC failed, or the firmware doesn't sample.
  - `Stale { age_ms }`: older than three monitor periods.
- `below_floor`: the owner's verdict (`wellfed_core::supply::classify` against `power::PACK`). `None` before it judges, without an owner, and with no pack.

It appears in:

- **Status** (`FeederStatus { supply }`, printed by `wellfed status`), always judged, saying so when unread or stale.
- **`Metric::Supply`**, every 5 s to sample subscribers, beside `Power`.
- **`EventBody::Supply`**, for adapter changes and floor crossings, at most once a minute (`feeder::supply::EVENT_MS`). Changes within the minute are recorded at its end as the supply then stands, so chatter costs one event a minute. A boot's first reading is a baseline; a stale sample changes nothing. Events use the existing 16-byte ring records.
- **Readiness**: `Blocked::Battery`, refused `Busy`. It isn't lasting, so it raises no `FeedingBlocked` event; the `Supply` event covers it.
- **Statistics**: the `Power` section's history, least and most take `Reading` values only.

## Battery mode

On batteries the feeder behaves as stock does ([stock firmware](../devices/plaf103/stock/README.md#power-and-battery)): Wi-Fi off, meals from the running clock, buttons, lock and lights as usual. An update offer is refused `Update(Battery)` as soon as the adapter reads absent.

**Entry.** Every fresh sample for 5 s (`feeder::supply::RADIO_OFF_MS`) must read the adapter absent with a plausible pack. A shorter loss changes nothing. An unknown adapter, or a pack reading absent or (on the pack alone) implausible, neither starts nor ends the wait.

**No cells versus a flat pack.** An empty holder reads about 16 mV at the pin. Anything from 500 mV at the pin (`NO_PACK_PIN_MV`, about 1 V pack, about 0.34 V a cell) is a pack, however flat; below that reads as no batteries, `below_floor` is `None`, and `wellfed status` says `adapter; no batteries`. No flat pack has been measured yet ([check 2](power-bench-checks.md#2-the-floor-without-motion)).

**The phantom at a cut.** With no cells, a power cut reads the dying 5 V rail as a plausible pack (3.86 V seen). So after the adapter's samples read no pack, the loop keeps reporting none until the channel reads otherwise for 5 s (`feeder::supply::CELLS_MS`), whatever PA19 says. The cut's `Supply` event reads `Absent`, and status and metrics, version 1 `Power` included, show no millivolts. The phantom is too brief to start battery mode or lift a [brownout hold](#brownout). It also delays entry: from a no-pack reading, the radio goes off about 10 s after the pull. Cells fitted on the adapter show after 5 s.

**Radio off.** The loop asks for it every pass (`Platform::radio`). The C Wi-Fi task (`example_entry.c`) then:

1. waits 2 s if no association is held, as after a failed or rejected join, so the driver's key-table teardown finishes first (powering the MAC down under it can stall the bus into a watchdog reset). If the adapter returns meanwhile, the radio stays on;
2. takes the join's turn, which waits out a moving feed;
3. if joined, leaves as before a restart (DHCP stopped, address released, disconnect, up to 1 s for the association to drop so the access point doesn't hold it against the next join), then waits 500 ms for the disconnect's teardown;
4. calls `wifi_off`, then gives the turn back;
5. polls every 50 ms until asked on.

The 2 s and 500 ms are provisional.

**Radio on.** The moment the adapter returns, the task takes the turn, calls `wifi_on` in station mode with the boot's power-save settings, gives the turn back, and joins with the backoff reset. A failed `wifi_on` retries every 5 s; after 6 in a row (about 30 s) the feeder restarts, still holding the turn so no feed starts into the reset. A restart asked meanwhile parks the task with the radio off. A join cut short doesn't count toward the credentials' fallback. Once on, the radio stays on at least 60 s (`RADIO_MIN_ON_MS`), so a chattering adapter cycles the WLAN driver, its heap and its firmware load at most once a minute; the boot's first loss isn't held back.

**Turns.** `wifi_off` and `wifi_on` stall every task in the vendor driver, like an association ([scheduling gaps](resource-budget.md#scheduling-gaps-across-wi-fi-rejoins)), which would break a feed's 1 s beat. So they share the join's turn ([the interlock](resource-budget.md#the-interlock)): a moving feed defers the change, and a feed asked during one is refused (`DispenseRefused` for a press; a meal retries within its grace).

**Installs.** Wi-Fi going off sends a receiving install a cancel at the update stage; it would otherwise hold the write turn about 65 s until it timed out. Bank 1 stays invalid, as at any stop before activation, the server sees `Cancelled`, and the next offer installs.

**Setup mode** keeps Wi-Fi on, since the vendor's BLE stack needs the WLAN driver. It already stops advertising without the adapter ([Bluetooth](bluetooth.md)); after it ends (with a restart), battery mode turns Wi-Fi off.

**The clock** isn't synced on batteries. It stays valid until its error bound runs out, about 6.6 hours after the last sync with an unlearned rate, about 7 days with a learned one ([clock](clock.md)). After that, **meals hold** until the adapter returns and a time source syncs it. Dispense still feeds.

**Records:** `PLBAT2 radio off|on` and `PLBAT2 install stop queued=1` from the loop; `PLWIF3 … kind=off`, then `kind=on`, from the task, with a failed `kind=on` carrying its failures in a row.

## The floor

`wellfed_hal::power::FLOOR`, in pin millivolts.

> [!NOTE]
> The floor is **provisional**. The one sag measured is a single feed on fresh cells, far above it. The regulator's dropout and the chip's brownout level are unmeasured, and the SDK leaves the brownout thresholds at reset values documented only as codes.

| Limit | Pin mV | Pack | Why |
| --- | ---: | ---: | --- |
| `floor` | 1650 | 3.40 V (1.13 V a cell) | Stock's 0 %, below which stock stops its outputs on battery ([stock firmware](../devices/plaf103/stock/README.md)) |
| `clear` | 1710 | 3.53 V | Stock's 10 % step: a pack must recover a step before feeding again |
| `cutoff` | 1550 | 3.20 V | Little 3.3 V regulator headroom left; stop at once rather than brown out mid-drive |
| `hold_ms` | 300 | | From the first reading under `floor`: 15 polls in a feed, 3 conversions at rest. On the adapter the start-up surge lasted one poll; a longer sag is the pack failing under load |

`wellfed_core::supply::Floor` judges every battery conversion the owner takes, on the adapter too, so the verdict is current when the adapter goes:

- At or above `clear`, the pack is above the floor.
- Between `floor` and `clear`, it keeps its last verdict. A first reading there counts as above.
- Under `floor` for `hold_ms` in a row, or under `cutoff` even once, it is below. A failed conversion counts as a reading under `floor`.

On the pack alone (PA19 LOW):

- **A running feed** stops when the pack is no longer judged above the floor, or its conversion fails. It ends `Failed`, `Power` with STOP applied, written like any failure, and is never restarted.
- **A new feed** is refused before reaching the owner: `Feeding::blocked` answers `Battery` while `OwnerStatus::supply_low` holds. The controller's own start check backs it up; a feed that lost the race ends `Failed`, `Power` without driving when its prepare window runs out.
- **A due meal** retries within its 15-minute grace, then is recorded `Missed { reason: Blocked(Battery) }` ([schedules](schedules.md)), never fed late.
- **The warning light** blinks.

A source change mid-feed fails the feed: it unsettles power for 100 ms (stock's settle). A pack that sags under load but recovers past `clear` at rest feeds again and may fail again. Each attempt ends with a result and none is retried, so the cost is failed feeds, never motion without a result.

Stock instead suspends below 0 %, stopping its motor, emitters and lights and polling for mains every second.

## Brownout

A brownout is a reset from the supply dropping too low. Three rules stop it from feeding twice, looping, or rolling back a good build.

**No replay.** Journal recovery at the next boot resolves the interrupted feed as `Uncertain` under its own id. It never resumes.

**A hold before the next feed.** A brownout boot holds feeding as `Blocked::Battery` (`feeder::supply::Watch`, logged `PLBAT2 hold brownout`). Otherwise the rested pack's rebound would start a feed that sags it into another brownout, each loop leaving a feed uncertain. The hold ends (`PLBAT2 hold released by=adapter|pack`) when the adapter is seen, or when every fresh reading for 3 minutes (`RELEASE_MS`) is at least 50 % (`RELEASE_PERCENT`): above 1890 mV at the pin, about 3.9 V pack. Anything else restarts the 3 minutes. Fresh cells read 100 %, and cells fitted after a power cycle start a power-on boot, which never holds.

> [!WARNING]
> The release is **provisional**: it assumes a browned-out pack rebounds well under 50 % at rest, unmeasured on this board ([checks 2 to 4](power-bench-checks.md)). If it rebounds higher for 3 minutes, the next feed may brown out again: at worst one uncertain feed per brownout, from a due meal or a press, never a retry. Each brownout restarts the hold.

**Not a reset storm.** Brownouts keep the backup registers, so KM0 would otherwise count them as unrequested resets. A pack browning out at idle on Wi-Fi transmit peaks could reach the storm limit (five in ten minutes) and make a confirmed bank 1 give way to older bank 0 for a supply fault. So KM0 reads `BOOT_Reason` before its trial step and, on a brownout boot, takes back the previous boot's pending count (`trial::decide_after`, `PLTRL1 brownout=1 reg=…`). A brownout neither counts nor ends a burst. Trials still count brownout boots, so none is shortened or skipped ([OTA design](ota-design.md)).

**The cause** is `BOOT_Reason` bit 18, from the ROM's flags. Statistics report `ResetCause::Brownout`; the `Boot` event carries the raw flags. The bit may be stale: the loader that copies the flags returns early when none are set, so after a real brownout, a later flagless reset can read as one. Two guards:

- No two boots in a row are forgiven (`trial::FORGIVEN`, bit 31 of backup register 4, leaving its identity tag 7 bits). A fault still gives way within 10 early resets and halts within 16, however the bit reads.
- A stale bit can hold feeds like a real brownout. The hold still ends when the adapter is seen, or after 3 minutes on a charged pack.

The floor makes a motor-driven brownout unlikely, but can't prevent an idle one on a pack below the regulator's dropout.

## Sleep

The chip doesn't sleep (recovered):

- **Wakelocks.** Both cores hold `PMU_OS` from `main` (`firstboot_km0.c`, `firstboot_km4.c`), and the owner takes its own PMU wakelock. KM4's tickless idle only stops the tick while every task sleeps; with the wakelock held it never enters a low-power mode.
- **KM0's DLPS path** reaches flash users (the flash lock and its clock changes) outside the flash authority, so sleep entry could touch flash during a KM4 lease. They must go through the authority, or a KM0 deny provider must refuse them, before KM0 sleeps.
- **Safe sleep needs**, none of it present: motor at STOP and emitters off before entry; the watchdog stopped or fed across it; the 32 kHz clocks the owner and [clock](clock.md) use kept running; and no feed resumed on wake (the journal covers resets, but sleep isn't one).
- **The saving.** Stock drops Wi-Fi and MQTT 5 s after the adapter goes and duty-cycles its beams (recovered); whether it uses the chip's low-power modes isn't recorded. Dropping Wi-Fi is the bigger saving, and [battery mode](#battery-mode) does it without sleep. Beam duty-cycling isn't done yet.

## Open questions

- The floor under load, and the board's brownout level.
- Whether a real brownout sets bit 18 here; the storm rule and the hold rely on it.
- Sleep, and KM0's flash users on the DLPS path.
- How long `wifi_off` and `wifi_on` stall the tasks, and how much current battery mode saves.
- Whether 2 s is enough for a failed join's teardown before `wifi_off`, and 500 ms for a leave's.
- Whether a browned-out pack rebounds under 50 %.
- Stock's warning light at ≤20 % whenever a pack reads, and its beam duty-cycling on batteries; the feeder does neither yet.

## Evidence

- **Battery mode.** Observed, October 28 (unattended apart from the owner's press, three D cells, empty hopper). A relay-1 cut read `on batteries` at once and the radio went off after 5 s; Dispense fed 1 portion; with the adapter back it rejoined in 12 s in the same boot. Over 20 cycles (20 s on cells, a minute apart) each rejoined in 12–13 s in the same boot, heap low water steady at 129,440 B, every leave 0, no status-30 reject or hang.
- **Inputs.** Observed, October 23 (UART, meter, `adc` probe, three D cells, no reboot between states):

  | State | PA19 | Ch 5 mean (raw) | Scaled (mV) | Meter |
  | --- | --- | --- | --- | --- |
  | Adapter, no batteries | 1 | 240 | 16 | — |
  | Adapter and batteries | 1 | 2849 | 2357 | 4.85 V |
  | Batteries only | 0 | 2737 after the unplug, ≈2793 after 20 s | 2256 to ≈2306 | 4.77 V |
  | Adapter back | 1 | 2848 | 2356 | — |

  So channel 5 is the pack, divider about 2.06 (2.058 and 2.068). Channel 4 stayed at 240; no reset or visible change at either transition. Channel 5 is noisier: about 110 raw min-to-max per 16-conversion sample, against 3–4 on channels 4 and 10. Meter readings weren't time-matched to samples.
- **Health.** Observed, October 23: `"adapter":true,"battery_mv":4862` (meter 4.85 V) and `"adapter":false,"battery_mv":4755` (4.77 V). With no pack, `PLBAT1 open ok=1` came only after the startup probe, then `sample pin_mv=15 adapter=1`.
- **A feed on fresh cells.** Observed, November 1 (full hopper, press after a relay-1 cut): 1 of 1, no reset, 10,189 ms drive, peak current 268 and mean 86 on the feeder's scale. Pack 4,839 mV at rest on the adapter, 4,586 mV lowest by the feeder's own samples (no meter, so a dip between samples wouldn't show). The radio went off 35–46 s after the cut (about 45 s after boot), likely the 60 s minimum on time, unconfirmed.
- **Surge.** Observed: one 20 ms poll on the adapter.
- **Power cut mid-drive.** Observed: uncertain at the next boot, never resumed ([motor and feeds](motor-pulse.md#a-power-cut-mid-feed)).
- **Stock's floor and battery behavior.** Recovered.
- **Phantom at a cut.** Observed, October 28, before the fix: a no-cell cut recorded `Supply { adapter: false, battery: Reading 3861 mV }`, the decaying rail. The fix is modeled only.
- **Brownout bit on hardware.** Unknown.
- **Modeled**, October 26 (floor, reporting, brownout) and October 28 (battery mode, charge, hold release, phantom fix), in these tests:
  - `wellfed-core/tests/supply.rs`, `tests/sensor_inputs.rs`: readings, charge steps, hold, cutoff, hysteresis, wrap, surges, failed conversions.
  - `wellfed-hal/tests/power.rs`, `tests/link.rs`; `wellfed-hal-common/tests/trial_brownout.rs`: unread, stale and implausible samples; the radio's leave, restarts, refused and deferred feeds, a radio that won't return; a stuck brownout bit still giving way and halting, no two forgiven in a row, trials counting brownouts.
  - `wellfed-device-runtime/tests/network.rs`: cut-short joins don't count toward the fallback.
  - `wellfed-feeder/tests/supply.rs`: events and their rate, the 5 s wait, the minute back on, boots across the uptime wrap, setup mode, the phantom, the hold and its release.
  - The firmware's `tests/manual_feed.rs`, `wellfed-client/tests/api.rs`, and wire fixtures for every variant: a refused press on a low pack, the battery reason on the wire.
  - Simulation: `wellfed-sim` (sags via `surge_sag_adc`, `drive_sag_adc`, zero by default; scenarios `adapter-loss-mid-feed.json`, `battery-sag.json`), `wellfed-feeder-sim/tests/battery.rs` and `tests/update.rs` (battery mode end to end over TCP, an install cancelled mid-transfer). Its console takes `adapter in|out` and `pack <mV>|out`.
  - The C task's `wifi_off`/`wifi_on` path is static analysis only.
