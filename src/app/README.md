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
- **Alerts** (Android): a notification when a feeder says something is wrong, by level, and each feeder's open alerts behind the bell on its page, from the feeder itself while the phone is at home. App settings has the alert settings: what to mute, by level or kind, and "instant while home". See [alerts](#alerts).
- **App settings:** appearance, this phone's name, alerts, kept backups and About.
- **Restore:** the vendor's firmware back on the feeder, from the backup the install kept or a copy you saved. You press Dispense on the feeder, and it goes across as an update. Then stock runs, which this app cannot talk to.

The app talks to each feeder directly, on the home network; there is no server between them. Meal reminders are a setting on the feeder. Away from home, alerts don't reach the phone: there is no push service or relay ([deferred](../../docs/firmware/product-architecture.md#deferred)).

`cargo xtask check-repository` fails when an `Op` has no client in the app, or a request the bridge offers has no screen.

## How it is built

| Part | Where | Does |
| --- | --- | --- |
| Screens, state, platform IO | `lib/` (Dart) | BLE through `flutter_reactive_ble` on Android and through the bridge's `wellfed-ble` on a computer, NSD through `nsd` on Android, the client secret in `flutter_secure_storage`, alerts through `flutter_local_notifications`, saving the backup through `share_plus` |
| Background work | `android/app/src/main/kotlin/` (Kotlin) | The periodic catch-up through WorkManager, the one foreground service for runs and instant alerts, and the engine they run Dart in |
| `wellfed-app-bridge` | `rust/` (Rust) | The client key, the setup and recovery sessions (`wellfed_client::setup`), device API sessions (`wellfed_connect`), the install (`wellfed_install`), the app's file |
| Generated | `lib/src/rust/`, `rust/src/frb_generated.rs` | The Dart API and types, from `flutter_rust_bridge_codegen generate` |
| Theme | `lib/theme/tokens.g.dart` | Colors and sizes from [`assets/brand/tokens.json`](../../assets/brand/tokens.json) |
| Limits and roles | `lib/src/limits.g.dart`, `lib/src/roles.g.dart` | The feeder's limits (portions, meals, calls, clients, portion weight, desiccant period) and who may send which op (`mayAsk`, the feeder's own matrix), from `wellfed-model`, as constants a widget reads without the Rust library |

- **The Rust half checks what the feeder checks.** Dart moves bytes and shows what the session says. A feed goes out once; a feed whose answer never came is shown as "may have run" and never sent again.
- **Every feeder request goes through the router** (`rust/src/route/`, `FeederRouter` in Dart), on the phone's own session with the feeder. Every write is written down in `pending.json`, beside the app's file, before it goes, and is sent once. One that may have run stays pending: it is never sent again, and resolving it only reads the feeder's history, where a feed's `Fed` event settles it. A new feed on that feeder waits until an uncertain one is settled or put away: closing the feed sheet that asks "Did it feed?" puts it away, and one an earlier run left, after a restart too, is asked about before the next feed. Each session is attached to the router before it runs, and a session offers Dart no request of its own, so every request and feed a screen sends goes through the router. Outside it, each a flow of its own that never sends a feed: the reads a session makes for the card and the history, its clock sync when it opens (after pairing, or when the feeder's clock is unset), a firmware update, the restore of stock, and the install's, setup's and join's own sessions.
- **What can't be written down holds things back.** A pins file that doesn't read is set aside, never deleted, and holds every feed on every feeder back until the owner has checked the bowls. One the phone's storage can't open at all (an IO error, or another process holding `pending.json.lock`) blocks every request instead of sending writes unpinned. A router left behind by a restarted engine in the same process is taken over by the next one and sends nothing again. A change other than a feed that never said how it ended shows on the feeder's screen until the owner, having looked, puts it away, since no history names it; a feed's own answer reaches a screen made again while it was out by the feed's key.
- **Images** are links to the committed renders in `assets/renders/`, and the launcher icon links to `assets/brand/android/`.
- **The client secret never leaves the phone.** Android backup is off, so a new phone pairs again.
- **A session gives a feeder this phone's clock** right after pairing, and whenever the feeder's clock is not set, as `wellfed time` does.
- **Every socket follows the Wi-Fi network.** Android keeps mobile data as the default network while the Wi-Fi has no internet, so `MainActivity` binds the app to Wi-Fi. A VPN that apps cannot bypass (Tailscale, for one) refuses that: exclude Wellfed from it.

## Alerts

The feeder decides every alert ([alerts](../../docs/firmware/alerts.md)). The phone catches up with each feeder's table, posts each alert once, takes the notification off when the feeder clears it, and decides one thing itself: that it can't reach a feeder while it is home. Alerts work only at home, and the alert settings page says so first.

- **Rust decides, Dart posts.** A catch-up (`rust/src/catch_up.rs`) opens a session that can ask for the status and the alert table and nothing else, so it never sends a feed or a write. It reads the table only when the status shows it moved since the last one applied: the cursor is the ring's epoch, the boot count and the table's generation. `wellfed_client::alerts` compares the table with what the phone tracks by each alert's key, and says what to post and what to take off. While the feeder can't vouch that an absent alert ended (`settling`, or `more` for a report), its notification stays, but not for good: `settling` is one flag for the whole table and can stay set while the clock is unknown, so a condition comes off after 2 hours absent (counted from the first read with a valid clock for the desiccant, and with local time for stuck plans), and a report once it is past the feeder's 48 hours, by its own time or, with none, by how long it has been absent ([alerts](../../docs/firmware/alerts.md)). A posted alert whose kind changes says so again (an uncertain meal found failed); the same words again under its id don't buzz. Dart posts and removes notifications, then confirms each change, and one not confirmed is handed over again at the next catch-up under the same id.
- **Mutes and dismissals stay on the phone.** A muted kind or level is listed and never posted; muting takes its notifications off. Dismissing hides an alert on this phone until the feeder clears it; the feeder is never told. A feeder the phone forgets takes its alerts with it, and a catch-up that ends after that brings none back and takes off anything it put up.
- **Asking to post.** The app asks Android once, when Home shows a feeder of the phone's own (as it opens, or when the first is added), and remembers that only once the request has run. After that the owner asks, from the alert settings or a feeder's alerts, which say when notifications are off. The button says "Allow notifications" and asks Android until a request on this install has been refused; a refusal is the owner's answer, and the app leaves them where they were. From then on, and always below Android 13, it says "Open notification settings" and opens the app's page in Android's settings without asking, since past a refusal Android may never show its dialog again and nothing tells the app whether it would. `alerts.json` keeps the refusal. Back from the settings, both screens read Android's word again and post the open alerts, unless an ask of their own is out, which posts for itself. Once Android allows it, alerts still open are posted; ones that ended meanwhile are not.
- **Unreachable is the phone's only judgement.** A catch-up first tries where the feeder last answered; if nothing answers, NSD looks for it and a last try goes there. Only that last try counts, and only on the network where the feeder last answered, recognised by its gateway and subnet with no location permission. Three failures in a row, at least 10 minutes from the first, open it; the next answer clears it. Away from home nothing the phone tries counts.
- **When it runs.** WorkManager runs a catch-up about every 15 minutes (`CatchUpWorker`), and far less often while the phone sleeps. "Instant while home" runs one each minute from the foreground service, with a wake lock and a Wi-Fi lock, which costs battery. The service stops when the phone restarts or Android stops it, and Android lets only a visible app start it, so instant mode comes back when the app opens; the settings page says when it is on but not running, and starts it again from there. Installs, updates and restores keep catch-ups off every feeder while they run: the engine holds them for each run's Dart steps, and each run's own thread holds them for as long as it lives, so an update's restart or an install's listener, which outlive a swiped-away app, still keep catch-ups off. Swiping the app away clears only the engine's holds.
- **Background engines never load the Bluetooth plugin.** A catch-up with no screen runs Dart in a Flutter engine of its own (`BackgroundEngine`), built with no automatic plugin registration and given only the notification, secure storage and NSD plugins. The Flutter plugins for background work and foreground services start their engine with every plugin attached, and `flutter_reactive_ble` keeps its controller and its open links in statics: a second engine attaching it takes the app's scan and links over, and its shutdown disconnects them, so a catch-up during Bluetooth setup would cut the setup. That is why WorkManager and the service are this app's own Kotlin, not plugins.
- **Licenses.** `androidx.work` is Apache-2.0 and `flutter_local_notifications` BSD-3-Clause, its Linux and Windows sides included. The bridge reads a computer's interfaces with `if-addrs`, MIT or BSD-3-Clause. `flutter_local_notifications` needs core library desugaring for its scheduling API, which this app doesn't use, and that adds `desugar_jdk_libs` (GPL-2.0 with the Classpath Exception, so linking it imposes nothing on the app). No check in `make check-app` reads the licenses of Dart or Gradle dependencies; cargo-deny covers Rust only.

### On a computer

A computer follows its feeders only while the app is open: the phone stays the main way to hear. Closing the window ends the checks, and the alert settings say so. There is no tray icon to keep the app running with its window closed. On GNOME, a tray needs an extension, and each tray plugin adds native code on all three systems, so closing would mean different things on different desktops.

- **The same Rust, the same Dart.** `AlertWatch` (`platform/alert_watch.dart`) runs a round of catch-ups now and each minute, one at a time, the minute of "instant while home". A round reads the feeders the app knows at that moment, and posts and confirms through `Notices` as Android does. So the dedupe, the clearing, the mutes and the words are Android's own. The settings page has no instant switch on a computer (`OpenWatching`).
- **A computer's network.** The unreachable judgement needs to know which network the computer is on, and a computer may be on a cable. The bridge's `computer_network` names the IPv4 network of the interface its default route leaves by (`rust/src/network.rs`), fingerprinted as a phone's gateway and subnet are. A full-tunnel VPN's address is the same everywhere, so past a tunnel (a point-to-point link, a /32 or a tunnel's name) it names the one private LAN beside it. With none, or two, the network has no name: a catch-up still dials, but learns no home and counts no failure. With no IPv4 default route, an IPv6-only uplink included, the computer reads as away and dials only an address the owner gave.
- **One app at a time.** Opening Wellfed again brings the running one forward instead of starting a second, which would watch the same feeders and post every alert twice. Linux's runner is a unique GApplication, and Windows's holds a named mutex; macOS opens one app anyway. A debug build stays out of it, so `flutter run` starts beside an installed Wellfed rather than raising it: on Windows its mutex has a name of its own, and on Linux it isn't unique, since path_provider names the app's folder by its id.
- **Each system's notifications.** On macOS the app asks to post at the moment Android would, when Home first shows a feeder, and the button opens the Notifications pane once a request was refused. A click brings macOS's app forward by itself. On Linux they go to the freedesktop notification server, with an urgent alert marked critical, which stays until dismissed. A desktop with none answering gets no notifications, and both alert screens say so with no button to press. Windows posts toasts under the app user model id `io.github.isaacduarte.wellfed`, which the plugin registers for an unpackaged app in the user's registry at start, with the class a toast's click activates. Windows has no permission to ask: while its settings keep the app's toasts off, the button opens them.
- **What the plugin leaves out.** Its Windows side reads the notification setting, lists the toasts showing and takes one off only for a packaged app, so the runner does all three for this unpackaged one (`windows/runner/toasts.cpp`), by the app's id and each toast's tag. Windows answers a notifier's own setting for this app id with "element not found", so the runner reads the two switches the Settings app writes instead: the app's `Enabled` and every app's `ToastEnabled`, either at 0 meaning off and an absent one on. Its Linux side lists nothing showing and can't say whether a notification launched the app, so the sweep reads the ids it keeps a server id for, and a click reaches only a running app. A notification server gone mid-round counts a notification as off. Anything failing as alerts start is logged, and the app opens anyway. A notification posted before the app closed stays until the app is open again, and then comes off at the first round that sees its alert end.
- **One delivery at a time.** A delivery hands over every change Rust still holds unconfirmed, so a screen's check during a minute's round would post the same notice twice. On a computer, rounds, dismissals, mutes and posting the open alerts go one after another (`oneDeliveryAtATime`); a phone's app engine runs no rounds and goes as before. A Linux notification server that takes over 5 s to answer counts as none, so it can't hold the rest up.
- **Known limits.** A TAP-mode VPN looks like a LAN, not a tunnel, so it names its own network. On Windows, a Hyper-V virtual switch (`vEthernet`) counts as a guest bridge, so past a tunnel a LAN behind one can't be named. The sweep after a set-aside `alerts.json` sees only what each system lists: on Linux what the plugin keeps a server id for, and on Windows the action centre's toasts.
- **A click opens the feeder's alerts, in front.** The tap goes to the alerts page as on Android, and the runner's window channel (`platform/app_window.dart`) restores and raises the window. A Wayland compositor may only mark it as wanting attention, since the plugin hands over no activation token.

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

A bench image carries the build's Wi-Fi and trusts the bench key, so it never goes into a release. `make app-release-apk` refuses an `assets/installer/` that holds anything but a consumer image or nothing, and a release or profile build of the bridge refuses to install a bench image whatever its pin. Install a bench image from a debug APK.

### What the app keeps

The app keeps three files in its private storage, each `{"version": n, "data": …}` (`rust/src/stored.rs`):

| File | Holds |
| --- | --- |
| `wellfed.json` | The phone's name for itself, the feeders it knows and the theme |
| `pending.json` | The writes that may have run, and this phone's feeds running whose end it has not read. A feed keeps its portions and the newest event its session knew, and any other write its kind: no device API type, so a change to the device API never changes this file |
| `alerts.json` | Each feeder's alert watch (the cursor its table was read to, the notices open and those still to post or take off, where the phone last reached it and on which network), the mutes and "instant while home". In a file of its own, so a bad one can never keep the app from its feeders. A set-aside one is named in the file until the owner reads the notice, since a background catch-up may be the first to open it, and every alert notification that no watch in the new file knows comes off, since nothing else would take it off; a post of one of those ids is held back for at most a minute while that happens. Its version 1 shape is pinned by `rust/tests/fixtures/alerts-v1.json`, as `wellfed.json`'s and `pending.json`'s are by theirs |

The client key is in secure storage, and the stock backups in their own folder, so neither is in these files.

The files follow the beta upgrade rule (`wellfed_formats::kept`). Within its version a file grows by fields an older build skips. A build that changes a shape further, or adds an enum value an older build can't read, bumps the version and keeps a reader for every version before it, converting. `rust/tests/fixtures/kept-enums.txt` pins every enum in the files per version, so a new variant fails until the version is bumped. A file a newer build wrote (after a downgrade) is left as it is: the start fails and says so, or, for `pending.json`, every request is blocked, until the app is updated. A file the start can't read (damaged, or from a version no reader takes) is kept as `<name>.damaged-<n>` beside it, never deleted, and replaced with an empty one, and the app says so on a screen before Home. For `pending.json`, a lost pin may have been a feed that ran, so every feed waits until the owner says the bowls are checked; the hold is written into the new file, so a restart keeps it. A file the phone's storage can't read at all fails the start, which says so rather than guess.

A set-aside `wellfed.json` loses the list of feeders, but the feeders still know the phone, since its key is kept: the notice sends the owner back to each through Recover a feeder at the panel (a feeder that still lists the phone answers "already enrolled" and logs it; one that revoked it enrolls it again), or through an invite. Without the panel or an invite, the app has no way to find them again.

The versions start at 1 with this build, and an earlier build's unversioned files are set aside like any other. No phone outside the bench ran one.

## Release signing

A release APK is signed with the owner's own Android keystore, which never sits in the repository. Debug builds keep Android's debug key and need none of this. A release build, `make app-release-apk` or any `flutter build apk --release`, reads `WELLFED_ANDROID_KEY_PROPERTIES`: the absolute path of a `key.properties` file outside the repository. Without it, or with a file that lacks a field, names a keystore that isn't there, or sits inside the repository, Gradle stops before building and says which.

> [!WARNING]
> Android installs an update only if it is signed by the same key as the installed app. If you lose the keystore, every phone must uninstall the app to take a build signed with a new one, and uninstalling deletes the phone's key, so each feeder must take the phone back through Recover a feeder or an invite. Keep two copies, offline.

You create the keystore once, offline, onto removable media, as with the [firmware release key](../../docs/development/firmware-builds.md#the-release-key):

1. Unplug the network. Insert a removable drive, for example at `/run/media/$USER/KEY1`.
2. Create the keystore with the JDK's `keytool`:

   ```sh
   keytool -genkeypair -v -storetype PKCS12 -keystore /run/media/$USER/KEY1/wellfed-android.jks \
     -alias wellfed -keyalg RSA -keysize 4096 -validity 10000
   ```

   It asks for a password and for the name fields. A PKCS12 keystore has one password, for the store and the key alike.
3. Write `/run/media/$USER/KEY1/key.properties` beside it:

   ```properties
   storeFile=/run/media/<you>/KEY1/wellfed-android.jks
   storePassword=<the password>
   keyAlias=wellfed
   keyPassword=<the same password>
   ```

   Both paths must be absolute.
4. Note the certificate's SHA-256 on paper: `keytool -list -v -keystore /run/media/$USER/KEY1/wellfed-android.jks`.
5. Copy both files to a second drive, check the copies with `cmp`, and keep the drives in two different places. Keep the password in a password manager too.

To build the signed APK, insert one drive and run, from the repository root:

```sh
WELLFED_ANDROID_KEY_PROPERTIES=/run/media/$USER/KEY1/key.properties make app-release-apk
```

It builds `build/app/outputs/flutter-apk/app-release.apk` for arm64 phones, verifies it with `apksigner` and prints the certificate it was signed with. Check its SHA-256 digest against the one you noted: `apksigner` prints it in lowercase without colons. Unmount the drive afterwards.

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

It is the same app, started with `flutter run -d linux`, or built with `flutter build linux` into `build/linux/x64/release/bundle/`. So far it starts and finds feeders on the LAN. Recovery over the LAN needs nothing more, but has not been walked on Linux yet. Bluetooth setup goes through the bridge's `btleplug` over BlueZ, the CLI's radio, and has not run against a feeder. The install, firmware updates and restores run as on the other desktops ([below](#installing-updating-and-restoring-on-a-computer)). Alerts post while the app is open ([on a computer](#on-a-computer)), through the desktop's notification server, which a bare window manager may not run.

- **Discovery goes through Rust.** `nsd` has no Linux side, so `platform/discovery.dart` browses through the bridge's `FeederBrowse`, over the same `wellfed_connect::Browse` the CLI locates feeders with. Android keeps NsdManager, which holds the multicast lock a phone needs.
- **Bluetooth goes through Rust.** The Bluetooth plugin is Android's alone here, so `platform/setup_link.dart` hands a computer's scan, link and power to `DesktopBluetooth`, over the bridge's `api::bluetooth` and the CLI's `wellfed-ble`. The setup, install and restore state machines get the plugin's own types and run unchanged. A scan that fails or a link that breaks is a `BluetoothFailure`, read as the radio's fault as the plugin's own errors are. On Linux the scan names no service, since a filtered discovery once crashed bluetoothd. The adapter's one discovery runs while any scan does, so a pairing's search and the install's watch for setup mode never end each other's. A feeder counts as advertising when BlueZ holds a signal strength for it, and elsewhere when it was heard in the last 5 s, since CoreBluetooth and WinRT keep a device's last signal for good. Those two also forget a feeder when its link closes, so a retry at the same feeder listens for it again first. Discovery starts again when Bluetooth comes back on under a running scan. A link's open is bounded at 75 s in Rust, inside Dart's 90 s wait; a link that opens late is hung up, and only the newest link to a feeder ever hangs it up, since the stacks keep one link per device.
- **Nothing asks for permissions.** BlueZ and NetworkManager answer any process of the logged-in user, so `AppPhonePorts` reports everything granted, and the computer's name is its hostname.

## The macOS desktop app

It is the same app, started with `flutter run -d macos`, or built with `flutter build macos` into `build/macos/Build/Products/Release/Wellfed.app`. It needs macOS 12 or later, the oldest that `file_selector` supports. On a Mac it has found the simulator on the LAN by mDNS and recovered it. Integration tests then drove it against the simulator, in simulation only: a feed, meals added, edited and deleted, a setting, the time zone, the portion size, History with a button feed arriving, an invite made and a second key joining by its link, a meal call sent from a WAV the test handed over and played, and a restart asked through its confirmation dialog (the fault screen's button was not reached). Bluetooth setup goes through the bridge's `btleplug` over CoreBluetooth, as on Linux, and has not been tried on a Mac. **Turn on Bluetooth** opens the Bluetooth pane of System Settings. The install, firmware updates and restores run as on Linux ([below](#installing-updating-and-restoring-on-a-computer)). It posts alerts while it is open, and asks to post them once it has a feeder ([on a computer](#on-a-computer)).

- **It is signed ad hoc, outside the sandbox.** Building it needs no Apple developer account. The device and network entitlements in `macos/Runner/` are inert outside the sandbox and without the hardened runtime, which this build does not enable.
- **Discovery goes through Rust, as on Linux.** Feeders come through the bridge's `FeederBrowse`. The desktops use no `nsd`. The first browse may make macOS ask whether Wellfed can find devices on your local network: allow it.
- **The secrets are in the login keychain.** The data protection keychain needs a team ID, which an ad hoc build lacks, so `platform/secure_storage.dart` turns it off. The key survives a restart of the same build. The keychain knows an ad hoc build by its hash, so after each rebuild macOS asks whether Wellfed may read its own key, and the window stays blank until you answer. If you deny it, the app cannot read its key and stays blank: quit it, open it again and allow.
- **Nothing asks for permissions up front.** `permission_handler` has no macOS side, and macOS asks on first use instead. Nothing asks for location yet, so the Wi-Fi's name is never filled in for you. The computer's name is its hostname, without the `.local` a Mac adds.
- **Shortcuts use ⌘.** ⌘1 to ⌘9, ⌘0 and ⌘N do what Ctrl does on Linux. ⌘, belongs to the menu bar's Settings…, so the window does not bind it as well.
- **The menu bar is the Mac's own.** Settings… (⌘,) in the Wellfed menu opens the app's settings: the runner (`macos/Runner/AppDelegate.swift`) sends the pick to Dart over the `wellfed/menu` channel (`platform/menu.dart`). That has been seen in the running app. Its tests also cover that a dialog open over the window keeps it, and that a second pick stacks no second page. File › Close (⌘W) is wired to close the window, which quits the app, but nobody has pressed it yet. About Wellfed is wired to the standard panel, which has not been opened yet.
- **Links go through the clipboard.** A Mac's share menu opens from a corner of the window, so an invite's link and the live log are copied instead. Joining takes the pasted link, and the camera opens only when asked, on a Mac and not on Linux, which `mobile_scanner` has no side for.
- **A backup is saved through a save dialog** in place of the share sheet, at the install's end and from the settings' backups. That has not been tried on a Mac.
- **The microphone is asked for on the first recording.** The app expects macOS to ask then, but the prompt has not been seen: in the runs so far, all started by `flutter test`, macOS refused the microphone and the camera without asking. If it is refused, the meal call says where to allow it (System Settings › Privacy & Security › Microphone). A WAV file works in its place.

## Installing, updating and restoring on a computer

The desktops go through the same state machines and the same Rust runs as Android, with three differences. None has been run on a feeder: every one is host-tested with fakes only, and the Windows parts have not been run on Windows.

- **The owner joins the feeder's setup network by hand.** `AppInstallPorts` answers `feederJoin` with `FeederJoin.byHand`, and `platform/feeder_network.dart` watches the computer's routing table through the bridge's `routeSource`, which connects a UDP socket and reads its address without sending anything. Joined means that route changed to leave from an address in 192.168.1.0/24 that no interface held when the watch began, and that the interface holding it is on a Wi-Fi the system names `PL-AF103-…`. It fails closed: an unreadable name is a refusal. `platform/wifi_name.dart` reads that interface's name: CoreWLAN through the runner on a Mac (`MainFlutterWindow.swift`, which asks for location first), `netsh wlan show interfaces` on Windows, and `iw`, then NetworkManager's single access point in use for that device, on Linux. A name with a control character, or a helper that takes over 3 s, gives no name. A route already on another 192.168.1.x network when the watch starts, a home address that isn't private (a VPN's, unless the VPN's address is private), and a computer already on the feeder's network are refused with what to change. The run gets the address as `InstallAnswer::Joined`, checks the route still leaves from it right before the frame, and binds the frame's socket to it. After the frame the screen says to switch straight back, and the run waits until the default route leaves from somewhere else ([on a computer](../../docs/firmware/consumer-installation.md#stocks-setup-protocol)).
- **Keeping the run alive is keeping the computer awake.** `platform/desktop_awake.dart` holds off idle sleep while a run holds (`RunHolds`): `caffeinate -i -w <pid>` on macOS, `systemd-inhibit --what=idle:sleep … cat` on Linux, with `cat` reading a pipe from the app so a crash lets go, and `SetThreadExecutionState` on Windows. A helper that can't start, or exits within 250 ms (logind refused), counts as refused, and one that ends mid-run reports the hold lapsed through `RunHolds.lapsed`: either way the run's screen says to keep the computer awake. None of them covers a closed lid.
- **Closing the window mid-run asks first.** The app's `AppLifecycleListener` answers Flutter's exit request through `CloseRunGuard`, which shows `CloseRunDialog` while any run holds, and refuses a second request while it asks. GTK and Windows route a window close there; on a Mac the runner's `performClose` quits the app (as closing its one window already did), and quitting goes through the same request.

When a feeder never reaches the broker or the image server, the stop screen names the computer's firewall in its own system's words (`screens/install/firewall.dart`). The app never changes a firewall.

## Change the bridge

1. Edit `rust/src/api/`.
2. Run `flutter_rust_bridge_codegen generate` here.
3. Commit the regenerated `lib/src/rust/` and `rust/src/frb_generated.rs` with the Rust change.

After a change to `tokens.json`, run `UPDATE_BINDINGS=1 cargo test -p wellfed-app-bridge --test tokens` and commit `lib/theme/tokens.g.dart`. After a change to a limit in `wellfed-model` or to the role matrix in `wellfed_proto::access`, run `UPDATE_BINDINGS=1 cargo test -p wellfed-app-bridge --test protocol` and commit `lib/src/limits.g.dart` and `lib/src/roles.g.dart`.

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
