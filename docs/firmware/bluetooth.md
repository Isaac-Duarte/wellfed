# Bluetooth LE and setup mode

The feeder uses Bluetooth LE only to set itself up, only when you ask or it has no network, and never as an audio device. This page is for owners setting up a feeder and contributors working on the setup path. Keys and pairing in general are in [pairing](pairing.md).

## The one idea

Bluetooth is off unless you turn it on with the buttons, or the feeder has no network to join. Stock 1.1.15 uses Bluetooth LE to give an offline feeder its Wi-Fi and MQTT settings from the vendor app ([stock behavior](../devices/plaf103/stock/README.md#bluetooth-le)). The `feeder` firmware keeps that job in **setup mode**:

- **When:** you hold Lock, then Mic, for 5 s on an unlocked panel. A feeder with no build Wi-Fi and none stored (a consumer image, fresh from stock or after a factory reset) also starts it at boot, unless it has an enabled meal plan. A boot with a network to join never starts the Bluetooth stack.
- **What:** your phone (or `wellfed setup`) runs a Noise XX handshake with the feeder and shows a one-time code of 6 button presses. You key it in on the feeder, which ticks for each press. Both sides prove the code to each other. Only then does the phone send its Wi-Fi, and its key becomes an owner key.
- **Then:** the feeder saves the Wi-Fi for its next join, and reboots without Bluetooth.

Nothing over Bluetooth can feed, play a sound, or change anything but the Wi-Fi and the client list. The AmebaD's Bluetooth is LE only (`bt_flags.h` sets `F_BT_BREDR_SUPPORT 0`), with no A2DP or LE Audio, so the feeder never appears to a phone as an audio device.

## Setting up from a computer

`wellfed setup --ssid NAME` reads the Wi-Fi passphrase from stdin, uses the computer's client key (`wellfed-owner.json` in the CLI's data directory; `--create-key` makes it the first time, mode 0600, and every command refuses it once other users can read it), prints the code with each button's position and name, and records the feeder's identity in the same file after `Done`. The protocol is `wellfed-client`'s, which the Android app shares.

It uses BlueZ (`bluer`) on Linux and CoreBluetooth (`btleplug`) on macOS, finding the feeder by its service UUID among devices advertising now, or by `--address`. macOS never shows a Bluetooth address, so there `--address` takes the UUID the Mac assigns, which the CLI prints when it finds a device. The macOS path has not run against a feeder yet.

> [!NOTE]
> The CLI's own search can miss an advertising feeder. If it does, run `bluetoothctl scan le`, then pass the feeder's address with `--address`.

## Setup exchange

Service `504c5345-5455-5000-8000-000000000000` has one control characteristic (`…0001`, write and notify), defined with the framing in `wellfed_core::ble_setup`. Every message either way is a u16 little-endian length, then at most 448 bytes. Each side reassembles the pieces, so nothing depends on the negotiated MTU.

The messages are `wellfed_proto::setup` (postcard) inside Noise XX with the prologue `wellfed-setup-v2`. The client initiates; the feeder responds under its identity key:

1. The client enables notifications and sends XX's first message (its ephemeral key).
2. The feeder answers with the second, carrying its identity key, encrypted.
3. The client sends the third: its static key, with its `Request` (a label) as the encrypted payload.
4. The feeder checks the label, offers the key to pairing, and replies `EnterCode { seconds: 60 }`. The client shows its code: 6 presses, each one of the four buttons, numbered from the left (1 Lock, 2 Dispense, 3 Mic, 4 Reset).
5. The owner keys the code in within 60 s. Pairing takes every gesture until the code is complete, and nothing reaches the panel, so no press feeds, toggles the lock or resets the keys. A press counts as it comes up, and only if its button went down during the code and came up on its own, short. A chord or a hold refuses the offer (`unconfirmed`), and a setup or recovery hold then acts as it would with nothing pending. The speaker ticks (`key`) for each press that counts.
6. Within 15 s of the last press, the two sides prove the code to each other (`wellfed_session::setup_code`, [below](#why-a-code)).
7. Once the feeder has proven the code, the client's last step carries the Wi-Fi. When it opens, the settings writer enrolls the key as an owner, then saves the Wi-Fi. The feeder replies `Done { enrolled, wifi }` and reboots without Bluetooth about 3 s later, joining with the saved credentials. Only now does the client trust the feeder's key, whose first 8 bytes match the feeder's `PLID1` fingerprint.
8. The speaker plays `coin` on `Done`, and `alert` when the code was wrong or never finished. The `sound` setting off mutes all three.

**Refusals** close the link; setup mode stays on for another try.

| Case | Answer |
| --- | --- |
| Code not keyed in, or the proof not finished, in time | `Refused(Unconfirmed)` |
| The client's proof failing at the feeder | `Refused(WrongCode)` |
| Another ask already waiting | `Busy` |
| Eight clients already (`CLIENTS_MAX`) | `Full` |
| A store that cannot be written | `NotWritable` |
| A label or credentials the stores refuse | `Invalid` |

- **The feeder's proof failing at the client:** the client stops without sending its Wi-Fi and reports a mismatch (mistyped, or another device answering). The feeder then times the proof out.
- **A failed handshake** (another protocol or version, tampering), a message out of turn, or one over 448 bytes drops the link without a reply.
- **A client that disconnects** while its ask waits for the code cancels it, so the next Dispense press feeds again. One that leaves after the proof is still enrolled and its Wi-Fi saved, and a client connecting meanwhile is dropped (`fault stage=busy`) until that finishes.
- **A second client** whose ask arrives after an earlier client's link has vanished replaces that stale offer (`a_new_client_replaces_an_ask_whose_link_vanished`).

### Why a code

XX alone trusts whichever device answers. With only a Dispense press as the check, a device in radio range during the window could relay between phone and feeder: the phone would take the relay's key for the feeder's, the press would enroll the relay as an owner, and the relay would read the Wi-Fi passphrase. The feeder has no display for a comparison, so the owner carries a secret the other way, from the phone's screen to the feeder's buttons.

**The proof** is modeled on Bluetooth's passkey entry:

- **Bound to the handshake.** Both sides bind every step to their own Noise handshake hash. A relay has a different handshake on each leg, so a step from one leg never opens on the other.
- **One press at a time.** A proof of the whole code at once could be tried against every code offline. So for press `i` each side sends a commitment (HMAC-SHA256, keyed by a fresh nonce, over the side, `i`, the press and the hash), and opens it (sends the nonce) only after the other side's commitment to press `i` has arrived. The feeder opens first. Each message opens the sender's last commitment and commits to its next press: 7 messages each way.
- **The odds.** A relay must commit to each press before it learns it, and failing one leg teaches it one press. So it fools the phone with odds of 4^-5 (1 in 1,024) and gets enrolled with odds of 4^-6 (1 in 4,096), once per code the owner keys in. Six presses (`PRESSES`) trade some margin for a code quick to key in. Each code is used once.
- **A client that chose its own code.** The odds cover a relay. A client in radio range, or on the LAN in a recovery window, offers directly and chooses its own code, so it wins if the owner's other gestures key that code in. The feeder counts only a press that went down and came up alone and short during the code, so the first button of the owner's setup or recovery hold, which goes down before the chord forms, counts for nothing, and the hold refuses the stranger's offer instead. It refuses a code of one button pressed six times too, which an owner pressing Dispense for a feed that does not come would key in, and clients never draw one (`Code::from_bits`). What is left is an owner keying in a code shown on someone else's screen. A client on the LAN can't take recovery, but it can deny it: offering first in every recovery window takes the one offer each window holds, so the owner's own client is refused as busy. BLE setup, which needs the buttons and radio range, is the fallback then.
- **The secret goes last.** The client sends the Wi-Fi only after the feeder's last opening has matched, and accepts `Done` only after that. A relay that fails the proof gets neither the passphrase nor a believed `Done`.

## Runtime

**The C side.** `wellfed_ble.c` starts one task (`wellfed_ble`, 1024 words, priority 1). It waits up to 60 s for the Wi-Fi task's radio, since `hci_board_init` refuses before `wifi_on`, then brings the stack up in the vendor example's order and runs the GAP message loop.

- **One link, no bonding.** `gap_config_le_key_storage_flag(0)` keeps no keys, and BLE pairing is off (a just-works request is rejected). Access control is the Noise handshake and the code.
- **Advertising** runs only while Rust asks: connectable, about every 100 ms, with the 128-bit service UUID, and the name `Wellfed` in the scan response.
- **GATT callbacks** only copy bytes into rings (control 1 KiB, notifications 512 B) and record events. Rust's requests (notify, advertise, disconnect) go through the task's IO queue, so every stack call runs on the task that started the stack.

**The Rust side** is setup mode in the loop task (`wellfed_feeder::feeder::setup`), stepping the link `wellfed_hal::ble::BleSetup` through the firmware's `Platform`:

- **Started** only when setup mode begins, so the stack's heap and C tasks cost nothing on a boot with a network.
- **At boot** the loop decides once, after the settings writer has loaded the Wi-Fi store (writable) and the plans. It starts setup mode if the image has no build Wi-Fi (`Platform::build_wifi`, from the runtime's `BUILD_WIFI`), nothing is stored and no plan is enabled. A boot a reset storm holds waits for the hold to end. Credentials cleared later start nothing until the next boot. An unreadable Wi-Fi store starts nothing, since setup could not save to it: hold Lock and Mic, or reset the feeder.
- **Unprovisioned feeders** enter setup mode at every boot until they have Wi-Fi, and reboot after 15 minutes with no session. Dispense still feeds while no phone runs a session, and feeds nothing during one, since the code uses the same buttons. Holding Lock and Mic while unprovisioned setup runs changes nothing. A setup started with Lock and Mic refuses every feed.
- **Advertising** only while the power monitor reports the adapter (as stock does), never while connected or after setup has finished.
- **Idle hosts:** a connection not finished within 150 s is dropped (the handshake, then up to a minute for the code).
- **The end:** about 3 s after `Done`, or after 15 minutes with no session, the loop reboots through the watchdog, waiting for a feed in flight. The restart is deliberate, so it ends KM0's reset-storm count however soon it comes; only an unconfirmed build's trial counts it. An unprovisioned feeder that idles out enters setup mode again.
- **The speaker.** The feeder's two firmware tasks are taken, so setup mode's loop steps the speaker itself. Linking it costs about 9 KB of flash and 4.8 KB of static RAM, mostly the codec's DMA ring. Setup mode's state lives in the owner's static, built at compile time, because on the loop's stack it overflowed the audit's limit ([evidence](#evidence)).

**Records.** `PLBLE1`: `ready heap_free=…`, `advertising on=…`, `connect`, `mtu`, `drop reason=…`, `disconnect`, `failed stage=…`. `PLSET1`: `start cause=hold|unprovisioned`, `session`, `asked wifi=…`, `enroll result=…`, `wifi saved=…`, `reply …`, `fault stage=…`, `reboot ended=…`. None carries a key, a label or a credential.

## Build

A firmware opts in with `bluetooth = true` in `[package.metadata.wellfed]`, and the device names its BLE source (`"bluetooth"` in `device.json`). Both are identity inputs. Only such a firmware gets these overlays; every other build restores the pinned files, so its image is unchanged.

- **`.config`** sets only `CONFIG_BT_EN=y`, which builds the BT board code and links `btgap.a`. The C macro `CONFIG_BT` stays undefined, keeping out the boot-time FTL init, the BT AT commands and the vendor examples.
- **`ftl.c`** is dropped from its makefile. The stack's bond store is FTL, which erases and programs `0x102000–0x105000`, inside OTA bank 0. `wellfed_ble.c` defines the two FTL entry points the stack calls, and both refuse.
- **`check_sw()` in `hci_board.c`** is pinned to an erased sector's value. The board code reads active-low switches from system data at flash `0x3028`, choosing which controller patch loads (the embedded one, or one read from a fixed address inside our banks), whether PA26 becomes a trace UART, whether the controller logs, and whether the host stack starts at all. The research unit's word is `0xFFFFFFFF`, which the pin matches, so the image never depends on a unit's system data, and the read takes no flash lease.
- **`wellfed_ble.c`** is staged beside `example_entry.c` and added to the example makefile.

The link audit is unchanged and passes: `ftl_init` and every flash erase and program routine stay absent, and no new eFuse writer is linked (`bt_write_phy_efuse_value` has no caller and is collected). The image carries the 11.5 KB embedded controller patch (`rtlbt_fw`).

## What's open

- **Not yet on hardware:** a deliberately mistyped code (`wrong-code` at the CLI, then `refused reason=unconfirmed` and `alert` after 15 s), and the stale-offer fix. Host tests cover both.
- **CLI discovery:** why the CLI's search misses an advertising feeder.
- **Flash writes with BLE up:** the settings writer and the confirmation record mask interrupts during a sector operation. Watch for HCI disruption.
- **Heap:** `ready heap_free` read 45–70 KB with an earlier speaker build.
- **Task stacks:** the C-created BT tasks (`bte_task`, `trace_task`, the HCI send task and `wellfed_ble`) are outside the stack audit, resting on the SDK's sizes and the setup-mode heap figure in the [resource budget](resource-budget.md#ram).
- **Starting the stack beside a joined Wi-Fi:** the resource budget started it at boot; setup mode starts it later, with Wi-Fi up.
- **Setup at an unprovisioned boot** runs from bank 0, where stock installs the image, as both installers show ([consumer installation](consumer-installation.md#evidence)). Its heap has been measured from bank 1 only. The stack needs `wifi_on` first, which a consumer image's Wi-Fi task still calls with nothing to join.

## Evidence

- **Modeled** (October 29, 2026), setup at an unprovisioned boot: `wellfed-fw-feeder/tests/setup.rs` (starts once the stores load; not with stored or build credentials or an enabled plan) and `wellfed-feeder-sim/tests/setup.rs` (pairs over a simulated link, saves the Wi-Fi, starts nothing next boot; Dispense feeds with no session, not during one or after Lock and Mic; the idle reboot waits for a stalled feed).
- **Observed** (October 24, 2026), the setup code (owner pressing, no motion, a bounded `serve` stopped afterwards). `ota fw45-code-02` installed `feeder` v111 (`e5650894`), confirmed.
  - `fw45-code-05`: six `pair setup keyed=` records, each with a `key` tick, then `state=entered`, `PLSET1 proving`, `proven wifi=1`, `reply done enrolled=already wifi=saved` (the key already enrolled, `result=duplicate`, Wi-Fi generation 21), `coin`, the reboot without Bluetooth, the identity unchanged (`8019237d8eba8b9d`) and a rejoin with `source: stored`.
  - `fw45-code-06`: every tick logged but none heard, while the longer `alert` was. The amplifier settle went from 60 to 150 ms and the tick from 50 to 80 ms; v112 (`bc5561ae`, `fw45-code-07`) confirmed, and with it the ticks were heard.
  - `fw45-code-04` and `-06`: windows that ran out before the sixth press refused `unconfirmed` with `alert`; later presses went to the locked panel (`ask action=feed result=refused`).
  - `fw45-code-09`: a second client connecting while the first's code was pending, with no disconnect seen, got `busy`, and the presses counted towards an offer nobody held: the reason for the stale-offer fix.
  - BlueZ listed the feeder with the setup UUID, but `wellfed setup` missed it twice; `--address` after `bluetoothctl scan le` connected every time.
  - A signed clear then emptied the Wi-Fi store (generation 22).
- **Observed** (October 24), setup with a Dispense press as the check, before the code (v108–v109, owner pressing, no motion). `ota fw45-setup-01` installed v108 (`46255eec`), confirmed.
  - `fw45-setup-02`: Lock and Mic gave `pair setup state=opened`, `PLSET1 start`, `PLBLE1 ready heap_free=104416` (lower than at boot, with Wi-Fi up), advertising and a link at MTU 247. `wellfed setup` found the feeder and finished the handshake (`asked wifi=1`, `reply confirming seconds=60`). After the press: `pair admit path=setup role=owner`, `PLCFG1 keys change=enroll result=done enrolled=1`, `net saved generation=19 stored=1`, `reply done enrolled=new wifi=saved`, `reboot ended=done`. The next boot had no Bluetooth record, kept `enrolled=1`, matched the CLI's fingerprint `8019237d8eba8b9d`, and was online with `source: stored` 23 s in.
  - `fw45-setup-03`: on a locked panel the chord was refused (`refusal=locked`). With no press, the 60 s refusal (`reply refused reason=unconfirmed`) was followed at once by a dropped link, so the CLI never reported it and stalled in BlueZ's disconnect. v109 (`fw45-setup-04`) waits 2 s for the client to hang up and bounds the CLI's disconnect; in `fw45-setup-05` the CLI reported the timeout and setup mode kept advertising.
  - `fw45-battery-01`–`03` (three D cells): cutting the adapter relay gave `PLBAT1 sample adapter=0`, then `advertising on=0`, the feeder still running; restoring it reversed both.
  - With no session, setup mode rebooted at its 15-minute limit (uptime 1,073 s back to 23 s), online with `source: build`. A signed clear emptied the Wi-Fi store (generation 20); the CLI key stayed.
- **Static analysis** (image audits of the setup-mode builds, before the loop's stack grew to 8 KiB): built on the loop's stack, setup mode's state pushed the loop's bound to 8,228 B of its then 6,144 B stack. Made static, the bound was 4,396 B, under the audit's then limit of 4,608 B.
- **Observed** (October 23), a retired `ble` bring-up firmware with an earlier HMAC-signed exchange (`fw34-setup-01`–`03`, OTA v70): with the hotspot down, the feeder advertised about 124 s after boot (`ready heap_free=131072`). The host sent the hotspot's credentials; the UART showed `PLCFG1 net saved generation=16 stored=1`, the link staying up through the NOR write. When the hotspot came up, join attempt 14 connected with source `stored`, after 13 failed attempts on the build's. `bluetoothd` 5.87 segfaulted twice around a filtered discovery, so the client checks the service UUID itself.
- **Observed** (October 23), an earlier speaker build with Bluetooth: the stack came up beside Wi-Fi (`PLBLE1 ready heap_free=…`, `BT INIT success`), and BlueZ saw it advertising. Links settled at MTU 247, data length 251, 2M PHY and a 7.5–15 ms interval. A signed 10 s audio stream played clean, and with Bluetooth running the integration suite's smoke tier passed: OTA install and confirmation, signed requests, refusals, and two settings saves to NOR. That audio demo is removed.
- **Recovered:** the research unit's system-data word at `0x3028`, from the October 22 full dump. The first-boot image with Bluetooth was 832 KB, well within a bank.
- **Modeled** (October 24): `wellfed-session/tests/setup_code.rs` (the proof press by press; a wrong press, a relay between handshakes and a reflected message failing), `wellfed-client/tests/setup.rs` (the client against the responder through the BLE framing; mistyped codes, a `Done` without proof, refusals, tampering and a foreign prologue), `wellfed-fw-feeder/tests/setup.rs` (setup mode in the loop: enroll, Wi-Fi save, `Done`, reboot; a wrong code; ticks, `coin`, `alert` and `sound` off; unanswered asks, leaving clients, foreign messages, mains-only advertising and the idle reboot), `wellfed-hal/tests/ble.rs` (advertising on request, framed messages, oversize and long sessions dropped, radio failure) and `wellfed-device-runtime/tests/network.rs` (a mid-boot save joined with a fresh count).
