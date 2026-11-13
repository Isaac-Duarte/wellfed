# Consumer installation and rollback

This page explains how a feeder on **stock 1.1.15** moves to Wellfed with nothing but an installer app, and how it goes back. It is for owners wondering what installing involves, and for contributors working on the installer. The goal is an experience like the vendor app's, with no cloud and no account. The bench runs are in [bank, install and rollback bench checks](install-bench-checks.md).

No vendor code, credentials or device identifiers appear here. The evidence index for the static findings is private: `private/research/stock-installer/evidence.md`.

## The one idea

Stock's own updater does the hard part: stock 1.1.15 installs any image we serve into its other bank, with no UART and no soldering. So the installer only has to get stock talking to it, ask for one update, and serve the image. You hold Reset, which erases stock's settings and meals, and your phone hands the feeder your Wi-Fi over the feeder's own setup network. Before Wellfed overwrites stock's last copy, the installer saves the feeder's own stock image, so you can always go back.

The Android app runs the whole move and the trip back ([Evidence](#evidence)). The desktop apps run the same steps, with the owner switching networks by hand ([on a computer](#stocks-setup-protocol)), which hasn't been run on a feeder; status is on the [roadmap](../roadmap.md). Going back works from a stock image we serve as well as from the feeder's own copy, and a vendor app setup after a restore completed. The image contract is in [stock OTA requirements](../devices/plaf103/stock/ota-requirements.md).

## What every install needs

| Requirement | Why |
| --- | --- |
| One `OTA_UPGRADE`, never resent | A refused precheck can erase stock's OTA status sector, and an uncertain request may still be downloading |
| Resolve a failed request before the next one | The server's ledger refuses a new trial while one is failed or uncertain. It resolves from fresh evidence: 20 min, 10 quiet connected minutes, a fresh version report |
| The image port reachable from the feeder | With only TCP 1883 allowed, stock connected for about 25 s, then reported `OTA_INFORM` error 2003 |
| Tolerate stock's ~61 s heartbeat | A 60 s telemetry bound restarted the quiet interval at every heartbeat |
| Ask stock for its attributes when its version report is stale | Idle stock reports its version only around a connect |
| A URL under 128 bytes, `Content-Length` title-cased, a direct `200` | Stock's parser limits ([requirements](../devices/plaf103/stock/ota-requirements.md#http-server-and-url-requirements)) |
| Mains power, idle, model and version checked first | Stock's own precheck; the server's trial gate checks the same reports |

The bench still uses a build carrying the owner's Wi-Fi credentials and update key, which a consumer must not need; generic images remove it.

## Stock's setup protocol

This is the app's main route. The vendor app provisions a PLAF103 through the feeder's **setup access point**, not over Bluetooth. In setup mode the feeder runs an open access point, `PL-AF103-` and the last two bytes of its MAC, on channel 11 (observed). The app joins it and sends one UDP datagram to **192.168.1.1:17484** (`0x444C`), where stock's `Ap_Config_Udp_Thread` listens. BLE service `ffee` is connected alongside (`ffe1` write, `ffe2` data, `ffe3` read, `ffe4` notify), but the frame goes over UDP.

**The frame.** PLAF103 is on the app's list of products sent without encryption, so the frame is plaintext with no checksum:

```text
'D' 'L' | type=3 | checksum flag=0 | 0 | payload length | lp(SSID) lp(password) lp("host:PPPP") lp(numeric id) | 0xFF
```

- **`lp(x)`** is one length byte, then the UTF-8 bytes.
- **Types:** 1 Wi-Fi only; 2 server only; 3 both (what the app sends for PLAF103); 4 three other strings of up to 50 bytes, meaning unknown.
- **Field limits:** SSID up to 32 bytes, password up to 64, `host:PPPP` up to 100, numeric id up to 30 ASCII digits.
- **`host:PPPP`** is the MQTT broker, from the app's cloud configuration. Stock takes the host before the `:` and up to four port digits after it, so 1883 fits.
- **The numeric id** is most likely the account's user id. It must be digits, and nonzero.

**What stock does with it** (runtime addresses):

| Step | Function |
| --- | --- |
| Check the `DL` prefix and `0xFF` terminator, dispatch on the type | `0e01a160` |
| Store the Wi-Fi | `0e007bb0` |
| Store port, numeric id and host in a 0x484-byte record (`Set_Dev_Info`) | `0e007cf8` |
| Set the MQTT port, host and id; poll whether MQTT came up (`0e012780`, up to 20 tries); only then persist the record to flash `0x3e8000`, and a 0x60-byte record to `0x3e9000` | Setup thread `0e0084d8` |
| At every boot: load `0x3e8000` and connect to that host and port | — |

**Saving it takes an answer.** The setup thread waits for the state stock also requires before an install, which clears when `BINDING` is answered. With no answer, stock joined the Wi-Fi and reached the broker, refused the install with error 1204 ("device not bound"), and about 20 s after it arrived went back to its setup network with nothing saved (observed). So the broker must be listening before the frame goes, and reachable within that time. An installer's server has no captured reply, so it answers `code` 0, a nonzero `bindId` and an empty `cameraAuthInfo`. Stock accepts that, saves the frame and takes an install (observed). What the vendor's `bindId` means is not established.

**Re-provisioning guard.** While the `0x3e9000` record is non-blank and the device is bound, types 1–3 are acknowledged but not applied, so a feeder must be reset or unbound first.

**Credentials.** The MQTT client ID, username and password are the feeder's own, loaded from flash at boot, not issued at bind. `BINDING` is an application-level association, which `wellfed-stock` answers by replaying the captured cloud reply. Stock dispatches `OTA_UPGRADE`, `GET_CONFIG`, `BINDING`, `UNBIND`, `DEVICE_REBOOT`, `WIFI_RECONNECT_SERVICE`, `WIFI_CHANGE_SERVICE`, `ATTR_SET_SERVICE` and `ATTR_GET_SERVICE` over MQTT ([stock behavior](../devices/plaf103/stock/README.md)).

**Getting into setup mode** costs stock's plans. A set-up stock that can't reach its Wi-Fi retries forever and never opens its access point on its own. The only way in is the Reset hold: Reset (the rightmost button) for about 5 s on an unlocked panel. That runs stock's full factory restore, erasing its broker, Wi-Fi and binding records, its feeding plans and their record rings, then restarts ([what the restore clears](../devices/plaf103/stock/README.md#reset-and-the-cloud)). The feeder says it is ready for a Wi-Fi connection and opens its setup network, and the plan sectors read back erased (observed). To unlock the panel, hold Lock about 5 s while the lock light is on.

**On a phone.** The app asks Android for a network whose name starts with `PL-AF103-`, through Android's own picker. Once joined, the process's sockets use that network until the frame is out.

- A phone that can hold a local-only network beside its Wi-Fi stays on both, as the research unit's Pixel did (observed). One that can't leaves home for those seconds and must be back within stock's wait (modeled, not run). It must come back at the address the frame named, or the run stops, since the feeder would look in the wrong place.
- The password goes unencrypted over the setup network, and the screen says so. A name starting `PL-AF103-` is no proof the network is a feeder's either: anyone can name one so, and the app, like the vendor's, can't tell.
- A phone that set the feeder up in the vendor's app may keep that network saved. On the research unit's Pixel, Android's picker then found nothing, though the feeder was broadcasting, until the saved network was forgotten in Wi-Fi settings (observed once). The screen says so when a join fails.

**On a computer.** No desktop lets an app join a Wi-Fi network in a portable way, so the app guides the owner through each switch and watches for it by the computer's routing table. It sends nothing to the feeder to tell, since stock takes one frame.

- **Refused first** when the home address the frame names isn't private (RFC 1918), as a VPN's tunnel address in 100.64.0.0/10 or a public one isn't: the feeder can't reach it, and the screen says to turn the VPN off. A VPN whose own address is private isn't caught here; the feeder then never arrives, and the screen's advice after a failed join includes turning a VPN off. Refused too when the route toward 192.168.1.1 already leaves from another 192.168.1.x address as the watch starts (a guest network, an ISP's or a travel router, on Wi-Fi or a second adapter), since the frame would reach that router; the screen says to disconnect it. And refused when that address is the home address on a network named `PL-AF103-…`: the owner joined the feeder's network before the handover read where home is, so the frame would name the wrong address.
- **Joined, failing closed.** The route toward 192.168.1.1 must change to leave from an address in 192.168.1.0/24 that no interface held when the watch began, other than the home address. A docked computer whose cable and Wi-Fi are both on a home network on those addresses already holds its Wi-Fi's address, so unplugging the cable isn't a join. Then the interface holding that address must be on a Wi-Fi network the system names, starting `PL-AF103-`. A name the system won't give is a refusal that says how to allow it, never a match:
  - **macOS** names it through CoreWLAN only to an app allowed the location; the runner asks for that the first time.
  - **Windows** reads `netsh wlan show interfaces` for that interface, which Windows 11 24H2 answers only with Location on for desktop apps.
  - **Linux** asks `iw dev <if> link`, then NetworkManager for that device's access point in use. iw comes first because it names only the linked network; NetworkManager lists every network nearby, so a list with more than one marked in use, which a crafted nearby name can make, names none. NetworkManager's own "current network" is the profile holding the default route, which with a cable is the wired one, so it isn't used.

  A name holding a control character is refused, as a possible forged line. Each helper has 3 s to answer and the Mac's runner 30 s (the location prompt), and a stop ends the read; the name is then read as none. The route is read again after the name, and must still leave from the same address.

  The name alone isn't enough either: a computer can be on the feeder's Wi-Fi while its route to 192.168.1.1 still goes over a cable to a home router on the same addresses.
- **The frame goes from that address.** Right before the frame the run reads the route to 192.168.1.1 again, and stops if it no longer leaves from the joined address, as when a cable was plugged back in. The frame's socket is then bound to the address, so one gone by the send fails it. Binding doesn't pin the interface, which is why the route is read again; a change in the moment between the two isn't caught.
- **Back** once the default route leaves from an address other than the one on the feeder's link. If that isn't the address the frame named, the run stops, as on a phone. The app waits up to two minutes, the run's own wait for stock after the frame.
- **The window is short.** Stock polls a limited number of times for the broker to come up (static analysis), and gives up about 20 s after reaching it if nothing answers `BINDING` (observed), so the screen says to switch back at once. A computer that keeps the home network on a cable or a second adapter needs no switch back, and the screen recommends that. A home network on 192.168.1.0/24 needs the computer fully off it while the frame goes. How long an owner takes to switch back, against stock's window, hasn't been measured.
- The computer is kept awake for the run (a power assertion through `caffeinate` on macOS, a logind inhibitor through `systemd-inhibit` on Linux, `SetThreadExecutionState` on Windows), and closing the window mid-run asks first. Neither covers a closed lid.
- A feeder that never arrives may be the computer's firewall, since the broker (TCP 1883) and the image server (TCP 8088) take connections from the feeder. On a computer the stop screen names the system's own prompt or setting. The app changes no firewall itself.

**If the feeder doesn't arrive.** Two minutes after the frame with no feeder at the broker, the run stops with nothing asked of stock, as after a wrong password or a Wi-Fi with no 2.4 GHz band. Stock keeps nothing and reopens its setup network, so a new frame needs no second Reset hold (static analysis, and seen for an unanswered `BINDING`; a retry after a wrong password also ran once from the app on October 30, 2026, with no second hold recorded). That is a new frame to a feeder visibly still in setup mode, unlike the install request, which is never resent. Once stock has reached the broker it has kept the frame and has no setup network left, so a later run only waits for it.

**Still open:** what the numeric id is (stock checks only its form, and repeats it in `BINDING`); what `UNBIND` (`0e015fd4`, not decompiled) clears; and whether the credentials are fixed at the factory or derived from the serial. Either way they reach our broker unchanged and survive the Reset hold (observed).

## What the installer's server must answer

`wellfed-stock` implements the stock side:

- a broker on TCP 1883 that accepts stock's legacy MQTT 3.1 CONNECT;
- the `dl/PLAF103/<device-id>/…/post` and `/sub` topics;
- replies to `NTP` and `BINDING` (a captured reply if there is one, else the generic one a stock fresh from the setup frame needs), and to `GET_FEEDING_PLAN_EVENT` only with a captured plan snapshot. An installer's config has none, so that request goes unanswered: stock erases its plan table on any reply with a `plans` key, and a stock reached with no Reset hold may still hold plans ([stock server](../development/stock-server.md#an-installers-server-trust-on-first-use));
- acknowledgements of the startup, attribute, log and error events.

For a consumer it must also accept a device it has never seen. **Trust on first use:** an installer's config (`wellfed-stock init-installer --device plaf103`) names no device. `serve --enroll-minutes N` admits the first stock-shaped CONNECT within the window, records that client's ID, username, password and address beside the config (`installer.json`'s record is `installer.device.json`), and pins it, so nobody else logs in, then or later. Stock's client ID is its device ID.

## The installer

One run of the app goes from stock to a confirmed replacement. You hold Reset; the installer sends one type-3 frame with your home network and its own `host:1883`; stock joins and connects to the installer's broker with its own credentials, so it needs no redirect and no captured credentials. After the pre-flight (model PLAF103, version 1.1.15 on an allow-list, mains power, idle: stock's own precheck, which the bench's `stock-ready` stage also checks), one `OTA_UPGRADE` installs the build into bank 0, with stock's progress reports driving a progress bar for about 40 s. The build answers the device API from bank 0, the installer becomes its first owner, takes the backup, promotes the build into bank 1, and hands the confirmed build to the [pairing](pairing.md) flow: your app is its first client, and a server joins later by invite if you run one.

The replacement starts with no meals, since the installer doesn't read stock's plans. After the setup frame stock's clock is set whatever it reads, since the Reset hold erased its plans and no meal can come due in the jump. Stock with its clock two days behind refused an install with error 1214 (observed once; the clock as the cause is likely), and a run that stops there names the error.

The run has three layers:

- **The steps** are a sans-IO state machine in [`wellfed_client::install`](../../src/client/wellfed-client/src/install.rs), which makes every decision between steps.
- **The driver,** [`wellfed-install`](../../src/client/wellfed-install), is a tokio crate that does each step's IO: stock's server, the request, pairing, the backup, the promotion, and the record a resumed run reads. Stock's side runs in the driver's own process, since an app can't spawn the `wellfed-stock` binary: the broker, the session with stock and the image server are tasks the run starts and stops ([stock server](../development/stock-server.md#how-the-crate-is-laid-out)).
- **The platform** supplies what only it owns through the driver's traits: prompts to the person at the feeder, the Bluetooth radio, finding a feeder on the network, the client key, and progress. The app uses its screens, Android's Bluetooth and NSD, and its secure storage.

| Step | Decided in `wellfed-client` | IO in `wellfed-install` and the app |
| --- | --- | --- |
| 1. Provision | The type-3 frame, refused before anything is sent if stock would misread a field; sent once | The owner is told the Reset hold erases stock's plans and enters setup mode. The phone goes to its access point ([on a phone](#stocks-setup-protocol)), sends one UDP datagram to 192.168.1.1:17484, and comes back to the home network |
| 2. Stock ready | Online, 1.1.15 (any other version ends the run), on its adapter | Stock's server in this process, with the trial enabled and a 15-minute enrollment window: the broker trusts the first feeder, and holds its CONNACK until the server has subscribed, so stock's startup reports reach it. Its status, polled. If stock still reports no version after 90 s, the owner restarts it once to replay them |
| 3. Confirm the feeder | The owner checks the enrolled device ID and address before anything is sent; a contested enrollment ends the run | The status document's `enrollment` (never its credentials) and a screen that asks |
| 4. Request | Exactly one `OTA_UPGRADE` per run; refused or uncertain ends the run | The image server in this process, serving the review to the enrolled feeder's address, then the trial request. The server's record is read back: refused only when the server refused and a successful ledger read shows no record, uncertain otherwise (an unanswered request included). The server refuses while the enrollment is contested |
| 5. Install | Progress; stock's refusal ends the run, even before the request's outcome is read; stock going quiet means it restarted | The trial phase, and the OTA reports since the request that name it (or nothing) |
| 6. Pair | The installer must be an owner first. The route follows the review's provisioning (`PairRoute`): a consumer image (no network yet, so in BLE setup) by Bluetooth setup with the home network, then a wait for its join; a bench image (on its build network, knowing no client key) by the recovery window | A key the build knows opens a session at once. A run whose broker enrolled the same stock device as an earlier run reads that run's record (`installing.json` beside the backups) and looks for that feeder where it last answered, where stock connected from, and over NSD. Otherwise a known feeder is tried only where stock connected from, since the app may know another, then setup. A run from bank 0 given an address (where the app's listener heard something answer at stock's old address) tries a known feeder there, but records nothing for the stock device, since the address may have passed to another feeder, so that answer closes no stock record. The finishing run removes its record. After Bluetooth setup the feeder is looked for by the identity it proved and where stock connected from; the recovery hold (Lock, then Dispense, on the unlocked panel, then the code) uses the address stock connected from |
| 7. Bank 0 | Only the served build moves on. On bank 0 the stock server's record is then closed; on bank 1 (a run resumed after activation) only the confirmation is left | `Status` over the device API; the server's record closed on that answer, asked again while stock's telemetry is still current |
| 8. Backup | **No promotion without a `VerifiedBackup`**. `Bank1Reader` reads bank 1 a page at a time, and `StockBackup::from_bank` restores the marker's first word, checks the payload against the pin and rebuilds the vendor's container | `ReadBank1` over the device API (owners only, bank 0 only); the copy kept in the app's storage, or one an earlier run kept |
| 9. Version | The floor (`UpdateState`'s highest confirmed version) decides. A manifest at or below it is refused, since it would need a downgrade statement | The manifest the APK carries, pinned at build time ([the image it installs](../../src/app/README.md#the-image-it-installs)) |
| 10. Promote | `update::Installer`, the bench's push, with the signed manifest: a consumer image trusts only the release key and keys its owner enrolled, and the app's run enrolls nothing. Its error ends the run. Once writing began, bank 1 may be partly overwritten, so the verified backup is then the copy of stock | Chunks over the device API until bank 1 is activated. A dropped session is reopened, and the install resumes where bank 0 says |
| 11. Confirm | Done only when bank 1 runs the build and reports the manifest's version confirmed or is ready to feed, which an unconfirmed trial never is. A reinstall of a build the feeder confirmed before skips the trial and leaves the floor below the version. Bank 0 means it fell back | `Status` and `UpdateState` after the restart |

**Trust on first use, and its risk.** The broker trusts the first stock-shaped client inside the window. A host on the same network that wins that race, or a second feeder, would be enrolled instead and could be walked to a false "Installed" while the real feeder stays on stock. So:

- The run stops for the owner to confirm the enrolled device ID (on the feeder's label or in the vendor app) and its address (the router's client list) before the install request.
- A second stock-shaped identity inside the window, or a wrong password for the enrolled one, marks the enrollment contested (`installer.contested.json` beside the config). The status shows it, the server refuses to install, and the run ends.
- A contest counts only inside the window. After it, a second identity (even the real feeder arriving late after a stranger was enrolled) is refused like any wrong password but not flagged, so the owner's check of device ID and address is the only one left.
- To clear a contest, remove both files and run again on a network only the feeder joins.
- The device API's pairing is the second check: a false feeder can't complete the setup code keyed in on the feeder's own panel.

**The backup gate.** `VerifiedBackup::verify` (and the `StockBackup` the stream makes) is the only way to make the token the promotion waits for.

- It takes either stock 1.1.15's bare application payload, whose SHA-256 is the pinned payload's (`802ef620…`, the stock review's `payload_sha256`), or the vendor's exact container (`05147476…`). The payload is pinned because the bank holds the payload, and a factory-flashed unit may never have been written from that exact container.
- A container is never taken apart. Any other header over the right payload is refused, since its fields aren't checked and a restore would write it as given.
- At the backup step the run reads bank 1 from the replacement on bank 0, and keeps the rebuilt container.
- A kept file is written whole or not at all (a temporary file moved into place by a step that never replaces). One that doesn't check out stops the run with its path, and in the app you set it aside on that stop.
- If the read is refused or the bank doesn't match the pin, the run stops at the gate with the replacement on bank 0 and stock intact in bank 1. **Pick up where it stopped** resumes from pairing without asking stock anything, and reuses a copy an earlier run kept. The same resume finishes a run that stopped after activation: it finds the build on bank 1 and waits for its confirmation.

**Which bank stock runs from** is an open question. Every install so far started from stock in bank 1, so stock wrote bank 0 and promotion overwrote bank 1. A consumer unit whose stock ran from bank 0 (after a vendor update, say) would have stock write the replacement into bank 1, leaving its own copy in bank 0. The steps, the backup source and the promotion all assume the research unit's layout. The installer doesn't refuse the other layout: a served build answering from bank 1 reads as a promotion an earlier run finished, so with its version known the run waits for confirmation and takes no backup (static analysis; never run). This needs settling on a unit in that state before the installer is relied on for consumers.

**What a run needs:**

- an image the APK carries, named by the build's pin ([the image it installs](../../src/app/README.md#the-image-it-installs)). The app checks the review, the manifest and the image against that pin, and each other, before the feeder is touched;
- for a consumer image, the home network to give it and Bluetooth in range of the feeder. A run from stock with no Wi-Fi to give is refused before it starts;
- an app built from a clean commit of the sources that decide and send the install request (`wellfed-stock`, `wellfed-client` and `wellfed-install`). Stock's server refuses to enable the trial otherwise, and the app's first screen says so;
- a phone or computer on the home network, where the feeder reaches it on the broker's and the image server's ports.

**How it asks.** The run stops at each step that needs you: setup mode, the feeder check, the recovery hold, and a restart if stock's reports were lost. The app asks on a screen for each ([the app](../../src/app/README.md#the-install)).

One step needs presses inside a window the feeder times itself: the setup code (six presses). The run shows it and goes on at once, since only the feeder's own check decides the outcome and waiting would spend the window. The presses don't need the panel unlocked. The install enrolls no update key: it promotes a manifest signed elsewhere, whose signer the image builds in.

**If the code's window runs out,** nothing is lost: stock is still in bank 1, and the replacement runs from bank 0. The feeder stays in Bluetooth setup and advertises again, with no restart. A mistyped code isn't offered again, since it may be a relay ([Bluetooth](bluetooth.md#setup-exchange)). The run ends either way, and **Pick up where it stopped** resumes it; a bench image's recovery window must be opened again first, and the run asks for that.

**Stopping it,** or leaving its screen, ends the run as its end does: it stops the image server and the broker, then reports the step it stopped in. It also names a write whose outcome hadn't settled: the install request, from when it is sent until the replacement answers, or the promotion's push, until bank 1 confirms it. That write may have run and is not sent again, so read what the feeder runs before another run. Pairing is only an owner enrollment, safe to repeat; if the install request is still unsettled while the run pairs, the report says the enrollment may be repeated and the request may not.

**A consumer image** (`PROVISION=consumer`: no build Wi-Fi, the release key built in) starts BLE setup mode at boot with nothing stored ([product architecture](product-architecture.md#setup-and-pairing)). The app has run this on the feeder from a real Reset hold ([Evidence](#evidence)).

**The release key** isn't generated yet. The app carries a manifest signed in advance, which must be signed by the key built into the image: enrolled keys go in the handover from stock, and a manifest signed by any other key is refused as `update-key` at promotion, after the backup. Until the real key exists, a bench build uses a private stand-in pair ([bench and consumer images](../development/firmware-builds.md)).

## Rollback

The vendor's firmware can't ship with the app, but each feeder holds its own copy of stock until promotion overwrites bank 1.

**The main route: the installer saves the feeder's own stock.**

- **Where it is.** After stock installs the build into bank 0, bank 1 still holds the stock application, with only the first word of its 8-byte marker overwritten. Stock's activation programs that one 4-byte word and nothing more (inferred from the bytes and the static read). On the research unit it left `00 00 21 01` over the marker's `38 31 39 35` (physical observation, October 29, 2026): programming only clears bits, so each byte is the marker's with some bits gone.
- **Upload.** The build on bank 0 reads bank 1 (`sdk::bank1_read`, also used for write readback) and, before the installer offers the promotion, streams it on the device API: `ReadBank1 { offset }`, answered by `Bank1Page`s of 768 bytes, owners only, served only from bank 0, refused `Busy` while an install holds the flash ([stock backup](device-api-diagnostics.md#stock-backup)). The transport carries no trust; the hash check does.
- **Promotion waits for it.** Promotion overwrites bank 1, the feeder's only copy of stock, so the installer refuses to promote until a verified backup is stored. You can't lose stock without holding a copy.
- **Rebuild and check.** The backup lands in the app, since the installer is the server during an install. It restores the marker's first word, accepting only bytes that lost bits from the marker's (the whole bank must still hash to the pin), and rebuilds the OTA container: a 32-byte header, the payload length and byte-sum, as in [the container format](../devices/plaf103/stock/ota-requirements.md#the-image-container). For 1.1.15 the payload must match the pinned hash exactly, so a damaged read shows before anything relies on it.
- **Storage** is per device. The app's copy and the one `wellfed backup read` keeps under `stock-backups/<identity>/stock-1.1.15.bin` in the CLI's data directory are written once and never replaced by a different copy.
- **Forget this install,** for a stuck or damaged install record, clears the paired record and the feeder the phone's broker trusted, sets aside an unreadable `installer.json`, and changes nothing on the feeder. It keeps every backup and stock ledger, so a request with no known outcome still blocks a second one when that feeder returns (the app warns before you confirm). It isn't offered on an uncertain write's stop. It ends "Pick up where it stopped", so a feeder already on the replacement comes back through **This feeder already runs Wellfed**, which reads where the feeder got to and asks stock nothing. With no record, the app asks which feeder this is (unless it knows none): a known one by name, or another over Bluetooth through setup. A picked feeder takes no record and closes no stock trial, since it says nothing about where stock's install went.
- **Set a damaged file aside:** a stock ledger or backup that was read and doesn't parse, or a kept backup that differs from what a run read, is renamed `<name>.damaged-<n>` by your action only, never deleted or put over an earlier one. An unreadable ledger counts as an open request until then, and afterwards the phone no longer knows whether one is open, so check the feeder first. A backup set aside can't take you back to stock, and a fresh one is read only if the feeder still holds stock.
- **From the CLI:** `wellfed backup read` reads and keeps it from a replacement on bank 0, outside an install; `wellfed backup show` prints its version, digest and path; `wellfed backup export --output <file>` checks it again and copies it byte for byte to a new file; `wellfed restore-stock` is "Restore stock", below. After promotion the feeder holds no copy, so a restore uses the owner's copy.
- **From the app:** when an install ends, the app hands the backup to any app through the share sheet. It shows the kept backup (stock version, checked) and offers "Restore original firmware"; with no copy, it says so and takes a saved file through the same check. Connected to a server, the app will later show the backup, copy it to the server and offer "Restore stock" through the same `wellfed-client` steps.
- **Why not keep it on the feeder.** It fits only in bank 1, which means running from bank 0 for good with no known-good fallback, and the first update into bank 1 would destroy it anyway. No other 1.3 MiB of flash is established as free.
- **Why this route.** These are your own bytes, from your own device, so there is no redistribution question.

**A second route: the vendor's update service.** The vendor app asks the cloud for a device's update metadata (private evidence E12), so an installer could fetch the official package with the user's own account. That depends on the vendor's service, and may offer only the latest version.

**Rolling back.** Our firmware installs a stock image as a signed update (`wellfed-bench ota --review <stock review>`). The bank-0 build writes it into bank 1, and stock's own KM0 then runs, so our trial counter never counts it. `wellfed restore-stock` and the app do the same from the owner's backup, through one driver, `wellfed_install::restore`:

- **The key.** A stock offer always needs a downgrade statement bound to the feeder's identity, the manifest's digest and the floor, so nothing can be signed ahead of time. The restore makes an update key when it starts, enrolls it (you press Dispense, the physical consent) and signs on the phone or computer. `--key` on the CLI gives a key the feeder already trusts instead, for the bench, and enrolls nothing.
- **The key's file** sits beside the backup only until the restore ends. A run within the hour after one that enrolled but didn't offer finds the key listed and asks for no second press; an older file is removed and a new key enrolled with a new press, since a press consents to this restore, not one days later. The file is deleted before the offer goes (the manifest and statement already signed in memory), so a killed run can't offer again without a press, and a feeder that never answered costs a second press. It also goes once the feeder says the key isn't enrolled, and in the app when you leave the screen. A spent key stays listed with its private half gone, signing nothing but taking a client slot until removed.
- `wellfed_signing::Frozen::stock` signs it as the stock image, at a version above the feeder's floor (`--version` overrides that). Only the pinned container passes, as for a stock review.
- The push is the installer's own, from either bank. From bank 1, the normal case after promotion, bank 1 gives way and bank 0 writes and activates stock. Both banks read one store and one list of enrolled keys, so bank 0 takes the same offer.
- Nothing re-offers once the push has returned; within it, the shared installer offers again only where the protocol needs it, as after bank 1 gives way.
- A refusal ends the restore. A feeder that stops answering after the offer leaves the result **uncertain**, since it may already run stock, which serves no device API. That is reported as such, and never re-offered.

**What a rollback keeps and leaves behind:**

- **Stock's settings stay.** Our firmware never writes `0x3e8000`–`0x3f0000`, so stock comes back provisioned, still naming the installer's broker and retrying forever. Hold Reset for about 5 seconds and set it up in the vendor's app.
- **Our storage keeps only what belongs to the feeder, not its owner.** In `0x340000`–`0x372000`, which stock doesn't read, the journal, the lifetime statistics, the portion calibration, the generator's seed and the feeder's identity stay. The feeder may change hands while it runs stock, so nothing of the previous owner's does. Bank 0 leaves a handover mark when it activates stock. The next Wellfed boot (a later install, or the loader's fallback because stock didn't boot) clears the settings, the stored Wi-Fi, every plan, the event history, the feed traces, the time zone and NTP server, the meal-call recording, the uplink store's sectors and every client, resets the update floor and forgets the boot confirmation, before anything reads them. The history starts again under a new epoch, as after a factory reset. On a fallback that is fine, since the owner's signed downgrade already gave up the floor. So reinstalling the same build runs a real trial and raises the floor again.
  - Only bank 0 forgets the confirmation: on bank 1 the build's next boot would run a trial with no Wi-Fi left to pass it.
  - Stock's activation record stays until the next install overwrites it; a manifest still pending is cleared.
  - A step that fails, or a client or update store that can't be read (it may still hold clients or a floor), leaves the mark, and the next boot tries again. Until then that boot serves no client from a client store it couldn't read.
  - The identity stays, since the owner's backup is filed by it. So a later consumer install behaves like a first one: it starts Bluetooth setup by itself with no meals, and a phone that was the owner pairs again.
  - An image with build Wi-Fi (a bench image) pairs through the recovery window, so it clears none of the stores above, but still resets the floor, forgets the confirmation and erases the mark.
  - A factory reset pending when stock activates leaves no mark, so the floor and the confirmation stay. That is a known limit: only a promotion signed above that floor gets in, which the installer's own key can sign.
- **The mark is bank 0's build's to write.** A bank 0 older than this feature writes no mark, so the feeder keeps its people until a factory reset, and a later install finds them as they were.
- **A fallback feeds nothing.** With no plans and no network until it is set up again, a fallen-back Wellfed runs no meal by itself. Plans stock still holds stay in stock's sectors, for stock if it comes back; after a Reset hold there are none.
- **A cut stays recoverable.** The record goes first, then the mark, then the bank 1 marker. A cut before the mark leaves this Wellfed with its owner. A cut after it leaves a boot that clears itself into Bluetooth setup, with no meals, whether or not stock went in, and the clearing repeats after a cut because the mark goes last. The record kept through the handover refuses the same manifest as failed. That protection is narrow: it can't tell whether stock booted, so it also refuses a retry after stock ran well, and it stops only that exact manifest; a restore signed with a fresh key has a new digest, and a promotion overwrites the record. A run cut after give-way or mid-write leaves its manifest pending, and the next restore cancels it once before its own offer. An install running now stops that restore, and so does a cancel in any doubt.
- **Bank 0 keeps our build** as a fallback, which the loader never chooses while stock in bank 1 is valid.
- **Stock's OTA status sector** (`0x3ef000`) may differ from the original after the install.
- **No automatic fallback from a stock that hangs.** The loader falls back to bank 0 only when bank 1 is invalid, and Wellfed is then in Bluetooth setup. If a valid stock fails to boot from bank 1, recovery is UART only (`wellfed-bench stock-prepare` / `stock-execute`).

## Where the installer lives

The decisions are in [product architecture](product-architecture.md). For the installer:

- **The steps live in `wellfed-client`:** the setup frame, the single stock request, progress tracking, the backup gate and the backup receiver. The Android app runs them through `wellfed-install`, which supplies the image server, the broker and the sockets, and serves the image from a foreground service ([the app](../../src/app/README.md#the-install)).
- **The CLI** keeps stock's image and goes back to it (`wellfed backup`, `wellfed restore-stock`), but doesn't install from stock. `wellfed-stock` holds the stock protocol, and its binary stays the bench's tool for reading stock and closing a request.
- **The server is not an installer.** It links neither `wellfed-stock` nor `wellfed-install`, and only installs releases on a feeder already running Wellfed.
- **Afterwards** the feeder is handed to the pairing flow. The app talks to it directly on the LAN.

## Evidence

- **Observed** (October 23, 2026, research unit):
  - `fw40-stock-ota-05`: stock 1.1.15 installed a replacement build through its own MQTT-triggered HTTP updater, with no UART write. It downloaded the image in about 40 s, checked its byte-sum and wrote it to bank 0, and the loader booted the build from bank 0.
  - `fw40-stock-ota-06-promote`: the build on bank 0 installed the same build, signed, into bank 1, where it confirmed. Promotion took about 3.5 min with the updater of the time.
  - `fw40-stock-ota-07`: the whole sequence in one command, with a fresh build: install, health from bank 0, promotion to bank 1.
  - `fw41-roundtrip-01`: to stock over our updater and back over stock's, with no UART write and nobody at the bench.
  - The requirements table came from earlier tries: `fw40-stock-ota-02` (the image port blocked, error 2003), `fw40-stock-ota-03` (the 60 s telemetry bound) and `fw40-stock-ota-04` (the stale version report).
- **Observed:** stock 1.1.15 dispensing locally through `wellfed-stock`. Stock's client ID equals the device ID in its topics, from a captured CONNECT.
- **Recovered:** stock's setup protocol, frame and re-provisioning guard, from the stock image and the vendor app. The frame and the reset gesture have since run on the research unit (below).
- **Recovered** (October 28, read-only static pass over the 1.1.15 image): the broker kept as a DNS name in `0x3e8000` on plaintext 1883 with no TLS or pinning; setup mode reached only through the Reset hold, whose factory restore erases the plans; the plan reply's fields.
- **Observed** (October 23, `fw40-stock-ota-07-uart-01`): stock looking its broker name up through the network's resolver and then a public one, and a set-up stock connecting with no `BINDING`.
- **Observed** (October 30): the install run from the app on the research unit: a real Reset hold, the setup frame, stock's install, Bluetooth setup, the kept backup, the promotion and confirmation on bank 1. After a restore it ran again at the same version. A command-line installer, since removed, ran from bank 0 with a virtual panel the same day: Bluetooth setup with a new client key, the join, the kept backup, the promotion and confirmation.
- **Observed** (October 30, unattended): the install from stock as a bench server job, since removed, with a virtual panel, on a route that reached stock by DNS, since removed.
- **Observed** (October 31): the install run by that command-line installer from a real Reset hold: the frame, stock's install into bank 0, and the rest of the run, resumed once from bank 0 after a setup code timed out. In a second run the first code expired too, and a second code offered in the same run worked. A vendor app setup after a restore also completed, with stock on the vendor's cloud.
- **Observed** (October 30): the restore, run on the research unit from the app from bank 0 and from bank 1, and by the bench job from bank 1.
- **Observed** (October 31): a manual `nmcli dev wifi connect` to stock's setup network timed out, with NetworkManager reporting `acd conflict` on the offered 192.168.1.100. After `nmcli con modify` set `ipv4.dad-timeout 0` and `connection.autoconnect no` on that profile, it joined and the frame reached stock.
- **Modeled** (November 13): the install from a computer, with the networks switched by hand, host-tested with fakes and the bridge's frame binding.
- **Modeled** (October 26): the installer, trust on first use, the setup-frame encoder, the backup gate, the stock backup read and `wellfed restore-stock`, host-tested and simulated on synthetic bytes.
