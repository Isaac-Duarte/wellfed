# Product architecture

This page records the owner's decisions about what Wellfed is building, and why each beat its alternatives. It is for anyone who wants to know why the firmware, app and server look the way they do. The goal is the feel of the vendor's app, fully open and usable offline.

Most of this is design. What has been built and observed is on the [roadmap](../roadmap.md).

## Decisions

### The firmware

- **One `feeder` firmware, with the server optional.** Separate connected and offline builds would mean proving every feeding guarantee, soak test and update path twice. "Offline" is the connected firmware without its server.
- **The feeder owns its plans, settings, keys and recent history.** Server-owned state would contradict running with no server.
- **Offline means no server and no cloud, but still on Wi-Fi.** Public NTP is allowed.
- **Two kinds of owner.** Some have only the app; self-hosters also run a server.
- **Two of the firmware's four tasks:** the button service, and one loop that runs the network and feeding. A task per client was rejected: each needs its own stack holding Noise state, and four sessions would take every slot the firmware has ([task slots](../../src/firmware/wellfed/README.md#what-the-sdk-runtime-guarantees)).
- **Code layout:** mechanisms in the SDK, product policy in a `wellfed-feeder` crate, and a thin firmware per model. Policy in the SDK would burden other device types, and policy in each firmware would get copied per model.
- **Hardware:** the PLAF103 first, with a device layer generic enough for other AmebaD models from the same vendor ([device layer](device-layer.md)).

### Talking to the feeder

- **The feeder hosts one message stream per client:** typed requests and responses, plus pushed events and samples. REST with an event stream was rejected: an HTTP parser and several connections cost RAM, and encryption would mean TLS. An MQTT client was rejected because a phone can't host a broker.
- **Noise encryption for every client.** HMAC signing alone leaves schedules and keys readable on the LAN. TLS costs mbedTLS RAM, and owners find self-signed certificates hard to trust.
- **Concurrent edits:** each write names the generation it was based on, and a stale write is refused. Last-writer-wins can silently undo someone else's schedule change.
- **Several feeders:** the app manages each one directly. Requiring a server for a second feeder would weaken the no-server promise.

### Setup and more clients

- **Setup over Bluetooth LE, only in a setup mode** the owner asks for with a button hold, ended by a reboot. Always-on BLE would pay heap, tasks and radio time forever. The feeder's own access point would make the phone switch networks. Offering both would double the setup surface.
- **More clients join by an invite from a paired client, or through a button-hold pairing window.** Invites alone leave no way back if every client is lost. The button alone means a walk to the feeder every time.

### Updates

- **The app or server fetches signed releases, and the owner approves each install.** Automatic installs would put a late-misbehaving release on every feeder a pet depends on.
- **Signing:** the project key, plus an owner key enrolled for self-built images.

### Time

- **With no time source, schedules hold, the feeder shows that it needs the time, and manual feeds still work.** Resuming from a saved time shifts meals late and can repeat them. Falling back to interval feeding is a new policy at the wrong times.
- **The clock counts as valid until its error could exceed a bound.** A fixed week is arbitrary, and never expiring lets meals drift with no warning.

### Data

- **The feeder keeps at least 7 days of events.**

### Apps, server and web

- **A new `wellfed-server` crate.** Growing the bench's server (now `wellfed-stock`'s binary) would mix bench shortcuts and the stock protocol into the owners' product.
- **Stock firmware is used only to install and roll back.** Ongoing stock control would mean two device protocols forever.
- **One app in Flutter, over a sans-IO Rust core, for phones and computers.** Android comes first, then macOS and Linux, then iPhone, then Windows ([frontends](#frontends)). It talks to feeders directly, and to a server when one exists. Tauri was rejected: one UI, but mobile BLE and Wi-Fi need native plugins, and the app feels like a web page. Native Swift and Kotlin would mean two or three UIs for one maintainer.
- **No web UI for owners, and the browser never talks to the feeder.** Browsers have no raw TCP, no mDNS, and no Web Bluetooth on iOS or Firefox, and Chrome restricts HTTPS pages from reaching LAN devices. A UI served by the feeder would cost flash and a second listener, and arrive over plain HTTP. A browser UI on the server was dropped on October 30, 2026: it would repeat every app screen, its feeding-safety logic included, and reach only owners who run a server. It would come back only as the way onto iPhones if Apple refuses the app.
- **The app alone is the complete product.** The server is an opt-in add-on for long history, remote access, accounts, pets and food, and firmware releases.
- **The installer** is the app. Every install from stock runs from the app; the server only installs releases on a feeder already running Wellfed. An install on an iPhone is an open question: iOS suspends an app soon after it leaves the screen, and the phone is serving the feeder its firmware.
- **Distribution:** APKs and server builds as GitHub releases. App stores and F-Droid come later. Apple's review of an app that replaces a feeder's firmware is an open question, to settle before any public iPhone release.
- **App and server together.** With a server configured, the app goes through it first, and straight to the feeder on the LAN only when the server is down ([two paths, one feed](../server/README.md#two-paths-one-feed)). App-direct only was rejected: no remote access, and each person's actions would reach the audit log only through the feeder.
- **Alerts** are decided by the feeder, which keeps a small table of what is wrong for every client to read ([alerts](alerts.md)). The Android app notifies them at home, and away from home through the owner's server, which also posts them to a webhook of the owner's ([the app](../../src/app/README.md#alerts), [server alerts](../server/README.md#alerts)); push is [deferred](#deferred).
- **Remote access** goes through a reverse proxy in front of the server, or the owner's VPN ([deferred](#deferred)).
- **Home Assistant reaches the feeder directly**, as its own client, **or through the owner's server**, chosen at setup ([below](#home-assistant)).

## Components

```text
  the app (Flutter) ─────── BLE (setup mode only) ──┐
      │   │                                        ▼
      │   │  Noise message stream (mDNS)       ┌────────┐   NTP
      │   └───────────────────────────────────►│ feeder │◄──────
      │ HTTP                                   └────────┘
      ▼                                            ▲
  wellfed-server (optional) ── Noise stream ──────┤
      history store, release fetching, release installs
      ▲                                            │
      │ HTTP                                       │
  the CLI, Home Assistant (server route)           │
  Home Assistant (direct) ── Noise stream ─────────┘
```

| Part | Where | What it holds |
| --- | --- | --- |
| SDK | `src/sdk/` | Mechanisms only: runtime, clock and time sources, buttons, Noise transport, the BLE link and the updater, device-neutral in `wellfed-core`; a kind's own mechanisms beside it (`wellfed-core-feeder`: the feed controller and its journal; `wellfed-hal-feeder`: the feeding owner and its drivers). It depends on no product crate, and `wellfed-core` names no kind; where the runtime needs the product's stores (the settings writer, stored Wi-Fi) or the kind's logic (the startup's synthetic check), it calls a hook the product's runtime defines |
| Device policy | `src/product/wellfed-device` | `no_std`: what every device kind keeps (settings, Wi-Fi, client, time, event and trace stores, the reset marker, the update view) |
| Device runtime | `src/product/wellfed-device-runtime` | Those stores on the SDK: the settings writer and its mailboxes. The writer is generic over a kind's own stores (`writer::Product`), statically, so a fountain links it with no `wellfed-feeder` or `wellfed-feeder-runtime`. It still links `wellfed-core-feeder` through the device API, whose vocabulary names the feeder's settings and plans |
| Product policy | `src/product/wellfed-feeder` | `no_std`: schedules, child lock, reminders, history policy, the request and entity model, and the feeder's own stores (plans, calibration, meal call, lifetime statistics). Its loop, `Feeder<P: Platform>::pass`, owns the panel, BLE setup mode, the device API's driver and the event recorder. The host tests and the simulator run the same pass |
| Product runtime | `src/product/wellfed-feeder-runtime` | The settings stage over the shared writer and the feeder's stores. Every PLAF103 firmware links it, so a firmware swap keeps the owner's data |
| Firmware | `src/firmware/wellfed/feeder` | The `Platform` (pins, drivers, sockets, mailboxes), stepping the feeding owner before the product's pass. Each model gets its own thin firmware |
| Client core | `wellfed-client` | Bytes in, bytes out: the Noise handshake, pairing, invites, requests, event-cursor sync and the installer's steps. No socket or radio |
| Client drivers | `wellfed-connect`, `wellfed-install` | Device API sessions over tokio TCP for the server, the CLI and the app; the install from stock for the CLI and the app, with stock's server in its own process |
| Home Assistant | `src/integrations/homeassistant` | The integration's Python, and its PyO3 bridge over `wellfed-connect` |
| Server | `wellfed-server` | The owner's server, over `wellfed-client`, `wellfed-model` and `wellfed-connect`. No stock protocol and no signing key: it verifies releases, and an owner signs a return to stock on their own computer |

## Frontends

- **The Android app** (`src/app/`) does platform IO in Dart through maintained, permissively licensed plugins: BLE (`flutter_reactive_ble`, carrying the setup bytes), NSD and notifications, with its background work (WorkManager and one foreground service) in a little Kotlin of its own, since the plugins for those start engines that take over the Bluetooth plugin. Its Rust half, `wellfed-app-bridge`, runs every protocol step through `wellfed-client` and LAN sessions through `wellfed-connect`, over `flutter_rust_bridge`, which also generates the Dart types. An install runs `wellfed-install` on its own thread, asking Dart for what only the phone has. It runs the stock installer through stock's setup access point and the `DL` frame ([consumer installation](consumer-installation.md)). Setup, pairing and the installer always go to the feeder; everything else goes to the server first when there is one.
- **Other platforms** are the same app and Rust, each with its own Bluetooth, discovery, network joining and keep-alive. In order: macOS and Linux, which run the install and can hand a feeder to a phone with an owner's invite; iPhone, for setup and daily use; Windows, once there is a machine to test it on. WASM can run the client core later.
- **The CLI** (`wellfed`) speaks the device API and, under `wellfed server`, the server's HTTP API. It keeps and restores stock's backup but doesn't install from stock, and stays the bench's tool, on Linux.
- **One UI** means one codebase for one maintainer. The server's HTTP API stays public through `openapi.json` for anyone building another client.

## Setup and pairing

**Bluetooth runs only in setup mode** ([Bluetooth](bluetooth.md)): a Lock-then-Mic hold for 5 s on an unlocked panel, or a boot with no network to join. Setup ends in a reboot without BLE, on success or after 15 minutes without a session.

- **Not when Wi-Fi drops.** A moved feeder still has its stored network, so it needs the button hold.
- **At boot only when unprovisioned:** no build credentials and none stored, once the settings writer has loaded the Wi-Fi store and can save to it. Every boot works this out from the stores, so nothing is carried across a reset, and every KM0 backup register the SDK leaves free is taken anyway. An enabled meal plan prevents it, since a setup session refuses every feed, and so does a reset storm's hold, until it ends.
- **Never torn down live.** AmebaD's BT deinit is unverified on the bench and could leak heap.
- **Cost.** A running feeder pays only BLE's flash, the linked stack and an 11.5 KB controller patch. Heap, the C tasks (outside our stack audit) and radio time are spent only in setup mode.

**Consumer first boot.** A consumer image (`PROVISION=consumer`, [firmware builds](../development/firmware-builds.md#bench-and-consumer-images)) has no build credentials and trusts the project release key instead of the bench key. With nothing stored it neither scans nor associates, but keeps the radio on for BLE and looks again every 30 s, so credentials stored over BLE are joined within one interval. It boots into setup mode (`PLSET1 start cause=unprovisioned`) at every boot, from either bank, until the app or `wellfed setup` gives it Wi-Fi ([Bluetooth](bluetooth.md#runtime) has the idle reboot and Dispense between sessions).

**Clients** pair as [pairing](pairing.md) describes. The first is an owner, set up over BLE with a code keyed into the buttons; the installer ends by handing the feeder to that flow, not to a server. Later clients redeem an invite (QR or link) from a paired client, as the server and the CLI do, or use the button-hold window, which is also recovery when every client is lost.

## Device API

The [device API](device-api.md) serves one Noise session per client from one loop task. A firmware gets four tasks, each with its own stack, so the loop polls every socket without blocking and does a bounded amount of work per client each pass before stepping the feeding owner. A flood or a malformed client costs only its turn, and a handshake operation's time bounds the loop's latency ([resource budget](resource-budget.md#handshakes-and-loop-latency)).

**One feeding owner, in the loop.** The button task only queues gestures ([buttons](buttons.md)), so a press and two clients reach the same owner in the same task, and can't double-feed. The owner isn't in its own higher-priority task for a measured reason: a Wi-Fi association holds every task, a priority-4 one included, for 0.45–0.72 s ([resource budget](resource-budget.md#scheduling-gaps-across-wi-fi-rejoins)). The stall is in the vendor driver, below the scheduler, so no priority keeps a feed's 1 s beat. Instead feeds and joins take turns (`wellfed_hal::interlock`), and storage writes take the same turn ([no flash write while the motor may move](device-api.md#no-flash-write-while-the-motor-may-move)).

## Time

Details are in [clock](clock.md). Sources are NTP (the gateway by default, or an owner's address) and syncs from clients over the API; only an owner's may step the clock, the rest only narrow a valid one. The zone is a feeder setting, a POSIX rule evaluated for any year. The clock is valid while its possible error stays under 2 minutes: about 6.6 hours after a fix before its drift is learned, about 7 days after. Warm resets (watchdog, update activation, trial boots) carry it through the always-on RTC; a power cut loses it. Without it, scheduled meals hold, the lights and sound say the feeder needs the time, and Dispense and app feeds still work, until the missed-meal rule takes over.

## Updates

- The app or server fetches signed releases and shows them. The owner approves each install, which reaches the feeder as an offer over the API.
- The feeder never downloads from the internet, so it needs no TLS client.
- A new build runs a trial and confirms itself. Updates ride the device API ([OTA design](ota-design.md#updates-over-the-device-api)): bank 1 gives way, and bank 0 (the feeder build, which also feeds) installs into bank 1.
- Images are signed by the project key or the owner's enrolled key. A downgrade needs a statement bound to the feeder.

## Data

What the feeder can measure decides what anyone can store. On the PLAF103:

| Signal | Source | What we have |
| --- | --- | --- |
| Feed requested, source, result | Feeding owner, journal | Recorded for every feed |
| Delivered portions | Rotor marks on PA18; the outlet beam for food passing | One mark per portion is observed. Grams come from the owner's calibration, and outlet-beam breaks are a lower bound on food that fell |
| Food level (present, low) | Food beam | The status and the low-food light show the latest reading. It warns low, and doesn't measure how much is left |
| Motor current per feed | ADC channel 4 | Read during drives. Scaling to milliamps needs a reference |
| Supply voltage, adapter or battery | ADC channel 5, PA19 | Read and reported |
| Buttons, lock, resets | PB4–PB7 | Read and recorded |
| Connectivity, boots, updates, clock syncs | Runtime | Reported in the status and statistics |

The feeder has no scale, bowl sensor or camera. It knows what it dispensed, never what the pet ate.

Anyone should be able to do anything with their feeder's data, so the server keeps everything raw, and features like battery health are queries, not firmware changes. The feeder sends:

- **Durable events**, in a ring that holds at least 7 days, each with a sequence number: feeds, blocked feeding, food-level and supply changes, lock changes and refused presses, setting and plan changes with the client that made them, boots with their reset flags, clock syncs and enrollments ([the full list](device-api.md#events-and-samples)). A feed's event names the portions asked for and delivered, the outcome and its reason. A client that reconnects reads everything after its cursor, and records already lost to the ring are reported as lost, not skipped.
- **Live samples**: power and the judged supply, Wi-Fi signal, heap, loop timing, the gateway's round trip and the Wi-Fi channel, each at its own rate ([samples](device-api-ops.md#samples)). They go only to subscribers and are never stored on the feeder, so a sample missed during a server outage is gone.
- **Per-feed traces.** Each feed's current, marks, beams, battery and motor commands over the drive, with its drive time and peak and mean current. The feeder keeps them for its last 8 to 12 feeds, read on demand ([feed traces](device-api-diagnostics.md#feed-traces)), since the server is optional.

**On the server:**

- **Raw store.** Every event and sample is kept forever by default. The owner can prune, export everything as CSV or JSON, and import it to move servers.
- **Open schema.** Event and sample types live in `wellfed-proto`, versioned and documented. The feeder links that crate; `wellfed-formats` holds host-side files only.
- **Owner data the feeder doesn't hold:** pet profiles (name, photo, weight log, daily target) and food (brand, kcal per gram, bag size).
- **Derived views:** food remaining and a refill estimate (calibrated grams per portion × portions since the last refill, against the bag size); grams and calories against each pet's target; feeding anomalies (missed, uncertain or short feeds); hardware health (motor current trends, battery curves, Wi-Fi drops, resets and clock drift); and an audit log of who did what.

## Home Assistant

A custom integration reaches each feeder one of two ways, chosen at setup. Directly, it pairs as an ordinary device API client, by an invite, and holds one session to it, with no server. Through the owner's server, it signs in once for a token of its own and follows the feeder over `/api/v1`. Both routes present the same entities under the same unique ids, so an entry can switch routes and keep its history. It needs no code on the feeder. Its Python is Home Assistant's UI and glue only: the sessions run in Rust, in `wellfed-connect`, `wellfed-client` and `wellfed-server-client` through a PyO3 bridge, so Home Assistant, the app and the CLI share one implementation. It lives in [`src/integrations/homeassistant`](../../src/integrations/homeassistant/README.md), the one Python under `src/`.

- **One press, one feed.** Each press goes out once, under a fresh request id or idempotency key, and a feed with no session open isn't sent. A feed whose answer was lost is reported uncertain and never retried. Directly, a later `Fed` event naming Home Assistant as its source settles it; through the server, the server's journal does. With no record of it, it is reported unknown, never as a count.
- **A member, nothing more.** Directly, it joins only by a member invite. An invite doesn't say its role until redeemed, so a key an owner invite enrolled revokes itself at once with the right it was just given, and setup asks for a member invite. Firmware installs, Wi-Fi, restarts and removing clients stay with the owner's phone and computer.
- **The same day as the app.** Last fed, today's meals, the next meal and the portions fed today come from `wellfed_client::today`, the facts the app's Home card reads.
- **Its cost:** directly, one of the feeder's four session slots, held while Home Assistant runs, and one of its 8 client keys. Through the server, nothing on the feeder: the server holds the session.
- **Both routes, not one.** Direct alone was the route chosen on October 30, since most owners won't run a server. The server route was added for the beta, so an owner who does run one keeps the history, the audit log and remote access in one place. Neither route is required.
- **Rejected: the ESPHome native API on the feeder.** It reimplements an unofficial protocol that changes with Home Assistant releases, and spends firmware RAM.
- **Rejected: MQTT discovery through `wellfed-server`.** It adds a broker and a second protocol, and MQTT can deliver a command twice, so a feed would need its own protection against duplicates.
- **Rejected: Matter on the feeder.** Its C++ stack wouldn't fit the firmware's flash and RAM headroom or its task slots, it needs a second BLE commissioning path, and Matter has no feeder device type, so a feed would look like an on/off switch.

## Deferred

- **Push alerts.** Away from home the app hears alerts through the owner's server, but only when its periodic job runs, and a phone with no server hears nothing ([roadmap](../roadmap.md)). Push on iOS and Android goes through Apple and Google services, which need credentials tied to the app's publisher, so a self-hosted server can't send them alone.
- **Remote access.** Only through the owner's server; the owner's VPN is one route.

## What this asks of the SDK

| Work | Adds | Read |
| --- | --- | --- |
| Resource budget | RAM, flash, stack and task budget for the product image, setup-mode BLE and Noise included: the two firmware tasks, the handshake time in the loop, the client limit and buffers | [Resource budget](resource-budget.md) |
| Client keys and pairing | The random generator, per-client keys, BLE setup mode, invites, the button-hold window, revocation, owner signing-key enrollment | [Pairing](pairing.md) |
| Offline time | NTP, drift learning, the error-bound validity, time across warm resets, the no-source hold | [Clock](clock.md) |
| Device-hosted API | The Noise message stream, mDNS, generation-checked writes, the event cursor and samples, several clients served by one loop task | [Device API](device-api.md) |
| Updates over the device API | Offers from any paired client, the project key and an owner key | [OTA design](ota-design.md) |
| Device layer for other models | An audit of the HAL and `device.json` for PLAF103 assumptions | [Device layer](device-layer.md) |

## Evidence

- **Design:** the decisions above are the owner's, recorded on October 23, 2026, with the app-and-server path decided on October 26, the "not automatic" setup rule on October 24, the frontends, the dropped web UI, notifications and the Home Assistant route on October 30, that the server is not an installer on October 31, and Home Assistant's direct pairing on November 3.
- **Observed:** a Wi-Fi association held every task, a priority-4 one included, for 0.45–0.72 s on October 25 ([resource budget](resource-budget.md#scheduling-gaps-across-wi-fi-rejoins)).
- **Observed:** PA18 rests HIGH and reads LOW for about 1.17 s at each rotor mark, one mark per portion. During a 3 s empty drive, channel 4 read a scaled maximum of 166 and a mean of 64 ([motor pulse](motor-pulse.md)).
- **Observed:** the task-slot limit was found in another session's captures (`fw44-simon-01`).
- **Modeled** (October 29): setup mode at an unprovisioned boot, starting only when it should and pairing over a simulated link (`wellfed-fw-feeder/tests/setup.rs`, `wellfed-feeder-sim/tests/setup.rs`). Not yet on hardware.
- **Observed** (October 30): a consumer image on the research unit started setup mode by itself at boot, from bank 0, and both the app and the CLI paired through it and gave it Wi-Fi ([consumer installation](consumer-installation.md#evidence)). Setup mode's heap has been measured from bank 1 only.
