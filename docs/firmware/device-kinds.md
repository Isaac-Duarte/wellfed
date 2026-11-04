# Device kinds and models

This page explains how Wellfed's code is split so that more than one kind of device can run it: feeders, water fountains, litter boxes. It is for contributors bringing up a new model or a new kind. What works on which model is on [the roadmap](../roadmap.md) and the [supported devices](../devices/README.md) page.

## The one idea

Every device is a **model** of a **kind**. Code every device needs is written once, in a shared crate. Code only one kind needs sits beside it, in that kind's own crate, never inside the shared one. Code only one model needs sits in that model's board crate and firmware.

Each layer names what it serves with a Rust enum or a build-time choice, never a string or a runtime flag. The firmware has no `dyn` on the kind and board paths, so the stack audit follows every call.

## Model and kind

`wellfed_proto::device` defines both:

- `Model` names the product: `Plaf103` is the Granary feeder and `Plwf106` the Dockstream fountain.
- `DeviceKind` names what it does: `Feeder`, `Fountain`, and `LitterBox`, which has no model yet and keeps the shape honest.
- `Model::kind` ties each model to its kind. A session's welcome and an invite's answer name the model, one byte on the wire; the kind follows from it.

## The layers

| Layer | Shared by every device | The feeder's own | What a fountain adds |
| --- | --- | --- | --- |
| Chip | `wellfed-hal`, the AmebaD chip HAL with the generic drivers (buttons, LEDs, buzzer, audio, ADC) | — | — |
| Board | `wellfed-board`, the board contract, which re-exports the one board crate its feature selects | `wellfed-board-plaf103` | A board crate of the same shape |
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
2. **The board gets its own crate.** A `wellfed-board-plwf106` crate would hold its pins, its outputs' safe state (the pump stopped), its storage map and its `MODEL_CODE`. It would fill the same contract the PLAF103's crate does, which `wellfed-board` checks at compile time. Two limits stand in the way. The board is chosen by a Cargo feature, which a workspace build unifies, so a second board needs a per-image choice first. And some chip HAL drivers still read the PLAF103's pin names (the buttons, LEDs, buzzer, amplifier and battery channels), so those need generalizing.
3. **The pump gets an owner.** A fountain's own driver crate sits above the chip HAL, as `wellfed-hal-feeder` does for the motor. The chip HAL's turns (`interlock`) and its stopped-output check (`stopped`) still say feed and motor, and would take the pump too.
4. **The stores come for free.** The fountain links `wellfed-device` and `wellfed-device-runtime`. Its runtime implements the writer's `Product` for whatever it keeps of its own, such as a scale's calibration. It says which of its stores the stock handover clears, and what a factory reset keeps (nothing, if it keeps nothing). It then defines `wellfed_store_step`, the hook the SDK runtime calls. A test in `wellfed-device-runtime/tests/product.rs` runs the writer with no product stores at all. The shared settings record still holds the feeder's fields (bowl mode, meal reminder, desiccant), so a fountain's settings need their own record.
5. **The protocol knows what it is.** Shared requests (status, Wi-Fi, clients, time, updates, events) reach every device through `wellfed_device_api::Device`. A fountain's `Device` names `NoFeeder` as its feeder module, so a feeder request is refused `Unavailable` on the device itself. Its own requests go in a module of their own, as the feeder's do under `Op::Feeder`.
6. **The server keeps them apart.** Adoption stores the model the invite's answer names, and its kind. The service refuses another kind's read or write with `409 other_kind` before it is journaled or sent, and `wellfed-connect` refuses it before it is sent ([server design](../server/design.md#runtime)). Every device joins the one home the server serves.
7. **It runs without hardware.** `wellfed-device-sim` gives a simulated fountain its storage image, the shared stores and their mailboxes, the host sockets, setup's Bluetooth link and the update banks. A fountain simulator adds its own loop, as `wellfed-feeder-sim` adds the feeder's.

A new feeder model takes fewer steps. It shares the feeder's crates, and supplies a board crate, a firmware crate and its `src/firmware/devices/<model>/` files ([adding a device](../devices/README.md#adding-a-device)).

## Evidence

- **Modeled** (November 3, 2026), the shared writer with no product stores: it opens, publishes and saves the device's settings, and finishes a factory reset with nothing kept (`wellfed-device-runtime/tests/product.rs`).
- **Modeled** (November 3, 2026), the shared simulator stores with no kind linked: a settings change, a client change and an event read through the firmware's read turns (`wellfed-device-sim/tests/stores.rs`).
- **Modeled** (November 3, 2026), the server refusing a feeder read and write to a device of another kind, unjournaled (`wellfed-service/tests/kinds.rs`).
- **Modeled** (November 3, 2026), static analysis of the linked images by the build's own audits: after the split, the feeder images build with every link, RAM-park and stack audit passing and every task's stack bound unchanged.
