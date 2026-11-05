# Consumer installation and rollback

This page explains how a feeder on **stock 1.1.15** can move to Wellfed with nothing but an installer app, and how it can go back. It is for owners wondering what installing involves, and for contributors working on the installer. The goal is an experience like the vendor app's, with no cloud and no account.

No vendor code, credentials or device identifiers appear here. The evidence index for the static findings is private: `private/research/stock-installer/evidence.md`.

## The one idea

Stock's own updater does the hard part. Stock 1.1.15 will install any image we serve it into its other bank, with no UART and no soldering. So the installer only has to get stock talking to it, ask for one update, and serve the image. You hold Reset, which erases stock's settings and meals, and your phone hands the feeder your Wi-Fi over the feeder's own setup network. Before Wellfed overwrites stock's last copy, the installer saves the feeder's own stock image, so you can always go back.

Where each of these stands is on the [roadmap](../roadmap.md). Going back works from a stock image we serve as well as from the feeder's own copy, and a restore was checked with a vendor app setup afterward.

## What already works on the bench

The Android app runs the whole move from stock to Wellfed, and the trip back to stock works too ([Evidence](#evidence)). The image contract is in [stock OTA requirements](../devices/plaf103/stock/ota-requirements.md).

What made those runs work, and must stay in any installer:

| Requirement | Why |
| --- | --- |
| One `OTA_UPGRADE`, never resent | A refused precheck can erase stock's OTA status sector, and an uncertain request may still be downloading |
| Resolve a failed request before the next one | The server's ledger refuses a new trial while one is failed or uncertain. It resolves from fresh evidence: 20 min, 10 quiet connected minutes, a fresh version report |
| The image port reachable from the feeder | With only TCP 1883 allowed, stock connected for about 25 s, then reported `OTA_INFORM` error 2003 |
| Tolerate stock's ~61 s heartbeat | A 60 s telemetry bound restarted the quiet interval at every heartbeat |
| Ask stock for its attributes when its version report is stale | Idle stock reports its version only around a connect |
| A URL under 128 bytes, `Content-Length` title-cased, a direct `200` | Stock's parser limits ([requirements](../devices/plaf103/stock/ota-requirements.md#http-server-and-url-requirements)) |
| Mains power, idle, model and version checked first | Stock's own precheck; the server's trial gate checks the same reports |

What the bench still uses, and a consumer must not need: a build carrying the owner's Wi-Fi credentials and update key. Generic images remove it.

## Stock's setup protocol

This is the app's main route. The vendor app provisions a PLAF103 through the feeder's **setup access point**, not over Bluetooth:

1. The feeder, in setup mode, runs its own open Wi-Fi access point, `PL-AF103-` and the last two bytes of its MAC, on channel 11 (observed).
2. The app joins it and sends one UDP datagram to **192.168.1.1:17484** (`0x444C`). Stock's `Ap_Config_Udp_Thread` receives it on that port.
3. BLE service `ffee` is connected alongside (`ffe1` write, `ffe2` data, `ffe3` read, `ffe4` notify), but the frame itself goes over UDP.

**The frame.** PLAF103 is on the app's list of products sent without encryption, so the frame is plaintext with no checksum:

```text
'D' 'L' | type=3 | checksum flag=0 | 0 | payload length | lp(SSID) lp(password) lp("host:PPPP") lp(numeric id) | 0xFF
```

- **`lp(x)`** is one length byte, then the UTF-8 bytes.
- **Types:** 1 is Wi-Fi only; 2 server only; 3 both (what the app sends for PLAF103); 4 three other strings of up to 50 bytes, meaning unknown.
- **Field limits:** SSID up to 32 bytes; password up to 64; `host:PPPP` up to 100; numeric id up to 30, ASCII digits only.
- **`host:PPPP`** is the MQTT broker, which the app reads from its cloud configuration. Stock takes the host before the `:` and up to four port digits after it, so 1883 fits.
- **The numeric id** is most likely the account's user id. It must be digits, and nonzero.

**What stock does with it** (stock functions at runtime addresses):

- **Parsing:** `0e01a160` checks the `DL` prefix and the `0xFF` terminator, then dispatches on the type.
- **Wi-Fi:** stored by `0e007bb0`.
- **Server:** `0e007cf8` (`Set_Dev_Info`) stores the port, numeric id and host in a 0x484-byte record.
- **Applying it:** the setup thread `0e0084d8` sets the MQTT port, host and id. It then polls whether MQTT came up (`0e012780`, up to 20 tries). Only then does it persist the record to flash `0x3e8000`, and a 0x60-byte record to `0x3e9000`.
- **At every boot:** stock loads `0x3e8000` and connects to that host and port.

**Saving it takes an answer.** The setup thread's wait is for the state stock also requires before an install, and that state clears when `BINDING` is answered. With no answer, stock joined the Wi-Fi and reached the broker, then refused the install with error 1204 ("device not bound") and, about 20 s after it arrived, went back to its setup network with nothing saved (observed). So the broker must be listening before the frame goes, and reachable within that time. An installer's server has no captured reply, so it answers with `code` 0, a nonzero `bindId` and an empty `cameraAuthInfo`. Stock accepts that, saves the frame, and takes an install (observed). What the vendor's `bindId` means is not established.

**Re-provisioning guard.** While the `0x3e9000` record is non-blank and the device state is bound, types 1–3 are acknowledged but not applied. A feeder must be reset or unbound first.

**Credentials.** The MQTT client ID, username and password the feeder presents are its own: they are loaded from flash at boot, not issued at bind. `BINDING` is an application-level association, which `wellfed-stock` already answers by replaying the captured cloud reply. The commands stock dispatches over MQTT are `OTA_UPGRADE`, `GET_CONFIG`, `BINDING`, `UNBIND`, `DEVICE_REBOOT`, `WIFI_RECONNECT_SERVICE`, `WIFI_CHANGE_SERVICE`, `ATTR_SET_SERVICE` and `ATTR_GET_SERVICE` ([stock behavior](../devices/plaf103/stock/README.md)).

**Getting into setup mode** costs stock's plans. A set-up stock never opens its setup access point on its own: when it cannot reach its Wi-Fi, it retries forever. The only way in is the Reset hold: Reset (the rightmost button) held about 5 s on an unlocked panel. That runs stock's full factory restore. It erases its broker, Wi-Fi and binding records, its feeding plans and their record rings, then restarts ([what the restore clears](../devices/plaf103/stock/README.md#reset-and-the-cloud)). The feeder then says it is ready for a Wi-Fi connection and opens its setup network (observed). The plan sectors read back erased (observed). The panel is unlocked by holding Lock about 5 s while the lock light is on.

**On a phone.** The app asks Android for a network whose name starts with `PL-AF103-`, and Android shows its own picker. Once joined, the process's sockets go over that network until the frame is out. A phone that can hold a local-only network alongside its Wi-Fi stays on both, as the research unit's Pixel did (observed). One that cannot leaves home for those seconds and has to be back within stock's wait, which is modeled but not run. It comes back at the address the frame named, or the run stops: the feeder would look in the wrong place. The password goes unencrypted over the setup network, so the screen says so. A phone that set the feeder up in the vendor's app can keep that network saved, and on the research unit's Pixel Android's picker then found nothing, though the feeder was broadcasting, until the saved network was forgotten in Wi-Fi settings (observed once). The screen says so when a join fails.

**If the feeder does not arrive.** Two minutes after the frame with no feeder at the broker, the run stops with nothing asked of stock. A wrong password or a Wi-Fi with no 2.4 GHz band ends this way. Stock then keeps nothing and reopens its setup network, so a new frame needs no second Reset hold (static analysis, and seen for an unanswered `BINDING`; a retry after a wrong password also ran once from the app on October 30, 2026, with no second hold recorded). That is a new frame to a feeder visibly still in setup mode, unlike the install request, which is never resent. Once stock has reached the broker it has kept the frame and has no setup network left, so a later run only waits for it.

**Still open:**
- What exactly is the numeric id? Stock checks only its form, and repeats it in `BINDING`.
- What does `UNBIND` (`0e015fd4`, not decompiled) clear?
- Are the credentials fixed at the factory, or derived from the serial? Either way they arrive at our broker unchanged, and they survive the Reset hold (observed).

## What the installer's server must answer

`wellfed-stock` already implements the stock side:

- a broker on TCP 1883 that accepts stock's legacy MQTT 3.1 CONNECT;
- the `dl/PLAF103/<device-id>/…/post` and `/sub` topics;
- replies to `NTP` and to `BINDING` (a captured reply if there is one, otherwise a generic one, which a stock fresh from the setup frame needs before it saves the frame or takes an install), and to `GET_FEEDING_PLAN_EVENT` only with a captured plan snapshot. An installer's config has none, so that request goes unanswered: stock erases its plan table on any reply with a `plans` key, and a stock reached with no Reset hold may still hold plans ([stock server](../development/stock-server.md#an-installers-server-trust-on-first-use));
- acknowledgements of the startup, attribute, log and error events.

For a consumer it must also accept a device whose credentials it has never seen. **Trust on first use:** an installer's config (`wellfed-stock init-installer --device plaf103`) names no device. `serve --enroll-minutes N` admits the first stock-shaped CONNECT within the window. It records that client's ID, username, password and address beside the config (`installer.json`'s record is `installer.device.json`) and pins it, so nobody else logs in, then or later ([stock server](../development/stock-server.md#an-installers-server-trust-on-first-use)). Stock's client ID is its device ID.

## The install flow

1. **Reset:** you hold Reset, and the feeder opens its setup access point (which erases its plans).
2. **Frame:** the installer sends one type-3 frame there with your home network and its own `host:1883`.
3. **Connect:** stock joins your network and connects to the installer's broker with its own credentials, which the installer records.
4. **Pre-flight:** model PLAF103, version 1.1.15 on an allow-list, mains power, idle. These are stock's own precheck conditions; the bench's `stock-ready` stage checks them.
5. **Install:** the installer serves the image and sends one `OTA_UPGRADE`. Stock's progress reports drive a progress bar, for about 40 s.
6. **First boot:** the build boots from bank 0 and answers the device API while it waits for an update. A consumer image has no network yet, so it starts BLE setup mode: the installer pairs and sends your home Wi-Fi, and the build reboots and joins it.
7. **Pair:** the next steps go over the device API, which needs a client key. With a consumer image, the Bluetooth setup in step 6 is that pairing: the installer becomes the first owner. A bench image joins its build network instead, knows no client key, and pairs through the LAN recovery window.
8. **Backup:** the build streams bank 1, which still holds stock, to the installer, which verifies it ([rollback](#rollback)).
9. **Promote:** only once a verified backup is stored, the installer offers the same build, signed, over the device API. A consumer image trusts only the release key and keys its owner enrolled. The app's run is given a signed manifest, so it enrolls nothing. Bank 0 installs it into bank 1, and its trial boot confirms.
10. **Hand-off:** the confirmed build enters the [pairing](pairing.md) flow. Your app is its first client, and a server joins later by invite if you run one.

Steps 1–3 are how stock finds the installer: no redirect and no captured credentials. The replacement starts with no meals: the installer does not read stock's plans, and you set meals up in Wellfed afterwards.

After the setup frame, stock's clock is set whatever it reads, since the Reset hold has erased its plans and no meal can come due in the jump. Stock with its clock two days behind refused an install with error 1214 (observed once; the clock as the cause is likely). A run that stops there names the error.

## The installer

One run of the app goes from stock to a confirmed replacement. It is built in three layers:

- **The steps** are a sans-IO state machine in [`wellfed_client::install`](../../src/client/wellfed-client/src/install.rs). Every decision between steps is made there.
- **The driver** is [`wellfed-install`](../../src/client/wellfed-install), a tokio crate that does the IO each step asks for: stock's server, the request, pairing, the backup, the promotion, and the record a resumed run reads.
- **The platform** supplies what only it owns, through the driver's traits: the person at the feeder (prompts), the Bluetooth radio, finding a feeder on the network, the client key, and how progress is shown. The app uses its screens, Android's Bluetooth and NSD, and its secure storage.

Stock's side runs in the driver's own process, since an app cannot spawn the `wellfed-stock` binary. The broker, the server's session with stock and the image server are tasks the run starts and stops ([stock server](../development/stock-server.md#how-the-crate-is-laid-out)).

| Step | Decided in `wellfed-client` | IO in `wellfed-install` and the app |
| --- | --- | --- |
| 1. Provision | The type-3 frame, refused before anything is sent if stock would misread a field; sent once | The owner is told the Reset hold erases stock's plans and puts stock into setup mode. The phone goes to its access point ([on a phone](#stocks-setup-protocol)) and sends one UDP datagram to 192.168.1.1:17484, then comes back to the home network |
| 2. Stock ready | Online, 1.1.15 (any other version ends the run), on its adapter | Stock's server in this process, with the trial enabled and an enrollment window of 15 minutes: the broker trusts the first feeder, and holds that feeder's CONNACK until the server has subscribed, so stock's startup reports reach it. Its status, polled. If stock still reports no version after 90 s, the owner restarts it once to replay them |
| 3. Confirm the feeder | The owner checks the enrolled device ID and address before anything is sent; a contested enrollment ends the run | The status document's `enrollment` (never its credentials) and a screen that asks |
| 4. Request | Exactly one `OTA_UPGRADE` per run; refused or uncertain ends the run | The image server in this process, serving the review to the enrolled feeder's address, then the trial request. What the server recorded is read back: refused only when the server refused and a successful ledger read shows no record, uncertain otherwise (an unanswered request included). The server itself refuses while the enrollment is contested |
| 5. Install | Progress; stock's refusal ends the run, even before the request's outcome is read; stock going quiet means it restarted | The trial phase, and the OTA reports since the request that name it (or nothing) |
| 6. Pair | The installer must be an owner before anything else. The route follows the review's provisioning (`PairRoute`): a consumer image by Bluetooth setup, then a wait for its join; a bench image by the recovery window | A key the build already knows opens a session at once. A run whose broker enrolled the same stock device as an earlier run reads that run's record (`installing.json` beside the backups): it looks for the feeder that run paired with where it last answered, where stock connected from and over NSD. Otherwise a known feeder is tried only where stock connected from, since the app may know another feeder, and setup follows if it does not answer. The run that finishes removes its record. A consumer image: the setup exchange over Bluetooth with the home network, the code shown to key in; then the feeder is looked for by the identity it proved, and where stock connected from, until a session opens. A bench image: the owner holds Lock, then Dispense, on the unlocked panel and keys in the code, at the address stock connected from |
| 7. Bank 0 | Only the served build moves on. On bank 0 the stock server's record is then closed; on bank 1 (a run resumed after activation) only the confirmation is left | `Status` over the device API; the server's record closed on that answer, asked again while stock's telemetry is still current |
| 8. Update key | Only for a run that signs for itself, whose image does not build the signer's key in. The app's run promotes a manifest signed elsewhere, so it enrolls no key | The app carries none. `wellfed_install::update_key` is the same exchange `wellfed update-key enroll` and a restore run: `ListClients`, then `EnrollUpdateKey` with the owner pressing Dispense within 60 s, then `ListClients` again if it went unanswered |
| 9. Backup | **No promotion without a `VerifiedBackup`.** `Bank1Reader` reads bank 1 a page at a time, and `StockBackup::from_bank` restores the marker's first word, checks the payload against the pin and rebuilds the vendor's container | `ReadBank1` over the device API (owners only, bank 0 only), the copy kept in the app's storage, or one kept by an earlier run |
| 10. Version | The floor (`UpdateState`'s highest confirmed version) decides. A manifest at or below it is refused, since it would need a downgrade statement | The manifest the APK carries, pinned at build time ([the image it installs](../../src/app/README.md#the-image-it-installs)) |
| 11. Promote | `update::Installer`, the same push as the bench's. Its error ends the run. Once writing began, bank 1 may be partly overwritten, so the verified backup is then the copy of stock | Chunks over the device API until bank 1 is activated. A dropped session is reopened, and the install resumes where bank 0 says |
| 12. Confirm | Done only when bank 1 runs the build and either reports the manifest's version confirmed or is ready to feed, which an unconfirmed trial never is. A reinstall of a build the feeder confirmed before skips the trial and leaves the floor below the version. Bank 0 means it fell back | `Status` and `UpdateState` after the restart |

**Trust on first use, and its risk.** The broker trusts whichever stock-shaped client reaches it first inside the window. A host on the same network that wins that race (or a second feeder) would be enrolled instead. Without a check, it could be walked through the whole run to a false "Installed" while the real feeder stays on stock. So:

- The run stops for the owner to confirm the enrolled device ID (on the feeder's label or in the vendor app) and its address (the router's client list) before the install request.
- Any second stock-shaped identity inside the window, or a wrong password for the enrolled one, marks the enrollment contested (`installer.contested.json` beside the config). The status shows it, the server refuses to install, and the run ends.
- A contest counts only inside the window. Once it has closed, a second identity (even the real feeder arriving late, after a stranger was enrolled) is refused like any wrong password, but not flagged. The owner's confirmation of the device ID and address is then the only check left.
- A contested enrollment is cleared by removing both files and running again on a network only the feeder joins.
- The device API's pairing is the second check: a false feeder cannot complete the setup code keyed in on the feeder's own panel.

**The backup gate.** `VerifiedBackup::verify` (and the `StockBackup` the stream makes) is the only way to make the token the promotion waits for.
- It takes either stock 1.1.15's bare application payload, whose SHA-256 is the pinned payload's (`802ef620…`, the stock review's `payload_sha256`), or the vendor's exact container (`05147476…`).
- The payload is pinned because the bank holds the payload, and a factory-flashed unit may never have been written from that exact container.
- A container is never taken apart. Any other header over the right payload is refused, since its fields are not checked and a restore would write it as given.
- At the backup step the run reads bank 1 from the replacement on bank 0, and keeps the rebuilt container.
- A kept file is written whole or not at all (a temporary file moved into place by a step that never replaces). One that does not check out stops the run, with its path. In the app, you set it aside on that stop.
- If the read is refused, or the bank does not match the pin, the run stops at the gate with the replacement on bank 0 and stock intact in bank 1. **Pick up where it stopped** resumes from pairing without asking stock anything, and reuses a copy kept by an earlier run. The same resume finishes a run that stopped after activation: it finds the build on bank 1 and waits for its confirmation.

**Which bank stock runs from** is an open question. Every install so far started from stock in bank 1, so stock wrote bank 0 and promotion overwrote bank 1. A consumer unit whose stock ran from bank 0 (after a vendor update, say) would have stock write the replacement into bank 1, leaving its own copy in bank 0. The steps above, the backup source and the promotion all assume the research unit's layout. The installer does not refuse the other layout: a served build answering from bank 1 reads as a promotion an earlier run finished, so with its version known the run waits for confirmation and takes no backup (static analysis; never run). This needs settling on a unit in that state before the installer is relied on for consumers.

**What a run needs:**
- an image the APK carries, which the build's pin names ([the image it installs](../../src/app/README.md#the-image-it-installs)). The app checks the review, the manifest and the image against that pin, and against each other, before the feeder is touched;
- for a consumer image, the home network to give it over Bluetooth, and Bluetooth in range of the feeder. A run from stock with no Wi-Fi to give is refused before it starts;
- an app built from a clean commit of the sources that decide and send the install request (`wellfed-stock`, `wellfed-client` and `wellfed-install`): stock's server refuses to enable the trial otherwise, and the app's first screen says so;
- a phone on the home network, where the feeder can reach it on the broker's and the image server's ports.

**How it asks.** The run stops at each step that needs you: setup mode, the feeder check, the recovery hold, and a restart if stock's reports were lost. The app asks on a screen for each ([the app](../../src/app/README.md#the-install)).

Two steps need presses at the panel inside a window the feeder times itself: the setup code (its six presses) and, for a run that signs for itself, the Dispense press that enrolls the update key. The run does not stop for these. It shows them and goes on at once, since only the feeder's own check of the code or the press decides the outcome, and a wait for an answer would only spend the window. The Dispense prompt goes out a second after the enrollment request (`PRESS_AFTER`), by which time the feeder should be waiting for the press. That is a timing assumption, not a signal from the feeder. If it fails, the press comes first: on a consumer image's panel, locked since its boot, it does nothing, the window closes `Unconfirmed`, and the run stops without a second request. Neither press needs the panel unlocked. The feeder takes the code's presses and the confirming Dispense whatever the lock says.

**If the code's window runs out.** Nothing is lost: stock is still in bank 1, and the replacement runs from bank 0. The feeder stays in Bluetooth setup and advertises again, so it needs no restart. A mistyped code is not offered again, since it may be a relay ([Bluetooth](bluetooth.md#setup-exchange)). The run ends either way, and **Pick up where it stopped** resumes it. A bench image's recovery window has to be opened again first, and the run asks for that.

**Stopping it.** Stopping the run, or leaving its screen, ends it the way its end does: it stops the image server and the stock server's broker, then reports the step it stopped in. It also names a write whose outcome had not settled:

- the install request, from when it is sent until the replacement answers;
- the promotion's push, until bank 1 confirms it;

That write may have run, and it is not sent again: read what the feeder runs before you start another run. Pairing is only an owner enrollment, which is safe to repeat. If the install request is still unsettled while the run pairs, the report says so: the enrollment may be repeated, the request may not.

**A consumer image.** The builder makes a consumer image (`PROVISION=consumer`: no build Wi-Fi, the release key built in), and with nothing stored it starts BLE setup mode at boot ([product architecture](product-architecture.md#setup-and-pairing)). The app has run this on the feeder from a real Reset hold ([Evidence](#evidence)).

**Not done yet:**
- The release key. It is not generated yet. The app carries a manifest signed in advance, so that manifest has to be signed by the key built into the image: enrolled keys go in the handover from stock, and a manifest signed by any other key is refused as `update-key` at promotion, after the backup. Until the real key exists, a bench build uses a private stand-in pair ([bench and consumer images](../development/firmware-builds.md)).

### Running an install on the bench

This needs the owner at the panel and a phone. Build the APK with the image to install ([the image it installs](../../src/app/README.md#the-image-it-installs)), and clear the app first if the phone ran an earlier test.

1. **Return the unit to stock** over the air: `wellfed-bench ota <name>-stock --review <stock review>`. You should see stock 1.1.15 on bank 1 and the replacement in bank 0. A consumer image starts Bluetooth setup only with no Wi-Fi stored, so while a bench image still runs on bank 1, first clear its Wi-Fi with `wellfed wifi --clear`.
2. **Check the network:** the phone must be on a network the feeder can join, and the feeder must reach the phone on the broker's and the image server's ports. Resolve any trial left open first ([the install ledger](../development/stock-server.md#the-install-ledger)).
3. **At the panel:** unlock stock (Lock about 5 s if the lock light is on). Hold nothing else yet: the app asks for the Reset hold after its warning.
4. **Run the install in the app** ([the steps](../guide/install.md)). You should see the replacement confirmed on bank 1, and the backup offered for saving.
5. **Check:** `wellfed-bench banks` still names what the bench last wrote, since the phone's install is not a job: read what the feeder runs (`bench_status`, or the feeder's status over a session) and record the run.

A phone takes no bench lease, and the bench server does not know about its install. So start no job while it runs: nothing refuses one.

## Rollback

The vendor's firmware can't ship with the app. But each feeder already holds its own copy of stock, until promotion overwrites bank 1.

**The main route: the installer saves the feeder's own stock.**

- **Where it is.** After stock installs the build into bank 0, bank 1 still holds the stock application, with only the first word of its 8-byte marker overwritten. Stock's activation programs that one 4-byte word and nothing more (inferred from the bytes and the static read). On the research unit it left `00 00 21 01` over the marker's `38 31 39 35` (physical observation, October 29, 2026): programming only clears bits, so each byte is the marker's with some bits gone.
- **Who reads it.** The build on bank 0 can read bank 1 (`sdk::bank1_read`, also used for write readback).
- **Upload.** Before the installer offers the promotion, bank 0 streams bank 1 to it on the device API: `ReadBank1 { offset }`, answered by `Bank1Page`s of 768 bytes, owners only, served only from bank 0, and refused `Busy` while an install holds the flash ([device API](device-api-diagnostics.md#stock-backup)). The transport carries no trust; the hash check below is what shows the copy is intact.
- **Promotion waits for it.** Promotion overwrites bank 1, the feeder's only copy of stock. So the installer refuses to promote until a verified backup is stored. You cannot lose stock without holding a copy of it.
- **Rebuild.** The installer is the server during an install, so the backup lands in the app. It restores the marker's first word, accepting only bytes that lost bits from the marker's (the whole bank must still hash to the pin), and rebuilds the OTA container: a 32-byte header, the payload length and byte-sum, as in [the container format](../devices/plaf103/stock/ota-requirements.md#the-image-container).
- **Checks.** The rebuilt image must match the stock image profile. For 1.1.15 it must match the pinned payload hash exactly, so a damaged read shows before anything relies on it.
- **Storage** is per device. The app keeps it under its own storage, written once and never replaced by a different copy. `wellfed backup read` keeps one under the owner's data directory, `private/stock-backups/<identity>/stock-1.1.15.bin`, the same way. A server keeps its copy under its data directory ([server design](../server/design.md)).
- **Forget this install.** The app offers this when an install's record is stuck or damaged. It clears the paired record and the feeder the phone's broker trusted, and sets aside an unreadable `installer.json`. It changes nothing on the feeder. It keeps every backup and every device's stock ledger, so a request with no known outcome still blocks a second one when that feeder comes back. If such a request is open, the app says so before you confirm. It is not offered on an uncertain write's stop. It ends "Pick up where it stopped", so a feeder already on the replacement comes back through **This feeder already runs Wellfed** on the first screen, which reads where the feeder got to and asks stock nothing. With no record, a feeder you know answering says nothing about which one this is, so the app asks. You pick a known feeder by name, or another over Bluetooth, which goes through setup again. A picked feeder takes no record and closes no stock trial on its word, because it says nothing about where stock's install went. The app skips the question only if it knows no feeders.
- **Set a damaged file aside.** Forgetting can't repair a stock ledger or a backup, so each has its own action, which only you can take. The file is renamed `<name>.damaged-<n>`, never deleted and never over an earlier one, and only if it was read and doesn't parse, or the kept backup differs from what a run read. An unreadable ledger counts as an open request until you accept this. After that, the phone no longer knows whether a request to that feeder is open, so check the feeder first. A backup set aside can't be used to go back to stock, and a fresh one is read only if the feeder still holds stock.
- **Export and restore** from the CLI:
  - `wellfed backup read` reads and keeps it from a replacement on bank 0, outside an install;
  - `wellfed backup show` prints its version, digest and path;
  - `wellfed backup export --output <file>` checks it again and copies it byte for byte to a new file;
  - `wellfed restore-stock` is "Restore stock", the signed install below.

  After promotion the feeder holds no copy, so the backup comes from the owner's copy, never read back from the feeder.
- **Saving a copy:** when an install ends, the Android app can hand the backup to any app through the share sheet.
- **Restoring from the app:** the app shows the kept backup (stock version, checked) and offers "Restore original firmware". If the phone holds no copy, it says so and takes a file you saved, through the same check.
- **Still to come:** the app, connected to a server, will show the backup, copy it to the owner's server and offer "Restore stock", through the same `wellfed-client` steps.
- **Why not keep it on the feeder.** The only place it fits is bank 1. That means running from bank 0 for good, with no known-good fallback bank, and the first update into bank 1 would destroy it anyway. No other 1.3 MiB of flash is established as free.
- **Why this route.** These are your own bytes, from your own device, so there is no redistribution question.

**A second route: the vendor's update service.** The vendor app asks the cloud for a device's update metadata (private evidence E12). An installer could fetch the official package the same way with the user's own account. This depends on the vendor's service, and may offer only the latest version.

**Rolling back.** Our firmware installs a stock image as a signed update (`wellfed-bench ota --review <stock review>`). The bank-0 build writes it into bank 1. Stock's own KM0 then runs, so our trial counter never counts it.

`wellfed restore-stock` and the app do the same from the owner's backup, through one driver, `wellfed_install::restore`.
- **The key.** A stock offer always needs a downgrade statement bound to the feeder's identity, the manifest's digest and the floor, so nothing can be signed ahead of time. The restore makes an update key when it starts, enrolls it (you press Dispense at the feeder, the physical consent) and signs on the phone or computer. `--key` on the CLI gives a key the feeder already trusts instead, for the bench, and enrolls nothing.
- **The key's file** sits beside the backup only until the restore ends. If the enrollment went through and the run ended before its offer, the next run within the hour finds the key listed: it sends no second enrollment and asks for no second press. A press is consent to this restore, not to one days later, so a file older than an hour is removed and the run enrolls a new key, with a new press. The file goes before the offer does, with the manifest and statement already signed in memory, so a run killed after that cannot offer again with no press; a feeder that never answered the offer costs a second press. It also goes once the feeder says the key is not enrolled, and in the app when you leave the screen. A spent key stays listed on the feeder with its private half gone: it signs nothing more, but it takes a client slot until someone removes it.
- `wellfed_signing::Frozen::stock` signs it as the stock image, at a version above the feeder's floor (`--version` overrides that). Only the pinned container passes, as for a stock review.
- The downgrade statement is bound to the feeder's identity and the floor it reports.
- The push is the installer's own, from either bank: from bank 1, the normal case after promotion, bank 1 gives way and bank 0 writes and activates stock. Both banks read one store and one list of enrolled keys, so bank 0 takes the same offer.
- Nothing re-offers once the push has returned. (Within it, the shared installer offers again only where the update protocol needs it, as after bank 1 gives way.)
- A refusal ends the restore. A feeder that stops answering after the offer leaves the result **uncertain**, since it may already run stock, which serves no device API. That is reported as such, and never re-offered.

What a rollback keeps and leaves behind:

- **Stock's settings stay.** Our firmware never writes `0x3e8000`–`0x3f0000`, so stock comes back still provisioned. It still names the installer's broker and retries forever: hold Reset for about 5 seconds and set it up in the vendor's app.
- **Our storage stays, except the owner's Wi-Fi, plans and clients.** `0x340000`–`0x370000` holds the journal, the lifetime statistics and the feeder's identity. Stock does not read it. Bank 0 leaves a handover mark when it activates stock, and the next Wellfed boot, a later install or the loader's fallback, clears the stored Wi-Fi, every plan and every client, resets the update floor and forgets the boot confirmation, before anything reads them. The handover runs the same when the loader falls back to Wellfed because stock did not boot, which is fine: the owner's signed downgrade already gave up that floor. So reinstalling the same build runs a real trial and raises the floor again. Only bank 0 forgets the confirmation: on bank 1 the build's next boot would run a trial with no Wi-Fi left to pass it. Stock's activation record stays until the next install overwrites it; a manifest still pending is cleared. The handover keeps the identity, since the owner's backup is filed by it. So a later consumer install behaves like a first one, starts Bluetooth setup by itself with no meals, and a phone that was the owner pairs again. An image with build Wi-Fi (a bench image) pairs through the recovery window, so it keeps all three, but it still resets the floor and forgets the confirmation, and erases the mark. A factory reset pending when stock activates leaves no mark, so the floor and the confirmation stay. That is a known limit: only a promotion signed above that floor gets in, which the installer's own key can sign.
- **The mark is bank 0's build's to write.** Bank 0 runs the build from an earlier install or update, so a bank 0 older than this feature writes no mark, and the feeder keeps its people until a factory reset. A later install then finds them as they were, and nothing is cleared.
- **A fallback feeds nothing.** If the loader falls back to Wellfed, the feeder has no plans and no network until it is set up again, so no meal runs by itself. Any plans stock still holds stay in stock's sectors, for stock if it comes back. After a Reset hold there are none.
- **A cut stays recoverable.** The record goes first, then the mark, then the bank 1 marker. A cut before the mark leaves this Wellfed with its owner. A cut after it leaves a boot that clears itself into Bluetooth setup, with no meals, whether or not stock went in. The clearing itself repeats after a cut, because the mark goes last. The record kept through the handover refuses the same manifest as failed. Its protection is narrow: it cannot tell whether stock booted, so it also refuses a retry after stock ran well, and it stops only that exact manifest. A restore signed with a fresh key has a new digest and gets past it, and a promotion overwrites it. A run cut after give-way or mid-write leaves its manifest pending, and the next restore cancels that once before its own offer; an install running now stops that restore, and a cancel in any doubt stops it too.
- **Bank 0 keeps our build** as a fallback, which the loader never chooses while stock in bank 1 is valid.
- **Stock's OTA status sector** (`0x3ef000`) may differ from the original after the install.
- **No automatic fallback from a stock that hangs.** The loader falls back to bank 0 only when bank 1 is invalid, and Wellfed is then in Bluetooth setup. If a valid stock fails to boot from bank 1, recovery is UART only (`wellfed-bench stock-prepare` / `stock-execute`).

### Proving rollback on the bench

This needs the owner. It starts from a confirmed `feeder` on bank 1 and an older `feeder` on bank 0, and a phone with an APK that carries a `feeder` image. The bench rules apply: one holder, the hotspot and `wellfed-stock serve --device plaf103` as bounded foreground processes, and everything restored afterwards. To watch stock report on `serve`, stock must already hold this host's broker from an earlier setup frame, and the host must admit TCP 1883 and 8088 from the hotspot ([the bench host's firewall](../development/stock-server.md#the-bench-hosts-firewall)). This form, with the app installing, has not run in this order.

1. **Go to stock:** `wellfed-bench ota <name>-stock --review <stock review>` from the confirmed build. You should see stock 1.1.15 on bank 1, and the older build on bank 0.
2. **Install and take the backup:** run the install in the app, from a real Reset hold ([above](#running-an-install-on-the-bench)). It reads bank 1 (1,792 pages), keeps it, promotes and confirms. Save the backup through the share sheet.
3. **Check it:** `sha256sum` of the saved file prints `05147476…` (so its payload is `802ef620…`), and `cmp` finds it identical to the stock review's `image.bin`. Then stop `wellfed-bench serve --device plaf103`, since `wellfed join` is refused beside it, and join this computer with an Owner invite from the app (`wellfed join`). An update needs an owner's session, and the invite sheet starts on Member.
4. **Roll back.** With `wellfed-stock serve --device plaf103 --config private/installer.json` running if stock holds this host's broker (`init-installer` writes that config; `serve` refuses the bench's `server.json`, whose `plans: []` would erase stock's plans as soon as stock comes back) and a UART capture (`wellfed-flash read observe`), run `wellfed restore-stock --backup <the saved file>`. It makes a fresh key and asks for a Dispense press at the panel to enroll it, because the app's image doesn't trust the bench key. Bank 1 gives way, bank 0 writes stock, and the restore reports activated. You should see stock's `app_main:version:` banner from bank 1 on the UART.
5. **Go back and restore:** run the install in the app again. You should see the replacement confirmed on bank 1. Stop the hotspot as before, and record the run: versions, digests and the kept file's path.

## Where the installer lives

The decisions are in [product architecture](product-architecture.md). For the installer they mean:

- **The steps live in `wellfed-client`:** the setup frame, the single stock request, the progress tracking, the backup gate and the backup receiver. The Android app runs them through `wellfed-install`, which supplies the IO: the image server, the broker and the sockets.
- **The Android app** hands stock your Wi-Fi over its setup network, and serves the image itself from a foreground service ([the app](../../src/app/README.md#the-install)).
- **The CLI** keeps stock's image and goes back to it (`wellfed backup` and `wellfed restore-stock`). It does not install from stock. `wellfed-stock` holds the stock protocol, and its binary stays the bench's tool for reading stock and closing a request. The server is not an installer: every install from stock runs from the app. It links neither `wellfed-stock` nor `wellfed-install`, and it only installs releases on a feeder that already runs Wellfed.
- **Afterwards,** the feeder is handed to the pairing flow, not to a server. The app talks to it directly on the LAN, and a server is optional.

## Evidence

- **Observed** (October 23, 2026, research unit):
  - `fw40-stock-ota-05`: stock 1.1.15 installed a replacement build through its own MQTT-triggered HTTP updater, with no UART write. It downloaded the image in about 40 s, checked its byte-sum and wrote it to bank 0. The loader then booted the build from bank 0.
  - `fw40-stock-ota-06-promote`: the build on bank 0 installed the same build, signed, into bank 1, where it confirmed. Promotion took about 3.5 min with the updater of the time.
  - `fw40-stock-ota-07`: the whole sequence in one command, with a fresh build: install, health from bank 0, and promotion to bank 1.
  - `fw41-roundtrip-01`: to stock over our updater and back over stock's, with no UART write and nobody at the bench.
  - The bench requirements came from earlier tries: `fw40-stock-ota-02` (the image port blocked, error 2003), `fw40-stock-ota-03` (the 60 s telemetry bound) and `fw40-stock-ota-04` (the stale version report).
- **Observed:** stock 1.1.15 dispensing locally through `wellfed-stock`. Stock's client ID equals the device ID in its topics, from a captured CONNECT.
- **Recovered:** stock's setup protocol, frame and re-provisioning guard, from the stock image and the vendor app. The frame and the reset gesture have since run on the research unit (below).
- **Recovered** (October 28, read-only static pass over the 1.1.15 image): the broker kept as a DNS name in `0x3e8000` on plaintext 1883 with no TLS or pinning; setup mode reached only through the Reset hold, whose factory restore erases the plans; the plan reply's fields.
- **Observed** (October 23, `fw40-stock-ota-07-uart-01`): stock looking its broker name up through the network's resolver and then a public one, and a set-up stock connecting with no `BINDING`.
- **Observed** (October 30): the install, run from the app on the research unit: a real Reset hold, the setup frame, stock's install, Bluetooth setup, the kept backup, the promotion and confirmation on bank 1. After a restore it ran again at the same version. A command-line installer, since removed, ran from bank 0 with a virtual panel the same day: Bluetooth setup with a new client key, the join, the kept backup, the promotion and confirmation.
- **Observed** (October 30, unattended): the install from stock as a bench server job, since removed, with a virtual panel, on a route that reached stock by DNS. That route has since been removed.
- **Observed** (October 31): the install, run by that command-line installer from a real Reset hold: the frame, stock's install into bank 0, and the rest of the run, resumed once from bank 0 after a setup code timed out. In a second run the first code expired too, and a second code offered in the same run worked. A vendor app setup after a restore also completed, with stock on the vendor's cloud.
- **Observed** (October 30): the restore, run on the research unit from the app from bank 0 and from bank 1, and by the bench job from bank 1.
- **Observed** (October 31): a manual `nmcli dev wifi connect` to stock's setup network timed out, with NetworkManager reporting `acd conflict` on the offered 192.168.1.100. After `nmcli con modify` set `ipv4.dad-timeout 0` and `connection.autoconnect no` on that profile, it joined and the frame reached stock.
- **Modeled** (October 26): the installer, trust on first use, the setup-frame encoder, the backup gate, the stock backup read and `wellfed restore-stock`, host-tested and simulated, on synthetic bytes.
