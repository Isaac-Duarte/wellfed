# PLWF106 stock firmware 0.0.17

How the stock fountain firmware behaves, with the addresses to start from, for developers porting Wellfed to the fountain. Read it with the [board](../board.md), [flash layout](../flash-layout.md) and [stock OTA](ota-requirements.md) pages. The findings come from the bank-0 image; bank 1 carries the same version text but different code, so check it separately.

## Reading the analysis

Addresses are KM4 runtime addresses unless a table labels them flash offsets. The application is little-endian Thumb code.

- Map both the XIP and RAM bodies from the [segment table](../flash-layout.md#application-segments) before following calls.
- Use Ghidra's ARMv8-M language, with Thumb context set in the code regions.
- Leave peripheral registers and unread ROM or RAM empty, not filled with invented bytes.
- Read the instruction listing for any argument that sets a GPIO level, an erase address or a length. The pseudocode can drop arguments: ROM signatures are missing, and the compiler passes some values in registers across calls.
- The secure loader's write at `0x1007d922` stays undecoded even with ARMv8-M. Keep its original bytes rather than patching them for cleaner pseudocode.

These pages describe the firmware in our own words, with no vendor binaries, credentials, captures or decompiled source. Without your own backup you can follow the contracts but not check them against the image.

## Startup and task structure

Startup `0x0e0015c8` initializes the board, stored data, Wi-Fi and setup, and the application tasks. `0x0e001574` loads the attributes, weight calibration, logs and identity. Task creation `0x0e00150c` starts button handling, communications, the weight and pump workers, OTA and factory tests. The local watering tasks start whether or not the cloud connects.

SysTick setup `0x1000ca98` programs `SystemCoreClock / 1000 - 1`, and the delay thunk `0x0e08f0d8` reaches RTOS delay `0x1000f31c`, so delays are in milliseconds.

| Worker / callback | Entry | Normal delay or configuration |
| --- | --- | --- |
| Pump owner loop | `0x0e009848` | 50 ms |
| Weight sampling and accumulated logs | `0x0e009aac` | 200 ms before reading |
| Light/button-hold worker | `0x0e01287c` | 50 ms; light policy every ten visits |
| RTC worker | `0x0e01279e` | 300 ms |
| Watchdog callback | `0x0e0127b0` | 2,000 ms refresh; watchdog initialized for 5,000 ms |
| Communications owner | `0x0e008cdc` | Handles events and settings/reset requests |

These waits are not deadlines: ADC readiness and scheduling add delay. The watchdog runs independently of sensor and pump progress, so its refresh proves neither worker healthy.

## Attributes and stored settings

Attributes load into the object at `0x1001f6f4` from flash `0x3ea000`, a 111-byte read. Offsets are relative to that object; integers are little-endian.

| Offset | Format | Meaning |
| --- | --- | --- |
| `0x0b` | `u8` | Circulation mode: 0 continuous, 1 interval |
| `0x0c` | `u16` | Interval, expressed as nominal minutes |
| `0x0e` | `u16` | Circulation duration, expressed as nominal minutes |
| `0x1c` | `u32` | Low-water threshold |
| `0x20` | `u32` | Lack-water threshold |
| `0x2f` | Byte flag | `waterStopSwitch` |

The owner's saved configuration used mode 0, interval and duration 15, and thresholds 1,100 and 850: observed settings, not proven defaults or physical limits.

## Pump policy and competing writers

Pump PA30 is HIGH for on, and initialization writes LOW before enabling the output. The owner at `0x0e009848` reads the converted gram value at `0x1001f6e4`:

1. Aging flag `0x1002275e` or production-test flag `0x1001fe92` selects a factory branch, where `ROTOR_IDLING` at `0x1001f6e1` requests a nominal three-second pump pulse.
2. Otherwise, `waterStopSwitch` writes LOW and waits 200 ms.
3. Lack water compares against `lack_threshold + 609`. After 61 consecutive low visits, nominally 3.05 seconds, it requests LOW unless the factory override is set.
4. Between the lack and low points (each threshold plus 609) it shows low water, above them normal. During the initial lack debounce stock may still reach the watering branch, so low water doesn't inhibit startup at once.
5. Mode 0 circulates continuously. Mode 1 counts duration and interval phases against their settings times 1,200 visits (nominal minutes at 50 ms a visit). It begins in its duration phase without driving HIGH, so the first phase depends on the prior output state.

With the reviewed settings the comparisons are 1,709 and 1,459 converted grams. That 609 is the vessel's tare is an unmeasured inference.

Factory handler `0x0e011418` handles `PUMP_ON` and `PUMP_OFF`, and both set override byte `0x10022714` to 1. The owner then stops changing PA30, lack-water stop included. No exact-address reference in bank 0 clears it; zeroed BSS does on restart. A computed write is not excluded.

The `waterPumpState` handler `0x0e011160` writes PA30 directly without the override, and the owner can overwrite it on a later visit: a competing writer, not a persistent mode.

## Scale transaction and filtering

Driver `0x0e024e48` uses PB5 as clock and PB6 as data:

1. Drive clock LOW and wait, with no timeout, until data is LOW.
2. Delay one RTOS tick.
3. For 24 bits, drive clock HIGH then LOW, sample data after the falling edge, shift the value left and add the bit.
4. Send one more HIGH, LOW pulse and XOR the 24-bit result with `0x800000`.

The loop at `0x0e024e6a..0x0e024e96` makes consecutive ROM GPIO calls with no explicit delay or interrupt masking, so call overhead sets the pulse width and faster register writes change the timing. No timing or gain was measured.

The transaction matches the HX711/HX710 family's two-wire protocol: data falls when a conversion is ready, then 24 bits are read most significant first. On the HX711 the 25th pulse selects channel A at gain 128 for the next conversion. That narrows U1 to this family or a clone, without fixing the part or its gain.

Stock keeps ten samples, sorts a copy, drops the two highest and two lowest, and averages the middle six; history initialization fills zeros. A change of at least 400 ADC counts from the previous filtered value clears the history but still returns the current result, so it is not a rejected sample.

Conversion `0x0e024f88` ordinarily returns:

```text
max(filtered_adc - stored_zero, 0) / stored_coefficient
```

### Calibration records

| Flash offset | Layout | Accepted-state value seen in stock |
| --- | --- | --- |
| `0x7ce000` | State byte, then unaligned little-endian `u32` zero | 2 |
| `0x7cf000` | State byte, then unaligned little-endian `u32` coefficient | 6 |

Initialization is `0x0e024d34`, and the coefficient fallback is 314. A replacement must reject erased or invalid records and zero coefficients before dividing; no safe bound for every load cell is established.

Calibration in `0x0e024f88` counts samples and reacts to changes over 1,000 ADC counts in its early window. It captures zero at count 10 and derives the scale at count 20 by dividing the difference by 1,500, which implies a 1,500 g reference load (inferred, not a validated procedure). A separate acceptance state persists the nonzero scale, so deriving a coefficient and durably accepting it are distinct steps.

Retained stock boot logs corroborate the zero and scale decoding. Their 40 and 81 weight samples were all zero, so they show no known-load response, sensor polarity, readiness timing or valid water threshold. Preserve the factory records, and never recalibrate as a side effect of startup or a settings reset.

### Telemetry percentage

The table at `0x0e09bb90` maps converted grams to the reported percentage of the first upper bound that includes the weight:

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

Above the final bound also reports 100, and zero falls in the 10% bucket. The final bound is 3,000, not 2,500; the table establishes neither reservoir capacity nor a calibrated fill.

Weight helper `0x0e0135d4` batches timestamped records, and factory aging changes the cadence. A weight change alone doesn't mean drinking: removing or refilling the vessel changes the load too.

## Button, reset and indication

`0x0e012660` configures both edges on PA15, and handler `0x0e0124e8` publishes button value 1 for LOW and 2 for HIGH. Long-hold checker `0x0e00dc20` acts after 60 worker visits, nominally three seconds. The `MicroSw_Debounce` callback is empty, so no debouncing is established.

The hold requests reset reporting through `0x0e009570`, waits 200 ms, then requests a Wi-Fi clear through `0x0e00953c`. Communications worker `0x0e008cdc` runs clear helper `0x0e00601c`: it erases `0x3e8000`, `0x3e9000` and the attributes at `0x3ea000..0x3ec000`, clears in-memory Wi-Fi, disconnects and requests a reset after 1,000 ms.

Attribute-reset handler `0x0e011230` clears the two attribute sectors and reloads defaults. Neither path erases the weight zero or scale, so a reset is narrower than erasing the whole stock store.

Light policy `0x0e00db08` runs every ten 50 ms worker visits: enabled or disabled, always-on or a time window, and filter-reminder timestamps. Network indication uses PA26/PA28, water status PA19/PB3/PA25, and the filter reminder PB7. Factory and LED tests bypass it. Physical colors, time-zone edge behavior and the other three startup outputs are unconfirmed.

## Communications surface and radar limit

The recovered field names are search anchors, not a complete wire protocol, and no sign that the PLAF103 stock server supports this fountain:

| Area | Recovered names |
| --- | --- |
| Circulation settings | `useWaterType`, `useWaterInterval`, `useWaterDuration`, `waterStopSwitch`, `lowWater` |
| Lighting | `enableLight`, `lightSwitch`, `lightAgingType`, `filterLedSwitch` and time-window fields |
| Filter reminder | `filterNextReplacementTimestamp` |
| Device telemetry | `DEVICE_DATA_EVENT`: `weight`, `radarState`, `waterPumpState`, `buttonState` |
| Capacity telemetry | `weightPercent` |
| Calibration services | `WEIGHT_CLR_SERVICE`, `WEIGHT_CAL_SERVICE` |

Event encoder `0x0e00ef68` is reached from communications worker `0x0e008cdc`. Event state begins at `0x100246c8`, and radar slot 1's double at `0x100246e8` initializes to 2. The ten direct calls to setter `0x0e00df10` set slots 2, 5, 13, 14 and 15, never slot 1, and no stored literal points to either setter entry. Computed pointers or indirect writes are not excluded.

No radar producer or input driver was found, and the RADAR footprints are unpopulated, so no connected module lacks a driver. Bank 1 has no radar string, though that alone wouldn't prove missing hardware.

The image's strings put it on the same vendor framework as stock PLAF103 1.1.15:

- **Topics:** the same `dl/%s/%s/device/%s/%s` format, `post` channel suffix and all-device and broadcast subscriptions.
- **Shared messages:** every message name Wellfed's stock server uses on the feeder (`BINDING`, `DEVICE_START_EVENT`, `ATTR_PUSH_EVENT`, `ATTR_GET_SERVICE`, `ERROR_EVENT`, `DEVICE_LOG_REPORT_EVENT`, `OTA_INFORM`, `OTA_UPGRADE`, `OTA_PROGRESS`), and every JSON key of that server's shared core apart from the feeding-plan keys. Both also have `ATTR_SET_SERVICE`, `NTP_SYNC`, `DEVICE_CONFIG_SYNC` and the Wi-Fi services.
- **Fountain-only messages:** `WATER_CAP_EVENT`, `WEIGHT_CAL_SERVICE`, `WEIGHT_CLR_SERVICE`, `DISPLAY_MATRIX_SERVICE`, `DEVICE_FUNCTION_TEST_SERVICE` and `DEVICE_PRINT_LOG_SWITCH_SERVICE`. The feeding, grain and audio messages are absent.
- **Setup:** the same softAP UDP setup task (`Ap_Config_Udp_Thread`) and Bluetooth configuration events (`BLE_CONFIG_NETWORK_BIT`, `BLE_CONFIG_DEV_INFO_BIT`, `BLE_WRITE_DEV_INFO_BIT`).

Matching names justify porting the shared core but don't prove identical behavior. The setup frame's parser, each message's fields and replies, the broker login and the Bluetooth services need code review or a capture before a stock server serves this model.

## What a port must change

Make the safety policy explicit instead of copying stock's bypasses:

- Give one owner every pump request, diagnostics included, and check calibration, fresh sensor data, water state, stop requests and flash holds before enabling PA30.
- Bound ADC-ready waits, and stop when samples or owner progress stall; an unrelated watchdog refresh is not enough.
- Keep the pump LOW before output enable and during panic, update holds, trial boot and cross-core flash operations, with latch and pad readback where the SDK supports it.
- Define interval phase entry, refill and restart stability, debounce, unknown-clock behavior and reset scope, with monotonic deadlines rather than nominal loop counts.
- Preserve the factory zero and scale and all other stock material outside a reviewed write allowance. A new store needs its own allocation review.

The [device layer](../../../firmware/device-layer.md) lists where each board fact lives. The board contract, `wellfed-board`, selects the board crate, whose storage bounds and `safe` output masks `src/sdk/wellfed-amebad-sys/src/target.rs` reads. The other integration points are `wellfed-hal-feeder`, the HAL stopped state, the cross-core flash hooks and `src/sdk/wellfed-hal-common/src/trial.rs`. `wellfed-board-plwf106`'s safe state clears PA30 only, so a fountain build's startup, panic and flash-lease stops clear the pump, and its storage region has no sectors until an allocation is reviewed. A fountain image must keep the required C exports and rerun the link, RAM-park and stack audits.

A full port also needs typed fountain product and settings values, and shared-client bindings where exposed; feeder portions, infrared sensors and jam logic don't describe circulation. Hardware acceptance must establish sensor timing and load response, the lamp roles, stopped boot, recovery, low-water shutdown, sensor-disconnect behavior and sustained wet operation. These docs authorize no device operation.

## Evidence

- **Recovered, 2026-11-03:** direct Ghidra review of stock 0.0.17 task creation, timing, GPIO, scale/filter/calibration, settings, reset, indication and event paths. Instruction-sensitive conclusions were checked in listings; ten setter call sites were also checked against original instruction bytes.
- **Observed, reviewed 2026-11-03:** retained stock UART logs corroborate stored calibration and thresholds; all recorded weight reports in the two sampled captures are zero. Owner photos show the unmarked U1 and unpopulated RADAR footprints.
- **Recovered, 2026-11-03:** comparison with the existing feeder SDK identifies pin/storage coupling and missing PA30 stops. No new firmware implementation, simulation result or physical timing/safety test is claimed.
- **Recovered, 2026-11-04:** a comparison of bank 0's KM4 strings with a stock PLAF103 backup's, and with the message names and JSON keys Wellfed's stock server uses. String analysis only; no code paths were traced.
