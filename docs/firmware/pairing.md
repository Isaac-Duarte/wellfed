# Keys, randomness and pairing

This page explains how the feeder decides who may talk to it: its random generator, identity key, trusted clients and the ways in. It is for owners and contributors. Bluetooth setup is on [its own page](bluetooth.md).

## The one idea

Every client has its own key, and the feeder trusts a short list of them. Getting on the list takes someone at the feeder or a client already on it: the first owner keys a code into the buttons, and later clients need an invite or a button hold. The network alone can't make a stranger an owner. The keys must be unpredictable, and the chip has no vetted random generator, so the firmware builds one from measured noise.

## No hardware RNG

The pinned SDK exposes no hardware random generator (static analysis). `rtw_get_random_bytes` calls `arc4random`, which on the 8721D is ROM `Rand()`, a Tausworthe generator whose four words start at the constant 12345 (`swlib/string/rand.c`). Mixing that with SysTick and the tick count, as the firmware's first generator did, rests on timer values alone: enough for a value that need only differ between sessions, not for a long-term key.

**What replaces it** (`wellfed-core::rng`):

1. **Raw noise** (`wellfed-hal::entropy`): the low byte of a raw ADC conversion of the chip's internal supply, and SysTick's cycle count between 32 kHz timer edges ([measured](#measured-noise)).
2. **Health tests** (SP 800-90B 4.4): a Repetition Count Test and an Adaptive Proportion Test (window 512) on every sample, with cutoffs from the claimed min-entropy at a false-alarm rate of 2^-20, as the standard's table gives. No output until 1,024 samples have passed; a failure latches until reboot.
3. **Conditioning:** SHA-256 over tested samples. An input is ready once twice its 256 bits have been claimed.
4. **HMAC-DRBG** (SP 800-90A, SHA-256), checked against NIST's CAVP vector, demanding a reseed after 65,536 requests.
5. **A seed across boots** (`store::seed`, sectors 7–8). Each boot instantiates the DRBG from fresh pool input with the stored seed as its nonce, and stores the next seed before anything draws. So a boot whose source repeats an earlier input never repeats an earlier stream.

## Measured noise

A million one-byte samples per source, streamed over the UART from a cold boot, went through NIST's SP 800-90B tool (`ea_non_iid`, non-IID track, 8-bit symbols). Its conservative estimators rate 1,000,000 bytes of `/dev/urandom` at 6.92 bits per sample.

| Source | Samples assessed | H_original | 8 × H_bitstring | Min-entropy per sample | On-device health tests at 1 bit |
| --- | ---: | ---: | ---: | ---: | --- |
| Clock jitter: SysTick cycles between 32 kHz edges, low byte | 999,712 | 0.189 | 0.310 | **0.19** | passed |
| ADC internal supply channel (10), low byte of a raw conversion | 972,272 | 2.050 | 1.441 | **1.44** | passed |

- **Jitter is weak:** three values make 90 % of samples (0x6f 36 %, 0x39 36 %, 0x70 17 %). The CPU clock and 32 kHz timer are nearly locked, so the count varies mainly with interrupts. The ADC spreads over 92 values, the most common at 22 %.
- **The health tests** passed both at a 1-bit claim: they catch stuck or broken sources, not an over-claim, as jitter passing with a fifth of that shows.

**So each ADC sample is credited 1 bit** (8 eighths), below the measured 1.44: 512 samples per 256-bit input, after the 1,024-sample start-up test. Jitter is hashed in, credited nothing.

## The generator in the firmware

- **At boot,** `wellfed-hal::rng` starts in the runtime's startup task, before anything else uses the ADC. It fills the pool from 1,024 ADC samples (the start-up test, which also covers the claim), closes the ADC, and records `PLRNG1 boot result=ready samples=1024 first=<16 raw samples> check=<hash of one discarded block>`.
- **After boot** the power monitor owns the ADC, so the generator never reseeds. The reseed interval is far beyond a boot's needs, and every boot starts fresh.
- **Drawing.** Every task draws from the one DRBG with the scheduler paused. BLE setup's Noise ephemeral keys come from it alone; no session starts without it.
- **The settings writer,** once armed, mixes the stored seed in and stores the next, opens the client store, and creates the identity secret on the first boot with a generator: `PLCFG1 keys seed=… mixed=… saved=… clients=… enrolled=… identity=…`. Its task's stack bound is 2,948 of 4,096 bytes, since the client store decodes only the winning slot's table and saves a table by reference.

## Keys and the panel

**Between tasks** (`wellfed-device-runtime::keys`), only the settings writer writes the client store, and only the loop reads the panel and runs handshakes. The loop queues one change (enroll, revoke or reset) in a one-slot mailbox; a second before the writer takes it is `busy`. Results log as `PLCFG1 keys change=… result=… enrolled=…`. The [factory reset](#factory-reset) needs the generator (else `result=no-generator`); nothing weaker is written.

**The identity.** The writer publishes the secret in RAM with a per-boot generation. The loop derives the public key (one X25519 on the Montgomery ladder), keeps the pair for handshakes, and logs `PLID1 identity generation=N fingerprint=<the public key's first 8 bytes>`. No record carries the secret.

**Gestures.** Pairing sees each gesture before the panel, which gets only what pairing leaves. For recovery, press Lock, then add Dispense, and hold both: Dispense first would be a feed, and Lock's own 5 s hold toggles the lock only when Lock is alone. A Reset hold (about 5 s) on an unlocked panel starts the factory reset; a locked panel refuses it.

## Factory reset

The Reset hold restores a new owner's state. Stock's own restore was never decompiled, so the owner decided what it clears (October 28, 2026).

| Cleared | Kept |
| --- | --- |
| Paired clients and the identity | The feed journal: its high water and any uncertain feed's record, so a feed is never re-run and feed numbers never go back |
| Wi-Fi credentials | The portion calibration, a property of the hardware |
| Settings (lights, sound, volume, their windows, and what later joins the record) | The update store and the boot confirmation: the anti-rollback floor and the build in trial |
| Time settings: the zone and the NTP server | The generator's seed |
| Feeding plans, their marks and last claim | The flash erase counts by sector and of bank 1, the hardware's wear (owner, October 28) |
| The event history and the per-feed traces | |
| The meal call's clip | |
| The uplink store's reserved sectors, and any store a later build adds | |
| An update store the build can't read, if you held Reset on the panel | |
| The rest of the lifetime statistics: uptime, feeds, portions, presses, sessions, link figures and boots by cause | |

`wellfed_board::storage::FACTORY_KEPT` lists what the reset keeps. It erases every other sector of the region, the clients first, then in region order. Whichever bank finishes a reset clears by its own build's list, and bank 0 keeps one build for the life of the feeder. So a store a later build adds is cleared by default, even by an older bank 0. The statistics are rewritten, not erased, in one atomic save holding only the erase counts (`Totals::wear_only`).

**How it runs.** It erases 36 sectors and rewrites one store, surviving a power cut at any point:

1. **The hold** is refused while a feed is queued or runs (`PLPANEL1 reset refused=feeding`) or an install (`reset refused=updating`), and otherwise queued (`reset keys=queued`). Until the restart an install offer is refused `busy`, so the reset's boot is never a trial boot.
2. **The marker.** Under its write turn, so after a feed's outputs stop, the writer programs a 16-byte word saying you asked at the panel, then a 16-byte marker in sector 36, committing the reset. It forgets every client, ends every session, and answers `done` even if the client store can't be written, since the next boot erases it (`PLCFG1 keys change=reset result=done`).
3. **The restart** (`reset result=done`) comes once no feed is queued or running and no claimed meal waits, so a claimed meal is never missed. From then no feed starts ([readiness](device-api.md#feeds) `Restarting`).
4. **The erases.** The boot that finds the marker logs `PLCFG1 reset pending` and opens nothing the reset clears. Once armed, it erases each cleared sector under its own write turn and attempt marker and reads it back blank. If the update store is one this build can't read and the word says you asked at the panel, it erases that too (`reset update=cleared`). It then saves the wear-only statistics, leaving a newer major's alone and erasing a pair this build can't read. That pair is the region's most written, so a failed save erases it instead, losing only the erase counts (`reset wear_lost=1`); only a pair that takes neither stops the reset. The marker is erased last (`reset done erased=36`). Events recorded meanwhile wait in the queue and land in the new ring.
5. **The fresh boot** opens empty stores, draws a new identity, and starts the event ring under a new epoch.

**Power cuts.**

- Before the marker is whole: no reset, nothing erased; hold Reset again.
- After it: the next boot finishes the reset however far it got. A torn erase is erased again. A cut statistics save leaves the old or wear-only totals, and a cut erase of a worn pair leaves part of it, both of which the next boot redoes.
- A torn marker erase reads as the marker (the erases run again) or as no reset (they are done).
- The Wi-Fi task reads stored credentials only with no marker down, so an unfinished reset never joins the old network.
- A boot that can't erase (unarmed, or an erase that failed or left bits) logs `reset error stage=… restart=…`, keeps the marker, and serves nothing: no identity, stores, API, recovery window or BLE setup.

**The escape.** Such a boot may ask for one restart (`restart=1`), once no feed or claimed meal is pending. Reset flags can't tell a hang from a deliberate restart (both are watchdog resets here), so each armed boot programs an attempt word after the marker, with no erase, before its first erase. A restart is asked only while fewer than two are down, and only after a failed erase or an arming skip (the boot before hung inside a write, and the skip cleared its attempt marker). So the hold's boot and one escape restart each get a try; after both (`restart=0`), or another arming failure, the boot waits for a power cycle. The attempts are erased with the marker.

**After the reset,** the feeder joins with the build's credentials, if any, until BLE setup provisions new ones; setup and LAN recovery admit a new owner. Event numbers restart at 1 under a new epoch, so a client sees a new history, not missing events ([device API](device-api.md#events-and-samples)). With a new identity and no old clients, the server sees a new feeder.

## Storage sectors

One table names every sector, `wellfed_board::storage` (the PLAF103 board's), checked at compile time to be disjoint and inside the 48-sector region, 0x340000–0x370000 ([resource budget](resource-budget.md#the-storage-region)).

| Sectors | Owner |
| --- | --- |
| 0–1 | Feed journal |
| 2–3 | Settings |
| 4 | Boot confirmation |
| 5–6 | Wi-Fi |
| 7–8 | RNG seed |
| 9–10 | Identity and clients |
| 11–12 | Feeding plans ([schedules](schedules.md#storage)) |
| 13–24 | 7-day event ring ([device API](device-api.md#events-and-samples)) |
| 25–26 | Update store: the anti-rollback floor, the give-way's candidate, update keys, bank 0's image ([OTA design](ota-design.md#keys-and-anti-rollback)) |
| 27–28 | Time settings: the zone's rule and the NTP server ([clock](clock.md#time-settings)) |
| 29–31 | Per-feed trace ring: the last 8 to 12 feeds' traces ([feed traces](device-api-diagnostics.md#feed-traces)) |
| 32–33 | Lifetime statistics ([statistics](device-api-diagnostics.md#statistics)) |
| 34–35 | Portion calibration ([calibration](calibration.md#storage)) |
| 36 | The factory reset's marker, or the [handover mark](consumer-installation.md#rollback) stock's activation leaves |
| 37–39 | Reserved for the uplink store, the owner's server link. No build writes it yet; the factory reset and the handover from stock clear it |
| 40–47 | The meal call's clip ([schedules](schedules.md#meal-call)) |

### Record versions

Every store but the event ring and the meal call keeps one record in each of two sectors, written in turn ([`wellfed_core::store::slot`](../../src/sdk/wellfed-core/src/store/slot.rs)). Its header holds the store's magic, a major and a minor version, and the record's length.

Bank 0 never updates over the air. The build the installer writes there reads these records for the life of the feeder, so a later build may change a record only in ways that build can follow:

- **A newer minor** only adds fields, in spare room the store reserves or past the record's end. An older build of the same major reads the fields it knows, sees the rest as zero, and keeps those bytes when it rewrites the record. The settings and the feed journal read their sectors whole, so their records keep one length and grow only into their spare room.
- **A new field reads zero as "none"**, the value an older minor leaves there.
- **A field an older build's rewrite would make wrong** isn't a minor's to add. A per-plan or per-client field would outlive the plan or client it named once an older build moves entries around. The plans store ties any such field to the table revision it was written at, which every change bumps.
- **A newer major** makes the store read-only. So a change that isn't additive is a new store, in new sectors.

A store never writes over a record of its own it can't read, with three exceptions. Settings starts over on an older major, since defaults are safe. The seed is only a nonce. The Wi-Fi credentials, the time settings and the calibration are written only by your own change, which replaces what was there. The clients, the update store, the plans and the statistics open as corrupt and stay untouched until a factory reset erases them. So a build never draws a new identity over a client table it can't read.

| Store | Major | Size | Spare room |
| --- | --- | --- | --- |
| Feed journal | 1 | 128 B | bytes 48–120, of which 48–72 are kept for a durable request id |
| Settings | 4 | 128 B | bytes 52–120 |
| Clients | 1 | 584 B | past the record's end |
| Update store | 2 | 384 B | bytes 336–376 |
| Plans | 4 | 256 B | bytes 120–248, for per-plan grams and more plans |
| Lifetime statistics | 2 | 320 B | bytes 280–312 |

## Clients

`store::clients` keeps the X25519 identity secret and up to 8 clients in one 584-byte record, written whole to the other of two sectors, so an enrollment, revocation or reset lands entirely or not at all (power-cut tested at every flash mutation). A client is its 32-byte static key, a label of up to 16 bytes, a role and the generation that enrolled it.

The roles are owner, member, server and update signer ([what each may do](device-api.md#writes-roles-and-generations)). An update signer is an owner's Ed25519 update-signing key, enrolled with `EnrollUpdateKey` and a Dispense press ([OTA design](ota-design.md#keys-and-anti-rollback)), recorded as `Enrolled { path: Press }`. It never opens a session.

> [!NOTE]
> The identity secret and the RNG seed sit in plain flash. The board has no secure element, so anyone who reads the flash chip can impersonate the feeder until its next reset. The seed matters less, since every boot mixes fresh input in.

## Pairing

`wellfed_device_api::pairing` decides who may become a client, returning an `Admit` event the firmware applies to the client store. There are four ways in:

| Path | Who starts it | Confirmed by | Role |
| --- | --- | --- | --- |
| Setup | The app over BLE in setup mode, which a Lock and Mic hold on an unlocked panel starts | The app's one-time code keyed in on the buttons within 60 s, then proven by the app ([why](bluetooth.md#why-a-code)) | Owner |
| Signer | An owner over the API | A Dispense press within 60 s | Update signer |
| Invite | A paired client: an owner invites any role but a signer, a member invites members | The invite's secret as the PSK of a Noise XXpsk3 handshake, within 10 min, once | As invited |
| Recovery | Lock and Dispense held together for 5 s on an unlocked panel | Within 2 min, a client offers its key over the device API and proves the code the owner keys in; the first offer closes the window | Owner |

- **Presses that answer a pairing feed nothing.** A pending signer confirmation takes the next Dispense press. A pending code takes every gesture until complete, even on a locked panel, since someone at the feeder started it. Records count presses (`pair setup keyed=N`, `pair recovery keyed=N`) and never name them.
- **A locked panel** can't open recovery.
- **Invites:** at most two outstanding, in RAM, so a reboot cancels them. The invite's id travels in the first handshake message's payload, which isn't secret, because the responder needs it to pick the PSK before the third message.
- **Recovery over the LAN** runs BLE's setup exchange, code proof included, over the device API's stream (`Pairing::recovery`). No plain XX handshake is admitted, so a LAN neighbour can't slip in.
- **Interop:** `noise.rs` has XXpsk3, checked byte for byte against `snow` in both roles. A wrong or missing PSK fails closed.

Stock pairs through the vendor cloud, so these timeouts are ours ([stock behavior](../devices/plaf103/stock/README.md)).

## The pieces of BLE setup

The exchange and its refusals are in [Bluetooth](bluetooth.md#setup-exchange). In code:

| Piece | What it does |
| --- | --- |
| `wellfed-proto::setup` | The request and replies (postcard), shared by the feeder and every client |
| `wellfed-session::setup_code` | The code and its press-by-press proof, bound to the handshake hash |
| `wellfed_device_api::setup::Responder` | The feeder's side of Noise XX: the client's checked offer (key and label), the proof, and the Wi-Fi from the last step, checked against the Wi-Fi store's rules |
| `wellfed-client::setup::Session` | The client's side, with BLE framing, shared by `wellfed setup` and the Android app |
| `wellfed_feeder::feeder::setup` | The loop's setup mode, over the firmware's `Platform` |

The loop starts the stack on the gesture or at a boot with no network to join ([why](product-architecture.md#setup-and-pairing)), and offers the key to pairing (`Pairing::setup`), which collects the code and runs the proof. It then queues the enrollment (`keys::ticket`, `keys::outcome`) and the Wi-Fi (`network::save`). A client that leaves before the proof cancels its offer (`Pairing::cancel`), so the next press feeds again. A reboot ends setup.

## What's left

- **Not yet on hardware:** a deliberately mistyped setup code, and the stale-offer fix ([Bluetooth](bluetooth.md#evidence)).
- **Optional confidence:** SP 800-90B's full restart test (about 1,000 cold boots), and the noise on battery and when warm.

## Evidence

- **Observed** (October 24, 2026), the noise: the `feeder` firmware's `entropy` variant (v103) streamed `PLRNG1` records from a cold boot, 1,000,048 samples per source, on the research unit on USB power at room temperature (`fw45-rng-05` to `fw45-rng-11`). Segments end at the flash tool's 512 KiB observe limit, so a few records fell between them. The NIST tool was built from its public repository and stays outside this repository.
- **Observed** (October 24), the generator, which runs in every firmware from boot: `ota fw45-keys-02` (v106) logged `seed=empty saved=1 identity=created`. Across 16 cold boots (`fw45-restart-01`–`16`) every boot started it, the raw first samples and output checks all differed, and each logged `seed=loaded mixed=1 saved=1 clients=loaded identity=loaded`, so the identity survived. A sanity check that no boot repeats another, **not** SP 800-90B's restart test.
- **Observed** (October 24), keys and the panel (`feeder` v107, owner pressing, no motion): `fw45-pair-01` and `-02` kept one fingerprint across a cold boot. In `fw45-pair-03` a locked panel refused the recovery chord (`refusal=locked`), and unlocked it opened the window with no feed or lock change. In `fw45-pair-04` the window closed after 2 min, a locked Reset hold was refused, and an unlocked one logged `result=done enrolled=0` and a new fingerprint, which `fw45-pair-05` kept across a cold boot. The loop's stack bound was then 3,912 of 6,144 bytes.
- **Observed** (October 24): BLE setup, LAN recovery with the code, an invite and a revocation ([Bluetooth](bluetooth.md#evidence), [device API](device-api.md#evidence)).
- **Not yet covered:** battery power and temperature; the full restart test.
- **Modeled:** invite, signer and recovery paths, a wrong recovery code, and the client store under power cuts.
- **Modeled** (October 28), the factory reset: the decided stores cleared and the rest kept byte for byte, with a power cut after every erase and the marker's (`wellfed-feeder-runtime/tests/writer.rs`); the marker never pending after a cut program (`wellfed-device/tests/reset_marker.rs`); a hold refused during a feed on both platforms; and a full pair, feed, reset and re-pair in the simulator (`wellfed-feeder-sim/tests/serve.rs`). **Not yet on hardware:** the hold with the clearing and the restart.
