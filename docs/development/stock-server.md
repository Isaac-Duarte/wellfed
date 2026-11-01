# Stock server

`wellfed-stock` is the server a feeder running stock 1.1.15 installs the replacement from. This guide says how it is built and how to run its binary at the bench. It is for developers; the owner's installer, `wellfed install`, runs it for you.

The server exists only for stock. It holds an embedded authenticated broker, the stock protocol as far as an install needs it, the image server stock downloads from, and a bearer-authenticated loopback API. The replacement firmware speaks no MQTT. Its updates and status go over its [device API](../firmware/device-api.md), which `wellfed-bench` and the owner's `wellfed` CLI use directly.

## How the crate is laid out

The [`wellfed-stock`](../../src/tools/wellfed-stock) crate is both the library and the binary.

- **The library** holds everything stock speaks: its MQTT protocol, and the broker with stock's two identities (the stock device and the server) and the CONNECT normalization in front of it. It also holds the server's MQTT session with stock, the loopback API, the setup frame, the image server and the install-trial ledger. `server::Server` runs the broker, the session and (optionally) the API as tasks in the caller's process, and `ota::image::start` the image server; the caller owns and stops them.
- **The binary** adds the command line: `init-installer`, `serve`, `status`, `setup-send`, `ota-trial-resolve` and `ota-trial-answered`. Its `serve` is that same entry point, with the trial disabled, so it can never ask stock to install. The binary and the neighbour lookup sit behind the default `bench` feature.

The one crate that links the library is the install driver, [`wellfed-install`](../../src/client/wellfed-install), without the `bench` feature: `wellfed install` and the Android app run stock's side in their own process. `cargo xtask check-repository` refuses any other crate that depends on `wellfed-stock`.

The server does not control feeding on stock. The product uses stock only to install from and roll back to.

## Rules left from an older build

An older build of this tool could redirect the feeder's broker traffic with `nft`, and no tool here applies or removes those rules any more. If `sudo nft list ruleset` shows the table `inet wellfed_local`, or rules tagged `wellfed-local-broker` in `inet computer_security`, remove them by hand: `sudo nft delete table inet wellfed_local`, and `sudo nft delete rule inet computer_security <chain> handle <handle>` for each tagged rule. This changes the host's firewall, so it is the owner's to run.

## An installer's server: trust on first use

An install needs no capture. `init-installer` writes a config with fresh secrets and the broker address the setup frame names. It needs no device at all.

```sh
./target/debug/wellfed-stock init-installer --mqtt-listen 192.168.1.20:1883 --output private/installer.json
./target/debug/wellfed-stock serve --config private/installer.json --enroll-minutes 15
```

- **Enrollment.** With no device known, `serve` starts the broker first and waits. Within the window, it admits the first CONNECT that looks like stock's: a client ID stock's topics can carry (not the server's), with a username and password, from a known relayed address. It records that device beside the config, mode 0600, named for the config (`installer.json`'s is `installer.device.json`). The record holds the client ID (which is the device ID), username, password and the address it connected from. Then the rest of the server starts with that device.
- **Stock's first reports.** Stock sends its clock request, its start event and its version report as soon as it is admitted, and only heartbeats after. The broker keeps no message for a later subscriber, so it tells the server of the enrollment at once and holds the feeder's CONNACK until the server's session has subscribed (at most 3 s). Before, the server subscribed 0.7 s too late and the version report was lost (physical, October 30).
- **Pinned.** The record is created whole or not at all: written aside, then moved into place by a step that never replaces (`renameat2(RENAME_NOREPLACE)` on Linux and Android, a hard link elsewhere). So a reader never sees half a record, and a killed broker leaves none behind. A second feeder, a wrong password, or anyone after the window is refused, as with a captured config. A later `serve` needs no window and answers the enrolled feeder alone. To start over, stop the server and remove the record.
- **Contested.** Inside the window, any second stock-shaped identity, or a wrong password for the enrolled one, is noted in `installer.contested.json`, without its credentials. The status document's `enrollment` shows the device, its address and the contest. `POST /api/ota/trial` refuses (409) while the contest stands, because which feeder is the owner's can no longer be told. Only attempts inside the window count. After it closes, a second identity is refused but not flagged, so the installer's owner check of the device ID and address is then the only one.
- **One or the other.** A config that names a captured device and has an enrollment beside it is refused.
- **The image server** admits the enrolled feeder's address.
- **Stock's plans are left alone.** An installer's config holds no plan snapshot, so `GET_FEEDING_PLAN_EVENT` goes unanswered. Stock erases and rewrites its plan table on any reply that has a `plans` key, even an empty one. It does not check the code or the message ID, and it takes the message unasked. With no reply it keeps its table; it asks once and does not retry. An empty answer would erase the plans a rollback should find. The server never sends a plan message on its own: the commands it sends unasked are a closed set (`protocol::Sent`). (Static analysis of 1.1.15's KM4 image, with one UART capture of a stock install.)
- **Stock's clock is set only when no meal can come due.** After the Reset hold and setup frame, which erased its plans, the `NTP` reply sets it (`wellfed install --ssid`, and the app's route). With no frame, stock may still hold plans, so the reply leaves its clock as it reads; so does `wellfed-stock serve` with no captured table. Stock 1.1.15 refused an install with error 1214 while its clock was two days behind, likely for that reason.
  - **The log** says, for every `NTP` request, stock's `ts` (or `unknown`), the skew in seconds, and whether the clock was set.
  - **A captured nonempty table** holds the clock, with a warning: those plans may run any meals that came due in a jump. An empty one sets it.
- **No binding from a set-up stock.** A set-up stock sends no `BINDING` when it connects (observed). A stock fresh from the setup frame does, and an installer's config answers it with a generic reply built in the server, since it has no captured one ([saving the frame](../firmware/consumer-installation.md#stocks-setup-protocol)).
- **Its settings stay.** The server never sends `DEVICE_CONFIG_SYNC`, `WIFI_CHANGE_SERVICE` or the MQTT `RESTORE` (`protocol::Sent` is a closed set).

## The install ledger

Install trials are recorded beside the configuration, under an exclusive lock, in one ledger per device: `stock-ledger.<first 16 digits of the SHA-256 of the device ID>.json`. A captured config and an installer's in one directory share the ledger of the device they both name. So another feeder is not refused by this one's, and this one's open trial still holds back a second request however the device is reached, and after its trust is forgotten and given again.

- Each trial is persisted before its one `OTA_UPGRADE` goes out, and is never resent. A repeated request ID returns the stored record.
- A trial in flight when the server stops or loses the broker becomes uncertain.
- An uncertain or failed trial blocks the next until `ota-trial-resolve` closes it from fresh evidence. Resolving sends no command.
- A replacement install closes instead on `ota-trial-answered`, once the operator's client saw the served build answer its device API from bank 0 and stock has gone quiet ([below](#install-the-replacement)).

`setup-send` takes the [bench lease](../devices/plaf103/bench.md#bench-lease) for its one frame, and needs `--on-feeder-network <name>`: it refuses a name not starting `PL-AF103-`, but takes your word for it, unlike `wellfed install`, which checks. `serve` takes none: it only answers, beside the command that drives the feeder.

A server with the trial enabled starts only from a reviewable build: a clean commit of every input `build.rs` records. Those cover `wellfed-stock`, the crates it links, and the crates that decide and send the request in an installer (`wellfed-client`, `wellfed-install`). `server::Server::start` enforces it; tests set `WELLFED_ALLOW_UNREVIEWED_OTA_BUILD`.

## HTTP API

Write bodies are limited to 4096 bytes. Authenticate with the bearer token from the private configuration. The CLI handles it without putting the token in shell arguments.

| Route | Does |
| --- | --- |
| `GET /health` | Public process and build identity (`"service": "wellfed-stock"`) |
| `GET /api/status` | Authenticated stock telemetry, build identity, install trials and stock's OTA reports |
| `POST /api/ota/trial` | `{ "request_id": "UUID", "server_token": "UUID" }`: the stock-to-replacement install. Needs a server with the trial enabled, which `serve` never is, and a live image lease |
| `POST /api/ota/trial/resolve` | Explicit uncertain-trial reconciliation. Sends no command |
| `POST /api/ota/trial/answered` | `{ "request_id": "UUID", "build": "64 hex", "bank": 0, "note": "…" }`: the latest submitted or uncertain trial's served build answered the device API from bank 0 after it. Closes it as `replacement_healthy` and sends no command. 409 while stock still reports, or for another build or bank |

Trial dispatch needs a clean, identifiable source commit. Test-only bypasses are not a deployment procedure; see [fixtures](fixtures.md) and [recovery limits](../devices/plaf103/recovery.md).

## Install the replacement

`wellfed install` and the app drive this whole sequence in their own process.

1. Serve the frozen review. The image server serves its `image.bin` as `/ota/<token>/replacement.bin`, and does not command an update. It refuses an image that is not a build's, is larger than the stock footprint (1,376,288 bytes), lacks the loader marker, or does not match the review's `ota_sha256`. The review's layout audit is what establishes the container header and byte-sum stock checks. There is no `original.bin` beside it: the way back is the UART stock recovery. The lease records the served digest and the build's identity: the startup identity of the review's frozen `build-manifest.json`, which must match the review's `manifest_sha256`.

2. Keep the schedule snapshot empty or absent. A captured snapshot is replayed to stock's schedule queries, so a trial needs it empty. With no snapshot (an installer's config), the queries go unanswered and stock keeps its own plans.

3. Ask once. The trial needs the lease, announces target version `2.0.0` with the lease's digest, and records both in the ledger.

4. Close it on the replacement's answer (`ota-trial-answered`, or the installer's own call). Stock never reports after installing the build, and the build speaks no MQTT, so the operator's client reports instead. The report gives the build identity and bank the build's device API answered with after the submission, and a note of what it saw.

The server sends nothing, and closes only the latest submitted or uncertain trial, and only when both hold:

- **The answer names the served build on bank 0.** Its `build` equals the identity recorded at submission, and `bank` is 0. A record from before the identity was kept never closes this way.
- **Stock is not demonstrably running.** Its telemetry is not current (none for 90 s), and it has reported no version since the submission. A version report after the submission marks the trial for good (`stock_reported` in the ledger). Stock was still running then, so a dropped session that came back on 1.1.15 cannot be closed by an answer; only `ota-trial-resolve`'s version evidence resolves it. This fails safe: if stock also reports its version during a download, an install that did succeed waits for the resolution as before.

A proven session closes a trial too, with neither condition about stock. When a feeder this phone or computer paired from stock answers its own key's session with the trial's build, after the request, the installer records that answer beside the feeder's backups and closes the latest submitted or uncertain trial in the ledger: on any run that reads the feeder's status, a pick-up included, when a session opens from Home, and at a restore's activation. A trial no such session answered stays open.

Nothing a replacement reports ties it to stock's device ID, so that session cannot prove it is the feeder that took the request. A second feeder in setup on the same build could answer instead. So stock reporting after such a close reopens the trial as uncertain, for the version evidence above to resolve, unless a restore from this installer's records activated stock on that feeder first and marked the trial: the app's, `wellfed restore-stock`'s or the bench job's. The evidence's version report holds while stock stays in the session that sent it with its telemetry unbroken, since a stock still in that session cannot have installed, and it reports its version only around a connect; a dropped session or a telemetry gap voids it until a new one. A restore this ledger did not hear of, from another phone, waits for that evidence too.

A known limit: the mark trusts that the feeder restored is the feeder the answer came from. If a second feeder in setup answered and was then restored from this installer's records, the trial closes on it, and stock reporting on the first feeder does not reopen it.

The answer through the API is still the client's word. The server checks it against what it served, not against the device API itself. 
Operation with a nonempty plan, and stock's time-recovery behavior, are unvalidated; see [stock behavior](../devices/plaf103/stock/README.md).

## The replacement firmware (deleted roles)

The broker once served the replacement firmware too, as a role with its own credential and namespace (`firmware-credential`, health, requests and `ota-offer`). That role is deleted, with the firmware's MQTT transport. Builds take updates over the [device API](../firmware/ota-design.md#updates-over-the-device-api), which `wellfed-bench ota` pushes, and `wellfed-build` no longer reads `private/fw-mqtt.json`.

A configuration written while the role existed has a `firmware` section. `wellfed-stock` refuses a config with a field it doesn't know, so delete that section.

## Evidence

- **Observed:** local dispensing on stock 1.1.15 was demonstrated on the research unit, before stock control was removed from this server. The code is in Git history.
- **Modeled:** `tests/protocol.rs` loads a seeding config from a file and replays its table exactly as written, with calibration held, then leaves the request unanswered once `plans` is removed.
- **Observed (UART captures of 1.1.15, October 23 and 28, 2026, `fw40-stock-ota-05-uart-01` and `fp04-stock-idle-20261028n`):** stock's `NTP` request carries its own clock in `ts` (milliseconds, rounded to the second), and it asks once per connection. On October 28 its clock read October 26, the reply held calibration (`calibrationTag = 0`), and stock refused the install that followed with error 1214, with no download (`fpbatch-20261028n`). That the held clock caused the refusal is likely, not confirmed statically.
- **Modeled:** `clock.rs` unit-tests the rule (no table with stock's plans erased or unknown, an empty one and a nonempty one).
- **Modeled:** `tests/enroll_stack.rs` runs the real `serve`, which enrolls a simulated feeder with never-seen credentials, hears its version report, refuses a second identity and a guessed password, and after a restart without the window admits only the enrolled feeder. Not yet run against stock.
