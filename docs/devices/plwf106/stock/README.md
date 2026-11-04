# PLWF106 stock firmware 0.0.17

This page explains how the stock fountain firmware behaves, with the addresses to start from if you analyze it yourself. It is for developers porting Wellfed to the fountain. Read it with the [board](../board.md), [flash layout](../flash-layout.md) and [stock OTA](ota-requirements.md) pages. The findings come from the bank-0 image. Bank 1 carries the same version text but different code, so check it separately.

## Reading the analysis

Addresses are KM4 runtime addresses unless a table labels them flash offsets. The application is little-endian Thumb code.

- Map both the XIP and the RAM bodies from the [segment table](../flash-layout.md#application-segments) before you follow calls.
- Use Ghidra's ARMv8-M language, with Thumb context set in the code regions.
- Leave peripheral registers and unread ROM or RAM empty. Don't fill them with invented bytes.
- Read the instruction listing for any argument that sets a GPIO level, an erase address or a length. The pseudocode can drop arguments, because ROM signatures are missing and the compiler passes some values in registers across calls.
- The secure loader's write at `0x1007d922` stays undecoded even with ARMv8-M. Keep its original bytes rather than patching them for cleaner pseudocode.

These pages describe the firmware in our own words. They hold no vendor binaries, credentials, captures or decompiled source. Without your own backup, you can follow the contracts here but cannot check them against the image.

## Startup and task structure

Startup `0x0e0015c8` initializes the board, stored data, Wi-Fi/setup and application tasks. Stored-data initialization `0x0e001574` loads attributes, weight calibration, logs and identity. Task creation `0x0e00150c` starts button handling, communications, weight/pump workers, OTA and factory tests. Local watering tasks are created independently of a cloud connection.

SysTick setup `0x1000ca98` programs `SystemCoreClock / 1000 - 1`. The application delay thunk `0x0e08f0d8` reaches RTOS delay `0x1000f31c`, so its configured units are milliseconds.

| Worker / callback | Entry | Normal delay or configuration |
| --- | --- | --- |
| Pump owner loop | `0x0e009848` | 50 ms |
| Weight sampling and accumulated logs | `0x0e009aac` | 200 ms before reading |
| Light/button-hold worker | `0x0e01287c` | 50 ms; light policy every ten visits |
| RTC worker | `0x0e01279e` | 300 ms |
| Watchdog callback | `0x0e0127b0` | 2,000 ms refresh; watchdog initialized for 5,000 ms |
| Communications owner | `0x0e008cdc` | Handles events and settings/reset requests |

These waits do not establish real-time deadlines. ADC readiness and scheduling add delay. The watchdog timer runs independently of sensor and pump progress; a refreshed watchdog does not prove either worker is healthy.

## Attributes and stored settings

Attributes load into the object at `0x1001f6f4` from flash `0x3ea000`. The reviewed read is 111 bytes. Offsets below are relative to that object; integers are little-endian.

| Offset | Format | Meaning |
| --- | --- | --- |
| `0x0b` | `u8` | Circulation mode: 0 continuous, 1 interval |
| `0x0c` | `u16` | Interval, expressed as nominal minutes |
| `0x0e` | `u16` | Circulation duration, expressed as nominal minutes |
| `0x1c` | `u32` | Low-water threshold |
| `0x20` | `u32` | Lack-water threshold |
| `0x2f` | Byte flag | `waterStopSwitch` |

The owner's saved configuration used mode 0, interval/duration 15 and thresholds 1,100/850. These are observed stored settings, not proven universal defaults or recommended physical limits.

## Pump policy and competing writers

Pump PA30 is HIGH/on and LOW/off. Initialization writes LOW before enabling the output. The ordinary owner at `0x0e009848` reads the converted gram value at `0x1001f6e4`.

1. Aging flag `0x1002275e` or production-test flag `0x1001fe92` selects a factory branch instead of ordinary water-level control. `ROTOR_IDLING` at `0x1001f6e1` requests a nominal three-second pump pulse there.
2. In the ordinary branch, `waterStopSwitch` writes LOW and waits 200 ms.
3. The lack-water comparison uses `lack_threshold + 609`. After 61 consecutive low loop visits, nominally 3.05 seconds, it requests LOW unless the factory override is set.
4. Between lack and low comparison points, it selects low-water indication; above the low point, normal indication. Both use their threshold plus 609. During the initial lack debounce, stock may still reach the watering-mode branch: low water is not an immediate startup inhibit.
5. Mode 0 requests continuous circulation. Mode 1 counts duration/interval phases against their settings multiplied by 1,200 visits. At 50 ms per visit this is nominal minutes. It begins in its duration phase without explicitly driving HIGH on phase entry, so the first phase depends on prior output state.

With the reviewed settings, actual comparisons are 1,709 and 1,459 converted grams. A vessel/tare explanation for 609 is an inference; no measurement establishes it.

The factory command handler `0x0e011418` handles `PUMP_ON` and `PUMP_OFF`. Both set override byte `0x10022714` to 1. The ordinary owner then stops changing PA30, including its lack-water stop. Reviewed exact-address references find no explicit clearing write in bank 0; initial zeroed BSS clears it on restart. This bounded search cannot exclude every computed write.

The regular `waterPumpState` handler `0x0e011160` writes PA30 directly without setting that override. The owner can overwrite it on a later visit. Treat it as a competing output writer, not an independent persistent mode.

## Scale transaction and filtering

Driver `0x0e024e48` uses PB5 as clock and PB6 as data:

1. Drive clock LOW and wait until data is LOW. Stock has no timeout in this wait.
2. Delay one RTOS tick before the bit transaction.
3. For 24 bits, drive clock HIGH then LOW and sample data after the falling edge. Shift the accumulated value left and add the sampled bit.
4. Send one additional HIGH/LOW pulse and XOR the 24-bit result with `0x800000`.

The loop at `0x0e024e6a..0x0e024e96` has consecutive ROM GPIO calls. It has no explicit pulse delay or interrupt masking in that section. GPIO-call overhead contributes to pulse width; replacing it with faster register writes does not preserve timing automatically. No measured timing or gain setting is established, and the unmarked U1 must remain unidentified.

The transaction matches the HX711/HX710 family's two-wire protocol: data falls when a conversion is ready, then 24 bits are read most significant first. On the HX711, the 25th pulse selects channel A at gain 128 for the next conversion. That narrows U1 to this family or a compatible clone; it does not establish the part or its gain.

Stock keeps ten samples, sorts a copy, drops the two highest and two lowest, then averages the middle six. Zero entries are filled during history initialization. A change of at least 400 ADC counts from the previous filtered value clears history; that reset still returns the current filtered result. Do not interpret it as a rejected sample.

Conversion `0x0e024f88` ordinarily returns:

```text
max(filtered_adc - stored_zero, 0) / stored_coefficient
```

### Calibration records

| Flash offset | Layout | Accepted-state value seen in stock |
| --- | --- | --- |
| `0x7ce000` | State byte, then unaligned little-endian `u32` zero | 2 |
| `0x7cf000` | State byte, then unaligned little-endian `u32` coefficient | 6 |

Initialization is `0x0e024d34`. The coefficient fallback in stock is 314. A replacement must reject erased/invalid records and zero coefficients before division; this reference does not establish a safe numerical bound for every installed load cell.

Calibration logic in `0x0e024f88` advances a sample counter and reacts to changes over 1,000 ADC counts during its early window. It captures zero at count 10 and derives scale at count 20 by dividing the difference by 1,500. A 1,500 g reference load is inferred from the arithmetic, not a validated procedure. A separate acceptance state persists the nonzero scale; deriving a coefficient and durably accepting it are distinct operations.

Retained stock boot logs corroborate the saved zero/scale decoding. Their 40 and 81 reported weight samples were all zero. They do not prove known-load response, sensor polarity, readiness timing or a valid physical water threshold. Preserve factory records; do not recalibrate as a side effect of startup or settings reset.

### Telemetry percentage

The table at `0x0e09bb90` maps converted grams to a reported percentage. Select the first upper bound that includes the weight:

| Upper bound in converted grams | Percentage |
| --- | --- |
| 250 | 10 |
| 500 | 20 |
| 750 | 30 |
| 1,000 | 40 |
| 1,250 | 50 |
| 1,500 | 60 |
| 1,750 | 70 |
| 2,000 | 80 |
| 2,250 | 90 |
| 3,000 | 100 |

Above the final bound also reports 100. Zero falls in the 10% bucket. The final bound is 3,000, not 2,500; this table does not prove reservoir capacity or calibrated fill percentage.

Weight helper `0x0e0135d4` batches timestamped records; factory aging changes reporting cadence. A weight change alone does not establish drinking because vessel removal and refilling also change load.

## Button, reset and indication

Queue initialization `0x0e012660` configures both edges on PA15. Handler `0x0e0124e8` publishes internal button value 1 for LOW and 2 for HIGH. Long-hold checker `0x0e00dc20` acts after 60 worker visits, nominally three seconds. The empty `MicroSw_Debounce` callback does not establish debouncing.

The hold chain requests reset reporting through `0x0e009570`, waits 200 ms, then requests Wi-Fi clear through `0x0e00953c`. Communications worker `0x0e008cdc` runs clear helper `0x0e00601c`. It erases the sectors at `0x3e8000`, `0x3e9000` and attributes `0x3ea000..0x3ec000`, clears in-memory Wi-Fi, disconnects and requests reset after 1,000 ms.

Separate attribute-reset handler `0x0e011230` clears the two attribute sectors and reloads defaults. Neither reviewed path erases weight zero/scale. Reset scope therefore differs from erasing the entire stock store.

Light policy `0x0e00db08` runs every ten 50 ms worker visits. It supports enabled/disabled, always-on or a time window, and filter-reminder timestamps. Network policy uses PA26/PA28, water status PA19/PB3/PA25, and filter reminder PB7. Factory/LED tests bypass ordinary updates. Physical colors, time-zone edge behavior and the other three startup outputs remain unconfirmed.

## Communications surface and radar limit

The recovered field names are useful search anchors, not a complete wire protocol or evidence that the PLAF103 stock server supports this fountain:

| Area | Recovered names |
| --- | --- |
| Circulation settings | `useWaterType`, `useWaterInterval`, `useWaterDuration`, `waterStopSwitch`, `lowWater` |
| Lighting | `enableLight`, `lightSwitch`, `lightAgingType`, `filterLedSwitch` and time-window fields |
| Filter reminder | `filterNextReplacementTimestamp` |
| Device telemetry | `DEVICE_DATA_EVENT`: `weight`, `radarState`, `waterPumpState`, `buttonState` |
| Capacity telemetry | `weightPercent` |
| Calibration services | `WEIGHT_CLR_SERVICE`, `WEIGHT_CAL_SERVICE` |

Generic event encoder `0x0e00ef68` is reached from communications worker `0x0e008cdc`. Event state begins at `0x100246c8`; radar slot 1's double value at `0x100246e8` initializes to 2. The ten recovered direct calls to setter `0x0e00df10` set slots 2, 5, 13, 14 and 15, never slot 1. A literal-pointer scan finds no stored pointer to either setter entry address. This does not exclude computed pointers or indirect state writes.

No radar producer or input driver was established. The photographed RADAR footprints are unpopulated, so this is not a missing driver for a connected module on the inspected unit. Bank 1 has no radar string; absence of a string alone would not prove absence of hardware.

The image's strings place it on the same vendor framework as stock PLAF103 1.1.15, beyond the shared SoC:

- **Topics:** the same `dl/%s/%s/device/%s/%s` format, `post` channel suffix and all-device and broadcast subscriptions.
- **Shared messages:** every message name Wellfed's stock server uses on the feeder exists here too: `BINDING`, `DEVICE_START_EVENT`, `ATTR_PUSH_EVENT`, `ATTR_GET_SERVICE`, `ERROR_EVENT`, `DEVICE_LOG_REPORT_EVENT`, `OTA_INFORM`, `OTA_UPGRADE` and `OTA_PROGRESS`. So does every JSON key of that server's shared core, apart from the feeding-plan keys. The two images also share `ATTR_SET_SERVICE`, `NTP_SYNC`, `DEVICE_CONFIG_SYNC` and the Wi-Fi services.
- **Fountain-only messages:** `WATER_CAP_EVENT`, `WEIGHT_CAL_SERVICE`, `WEIGHT_CLR_SERVICE`, `DISPLAY_MATRIX_SERVICE`, `DEVICE_FUNCTION_TEST_SERVICE` and `DEVICE_PRINT_LOG_SWITCH_SERVICE`. The feeding, grain and audio messages are absent.
- **Setup:** the same softAP UDP setup task (`Ap_Config_Udp_Thread`) and the same Bluetooth configuration events (`BLE_CONFIG_NETWORK_BIT`, `BLE_CONFIG_DEV_INFO_BIT`, `BLE_WRITE_DEV_INFO_BIT`).

Matching names are strong grounds for porting the shared core, not proof of identical behavior. The setup frame's parser, each message's required fields and replies, the broker login and the Bluetooth services still need code review or a capture before a stock server serves this model.

## What a port must change

The recovered code supplies driver and policy entry points. A replacement should make its safety policy explicit instead of copying stock's bypasses:

- Give one owner all pump requests, including diagnostics. Check calibration, fresh sensor data, water state, stop requests and flash holds before enabling PA30.
- Bound ADC-ready waits and stop when samples or owner progress stall. An unrelated periodic watchdog refresh is insufficient.
- Keep pump LOW before output enable and during panic, update holds, trial boot and cross-core flash operations. Verify latch/pad readback where the SDK supports it.
- Define interval phase entry, refill/restart stability, debounce, unknown-clock behavior and reset scope. Use monotonic deadlines rather than nominal loop counts.
- Preserve factory zero/scale and all other stock material outside a reviewed write allowance. A new store requires a separate allocation review.

Existing integration points are the board contract (`wellfed-board`, which selects the board crate), `src/sdk/wellfed-amebad-sys/src/target.rs`, the feeder's drivers in `wellfed-hal-feeder`, the HAL stopped state, cross-core flash hooks and `src/sdk/wellfed-hal-common/src/trial.rs`. The target reads the board's storage bounds and `safe` output masks through the contract; the PLAF103's masks do not clear PA30. Select fountain masks/storage throughout, preserve required C exports, and rerun link, RAM-park and stack audits for the resulting image.

A full port also needs typed fountain product/settings values and shared-client bindings where exposed. Feeder portions, infrared sensors and jam/reverse logic do not describe circulation. Hardware acceptance must establish sensor timing/load response, physical lamp roles, stopped boot, recovery, low-water shutdown, sensor-disconnect behavior and sustained wet operation. These docs establish no new device-operation authorization.

## Evidence

- **Recovered, 2026-11-03:** direct Ghidra review of stock 0.0.17 task creation, timing, GPIO, scale/filter/calibration, settings, reset, indication and event paths. Instruction-sensitive conclusions were checked in listings; ten setter call sites were also checked against original instruction bytes.
- **Observed, reviewed 2026-11-03:** retained stock UART logs corroborate stored calibration and thresholds; all recorded weight reports in the two sampled captures are zero. Owner photos show the unmarked U1 and unpopulated RADAR footprints.
- **Recovered, 2026-11-03:** comparison with the existing feeder SDK identifies pin/storage coupling and missing PA30 stops. No new firmware implementation, simulation result or physical timing/safety test is claimed.
- **Recovered, 2026-11-04:** a comparison of bank 0's KM4 strings with a stock PLAF103 backup's, and with the message names and JSON keys Wellfed's stock server uses. String analysis only; no code paths were traced.
