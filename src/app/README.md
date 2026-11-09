# The Android app

This is Wellfed's phone app. It installs Wellfed on a feeder that runs stock, sets up a feeder over Bluetooth, finds it on the LAN, and does everything the device API offers. It is for owners; contributors start with the layout below.

So far it does:

- **Install:** from stock 1.1.15, through the feeder's own Wi-Fi after a Reset hold. This app is the only installer. It backs up stock and makes Wellfed permanent, and the meals are set up again in Wellfed afterwards ([consumer installation](../../docs/firmware/consumer-installation.md)).
- **Setup:** BLE setup of a new or reset feeder. You hold Lock, then Mic, give the Wi-Fi and the names, pick the feeder from those nearby in setup mode, then key in the 6-press code.
- **Recovery:** taking a feeder already on your network as its owner again. You hold Lock, then Dispense, and key in the same kind of code.
- **Home:** a card per feeder (compact when there are several), with whether it can feed now, the food, power, panel lock, Wi-Fi signal and anything that needs attention.
- **Feed now:** a feed of 1 to 48 portions, followed to its result.
- **Today:** what the feeder is doing, the day's meals, the lock, the desiccant and a restart.
- **Schedule:** the meals, each edited against the generation it read, and the meal call.
- **History:** what happened, by kind, and one feed's detail.
- **Settings:** lights, sounds and their hours, reminders, Wi-Fi, time zone, where the time comes from, and the portion size.
- **People:** who a feeder lists, removing them, invites as a code or a link, and joining with one.
- **Updates:** the image this APK carries, offered once and followed to its end. The run outlives the screen, and an end only the feeder can settle stays until it answers that nothing is under way. Restoring the original firmware opens from here.
- **Diagnostics:** the feeder's counters, motor traces and the live log.
- **Alerts at home** (Android): a notification when a feeder says something is wrong, by level, and each feeder's open alerts behind the bell on its page. App settings has the alert settings: what to mute, by level or kind, and "instant while home". See [alerts at home](#alerts-at-home).
- **App settings:** appearance, this phone's name, alerts, kept backups and About.
- **Home server** (optional, from App settings): add one by its address or as it announces itself on the LAN (`_wellfed-server._tcp`), log in with a name and password (the phone mints its own token; the password is kept nowhere), and log out or remove it. Either way the phone keeps its own keys on its feeders; deleting the account on the server revokes them. With a server, each feeder shows the way a request goes now, a feeder this phone paired that the server doesn't list is offered for adoption, History reaches the full history the server kept, and each feeder has a food view with its refills. Pets, their weights, intake and the feeders they eat from, and the home's foods, are under Pets and foods. A viewer's account has no Feed. Until a feeder's portion size is set, its food view and a pet's intake count portions, with no grams, calories, food left or target.
- **Restore:** the vendor's firmware back on the feeder, from the backup the install kept or a copy you saved. You press Dispense on the feeder, and it goes across as an update. Then stock runs, which this app cannot talk to.

People and the audit log are not on the phone yet ([roadmap](../../docs/roadmap.md)). Meal reminders are a setting on the feeder. Alerts reach the phone only at home: away from home, alerts need a server or a relay, which are planned ([deferred](../../docs/firmware/product-architecture.md#deferred)).

`cargo xtask check-repository` fails when an `Op` has no client in the app, or a request the bridge offers has no screen.

## How it is built

| Part | Where | Does |
| --- | --- | --- |
| Screens, state, platform IO | `lib/` (Dart) | BLE through `flutter_reactive_ble`, NSD through `nsd` on Android, the client secret in `flutter_secure_storage`, alerts through `flutter_local_notifications`, saving the backup through `share_plus` |
| Background work | `android/app/src/main/kotlin/` (Kotlin) | The periodic catch-up through WorkManager, the one foreground service for runs and instant alerts, and the engine they run Dart in |
| `wellfed-app-bridge` | `rust/` (Rust) | The client key, the setup and recovery sessions (`wellfed_client::setup`), device API sessions (`wellfed_connect`), the install (`wellfed_install`), the app's file |
| Generated | `lib/src/rust/`, `rust/src/frb_generated.rs` | The Dart API and types, from `flutter_rust_bridge_codegen generate` |
| Theme | `lib/theme/tokens.g.dart` | Colors and sizes from [`assets/brand/tokens.json`](../../assets/brand/tokens.json) |

- **The Rust half checks what the feeder checks.** Dart moves bytes and shows what the session says. A feed goes out once; a feed whose answer never came is shown as "may have run" and never sent again.
- **Every feeder request goes through the path router** (`rust/src/route/`, `PathRouter` in Dart), with a server or without one. It picks the server first and the phone's own session when the server can't reach the feeder ([two paths, one feed](../../docs/server/README.md#two-paths-one-feed)). Every write is written down in `pending.json`, beside the app's file, before it goes, and is sent once. One that may have run stays pinned to its path until that path settles it, and a check only reads. A new feed waits until an uncertain one is settled or put away: closing the feed sheet that asks "Did it feed?" puts it away, and one an earlier run left is asked about before the next feed. Each session is attached to the router before it runs, and a session offers Dart no request of its own, so every request and feed a screen sends goes through the router. Outside it, each a flow of its own that never sends a feed: the reads a session makes for the card and the history, its clock sync when it opens (after pairing, or when the feeder's clock is unset), a firmware update, the restore of stock, and the install's, setup's and join's own sessions. A pins file that doesn't read is set aside and holds every feed back until the owner has checked the bowls, and one the phone's storage can't open blocks every request instead of sending writes unpinned; a router left behind by a restarted engine in the same process is taken over by the next one and sends nothing again. A change other than a feed that never said how it ended shows on the feeder's screen until a check settles it or the owner puts it away, and a feed's own answer reaches a screen made again while it was out by the feed's key.
- **With a server, its list is the feeders.** The phone keeps its own key on the feeders it paired and uses it when the server can't reach one; a feeder it holds no key on goes through the server only, and one it paired that the server doesn't list is offered for adoption. The last list is kept in the app's file, so a cold start with the server down still reaches the feeders directly. A server that refuses the phone's token (revoked, or the account disabled), or one whose token the phone no longer holds, stops the direct path too, and that holds while the server is down, until the owner logs in again or removes the server. The account is kept with the list, and the router is told of both as it opens, before any session or token read, so a cold start keeps a viewer or an account with an expiry off the phone's own keys; with no account kept, none are used until the server answers. A change to the account the router reads while the app runs is kept for the next start. One made while the app was closed, with the server down when it starts, can't be known on the phone: only the server revoking the phone's key on the feeder stops it there. An adoption whose answer was lost is kept beside the pins, so a restart within its grace reads the server's list rather than making a second invite. Removing the server, or logging out, keeps the phone's own keys.
- **Images** are links to the committed renders in `assets/renders/`, and the launcher icon links to `assets/brand/android/`.
- **The client secret never leaves the phone.** Android backup is off, so a new phone pairs again.
- **A session gives a feeder this phone's clock** right after pairing, and whenever the feeder's clock is not set, as `wellfed time` does.
- **Every socket follows the Wi-Fi network.** Android keeps mobile data as the default network while the Wi-Fi has no internet, so `MainActivity` binds the app to Wi-Fi. A VPN that apps cannot bypass (Tailscale, for one) refuses that: exclude Wellfed from it.

## Alerts at home

The feeder decides every alert ([alerts](../../docs/firmware/alerts.md)). The phone catches up with each feeder's table, posts each alert once, takes the notification off when the feeder clears it, and decides one thing itself: that it can't reach a feeder while it is home. Without a server or a relay this works only at home, and the alert settings page says so first.

- **Rust decides, Dart posts.** A catch-up (`rust/src/catch_up.rs`) opens a session that can ask for the status and the alert table and nothing else, so it never sends a feed or a write. It reads the table only when the status shows it moved since the last one applied: the cursor is the ring's epoch, the boot count and the table's generation. `wellfed_client::alerts` compares the table with what the phone tracks by each alert's key, and says what to post and what to take off. While the feeder can't vouch that an absent alert ended (`settling`, or `more` for a report), its notification stays, but not for good: `settling` is one flag for the whole table and can stay set while the clock is unknown, so a condition comes off after 2 hours absent (counted from the first read with a valid clock for the desiccant and stuck plans, which wait for one), and a report once it is past the feeder's 48 hours, by its own time or, with none, by how long it has been absent ([alerts](../../docs/firmware/alerts.md)). A posted alert whose kind changes says so again (an uncertain meal found failed); the same words again under its id don't buzz. Dart posts and removes notifications, then confirms each change, and one not confirmed is handed over again at the next catch-up under the same id.
- **Mutes and dismissals stay on the phone.** A muted kind or level is listed and never posted; muting takes its notifications off. Dismissing hides an alert on this phone until the feeder clears it; the feeder is never told. A feeder the phone forgets takes its alerts with it, and a catch-up that ends after that brings none back and takes off anything it put up.
- **Asking to post.** The app asks Android once, when Home first shows a feeder, and remembers that only once the request has run. After that the owner asks, from the alert settings or a feeder's alerts, which say when notifications are off; once Android no longer shows its dialog, that opens the app's notification settings. Once Android allows it, alerts still open are posted; ones that ended meanwhile are not.
- **Unreachable is the phone's only judgement.** A catch-up first tries where the feeder last answered; if nothing answers, NSD looks for it and a last try goes there. Only that last try counts, and only on the network where the feeder last answered, recognised by its gateway, subnet, DNS servers and search domains with no location permission. Three failures in a row, at least 10 minutes from the first, open it; the next answer clears it. Away from home nothing counts.
- **When it runs.** WorkManager runs a catch-up about every 15 minutes (`CatchUpWorker`), and far less often while the phone sleeps. "Instant while home" runs one each minute from the foreground service, with a wake lock and a Wi-Fi lock, which costs battery. The service stops when the phone restarts or Android stops it, and Android lets only a visible app start it, so instant mode comes back when the app opens; the settings page says when it is on but not running, and starts it again from there. Installs, updates and restores keep catch-ups off every feeder while they run: the engine holds them for each run's Dart steps, and each run's own thread holds them for as long as it lives, so an update's restart or an install's listener, which outlive a swiped-away app, still keep catch-ups off. Swiping the app away clears only the engine's holds.
- **Background engines never load the Bluetooth plugin.** A catch-up with no screen runs Dart in a Flutter engine of its own (`BackgroundEngine`), built with no automatic plugin registration and given only the notification, secure storage and NSD plugins. The Flutter plugins for background work and foreground services start their engine with every plugin attached, and `flutter_reactive_ble` keeps its controller and its open links in statics: a second engine attaching it takes the app's scan and links over, and its shutdown disconnects them, so a catch-up during Bluetooth setup would cut the setup. That is why WorkManager and the service are this app's own Kotlin, not plugins.
- **Licenses.** `androidx.work` is Apache-2.0 and `flutter_local_notifications` BSD-3-Clause. `flutter_local_notifications` needs core library desugaring for its scheduling API, which this app doesn't use, and that adds `desugar_jdk_libs` (GPL-2.0 with the Classpath Exception, so linking it imposes nothing on the app). No check in `make check-app` reads the licenses of Dart or Gradle dependencies; cargo-deny covers Rust only.

## The restore

`wellfed_install::restore` drives it, the same as `wellfed restore-stock`, on a thread of its own (`rust/src/restore.rs`), with the install's asks and events on its stream and an end of its own. `platform/restore_ports.dart` is what its screens reach outside Dart.

- **The key lasts only as long as the restore.** It is made when the restore starts, enrolled with a Dispense press, and removed before an offer goes out, or when you leave the screen. A retry from the same screen after an enrollment that went through needs no second press.
- **An uncertain restore is never sent again.** Once the offer may be out, the screen has no way back, and a feeder that stops answering ends on "We can't tell if it went through" with no retry.
- **A picked file takes the same checks as the kept backup:** `StockBackup::open`, then the signer's own pin.

## The install

`wellfed_install::run` drives the whole run. Its future is not `Send`, so `rust/src/install.rs` runs it on a thread of its own, with a current-thread tokio runtime. Stock's broker and image server are tasks on that runtime, in this process, on unprivileged ports.

- **Dart moves things and draws them.** What the run needs from outside (a person's answer, a radio write, an NSD lookup) comes to Dart as an `Ask` on one stream, and Dart answers by id. Progress comes as generated `InstallEvent`s. `state/install/` follows them, and `screens/install/` draws a screen per stage.
- **Tests fake the edge, not the flow.** `platform/install_ports.dart` and `platform/pairing_ports.dart` are everything these screens reach outside Dart, so `test/` runs every state with no Rust and no phone. The bridge's own tests run the thread and the adapters against the simulated feeder.
- **Errors on a stream are values.** flutter_rust_bridge decodes a stream's errors as `AnyhowException` whatever Rust encoded, so nothing sends a typed error with `add_error`.
- **An uncertain request is never sent again.** The run ends, and a listener takes over: stock's server with no trial enabled, for up to 30 minutes. Stock reporting its version means it did not install, and the ledger may then close the request. The replacement answering on the network, or advertising Bluetooth setup, means it did, and a new run picks up from bank 0.
- **A wrong Wi-Fi is not a dead end.** A feeder that took the Wi-Fi over Bluetooth and never joined it is waited for, then the owner can enter the Wi-Fi again. A run from bank 0 then goes through Bluetooth setup again, with the feeder put back in setup mode by its buttons, and asks stock nothing. Setup on any feeder but the one the install paired with stops the run, so the backup and the promotion never move to another. A consumer image from stock with no Wi-Fi to give is refused in Rust before the run starts.
- **A foreground service keeps the process alive** while a run or a listener needs the broker (`KeepAliveService`, shared with instant alerts). The notification says so. Each run holds it under its own key (`platform/run_holds.dart`), so an update ending never lets go of a restore's hold. The screen stays on for as long, since a locked phone dozes and its sockets stall. Swiping the app away ends the run, which is why the screen asks you to keep it open.
- **Leaving does not strand a feeder.** A run records its feeder as soon as it pairs, and each feeder's request stays in its own ledger. The first screen reads both: a paired feeder can be picked up from bank 0, and a request with no settled outcome offers the listener and a look over Bluetooth. Neither ever sends the request again.

### Which builds can install

Stock's install goes only from a build whose sources can be reviewed, as with every other caller. A debug APK built from a clean commit can install. One built from a dirty tree builds and runs, and its first install screen says it cannot send an install. Commit first.

### The image it installs

The APK carries one image, the consumer build signed with the key the image trusts, so nothing is downloaded at install time. Installing, the phone never holds a key a feeder trusts: the update manifest is signed elsewhere, and the run promotes it as signed.

The image is a build-time input. `make app-installer-input` copies a review and its signed manifest into `assets/installer/`, which Git ignores, and prints the pin to build with:

```sh
make app-installer-input REVIEW=private/<review> MANIFEST=private/<version>.manifest
WELLFED_INSTALLER_PIN=<what it printed, with the version> flutter build apk --debug
```

The pin is compiled in: the digest of the review (which binds the image and its build manifest), the digest of the manifest, and the manifest's version. At startup the bridge checks the unpacked files against it, and against the review's own digests, as the image server does. A mismatch, or an image with no pin, is refused on the first screen.

A release build's pin goes in the bridge as `RELEASE_PIN`, which the environment cannot override. It is empty until a release image exists, since only the offline release key can sign one. Until then a build with no input says it has no firmware to install.

### What the app keeps

The app keeps three files in its private storage, each `{"version": n, "data": …}` (`rust/src/stored.rs`):

| File | Holds |
| --- | --- |
| `wellfed.json` | The phone's name for itself, the feeders it knows, the theme, and the server it logged in to with that server's last feeder list |
| `pending.json` | The writes that may have run, the direct feeds running, and the adoptions whose answer was lost. A feed keeps its portions and any other write its kind: no device API type, so a change to the device API never changes this file |
| `alerts.json` | Each feeder's alert watch (the cursor its table was read to, the notices open and those still to post or take off, where the phone last reached it and on which network), the mutes and "instant while home". In a file of its own, so a bad one can never keep the app from its feeders. A set-aside one is named in the file until the owner reads the notice, since a background catch-up may be the first to open it, and every alert notification that no watch in the new file knows comes off, since nothing else would take it off. Its version 1 shape is pinned by `rust/tests/fixtures/alerts-v1.json` |

The client key and a server token's secret are in secure storage, and the stock backups in their own folder, so none of them is in these files.

A build that changes a file's shape bumps its version and keeps the reader for the version before, converting. A file the start can't read (damaged, from a build too old or too new) is kept as `<name>.damaged-<n>` beside it, never deleted, and replaced with an empty one, and the app says so on a screen before Home. For `pending.json`, a lost pin may have been a feed that ran, so every feed waits until the owner says the bowls are checked; the hold is written into the new file, so a restart keeps it. A file the phone's storage can't read at all fails the start, which says so rather than guess.

A set-aside `wellfed.json` loses the server's link, so the start also deletes the server token's secret from secure storage: nothing could use it again. The token itself stays on the server, since revoking it from the phone needs the link; revoke it on the server (`/tokens`, [as for a lost phone](../../docs/server/running.md)). The feeders still know the phone, since its key is kept: the notice sends the owner back to each through Recover a feeder at the panel (a feeder that still lists the phone answers "already enrolled" and logs it; one that revoked it enrolls it again), or through an invite. Without the panel or an invite, the app has no way to find them again.

The versions start at 1 with this build, and an earlier build's unversioned files are set aside like any other. No phone outside the bench ran one.

## Toolchain

The versions this was built with:

- Flutter 3.47.8 (stable), for example in `~/.local/flutter`.
- The Android SDK command-line tools, with platforms 37 (the app) and 36 (the `rust_builder` plugin), build-tools 36.0.0, and the NDK that the pinned Flutter names as `flutter.ndkVersion`. Run `flutter config --android-sdk <path>`.
- The Rust targets: `rustup target add --toolchain 1.99.1 aarch64-linux-android x86_64-linux-android`.
- `cargo install flutter_rust_bridge_codegen --version 2.13.0 --locked`.
- For the Linux desktop build: clang, CMake, Ninja, pkg-config, and the GTK 3 and libsecret development files (`gtk3` and `libsecret` on Arch, `libgtk-3-dev` and `libsecret-1-dev` on Debian), then `flutter config --enable-linux-desktop`.
- For the macOS desktop build: Xcode, CocoaPods with `LANG=en_US.UTF-8` in the shell, and `rustup target add --toolchain 1.99.1 aarch64-apple-darwin x86_64-apple-darwin` (a release build holds both).

Then `make check-app` from the repository root regenerates the bindings and checks they are unchanged, formats, analyzes, runs the widget tests and builds a debug APK, and on a Linux or macOS host that desktop's app too. The bridge's own tests run in `make check`, against the simulated feeder.

## The Linux desktop app

It is the same app, started with `flutter run -d linux`, or built with `flutter build linux` into `build/linux/x64/release/bundle/`. So far it starts and finds feeders on the LAN. Recovery over the LAN needs nothing more, but has not been walked on Linux yet. It cannot yet set a feeder up over Bluetooth or install, which need a Bluetooth plugin with a Linux side and joining the feeder's own Wi-Fi through NetworkManager. Its install, firmware update and restore screens refuse, saying why: updates and restores keep running through Android's foreground service, which a desktop does not have. Alerts are Android's alone too: a desktop has no background catch-up.

- **Discovery goes through Rust.** `nsd` has no Linux side, so `platform/discovery.dart` browses through the bridge's `FeederBrowse`, over the same `wellfed_connect::Browse` the CLI locates feeders with. Android keeps NsdManager, which holds the multicast lock a phone needs.
- **Nothing asks for permissions.** BlueZ and NetworkManager answer any process of the logged-in user, so `AppPhonePorts` reports everything granted, and the computer's name is its hostname.

## The macOS desktop app

It is the same app, started with `flutter run -d macos`, or built with `flutter build macos` into `build/macos/Build/Products/Release/Wellfed.app`. It needs macOS 12 or later, the oldest that `file_selector` supports. On a Mac it has found the simulator on the LAN by mDNS and recovered it. Integration tests then drove it against the simulator, in simulation only: a feed, meals added, edited and deleted, a setting, the time zone, the portion size, History with a button feed arriving, an invite made and a second key joining by its link, a meal call sent from a WAV the test handed over and played, and a restart asked through its confirmation dialog (the fault screen's button was not reached). Bluetooth setup has not been tried on a Mac: `flutter_reactive_ble` has a macOS side, but nothing has proven it. Both desktops refuse the install, since it joins the feeder's own Wi-Fi, which only the Android app can do. They refuse firmware updates and restoring the original firmware too, since those keep running through Android's foreground service, and show no alerts.

- **It is signed ad hoc, outside the sandbox.** Building it needs no Apple developer account. The device and network entitlements in `macos/Runner/` are inert outside the sandbox and without the hardened runtime, which this build does not enable.
- **Discovery goes through Rust, as on Linux.** Feeders come through the bridge's `FeederBrowse`. The desktops use no `nsd`, so an owner's server is entered by its address. The first browse may make macOS ask whether Wellfed can find devices on your local network: allow it.
- **The secrets are in the login keychain.** The data protection keychain needs a team ID, which an ad hoc build lacks, so `platform/secure_storage.dart` turns it off. The key survives a restart of the same build. The keychain knows an ad hoc build by its hash, so after each rebuild macOS asks whether Wellfed may read its own key, and the window stays blank until you answer. If you deny it, the app cannot read its key and stays blank: quit it, open it again and allow.
- **Nothing asks for permissions up front.** `permission_handler` has no macOS side, and macOS asks on first use instead. Nothing asks for location yet, so the Wi-Fi's name is never filled in for you. The computer's name is its hostname, without the `.local` a Mac adds.
- **Shortcuts use ⌘.** ⌘1 to ⌘9, ⌘0 and ⌘N do what Ctrl does on Linux. ⌘, belongs to the menu bar's Settings…, so the window does not bind it as well.
- **The menu bar is the Mac's own.** Settings… (⌘,) in the Wellfed menu opens the app's settings: the runner (`macos/Runner/AppDelegate.swift`) sends the pick to Dart over the `wellfed/menu` channel (`platform/menu.dart`). That has been seen in the running app. Its tests also cover that a dialog open over the window keeps it, and that a second pick stacks no second page. File › Close (⌘W) is wired to close the window, which quits the app, but nobody has pressed it yet. About Wellfed is wired to the standard panel, which has not been opened yet.
- **Links go through the clipboard.** A Mac's share menu opens from a corner of the window, so an invite's link and the live log are copied instead. Joining takes the pasted link, and the camera opens only when asked, on a Mac and not on Linux, which `mobile_scanner` has no side for.
- **A backup would be saved through a save dialog** in place of the share sheet. That has not been tried: a backup exists only after an install, and the desktops refuse the install.
- **The microphone is asked for on the first recording.** The app expects macOS to ask then, but the prompt has not been seen: in the runs so far, all started by `flutter test`, macOS refused the microphone and the camera without asking. If it is refused, the meal call says where to allow it (System Settings › Privacy & Security › Microphone). A WAV file works in its place.
- **https should need no extra wiring.** `rustls-platform-verifier` checks a certificate through Security.framework, with no setup call like Android's JNI hook. It has been checked only against public sites: one with a valid certificate was reached, and a self-signed and an expired one were refused. A login over plain HTTP to a local `wellfed-server` reached its account; no login over https has been tried on a Mac.

## Change the bridge

1. Edit `rust/src/api/`.
2. Run `flutter_rust_bridge_codegen generate` here.
3. Commit the regenerated `lib/src/rust/` and `rust/src/frb_generated.rs` with the Rust change.

After a change to `tokens.json`, run `UPDATE_BINDINGS=1 cargo test -p wellfed-app-bridge --test tokens` and commit `lib/theme/tokens.g.dart`.

## Run it against the simulator

The simulator has no Bluetooth, so use recovery:

1. Start it on the LAN, with your computer's LAN address:

   ```sh
   cargo run -p wellfed-feeder-sim -- --state /tmp/sim --listen 0.0.0.0:4747 --mdns 192.168.1.10
   ```

2. Install the app: `flutter run` with the phone on the same network.
3. In the app, choose **Recover a feeder**, name the phone, and pick the feeder.
4. At the simulator's prompt, open the window with `hold lock`, then `hold lock+dispense`.
5. When the app shows the code, type it at the prompt as positions: `press 1 3 2 …` (1 Lock, 2 Dispense, 3 Mic, 4 Reset).

Home then shows the simulated feeder, and **Feed now** runs a feed in the simulator. Stop the simulator with `quit` when you are done.
