# Portions, phase and calibration

The feeder counts portions, not grams. A portion is one compartment of the rotor, credited when the rotor's mark sensor sees it pass. What a portion weighs comes from your kitchen scale, entered once with `wellfed calibrate`. This page explains what a portion is, how the feeder copes when the rotor stopped between marks, and how the weight is kept. Measuring it is the [weighing session](weighing-session.md).

Phase handling is observed on our unit, and one portion of the owner's dry food weighs 7.11 g on it.

## What a portion is

The rotor has three compartments over one drop hole. Each fills from the hopper and empties by gravity over the hole, so a portion is a third of a turn.

The mark sensor (PA18) rests HIGH and reads LOW for about 1.2–1.4 s at each compartment's mark. The owner credits a portion on the mark's rise, about 9.1 s of drive apart empty and 9.8 s with food, and stops the motor about 20 ms later, so a completed feed leaves the rotor just past a mark.

The feeder has no scale. It knows how many marks passed, never what fell or what the pet ate ([product architecture](product-architecture.md)). Delivered food is an estimate: credited portions times your measured weight of one.

## Phase

The phase is where the rotor rests between feeds (`RotorPhase`, `wellfed_core_feeder::feed::storage`):

- **At a mark:** the last feed ended `Done`, on its last portion's rise.
- **Unknown:** the last feed that may have moved the motor ended any other way (cancelled, failed, or uncertain after a reset). The rotor may be anywhere between two marks, or inside one (PA18 LOW).

A feed that never moved the motor leaves the phase as it was. The phase is never stored: the owner reads it at boot from the feed journal's last result (`OwnerStorage::last_result`). A journal with no result yet, from a new install or a unit that ran stock, reads as unknown. The journal's layout is unchanged, so an older build in bank 0 reads the same journal.

### A feed from an unknown phase

Stock credits a single bowl's portion on the first PA18 rise at least 5 s after the drive starts or after the last portion. It never homes the rotor or blocks a feed after an interruption. Wellfed keeps that crediting and adds a homing mark:

- **A rise inside the 5 s inhibit window is the homing mark.** The rotor reached its first mark having turned less than about 55 % of a compartment (5 of 9.1 s). It is never credited, and the portions count from it, as if the drive had started at a mark. Stock ignores the same rise and credits the next, so without a jam the motor does what stock's does. Wellfed also records the homing: `homed_ms`, the `PLOWN1 marks` line and the portions statistics.
- **A first rise past the window is credited**, as in stock: more than about 55 % of a compartment moved before it.
- **A jam before the homing mark** backs off and drives on ([jams](jams-and-sensor-faults.md)). While homing, the window grows by what the reversals cost, as the deadline does, so it still bounds how far the rotor turned. Here Wellfed parts from stock, homing on a mark a fixed 5 s window would credit. A boundary recrossed after backing off is skipped first, so the homing mark is always a new one (`a_jam_before_the_homing_mark_still_homes_on_it`).
- **The deadline** of a request that may home is 5 s longer, within the 48-portion cap. Otherwise a 1-portion feed from about 4–9 s past a mark would time out: the homing rise at up to 5 s, then 9.8 s of compartment with food, past the 12 s a portion allows (`a_feed_after_a_cancel_homes_the_rotor_then_counts_from_the_mark`).

So a feed from an unknown phase drops its portions plus whatever was left of the interrupted compartment: at most about 55 % of a portion more than it credits when it homes, and about 45 % less when its first credited compartment was partly emptied before. Whether a part-turned compartment holds or drops its food depends on where the hole sits in its turn, which the [weighing session](weighing-session.md) can bound.

**Why the phase does not block feeds.** Blocking every feed until a homing move would starve a pet after any cancel or power cut, schedules included, and the homing move would itself drop food. Homing inside the next real feed costs at most one part-compartment, once, and needs no readiness reason. Its window never holds a whole compartment of forward drive: 5 s plus three reversals' 1 s returns stays under the shortest 9.1 s between marks, asserted at compile time (`MARK_PERIOD_MIN_MS` in `feeding.rs`). The phase is reported instead: `GetCalibration`'s `phase`, and the portions statistics' `homed`.

A double bowl alternates its crediting direction every second mark, so one mark cannot home it. The owner refuses a double-bowl feed with an unknown phase (`SubmitResult::RejectPhase`), and the controller refuses a homing request in double-bowl mode.

## Bowl mode

The PLAF103 has one drop hole and feeds clockwise (`Drive0`, PA13 HIGH). Stock also has a double-bowl branch that alternates direction each portion, pauses between, and needs two marks per portion. Nothing on this unit has run it, and with one hole it has nowhere to send a second bowl's food.

So the bowl mode belongs to the build, not a setting. `feeding::BOWL` is `Single`, and the feed journal pins it: a journal written in another mode reads as corrupt. The device API reports it (`Calibration::bowl`) and refuses a `PutCalibration` naming another mode with `Refusal::Unverified`, so the mode exists on the wire with no way to turn it on. A double-bowl build would need its own journal binding and a bench run of the pauses and crediting. The settings record's bowl byte holds `Single`.

## Calibration

| Parameter | Unit | Limits | Default | Stored |
| --- | --- | --- | --- | --- |
| `portion_mg`, the mean weight of one portion | Milligrams | 1,000–50,000 (`PORTION_MG_MIN`, `PORTION_MG_MAX`), or none | None | Storage sectors 34–35 |

The limits are sanity bounds: a compartment of dry food weighs a few grams to a few tens of grams. A value outside them is refused `Invalid` at the device API and never written, and a record holding one is not read. The weight never changes how far the motor turns.

Only one weight is kept. The food beam (PA17) says only whether food is present, so a level-dependent weight would need a level the feeder does not measure. The weighing session measures both ends; if they differ by more than about 10 %, a later change can key a second weight on `feeds_since_refill`.

### Storage

A dual-slot store, `PLFCAL01` (`wellfed_feeder::store::calibration`), holds the generation, `portion_mg` at bytes 20–23 (0 for none), zero padding, a CRC and a commit marker, pinned in `store_layout.rs`. Its sectors, 34 and 35 of the 48-sector storage region (`storage::CALIBRATION`), were never allocated before, so a build that predates the store never touches them. The settings writer saves it on its write turn, never during a feed, inside a marked attempt like the time settings. The [factory reset](pairing.md#factory-reset) keeps it, since the weight belongs to the hardware.

### Wire

- `FeederOp::GetCalibration` returns `FeederOutcome::Calibration(CalibrationView { generation, writable, calibration: Calibration { portion_mg, bowl }, phase })`. `phase` is `None` without a feeding owner (the simulator) or before it opens.
- `FeederOp::PutCalibration { base, calibration }` is for owners and members, not the server's key, answered `Done { generation }` once stored. It is refused `Stale` past `base`, `Busy` while the outputs are claimed or a change waits, `Invalid` outside the limits, and `Unverified` for another bowl mode. The server maps `Unverified` to `Forbidden`, as for update refusals, so a server session cannot write it.
- `StatsSection::Portions` returns `Stats::Portions(Portions { portions, homed, outlet_breaks, portion_mg, delivered_mg })` since this boot: credited portions, feeds that began with a homing mark, outlet-beam breaks, the stored weight, and `portions × portion_mg`.

`wellfed-proto`'s `tests/fixtures/v1.txt` pins the messages.

### Delivered-food estimate

- **Credited portions** (`Fed { completed }`, `Counts::portions`, `Portions::portions`), one per mark, are the estimate's basis.
- **Homing feeds** (`Portions::homed`) may each have dropped up to about half a portion more than they credited.
- **Outlet-beam breaks** (`Portions::outlet_breaks`, and per feed `PLOWN1 marks … outlet_breaks=`) come from the outlet beam (PA16) sampled every 10 ms during a feed, two changes a break. Food falls as a stream of kibble, and a passage shorter than 10 ms falls between samples, so this is a lower bound and an activity check, not a portion count. Credited portions with no breaks at all signal an empty hopper or a jam above the rotor.

Each feed ends with `PLOWN1 marks id=N homed_ms=… outlet_breaks=… phase=at_mark|unknown` after its `fed`, `current`, `edges` and `steps` lines.

## Evidence

- **Rotor and marks.** Observed (October 25): three compartments over one drop hole (owner photos), marks 9.1 s apart empty and 9.8 s with food, the stop about 20 ms after the rise ([motor and feeds](motor-pulse.md#evidence)). Three 1-portion feeds with food saw 0, 4 and 8 outlet changes. Every owner feed turned clockwise.
- **Stock crediting.** Recovered: ISR `0x0e01a55c` credits the first PA18 rise at least 5 s after the drive starts or the last portion. The double-bowl branch is recovered too, never run.
- **Phase across a cancel.** Observed (October 26, v236, hopper empty, `phd-*` captures): feed 54 credited 1 and ended `at_mark`; feed 55, cancelled at 6.28 s, left `unknown`; feed 56 homed at 2,848 ms, completed 1 and ended `at_mark`.
- **Phase across a power cut.** Observed (same session): feed 57, cut by relay 1 mid-drive and never resent, logged an uncertain `Fed` (outcome 5) and phase `unknown` at the next boot. Feed 58 homed at 3,301 ms, completed 1 and ended `at_mark`, and the Portions statistics read `homed 1`.
- **A feed after a mid-drive cut.** Observed (October 27, the sustained bench run): two mark edges for one credited portion, the first at 3.1 s against the usual 7.9 s, likely the rotor finishing the cut portion. The weighing session's phase step should measure it.
- **Weighing.** Observed (October 29, one dry food, hopper refilled between runs): two 5-portion feeds, 5 of 5 each, drives of 48.0 and 48.6 s, dropped 71.1 g, 7.11 g a portion; an earlier 3-portion feed dropped 21.3 g, 7.10 g a portion. The outlet beam broke 11, 6 and 2 times, so breaks do not count portions. `wellfed calibrate --weighed 71.1 --portions 10` stored 7,110 mg, unchanged after a reboot.
- **Phase effect.** Observed (October 29, hopper below the food beam): a 3-portion feed cancelled at 5.0 s credited nothing and left `unknown`; its drop was discarded. The next 1-portion feed homed at 4.6 s, credited at 12.8 s (a 14.1 s drive) and dropped 6.6 g, 93 % of a portion, so the part-turned compartment added nothing.
- **Low hopper.** Observed (October 29): eight 1-portion feeds from a hand-emptied hopper dropped 1.7, 11.4, 6.5, 7.0, 6.0, 3.0, 1.9 and 0 g while the food beam read low ([optical inputs](optical-inputs.md#evidence)).
- **The weight across installs.** Observed (October 29): a give-way and an install over the air kept the stored weight.
- **Host and simulation.** Modeled (October 26): phase from the journal, the homing window and deadline, the compile-time bound (8.0 s of forward drive against the 9.1 s empty mark period), the bowl refusal, the store, the wire messages and the Portions statistics.
