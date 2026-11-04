# Boot test image

This page describes the smallest Wellfed image for the PLWF106: one that tells us whether the factory loader runs our firmware from bank 1. It is for developers bringing Wellfed up on the fountain. Installing it is covered in [backup and recovery](recovery.md#run-a-boot-test).

> [!WARNING]
> Keep the pump unplugged for this test. The image never drives the pump on, but stock's ROM, its factory loader and its pad table leave the pump enable weakly pulled up before our code runs.

## What it does

The image is the `fountain` firmware (`src/firmware/wellfed/fountain`) on the `firstboot` profile. On KM4 it:

1. Latches the pump enable, PA30, LOW, then makes it an output, so the pin never drives HIGH as it becomes one.
2. Prints the build banner, a `PLFW1 start` line with the build and bank, and a `PLBOOT1 start` line with the model and the pump's readback.
3. Starts one task that prints a `PLBOOT1 beat` line every 5 seconds. A beat that finds the pump not reading LOW latches it LOW again and says `relatched=1`.

A boot from bank 1 looks like this:

```text
Wellfed local firmware: Rust bringup
PLFW1 start build=<build id> bank=1 reset=<flags> mmu=<registers>
PLBOOT1 start model=plwf106 bank=1 pump=off latch=0 pad=0 output=1
PLBOOT1 beat n=1 uptime_ms=<ms> pump=off latch=0 pad=0 output=1 relatched=0
```

`pump=off` means the latch, the pad and the direction all read the pump stopped. Anything else prints `pump=unverified`.

## What it never does

- **No other pin.** It configures PA30 and nothing else. The indicators, the button, the scale, the three outputs of unknown role and the radar footprint keep whatever stock's startup leaves.
- **No network, Bluetooth, store, update or trial.** The firmware is not on the SDK runtime. It links no Wi-Fi, BLE, settings, store or updater code, and arms no watchdog.
- **No flash writes or erases.** The build's write audit refuses the image if any Wellfed code on either core references a flash or eFuse writer. The KM0 main boots KM4 and nothing else: no boot trial, so it never reads a confirmation record or clears bank 1's marker. The build's startup audit pins that call list.
- **No fallback.** Nothing gives way to bank 0. A bank-1 boot that fails stays failed until the bench recovers it over UART.

A panic clears PA30 again, and the flash-lease fault path clears PA30 and nothing else. The build pins that path's bit masks.

## The pad table

Stock's KM0 pad table pulls PA30 up. So, until stock's KM4 latches the pump LOW, the pump enable sits weakly HIGH. The test image uses stock's table with one deliberate change: PA30 is pulled down. Every other pad matches stock, including PA22, which stock pulls up and keeps in sleep, unlike the SDK's default table.

The build checks the linked table byte for byte and pins its SHA-256. Its audit records `matches_stock: false` because of the PA30 change. The window before our table applies is still stock's: the ROM and the factory loader run first, and they set no pull for us.

## Build and review it

1. Place the fountain's pinned full backup at `private/flash-backups/plwf106-full-3/flash-read.bin`. Its `device.json` names that path, and the build reads stock's pad table from it.
2. Build the image:

   ```sh
   make -C src/firmware/vendor firstboot DEVICE=plwf106 FIRMWARE=fountain
   ```

   You should see the build, then its audits, finish without a refusal. The payload must fit the layout's bank-1 cap of `0xeb000` bytes.

3. Freeze it for the flash writers:

   ```sh
   target/release/wellfed-build review build \
     --build private/firmware-build/plwf106/firstboot \
     --output private/verification/<name>
   ```

   The review re-runs the audits and emulates the factory loader on the fountain's backup with the image alone in each bank.

## Evidence

- **Modeled, 2026-11-04:** the boot test's start, beat and panic paths against the host fake: the pump latched LOW before it becomes an output, no other pin configured, and a pump found on latched LOW again (`wellfed-fw-fountain/tests/boot.rs`, `wellfed-hal/tests/pump_stopped.rs`).
- **Recovered, 2026-11-04:** the stock KM0 pad table at `0x0c012eec` in both banks of our backup. It is the pinned SDK's table with PA22 pulled up and kept in sleep, and it pulls PA30 up.
- **Modeled, 2026-11-04:** static analysis of the built image by the build's audits: payload `0x1b000` bytes, task stack bound 1280 of 4096 bytes, no Wellfed reference to a flash or eFuse writer on either core, and the factory loader emulated to accept it in either bank. The image has not run on a fountain.
