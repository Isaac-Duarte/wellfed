# Roadmap

This page tracks what works today and what comes next. It covers the replacement firmware, the tools around it and the optional server. Everything here was tested on one PLAF103 on our bench. It is not a promise that it works on yours.

| Word | Meaning |
| --- | --- |
| **Works** | Done and observed on a real feeder. |
| **Partial** | Works on a real feeder, with a known gap. |
| **Experimental** | Built and tested on the host or in simulation; not yet confirmed on hardware. |
| **Not started** | Planned, nothing built yet. |

## Getting Wellfed on and off a feeder

| Capability | Status | Notes |
| --- | --- | --- |
| Install over stock's own updater | Works | No soldering or UART needed. See [consumer installation](firmware/consumer-installation.md). |
| Go back to stock over the air | Works | Using a stock image we serve. |
| Guided installer (the Android app) | Works | Run from stock on the research unit: the setup frame, stock's install, Bluetooth setup, the kept backup, promotion and confirmation. |
| Roll back to your feeder's own stock image | Works | Run from the CLI and the app, then set up in the vendor's app; checked on the feeder once |
| UART backup and recovery | Works | For developers. See [recovery](devices/plaf103/recovery.md). |
| Updates over the air, with automatic rollback | Works | A new image that fails to start or hangs gives way to the previous one. |

## Feeding

| Capability | Status | Notes |
| --- | --- | --- |
| Feeding with motor supervision | Works | Stops on time, survives a power cut without feeding twice. |
| Dispense button | Works | Respects the child lock. |
| Feed from the CLI | Works | |
| Portion calibration | Works | Weighed once with a kitchen scale (`wellfed calibrate`); the weight survives reboots, installs and the Reset button. |
| Jam and sensor fault handling | Works | A jam, a blocked outlet or a sensor fault ends the feed by name, never retried, and shows in the status and history. Simulated jams on the feeder end as `jammed` and `mark_stuck`, and a blocked outlet stops the motor within about a second; kibble pressed into the rotor by hand did not jam it. |
| Schedules | Works | Up to ten plans, run on the feeder with no server. |
| Feeding history | Partial | `wellfed feeds` lists each feed the feeder keeps, across reboots; a long history needs the server. |
| Food level | Partial | The status and the low-food light show the hopper's latest reading. A feed reads it, and on the adapter so does a short window soon after a boot and every minute, so a refill shows with no feed. A feed on low food still runs, as stock's does, and its history entry says the food read low. Checked on the feeder with and without food during feeds, and at rest with the beam clear and blocked by hand; its emitter current is unmeasured. The food beam is a low warning, reading empty while about five portions are still left. |

## The feeder itself

| Capability | Status | Notes |
| --- | --- | --- |
| Wi-Fi join and reconnect | Works | |
| Bluetooth setup and pairing | Works | |
| Encrypted local API | Works | See [device API](firmware/device-api.md). |
| Child lock | Works | |
| Reset button | Works | Clears clients, Wi-Fi, settings, plans, history and statistics, and keeps the calibration, the feed journal and the flash wear counts. Checked on the feeder: a hold on an unlocked panel wipes it, and the owner pairs again over Bluetooth. |
| Buttons | Works | |
| Startup and build identity | Works | Starts with the motor stopped and reports its build. |
| Flash, journal and settings | Works | See [flash layout](devices/plaf103/flash-layout.md). |
| Stop timer and watchdog | Works | |
| Indicator lights | Works | Which states show is not settled. |
| Speaker | Works | Plays sounds; which sounds play when is not settled. |
| Clock and time zones | Partial | Needs a time source after each power-up. Offline time is experimental. |
| Battery and brownout | Partial | Stock's battery mode works on the feeder: Wi-Fi off 5 s after the adapter goes, the Dispense button still feeds, and Wi-Fi back on when it returns, 20 times in a row. The charge on stock's curve, the floor and the brownout hold are tested only in simulation. |
| Statistics and per-feed traces | Experimental | |
| Lights and sound settings | Works | Lights and sound windows, volume-0 mute and a beep on every button press, checked on the bench. |
| Desiccant reminder | Works | Counted on the feeder from the last replacement; the period and the date survive a reboot. |
| Meal reminder | Works | An event on the event stream before each scheduled meal, repeated once after a reboot inside the lead. |
| Skip a meal | Experimental | Today's next meal of a plan, skipped from the device API and undone until it is due. No app or CLI command yet. Tested in simulation only. |
| Meal call | Works | Your own clip, uploaded from the app or the CLI, played before each meal whose plan asks for it, ending as the meal is due, and muted with the sound off. Checked by ear on the feeder at low, middle and full volume. |

## Apps and server

| Capability | Status | Notes |
| --- | --- | --- |
| Command-line client (`wellfed`) | Partial | Every owner setting and action: pairing, status, settings, plans, the meal call, feeds, history, the desiccant reminder, lock, Wi-Fi, time and invites, with text for people and `--json` for scripts. See [the command overview](../src/tools/README.md#the-wellfed-command-line). |
| Self-hosted server | Partial | Heavy work in progress, a proof of concept. Optional: the app alone is the complete product, and the server adds long history, remote access, accounts, pets and food, and firmware releases. It has no web page of its own; you use it through the app or the CLI. Adopts a real feeder and keeps its history whole across a server restart, a power cut and a Wi-Fi drop. The server and two clients talking to the feeder directly follow its events at once, each getting every event, in simulation. Plans changed from the app went through it to the real feeder, and an app adopted that feeder through it. Feeds, the next meal, accounts, releases and release installs are tested in simulation only, as are its food views: food left, when it runs out and each pet's intake, in grams once the feeder's portion weight is set and in portions until then. Installs from stock run from the app or the CLI, never from the server. |
| Android app | Partial | Installs from stock through the feeder's own Wi-Fi after a Reset hold, sets up a feeder over Bluetooth or takes it back over the LAN, and has a screen for everything the device API offers: meals, history, settings, people and invites, updates and diagnostics. Its install and its return to the vendor's firmware have run end to end on the research unit several times, including from a feeder set up in the vendor's app. Every screen has been walked on that feeder, including a power cut mid-feed, an interrupted recording and a damaged install record. The known gap: the install's search for its feeder changed after the last run from stock, so that step has only its tests until the next real Reset. It connects to the optional server: login, the server's feeder list with adoption of feeders paired before it, every request routed through the server or directly with a lost answer never resent, the full history, pets, foods, weights, refills and intake. On the research unit a phone logged in to a server on the home network over http, adopted the feeder, read its history through the server and changed its meals. A feed sent through the server, the fallback to direct with the server down, a lost answer and an https login have run in simulation only. There are no alerts yet. See [the app](../src/app/README.md). |
| Desktop app (macOS and Linux) | Not started | The same app as Android, including the install. |
| iPhone app | Not started | The same app, for setup and daily use. Whether it can install is an open question, so you would install from Android or a computer. |
| Windows app | Not started | Waits for a machine to test it on. |
| Home Assistant | Experimental | Pairs with the feeder directly, with no server, by invite or zeroconf: feeds and cancels, the day's meals and portions, settings, plan switches, repairs for a lost feed, and the portion weight, as a member only. Tested in simulation only. See [the guide](guide/home-assistant.md). |

## More devices

| Capability | Status | Notes |
| --- | --- | --- |
| Code shared across device kinds | Experimental | Shared device crates with each kind's beside them, the device API's core with a module per kind, one home in the server with each device's model and kind, and a shared simulator. Tested on the host and in the feeder's simulator; the feeder has not yet run a build of the new layout. See [device kinds and models](firmware/device-kinds.md). |
| Water fountain (PLWF106) | Not started | Its board and stock firmware are recovered ([PLWF106](devices/plwf106/README.md)), and its board crate builds; no Wellfed image for it yet. UART writes of a bank-1 test image and their recovery are tested on the host only ([recovery](devices/plwf106/recovery.md)). |

## Development hardware

| Capability | Status | Notes |
| --- | --- | --- |
| Wellfed Bench board | Not started | Designed in KiCad with its firmware written; no board built yet. See [the bench board](../src/hardware/bench/README.md). |

## What's next

1. A long unattended run with injected faults, to prove the feeder stays safe over hours.
2. Checking the battery floor and a brownout on a bench supply, and a feed on a tired pack.
3. Finishing the Android app: its next install from stock on the research unit, then alerts.
4. Running feeds through the server on a real feeder, including a lost answer and its retry, from the app as well as the CLI.
5. A first research release.
6. The desktop app on macOS and Linux, then iPhone, then Windows.
