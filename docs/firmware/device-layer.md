# Device layer

This page lists what in the firmware is specific to the PLAF103, and where each fact lives. It is for contributors bringing Wellfed to a second model, which it does not support yet. How the code splits by kind is in [device kinds](device-kinds.md).

Each fact is a **board** fact (another feeder would differ), a **chip** fact (any AmebaD board shares it), or a **unit or stock** fact (tied to our research unit or to stock 1.1.15).

## Where the facts live

| What | Where | Kind |
| --- | --- | --- |
| Every GPIO the ports use (motor, emitters, sensor inputs, buttons, indicators, amplifier) and the board's ADC channels | `pins` in `wellfed-board-plaf103`, read through `wellfed_board::pins` | Board |
| Flash layout: OTA banks, stock's bank table, preserve ranges | `src/firmware/devices/<model>/layout.json` (`wellfed_formats::layout::relative_path`), read by the builder and its audits | Unit and stock |
| The replacement's storage region and who owns each sector | `STORAGE_OFFSET`, `STORAGE_SECTORS` and `storage` in `wellfed-board-plaf103`, read through `wellfed_board` | Unit |
| The outputs' safe state a fault or a flash lease writes raw | `safe` in `wellfed-board-plaf103` | Board |
| The model the board is | `MODEL_CODE` in `wellfed-board-plaf103`, checked against `Model` by the firmware | Board |
| Build inputs: stock image, C entry and BLE ports, first-boot mains | `device.json` | Board |

**Three layers.** The chip HAL (`wellfed-hal`) holds what any AmebaD device needs, plus the generic drivers (buttons, LEDs, buzzer, audio, ADC) that take their pins from the board. The board crate holds the pins, polarities, the outputs' safe state, the storage map and the model. `wellfed-hal-feeder`, above the chip HAL, holds the feeder's own hardware: the motor, optics and sensor board, its timer tick and feedback capture, the feeding owner, and the bench variants' injected stop failures, jams and sensor faults.

**The board contract.** Nothing reads a board crate directly. The SDK, both driver crates and the product read `wellfed-board`, which re-exports the one board `WELLFED_BOARD` names and checks at compile time that it has every item the contract names. The build wrapper sets the variable from the device it builds for, and `.cargo/config.toml` sets `plaf103` for every other build, so the workspace and its tests are the feeder's. A name that is not exactly one known board stops the build, and nothing reads the board at run time.

- **Constants, not a trait.** Every value is used in a constant context (a static's length, a compile-time check, a flash lease's RAM-resident code), and a trait's associated constants would need a generic parameter through each, which a static cannot take.
- **Gating.** Code only one board runs is gated on `cfg(wellfed_board = "<name>")`, and code that keeps a store on `cfg(wellfed_board_store)`. `wellfed-board`'s build script emits both for the crates that gate code, so one check decides for all. A Cargo feature would not do: features unify across a workspace build, turning on two boards at once.
- **A second board** is a second crate of the same shape and a name in `wellfed-board`'s build script.
- **Why not `device.json`.** Several pin constants feed audited code (the stopped-output check, the panic and RAM-park output writes, the KM0 pad-table audit). Generating them from JSON would put a code generator in front of audited binaries, for no gain with one board. `device.json` describes what the builder needs.

## Known limits

These facts stay where they are, and are listed here instead. A second model would revisit each one.

### Board facts outside the port layer

- **Panel layout.** `wellfed_core::buttons::Buttons::from_port_b` maps PB4–PB7 to reset, mic, dispense and lock, active LOW. Core is hardware-independent and cannot depend on the SDK bindings. `pins::BUTTONS` is in the same order.
- **Motor gate.** The `wellfed_core::feed::motor_gate::MotorPins` trait names its arguments `pa13_high` and `pa14_high`. It is a Rust trait, so a board-neutral rename is a Rust-only change.
- **Rust interfaces between Rust crates.** The motor pins (`MotorPins`), a store's flash (`store::slot::SlotIo`) and the owner's journal (`feed::storage::OwnerStorage`) are traits the caller's types implement. Each call is direct: the build's stack audit follows it, and no context pointer crosses it. Only what C or the SDK calls stays `extern "C"`: the flash-lock hook provider the RAM interposer installs, the interrupt handlers, and the `wellfed_*` exports.
- **Motor driver.** The TC118AH's truth table is split between two crates: `wellfed_core::feed::motor_gate` makes both inputs HIGH its STOP (the driver's brake), and the board's `MotorPins` implementation, `wellfed_hal_feeder::board::BoardPins`, refuses both LOW. The dispensing direction is clockwise, `Drive0`, with PA13 HIGH ([motor pulse](motor-pulse.md)).
- **Beams.** `wellfed_core::feed::sensor_inputs` holds their polarity, and which is food and which is outlet.
- **Stopped outputs.** `wellfed_hal::stopped` checks exactly the pins the board's `safe` state covers. On the PLAF103 stopped is motor HIGH, emitters LOW and SWD released; on the PLWF106 it is the pump LOW. The raw-register copies in `target.rs` (`panic_outputs_safe`, `ram_outputs_safe`) write the board's `safe` state. The RAM-park audit checks `ram_outputs_safe` as leaf code.
- **Raw outputs are `unsafe`.** In `wellfed-amebad-sys` they are `unsafe fn`s whose contract is ownership: the GPIO writers (`gpio_port_write`, `gpio_write_bit`, `gpio_init`) belong to `board` and `stopped`, which own the motor pins, the watchdog (`wdg_init`, `wdg_cmd`, `wdg_refresh`) to the runtime's supervision, and the flash writers (`storage_erase`, `storage_program`, `bank1_erase`, `bank1_program`, `bank1_give_way`) need the armed hooks' lease. Product firmware reaches them only through those owners. Bring-up code (the other firmwares and the probes) takes the GPIO, watchdog and storage writers from `wellfed_hal::bringup`, in an `unsafe` block stating why the contract holds; the bank-1 writers stay the updater's and are not re-exported there. `cargo xtask check-repository` refuses a raw writer or `bringup` in product code (the `feeder` firmware's sources and `src/product/`), and a raw writer named other than as `bringup::…` in bring-up code.
- **Audio pages.** An `sdk::Audio` handle (one at a time, from `Audio::take`) lends one ring page at a time as a `Page` guard, which `submit` consumes. So no two references to a ring page exist, and none outlives the handle.
- **Shared audio pads.** `firstboot_km0.c` powers the audio-shared pads so PA0–PA7 work as GPIO. `audit/pinmap.rs` checks the stock pad table.
- **Limits.** The probes' and simulator's feed and sensor limits (300 mA, battery 1650 scaled, settle times) are bench values for this board.

### Supervision

- **Shared handles.** `Board` and `Events` are `&self` handles over interior cells. They are borrowed only inside `IrqGuard`, for one call, so the TIM2 and feedback interrupts never alias a task's borrow. A `Board` call nested inside another (from code the first call runs) is refused with `Reentered`, rather than lent the state a second time: on one core, waiting for it would never end. The owner keeps the handles beside its task state, never inside it. The probes still hold them inside the state their task borrows `&mut` through its `Slot`. They are bench tools, but moving them out the same way is still open.
- **Flash leases during a feed.** The lease provider checks stopped outputs (`stopped::observe`) when it quiesces, so a lease during a feed halts the hook and the watchdog resets the board, and the owner faults a feed that sees a lease. So no lease starts while a feed may move the outputs: the feed journal writes only at the owner's rest boundaries under the loop's claim, and every other store write takes the write turn in `wellfed_hal::interlock` ([device API](device-api.md#no-flash-write-while-the-motor-may-move)). The provider's request and quiesce run under the board's serialization.
- **One watchdog owner.** The runtime feeds the hardware watchdog; the feeding owner only publishes a beat (`supervision::beat`). A fault outside initialization stops the beat, so it resets the board. The beat promises two gaps: 1 s while outputs may move, and the runtime's 10 s liveness limit at rest. The second exists because the Wi-Fi driver's WPA3 handshake held the loop off for over a second, and an idle owner then reset the board for nothing.

### Unit and stock facts

- **Battery divider.** `DIVIDER_PER_MILLE` (2063) was measured on the research unit.
- **Indicator readback.** PA0–PA7 read back LOW on the tested unit, so only PB22 has input readback.
- **Flash ID.** `c84017` (GigaDevice, 8 MiB), in the flash probe.
- **Settings binding.** `SETTINGS_BINDING` names the research unit's installation.
- **Bank addresses** live in Rust (`wellfed-amebad-sys`, `core::ota` const-asserted equal, `FlashMmuEntry::bank`, `wellfed-flash`, `wellfed-emu` and `review.rs`), in C (`firstboot_km0.c`, with the confirmation sector), and in `layout.json`, the only data-driven copy, which the builder patches into the SDK's OTA addresses.
- **BLE system-data patch.** The builder pins `check_sw()` to an erased sector's value, as the research unit's is ([Bluetooth](bluetooth.md#build)).
- **Stock image constants.** The SHA-256, and the bank-table and pad-table addresses, in the builder's audits.

### Chip facts

ADC channel 10 (the internal supply, `wellfed_amebad_sys::ADC_SUPPLY_CHANNEL`), the ADC eFuse calibration, SWD pad muxing, the audio codec rates, and the `81958711` bank marker.

## For a second model

A new feeder needs a board crate (pins, ADC channels, polarities), a `layout.json` from its stock image, a checked storage region, a `device.json` naming its ports, and a thin firmware crate ([product architecture](product-architecture.md)). Every known limit above then needs a decision, the panel mapping and the motor gate's names first, since core carries them.

## Evidence

- **Recovered** (static analysis, October 23, 2026): the audit of the SDK, the HAL and the runtime for PLAF103 assumptions. Nothing ran on the feeder for it.
- **Observed** (October 25): the Wi-Fi driver's WPA3 handshake held the loop off for over a second on hardware, which set the 10 s liveness gap at rest.
- **Observed:** the dispensing direction, seen by the owner in the first owner feed ([motor pulse](motor-pulse.md)).
