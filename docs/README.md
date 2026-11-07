# Documentation

Start here to find the right page, grouped by what you want to do: use Wellfed, understand how it works, learn the hardware, or help build it.

Every finding carries an [evidence level](evidence.md) saying how we know it, and outside sources are in [references](references.md). A documented finding is not a request to repeat an experiment. Historical bench diaries and raw evidence live outside this repository.

## Using Wellfed

If you own a feeder, start with the [owner guide](guide/README.md), which [docs.well-fed.dev](web/docs-site.md) publishes with a few of the pages below. This index lists everything.

| Page | What it covers |
| --- | --- |
| [Roadmap](roadmap.md) | What works today and what is planned |
| [Glossary](glossary.md) | The words the rest of the docs use without explaining |
| [Supported devices](devices/README.md) | Which feeders Wellfed runs on |
| [Consumer installation](firmware/consumer-installation.md) | Moving a stock feeder to Wellfed over the air, and going back |
| [Bank, install and rollback bench checks](firmware/install-bench-checks.md) | Checking the bank rules, an install from stock and a rollback on the bench |

## How it works

The firmware docs describe Wellfed's own replacement, which isn't tied to one device.

| Page | What it covers |
| --- | --- |
| [Product architecture](firmware/product-architecture.md) | The product we are building toward, and why each choice was made |
| [Firmware architecture](firmware/README.md) | How the replacement firmware is put together |
| [Device API](firmware/device-api.md) | The feeder's encrypted local API |
| [Device API operations](firmware/device-api-ops.md) | Each request's answers, refusals and roles, what a feed records, and the live samples |
| [Diagnostics and the stock backup](firmware/device-api-diagnostics.md) | Statistics, feed traces, the debug log and bank 1's copy, over the device API |
| [Pairing](firmware/pairing.md) | Keys, randomness and how clients pair |
| [Bluetooth](firmware/bluetooth.md) | Bluetooth LE and setup mode |
| [OTA design](firmware/ota-design.md) | Over-the-air updates |
| [Bank policy](firmware/bank-policy.md) | Which bank boots, and how a trial image is confirmed |
| [Startup diagnostics](firmware/startup-diagnostics.md) | What every boot checks before firmware code runs |
| [QA firmware](firmware/qa.md) | The guided self-test image |
| [Resource budget](firmware/resource-budget.md) | Flash, RAM and task limits of the product image |
| [Scheduling gap bench check](firmware/scheduling-gap-check.md) | Measuring how long a Wi-Fi rejoin holds the feeder's tasks |
| [Device layer](firmware/device-layer.md) | Where the board's assumptions live, so a second model can be added |
| [Device kinds and models](firmware/device-kinds.md) | How the code splits by chip, board, kind and model, and how a fountain would slot in |
| [Bring-up probes](firmware/bring-up-probes.md) | The bounded tests that first brought each piece of hardware up |
| [The server](server/README.md) | What the optional server adds, [running it](server/running.md), and [how it's built](server/design.md) |
| [well-fed.dev](web/site.md) | The project's landing page |
| [docs.well-fed.dev](web/docs-site.md) | This documentation as a website |

Feeder components:

| Page | What it covers |
| --- | --- |
| [Motor pulse](firmware/motor-pulse.md) | The supervised motor drive |
| [Motor bench checks](firmware/motor-bench-checks.md) | Rerunning the first pulse, the stall, failed-stop, jam and sensor-fault checks on the bench |
| [Schedules](firmware/schedules.md) | Feeding plans, the meal reminder and the meal call |
| [Portions and calibration](firmware/calibration.md) | Counting portions and calibrating them |
| [Weighing session](firmware/weighing-session.md) | Measuring what one portion weighs and storing it |
| [Jams and sensor faults](firmware/jams-and-sensor-faults.md) | What happens when food or feedback goes missing |
| [Optical inputs](firmware/optical-inputs.md) | The food, outlet and feedback sensors |
| [Buttons](firmware/buttons.md) | Panel buttons and gestures |
| [Indicators](firmware/indicators.md) | The panel lights |
| [Indicators bench check](firmware/indicators-bench-check.md) | Checking each light's states on the bench |
| [Audio](firmware/audio.md) | The speaker and built-in tunes |
| [Desiccant reminder](firmware/desiccant.md) | Counting the days until the desiccant pack is due |
| [Alerts](firmware/alerts.md) | The feeder's table of what is wrong, and how a client reads it |
| [Clock](firmware/clock.md) | Wall clock, offline time and missed time |
| [Clock bench checks](firmware/clock-bench-checks.md) | Checking live NTP, rate learning, warm resets and expiry on the bench |
| [Power](firmware/power.md) | The supply, the battery floor, brownout and sleep |
| [Power bench checks](firmware/power-bench-checks.md) | Checking the readings, the floor, the brownout hold and battery mode on the bench |

## Hardware

Facts about a **chip** hold for every board built on it; facts about a **device** hold for one model and its stock firmware. Adding a model adds a folder under `devices/` and reuses the rest.

| Page | What it covers |
| --- | --- |
| [PLAF103](devices/plaf103/README.md) | The feeder: identification, chips and feature support |
| [Board and signal map](devices/plaf103/board.md) | What each chip pin connects to |
| [Motor driver](devices/plaf103/motor-driver.md) | The motor driver and its electrical checks |
| [UART](devices/plaf103/uart.md) | Console wiring and loader entry |
| [Flash layout](devices/plaf103/flash-layout.md) | Boot flow and what lives where in flash |
| [Recovery](devices/plaf103/recovery.md) | UART backup and the recovery writers |
| [Relay bench](devices/plaf103/bench.md) | Powering and flashing the feeder from the bench |
| [Bench server](devices/plaf103/bench-server.md) | The server that holds the bench: its routes, feeder session, console, lease and records |
| [Integration suite](devices/plaf103/integration-suite.md) | Real-device tests on the bench, and the sustained run |
| [Stock behavior and protocol](devices/plaf103/stock/README.md) | What stock firmware 1.1.15 does |
| [Stock OTA requirements](devices/plaf103/stock/ota-requirements.md) | What stock's updater accepts |
| [PLWF106](devices/plwf106/README.md) | The Dockstream 2 fountain, its board and stock firmware, for a port (Wellfed does not run on it) |
| [PLWF106 board signals](devices/plwf106/board.md) | What each of the fountain's chip pins connects to |
| [PLWF106 UART](devices/plwf106/uart.md) | How the fountain's console and loader entry differ from the feeder's |
| [PLWF106 flash layout](devices/plwf106/flash-layout.md) | The fountain's stock image and what the writers must preserve |
| [PLWF106 recovery](devices/plwf106/recovery.md) | Backing up the fountain, writing a test image and putting stock back |
| [PLWF106 boot test](devices/plwf106/boot-test.md) | The smallest image that shows the fountain's loader runs our firmware |
| [PLWF106 stock firmware](devices/plwf106/stock/README.md) | What the fountain's stock 0.0.17 does |
| [PLWF106 stock OTA](devices/plwf106/stock/ota-requirements.md) | Stock 0.0.17's update request, transfer and activation |
| [RTL8721DM (AmebaD)](chips/rtl8721dm/README.md) | The SoC and its SDK |
| [RTL8721DM pins](chips/rtl8721dm/pins.md) | Each pin's job on each device |
| [Device template](devices/_template/README.md) | The starting point for a new device folder |

## Contributing

| Page | What it covers |
| --- | --- |
| [Development setup](development/README.md) | Toolchain, check targets and fuzzing |
| [Writing docs](development/writing.md) | How these docs are written |
| [Rust standard](development/rust.md) | Rules for every Rust crate |
| [TypeScript standard](development/typescript.md) | Rules for the web frontends |
| [Dart standard](development/dart.md) | Rules for the Android app |
| [Python standard](development/python.md) | Rules for the Home Assistant integration |
| [SDK builds](development/firmware-builds.md) | Full firmware images with the vendor SDK |
| [Stock server](development/stock-server.md) | The bench's server for stock 1.1.15 |
| [Bench hardware](development/bench-hardware.md) | The bench board's connectors, feeder wiring and the order of a flash session |
| [Rust flash client](development/rust-flash.md) | Reading and writing flash over UART |
| [UART development updates](development/uart-development.md) | Faster region-scoped updates during development |
| [Optional fixtures](development/fixtures.md) | Checks that need private binary inputs |
| [Import provenance](development/provenance.md) | Where the first code came from |
| [Host tools](../src/tools/README.md) | Every host tool and what it does |
| [Firmwares](../src/firmware/wellfed/README.md) | Every firmware crate and how to build one |
