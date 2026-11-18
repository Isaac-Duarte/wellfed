# PLWF116 board signals

The GPIO assignments in stock 0.3.62, for reviewing a cordless fountain port. These are **Recovered** software roles. No PLWF116 continuity, voltage, current or startup waveform was measured.

## GPIO assignments

SDK numbers are `(port << 5) | pin` ([chip reference](../../chips/rtl8721dm/README.md#pins)). A startup level is the software latch value. It does not establish the voltage before firmware takes control.

| Signal | SDK number | Pin | Recovered configuration and use |
| --- | --- | --- | --- |
| External-power sense | 13 | PA13 | Input, no internal pull; LOW selects an externally powered classification, HIGH battery operation |
| Charge-complete input | 12 | PA12 | Input, no internal pull; HIGH is checked again with battery voltage before reporting completion |
| Reset button | 15 | PA15 | Input, no internal pull; active LOW |
| Radar detection | 14 | PA14 | Input, no internal pull; active LOW |
| Radar power control | 16 | PA16 | Output, active LOW; initialization pulses HIGH then LOW |
| Left radar configuration | 19 | PA19 | Bidirectional one-wire configuration; output HIGH between configuration transactions |
| Right radar configuration | 33 | PB1 | Bidirectional one-wire configuration; output HIGH between configuration transactions |
| Radar hardware identification | 18, 17 | PA18, PA17 | Inputs, no internal pull; two-bit classification read during attribute initialization |
| Pump boost control | 36 | PB4 | Output, LOW at initialization; HIGH before pump enable on battery |
| Pump enable | 30 | PA30 | Output, LOW at initialization; HIGH requests circulation |
| Battery measurement | 34 | PB2 | ADC input; conversion and calibration are on the [power page](power.md#battery-adc-and-voltage-conversion) |
| Scale clock | 37 | PB5 | Output; initialization pulses LOW, HIGH, LOW |
| Scale data | 38 | PB6 | Input, no internal pull; LOW signals data ready |
| Network indication pair | 26, 28 | PA26, PA28 | Outputs, LOW at initialization |
| Low-water indication | 35 | PB3 | Output, LOW at initialization |
| Lack-water indication | 25 | PA25 | Output, LOW at initialization |
| Filter reminder | 39 | PB7 | Output, LOW at initialization |
| Battery low / red indication | 58 | PB26 | Output, LOW at initialization; role named by stock's timer |
| Battery charging / white indication | 55 | PB23 | Output, LOW at initialization; role named by stock's timer |
| Additional indicator | 54 | PB22 | Output, LOW at initialization; physical role **Unknown** |

The red and white names come from firmware strings, not observed lamp colors. The eight-output factory cycling test includes both battery indicators and PB22. It does not establish their physical arrangement.

## Initialization and control anchors

Addresses are KM4 runtime addresses in the supplied OTA image. ROM pointers include the Thumb bit.

| Address | Role |
| --- | --- |
| `0x0e0084f4` | Board initialization sequence |
| `0x0e033890` | Clears bit 0 of `0x48000230`, matching the SDK's SWD release |
| `0x0e007f90` | Button input initialization |
| `0x0e00849c` / `0x0e0084d0` | Power-sense / charge-complete input initialization |
| `0x0e007fd4` | Eight indicator outputs, LOW before output enable |
| `0x0e008374` | Radar input, enable and configuration-pin initialization |
| `0x0e008124` | Battery ADC and calibration initialization |
| `0x0e007e2c` | Pump boost and enable LOW before output enable |
| `0x0e008260` | Scale GPIO initialization |
| `0x0e008558` / `0x0e0085a0` | Pump off / on helpers |
| `0x0e00841c` / `0x0e00845c` | Radar power on / off helpers |
| `0x0e0194a4` | Both-edge interrupt registration for PA15, PA14 and PA13 |
| `0x0e015b48` | Eight-output indicator cycling test |
| `0x1010c871` / `0x1010c84d` | ROM GPIO write / read pointers |

Board initialization releases SWD, configures the inputs and indicators, starts the radar, configures battery measurement, then configures the pump and scale. The pump helper writes PB4 LOW before enabling that output, then does the same for PA30. Radar startup occurs before this pump initialization; it is not a proof of stopped electrical behavior throughout boot.

## Pump and radar electrical limits

Pump-on requests set PB4 HIGH only when `powerType == 2`, then set PA30 HIGH. Externally powered operation sets PB4 LOW before PA30 HIGH. Pump-off clears both. These roles justify the names enable and boost; they do not identify the transistor, converter, pump voltage or current rating.

The helpers track their state in RAM. An off request writes the pins only if the pump shadow says it is on. Radar control similarly avoids a write when its shadow already matches. A replacement's emergency stop should drive and verify its output state directly, including after an interrupted startup.

The radar driver configures separate left and right control lines, then returns each to input while awaiting acknowledgment. This establishes two software configuration endpoints. It does not identify the radar chip, frequency, detection range or the number of physical modules.

## Differences from PLWF106

The corded board's pin map cannot serve as a complete cordless board contract:

| Pin or group | PLWF106 stock role | PLWF116 stock role |
| --- | --- | --- |
| PA19 | Normal-water indicator | Left radar configuration |
| PB1 | No role established | Right radar configuration |
| PB4 | No role established | Pump boost |
| PA12 / PA13 | No role established | Charge-complete / power-source inputs |
| PA14 / PA16 | No role established | Radar detection / power control |
| PB2 | No role established | Battery ADC |
| PB26 / PB23 / PB22 | Startup HIGH, roles unknown | Startup LOW; battery red / white and another indicator |

Both images use PA30 for the pump, PA15 for reset and PB5/PB6 for the scale. Shared assignments do not prove identical electronics. The PLAF103 feeder conflicts with still more of these signals: its motor uses PA13/PA14, its reset input is PB4 and its scale pins are panel buttons.

## Connectors and parts

The supplied evidence is an OTA file, with no photographs or electrical measurements of a PLWF116. PCB revision, connector pinouts, battery chemistry and capacity, charge-controller identity, protection circuitry, load-cell ADC part and pump-drive parts are **Unknown** here. The scale driver names HX717 in a log, but that is not a verified package marking.

Do not turn the PLWF106's observed empty BAT and RADAR footprints into PLWF116 population claims. Likewise, its photographed UART header is not a measured PLWF116 header.

## Evidence

- **Recovered, 2026-11-16:** static analysis of supplied PLWF116 OTA 000362, application version 0.3.62. GPIO initialization arguments, pump writes, radar polarity and interrupt assignments were checked against Thumb listings and literal-pool bytes.
- **Recovered, 2026-11-16:** comparison with the separately documented PLWF106 stock 0.0.17 assignments. No PLWF116 physical operation or simulation was performed.
