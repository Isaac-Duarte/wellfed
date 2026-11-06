# PLWF106 board signals

A reference for implementing the fountain's board layer. Signal assignments are recovered from stock initialization and callers, not measured for continuity or voltage. The [shared pin table](../../chips/rtl8721dm/pins.md) compares models.

## GPIO assignments

SDK numbers are `(port << 5) | pin` ([chip](../../chips/rtl8721dm/README.md#pins)). A startup level is the software latch value, not a measured voltage.

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

The six identified indicators select HIGH when enabled, and stock's cycling LED test covers only those six. Their physical colors are unknown. Don't infer RGB or decorative-light functions for the last three outputs. Bank 1 independently agrees on the pump, the button and all nine LED startup assignments.

## Board initialization anchors

KM4 runtime addresses in bank 0. ROM call addresses include the Thumb bit.

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

The pinmux operation matches the SDK's SWD release: bit 0 holds PA27/PB3 for debugging. Reuse the SDK's readback-capable `swd_off`, and verify the generated register address during the fountain build. This is a source and instruction match, not an electrical measurement.

## Photographed connections

The white ribbon joins the main board's WEIGH connector to the weight and LED daughterboard, which carries U1 (eight visible leads per side) and the load-cell harness. U1's marking is not readable, and neither its package nor the recovered protocol establishes that it is an HX711. Three light pipes sit below the daughterboard; which GPIO group lights each is unmeasured.

The RADAR footprints have no fitted connectors or cables, so stock's `radarState` field doesn't establish a radar sensor. Their pin routing is unknown, and a populated variant needs its own analysis. A BAT footprint is also unpopulated. The UART header is labelled GND/TX/RX/3V3. The photographs don't establish supply tolerance, sensor clock limits, battery support, output-current limits or motor-drive electronics.

## Conflicts with the feeder board

Renaming the device can't make a PLAF103 build suitable:

| Pin | PLAF103 use | PLWF106 use |
| --- | --- | --- |
| PA15 | Infrared emitter output | Button input |
| PA19 | Adapter-presence input | Water-status output |
| PB3 | Infrared emitter output | Water-status output |
| PB5/PB6 | Panel buttons | Scale clock/data |
| PA25 | Buzzer | Water-status output |
| PA30 | Outside the feeder motor stop pair | Pump enable |

The feeder's panic and RAM flash-stop paths stop PA13/PA14 and clear the emitter outputs, not PA30. A fountain build takes its masks from `wellfed-board-plwf106`'s safe state, which clears PA30 ([device layer](../../firmware/device-layer.md)). Changing them for startup, panic, cross-core flash holds or a trial confirmation means rerunning the link, RAM-park and stack audits.

## Evidence

- **Recovered, 2026-11-03:** Ghidra review of stock 0.0.17's initialization and call sites, and of bank 1's initialization. SDK numbering and SWD behavior compared with the AmebaD source interface.
- **Observed, 2026-11-03:** owner photographs of the main board and daughterboard show connector population and the visible package and board markings. No continuity, polarity or timing was measured.
