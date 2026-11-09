# Device kinds and models

This page explains how Wellfed's code is split so that more than one kind of device can run it: feeders, water fountains, litter boxes. It is for contributors bringing up a new model or a new kind. What works on which model is on [the roadmap](../roadmap.md) and the [supported devices](../devices/README.md) page.

## The one idea

Every device is a **model** of a **kind**. Code every device needs is written once, in a shared crate. Code only one kind needs sits beside it, in that kind's own crate, never inside the shared one. Code only one model needs sits in that model's board crate and firmware.

Each layer names what it serves with a Rust enum or a build-time choice, never a string or a runtime flag, and the firmware has no `dyn` on the kind and board paths, so the stack audit follows every call.

## Model and kind

`wellfed_proto::device` defines both:

- `Model` names the product: `Plaf103` is the Granary feeder and `Plwf106` the Dockstream fountain.
- `DeviceKind` names what it does: `Feeder`, `Fountain`, and `LitterBox`, which has no model yet and keeps the shape honest.
- `Model::kind` ties each model to its kind. A session's welcome and an invite's answer name the model, one byte on the wire; the kind follows from it.

## The layers

| Layer | Shared by every device | The feeder's own | What a fountain adds |
| --- | --- | --- | --- |
| Chip | `wellfed-hal`, the AmebaD chip HAL with the generic drivers (buttons, LEDs, buzzer, audio, ADC) | — | — |
| Board | `wellfed-board`, the board contract, which re-exports the one board crate `WELLFED_BOARD` names | `wellfed-board-plaf103` | `wellfed-board-plwf106`, of the same shape |
| Kind logic | `wellfed-core`: the clock, the slot store, the updater's checks, the UART records | `wellfed-core-feeder`: the feed controller, the feed journal, the meal schedule, the pack's floor and the owner's records | Its own crate beside it |
| Kind drivers | — | `wellfed-hal-feeder`: the motor, beams and feedback, and the feeding owner | Its pump and scale drivers |
| Device policy | `wellfed-device`: the settings, Wi-Fi, client, time, event and trace stores, and the update view | `wellfed-feeder`: plans, calibration, the meal call, statistics, the panel and the loop | Its own policy crate |
| Stores on the SDK | `wellfed-device-runtime`: the settings writer over a kind's stores, and its mailboxes | `wellfed-feeder-runtime`: the feeder's stores in that writer, and the writer's settings stage | Its own runtime crate |
| Device API | The shared core in `wellfed-proto` and `wellfed-device-api` | The feeder module: `Op::Feeder` and `FeederDevice` | A module of its own beside the feeder's |
| Firmware | — | `src/firmware/wellfed/feeder`, one crate per model | Its own firmware crate |
| Simulator | `wellfed-device-sim`: storage, the shared stores, sockets, setup's link and updates | `wellfed-feeder-sim` | Its own simulator on the same harness |
| Server | One home, every device in it stored with its model and kind | The feeder's tables and routes | Its own routes, refused for other kinds |

The [device layer](device-layer.md) lists what is specific to the PLAF103 and where each fact lives. The [product architecture](product-architecture.md) records why the code is split this way.

## Following a fountain through

The PLWF106 shows how a new kind slots in. Its board and stock firmware are recovered ([PLWF106](../devices/plwf106/README.md)), but no Wellfed image runs on it yet. Each step below is what a fountain port supplies at one layer.

1. **The model is named.** `Model::Plwf106` exists, with `DeviceKind::Fountain` as its kind. Nothing else in the code has to learn its name to tell it from a feeder.
2. **The board gets its own crate.** `wellfed-board-plwf106` holds its pins, its outputs' safe state (the pump off, PA30 LOW), the stock flash a replacement must keep, and its `MODEL_CODE`, filling the same [board contract](device-layer.md#where-the-facts-live) as the PLAF103's, chosen by `WELLFED_BOARD`. The chip HAL's panel drivers (buttons, LEDs, buzzer, audio) and power monitor read the PLAF103's pins, so they build only for it. Its stopped-output check reads the chosen board's safe state, so a fountain build stops the pump at startup, on a panic and in a flash lease. The fountain's storage region has no sectors yet, so the updater and boot confirmation do not build for it, and no build there can be confirmed until an allocation is reviewed.
3. **The pump gets an owner.** A fountain's own driver crate sits above the chip HAL, as `wellfed-hal-feeder` does for the motor. The chip HAL's turns (`interlock`) still say feed and motor, and would take the pump too.
4. **The stores come for free.** The fountain links `wellfed-device` and `wellfed-device-runtime`. Its runtime implements the writer's `Product` for whatever it keeps of its own, such as a scale's calibration. It says which of its stores the stock handover clears, and what a factory reset keeps (nothing, if it keeps nothing). It then defines `wellfed_store_step`, the hook the SDK runtime calls, and `wellfed_startup_check`, its synthetic startup check. A test in `wellfed-device-runtime/tests/product.rs` runs the writer with no product stores at all. The shared settings record still holds the feeder's fields (bowl mode, meal reminder, desiccant), so a fountain's settings need their own record.
5. **The protocol knows what it is.** Shared requests (status, Wi-Fi, clients, time, updates, events) reach every device through `wellfed_device_api::Device`. A fountain's `Device` names `NoFeeder` as its feeder module, so a feeder request is refused `Unavailable` on the device itself. Its own requests go in a module of their own, as the feeder's do under `Op::Feeder`.
6. **The server keeps them apart.** Adoption stores the model the invite's answer names, and its kind. The service refuses another kind's read or write with `409 other_kind` before it is journaled or sent, and `wellfed-connect` refuses it before it is sent ([server design](../server/design.md#runtime)). Every device joins the one home the server serves.
7. **It runs without hardware.** `wellfed-device-sim` gives a simulated fountain its storage image, the shared stores and their mailboxes, the host sockets, setup's Bluetooth link and the update banks. A fountain simulator adds its own loop, as `wellfed-feeder-sim` adds the feeder's.

A new feeder model takes fewer steps. It shares the feeder's crates, and supplies a board crate, a firmware crate and its `src/firmware/devices/<model>/` files ([adding a device](../devices/README.md#adding-a-device)).

## Evidence

- **Modeled** (November 3, 2026): `wellfed-device-runtime/tests/product.rs` (the shared writer with no product stores opens, publishes and saves settings, and finishes a factory reset keeping nothing), `wellfed-device-sim/tests/stores.rs` (the shared simulator stores with no kind linked: a settings change, a client change, an event read) and `wellfed-service/tests/kinds.rs` (a feeder read and write to another kind refused, unjournaled).
- **Modeled** (November 3, 2026), static analysis by the build's own audits: after the split, the feeder images pass every link, RAM-park and stack audit, with every task's stack bound unchanged.
- **Modeled** (November 4, 2026), the fountain's board on the host fake (`WELLFED_BOARD=plwf106`): startup latches the pump LOW before it becomes an output and configures no other pin, the panic stop clears only PA30, every storage sector is refused, and no record can confirm a build (`wellfed-hal/tests/pump_stopped.rs`, `wellfed-hal-common/tests/trial.rs`). No fountain image had been built then; the [boot test](../devices/plwf106/boot-test.md) image now builds, and has not run on a fountain.
