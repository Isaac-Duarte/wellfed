# PLWF116 stock firmware 0.3.62

How the cordless fountain's stock application controls water, sensing and indication, for developers reviewing a replacement. Read it with the [board signals](../board.md), [power management](../power.md), [image layout](../flash-layout.md) and [OTA contract](ota-requirements.md).

## Reading the analysis

These are **Recovered** findings from the supplied OTA 000362, not observations of a running fountain. The application identifies itself as `0.3.62`, built `Oct 15 2026 16:55:09`. The URL's later date is not the application's build date.

Addresses are little-endian Thumb KM4 runtime addresses. Map both the XIP and RAM bodies before following calls. The ROM has no bytes in this OTA file, so a call through a ROM pointer needs the SDK contract and the instruction listing. Pseudocode alone can omit arguments or misread floating-point event values. Unknown flash contents, peripheral state and physical parts stay unknown.

## Startup and workers

Application startup `0x0e001df4` starts the monotonic timing support and power support, initializes the board, loads stored state and starts Wi-Fi. Start task `0x0e001d20` creates the GPIO queue, communications, water control, OTA and factory workers. These local workers start before its wait for Wi-Fi configuration to finish.

Stored-state initialization `0x0e001da0` loads attributes and power classification, scale calibration, report logs, clock and identity. Board initialization is `0x0e0084f4` ([pin map](../board.md#initialization-and-control-anchors)).

| Worker or callback | Entry | Recovered timing or role |
| --- | --- | --- |
| Water control | `0x0e00d578` | Scale read followed by a nominal 100 ms delay |
| Weight reporting | `0x0e00d200` | Nominal 50 ms polling; reads when a reporting flag requests it |
| GPIO queue | `0x0e019128` | Processes button, radar and USB-source edges |
| Device events | `0x0e0196d8` | Nominal 50 ms loop; lights every ten visits, power service every 1,200 visits |
| RTC worker | `0x0e019600` | Nominal 300 ms delay |
| Radar configuration retry | `0x0e015a34` | 3,000 ms software timer |
| Watchdog refresh | `0x0e019688` | Called by the device-event loop, nominally every 50 ms |

Watchdog setup `0x0e0197bc` requests 5,000 ms and enables refresh after initialization. The independently running event loop can refresh it while a scale reader or water worker is stuck. These loop counts are not measured deadlines, and normal suspend changes which workers execute.

The millisecond interpretation comes from the image itself: delay thunk `0x0e0a2a08` forwards its argument unchanged to RTOS delay `0x100103b8`. Scheduler setup `0x1000d670` programs SysTick reload to the core-clock value divided by 1,000, minus one. This establishes nominal ticks, not measured oscillator accuracy or scheduling latency.

## Attributes and defaults

The 107-byte attribute object begins at `0x10023e14`. Loader `0x0e00e2c4` reads it from flash `0x3ef000`; this differs from the corded model. Integers below are little-endian, including unaligned fields.

| Offset | Format | Meaning and initialization |
| --- | --- | --- |
| `0x01` | `u8` | Power source; 1 external without detected battery, 2 battery, 3 external with battery |
| `0x02` | `u8` | Charging state; see [power](../power.md) |
| `0x03` | `u8` | Reported battery percentage |
| `0x06` | `u8` | Effective circulation mode: 0 continuous, 1 interval, 2 radar |
| `0x07` | `u16` | Interval in minutes; default 60 |
| `0x09` | `u16` | Run duration in minutes; default 1 |
| `0x0b` | Byte | Effective lighting state |
| `0x0c` | Byte | Light switch |
| `0x0d` | Byte | Light policy: 1 always, 2 custom window |
| `0x0e..0x11` | Four bytes | Custom window start hour/minute and end hour/minute |
| `0x18` | `u32` | Low-water threshold; default 1,100 |
| `0x1c` | `u32` | Lack-water threshold; default 850 |
| `0x22` | `u64` | Filter replacement timestamp |
| `0x2a` | Byte | Filter reminder switch; default 1 |
| `0x2b` | `u8` | Radar gain; default 3 for detected radar type 2, otherwise 5 |
| `0x2c` | `u16` | Radar threshold; default 45 |
| `0x2e` | `u8` | Radar hardware classification from PA18/PA17 |
| `0x2f` | Byte | `waterStopSwitch`; default 0 |
| `0x30` | `u8` | Radar watering duration in seconds; default 45 |
| `0x31` | `u8` | Cooling policy; only value 1 enables the checked cooling branches; default 2 |
| `0x32` | `u16` | Maximum continuous radar watering, seconds; default 300 |
| `0x34` | `u8` | Cooling duration, seconds; default 60 |
| `0x35` | `u8` | Original user mode, retained while battery operation forces radar mode |

Default initializer `0x0e00e23c` clears the object before setting these values. Startup rejects water thresholds above 10,000 or an invalid light policy by reinitializing the attributes. It separately repairs unset radar and cooling values. Those checks are not a comprehensive record validator.

Radar watering duration is clamped to a default when outside 1..150 in the radar initialization branch. Maximum continuous duration repairs 0 and 65,535 to 300; cooling duration repairs 0 and 255 to 60. Initialization accepts cooling policy 1 or 2, and resets any other value to 1. Defaults therefore disable cooling while invalid stored policy enables it.

The attribute update handler `0x0e016558` records both effective and original mode when `useWaterType` changes. Startup forces the effective mode to radar on battery and restores the original choice on external power. The live update handler has no equivalent power-source gate: a remote update can select continuous or interval mode on battery. The startup choice is not a runtime invariant, and a reported radar mode does not prove that the user selected it.

## Pump control

Both pump output helpers use a RAM shadow. On requests do nothing when the shadow already says on; off requests clear the pins only when it says on. Pump-on `0x0e0085a0` first checks `waterStopSwitch`, and redirects to off when set. Otherwise it sets PB4's boost request according to power source, then sets PA30 HIGH. Off `0x0e008558` clears PB4 and PA30.

### Water checks and modes

The water worker `0x0e00d578` starts with watering inhibited. Each pass reads the scale and compares converted weight with the stored thresholds plus **629**:

1. Below `lack_threshold + 629`, four successive low samples request pump-off and clear the radar keep-on state. The first three samples do not newly declare lack water.
2. At or above that boundary, the lack-water counter resets and watering becomes eligible immediately. Below `low_threshold + 629` it requests the low-water indication; above it, it clears that indication.
3. Mode 0 requests continuous circulation. Mode 1 switches run and interval phases using RTC seconds and settings multiplied by 60. Mode 2 powers the radar and services its keep-on deadline.

With defaults, the lack and low boundaries are 1,479 and 1,729 converted grams. The added 629 is consistent with a vessel allowance, but its physical meaning and correctness are **Unknown**. These are not measured water volumes or safe dry-run limits.

Four samples mean at least four scale transactions, each followed by the worker delay. They do not establish a fixed 400 ms shutdown deadline. A blocked ADC-ready wait prevents the comparison from running. There is no comparable multi-sample recovery delay after the weight rises above the lack boundary.

Interval mode starts in its off phase with a zero-initialized deadline, so an eligible first pass ordinarily requests on and sets a run deadline. These deadlines use the wall clock, unlike a monotonic safety deadline. Clock corrections, suspend and scheduling can change their elapsed duration.

### Radar watering and cooling

GPIO worker `0x0e019128` interprets PA14 LOW as detection and acquires radar wakelock 23. It updates radar event state, invokes `0x0e00d968`, optionally reports the event, and requests the serialized Wi-Fi reconnect path. HIGH clears the detection event and releases the lock.

Radar handler `0x0e00d968` also runs on suspend and resume. It checks a pending detection and its current state, extends the keep-on deadline to current RTC seconds plus `sensingWaterDuration + 2`, and requests an active interval of `sensingWaterDuration * 1000 + 100` ms. Keep-on ownership uses wakelock 31 through `0x0e00d484`.

Stop helper `0x0e00d4ac` clears expired keep-on state and requests off. When cooling is enabled, it checks the continuous-run start against the maximum duration. The handler and ordinary water worker both have maximum-run checks; entering cooling clears keep-on, requests off and ignores radar triggers until the cooling deadline.

The low-level pump-on helper checks the stop switch, but does not read weight or check calibration, water freshness or cooling itself. A radar event can therefore request on before the ordinary water worker next evaluates lack water. These separate callers are not one atomic safety owner.

### Factory and remote writers

Factory dispatcher `0x0e017388` can set flag `0x1002c32d`, which makes the ordinary water worker wait in 200 ms increments. The production-test mode does the same. Normal water checks therefore do not run while those modes own the pump.

`ROTOR_IDLING` invokes `0x0e017300`: request on and arm a nominal 3,000 ms off timer. It still passes through the stop-switch check, but not the worker's scale checks. Factory aging can force continuous mode through flag `0x1002c32c`. Remote `waterPumpState` handler `0x0e017020` likewise calls the pump helpers directly; ordinary policy can overwrite that output later.

## Scale transaction and filtering

Raw reader `0x0e036e0c` shares a mutex with the other scale users:

1. Wait indefinitely for the mutex, drive PB5 LOW and wait indefinitely for PB6 LOW. Readiness polling uses a short busy delay.
2. Clock 24 bits, sampling PB6 after each falling edge and shifting most significant bit first.
3. Send one extra clock pulse and XOR the result with `0x800000`.
4. Release the mutex before updating the filter history.

The bit loop has no explicit pulse delay or interrupt masking. Consecutive GPIO calls set its timing. The log calls the part HX717; no part marking or timing measurement confirms that identity.

The filter keeps ten samples, sorts a copy, removes the highest two and lowest two, then averages six. Empty history slots are filled with the current reading. A change of at least 400 counts clears history for the next pass but returns the current result; it is not a rejection of that reading.

Two workers call the converted reader. The mutex covers the wire transaction, while filter and calibration state change after release. The reviewed code therefore does not establish serialization of the entire conversion and calibration operation.

### Conversion and calibration

Converter `0x0e036f40` normally returns:

```text
max(filtered_adc - stored_zero, 0) / stored_coefficient
```

| Flash offset | Record | Ordinary accepted state |
| --- | --- | --- |
| `0x7ce000` | State byte, then unaligned little-endian `u32` zero | 2 |
| `0x7cf000` | State byte, then unaligned little-endian `u32` coefficient | 6 |

Initialization `0x0e036c40` supplies coefficient 314 for an empty all-zero record or an erased coefficient word. It does not reject every malformed record or a zero coefficient inside an otherwise nonempty record. A replacement must validate before division and preserve factory calibration.

Zero calibration changes state 0 to 1 and acquires wakelock 20. Coefficient calibration changes state 4 to 5. During the early sample window, a difference greater than 1,000 counts from the preceding sample aborts the relevant calibration. At sample count 3 it captures zero or computes `(reading - zero) / 1500`, then persists the record. That divisor suggests a 1,500 g reference load; it is not a validated calibration procedure.

The explicit acceptance command changes the coefficient state to 8. A later conversion persists the remembered nonzero coefficient and returns to state 6. The third-sample coefficient branch already writes its record; acceptance is not the only persistence point in this build. This differs from the corded analysis and must be reviewed independently.

Scale sleep `0x0e036d74` attempts the mutex without waiting. If available, it waits for data ready, sends 29 complete HIGH/LOW pulses, then drives the clock HIGH and marks sleep. Its ready wait is unbounded too. The [power reference](../power.md) covers the consequences for suspend and low-battery shutdown.

### Weight telemetry

Table `0x0e0ad9a2` maps the first inclusive upper bound to a percentage:

| Upper bound in converted grams | Percentage |
| --- | --- |
| 350 | 10 |
| 700 | 20 |
| 1,050 | 30 |
| 1,400 | 40 |
| 1,750 | 50 |
| 2,100 | 60 |
| 2,450 | 70 |
| 2,800 | 80 |
| 3,150 | 90 |
| 3,500 | 100 |

Ordinary reports above the table return 100; zero is in the 10% bucket. This does not establish a 3.5 L usable capacity or a calibrated fill percentage.

Reporting worker `0x0e00d200` accumulates timestamped records containing converted weight, percentage and radar state. Its normal externally powered batch target is 45 samples, while battery, interval and factory branches choose other targets. Without a nonzero synchronized timestamp it waits and logs instead of producing the ordinary record. A weight drop can be drinking, vessel removal or another load change; the raw report alone does not distinguish them.

## Radar configuration

Hardware classifier `0x0e007ea0` reads `(PA18, PA17)`: `00` gives type 0, `10` type 1, `11` type 2 and the remaining combination type 4. This is software identification, not a decoded radar part number.

Configuration `0x0e02a6bc` sends eight bytes on the selected left or right control line: a seven-byte frame beginning `55 05 0b 03`, then gain and the threshold high and low bytes, followed by their byte-sum. Bits travel least significant first. A zero uses a nominal 800/1,600 delay-unit HIGH/LOW pair, and a one reverses those durations. Acknowledgment returns the line to input and requires LOW twice, separated by 50 delay units, within 120 polling attempts.

Initialization programs both sides and retries a failing side once. Attribute changes start a 3,000 ms retry timer with a bounded counter and wakelock 30; callback `0x0e015a34` programs both sides and stops the timer after success or exhaustion. No radar range, sensitivity calibration or waveform was measured.

## Button, reset and indicators

PA15 LOW reports pressed; HIGH reports released. Holding the reported pressed state for 60 device-event visits requests reset reporting, waits 200 ms, requests Wi-Fi clearing and waits another 500 ms. The normal loop delay makes this nominally three seconds, not a hard hold deadline. `MicroSw_Debounce` logs a callback name but establishes no actual debouncing.

Wi-Fi clear helper `0x0e006a28` erases sectors at `0x3e8000`, `0x3ef000`, `0x3e9000` and `0x7d3000`, then requests restart. Attributes-only helper `0x0e006f04` erases `0x3ef000`. Neither reviewed path clears the scale records at `0x7ce000` or `0x7cf000`. A whole-flash erase would lose more than a stock settings reset.

Light policy `0x0e012350` runs every ten event-worker visits. It applies the light switch and always-on or custom-window policy, drives network indication and evaluates the filter reminder. Window helper `0x0e012108` accepts both endpoints, handles a midnight crossing and treats equal start and end times as always inside. Unknown or unsynchronized wall-clock behavior remains a porting decision.

Network indication uses PA26/PA28, low and lack water PB3/PA25, and the filter PB7. Battery indication uses PB26/PB23 ([power](../power.md)). Factory tests bypass ordinary indication. Physical colors and PB22's ordinary role remain **Unknown**.

## What a replacement must establish

- One pump owner must check stop, fresh valid scale data, water state, cooling and update holds before every on request, including factory and radar requests.
- Bound scale-ready and mutex waits. Supervision must prove pump-owner progress, not just refresh from an unrelated worker.
- Preserve calibration and validate zero, coefficient and thresholds before using them. Resetting settings must not silently recalibrate.
- Review power transitions, converter control, wake sources and stopped states together. A corded board's safe-output mask does not cover the cordless boost output.
- Define monotonic run deadlines, refill recovery, clock correction, fault behavior and reporting before treating stock's timing as a product contract.

These findings establish code behavior, not electrical safety, successful recovery, dry-run tolerance or sustained wet-operation reliability.

## Evidence

- **Recovered, 2026-11-16:** static Ghidra analysis of PLWF116 OTA 000362, stock 0.3.62. Application and RAM segments were mapped from the OTA headers; GPIO arguments, threshold additions, sample counts, pulse counts and table bytes were checked against listings or original bytes.
- **Recovered, 2026-11-16:** direct review of startup, water and reporting workers, scale filtering and calibration, radar configuration and interrupts, indicator policy, hold detection and reset paths. No firmware was run, no pump operated and no physical timing or load response measured.
