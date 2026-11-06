# Relay bench

The relay bench lets one computer power the feeder, enter its ROM loader, flash it, update it over the air and run long integration tests with nobody at the table. This page is the guide and command reference for `wellfed-bench`, for contributors with the bench wired up; the [bench server](bench-server.md) and [integration suite](integration-suite.md) pages cover the server it runs under and the real-device tests.

`wellfed-bench` ([source](../../../src/tools/wellfed-bench)) adds only power and strap control. Every serial read, inspection, plan and write runs as a separate `wellfed-flash` process, so the [flash client's](../../development/rust-flash.md) pinned checks, plan hashes and no-retry stops apply unchanged.

`wellfed-bench serve --device plaf103` runs in its own terminal and holds the bench. Every other command is its client: it asks the server to run it as a job, prints the output and exits with the job's code. Any bench command is a physical operation on the feeder; `cargo test` and `make check` never reach the bench.

## Safety rules

These hold here and on the two companion pages.

- **Relay 1 is feeder power and the kill switch.** All four relays are normally open, so a relay board that loses USB leaves the feeder off and the strap released ([wiring](#wiring)).
- **One tool drives the bench at a time.** The server holds the [bench lease](bench-server.md#bench-lease) for its life and runs one job at a time; a second is refused, not queued. A killed server leaves a lease the next one refuses to override.
- **Unattended motion follows the owner's rules:** hopper empty, the D cells out so relay 1 is the kill switch, nobody at the bench, the bench lease free, and at most 3 portions a feed. Only `feed-writes` and `sustained`, under `--feeds`, ask the feeder to feed ([sustained run](integration-suite.md#sustained-run)).
- **Motion builds stay manual.** The bench never offers a startup-motion build, refuses the motor pulse under `--build`, and needs `--attended-motion` for any image whose manifest names a motion policy.
- **A halt cuts relay 1 and leaves it off.** A person looks before power returns.
- **The bench never changes routing or the firewall.** Stock reaches a broker only where a setup frame named it, and an install needs TCP 1883 and 8088 from the hotspot admitted by the owner's permanent rule ([the bench host's firewall](../../development/stock-server.md#the-bench-hosts-firewall)).
- **Don't rebuild `wellfed-flash` between prepare and execute.** A plan binds the SHA-256 of the binary that prepared it.
- **Keep heavy host work away from updates.** Loader NAKs came while `make check` ran beside an install.

## Set up

### Wiring

The bench runs on the [Wellfed Bench board](../../../src/hardware/bench/README.md), which puts the UART bridge and the four switches on one USB cable; [bench hardware](../../development/bench-hardware.md) has its connectors, FEEDER UART pinout and feeder wiring.

> [!NOTE]
> The bench board's FEEDER UART header names the feeder's pins, so it is wired label to label: feeder TX to the board's TX. The table below is the older relay bench, where a bare adapter's labels name its own pins, so feeder TX goes to adapter RXD.

The older relay bench added four switches to a bare FT232R adapter's [UART wiring](uart.md) with a DSD TECH SH-UR04A USB relay board (CP2102 serial, 9600 8N1), as the [wiring diagram](../../../src/tools/wellfed-bench/wiring.svg) draws. The bench board's channels do the same jobs under the same numbers.

| Relay | Contacts | Connection | Role |
| --- | --- | --- | --- |
| 1 | COM1, NO1 | In series with USB 5V between two screw-terminal USB-A adapters; GND, D+ and D− pass straight through | Feeder power |
| 2 | COM2, NO2 | Feeder TX side of the 4.7 kΩ strap resistor, and GND | Replaces the loader-entry button |
| 3 | COM3, NO3 | In series with adapter TXD → feeder RX | UART into the feeder |
| 4 | COM4, NO4 | In series with feeder TX → adapter RXD, on the adapter side of the strap | UART out of the feeder |

- **Relays 3 and 4 isolate the UART.** A powered adapter holds TXD and RXD high, which can back-power an unpowered feeder through its UART pins, so every power cut opens relays 3 and 4 after the supply, and every power-on closes them just before it. GND stays connected.
- `capture` connects the UART only to a powered feeder; `--uart-when-off connected` keeps relays 3 and 4 closed through cuts, as before they were wired, for comparison.
- `ota`, and `itest` without `--power`, leave the relay board alone and need a feeder the bench powered (which connected the UART).
- The board answers `AT` with `OK`. `AT+CHn=1`, `AT+CHn=0` and `AT+CHn=?` set and query channel *n*. Its front buttons toggle the same state, which queries report.

### Software

1. Build the binaries together, from the repository root:

   ```sh
   cargo build --locked --release -p wellfed-bench -p wellfed-flash -p wellfed-stock -p wellfed
   ```

   The server runs the `wellfed-flash` beside itself unless `serve --flash-bin` names another, and `restore-stock` the `wellfed` beside it unless `serve --wellfed-bin` does.

2. Write the ignored `private/bench.json` with `/dev/serial/by-id/` paths, since the relay board also enumerates as a USB serial port and can take the UART adapter's old name:

   ```json
   {
     "relay_port": "/dev/serial/by-id/<relay board>",
     "uart_port": "/dev/serial/by-id/<UART adapter>",
     "baseline": "private/<pinned full read>/flash-read.bin",
     "rom_report": "private/<ROM inspection>/report.json"
   }
   ```

3. Check the bench without a server: `target/release/wellfed-bench status`. You should see the lease (free), no server, the four relay states and both ports present.

4. Start the server in a terminal of its own: `target/release/wellfed-bench serve --device plaf103`. You should see the socket, the lease and the four relays. It runs until `q` or Ctrl+C in its [console](bench-server.md#the-console). Run every other command from a second terminal.

Run the bench from the repository root, or pass `--root`. A job resolves the paths you give it (`--target`, say) from the server's directory, so start the server at the root and give paths relative to it. The bench never links the flash client, so rebuilding the bench alone does not affect a plan.

### The crates

| Crate | What it does |
| --- | --- |
| `wellfed-relay` | The relay board driver, behind a `Board` trait the Wellfed Bench board can implement |
| `wellfed-rig` | The operator sequences, and the steps the suite builds on |
| `wellfed-itest` | The integration suite, on the rig |
| `wellfed-bench` | One binary over the others: the server (the lease, the board, the socket, the feeder session, the UART capture, the sequences and the suite as its jobs, and the bank ledger), `serve` and its clients |

Dependencies run down the table. Their shared shapes (the bench config, progress records, the server's live state, run records, the suite's catalog) are typed in `wellfed-formats`; the rig signs offers through `wellfed-signing`; builds run as `wellfed-build` processes, and flash reads and writes as `wellfed-flash` ones.

## Commands

| Command | Effect |
| --- | --- |
| `serve` | The [bench server](bench-server.md), in the foreground until `q`, Ctrl+C, SIGTERM or a hangup, drawn as a full-screen console |
| `status` | The [bench lease](bench-server.md#bench-lease), then the server's job, children, UART, feeder session, bank ledger and relays; with no server, the relay board itself. A server silent for 5 s is reported so, after the lease. `--break-lease` first frees a lease a killed server left |
| `attach ID` | Follow job `ID` again, from its first kept line |
| `console` | A read-only [full-screen console](bench-server.md#watching-from-another-terminal) of the running server, here or over `ssh -t`; `q` leaves it, and the server runs on |
| `answer JOB SEQ done\|yes\|no` | Answer question `SEQ` of job `JOB` ([restore as a job](#restore-as-a-job)), once |
| `banks` | The [bank ledger](bench-server.md#the-bank-ledger). `--seed DUMP_SESSION` starts it, from that dump's inspection or, when it names no build, from `--bank N --build HASH` |
| `note` | The [operator's notes](bench-server.md#the-operators-notes): the step under way, the next steps, timers and evidence by card |
| `mcp` | The bench's tools for an agent, over the [Model Context Protocol](bench-server.md#mcp) on stdio, as a client of the server |
| `on`, `off` | Power the feeder with the strap released, or release the strap and cut power |
| `capture NAME` | Receive-only recording of the running feeder; power untouched |
| `press CHORD` | Hold panel buttons on a running `virtual-panel` build ([below](#virtual-panel)): `dispense`, or several joined by `+` (`lock+dispense`), for `--hold-ms` (150 by default, at most 15000); power untouched |
| `boot NAME` | Power off, start a receive-only capture, confirm it holds the port, then power on |
| `loader NAME` | The same, with the strap held for 2 s across power-on. Requires the download banner and a NAK run, or powers off |
| `rom-inspect NAME --candidate-rom ROM` | `boot`, then `wellfed-flash read inspect-rom` on the stock console it reached, into `private/captures/NAME/rom`. The report names the model whose console answered, which every later read of that model needs. `--candidate-rom` repeats; `--seconds` (20) is the boot capture's length. Stops, powered and with nothing retried, on a boot that reached the loader or a failed inspection |
| `transients NAME` | Repeated logic-analyzer captures (`sigrok-cli`, fx2lafw) of relay power-ons or, with `--mode loader`, loader entries; each cycle arms the analyzer before power-on and ends powered off. No UART, no flash ([motor driver](motor-driver.md#capture-startup-transients)) |
| `dump NAME` | `loader`, then `read small` and `read full` in one helper session |
| `flash-prepare NAME` | `loader`, `read small`, `update inspect` and `update prepare`. Prints the plan summary and the exact `flash-execute` line, then stops |
| `flash-execute NAME` | Checks relay 1 stayed on and the strap is released, runs `update execute` with the reviewed hash, then `boot`; with `--bank 0`, then [finishes the bank-0 write](#finish-a-bank-0-write) |
| `give-way-prepare NAME`, `give-way-execute NAME` | The same pair for `give-way`. `--fallback` names bank 0's frozen review |
| `stock-prepare NAME`, `stock-execute NAME` | UART return to stock ([below](#to-stock-over-uart)) |
| `ota NAME` | One over-the-air update of the running feeder ([below](#over-the-air-updates)). Never touches power or opens the relay board |
| `restore-stock` | `wellfed restore-stock` as a job ([below](#restore-as-a-job)) |
| `itest NAME` | The [integration suite](integration-suite.md): its own hotspot and receive-only capture, installs over the air, and the bench restored at the end. Opens the relay board only with `--power`, for cold boots |

`wellfed-bench <command> --help` lists the flags. `flash-prepare` and `flash-execute` take the updater's `--bank`, `--source`, `--original-read`, `--target`, `--interrupted` and `--restore-marker`, and `--attended-motion` for a target whose manifest names a motion policy. With `--bank 0` both need `--bank1-source`, the frozen review of what bank 1 holds.

Every command from `on` down is a job, refused with no server running (the refusal names `wellfed-bench serve --device plaf103`).

**Names and evidence.** Session evidence goes to new directories named after `NAME` (a single plain name) under `private/captures/`, `private/flash-backups/` and `private/verification/`. An existing path is refused before any relay changes.

**Stopping a job.** Ctrl+C in the client asks the server to cancel the job, and the client keeps printing its cleanup. The server sends SIGINT to the `wellfed-flash` child, which stops terminally and records it, and SIGTERM to the job's other children with everything they started (a build's `cargo` and `make`). A writing flash session only ever gets SIGINT; a receive-only capture is killed after it. The sequence then runs its usual cleanup: the loader and transient sequences power off, and the demo turns the spares off. During a UART flash write the server refuses the first Ctrl+C and the client says so; only a second within 5 s cancels the write, which then stops for good. Once the cancel is taken, another Ctrl+C exits and leaves the job running; `attach ID` follows it again.

### Virtual panel

The feeder's `virtual-panel` variant lets the bench press the panel's buttons with nobody there, for Bluetooth setup codes, the recovery window and the Dispense press that enrolls an update key.

`press CHORD` runs `wellfed-flash read press`, which types one line, `press <chord> hold <ms>`, on the feeder's UART, one byte every 2 ms, after a line end that ends anything a stray byte started. A chord is spelled as the gesture records spell it: `press lock+dispense hold 5200` opens the recovery window as two fingers would, once `press lock --hold-ms 5200` has unlocked the panel (it starts locked at every boot and relocks after 30 s idle). The build merges the chord's buttons into the levels its button task reads every 5 ms for the hold, so the press goes through the same debounce, holds and gestures as fingers.

The tool requires the build's exact echo, `PLKEY1 virtual press=<chord> hold_ms=<ms>`, then the chord's `PLKEY1 up` record, both as whole lines; it returns once both arrive, or reads until 1.5 s past the hold and judges what it got. The press's report and capture are under `private/captures/press-<ms>`.

- **Only that variant reads the line.** No other Wellfed image links a UART reader (the build links no vendor shell), stock has its own console and the ROM loader reads its download protocol, so `press` stops on the missing echo. It checks only that relay 1 is on, so boot the variant first.
- **Each wrong answer stops the job, and the line is never resent.** No echo: not the variant, or not running. Only `virtual refused`: the line lost bytes (a FIFO overrun refuses it whole). Another press echoed: lost bytes made a different valid line, which the build pressed, so check the capture. A `virtual refused` before the exact echo is a stray byte's line, and the press still counts. No release: the gesture did not finish in the window.
- **Not shipping it is a convention.** The variant is chosen only with `--variant`, and the build identity and manifest name it, but nothing refuses one yet: the builder takes `--provisioning consumer --variant virtual-panel`, which an unattended consumer install needs, and no release signer exists. The release signer, when written, must refuse any manifest that names a variant.
- **A Dispense press is a feed** outside a setup or enrollment window, as a finger's would be. Keep the hopper empty.
- **The bench, not the client under test, presses,** so a pairing that passes with a press here would pass with a person there.

## Flash over UART

1. With the server running, run `target/release/wellfed-bench flash-prepare NAME` with the updater's flags the update needs (`--bank`, `--source`, `--original-read`, `--target`). You should see the plan summary and the exact `flash-execute` line to run next.
2. Review the plan: bank, sectors, erases, image hashes and the plan SHA-256. This is the check before anything is written.

   > [!WARNING]
   > The next step writes flash. Keep the feeder powered between the two commands, and don't rebuild `wellfed-flash`.

3. Run the printed `flash-execute` line. You should see the update complete and the feeder boot.

A failed step stops without retry and leaves the feeder powered for inspection; a stopped update is never followed by a boot. Finish it with a fresh `flash-prepare … --interrupted` session ([development updater](../../development/uart-development.md)). Relay state shows the supply was not switched off between the two commands, not that there was no brownout.

### Finish a bank-0 write

Bank 1 gives way for an update only to a bank 0 that booted and recorded its own image, and a UART write never boots it. So after a bank-0 write, bank 1 refuses every offer `Fallback` (reason `unrecorded`, or `changed` when bank 0 was rewritten) until bank 0 boots once ([OTA design](../../firmware/ota-design.md#bootstrap)), and `wellfed-flash update --bank 0` exits 3, never 0, naming the steps that finish it.

`flash-prepare NAME --bank 0 --bank1-source REVIEW` checks `REVIEW` against bank 1 after the inspection and before `update prepare`, so a refusal leaves no plan:

- Bank 1's first sector, as inspected, must equal the review's image there, byte 0 aside (a given-way bank 1 has it cleared). The rest of bank 1 is not read.
- An image the [bank ledger](bench-server.md#the-bank-ledger) names in bank 1 must be the review's: its OTA image's SHA-256, or the build or stock image it installs. Where the feeder's own report disputes the ledger, the report counts. When the ledger names no image there (no entry, a given-way bank 1, an unfinished write), the prepare says the review was checked against the first sector only.

The prepare binds `REVIEW` and its `review.json` hash to the session; `flash-execute` refuses before the write if it names another review or that one changed.

After the verified write, `flash-execute NAME --bank 0 --bank1-source REVIEW` runs the finishing steps itself, each through its usual prepare, under session names beside `NAME`:

1. **Give way** (`NAME-giveway`) to the image just written, as `give-way-execute --fallback` does, then boot. That boot is bank 0's.
2. **Bank 0 records itself:** its capture must show `PLOTA1 bank0 bytes=… recorded=…` (`recorded=0` when the same image was already recorded). This is a step, not the proof.
3. **Restore bank 1's marker** (`NAME-restore`) against `REVIEW`, as `flash-execute --restore-marker` does, then boot for 170 s. That boot is bank 1's.
4. **Bank 1's check:** its capture must show `PLOTA1 bank0 check=match`.

Before executing each plan, the job checks it pins exactly that step's image (the give-way's bank 0 the target review's OTA image; the restore's source and target both `REVIEW`'s, with the marker restore set) and executes no other; the updater checks the rest.

**A bank-0 write is finished only when bank 1, booted after the marker restore, logs `PLOTA1 bank0 check=match`**, or `UpdateView.bank0` reads `Match` over the device API. Nothing else counts, bank 0's own record included. If a step stops, bank 1 reports another `check=`, or nothing comes within the capture, the job stops without retry (exit 1), saying where the feeder stands and which commands go on:

| Bank 1 reports | Way on |
| --- | --- |
| `unrecorded`, `changed` or `key` | A fresh give-way and restore |
| `marker` | A full read, then a bank-0 write from it (`--original-read`), the only state in which the updater takes that read as the source |
| `extent` | Another build, written with the image just written as `--source` |
| `read` | Another boot of bank 1, then a full read to compare if it repeats, since writing the same image again changes no byte |
| Nothing | Another capture |

When bank 0's own record failed, `store` and `read` get another boot of bank 0, and `extent` the same new build. An error partway (a review that no longer verifies, a boot that reached the loader) prints the same, by what the steps so far wrote, and keeps the error.

**A stopped give-way or marker restore.** The feeder has not booted since, so unless power was cut the job enters the loader once more and tells bank 1's state from `wellfed-flash`'s read-only inspections, writing nothing:

- After a stopped **give-way**, an inspection that accepts the give-way means the marker is intact: run a fresh give-way. One that accepts a marker restore means byte 0 is cleared: boot bank 0, wait for its record, then restore.
- After a stopped **marker restore**, an inspection that accepts `--restore-marker` means bank 1 still has the given-way bytes: run a fresh restore. One that accepts `--interrupted` means the marker sector is erased or staged: run the restore with `--interrupted` alone (the updater refuses the two flags together). A plain inspection that accepts bank 1 as its review means the marker is back: boot bank 1 and look for its check.

Only the matching way on is printed. When every inspection runs and refuses (exit 2), bank 1 is in none of these states, as after a restore stopped between sector 0's program frames or a give-way frame that cleared only some of byte 0's bits. No `wellfed-flash` command applies; bank 1's marker is invalid, so the feeder boots bank 0, and the job prints only a full read to investigate. When the inspections could not run, or one stopped, every way on is printed with the result that selects it.

A bank-0 write reported with exit 0 came from an older `wellfed-flash` than the bench's: the job records the write, says so, and lists the rebuild, give-way and restore steps without running them.

Stock recovery keeps its own commands: `loader`, then `wellfed-flash stock-recovery --device plaf103` directly, or [`stock-prepare`](#to-stock-over-uart).

## Over-the-air updates

`ota` runs one [over-the-air update](../../firmware/ota-design.md) of the running feeder: build, checks, push over the device API, reboot, then build identity and status. The bench never starts the hotspot, so it must be up; a push that cannot start stops before any capture. A bank-0 change still needs the UART updater.

```sh
target/release/wellfed-bench ota unit-01 --build                     # build, review, offer, verify
target/release/wellfed-bench ota unit-02 --review private/verification/<review>
target/release/wellfed-bench ota stock-01 --review private/verification/<stock review>   # back to stock
```

1. **Build** (`--build` only): `wellfed-build build --device plaf103 --profile firstboot` (beside the bench binary, or `--build-bin`), then `wellfed-build review build --build private/firmware-build/plaf103/firstboot --output private/verification/NAME-review`, streamed to the progress log. A nonzero exit stops before signing; otherwise the run goes on as `--review` on that directory.
2. **Sign** in process with `wellfed-build`'s signing library and `private/ota-key.json`: it verifies the review as `wellfed-flash` does (refusing a directory with no `review.json`, an image or manifest that differs from it, a non-pinned "stock" image, or an unattended motion policy) and signs a version 2 manifest.
3. **Push** over the device API as `--keys`: the offer, bank 1's give-way, the offer again to bank 0, the chunks, and a resume after any drop.
4. **Watch** with receive-only UART captures until the new build confirms, falls back, or `--minutes` (20) pass.
5. **Check status:** only for a `feeder` build, since other firmwares serve no device API and their UART confirmation is the whole check. After confirmation, `Status` is read as `--keys` every 5 s for up to 90 s until it shows the offered build on bank 1, and reports the uptime, boot count, Wi-Fi source and clock validity. Each new reason for waiting (an older build, another bank, the request failing) is logged once.

**Choosing a build.** `--firmware qa|optics|buttons|adc|speaker|blink|trial|feeder` picks the build (default `qa`), and `--variant` a variant of it: `stall|fail` with `--firmware trial`; `entropy|owner-stall|held-stall|gap-probe|feed-stall|stop-fail|owner-fail|feed-jam|sensor-fault|buzzer|twin|virtual-panel` with `--firmware feeder`.

- `feed-stall`, `stop-fail`, `feed-jam` and `sensor-fault` move the motor only for a feed request, like the feeder, and inject a fault into it ([motor pulse](../../firmware/motor-pulse.md), [jams and sensor faults](../../firmware/jams-and-sensor-faults.md)). `owner-fail` never opens its feeding owner, so its trial must fall back. `owner-stall` resets about 3 minutes into every boot once confirmed, a reset storm ([OTA design](../../firmware/ota-design.md)).
- The bring-up areas and the trial images are variants of one `bringup` firmware: `--firmware optics` builds `wellfed-build build --device plaf103 --firmware bringup --variant optics` (likewise `buttons`, `adc` and `speaker`), `--firmware trial --variant stall` builds `--firmware bringup --variant stall`, and `--firmware trial` alone its normal image. `qa`, `blink` and `feeder` pass through unchanged.
- The motor pulse (`--firmware motor`, `--variant pulse`) is refused: motion builds stay manual, and go through `--review` with `--attended-motion`.

**Refused before anything runs:** an existing `NAME-review`, `--firmware` or `--variant` without `--build`, `--variant` without the firmware that declares it, and `--build` with `--review`.

**Other options:**

- **Version:** `--version`, or one more than the highest among the signed manifests in `private/verification/`. The manifest is written to `private/verification/NAME-vN.manifest`.
- **Build ID:** the review's `startup_identity`. A review whose `startup_motion_policy` is set needs `--attended-motion`, as for `flash-prepare`.
- **Downgrades:** stock gets a downgrade statement for the feeder's identity and floor. `--downgrade` signs one for a version at or below the floor.
- **Faults:** `--fault corrupt@N`, `unwanted@N`, `stall@N` or `drop@N` spoils the bench's own transfer. `--fault give-way-only` stops once bank 1 gave way, to offer bank 0 another manifest next.
- **Cancel:** `ota NAME --cancel` sends only a `CancelUpdate`, forgetting the candidate a give-way or a stopped install left recorded; without it, bank 0 refuses any other manifest (`Pending`). Run it between failure drills.
- **No stock server runs:** the bench signs through linked `wellfed-build` and offers over the device API.
- **Captures:** 60 s segments `private/captures/NAME-01`, `-02`, … A name with any numbered segment is refused before building or signing.

**Outcomes.** The `PL…` lines are read as each segment ends; stdout and the console show the give-way, the manifest, written sectors as a bar, activation, the new build's start and trial boots.

| Outcome | Lines | Exit |
| --- | --- | --- |
| Passed | `PLFW1 start build=<ID> bank=1`, then `PLCNF1 confirmed=1`, then the status check for a `feeder` build | 0 |
| Health missing | A `feeder` build confirmed on the UART, but the status never showed the build within 90 s | 1 |
| Fell back | `PLTRL1 give_way=1` after activation or the new build's start; reports the last `PLTRL1 confirmed=0 boot=N` | 1 |
| Update error | `PLOTA1 error stage=…` | 1 |
| Refused | The push ended before any UART outcome: the feeder refused the offer (a downgrade without a statement, a manifest bank 0 does not install, another signer) or reported a stop over the API | 1 |
| Timeout | None of these within `--minutes` | 1 |

**Timing.** Once any child has run, the command ends with times for build, review, sign and offer, give-way to activation (the download), activation to confirmation, the status check, and the total. The bench times build, review and offer itself; the feeder phases come from each capture's `serial-events.jsonl` chunk times, or else the end of the 60 s segment, saying so.

After a fallback, bank 0 refuses the manifest that failed its trial, so sign a new version. A cancel stops the child, the capture or the poll, and the job. To install stock, see [to stock over the air](#to-stock-over-the-air).

## Stock and back

Two bench routes go to stock 1.1.15: over the air (`ota` with a stock review, or the [`restore-stock` job](#restore-as-a-job) from the owner's backup), and over UART (`stock-prepare` and `stock-execute`).

From stock to Wellfed over stock's own updater is the Android app's way, from a phone and a real Reset hold, which erases stock's plans ([running an install on the bench](../../firmware/install-bench-checks.md#running-an-install-on-the-bench)); no bench job does it. The suite covers neither it nor the `restore-stock` job, the real path: run both [before a release](../../development/dart.md#before-a-release).

### To stock over the air

```sh
target/release/wellfed-bench ota NAME --review <frozen stock review>
```

A review whose `review.json` says `image_kind: stock` installs through the same signed update as any build: the bench signs the pinned stock payload with its `payload_sha256` as the build ID, plus a downgrade statement for the feeder's identity and floor. The running build gives way, and bank 0's updater writes and verifies stock in bank 1 before the marker. The command passes on stock's `app_main:version:` banner after activation and skips the status step, since stock does not report to this server. Stock's own KM0 then runs, so our trial counter never counts it. The integration suite refuses stock reviews.

> [!WARNING]
> Stock has no trial. If stock failed to boot from bank 1, the only way back would be UART [stock recovery](recovery.md).

### To stock over UART

Two reviewed commands, shaped like `flash-prepare` and `flash-execute`:

```sh
# Bank 1 still holds a valid image: give way to bank 0 first.
target/release/wellfed-bench stock-prepare NAME --fallback <bank-0 review>
target/release/wellfed-bench stock-execute NAME --fallback <bank-0 review> --expected-plan-sha256 <sha>
# Bank 1 invalid (a give-way, or just after stock's own install): recover.
target/release/wellfed-bench stock-prepare NAME
target/release/wellfed-bench stock-execute NAME --expected-plan-sha256 <sha>
```

`stock-prepare NAME` enters the loader, runs `read small` then `read full` in one helper session, refuses while bank 1's marker is valid (as `stock-recovery` refuses a valid target marker), and runs `wellfed-flash stock-recovery --device plaf103 prepare`, which writes the plan from the pinned baseline backup. It prints the next command with the plan's hash: review it, as the check before the write. With `--fallback`, both commands instead give way to that bank-0 review, under `NAME-giveway`, and point back at `stock-prepare`.

`stock-execute NAME` is single-use and requires unbroken power since the prepare. It runs `stock-recovery execute` with the plan hash, reading all 8 MiB three times (about an hour), then boots and must see stock's banner. The boot capture holds the unit's serial and cloud keys: keep it private. The bench reads the plan through `wellfed-formats`' `StockRecoveryReviewed`.

### Restore as a job

`restore-stock` runs `wellfed restore-stock` ([rollback](../../firmware/consumer-installation.md#rollback)) as the server's job, so the server never stops for it: a fresh update key enrolled with a Dispense press (unless `--key` names one the feeder trusts), then stock over the device API. Nothing is offered twice, and a feeder gone after the offer ends it uncertain. `--backup`, `--key`, `--version`, `--keys`, `--feeder` and `--address` pass through; `--virtual-panel` is the server's own.

```sh
target/release/wellfed-bench restore-stock --key private/ota-key.json
```

- The server closes its feeder session for the whole job, and starts the tool once its state shows that, since the tool opens its own sessions only then.
- The tool writes each question as one stdout line (`--prompts lines`), and goes on without waiting for the Dispense answer, since the feeder times its own window. The question goes into the live state (`prompt`: the job, its number, its words and the answers it takes) and the console, and waits for `wellfed-bench answer JOB SEQ done|yes|no` or `job_answer`.
- An answer is taken only for the current question, by its number, and only of its kind (done for an instruction, yes or no for a question). Anything else is refused, never held, since the tool reads answers in order with no number of their own. The job takes the question away before stopping the tool, so an answer the server took (202) reaches the tool unless it has exited.
- With `--virtual-panel`, the job answers `press-dispense` itself on a running [virtual panel](#virtual-panel) build, from its own thread: one 150 ms Dispense press, as a `wellfed-flash read press` child under the server's lease, recorded in the job's output. Nothing unlocks the panel first, since a waiting enrollment takes Dispense on a locked panel too, and the server's capture stops for every job, so the press has the port. The job answers done only once the press echoed and released; otherwise it stops the tool and ends naming the press, pressing nothing again. Just before the press it looks again, and presses nothing once stopping or after the tool exits. The press adds no step row of its own. Without the flag the prompt waits for a person at the panel, who answers done after pressing.
- The tool's stderr is the job's output. A stdout line that is no question stops the tool, and when it exits with an error, the last error it printed is the job's.
- A cancel stops the tool through its SIGTERM handler and takes the question away, as the job's end does however it ended.
- Only a whole restore marks bank 1 as stock in the [bank ledger](bench-server.md#the-bank-ledger); anything else marks it unknown, since the write may have stopped part-way.

## Failure modes

| Failure | Effect | Handling |
| --- | --- | --- |
| Relay board loses USB or the host reboots | Normally-open contacts drop: feeder off, strap released | A running update stops like any other power cut; finish it with `--interrupted` |
| Brownout or supply dip while powered | Relay state still reads on | Not detectable here; the updater's readback and status checks are the guard |
| Strap contact fails at its small current (under 1 mA) | Normal boot instead of the loader | `loader` requires the banner and NAKs, otherwise powers off; nothing is retried |
| UART adapter back-powers the feeder through its TX line | Power-off may not fully reset the chip, so the strap is ignored | Seen intermittently with the manual button (cured by replugging the adapter), not in 32 relay trials. Relays 3 and 4 now isolate the UART while off; holding a UART break while off is the fallback |
| Relay port disappears mid-session | The current command fails; power stays as last set | Cut power by hand if needed, then check `status` |
| Board power also feeds the motor harness | The motor driver is powered whenever the chip is | Only the ROM and flash helper run while flashing, and the driver sits in standby in the loader ([motor driver](motor-driver.md)) |
| UART adapter stops answering control requests | A run stops before its first step | A USB reset of that adapter alone recovered it |

## Evidence

- **Observed, October 22, 2026 (Rust bench):** `status`, `on`/`off`, `boot`, `loader` (99 NAKs) and a no-op `flash-prepare`/`flash-execute` with a captured boot; then a 158-sector bank-0 install with no loader NAK, and a UART give-way.
- **Observed, October 2026 (former Python bench, whose sequences the Rust bench ports line for line):** every command except `transients` ran on the research unit, including a full dump, installs on both banks, the give-way sequence and five refusals that changed no relay and wrote no flash. Relay boots and strap-held loader entries succeeded 16 of 16 times each with the UART adapter connected throughout. Two loader NAKs came in one session, one while `make check` ran beside an install; none came over three installs and four dumps with the host otherwise idle.
- **Observed, October 22, 2026:** `ota NAME --build` ran the whole loop in one command, 5 min 57 s end to end, over three consecutive changed builds with no manual step and no UART write. The classifier, status check and sequence are also host-tested with real log lines, status documents and fake tools.
- **Observed, October 23, 2026:** to stock over the air, then back through stock's updater and promoted (`fw41-roundtrip-01`, 6 min 31 s; 336 sectors written by the recovery role). `stock-ota` installed a `qa` build in about 40 s, booted from bank 0, reported health from bank 0, and promoted to bank 1 where it confirmed (`fw40-stock-ota-05` to `-07`). The stock UART route took about 65 minutes; `stock-prepare` and `stock-execute` are host-tested, and the same `wellfed-flash` steps ran physically.
- **Modeled:** the bench with a fake relay board and flash client.
