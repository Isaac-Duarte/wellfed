# PLWF106 board signals

Use this reference when implementing the fountain's board layer. Signal assignments are recovered from stock initialization and callers. They are not continuity or voltage measurements. The [shared pin table](../../chips/rtl8721dm/pins.md) compares models.

## GPIO assignments

SDK numbers use `(port << 5) | pin`. A startup level is the software latch value, not a measured output voltage.

| Signal | SDK number | Pin | Stock configuration / startup |
| --- | --- | --- | --- |
| Pump enable | 30 | PA30 | Output, LOW/off, no internal pull |
| Reset button | 15 | PA15 | Input, no internal pull; active LOW in the handler |
| Scale clock | 37 | PB5 | Output, LOW, no internal pull |
| Scale data | 38 | PB6 | Input, no internal pull |
| Network indication pair | 26, 28 | PA26, PA28 | Outputs, LOW |
| Normal-water indication | 19 | PA19 | Output, LOW |
| Low-water indication | 35 | PB3 | Output, LOW |
| Lack-water indication | 25 | PA25 | Output, LOW |
| Filter reminder | 39 | PB7 | Output, LOW |
| Additional LED-group outputs | 58, 55, 54 | PB26, PB23, PB22 | Outputs, HIGH; active roles unknown |

The six identified indicators select HIGH when enabled. Their physical colors are unknown. Stock's cycling LED test covers only those six. Do not infer RGB or decorative-light functions for the last three outputs. Bank 1 independently agrees on pump, button and all nine LED startup assignments.

## Board initialization anchors

All addresses are KM4 runtime addresses in bank 0. ROM call addresses below include the Thumb bit.

| Address | Role |
| --- | --- |
| `0x0e006458` | Board initialization: pinmux, button, lights, pump, scale |
| `0x0e021950` | Read/modify/write `0x48000230`, clearing bit 0 |
| `0x0e0061f4` | Pump initialization; low latch before output enable |
| `0x0e006498` / `0x0e0064dc` | Pump off / on helpers |
| `0x0e00624c` | Button input initialization |
| `0x0e006298` | Nine LED-group output initializations |
| `0x0e006410` | Scale GPIO initialization |
| `0x0e0101e4` | Six-indicator cycling test |
| `0x1010c871` / `0x1010c84d` | ROM GPIO output write / input read pointers |

The pinmux operation matches the existing SDK's SWD release: bit 0 holds PA27/PB3 for debugging. Reuse the SDK's readback-capable `swd_off` operation and verify the generated register address during the fountain build. This is a source/instruction match, not a new electrical measurement.

## Photographed connections

The white ribbon joins the main board's WEIGH connector to the weight/LED daughterboard. The latter carries U1, a package with eight visible leads per side, and the load-cell wire harness. U1's top marking is not readable. Package shape and the recovered protocol do not establish that it is an HX711.

RADAR-labelled footprints have no fitted connectors or cables in the photographed assembly. Stock's `radarState` JSON field therefore does not establish an installed radar sensor. Their pin routing remains unknown. A populated variant needs its own analysis.

The UART header is labelled GND/TX/RX/3V3. A BAT-labelled footprint is also unpopulated. Photograph evidence does not establish supply tolerance, sensor clock limits, battery support, output-current limits or motor-drive electronics.

## Conflicts with the feeder board

A PLAF103 build cannot be made suitable by renaming the device:

| Pin | PLAF103 use | PLWF106 use |
| --- | --- | --- |
| PA15 | Infrared emitter output | Button input |
| PA19 | Adapter-presence input | Water-status output |
| PB3 | Infrared emitter output | Water-status output |
| PB5/PB6 | Panel buttons | Scale clock/data |
| PA25 | Buzzer | Water-status output |
| PA30 | Outside the feeder motor stop pair | Pump enable |

The feeder's panic and RAM flash-stop paths stop PA13/PA14 and clear emitter outputs. They do not clear PA30. A fountain port must select model-specific masks in startup, panic, cross-core flash holds and trial confirmation, then rerun the relevant audits.

## Evidence

- **Recovered, 2026-11-03:** Ghidra initialization/call-site review of stock 0.0.17 and corresponding bank-1 initialization. SDK numbering and SWD behavior were compared with the existing AmebaD source interface.
- **Observed, 2026-11-03:** owner main-board and daughterboard photographs establish connector population and visible package/board markings. No continuity, polarity or timing measurements were made.
