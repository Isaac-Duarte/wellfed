# Roadmap

What works today and what comes next, for the replacement firmware, the Android app, Home Assistant and the tools around them. Everything here was tested on one PLAF103 on our bench, which is no promise that it works on yours.

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
| Guided installer (the Android app) | Works | Run from stock on the research unit: setup frame, stock's install, Bluetooth setup, kept backup, promotion and confirmation. |
| Roll back to your feeder's own stock image | Works | Run from the app, and from the developer CLI, then set up in the vendor's app; checked on the feeder once |
| UART backup and recovery | Works | For developers. See [recovery](devices/plaf103/recovery.md). |
| Updates over the air, with automatic rollback | Works | An image that fails to start or hangs gives way to the previous one. Each boot checks the recovery copy in bank 0, and the status (and the app) says why an update would be refused. After a UART write of bank 0 the bench tools finish the job so updates install again; both ran on the research unit, followed by an update. |

## Feeding

| Capability | Status | Notes |
| --- | --- | --- |
| Feeding with motor supervision | Works | Stops on time, survives a power cut without feeding twice. |
| Dispense button | Works | Respects the child lock. |
| Feed from the app | Works | |
| Portion calibration | Works | Weighed once with a kitchen scale; the weight survives reboots, installs and the Reset button. |
| Jam and sensor fault handling | Works | A jam, blocked outlet or sensor fault ends the feed by name, never retried, and shows in the status and history. On the feeder, simulated jams end as `jammed` and `mark_stuck` and a blocked outlet stops the motor within about a second; kibble pressed into the rotor by hand did not jam it. |
| Schedules | Works | Up to ten plans, run on the feeder with no phone or computer on. |
| Feeding history | Partial | The app's History lists each feed the feeder keeps, across reboots; the history is as long as the feeder's own record. |
| Food level | Partial | The status and low-food light show the hopper's latest reading, taken by each feed and, on the adapter, a short window soon after a boot and every minute, so a refill shows with no feed. A feed on low food still runs, as stock's does, and its history says so. Checked on the feeder with and without food, and at rest with the beam clear and blocked by hand; the emitter current is unmeasured. The beam is a low warning, reading empty with about five portions left. |

## The feeder itself

| Capability | Status | Notes |
| --- | --- | --- |
| Wi-Fi join and reconnect | Works | |
| Bluetooth setup and pairing | Works | |
| Encrypted local API | Works | It only grows, so a phone or Home Assistant of another release keeps working with a feeder and says which side to update; mixed releases are tested on the host only. See [device API](firmware/device-api.md#evolution). |
| Child lock | Works | |
| Reset button | Works | Clears clients, Wi-Fi, settings, plans, history and statistics; keeps the calibration, feed journal and flash wear counts. Checked on the feeder: a hold on an unlocked panel wipes it, and the owner pairs again over Bluetooth. |
| Buttons | Works | |
| Startup and build identity | Works | Starts with the motor stopped and reports its build. |
| Flash, journal and settings | Works | See [flash layout](devices/plaf103/flash-layout.md). |
| Stop timer and watchdog | Works | |
| Indicator lights | Works | Which states show is not settled. |
| Speaker | Works | Plays sounds; which sounds play when is not settled. |
| Clock and time zones | Partial | Needs a time source after each power-up. Offline time is experimental. |
| Battery and brownout | Partial | Stock's battery mode works on the feeder 20 times in a row: Wi-Fi off 5 s after the adapter goes, Dispense still feeds, Wi-Fi back when it returns. The charge on stock's curve, the floor and the brownout hold are tested in simulation only. |
| Statistics and per-feed traces | Experimental | |
| Lights and sound settings | Works | Lights and sound windows, volume-0 mute and a beep on every button press, checked on the bench. |
| Desiccant reminder | Works | Counted on the feeder from the last replacement; the period and the date survive a reboot. |
| Alerts on the feeder | Experimental | The rules and the read run in simulation. The Android app notifies them, and Home Assistant shows them. |
| Alerts at home | Experimental | The Android app catches up with each feeder about every 15 minutes, or each minute with "instant while home", notifies each alert once by level, clears it when the feeder does, and says when it can't reach a feeder while home. Mutes and dismissals stay on the phone. On macOS, Linux and Windows the app does the same each minute while it is open, with the system's own notifications. In simulation only: it has not run on a phone or a computer. |
| Alerts away from home | Not started | Nothing reads a feeder's alerts from outside the home network. Planned: a Home Assistant automation that sends them to its phone app, and the app reaching feeders over a VPN such as Tailscale. |
| Meal reminder | Works | An event on the event stream before each scheduled meal, repeated once after a reboot inside the lead. |
| Skip a meal | Experimental | Skip a plan's next meal today, or undo it until it is due, from the app's Today tab. On the research unit, from the developer CLI, skipped daily and one-time meals neither reminded nor fed, and an undone skip reminded and fed once. The feeder's refusals and the app ran in simulation only. |
| Meal call | Works | Your own clip, uploaded from the app, plays before each meal whose plan asks for it, ending as the meal is due; muted with the sound off. Checked by ear on the feeder at low, middle and full volume. |

## Apps

| Capability | Status | Notes |
| --- | --- | --- |
| Command-line client (`wellfed`) | Partial | A developer and bench tool on macOS and Linux, not one of the beta's owner apps and not packaged for owners. It speaks the whole device API, with `--json` for scripts; its Bluetooth setup has run on Linux only. See [the command overview](../src/tools/README.md#the-wellfed-command-line). |
| Android app | Partial | Installs from stock through the feeder's own Wi-Fi after a Reset hold, sets up a feeder over Bluetooth or takes it back over the LAN, and has a screen for everything the device API offers: meals, history, settings, people and invites, updates and diagnostics. It talks to each feeder directly, and a feed whose answer was lost stays held, never resent, until the owner checks the bowl (host and simulation only). Its install and its return to the vendor's firmware ran end to end on the research unit several times, including from a feeder set up in the vendor's app, and every screen was walked there, including a power cut mid-feed, an interrupted recording and a damaged install record. Alerts at home are built and have run in simulation only. See [the app](../src/app/README.md). |
| Desktop app (macOS and Linux) | Experimental | The same app as Android. Unsigned installers are built and checked: a disk image for macOS, and an AppImage and a .deb for Linux, which started in clean Debian and Ubuntu containers; none has been opened from a download on a real computer. The install, firmware updates and restores run on the desktops too, with the owner switching to the feeder's setup network by hand; that is host-tested only, and the window for switching back is not yet timed on a feeder. The Linux build starts and finds feeders on the LAN, and has not yet driven a feeder. The macOS build has a desktop layout of its own (a sidebar of feeders, dialogs for meals and invites, History with a feed beside the list) and the Mac's menu bar. It recovered the simulator over the LAN, and integration tests then drove it against the simulator: its screens at the smallest and a large window in both themes, a feed, meals added, edited and deleted, a setting, the time zone, the portion size, History with a button feed arriving, an invite made and a second key joining by its link, a meal call sent from a WAV the test handed over and played, and a restart asked through its confirmation dialog (the fault screen's button was not reached). All in simulation only. Recording, the file pickers and ⌘W have not been tried on a Mac. Bluetooth setup goes through the bridge's `btleplug` (the CLI's radio) on both desktops, and has not run against a feeder on either. |
| iPhone app | Not started | The same app, for setup and daily use. Whether it can install is an open question, so you would install from Android or a computer. |
| Windows app | Experimental | The same app as on macOS and Linux. It builds, starts and passes the app's tests in a Windows 11 virtual machine. Its unsigned per-user installer installs, starts the app and uninstalls there, keeping the app's data; SmartScreen on a real PC has not seen it. Bluetooth, the install and the other runs haven't met a feeder from Windows yet. |
| Home Assistant | Experimental | Directly as a member, by invite or zeroconf: feeds and cancels, the day's meals and portions, settings, plan switches, repairs for a lost feed, the portion weight and the feeder's alerts. Simulation only. See [the guide](guide/home-assistant.md). |

## More devices

| Capability | Status | Notes |
| --- | --- | --- |
| Code shared across device kinds | Experimental | Shared device crates with each kind's beside them, the device API's core with a module per kind, and a shared simulator. Tested on the host and in the feeder's simulator; the feeder has not run a build of the new layout. See [device kinds and models](firmware/device-kinds.md). |
| Water fountain (PLWF106) | Experimental | Board and stock firmware recovered ([PLWF106](devices/plwf106/README.md)). A [boot-test image](devices/plwf106/boot-test.md) builds and passes its audits but has not run on a fountain. UART writes of it into bank 1, and their recovery, are tested on the host only ([recovery](devices/plwf106/recovery.md)). |

## Development hardware

| Capability | Status | Notes |
| --- | --- | --- |
| Wellfed Bench board | Works | One batch of Rev B built. On November 2, 2026 one board passed bring-up and drove a stock feeder's power, loader entry and a full flash read, apart from the D-cell refusal; its current is unchecked against a meter. See [the bench board](../src/hardware/bench/README.md). |

## What's next

1. A long unattended run with injected faults, to prove the feeder stays safe over hours.
2. Checking the battery floor and a brownout on a bench supply, and a feed on a tired pack.
3. Finishing the Android app: its next install from stock on the research unit, then alerts on a phone at home.
4. Alerts away from home without a server: a Home Assistant notification recipe, and the app reaching feeders over Tailscale.
5. A first research release.
6. The desktop app on macOS, Linux and Windows in the beta: alerts while it is open, installers, and each run proved on a feeder from each system. Then iPhone.
