# Integration suite

`wellfed-bench itest` runs named real-device scenarios against the feeder on the [relay bench](bench.md); it is a bench command, not a test target. Builds go in over the air, and the UART stays attached, receive-only, throughout: the scenarios assert on it, it is the evidence, and it is the way back when the feeder stops answering. This page is for contributors running the suite, including the long unattended [sustained run](#sustained-run).

The suite is the `wellfed-itest` crate, linked into the [bench server](bench-server.md), which runs it as a job in its own process: the run, its cancel and its exit status are the job's, and the client exits with that status. `--list` answers in the client, with no server.

```sh
cargo build --release --bins --examples
target/release/wellfed-bench itest --list
target/release/wellfed-bench itest itest-YYYYMMDD-01 --build --tier smoke    # a few minutes
target/release/wellfed-bench itest itest-YYYYMMDD-02 --build --power         # standard (default tier)
target/release/wellfed-bench itest itest-YYYYMMDD-03 --build --power --destructive
target/release/wellfed-bench itest itest-YYYYMMDD-04 --review private/verification/<review> --only requests,request-auth
target/release/wellfed-bench itest itest-YYYYMMDD-05 --review private/verification/<review> --only feed-writes --feeds
target/release/wellfed-bench itest itest-YYYYMMDD-06 --build --power --destructive --feeds --only sustained   # the 8-hour run
```

**Tiers.** `--tier smoke` runs `ota-update`, `requests` and `request-auth`, minutes once the build under test runs. `standard`, the default, runs the whole functional suite. `soak` adds `sustained`, which is meant to run alone. `--only` picks scenarios whatever their tier.

**Builds.** `--build` first asks `wellfed-build source-id --device plaf103 --image` for the identity a build of these sources would embed (every source input, the firmware, the variant and the update key). If a complete review with that `startup_identity` is already in `private/verification`, the suite signs it instead of building; `--rebuild` builds anyway. `wellfed-bench ota --build` does the same, so unchanged sources rebuild neither the build under test nor the trial firmware's `fail` image.

## Scenarios

| Scenario | Checks | Needs |
| --- | --- | --- |
| `ota-update` | The build under test (`--build`, which builds the `feeder` firmware, or `--review`) installs over the air and confirms, unless the device API already shows it on bank 1. It always runs first, then syncs the feeder's clock; every other scenario needs it | |
| `requests` | Over the device API: status, with `PLAPI1 request … op=status`; a clock sync, after which the feeder's clock is within 5 s of the host's; a lights change against the generation just read saves a newer one (`PLCFG1 saved`), reads back, and is restored | |
| `request-auth` | A session with an unenrolled key is refused (`OpenRefusal::Unknown`, `reason=unknown-key`); a handshake with its last byte flipped is dropped (`reason=noise`); the owner's session still works | |
| `settings-persist` | Lights, sound and volume changed against the base generation; a write from the stale generation refused `Stale`, changing nothing; a cold boot, then read back; the originals restored | `--power` |
| `wifi-fallback` | Credentials stored for an absent SSID (`PLCFG1 net saved stored=1`). After a cold boot, `Status.wifi` shows `Fallback`. A clear (against the Wi-Fi generation) and another cold boot return it to `Build`. It refuses to start while credentials are stored | `--power` |
| `network-bounce` | The hotspot down for 20 s and back. The device API answers again with the same boot count and more Wi-Fi losses (association, lease or gateway) | |
| `feed-writes` | Writes from a second session during a feed, then a cancel ([details](#feed-writes)) | `--feeds` |
| `ota-recovery` | The trial firmware's `fail` image installs and fails its trial, and KM0 gives way to bank 0, which answers the device API. The suite pushes the re-signed baseline to it with one fault each, in its own transfer: a flipped byte → `stage=hash`, a chunk ahead of the wanted ones → `order`, 75 s of silence → `timeout`, none activating, bank 0 answering after each. A clean push then confirms and restores the baseline | `--build --power` |
| `ota-power-cuts` | Installs the feeder's `twin` variant: the same firmware under another build identity, so it still serves the device API, and unconfirmed, since a confirmed build skips the trial. **Mid-download** (48 sectors in), the next boot must be bank 0, and the offer's first progress record must show the download starting again. **10 s into trial boot 1**, before the 30 s confirmation, KM0 must count boot 1 again (backup registers clear on power loss), and the build must still confirm. A cut during activation itself is not attempted, since the marker write and reset take milliseconds. The re-signed baseline then goes back on | `--build --power --destructive` |
| `sustained` | `--sustained-cycles` (default 16) half-hour cycles of feeds every 10 minutes, each opening with one fault from a fixed rotation. Any failed check cuts relay 1 and halts ([below](#sustained-run)) | soak tier, `--build --power --destructive --feeds` |

A scenario whose flag is missing is reported **skipped**, never passed. `--destructive` requires `--power`. The design recovers from both power cuts over the air; if it ever does not, recovery is the UART flash the suite prints.

### feed-writes

The scenario holds two owner sessions: the suite's own, and a second held open from before the feeds, since the feeder admits no handshake while a feed holds its outputs.

1. A 2-portion feed on the first session. Once the motor drives (`PLOWN1 state=4`), the held session sends a lights change against the current generation (it must be refused `Busy`), a lock change and a clock sync, and their answers are logged.
2. No settings-writer save (`PLCFG1 saved`, `net saved`, `keys change`) or confirmation may appear between `PLOWN1 feed id=N` and `PLOWN1 fed id=N`. The feed must end `outcome=1` (Done) with `completed` = `requested`, and the refused change must have changed nothing.
3. After the feed the same change saves, and is restored with the lock.
4. A second feed is cancelled from the held session 2 s into its drive. The cancel answers `Done`, the owner logs its outputs stopped, and the feed ends `outcome=2 reason=2` (Cancelled).
5. The lock change and clock sync must answer `Done`, and the lock read the new value after the feed. When a `PLOWN1 interlock` record follows the feed's start, its `writes_deferred` must have grown.

Throughout: no `PLFW1 start`, no panic, no boot count change. The event ring logs nothing per append, so a ring write during the feed shows as the reset its failed flash lease causes.

Feeds and cancels are sent once, never resent on a fresh session: the feeder remembers a write's outcome only once its answer resolves, and a failed one is uncertain. After any failure the suite waits for a feed it started to end (`fed id=N`, then `state=1`) before restoring anything; if it does not end, nothing is restored and the run stops, whatever `--keep-going` says. The feeds run unattended, so the hopper may be empty.

## How a run behaves

**Bench state.** Before changing anything, the suite refuses if the UART port is missing or an evidence name is in use. It then records whether the hotspot connection named in `private/hotspot.json` is active and brings it up; with `--power`, records relay 1 and switches the supply on; and starts the UART capture. Updates go over the device API, pushed on a thread of their own while the scenario reads the UART. The suite talks to the feeder as the key in `--keys` (default `private/wellfed-owner.json`), finding it over mDNS or, failing that, through the hotspot's neighbour table and `target_mac`.

At the end, or on an error or Ctrl+C, the suite stops its capture and any push, and puts the hotspot and supply back as it found them, except after a `sustained` halt, which leaves the supply off. Without `--power` it never opens the relay board. It never flashes over UART, changes the firewall or routing, or offers a startup-motion build, and asks for feeds only as the [safety rules](bench.md#safety-rules) allow. `sustained` also installs the `feed-jam`, `sensor-fault` and `feed-stall` variants, which move the motor only for a feed request.

**Device state.** Once the baseline confirms, the suite reads the settings, whether Wi-Fi credentials are stored, and the build and bank, and again after every later scenario. A scenario that leaves any of them changed fails, even if its own checks passed. The suite puts back what a request can: settings at once, stored credentials from the next boot.

**Failures.** A failed scenario stops the rest, recorded as skipped, unless `--keep-going`, and exits 1. Scenarios that change device state restore it on failure too, while the feeder answers. A feeder with no `PL…` record for 180 s and no device API answer is silent; every wait but `wifi-fallback`'s wrong-credential wait checks this, so an unpowered feeder stops an install watch within 3 minutes, not at its 15-minute deadline. The suite then stops whatever `--keep-going` says, prints the last confirmed review and the `flash-prepare` command to recover it over UART, and exits 2.

**Cold boots.** The old session's sockets die with the power, so a request on them is lost. After a cold boot the suite waits for the baseline's `PLFW1 start` on bank 1, then accepts only a status read in a session opened after it.

**Evidence.**

- **Captures:** receive-only, in back-to-back 180 s segments `private/captures/NAME-uart-01`, `-02`, … (the flash client's limit for one observe). A thread starts each segment as the last exits, so the port stays held while the suite blocks on a build or a child. Segments are read while they record, so a record arrives within about a second.
- **Reviews and manifests:** `NAME-good`, `NAME-fail`, `NAME-restore`, `NAME-cut-twin` and `NAME-cut-restore`, in `private/verification/`. Each signs the next version, so a run uses up several. A reused review keeps its original name.
- **Record:** `private/verification/NAME-itest.json` holds each scenario's verdict, reason and duration, any silence, and the capture list, marked `"evidence": "physical"`.

## Sustained run

The `sustained` scenario (`src/tools/wellfed-itest/src/sustained.rs`) is the long unattended fault run.

1. Check the owner's unattended motion rules: the hopper is empty, the D cells are out (so relay 1 is the kill switch), nobody is at the bench, and the bench lease is free.
2. Run it, keeping the console:

   ```sh
   target/release/wellfed-bench itest itest-YYYYMMDD-NN --build --power --destructive --feeds --only sustained \
     | tee private/verification/itest-YYYYMMDD-NN-console.log
   # a shakedown of every fault once (4 h): add --sustained-cycles 8
   ```

The build under test is this checkout's `feeder` (`--build`). The run builds the `feed-jam`, `sensor-fault`, `feed-stall` and `twin` variants once and re-signs them later. Each install and restore signs one manifest version: 8 per eight-cycle rotation, about 17 for the full run with the baseline's. The logic analyzer may stay clipped on; it plays no part.

**Length and cadence.** 16 cycles of at least 30 minutes, about 8 hours; a step that outlasts its half hour (the update cuts can) stretches its cycle, and each cycle's note gives its real length. Each cycle runs its fault step, then feeds the baseline at once and every 10 minutes while the cycle lasts (at least one feed a cycle), with a device API status every 30 s between. Baseline feeds ask for 1, 2, 3, 1, … portions; **no feed in the run asks for more than 3**. By the simulation's timings that is about 70 feeds (36 in eight cycles), fewer on hardware, where an install takes about 4 minutes rather than 2.5. At about 9 s a portion, that is under 25 minutes of drive in 8 hours.

**The rotation.** Cycle n takes step n mod 8, so the full run takes each step twice:

| Cycle | Step | What it injects | How it must end |
| --- | --- | --- | --- |
| 1, 9 | `feed-writes` | The [scenario](#feed-writes): settings, lock and clock writes from a held session during a 2-portion feed, then a cancel 2 s into the next | Its own checks; its feeds join the run's journal |
| 2, 10 | Reboot | A `Reboot` request at rest | One restart into the baseline; reset cause `watchdog` (the firmware restarts by letting its watchdog lapse); the clock still valid (carried across a warm reset); no drive after the boot |
| 3, 11 | Cut while reserved | A 3-portion feed, relay 1 cut as soon as the `Feeding` answer arrives, which the feeder sends only once the owner has reserved the feed in its journal, before the drive starts | The next boot logs `fed id=N outcome=5` (uncertain) once; reset cause `power_on`; no drive after the boot; the run syncs the clock again |
| 4, 12 | `feed-jam` | Installs the variant (a trial, then confirmation), feeds 1, 2 and 3 portions, then puts the baseline back | 1: Done with 1 portion (one reversal); 2: Failed, `Jammed`; 3: Failed, `MarkStuck`; the owner Idle after each |
| 5, 13 | Cut mid-drive | A 3-portion feed, relay 1 cut 4 s into its drive | As the reserved cut |
| 6, 14 | `sensor-fault` | Installs the variant, feeds 1, 2 and 3 portions, asks for one more, reboots, puts the baseline back | 1: Failed, `FeedbackTimeout`; 2: Failed, `OutletBlocked`, no drive; 3: Failed, `Sensor`, the owner then in `Fault`; the extra feed refused `Fault`; the reboot clears the hold |
| 7, 15 | Outage mid-feed | The hotspot down for 60 s from 4 s into a 3-portion drive | The feed ends Done with 3 portions on the UART; no restart; the link losses grew; the device API answers again within 180 s |
| 8, 16 | `feed-stall`, then `ota-power-cuts` | Installs the variant: a 1-portion feed (600 ms loop stall) and a 2-portion feed (8 s stall); then the two update power cuts, mid-download and early in the twin's trial; the baseline back | 1: Failed, `Output`, then one restart; 2: one watchdog restart and `outcome=5` at the next boot; then `ota-power-cuts`' own checks |

**Pass criteria.** The run passes only if all 16 cycles pass every check, each reading evidence an existing tool or record produces:

| Criterion | Evidence | Check |
| --- | --- | --- |
| No feed sent twice, none run twice | The suite's own requests (`Ask::Feed` is never resent) and the `PLOWN1 feed id=` and `fed id=` records | Each feed id the run was given has exactly one owner record and one result, before or after a restart; no result names an id the run never asked for; ids only grow |
| Every feed ends as its step says | `PLOWN1 fed id=N … outcome= reason= completed=` | The outcomes in the table above; baseline feeds Done with every portion |
| Uncertain feeds are resolved, not retried | `fed id=N outcome=5` after the restart, and no `state=4` | A cut or a stall's reset leaves exactly one uncertain result and no drive in the boot that follows |
| Counters agree with the journal | `Stats { Counters }` | This boot's `feeds`, `feeds_failed` and `portions` equal the sum of this boot's `fed` records. Lifetime counts never go back within a boot, and across a reset their stored base (lifetime less the boot's own) never does: a restart whose save was refused once the early-save budget is spent loses only what the last boot counted |
| No unexpected restart, panic or fault | `PLFW1 start`, `PLPNC1`, `PLTRL1 storm=` and `give_way=1` on the UART; the status `boot_count` | None at rest or during a feed; each planned restart shows exactly one start record, and a warm one moves the boot count by one (it lives in a backup register a power cut clears); `Stats { Counters }.reset` names the planned cause; no `PLPNC1` anywhere |
| No motion at rest | `PLOWN1 state=4` | None outside a feed the run asked for |
| Memory | `Stats { Memory }`, after each baseline feed | Least free heap at least 64 KiB (provisional: half the 141,792 B measured for the product image); free heap within 8 KiB of the first reading in the same boot of the same build |
| Clock | `GetTime`, after each feed | When valid: error bound at most 120 s, and the host's time within that bound plus 2 s; valid after a reboot; an invalid clock (after a cut, or an expiry) is synced and counted |
| Updates install and confirm during the run | The install watch (`PLOTA1`, `PLTRL1`, `PLCNF1`) and the status build | Each variant and each restore of the baseline ends `Passed`; `ota-power-cuts` passes |
| The feeder stays reachable and as it was | Status every 30 s; the device state check | Every status answered, no Wi-Fi loss at rest, and after every step the baseline on bank 1 with the run's settings and Wi-Fi source |

Each cycle ends with a note of the figures so far (feeds, polls and the slowest answer, least free heap, the largest clock skew and bound, clock syncs), which with the UART captures and `NAME-itest.json` are the run's evidence.

**Stop conditions.** Any failed check, a feed result that does not arrive within 100 s, a silent feeder (180 s), an install that does not confirm, or the operator's Ctrl+C halts the run: the suite **cuts relay 1 and leaves it off**, runs no further cycle or scenario, puts the hotspot back as it found it, and records why. A host panic does the same: until the run ends cleanly, tearing the rig down cuts relay 1.

> [!NOTE]
> Nothing is restored on the feeder after a halt, so a halt during a `feed-jam`, `sensor-fault` or `feed-stall` step may leave that variant on bank 1 (the halt note says so). **A person looks before power returns.** Install the baseline before any other run. If the feeder then does not boot, the suite's UART recovery applies.

**Kept out of the unattended run (the owner's rules):**

- Real food: portion weights, food through the outlet beam, and the weighing session.
- Real jams with food in the rotor, taped or blocked sensors, a full hopper.
- Batteries: the D cells, adapter changes, brownout and the low-voltage floor, so the run has no low-power step.
- Button presses (Dispense during a feed) and LED observation.
- The `stop-fail` variant's 8 s held stop, which leaves the motor driving until the watchdog, and `owner-stall`. Both have run attended.
- DHCP-server loss without an access-point loss, and lease expiry, which need a host network change the authorization does not cover.

## Evidence

- **Observed, October 22, 2026:** the standard tier passed in 13.6 minutes (`itest-20261022-18`), each download taking 18–19 s. `ota-power-cuts` passed (`itest-20261022-10`): a cut at 48 of 171 sectors booted bank 0 and the download began again, and a cut 10 s into trial boot 1 was counted as boot 1 again before the build confirmed.
- **Observed, October 27, 2026:** the sustained run's first hardware attempts. Cycles 1–5 passed in one run. Halts in other runs came from two checks that were wrong (since fixed) and an intermittent bank-0 download fault, which stopped at a 50 MHz flash clock ([flash layout](flash-layout.md#flash-clock)). A full 16-cycle run has not yet passed.
- **Modeled:** the scenarios, including `sustained`, against a scripted feeder that emits the firmware's record formats.
