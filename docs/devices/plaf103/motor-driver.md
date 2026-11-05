# Motor driver and electrical checks

The PLAF103 turns its food wheel with a small DC motor behind an H-bridge driver. This page covers the driver, what we measured at it, and the checks that show the motor never moves when it shouldn't. It is a reference for firmware and hardware contributors.

In short: in the ROM loader the driver sits in standby, and at Wellfed's STOP it brakes. Wellfed's startup goes from power-on straight to brake with no drive pulse. The emitter currents and the pin-to-input trace are open.

## The driver

The driver is a **TC118AH** (Fine Made Microelectronics, SOP-8, single-channel H-bridge), designator **U7**, beside CON3. Its datasheet is S&CIC2102 v1.1.

| Pin | Name | Pin | Name |
| --- | --- | --- | --- |
| 1 | NC | 8 | OUTA |
| 2 | INA | 7 | PGND |
| 3 | INB | 6 | AGND |
| 4 | VDD | 5 | OUTB |

| INA | INB | OUTA | OUTB | Mode |
| --- | --- | --- | --- | --- |
| L | L | Hi-Z | Hi-Z | Standby |
| H | L | H | L | Forward |
| L | H | L | H | Reverse |
| H | H | L | L | Brake |

From the datasheet:

- **Pull-downs:** INA and INB have 150 kΩ internal pull-downs (120–200 kΩ), so undriven inputs mean standby.
- **Logic levels:** VIH is at least 2.0 V and VIL at most 0.8 V, so 3.3 V GPIO drives it directly.
- **Supply:** VDD is 2.0–7.2 V.
- **STOP is brake:** stock's STOP (PA13 = PA14 = HIGH), which Wellfed copies, drives both outputs low.

Which of PA13 and PA14 drives INA and which INB is inferred from stock's complementary drive table, not traced. The two `102` (1 kΩ) resistors beside pins 2 and 3 look like series resistors from the SoC, also untraced.

## The harness

- **Blue and white:** the motor leads, to a can-type DC motor with a disc suppression capacitor across its terminals. They run through ferrite beads FB1 and FB8 to OUTA and OUTB.
- **Yellow pair:** a pulled-up logic line and its ground. In the ROM loader they read 0 V and 3.3 V to ground, and 3.3 V between them. Most likely the PA18 feedback sensor.

## Measured states

Multimeter (Klein MM325), DC volts, COM on USB ground, the feeder powered through the relay bench with no batteries.

| State | INA (2) | INB (3) | VDD (4) | OUTA (8) | OUTB (5) | Motor |
| --- | --- | --- | --- | --- | --- | --- |
| ROM loader, no application code | 0.0 V | 0.0 V | 5 V | 0.13 V | 0.13 V | Still, no hum |
| Wellfed at STOP | 3.3 V | 3.3 V | 5 V | 0 V | 0 V | Still, no hum at power-on |

- **In the ROM loader the driver is in standby:** its pull-downs hold both inputs low and both outputs float. Every bench flash and dump runs in this state, so entering the loader cannot drive the motor.
- **At Wellfed's STOP the driver brakes:** both inputs HIGH, both outputs at 0 V. Equal inputs can't show which GPIO drives which input.

## Startup: straight to brake

Stopped startup writes both output latches HIGH while the pins are still inputs, then `gpio_enable_outputs` sets the pad and interrupt registers and makes both pins outputs in one register write. So the driver goes from whatever its undriven inputs read straight to brake. When the board opens later, only the latch write changes, so there is no coast either.

The old order enabled one pin per ROM `gpio_init`, leaving one input undriven for that call: if it read the opposite level, the motor drove for about 105 µs, and the board's opening coasted it for about 224 µs. The ROM's `GPIO_Init` for an output does, per pin, `Pinmux_Config` (pad function GPIO, pad shutdown cleared), `GPIO_INTMode` disable, and a `DDR` bit set. It sets no pull for an output, and the pinmap gives PA13 and PA14 none.

Host tests (`src/sdk/wellfed-hal-feeder/tests/stopped.rs`) run stopped startup and the board opening with the undriven inputs LOW and HIGH, sampling the driver inputs after every register write. The old order fails with HIGH (`motor driven: 0x4000`) and with the coast; the new one passes both.

### What a logic analyzer can and can't tell

- **The analyzer biases undriven inputs HIGH.** With it attached, a meter read INA and INB at 2.3 V in the loader, against 0 V without. So the old boot pulse needed the probe's bias and likely never happened unprobed. The coast at board open was firmware-driven and didn't. The fix holds either way.
- **The outputs alone can't prove drive during a supply ramp.** The winding joins the two leads. With the driver unpowered, the analyzer's two channels can resolve one common level differently near their threshold, so a differing read counts as drive only when the levels are firm. The inputs, always driven or pulled down, settle it.
- **In standby the outputs float** at the analyzer's floating level (HIGH on the owner's clone), so an outputs-only capture can't confirm the brake or the power-on edge.

## Emitter test points

CON1 (silkscreen 余粮红外, "remaining-food infrared") carries the food-level sensor, with TP20 and TP21 beside it near Q5. CON2 carries the outlet sensor, with TP22 beside it. The `fw13-optics-v2` image holds each emitter state for 5 s, long enough for a meter.

| Test point | Emitters off | PA15 on | PB3 on | Both on |
| --- | --- | --- | --- | --- |
| TP20 | 3.3 V | 1.2 V | 3.3 V | 1.2 V |
| TP21 | ~2.68 V | ~2.68 V | ~2.68 V | ~2.68 V |
| TP22 | 0.66 V | 0.11 V | 0.66 V | 0.11 V |

- **TP20** follows only the food emitter (PA15), on its drive path: 3.3 V off, about 1.2 V on, a typical infrared LED forward voltage. So **the food emitter's OFF state is confirmed**: 3.3 V, no LED current.
- **TP21** reads 2.67–2.68 V whatever the emitters do: most likely a supply or bias node.
- **TP22** follows PA15, not PB3, despite sitting beside CON2. Its levels (0.66 V dark, a base-emitter clamp; 0.11 V lit) fit the food receiver's phototransistor driving an inverting stage, which would make PA17 HIGH when lit, as measured.

No test point follows PB3. The outlet sensor's CON2 wires do:

| CON2 wire | At rest | Emitters on, feeding | Role (inferred) |
| --- | --- | --- | --- |
| Green | 0 V | 0 V | Ground |
| Blue | 3.3 V | 3.3 V | Supply |
| White | 2.6 V | 0.03 V | Emitter's switched side |
| Red | 0.6 V | 0.08 V | Receiver output, pulled low when lit |

The driver pulls white to ground to light the LED. At rest it sits at 2.6 V with no current path, so **the outlet emitter's OFF state is confirmed** too.

## Open checks

1. **Emitter current** for both emitters: the voltage across each series resistor, or a supply-current meter.
2. **Polarity:** trace PA13 and PA14 to INA and INB. The input captures suggest PA13 drives INB, opposite to the stock-derived mapping. Check the clip labels against the pin map before counting that.
3. **A reset mid-drive:** when a watchdog reset ended a drive, the leads read differently three more times, for 38–110 µs, within 0.2 ms of the drive's end. Two had the drive's polarity and one the opposite, and no drive followed. Back-EMF from the coasting motor explains the first two, not the third. Capture this with the inputs probed.
4. **The power-on skew:** inside the supply ramp, before any code runs, single threshold skews of up to 4.5 µs remain between the inputs, unclassified. If they also appear when the loader runs instead of our code, they belong to the probe and the ramp. A scope on the inputs, or a meter on the undriven inputs with the analyzer attached, would settle it.

## Capture startup transients

Use this to repeat the startup check or to record resets.

1. Install `sigrok-cli` and the `fx2lafw` firmware (Arch: `sigrok-firmware-fx2lafw`). Without the firmware the clone enumerates but cannot open.
2. Wire the analyzer: D0 to INA (pin 2), D1 to INB (pin 3), D2 to OUTA (pin 8), D3 to OUTB (pin 5), and analyzer GND to board GND (pin 6 or 7).

   > [!WARNING]
   > Check the analyzer's input rating before you connect the 5 V outputs. Probe pins 2–4 at the tip of each foot. Pin 3 (INB) is next to pin 4 (VDD), and bridging them drives INB high, which with INA low is reverse drive.

3. Run the captures. Use `--off-seconds 10` for loader sessions, so the board has discharged before each capture starts.

   ```sh
   wellfed-bench transients NAME --cycles 50
   wellfed-bench transients NAME --mode loader --off-seconds 10
   ```

4. Audit each capture:

   ```sh
   wellfed-audit transients --device plaf103 --capture private/captures/NAME --output NEW-REPORT
   ```

A cycle passes when it starts unpowered, no sample has INA ≠ INB or OUTA ≠ OUTB, there is no LOW, LOW coast when the board opens, and the brake is reached and held after boot (or standby held throughout a loader entry).

**If you can reach only the outputs:** the driver's pins are 1.27 mm apart, too close for ordinary hooks. `--probes outputs` records only D2 and D3 at the motor, with analyzer GND on the bench ground rail. Clip the motor terminals, or the suppression capacitor's legs where glue covers the tabs: the curved leg on the blue-lead side to D2, the straight leg to D3. Which is OUTA is untraced, and the check doesn't depend on it. The report records `probed: outputs` and what it could not see, and the polarity check stays open.

## Evidence

- **Observed, October 21, 2026:** the TC118AH, from its package marking. The datasheet is in the ignored `private/references/`.
- **Observed:** the harness colors and the can motor, from owner photographs. Earlier notes calling the yellow pair the motor leads were wrong. PA18 read HIGH throughout the optical-input bring-up.
- **Observed:** the loader and STOP meter readings, STOP with the optics bring-up build running: the first electrical confirmation that stopped startup reaches the driver and STOP is brake.
- **Observed, October 24, 2026 (outputs only, 2 MHz, captures `fw30-outputs-*`):** before the first fix, four boots each showed about 100 ms of standby, then 104.5–105 µs with D3 HIGH and D2 LOW, then brake. After a first fix (build v154, `e5b5c1dd…`), 51 boots went from 98–111 ms of standby straight to brake, and fifty loader entries held standby. Within about 0.3 ms of relay closure the outputs differed for 0.5–11.5 µs in 39 of 50 boots, and for up to 110 µs in the loader, where no code runs. Before power-on, with the board still discharging after a 2 s off, the leads read differently for up to 39 ms.
- **Observed, October 26, 2026 (inputs probed, 2 MHz, build v232, `fw30-inputs-20261026-02` and `fw30-inputs-loader-20261026-02`):** every complete boot still showed the 105 µs forward input drive 97–105 ms after the inputs rose, a 1.5–2 µs LOW, LOW interval, then brake, and a 220–228 µs coast at 925–937 ms as the board opened. The inputs read HIGH from power-on even in the loader, and INB fell first at board open. **Static analysis** against the capture: the pulse was the first `gpio_init`, and the first fix had only moved the assumption onto the undriven level.
- **Observed, October 26, 2026 (evening, build v236 with the current fix):** a meter read INA and INB at 2.3 V in the loader with the analyzer attached, 0 V without. Two boots and four loader cycles showed no drive and no coast before the analyzer dropped off USB.
- **Observed, October 27, 2026 (build v244, `807f7283`, sessions `fw30-latch50-20261027-01`…`07`, 10 s off):** the current fix passed 50 cold boots. After the supply ramp's first millisecond, INA = INB and OUTA = OUTB at every sample of every boot, with no drive at board open and every boot ending in brake.
- **Observed, October 26, 2026:** the reset mid-drive transitions, in `fw16-stopfail-20261026-03` ([motor pulse](../../firmware/motor-pulse.md)).
- **Observed:** the TP20, TP21 and TP22 readings. TP21 read 2.67–2.68 V on 13 of 16 cues; three ~0 V readings tracked no state and are taken as probe slips. TP22 matched on all 19 cues over three cycles. The roles are inferred from the voltages, not traced.
- **Observed, October 26, 2026:** the CON2 wire levels, probed by the owner at the wire entries with feeder v232 at rest and during 1-portion feeds 46 and 47. The roles are inferred from the levels.
