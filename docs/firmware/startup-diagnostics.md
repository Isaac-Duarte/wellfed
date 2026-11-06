# Startup and diagnostics

Every Wellfed boot puts the feeder in a known safe state before any firmware code runs: motor stopped, infrared emitters off, indicator lights dark. Then it proves it, by reading every output back and printing the result on the UART. This page explains that startup, what each UART record means, and how the firmware watches its own network link and its own hangs. It is for owners who want to know why the feeder is safe at power-on, and for contributors reading a capture. The bring-up probes that once ran at startup, and the policies the audit still checks for them, are in [bring-up probes](bring-up-probes.md).

## What every boot does

1. **KM0 powers the shared pads.** Before it starts KM4, KM0 enables the audio-shared pads, sets the audio LDO's voltage trim and logs `PLPWR1` ([below](#shared-pad-power)). It also starts the WLAN firmware, which runs on KM0.
2. **KM4 stops every output.** The motor goes to STOP and the emitters to OFF, and each pin is read back ([stopped outputs](#stopped-outputs)).
3. **A synthetic diagnostic** exercises the feeding core with no hardware attached and reports pass or fail.
4. **The indicators are checked LOW,** with outputs still stopped.
5. **The runtime starts:** Wi-Fi, the device API and the firmware's tasks. Probes open the sensors and the ADC only in the firmwares that run them.

Startup never opens the feeding owner. That does **not** mean startup has no electrical side effects: it drives pins, and their transients are measured separately ([motor driver](../devices/plaf103/motor-driver.md)).

## The startup report

A build's identity ties a capture to its exact sources: the SDK wrapper hashes canonical JSON of the profile and the root-relative map of source-input hashes. That hash is `startup_identity` in the manifest and `WELLFED_BUILD_ID` in the firmware. It excludes timestamps, and the final OTA hash stays a separate identity for the artifact. Compiling the port for the target requires a lowercase 64-digit ID; host tests use `host`. A boot prints:

```text
PLFW1 start build=<64 lowercase hex digits> bank=1 reset=00000000 mmu=00000003,0c000000,03e7a000
PLFW1 result diagnostic=pass backend=unopened
```

- **`bank`** comes from live RSIP mapping entry 0: virtual `0x0c000000` mapped to `0x08006000` is bank 0, to `0x08186000` bank 1, and anything else unknown.
- **`mmu`** is the raw mapping, and **`reset`** the factory-preserved reset flags. A zero reset word doesn't establish a particular physical reset cause.

`wellfed-audit startup --device plaf103` ([src/tools/wellfed-audit](../../src/tools/wellfed-audit)) checks a capture against its build: the frozen OTA image and manifest, the embedded identity, the finalized receive-only capture hashes, event coverage, the actual bank mapping, ordering and the observation interval. A missing or wrong build, faults, incomplete or malformed records, a repeated startup and each policy's own failures are distinct failing outcomes. It emits fixed fields, never arbitrary device text. With your own complete evidence bundle:

```sh
cargo run --locked -p wellfed-audit -- startup --device plaf103 \
  --capture private/captures/normal-boot \
  --manifest private/verification/review/build-manifest.json \
  --image private/verification/review/image.bin \
  --minimum-seconds 60 \
  --output private/verification/startup-result.json
```

A passing report shows that this build started cleanly once, not long-term uptime, persistent boot counters or the physical reset cause.

## Stopped outputs

The stopped state is the stock-derived STOP and OFF state, set up in an order that never lets the motor driver see a half-driven pair:

1. Enable and check the LP GPIO clock and function bits, leaving unrelated controls alone (`PLCLK1`, before and after).
2. With the interrupt mask held, preload the latches: motor PA13 and PA14 HIGH and HIGH, infrared PA15 and PB3 LOW and LOW.
3. Enable both motor pins as outputs in one direction-register write, so the driver never sees one pin driven while the other floats.
4. Verify the masked values and directions, then restore the interrupt mask.

- PB3 is also `SWD_CLK`. Before configuring it, startup clears `REG_SWD_PMUX_EN` bit 0, as stock does, and stopped readback requires that bit clear ([why](optical-inputs.md#pb3-is-swd_clk)).
- Infrared OFF needs LOW output latches as well as LOW pad inputs: a pad with a dead input path reads LOW whatever is driven, as PA0–PA7 do on our unit.
- `PLFW1 outputs` reports the PA and PB values and directions before the banner. If any check fails, startup logs an error and doesn't start the diagnostic task, so a synthetic pass can never hide an output mismatch. The manifest's policies must match the record formats in the hashed image.
- The CON3 harness carries both motor leads and board power, so unplugging it is not an established way to cut the motor while keeping the board powered.

## Indicators at startup

After the stopped outputs and a diagnostic pass, startup verifies each light LOW, prints `PLLED1 begin low_only` and `PLLED1 done low=all`, and starts the firmware's tasks at once. This is policy `low-only-v1`, with GPIO routing policy `read-only-v3`, whose only stage is `startup`. The lights were proved by an earlier blinking probe, `bounded-probe-v3` ([bring-up probes](bring-up-probes.md#indicator-probe)), which cost every boot about 15 s before Wi-Fi, the device API and the OTA watcher could start, so it no longer runs. What the lights mean is the [indicator service](indicators.md).

The pins, all lit HIGH on our unit: **PA0 Wi-Fi, PA2 low food, PA4 locked, PB22 warning** (only the warning's red color is recorded). PA0, PA2 and PA4 have no usable input readback, so they are output-only; PB22 is read back in full.

## Shared-pad power

The audio-shared pads need power before they work as GPIO. Stock and the vendor SDK set this up, but a minimal KM0 main leaves it out. Our KM0 restores the AMACRO and analog pad enables and calls the SDK's calibrated `SOCPS_AudioLDO(ENABLE)` before it enters the scheduler, without initializing audio playback.

- **Order.** Stock and the SDK power the pads after booting KM4. We do it before `km4_boot_on()`, so the pads are powered before KM4 configures them and `PLPWR1` completes before KM4 logs anything.
- **Trim.** `SOCPS_AudioLDO(ENABLE)` restores a voltage trim (bits 7:1) that only its disable path saves, so a cold start gets zero. Stock KM4 `CODEC_Init` and the SDK's `CODEC_Init` write 0x2A, or 0x68 on a 1.8 V supply, and our KM0 writes that trim after the enable.

With the trim applied the low Port A pads still read LOW. Raw Port A input sampled after each supply step gave the same word every time, with PA0–PA7 reading 0 before the first step, so the supply doesn't explain it: those pads appear to have no usable input path on our unit. PA13, PA14 and PB22 read back correctly.

## What the UART records mean

Every record starts with a fixed prefix. The ones a current firmware prints, with the page that documents each:

| Prefix | Written by | Meaning |
| --- | --- | --- |
| `PLPWR1` | KM0 | Shared-pad supply controls before and after |
| `PLCLK1` | KM4 startup | GPIO clock and function enable, and what was preserved |
| `PLFW1` | KM4 startup | Stopped outputs, build identity, bank and reset, diagnostic result or errors |
| `PLIO1` | KM4 startup | Read-only pad-group, AON and GPIO routing observations |
| `PLDBG1` | Indicator checks | Requested level, latch, input, directions, stopped state, pad control |
| `PLLED1` | KM4 startup | Indicator sequence begin, results, terminal LOW or failure |
| `PLWIF3` | Wi-Fi task | Join, keep, gateway and leave events ([link liveness](#link-liveness-without-internet-dns)) |
| `PLSUP1` | Watchdog feeder | Supervision start, stop, lease faults and hang reports |
| `PLFLT1` | KM4 fault handler | Hard-fault registers |
| `PLPNC1` | KM4 | Panic location |
| `PLLOG1` | The log ring | Lines the ring dropped ([logging](resource-budget.md#logging)) |
| `PLOTA1`, `PLWCH1` | Update stage | Updates over the device API |
| `PLBAT1` | Power monitor | Power samples ([power](power.md#inputs)) |
| `PLBTN1` | Button probe | Raw button changes ([buttons](buttons.md#the-raw-probe)) |
| `PLKEY1` | Button service | Gestures ([buttons](buttons.md#the-service)). Not to be confused with `PLKEY1\0\0`, the 8-byte magic of an update-key statement ([keys](ota-design.md#keys-and-anti-rollback)) |
| `PLTRL1` | KM0 | Trial counts, reset storms, give-way and brownout forgiveness ([reset storms](ota-design.md#reset-storms)) |
| `PLCNF1` | SDK runtime | Boot confirmation ([boot confirmation](ota-design.md#boot-confirmation-and-fallback)) |
| `PLCFG1` | Settings writer | Settings, key and Wi-Fi changes ([device API](device-api.md)) |
| `PLTIM1` | Clock | Syncs and the carry across restarts ([clock](clock.md)) |
| `PLAPI1` | Device API | Sessions and resources ([device API](device-api.md)) |
| `PLOWN1` | Feeding owner | Feeds, states and faults ([motor and feeds](motor-pulse.md)) |
| `PLINJ1` | Fault-injection variants | What a variant injected ([jams and sensor faults](jams-and-sensor-faults.md#fault-injection-variants)) |
| `PLBAT2` | The loop | Battery mode and the brownout hold ([power](power.md#battery-mode)) |
| `PLBLE1`, `PLSET1` | Bluetooth setup mode | The BLE stack and setup mode ([Bluetooth](bluetooth.md)) |
| `PLIND1` | Indicator service | Panel lights ([indicators](indicators.md)) |
| `PLAUD1` | Speaker | Tunes and the meal call ([audio](audio.md)) |

The bring-up probes' records (`PLADC1`, `PLOPT1`, `PLMOT1`, `PLGAP1` and the rest) are in [bring-up probes](bring-up-probes.md) and the guide pages that run them.

- **`PLPWR1` policies.** `before-km4-v1` requires one intact record before any KM4 report; `v2` adds the LDO trim, and `v3` the raw Port A input after each supply step. An image linking the `v3` record format is refused unless its manifest declares `before-km4-v3`.
- **`PLIO1`.** `read-only-v2` adds KM4's view of the shared-pad supply control and the raw Port A input word.
- **`PLDBG1`.** `bounded-probe-v2` adds `highs=N/64`, the extra input samples ([probes](bring-up-probes.md#indicator-probe)).
- **`PLWIF3`.** Policy `wifi-gateway-v1`: the `PLWIF2` join records plus a `gateway` event per probe (result, gateway IPv4, time). A loss carries its cause: `0` association, `2` lease, `3` gateway. Kind 7 `dns` and loss cause `1` belong to the retired DNS check and are never reused ([probes](bring-up-probes.md#wi-fi-dns-check-retired)). An intentional restart first logs `kind=leave` with `wifi_disconnect`'s `result`, `value` 1 if still associated after 1 s, and `ms` ([below](#leaving-before-a-restart)). The normal firmware logs `hold until=startup max_ms=180000` once and keeps the radio off until the firmware reports `startup_done` or 180 s pass; bank 0 waits the same way, since it runs the firmware and its probes.
- **`PLFLT1`.** A builder overlay of the vendor handler sends the queued lines, then prints `cfsr`, `hfsr`, `mmfar`, `bfar`, the exception `lr` and whether the process stack was in use, before the vendor backtrace. CFSR's IMPRECISERR, or a peripheral address in BFAR, marks a bus fault the stacked PC doesn't point at.
- **`PLSUP1`.** `open watchdog=… trial=…` on its first step. `stop …` once when it stops feeding the watchdog (`restart=1` for an intentional restart), and `stop watchdog_fault=…` once when a checkpoint is refused, so a stop that recovered and then lost its checkpoint logs both. At boot, before anything arms: `lease_fault …` ([OTA design](ota-design.md#resets-that-tell-you-why)), `hang …` after a silent watchdog reset ([below](#silent-watchdog-resets)), then after a `masked` or `spin` hang `trail …` with its `trail_mark …` and `trail_step …` lines, or `trail refused=empty|boot` ([the call trail](#the-call-trail)).
- **`PLLOG1`.** `dropped=N`: the N lines a full ring dropped at this point in the stream. Rust records reach the UART a few milliseconds after they are logged, in order and whole, from a low-priority task. A crash sends what is queued first.
- **`PLPNC1`.** `panic file=… line=… col=…`, with the path's last 95 bytes, printed after the outputs are stopped and the queued lines sent, before the halt the watchdog resets. UART only: no backup register is free to carry it across the reset.
- **`PLOTA1` and `PLWCH1`.** Policy `device-api-v1`, from `ota::update` ([OTA design](ota-design.md#bootstrap) has what each means).
  - At the stage's start: `update store=… min_version=… attempt=…`.
  - Bank 0: `bank0 bytes=… recorded=0|1` (`1` when the record changed this boot) or `bank0 recorded=0 error=extent|read|store`; `manifest version=… bytes=…`, `progress sectors=N/TOTAL` every 16 sectors, `activated version=… bytes=… sectors=…`, `error stage=…` (timeout, cancelled, order, image, write, hash, marker, store), `key retired=…` or `added=…`, and `restart watchdog=1`.
  - Bank 1, once per boot after the healthy window and the hooks' arming: `bank0 check=match|unrecorded|marker|extent|changed|read recorded_len=… found_len=… recorded=… found=…` (the hashes' first four bytes; fields a check never reached are left out). `check=match`, from bank 1 booted after a bank-0 write, is what tells the write finished. Then `PLWCH1 offer=1 version=…` and `give_way=1 version=… restart=1`, or `refused give_way reason=…` with the boot check's fields (`key` added), or `error stage=give_way_write`.
  - Either bank: `refused reason=…` for refused requests.

Records from retired probes (`PLTMR1`, `PLWDG1`, `PLFLS1`, `PLPRK1`, `PLRDS1`, `PLWRS1`, `PLJNL1`, `PLSET1 start marker=`, `PLADC1`, `PLWIF1`, `PLWIF2`, `PLMQT1`, `PLOPT1`) are described in [bring-up probes](bring-up-probes.md).

## Link liveness without internet DNS

The feeder judges its link only by what the LAN shows: the association, the DHCP lease and the lease's gateway, so a network with no internet route is fine. Nothing in the firmware looks up a name. lwIP's resolver is still linked, since the vendor `ping` console command reaches `lwip_gethostbyname`, `netconn_gethostbyname` and `dns_gethostbyname_addrtype`, and it takes the DHCP server's address, but no firmware path asks it anything.

The Wi-Fi task samples the link every 500 ms and asks `wellfed_core::liveness::Liveness` what to do: hold, probe now, settled, or leave with a cause. Every successful join starts a fresh one. The task's whole flow is `wellfed_core::wifi_task`, pure and host-tested in `wellfed-core/tests/wifi_task.rs` and `liveness.rs`. `wellfed_hal::link` performs its steps through one C wrapper per vendor call in `example_entry.c`, which creates the task.

| Check | How | Loss |
| --- | --- | --- |
| Association | `wifi_is_connected_to_ap` | Leave at once, cause 0 |
| Lease | `dhcp_supplied_address` and a nonzero address | Leave at once, cause 2 |
| Gateway | One ICMP echo every 5 s | Three unanswered in a row, cause 3 |
| Restart | A restart was requested | Leave, cause 4 ([below](#leaving-before-a-restart)) |

- **Lease.** lwIP renews and rebinds by itself, and gives the address up only when the whole lease ran out unanswered.
- **Gateway.** The echo goes over a raw socket (`LWIP_RAW` is on in the SDK's options) with a 1 s timeout, and the reply must match on source, type, identifier and sequence. Once the gateway has answered this join, three unanswered probes in a row (11–16 s of silence) are a loss. That catches an association that looks up while nothing gets through, which the driver's beacon-loss detection misses. A gateway that never answers this join is taken to refuse echo, since DHCP just worked over the same link, and the link then rests on association and lease alone.
- **Why not ARP.** No router can refuse ARP, but lwIP 2.0.2 here runs without core locking, so a request would have to be posted to the stack's thread, and its table can't tell a fresh reply from a cached entry.
- **Settled.** Once per join, on the gateway's first answer or after three probes without one, the task resets its rejoin backoff. So a network that drops the feeder right after DHCP backs off to 30 s instead of rejoining every second. A scan that finds the network again after one that missed it resets the backoff too: a hotspot or router back from an outage first rejects the driver's cached key (status 53), and the full join that follows should come a second later, not after the wait the outage grew.
- **Live.** `net_status::Status::live` is written only by the watch: settled and, once the gateway has answered, its latest probe answered too. Trial confirmation waits for it. A leave clears it before the loss event is drained.

Records are `kind=gateway` events. `result` is 0 answered, `-1` no gateway in the lease, `-2` socket, `-3` send, `-4` timeout. Only a timeout counts as silence: a probe that couldn't be sent, as when the device API's sessions and mDNS hold every connection, is no observation. `value` is the gateway and `ms` the round trip. A loss is `kind=lost` with `result` 0, 2 or 3, and the device API's `link_losses` counts every one. Its `dns_losses` and the health record's `"dns"` keep their old names and count losses of cause 2 or 3, which is how a silent access point usually shows: the gateway stops answering before the driver gives up on its beacons.

## Leaving before a restart

Every intentional restart leaves the access point first: an activation, a give-way, `request_reset` for a reboot the device API or setup asked for, and a trial variant's hang, each a watchdog reset. Without a leave the access point still holds the old boot's PMF-protected association, so the next boot's WPA3 SAE authentication succeeds but the association is rejected with status 30. After that reject the vendor's `wifi_connect` sometimes never returns, and KM4 stalls into a silent reset 4 s later. A trial boot right after an OTA restart nearly always meets status 30, so this could spend a good build's three trial boots.

- **Order.** The runtime's `restart()` first calls `wellfed_hal::supervision::stopping()`, which sets `wellfed_hal::link`'s restart flag and marks the breadcrumb, then stops the watchdog feeder as before. The reset never waits for the leave: if the disconnect hangs or the task is busy, the reset comes when it always did.
- **Supervision stops leave too, once certain.** A stop `supervise::feed` decides (a trial's 10-minute deadline, a hung task) isn't a latch: feeding resumes if the condition turns true within the watchdog's 1 s progress gap, and a sub-second false must not take the network down. So `supervision::supervised` calls `stopping()` only once the stop has lasted the whole gap and the reset is certain; a refused checkpoint calls it at once. An unconfirmed trial boot then leaves, and its next trial boot doesn't meet status 30.
- **The Wi-Fi task** checks the restart wish every 50 ms of its watch wait and leaves with cause 4 (`Cause::Restart`) whatever the link shows. The task stops DHCP, calls `wifi_disconnect()` from its own context (the driver queues a protected deauthentication), waits up to 1 s for `wifi_is_connected_to_ap` to report the association gone, logs `kind=leave` and parks, joining no more.
- **Inside an attempt** it checks twice more: after the scan (up to 10 s), so a restart asked meanwhile never calls `wifi_connect`, and right after a successful `wifi_connect`, so an association made while the restart was pending is left at once the same way and DHCP never runs.
- **Callers.** Every reboot the device API or setup asks for already waits for its reply to go out (`reboot_at`, `Setup::reboot_due`), and the update stage waits for its answer (`LEAVE_MS`), so disconnecting loses no answer.

The timing is tight but fits:

- **A requested restart** takes at most about 2 s from the request to the end of the wait: a gateway echo in flight (1 s), then the 1 s wait. The watchdog's shortest expiry less a feed period is 2.18 s at the clock's 54 kHz bound (`config::WATCHDOG`), and about 3.75 s at the measured 4.0 s expiry.
- **A supervision stop** is decided on the feeder pass 1 s after it began, about 1.25 s after the last refresh (passes are 250 ms apart). That leaves about 2.75 s at the measured expiry but only about 1.2 s at the 54 kHz bound (2.43 s shortest). A leave takes a 50 ms wake plus the disconnect, up to about 2 s with an echo in flight, so it fits the measured expiry and may not fit the bound, where the disconnect is best effort.

If a restart comes while `wifi_connect` or `LwIP_DHCP` runs, the leave waits for that call to return. If the reset comes first, the association it was making may outlive the boot.

## Silent watchdog resets

A hang that stops the watchdog feeder task itself prints nothing: no `PLSUP1 stop`, just the ROM banner about 4 s later and `reset=00100000`. So the FreeRTOS tick hook leaves a breadcrumb every tick in backup register 2, which survives the reset (`wellfed_hal::supervision::crumb`, whose module docs say why register 2 is free). The builder sets `configUSE_TICK_HOOK`, and the shim's `vApplicationTickHook` calls `wellfed_tick_hook`. The next boot prints the crumb once, before anything else runs:

```text
PLSUP1 hang kind=spin task=wellfed-wif since_ms=3968 suspended=0
```

- **`task`** is the task the last tick interrupted, printed as the name's first three characters in lower case, or the three after the shared `wellfed-` or `wellfed_` prefix: `wellfed-svc` the service task, `wellfed-wdg` the watchdog feeder, `wellfed-loo` the feeder's loop, `wellfed-wif` the Wi-Fi task, `wellfed` the diagnostic task (a name of exactly `wellfed-` prints the same), `tmr` the timer service, `idl` idle, `cmd` and `rtw` the Wi-Fi driver's threads, and `none` no task yet. The builder raises `configMAX_TASK_NAME_LEN` from the vendor's 10 to 16 so the suffix survives.
- **`since_ms`** is how long since the watchdog feeder task last ran, in 128 ms steps; 8064 means that long or longer. The feeder runs every 250 ms.
- **`kind`** is `spin` when `since_ms` is 2048 or more: the tick kept running without the feeder, so a task kept the CPU, and `task` names it (`suspended=1` if the scheduler was suspended). It is `masked` when less: the tick stopped while the feeder still ran, so interrupts were masked or the tick stopped, and `task` names what was running. It is `stopped` when the feeder had stopped feeding on purpose; the line says what ran until the reset.
- **A `spin` naming `idl`** means the CPU was free: the feeder task was blocked, not starved, so look for what it waits on.

Limits:

- **Only after a KM4 watchdog reset** (`BOOT_KM4_WATCHDOG`). Any other boot clears the register silently, and a cold boot finds it zero.
- **Tickless idle.** With `configUSE_TICKLESS_IDLE` 1, idle stops the tick while every task sleeps, and FreeRTOS steps the count afterwards without calling the hook, so `since_ms` can read short on a mostly idle system. A spin keeps the CPU busy, so it never suppresses ticks.
- **Inside a flash lease** no tick runs, because the lease masks interrupts; the lease fault's line reports a hang there.
- **Cost:** a few loads and one backup-register write per 1 ms tick.
- **The crumb word** is a 3-bit tag, the name in 6-bit characters, the two flags, `since` and a 2-bit check, so a stray value is rarely taken for a crumb.

### The call trail

The crumb names the task but not where it stalled. So the KM4 image also keeps a call trail in RAM a watchdog reset leaves alone: the last 64 calls into a few vendor primitives, and the last call of each rarer kind. After a `PLSUP1 hang` of kind `masked` or `spin`, the next boot prints it once:

```text
PLSUP1 trail boot=41 count=184211 n=64 skipped=0 depth=1 last=save_and_cli
PLSUP1 trail_mark mark=connect seq=180002 step=wifi_connect depth=0 isr=0 lr=0x00000000 arg=0x00000001
PLSUP1 trail_mark mark=connect_ret none
PLSUP1 trail_mark mark=indication seq=184001 step=wifi_indication_ret depth=0 isr=0 lr=0x0e05ae01 arg=0x00000001
PLSUP1 trail_mark mark=disconnect seq=183990 step=rtw_os_indicate_disconnect depth=0 isr=0 lr=0x0e038b71 arg=0x10035000
PLSUP1 trail_mark mark=timer seq=184120 step=rtw_del_timer depth=0 isr=0 lr=0x0e05ac91 arg=0x10030510
PLSUP1 trail_step seq=184147 step=save_and_cli depth=1 isr=0 lr=0x0e05b2a7 arg=0x00000000
…
PLSUP1 trail_step seq=184210 step=save_and_cli depth=1 isr=0 lr=0x0e05b2a7 arg=0x00000000
```

| Field | Means |
| --- | --- |
| `boot` | The KM4 boot count the trail started with |
| `count` | Notes the boot made |
| `n` | How many of the last 64 are shown (smaller with fewer than 64 notes) |
| `skipped` | Shown slots that held no whole note, such as one a reset cut |
| `depth` (header) | Wrapped critical sections open at the end |
| `last` | The newest note's step |
| `trail_mark` | The last note of each rarer kind, which the ring has usually lost: the last `wifi_connect` call (`connect`), its return (`connect_ret`), the last `wifi_indication` (`indication`), the last `rtw_os_indicate_disconnect` (`disconnect`) and the last timer call (`timer`), or `none` |
| `trail_step` | The ring, oldest note first |
| `step` | The call. `save_and_cli` is noted once it has masked interrupts, `restore_flags` just before it unmasks, and `rtw_set_timer`, `rtw_del_timer` and `rtw_cancel_timer` before they run. `wifi_indication` and `rtw_os_indicate_disconnect` are noted on entry and with `_ret` on return. `wifi_connect` and `wifi_connect_ret` come from the Wi-Fi task, around its join |
| `lr` | The return address in the calling function, Thumb bit set. A vendor function that tail-calls a wrapped primitive leaves its own caller's address. Look it up in the hung build's `target_img2.axf` (`arm-none-eabi-addr2line -f -e target_img2.axf`, or the sorted `target_img2.map`). The Wi-Fi task's own steps print 0 |
| `arg` | The first argument: the timer, the indication's event or the adapter; 0 for `save_and_cli` and `restore_flags`; the join attempt for `wifi_connect` and the result for `wifi_connect_ret` |
| `depth` (step) | Open wrapped critical sections, this one included (at most 127), counted for the whole CPU, so a section an interrupt opens inside a task's counts too |
| `isr=1` | The note came from an interrupt handler |
| `seq` | The note's number since the trail started. It wraps at 2^32; right after a wrap the ring shows only the notes since |

The driver masks interrupts with `save_and_cli` around every packet buffer it allocates or frees, so the ring turns over many times during a join, which is why the marks exist. A masked stall inside a wrapped critical section ends the ring at that section's entry, and its `lr` names the function that masked interrupts. If the `connect` mark is newer than `connect_ret`, or `connect_ret` is `none`, the join never returned. With no trail of the boot just before, the line says why: `PLSUP1 trail refused=empty` (none was started, as after a power cut) or `PLSUP1 trail refused=boot boot=… current=…` (another boot's).

**How it works.** The builder links the KM4 image with `-Wl,--wrap` for `save_and_cli`, `restore_flags`, `rtw_set_timer`, `rtw_del_timer`, `rtw_cancel_timer`, `wifi_indication` and `rtw_os_indicate_disconnect`, so every call from another object, the prebuilt Wi-Fi driver's included, goes through a wrapper in `example_entry.c` that notes it and calls the real function; KM0 links no wraps. The trail is 220 words (880 bytes) at `0x1007BC50`, just below the wall clock's time carry at `0x1007BFC0` ([clock](clock.md)), taken from the heap's end by the builder's KM4 linker script and named `wellfed_status_trail`. One builder constant sets its size, which a test checks against the shim's `WELLFED_TRAIL_WORDS`, and the build's audit pins the address with the time carry's and `__sram_end__`. Every boot reads the last boot's trail, then starts its own right after the crumb (`wellfed_hal::supervision::trail::take`). How a note claims its slot without masking, why a reset mid-note or mid-mark is dropped, why the D-cache writes the trail through to RAM before a reset, and why the SDK's startup can't overwrite it are in the comment above the wrappers in `src/firmware/vendor/ports/amebad/example_entry.c`. The layout and validity checks are in the module docs of `src/sdk/wellfed-hal/src/supervision/trail.rs`.

Limits:

- **Calls inside the defining object aren't seen,** since `--wrap` redirects only undefined references. `rtw_set_timer`, `rtw_del_timer` and `rtw_cancel_timer` are defined in the driver's `rtk_wlan_if.o`, whose internal calls go unnoted (`rltk_set_mode_prehandle` calls `cancel_timer_ex` and `rltk_set_mode_posthandle` calls `mod_timer` directly). So do `freertos_intfs.o`'s own calls to `rtw_os_indicate_disconnect`, `wifi_ind.c`'s to `wifi_indication`, and the osdep atomic operations in `freertos_service.o` that call `save_and_cli` inside its defining object.
- **FreeRTOS's own critical sections aren't wrapped:** `taskENTER_CRITICAL` in the kernel and the driver's `rtw_enter_critical_bh` spinlocks leave no note.
- **The driver's register-access sections aren't wrapped.** `rtw_enter_critical` and `rtw_exit_critical` guard every register access (`bus_read`, `bus_write`), the WLAN interrupt, the receive and transmit tasklets and the command queue. Wrapping them stretched the first WPA3 join past the watchdog feeder's 1 s gap (observed, October 28, 2026), so a stall inside one shows as the last note before it.
- **A boot that never armed** (it hung before arming, or skipped arming after an unfinished store write) doesn't count itself, so the next boot refuses its trail as `refused=boot`.
- **Power cuts** leave whatever the RAM holds, which the magic and check refuse, as they refuse a build without the trail that used the RAM as heap.
- **Timing.** About 40 instructions per wrapped call, a few more for a mark (masked for about 10), and 880 bytes of heap. Each wrapped critical section's masked window grows by about 40 instructions, so the hang rate may shift.
- **Many interrupts at once.** 64 or more interrupt notes landing while one task note is half written lap its slot, which may then mix two notes. Marks can't mix.

## Evidence

- **Startup report.** Observed: an artifact-bound startup passed `wellfed-audit startup` on the research unit. This doesn't establish long-term uptime, persistent boot counters or a verified physical reset cause.
- **Stopped outputs.** Observed: exact GPIO readback, an artifact-bound normal boot and operator-reported no motion. Independent voltage and infrared-off measurements, repeated measured resets and transient checks belong to the electrical output verification ([motor driver](../devices/plaf103/motor-driver.md)). The single direction-register enable passed 50 cold boots on the analyzer (observed, October 27).
- **PB3 and SWD.** Observed: before startup released SWD, PB3 was never driven, and its LOW readback was the pad's pull-down ([optical inputs](optical-inputs.md#pb3-is-swd_clk)).
- **Shared-pad power.** Observed: with the new order, an intact `PLPWR1` ahead of KM4 output with `after_pad=00000301` (LDO powered, zero trim); a later build with the trim read the same Port A word at every step. Recovered, not measured: the removed `trace-stock-pad-power.py` (in Git history) ran the stock initializer and LDO instructions against modeled registers, telling LDO power from the pad-enable bit.
- **Indicator pins and polarity.** Observed: with the pads powered, the operator saw all four indicators blink twice and finish dark (HIGH on, LOW off). The output-only build then passed the auditor on one boot: `PLPWR1` with trim 0x2A, PA0, PA2 and PA4 `status=output`, PB22 `pass`, terminal LOW. An earlier capture (HIGH latch with LOW Port A input, two operator boots, interleaved KM0 and KM4 reports) keeps its failing verdicts.
- **Radio hold.** Observed (`fw40-diag-01`, `fw40-diag-03-wifi`): the first WPA3 join starves lower-priority tasks for about 700 ms, past a startup probe's 300 ms owner lease. That is why the radio waits for `startup_done`.
- **Link liveness.** Recovered and modeled (October 25), not yet run on hardware. The old DNS check passed on the bench hotspot (no forwarding) only because the hotspot answers DNS locally. The bench host accepts ICMP on every interface, so expect `kind=gateway result=0` every 5 s and an outage to show as an association or gateway loss. A run with forwarding blocked is still to come.
- **Status 30 hangs.** Observed: 24 of 441 status-30 rejects on our builds stalled KM4, 17 of 35 on the October 25 evening builds, and 8 of 8 on stock; status 53 never did. The leave itself is recovered and modeled (October 25), not yet run on hardware. **Cause and fix (October 28, physical):** the call trail caught 9 hangs in 19 power cuts, each ending the same way: the join-timeout teardown in the timer task was clearing the key table (`rtw_clearstakey_cmd`, then `ROM_WIFI_CAM_WRITE`, plain stores to the WLAN MAC at `0x4008067x`) just after the power-save processor's `ips_enter` had powered the MAC down. A store to the powered-down MAC stalls the bus, and the tick with it. The Wi-Fi task now turns inactive power save off after `wifi_on` (`wifi_set_power_mode(0, 1)`, with leisure power save kept; both are off since November 1, 2026): 20 cuts, status 30 on each, no hang. The leave before a restart stays, since it saves a rejected join.
- **Silent-reset crumb.** Recovered and modeled (October 25). The first hardware crumbs all read `task=wellfed-`, because the vendor's 10-character name limit cut every runtime task's name; the builder raised it to 16 on October 26.
- **Call trail.** Recovered and modeled (October 28): the wrapped symbols are undefined in the prebuilt driver objects that call them (`nm` over `lib_wlan.a`), and host tests cover the decoding. The status-30 run above (October 28, physical) then printed trails after real hangs. A build that also wrapped `rtw_enter_critical` and `rtw_exit_critical` failed all three OTA trial boots (observed, October 28, capture `fw66-trail-20261028-02`): each stopped about 10 s in, during the first WPA3 join, with the watchdog feeder more than 1 s late, and KM0 fell back to bank 0.
