# Bring-up probes

Before the replacement firmware could feed, a probe brought up each piece of hardware: a bounded test that ran after startup, printed fixed UART records, then stopped. This reference lists what each probe did, its records, and what a passing capture looks like. It is for contributors reading old captures or rerunning a probe. The everyday startup is in [startup and diagnostics](startup-diagnostics.md).

The bring-up firmwares and the [QA firmware](qa.md) still run the optics, buttons, ADC, speaker and pulse probes from `src/sdk/wellfed-probes`. The normal firmware runs none of them.

**How the audit binds them.** `wellfed-audit startup --device plaf103` binds each live policy both ways to a record format in the image: a manifest that declares the policy needs the marker string in the image, and an image carrying the marker needs the policy. Policies for different probes exclude each other, so an image is audited under exactly the probe it carries. The retired probes below (the supervision probe, the flash-coordination probes from the [boot handshake](#boot-handshake) to the [settings power-cut test](#settings-power-cut-test), and the health transport) are refused: no current image carries them, so a manifest naming one of their policies, or an image carrying their start record, fails. Their sections stay as the record reference.

| Probe | Policy | Records | In the audit |
| --- | --- | --- | --- |
| [Indicator](#indicator-probe) | `bounded-probe-v3` | `PLLED1`, `PLDBG1` | Yes, for older captures |
| [Supervision](#supervision-probe) | `fw05-probe-v1` | `PLTMR1`, `PLWDG1` | Retired |
| [Boot handshake](#boot-handshake) to [settings power-cut test](#settings-power-cut-test) | `fw06-*`, `fw15-*` | `PLFLS1`, `PLPRK1`, `PLRDS1`, `PLWRS1`, `PLJNL1`, `PLSET1` | Retired |
| [Power input](#power-input-probe) | `fw14-adc-v1`, `-v2` | `PLADC1` | Yes |
| [Wi-Fi scan](#wi-fi-scan), [join](#wi-fi-join) | `fw07-scan-v1`, `fw07-join-v1` | `PLWIF1`, `PLWIF2` | Yes |
| [Wi-Fi DNS check](#wi-fi-dns-check-retired) | `fw07-dns-v1` | `PLWIF3` | Yes, until no frozen review declares it |
| [Health transport](#health-transport-deleted) | `fw08-transport-v1` | `PLMQT1` | Refused |
| [Optics](#optics-probe) | `fw13-optics-v1`, `-v2` | `PLOPT1` | Yes |

## Indicator probe

Policy `bounded-probe-v3`, kept so older captures still audit; startup now uses `low-only-v1` ([startup](startup-diagnostics.md#indicators-at-startup)).

The controller preloaded the indicator latches LOW, selected the stock-derived GPIO no-pull configuration, and changed only PA0, PA2, PA4 and PB22. Every request needed valid indicator directions and stopped-output readback, yielded 10 ms, then observed the latch, input, pad control and direction. After a nominal 3 s it gave each pin, in that order, HIGH 1 s, LOW 0.5 s, HIGH 1 s, LOW 1 s, then requested all LOW and ended. No indefinite blinking, no feeding request, no retry.

- **Output-only pins.** PA0, PA2 and PA4 have no usable input readback on our unit, so their steps report `status=output` when latch, pad control, directions and stopped state agree (`PLLED1 begin output_only=PA0,PA2,PA4`). `output` is never `pass`: the electrical level rests on the operator's eyes. PB22 keeps full input verification.
- **Audit fields.** The sequence outcome is `pass`, but `led_probe_verified` stays `false` with output-only pins present. `led_input_verified_pins` (`PB22`) and `led_output_only_pins` (`PA0`, `PA2`, `PA4`) say what was read back.
- **Extra samples.** 64 extra input samples (`highs=N/64` in `PLDBG1`) tell a dead input (0/64) from a marginal threshold (a partial count). Verification uses the single read, so a high count cannot verify a failed step.
- **Failures.** A HIGH input mismatch fails the observation but allows the bounded visible interval. A LOW mismatch, a changed latch, mux or direction, or a lost stopped state aborts, and lost stopped readback requests STOP and OFF again. `finish=input_mismatch low=all` is a failure.

## Supervision probe

Policy `fw05-probe-v1`, retired. It proved the stop timer and the watchdog with motion disabled. After the indicator probe, a cold boot opened the stopped board (motor STOP, emitters OFF, no sensor inputs, efuse or ADC) and TIM2, then:

1. logged 13 `PLTMR1 sample` snapshots: SYSTIMER raw, supervision ms, accepted TIM2 interrupts, and ready, healthy and stopped;
2. ran a 2 s busy stall;
3. armed the watchdog (`PLWDG1 arm`) and fed it 15 s (`fed`);
4. withheld progress (`stall_begin`, `stalled`) until one watchdog reset.

The next boot printed only `PLWDG1 skip`; the auditor treated the two-boot capture as `repeated_startup`. Before arming, the probe wrote a marker to SDK user backup register 1 and read it back, and any boot that saw a marker or a boot reason cleared it and skipped. Had both guards failed, the probe would have repeated about every 100 s until power-off; the UART loader stays available at power-on.

Nominal clocks: SYSTIMER and TIM2 at 32 768 Hz (SDK: 31 µs ticks), and a watchdog of 4095 × 32 counts at about 33.3 kHz (`WDG_Scalar`), bounded to 2.9–5.3 s by 25–45 kHz limits.

## Boot handshake

Policy `fw06-hello-v1`, retired. A pass was exactly one HELLO, READY, peer and done record, in order, after `PLLED1 done low=all`:

1. KM4: `PLFLS1 hello authority=<16 hex> count=<n>`, its session the boot count in the high word and a fresh low word.
2. KM0 answers READY with its own session. KM4 binds a flash coordinator only while outputs are stopped: `PLFLS1 ready peer=<16 hex> reply=0 phase=1 fault=0 inactive=1` (ACCEPTED, IDLE, no fault).
3. The test KM0 republishes a new session about 10 s later, simulating a rebooted peer: `PLFLS1 peer peer=<16 hex> reply=5 phase=8 fault=7 blocked=1` (FAILED, FAULT, PEER_LOST), the high word one above the first.
4. 30 s after READY: `PLFLS1 done phase=8 fault=7 blocked=1`.

One `PLFLS1 foreign` may follow HELLO. Failures: `PLFLS1 error timeout ms=<n>` or `PLFLS1 error clock`. By host receive times READY came within 2.5 s of HELLO, the peer change 9–12 s and done 29–32 s after READY. This is protocol evidence, not proof that any flash write path is safe.

## Park round trip

Policy `fw06-park-v1`, retired. Each boot prints HELLO and READY, with no `PLFLS1 peer` or `done`. Then `PLPRK1 start mode=<0|1|2> marker=<hex8> watchdog=<pass|fail>`, and `PLPRK1 batch leases=<n> park_max=<ticks> release_max=<ticks> frozen=<n> alive=<n>` at 25, 50, 75 and 100 leases. Maxima are 32 768 Hz ticks, bounded by the 40 ms exchange limit (1310). A pass is one power-on with exactly three startups:

1. **Mode 0** injects `PLPRK1 inject mode=0 lease=101 begin=0 ... ms=0`: KM4 stops acknowledging a park, and the watchdog resets 2–6 s later.
2. **Mode 1** injects `inject mode=1 lease=101 begin=1 ... ms=0` and withholds the release. KM0 publishes TIMEOUT (`peer=504b0081`) after its 700 ms park budget, as a second inject line with `ms` within 650–760. The watchdog resets 2–6 s after the first inject line.
3. **Mode 2** prints `PLPRK1 done leases=100 marker=00000002`, then nothing from `PLPRK1` or `PLFLS1` for at least 20 s.

Every batch needs frozen equal to leases and alive at least leases − 1. HELLO counts are 1, 2 and 3, and KM0's session high words nonzero and strictly increasing. Failures: `PLPRK1 error lease=... stage=... reply=... phase=... fault=...`, `PLPRK1 error no_reset ms=<n>` or `PLPRK1 skip marker=<hex8>`.

## Mediated flash read

Policy `fw06-read-v1`, retired. One boot, HELLO and READY as above, no `PLPRK1`, `PLFLS1 peer` or `done`. Then `PLRDS1 start marker=<hex8> watchdog=<pass|fail> installed=<0|1>`, `PLRDS1 batch reads=<n> id=<6 hex> mismatches=<n> park_max=<ticks> release_max=<ticks> frozen=<n> alive=<n>` at 25, 50, 75 and 100 reads, and `PLRDS1 done reads=<n> leases=<n> phase=<n> fault=<n>`. Failures: `PLRDS1 skip marker=<hex8>` or `PLFLS1 error ...`.

A pass needs `marker=00000000 watchdog=pass installed=1`; every batch with JEDEC ID `c84017` (the research unit's GigaDevice part), no mismatches, frozen equal to reads, alive at least reads − 1, and maxima in 1–1310 ticks; `done reads=100 leases=100 phase=1 fault=0` within 30 s; and 30 s more of capture with no `PLRDS1` or `PLFLS1`. It shows the mediated read, not that any write path is safe.

## Scratch-sector write

Policy `fw06-write-v1`, retired, on one owner-approved 4 KiB scratch sector at `0x185000`. One boot, HELLO and READY as above, none of the other probes' records. Then `PLWRS1 start marker=<hex8> watchdog=<pass|fail> installed=<0|1>`, `PLWRS1 preimage id=<6 hex> blank=<n>`, three `PLWRS1 cycle n=<n> erase_ticks=<ticks> blank=<n> program_max=<ticks> errors=<n>`, `PLWRS1 restore erase_ticks=<ticks> blank=<n>` and `PLWRS1 done cycles=<n> leases=<n> phase=<n> fault=<n> park_max=<ticks> release_max=<ticks> frozen=<n>`, in 32 768 Hz ticks. Failures: `PLWRS1 skip marker=<hex8>` or `PLFLS1 error ...`.

A pass needs:

- `marker=00000000 watchdog=pass installed=1`, and a preimage with ID `c84017` and all 1024 sampled words blank;
- cycles 1–3 in order, each `blank=1024 errors=0`, erase in 1–16384 ticks (the 500 ms lease budget) and program maximum in 1–1638 ticks (50 ms);
- a restore erase in 1–16384 ticks leaving `blank=1024`;
- `done cycles=3 leases=53 phase=1 fault=0 frozen=53`, maxima in 1–1310 ticks, within 60 s;
- 30 s more of capture with no `PLWRS1` or `PLFLS1`.

It covers one sector, not writes elsewhere or under power loss.

## Journal on NOR

Policy `fw15-journal-v1`, retired, on the owner-approved 64 KiB region `0x340000`–`0x350000` (16 sectors). Each boot prints HELLO and READY, none of the other probes' records, then:

- `PLJNL1 start mode=<0|1|2> marker=<hex8> watchdog=<pass|fail> installed=<0|1>`;
- in mode 0, `PLJNL1 preimage id=<6 hex> blank_sectors=<n>`;
- `PLJNL1 op kind=<n> id=<n> ok=<0|1> error=<n> ticks=<n>`, kind 0 create, 1 reserve, 2 finish, 3 open, 4 torn, 5 wipe;
- `PLJNL1 view sequence=<n> high_water=<n> pending=<0|1> result=<n> outcome=<n> slot=<n>`;
- in modes 1 and 2, `PLJNL1 refuse id=<n> reserved=<0|1>`;
- `PLJNL1 end mode=<n> reset=<0|1> leases=<n> frozen=<n> erases=<n> phase=<n> fault=<n>`.

Every boot needs `watchdog=pass installed=1` and its mode as the marker. A pass is one power-on with exactly three startups:

1. **Mode 0** finds ID `c84017` and 16 blank sectors, creates the journal, reserves and finishes ids 1 and 2, reserves id 3, prints `view sequence=6 high_water=3 pending=1 result=3 outcome=0 slot=1` and `end mode=0 reset=1 erases=5 phase=1 fault=0`. The watchdog resets 2.0–6.5 s later.
2. **Mode 1** opens it, prints the same view, refuses id 4 (`reserved=0`), tears the pending record and ends `mode=1 reset=1 erases=1 phase=1 fault=0`. The watchdog resets 2.0–6.5 s later.
3. **Mode 2** opens it, prints the same view and refusal, wipes it and ends `mode=2 reset=0 erases=2 phase=1 fault=0`, then nothing from `PLJNL1` or `PLFLS1` for 30 s.

Every operation needs `ok=1 error=0`. Reserve, finish and wipe need nonzero ticks, and reserve and finish at most 26214 (800 ms), since a journal write is an erase, two programs and verify reads, each in its own lease. Each end needs nonzero leases equal to frozen. HELLO counts and KM0 sessions as in the park round trip. Failures: `PLJNL1 skip marker=<hex8>` or `PLFLS1 error ...`.

## Settings power-cut test

Policies `fw15-settings-v1` and `-v2`, retired. HELLO and READY as above, none of the other probes' records. These `PLSET1 start marker=` records are not setup mode's `PLSET1 start cause=`.

Each boot prints `PLSET1 start marker=<hex8> watchdog=<pass|fail> installed=<0|1>` and `PLSET1 open status=<n> generation=<n> volume=<n> lights=<0|1> slot=<n> consistent=<0|1>` (status 0 LOADED, 1 EMPTY, 2 RECOVERED, 3 CORRUPT, 4 FUTURE, 5 IO, 6 WORN). It then saves settings, printing `PLSET1 batch saves=<n> generation=<n> ticks_max=<n>` every 25 saves, and in place of the last batch `PLSET1 end saves=<n> generation=<n> leases=<n> frozen=<n> phase=<n> fault=<n>`.

| Policy | Saves | Pace | Open record |
| --- | --- | --- | --- |
| `fw15-settings-v1` | 300 | About 8 a second | No `other` |
| `fw15-settings-v2` | 1500 | Back to back, about 35 a second | `other=<n>` before `consistent` |

`other` is the unloaded slot: 1 ERASED, 2 VALID, 3 FUTURE, 4 DIRTY (our record's prefix without its commit), 5 INVALID. Failures: `PLSET1 error save generation=<n> status=<n>`, `PLSET1 skip marker=<hex8>` or `PLFLS1 error ...`.

The capture is one receive-only recording while the operator cuts and restores power at will. Every `PLFW1 start` must report reset flags `00000000`, and a boot cut before its open counts for nothing. A pass needs:

- at least three opened boots, each `marker=00000000 watchdog=pass installed=1 consistent=1` and LOADED or RECOVERED (EMPTY with generation 0 only for the first);
- each later open generation within 25 above the previous boot's highest reported one (up to 24 saves after its last batch, plus one cut save);
- batches at 25, 50, … in order, generation equal to the open generation plus saves, `ticks_max` in 1–26214 (800 ms);
- an end with every batch before it (11 or 59), generation equal to the open generation plus saves, `phase=1 fault=0` and nonzero leases equal to frozen;
- no error or skip line, and at least two boots cut after their open and before an end, each followed by a startup;
- under v2 only, `other` on every open agreeing with its status (LOADED: ERASED or VALID; RECOVERED: DIRTY or INVALID; EMPTY: ERASED), and at least one later boot finding it not VALID, meaning a cut inside a save. Under v1 no open carries `other`.

## Power-input probe

Policies `fw14-adc-v1` and `-v2`, bound to `PLADC1 open outputs=` (v2 also to its ` supply=` field), needing the indicator policy. The `adc` bring-up firmware still runs it for as long as the build runs, and the [QA firmware](qa.md) takes five samples. It never binds the KM0 flash handshake, so the build confirms as usual.

After startup KM4 opens the board's outputs (motor STOP, emitters OFF, no flash authority, so every drive is refused), then its sensors, and prints:

- `PLADC1 clock function_before=<hex8> clock_before=<hex8> function_after=<hex8> clock_after=<hex8> status=<pass|fail>`: `REG_LP_FUNC_EN1` and `REG_LP_CLK_CTRL1` around the explicit ADC enable (bit 8 in each, clock first, no other bit changed).
- `PLADC1 open outputs=<0|1> efuse=<hex8|none> offset=<n> gain=<n> default_offset=<0|1> default_gain=<0|1> status=<pass|fail>`: efuse bytes 0x1d0..0x1d3 in read order and their decoding. Erased halves default to 2480 and 12050, as in stock.
- Once a second, `PLADC1 sample n=<n> ticks=<n> food=<0|1> outlet=<0|1> dc=<0|1> stopped=<0|1> current=<min,max,mean,scaled> battery=<min,max,mean,scaled>`, with ` supply=<min,max,mean,scaled>` under v2. `ticks` times the capture (32 conversions, 48 under v2) on the SYSTIMER.
- `PLADC1 error n=<n> ticks=<n> stopped=<0|1>` on a capture timeout or failed stopped readback, which ends the probe with the board tripped.

The audit checks only that the policy and markers agree, not the samples. What the channels mean is in [power](power.md#inputs).

## Wi-Fi scan

Policy `fw07-scan-v1`, bound to `PLWIF1 start created=`, with the indicator policy. One task in the `example_entry.c` shim ran `LwIP_Init()` and `wifi_on(RTW_MODE_STA)`, then three scans 5 s apart, each waiting at most 10 s and keeping the 32 strongest networks:

- `PLWIF1 start created=<0|1>` and `PLWIF1 on result=<n> ms=<n>`;
- per scan, `PLWIF1 scan n=<n> result=<n> ms=<n> count=<n> dropped=<n> stopped=<0|1>` (`count` kept, `dropped` beyond 32), then `PLWIF1 ap n=<n> i=<n> ssid_crc=<hex8> rssi=<dBm> channel=<n> security=<hex8>` per network;
- `PLWIF1 done scans=3`, or `PLWIF1 error timeout stage=<n> ms=<n>` from a stalled shim.

Networks appear only as a CRC-32 of the SSID; the build held no credentials and joined nothing. KM0 did not start the WLAN firmware in these builds. KM4 log lines print with the scheduler suspended when called from a task, so another task's print cannot split them; interrupts still run.

**The write audit.** The build sets `CONFIG_EXAMPLE_WLAN_FAST_CONNECT` to 0, because fast connect saves reconnect data to `FAST_RECONNECT_DATA` (0x105000), inside the application bank. The write-reachability audit (`wellfed-build`'s `audit/writes.rs`) fails the build if any eFuse or flash writer is linked: `rtw_efuse_boot_write`, `wifi_set_mac_address`, the fast-connect and FTL writers, the mbed flash write API, or any shell entry. It also fails if the references to ROM or RAM eFuse or flash write entries differ from a pinned set: the RAM `*XIP` wrappers, the unreferenced `FLASH_EreaseDwordsXIP`, the driver's private-command eFuse path (`rtw_private_handler`, then `rtw_ex_set`, then `rtw_config_map_write`), and the vendor shell command table, which no linked code starts. It sees direct and stored-address references only.

## Wi-Fi join

Policy `fw07-join-v1`, bound to `PLWIF2 start created=`, with the indicator policy. The shim ran `LwIP_Init()` and `wifi_on(RTW_MODE_STA)`, then repeated one attempt: scan for the configured SSID (learning its security and RSSI), `wifi_connect`, then `LwIP_DHCP(0, DHCP_START)`. Any DHCP state but assigned, including the SDK's static 192.168.1.80 fallback, stops DHCP, releases the address and disconnects.

A joined link is polled every 500 ms; a drop is recorded, and DHCP stopped and the address released. Attempts back off from 1 s, doubling to 30 s, reset to 1 s after a join. Nothing resets, writes flash or registers the SDK's reconnect. Events go to a 16-entry numbered ring the Rust side drains: `PLWIF2 ev seq=<n> kind=<on|scan|connect|dhcp|lost|backoff> attempt=<n> result=<n> value=<hex8> rssi=<dBm> security=<hex8> ms=<n> stopped=<0|1>`, or `PLWIF2 error overrun seq=<n>`. The everyday firmware keeps this loop and adds the [gateway watch](startup-diagnostics.md#link-liveness-without-internet-dns).

The SSID and password came from the ignored `private/hotspot.json`, which `wellfed-build` turned into an ignored generated header in the SDK worktree. Only the file's hash reaches the manifest, and our records print neither. The SDK driver prints the SSID (`set ssid [...]`), so join captures stay private.

**The WLAN firmware runs on KM0.** It comes from `lib_wifi_fw.a` through `wifi_FW_init_ram()`, which starts `TaskInit_8721D` and the IPC command task `TaskFW_IPCMD_8721D`; without it the driver's firmware commands are never consumed. `wellfed_wifi_fw_start()` calls it before `km4_boot_on()` when `BKUP_Read(0) & BIT_WIFI_ENABLE`, the vendor main's gate (`app_start` sets it from `ps_config.km0_config_wifi_enable`). The first-boot main audit (`audit/mains.rs`) pins the call. It creates three KM0 tasks, about 6 KB of stack from the 20 KB KM0 heap. A park runs with interrupts masked, so none of that code runs while KM4 holds a flash lease, though it stalls up to the park budget. The write audit covers KM0 too: `lib_wifi_fw.a` imports no flash, eFuse or shell symbol, every KM0 reference to a flash or eFuse write entry is pinned (the SDK's boot-time flash setup and the shell table), and nothing references the ROM shell entries. The firmware's ROM half is not covered.

The vendor makefiles track no header dependencies, so the build records the platform options' override digest and deletes every KM4 object when it changes.

## Wi-Fi DNS check (retired)

Policy `fw07-dns-v1`, bound to `PLWIF3 start created=`, with the indicator policy. The [gateway watch](startup-diagnostics.md#link-liveness-without-internet-dns) replaced it, because a feeder without a route to the internet churned rejoins under it.

Joined as under `fw07-join-v1`, the shim sent its own A query for `example.com` to the DHCP-provided DNS server (`dns_getserver(0)`) every 5 s, on a UDP socket with a 3 s timeout, bypassing lwIP's resolver, whose cache answered with no traffic. A response passed with the query's ID, no truncation, rcode 0 and an A record. Failures: -1 no server, -2 socket, -3 send, -4 timeout, -5 mismatch or truncation, -6 rcode, -7 no address. Two in a row were a DNS loss (`lost` result 1): DHCP stopped, the address released, a disconnect and a rejoin. A link drop was a loss with result 0. The backoff reset only after a good lookup, so a network with leases but no DNS backed off to 30 s. Records were the `PLWIF2` set under the `PLWIF3` prefix, plus `kind=dns`.

## Health transport (deleted)

Policy `fw08-transport-v1`, bound to `PLMQT1 start created=`, needing `fw07-dns-v1`. A second task connected to the local server's MQTT broker once DNS worked and reported health. `PLMQT1` recorded task start, broker connects, session ready, health publishes and their trigger, request outcomes, closes and their reason, or a config error. Deleted October 25: the builder records no transport policy, and the audit refuses an image with the start record.

## Optics probe

Policies `fw13-optics-v1` and `-v2`, bound to `PLOPT1 start created=`, with the indicator policy. Records, plans and results are in [optical inputs](optical-inputs.md).

## Evidence

All observed on the research unit over the UART, unless marked.

- **Supervision probe.** One receive-only cold-boot capture passed every startup policy on both boots, and the operator saw the indicator sequence twice.
  - SYSTIMER about 32 752 Hz against host receive times over 60 s, −475 ppm, within the host timestamps' ±0.1 %; it only moved forward. TIM2: 10.015 ms against 10.010 ms nominal for 328 ticks.
  - The 2 s stall (65 536 ticks) kept TIM2 interrupts running, and supervision stayed ready, healthy and stopped.
  - The watchdog armed with vendor word `5601001f` and was fed 15 s with 60 checkpoints. The stalled loop latched fault 4 (PROGRESS) and never refreshed again. The reset came about 3.0–3.1 s after the last refresh, but `stalled` printed once a second, so `ms=3015` bounds it only to about 3.0–4.3 s.
  - The next boot reported boot reason `00100000` (`BIT_BOOT_KM4WDG_RESET_HAPPEN`) and the marker, and skipped: both guards worked.
  - Owner-lease expiry under a stall was not exercised; real motion showed it later ([motor pulse](motor-pulse.md#a-stalled-loop-and-a-failed-stop-during-a-feed)).
- **Watchdog clock.** The handshake runs' resets came about 4.0–4.2 s after the last rate-limited refresh, near 31–33 kHz. On October 25, five resets from the `feeder` runtime came 3.97–4.05 s after the logged last refresh (`PLSUP1 stop … fed_ms=`), by the host's UART chunks: about 32.8 kHz, the SDK's SDM 32.768 kHz clock, which KM0 calibrates every boot (recovered).
- **KM4 SysTick CLKSOURCE** reads 1 whatever is written, so park and hook context checks test only ENABLE and TICKINT.
- **Boot handshake.** Passed on one cold boot. KM0 answered within one receive chunk, and a deliberately changed KM0 session latched PEER_LOST 10.0 s later, with outputs blocked. Nothing was written.
- **Park round trip.** One power-on gave three boots, as designed: the withheld ACK reset 3.91 s later; after the withheld RELEASE, KM0 published TIMEOUT at 705 ms and the platform reset; a clean run followed. All 300 leases showed KM0's heartbeat frozen while parked and advancing between. Worst park 6 SYSTIMER ticks, release 2.
- **Mediated read.** Passed on one cold boot. With the lease provider in the SDK flash-lock hooks, 100 `FLASH_RxCmdXIP` JEDEC reads returned `c84017`, each in its own lease with KM0 frozen. Worst park 4 ticks, release 2, 2.06 s in total; the coordinator ended idle and unfaulted. This shows one read-only SDK path through the hooks, not exclusive ownership for every SDK user.
- **Scratch write.** Passed on one cold boot, on `0x185000`: ID `c84017`, blank preimage, three cycles with zero errors, a final erase back to blank, all 53 leases freezing KM0. Erase 27–35 ms including the lease, a page program at most 0.7 ms. It covers the hooked read, sector-erase and page-program paths only; KM0's own SDK flash users and other KM4 users (Wi-Fi, OTA) were then unmediated.
- **Power inputs, adapter only.** One cold boot, no batteries, motor stopped. Startup had already set the ADC function and clock bits (`001115f7`, `00131557`), and the explicit enable read back unchanged. The efuse holds programmed calibration. 93 samples came 1.02 s apart, each 32-conversion capture in at most 10.1 ms. PA19 read 1, PA16 and PA17 0 with emitters off. Channels 4 and 5 read raw 239–243 (mean 240, scaled 16): no battery, no motor current.
- **Supply channel.** A `fw14-adc-v2` boot (95 samples) read channel 10 at raw 3911–3924, stock-scaled 3315–3317, while channels 4 and 5 stayed 239–244, so the select works. Channel 10 is the I/O supply (the SDK's 3.3 V detection reads it), and the idle UART TX at that supply measured about 3.29 V on a meter ([UART notes](../devices/plaf103/uart.md)): stock scaling gives millivolts within about 1 %, for this channel at least. Each 48-conversion capture took at most 15.1 ms.
- **Batteries** (October 23, UART and meter): the `adc` probe over the air (v64), confirmed. A cold boot with no batteries passed `wellfed-audit adc` (58 samples, `fw14-batt-01-baseline`). The owner then inserted three D cells and pulled and replugged the adapter without a reboot ([power](power.md#evidence)). Captures `fw14-batt-02-both`, `-04-unplug` and `-05-meter` have no startup, so they were read, not audited.
- **Wi-Fi scan.** Two cold boots on the adapter. `wifi_on(STA)` took 234–236 ms, and the three scans completed in 4.0–4.3 s with stopped outputs, hearing 51–63 networks. The driver printed `FW Enable failed` up to three times a boot, then suspected to come from the WLAN firmware not running on KM0. In the first boot that print split `PLWIF1 start` and the capture was rejected; the second, with scheduler-suspended logging and strongest-32 retention, passed. The owner's hotspot was not among those kept; whether it was broadcasting was not recorded.
- **Wi-Fi join.** Without the WLAN firmware on KM0, SAE, association and the 4-way handshake completed but DHCP got nothing, and the driver printed `fw cmd fifo full` 98 times. With it, those stopped and the DHCP DISCOVERs reached the hotspot well formed, where the host's nftables input policy dropped them. Once the owner allowed DHCP from the hotspot interface, one cold boot passed `fw07-join-v1` (UART plus the dnsmasq journal). `wifi_on` took 149 ms. The first association was rejected with status 30, the access point still holding the previous boot's protected association. After a 1 s backoff, attempt 2 joined 24.3 s after radio on (connect 5.9 s, DHCP 3.8 s, a dnsmasq-acknowledged lease). The operator's outage showed as a loss after 30.5 s connected; backoffs ran 1, 2, 4 and 8 s, and the feeder rejoined 41.7 s after the loss, renewing the same lease in 39 ms. The password never appears in the output. The same hotspot had leased to this feeder under stock. Objects compiled before the platform options override once still linked `restore_wifi_info_to_flash`, which the link audit caught.
- **DNS check.** Three cold boots on the owner's hotspot (UART plus the host's journal for block times). Through lwIP's resolver every lookup passed a 51 s DNS block in 0 ms, from its cache. With the explicit query, a boot with no block made 27 lookups in 1–63 ms, absorbing one spontaneous timeout. The passing boot joined 21.5 s after radio on. The operator blocked DNS; after two timeouts, 11 s later, the feeder declared a loss and rejoined, each rejoin leasing then failing DNS twice, with backoffs of 1, 2 and 4 s. DNS returned 50 s after the block, and the next attempt resolved 10 s later, 48.4 s after the first loss; 14 more lookups succeeded. Nothing reset. DHCP-server loss and lease expiry (1 h lease) were not exercised.
- **No flash writes from Wi-Fi.** A full 8 MiB read in loader mode, one 3-minute boot with a 24 s hotspot outage and a 41 s DNS block (passing `fw07-dns-v1`), then a second full read: byte-identical, so nothing wrote flash during that boot. The outage surfaced as a DNS loss before the driver's beacon-loss detection. One boot of a few minutes, not long-running operation or lease renewal.
