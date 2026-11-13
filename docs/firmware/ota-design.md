# OTA updates

This page explains how the replacement firmware updates itself over the network, and how a bad update rolls back on its own. It is for owners who want to know why an update can't brick their feeder, and for contributors working on the updater. The loader's bank rules it builds on are in [bank policy](bank-policy.md).

## The one idea

A new build always goes into bank 1, and bank 0 always holds a known-good build that can feed. The new build's marker is written last, so a half-written bank 1 is never booted. The new build then runs on trial. If it crashes, hangs or never proves it can feed, KM0 gives way to bank 0 after three boots, and the feeder keeps working on the old build.

The constraints behind it:

- **Bank 1 always wins when valid.** The factory loader boots bank 0 only when bank 1's signatures fail.
- **No writing the running bank.** Code executes in place from the active bank, and rewriting it under execution is unsafe.
- **The loader never falls back on its own.** A valid-marker image that crashes is selected again on every reset, and both banks invalid means a wait loop only UART recovers.
- **Writes go through the flash authority,** with outputs quiesced and KM0 parked for each lease, so nothing moves during an update.
- **The update is trusted, not the transport.** The signed manifest carries the image's SHA-256, version, size and target bank. The firmware embeds only public keys, and an eavesdropper learns only the image, which isn't secret.

## Updates over the device API

Offers, downloads and key changes go over the [device API](device-api.md)'s shared core, owner sessions only. Both banks run the `feeder` build, with the firmware, the settings writer and the update stage (`wellfed_hal::ota::update`) in the service task. The bank decides what the stage does with an offer:

| Bank | What it does with an offer |
| --- | --- |
| 1 | Checks it, then checks that bank 0 still holds exactly the image bank 0 recorded at its last boot (length from the bank's headers, SHA-256). It records the offer as pending in the update store and answers `GivingWay`. Once that answer is out, it clears its own marker's first byte and restarts. It never writes bank 1. |
| 0 | Records its own image at boot, only when it changed. Installs only the pending manifest, or the offered one when none is pending (fresh from stock, or after a fallback). Never confirms. |

Some refusals:

- Until bank 1's build confirms, its hooks are unarmed and every offer is refused `Busy`. A bank 1 on trial writes nothing.
- Bank 1 refuses (`Fallback`) an offer signed by a built-in key that bank 0's record doesn't name.
- Bank 0 refuses a manifest that was activated and failed its trial.
- Bank 0 refuses every offer after a reset mid-install, until power is lost.
- A build with battery mode refuses an offer (`Battery`) while the adapter reads absent, since the feeder then turns Wi-Fi off 5 s later ([power](power.md#battery-mode)), and stops an install that is receiving when Wi-Fi goes off. An older bank 0 without battery mode takes the offer.

The bench and the simulator drive the same client steps (`wellfed_client::update`).

### One update, step by step

1. An owner's client offers a signed version 2 manifest to bank 1 (`OfferUpdate`).
2. Bank 1 gives way and restarts into bank 0.
3. The client reconnects; the same identity answers, now from bank 0. It offers the same manifest again.
4. Bank 0 answers `Receiving` and names the chunk offsets it wants.
5. The client pushes them (`UpdateChunk`, 512 bytes, idempotent by offset).
6. Bank 0 erases, programs and reads back each sector, then checks the whole image's SHA-256.
7. Bank 0 records the activation in the update store, then writes bank 1's marker last and reads it back.
8. Bank 0 answers `Activated` and restarts.
9. The new build boots on trial. Confirming raises the floor ([below](#keys-and-anti-rollback)) before the confirmation record is written.

The device API's client pushes each offer once, and never again on its own.

**Clients share what they offer and how they read the end** (`wellfed_client::offer`). An update ends confirmed once bank 1 runs the build that was activated and either reports its version confirmed or is ready to feed: a trial build takes no feed until it confirms, so a ready one has confirmed, and readiness is what settles a reinstall of a build the feeder confirmed before, which skips the trial and leaves the floor below the version. Bank 0 answering means it fell back, and bank 1 running another build means another update came between. An end the app never heard settles by the same rule when the owner looks again, readiness included on purpose: the floor alone would leave such a reinstall unknown for good.

Bank 0 keeps the build it was written with, so these messages are frozen, and so are the manifest, the statements and the image limits it checks: a client of any later build still updates a bank 0 of any earlier one, and reads the bank from the update state, never the status ([the frozen update path](device-api.md#the-frozen-part)).

### The loop never verifies

The device API runs in the loop task, on an 8 KiB stack, and an Ed25519 check needs about 8 KiB. So the loop copies requests and chunks into a mailbox (`wellfed_hal::ota::mailbox`) and answers from what the service task's update stage publishes; nothing on the loop's side checks a signature. In the feeder build the service task's stack bound is 11,504 of 16,384 bytes, its deepest path the install's start (the offer's `verify_strict` is shallower), and the loop's is 5,016 of 8,192, a handshake.

### No flash write while a feed may move

Every update request (offer, cancel, key statement) takes the install lease as it is queued, in the loop that alone starts feeds:

- A request is refused `Busy` while a feed is queued or driven.
- A feed is refused `Busy` while the lease is held: at the device API, in `Feeding::submit`, and in the HAL feeding owner's `submit`.
- The stage releases the lease once it answers, except for an accepted offer: the install keeps it until it stops, and a give-way or an activation until the restart. A cancel during an install rides on the install's lease.

Every update-store save and bank-1 write also takes the write turn (`wellfed_hal::interlock::WriteTurn`), never by the forced path ([how the turn works](device-api.md#no-flash-write-while-the-motor-may-move)). A request waits in the mailbox, unanswered, while the outputs are claimed, and so does bank 0's boot record of its image. An install holds the turn from its first write until it stops, so every feed meanwhile is refused (`held=3`), and a give-way or an activation keeps it until the restart. The floor the confirmation raises is written under the confirmation's own turn. The attempt marker is set only once the turn is held, around each write, so a hang inside one is skipped once after the reset. The install and the settings writer share the service task, so their writes never overlap; the journal writes only with feeds.

### Bootstrap

Bank 1 gives way only to a bank 0 that recorded its own image (length, SHA-256 and built-in key id) in the update store, which bank 0 does at its first boot. A UART write never boots bank 0, so after a UART install of bank 0, bank 1 refuses every offer `Fallback` until bank 0 has booted once, and `wellfed-flash update --bank 0` exits 3 (incomplete) rather than 0. The bench's `flash-execute --bank 0` finishes the write itself: it gives way to the image just written, boots bank 0 so it records itself, restores bank 1's marker and boots bank 1 ([finish a bank-0 write](../devices/plaf103/bench.md#finish-a-bank-0-write)).

**A bank-0 write is finished** only when bank 1, booted again, logs `PLOTA1 bank0 check=match`, or `UpdateView.bank0` reads `Match`. Bank 0's own `bank0 bytes=… recorded=…` is the step before, not the proof.

**A refusal names the check that failed:**

| Reason | Check |
| --- | --- |
| `marker` | Bank 0's loader marker is invalid |
| `unrecorded` | No record |
| `key` | Bank 0's built-in key is not the offer's |
| `extent` | Bank 0's headers give no image |
| `changed` | Its length or hash differs from the record |
| `read` | Bank 0 couldn't be read |

The reason goes on the wire as `UpdateRefusal::Fallback { reason }` and in `PLWCH1 refused give_way reason=…`, with the recorded and found lengths and the first four bytes of each hash where the check got that far. A failed give-way write after every check passed is `PLWCH1 error stage=give_way_write`.

**The boot check.** Bank 1 runs the same check once per boot without an offer (all but `key`, which depends on the offer), so an owner sees a blocked bank 0 before an update is refused. It runs after the healthy window, once the hooks are armed, and pauses while a feed holds the outputs or the stage has work; an offer's own check replaces it. It hashes 8 sectors a step through mapped reads, with no lease or write turn, logs `PLOTA1 bank0 check=…` once, and fills `UpdateView.bank0` (`Unchecked` until then, and always from bank 0).

Both banks must carry the same built-in keys. Bank 1 checks the offer's key against bank 0's recorded key id before it gives way, so an offer bank 0 couldn't admit never strands the feeder on bank 0.

### Stops

Every failure before activation leaves bank 1's marker clear, so the loader keeps booting bank 0, which keeps answering and feeding. The failures: a flipped byte (the whole-image hash), a chunk not wanted or of the wrong length (order), a failed program or readback (write), a minute without a chunk (timeout), a cancel, an update store that can't be written, and a manifest or key the checks refuse.

**A marker that doesn't read back** is not one of them. Once its program has been tried, the marker may be down whatever the program or its readback reported (a stale read, a program that wrote and then reported a fault), so bank 1 may be valid. The activation stands, a stock image's handover mark stays, and the feeder restarts as after any activation (`PLOTA1 activated … marker=uncertain`); the boot decides. If the marker landed, the new build boots and raises the floor as it confirms. If none of it landed, bank 0 boots again with the manifest activated over a marker still erased, which the loader never booted, so an offer of it installs again; a marker with any bit down (some of it landed, or a trial gave way) reads as a build that failed, and is refused from then on. A cut between the activation record and the marker leaves the marker erased too, so it installs again. Reverting to the pending offer would let a build that boots from that marker confirm below the floor. Only a marker never programmed stops the install (`stage=marker`).

- The loop records each stop as an `UpdateStop` event, since the update's state forgets it at the next offer or restart ([alerts](alerts.md)).
- A stop leaves the candidate recorded, so bank 0 refuses any other manifest (`Pending`) until a `CancelUpdate` forgets it (`wellfed-bench ota NAME --cancel`). A cancel answers `Busy` when the store can't be written, rather than claiming it forgot.
- A dropped connection loses nothing: the install waits a minute for the next chunk, and `UpdateState` says where it stands.
- A power cut mid-download boots bank 0 again, which takes the offer again from the start.
- A reset mid-install sets the update-attempt bit (backup register 1, bit 9, beside the flash attempt bit). Offers are then refused until power is lost; the hooks still arm, so bank 0 still saves and feeds.

**A failed write says why.** A failed sector write stops the install with:

```
PLOTA1 error stage=write sector=<bank-1 sector> step=erase|program|read|readback offset=<byte> expected=<word> read=<word> reread=<word> erase_ms=…
```

On a readback mismatch the record holds the first differing word as programmed, as read back, and as read again with no write between. The second read first invalidates the data-cache lines over the word, so it reads the flash rather than the line the first read filled. A 0 bit where a 1 was programmed means the erase didn't finish; `ffffffff` means the program didn't take; a matching second read means the cells are right and the read path is at fault. The install still stops; nothing is rewritten.

**Lease timing.** Each `PLOTA1 progress` line is followed by `PLOTA1 leases count=… park_max=… release_max=… masked_max_ms=… masked_long=… feed_gap_max_ms=…`: the leases so far, the slowest KM0 park and release exchanges (in 32 768 Hz ticks), the longest time IRQs were held masked (from the acquire's masking, request and quiesce included), how many leases held them past 100 ms, and the feeder task's longest gap between checkpoints on the system timer. A lease's acquire masks IRQs before it reads the lease clock, so a preemption between the request and the park can't count against the park budget and halt the board.

## Keys and anti-rollback

**Manifest version 2** (`wellfed_signed::offer`) is 96 bytes, then the Ed25519 signature:

- `PLOTA2\0\0`, the version, the image length, the target bank;
- the image kind (1 replacement, 2 stock), the device model it is built for (`wellfed_proto::device::Model::code`, never 0) and two zero bytes;
- the signing key's id (the first 8 bytes of SHA-256 over its public key);
- the image's SHA-256 and the build ID.

A feeder admits only its own model: once the signature checks, a manifest naming another is refused (`Target`) before anything is written. The server lists releases by model (`GET /releases?model=`) and offers one only to a device of that model (`409 other_model`). The model travels in the manifest, which heads every release file, so the OTA container and bank image don't carry it.

Version 1 (`PLOTA1`) is deleted: an offer carrying its magic is refused as the wrong version. The bench still reads the versions of the version 1 manifests on disk when it picks the next.

**Trusted keys.**

- The key built into the firmware, `WELLFED_OTA_PUBLIC_KEY`: the bench key in a bench image, the project release key in a consumer image ([bench and consumer images](../development/firmware-builds.md#bench-and-consumer-images)). A consumer build refuses to run without the release key's public half rather than fall back to the bench key.
- The release slot, empty in the build and filled once by a `PLKEY1` statement a built-in key signed (`UpdateKeyStatement`), so a release key arrives without reinstalling bank 0.
- Owners' update keys, enrolled over the device API (`EnrollUpdateKey`) only once someone presses Dispense at the feeder, and removed like any client.

**Enrolling an owner's key.** `wellfed update-key new --output FILE` writes an owner's key in the bench key's file format, marked `"scope": "owner"`, and refuses to replace a file. `wellfed update-key enroll --key FILE` enrolls it:

- Only an owner's session that claimed updates may enroll one; the CLI refuses a member before anyone is asked to press.
- If the feeder already lists the key as an update signer, nothing is sent. If it holds the key as a client that opens sessions, the CLI refuses: an update key is a key of its own.
- Otherwise the CLI sends `EnrollUpdateKey` and asks you to press Dispense within 60 s. No press in time is refused `Unconfirmed`, and nothing is enrolled.
- A key that doesn't decode as a point, or one of small order, is refused `Invalid` before any press is asked for. Verification is strict, so such a key could sign nothing anyway.
- With no answer the outcome is uncertain, since a press the feeder took before the session ended still enrolls the key. The CLI says so and never sends the request again; check with `wellfed update-key show`.
- `wellfed update-key show` lists the enrolled keys with their key ids; they also appear in `wellfed clients`.

The bench key enrolls the same way, so a feeder whose image embeds no key still takes the updates `wellfed-bench ota` signs. An owner's key signs manifests and downgrade statements, which the firmware checks against every trusted key. It never signs a key statement, and no image embeds it: `wellfed-build revoke`, `add-key` and the build itself refuse it. The client side is `wellfed_client::update_key`, which the installer reuses.

A `PLREV1` statement retires any key id for good. Built-in keys and the release slot's key sign key statements, so retiring the bench key leaves rotation to the release key; a retired key signs nothing, and no key signs its own retirement.

**Rotating the release key.** The slot holds one live key, and filling it again is refused while that key is live (`Occupied`). Once the slot's key `R1` is the only one left that signs statements, statements alone can't replace it: `R1` adding `R2` finds the slot taken, and `R1` retiring itself would leave no key to sign the add. The release key never adds another key either: the statement names no feeder, so it would fill the empty slot of every consumer feeder it reached, and `add-key` refuses it. The rotation goes through a build:

1. Build the consumer image with `R2`'s public half as its release key, and sign it with `R1`: `wellfed-build sign --key <R1's file> --hand-over-to <R2's file>`, both on offline media. The feeder takes it through the slot.
2. Running it, `R2` is built in. Sign two statements with `R2`, `revoke` for `R1` and `add-key` for `R2` itself, and send them together: `wellfed update-key statement <revoke> <add>` sends both over one session, in that order, and reads both files before it sends either.
3. Bank 0 still builds in the key it was written with, and now finds `R2` in the slot, so it takes `R2`'s next offer and no longer `R1`'s.

The first rotation, from the built-in key `R0` with an empty slot, goes the same way: `R0` signs a build that builds in `R1`, and running it `R1` retires `R0` and adds itself.

Between the revoke and the add, bank 0 trusts no release key: its own built-in key is retired and the slot holds none. The running build still trusts `R2`, so nothing is lost while it runs, but a fallback to bank 0 then could take no update but over UART. So:

- Send the revoke and the add back to back, with no offer between them, as `update-key statement` does.
- If the add's answer doesn't come, send the add again until the feeder answers it. Adding the key the slot already holds changes nothing, so resending it is safe.
- Keep `R1` and keep signing with it until every feeder has taken the hand-over build: a feeder that hasn't still trusts only `R1`.
- Each rotation spends one of the store's 16 retired ids (`REVOKED`). With the list full, the revoke in step 2 is refused (`Full`) before anything changes, and the add then finds the slot still taken: the feeder keeps trusting `R1`. Rotate rarely.

The bench key stays in `private/ota-key.json`, for the bench only. The release key is made offline with `wellfed-build release-keygen` and never enters a checkout; consumer images build in its public half ([the release key](../development/firmware-builds.md#the-release-key)).

**The floor** (`min_version`) is the highest version a build confirmed at.

- An offer at or below it, and every stock image, needs a `PLDWN1` downgrade statement signed by a trusted key, naming this feeder's identity key, the exact manifest's digest and the current floor. A replayed old manifest, a statement for another feeder, or one made at another floor is refused. The statement binds the floor's value, not when it was set: the handover from stock's activation resets the floor to 0 (below), so a statement made at an earlier floor of 0 reads as current again, for the same feeder and the same manifest only.
- Confirmation raises the floor after arming, and before it erases the confirmation sector. A store that can't be written leaves the build unconfirmed.
- Sector 4's version field carries the version bank 0 activated: 0 for a build installed over UART, or after a torn confirmation rewrite (the store's candidate was already confirmed, so the retry finds nothing naming the build). Nothing decides on that field; the floor lives in the update store.

**The update store** (`wellfed_core::store::update`, storage sectors 25–26) is one 384-byte record, A/B, newest valid wins. It holds the floor, the candidate (pending, then activated), the release slot's key, up to 16 retired ids, and bank 0's recorded image. A torn write leaves the older, consistent record. It isn't the confirmation sector, which is erased before each write.

The store never writes over a record of its own it can't read. With no valid record, it starts empty only when both sectors are erased or hold a first save cut short. Anything else (an older major, a record that fails its checks) opens corrupt, and a newer major read-only. Beside a valid record, an unreadable one is the sector the next save goes into: the [record rules](pairing.md#record-versions) keep a newer build's records readable or in other sectors, so it holds nothing newer. Both leave the floor unknown, so every offer is refused. The factory reset leaves a readable store alone. It erases a corrupt one, and only when you held Reset on the panel. Anyone at the feeder could erase it over UART anyway, so this gives you a way back to updates without a cable.

A record that would lower the floor or forget a retired id is never written, with one exception: the handover from stock's activation resets the floor to 0, since an install from stock must take the same version again. It clears a pending candidate, which the install run never cancels before its promotion, and keeps an activated one, so that exact manifest stays refused as failed whether or not stock booted. That holds too when the loader falls back to bank 0 because stock didn't boot, since the owner's signed downgrade already reopened the floor. The retired ids, the release slot's key and bank 0's recorded image stay.

**Signing** uses `wellfed-build sign` (version 2), `downgrade`, `revoke` and `add-key`, with the bench key. `sign --key` and `downgrade --key` also take an owner's key; `revoke` and `add-key` refuse one. `wellfed-build` signs only a frozen review.

## Storage-format rule

Bank 0 never updates over the air, so the build the installer writes there reads every store for the life of the feeder. A store bank 0 can't read has no floor it can trust, so bank 0 refuses every offer, and the unit would be stranded until a UART install.

So the records bank 0 reads change only by additive minors ([record versions](pairing.md#record-versions)). Bank 0 reads the fields it knows from a newer minor and keeps the rest when it rewrites the record. A change that isn't additive is a new major, and a new major is a new store in new sectors.

## Boot confirmation and fallback

KM0 runs the trial before it starts KM4. Rust decides and writes the `PLTRL1` lines (`wellfed-hal-common/src/trial.rs`); `wellfed_trial_step` in `firstboot_km0.c` reads flash and the register, prints the lines, and does the give-way's marker write and watchdog reset.

- **Running bank:** read from bank 1's physical marker.
- **Confirmation** is keyed by the build identity, not the image hash, since the running image doesn't know its own OTA hash. The record lives in storage sector 4 (`0x344000`). So reinstalling the build the record names skips the trial: KM0 logs `PLTRL1 confirmed=1` straight after activation, and the build `PLCNF1 confirmed=1 already=1`. A test that needs a trial must install a build the record doesn't name, or go through stock, whose handover on bank 0 erases the record.
- **Count:** backup register 4: bits 0–15 the peer boot count, which KM0's peer task bumps at its start (`trial::boot_counted`), 16–19 the trial count on a trial boot (with bit 19 set, the storm's backoff level on any other), 20–23 the [storm](#reset-storms) count, 24–30 an identity tag, and 31 "forgive no brownout next boot" (`trial.rs` documents the layout).
- **Give way:** at boot 4, and only if bank 0's marker is valid, KM0 clears byte 0 of bank 1's marker and resets with its own watchdog. The program step bypasses `FLASH_Write_Lock`: after a watchdog reset, that lock's KM0 branch waits forever for an absent KM4, because `km4_status_on` still reads the HS clock bit set.

**How a build confirms.** Once the startup probe has ended, the link is live, and outputs have been verified stopped for 30 s continuously, the build arms the hooks, writes the record to sector 4 and reads it back (the service task in `wellfed-sdk`'s `runtime.rs`). A live link is associated, leased, and the lease's gateway answering an ICMP echo, or never having answered this join ([link liveness](startup-diagnostics.md#link-liveness-without-internet-dns)). It needs no internet DNS, so a build confirms on a LAN with no route out. A gateway that goes silent after answering restarts the 30 s.

**Supervision from startup.** The feeder task starts first and opens the watchdog on its first step, before the startup diagnostic (`PLSUP1 open watchdog=1 trial=N`), so a fault in early startup is a counted reset, not a hang. A watchdog that will not open (its enable bit already set, or a register that does not verify; `PLSUP1 open watchdog=0`) is a startup failure too: the task holds writes and resets the platform through the watchdog's reset path, unfed (`watchdog::force_reset`, `PLSUP1 stop watchdog=forced reset=1`), so KM0 counts the boot and a trial gives way. It never runs on unsupervised, where a reboot, the trial deadline, flash arming and updates would each wait for a reset that never comes. A halt that cannot open the watchdog ends the same way. The counter is programmed once and the watchdog never opened again after it (`watchdog::ForcedReset`): opening would program the counter afresh and a failed open disables it. If a whole expiry passes with no reset, the counter is enabled again (`PLSUP1 retry watchdog=forced reset=1`), which is safe whether or not enabling restarts the count on this chip. Host-tested against the fake register model, not observed on hardware. The decision lives in `wellfed-hal`'s `supervision` module:

- **Startup:** feed while the startup task keeps stepping.
- **A determined startup failure:** a trial image stops feeding, so the failure is a counted reset. A confirmed image keeps feeding and holds STOP, so there is no reset loop.
- **Running:** feed while every running task steps and outputs stay stopped or sensing.
- **Trial deadline:** a trial image that hasn't confirmed within 10 minutes of boot stops feeding.

Each stop logs `PLSUP1 stop …` once.

**The feeder task's priority.** It runs at FreeRTOS priority 7, above the vendor Wi-Fi driver's threads (5 and 6), whose WPA3 SAE work on each join takes 0.85–1.0 s (twice with hash-to-element) and starved it at priority 3 ([evidence](#evidence)). Its step never blocks, and a confirming trial prints its worst gap (`PLSUP1 trial feed_gap_max_ms=`). A driver thread that hangs with interrupts running still starves the beats the feeder checks, so feeding stops within 10 s at rest (the service task's beat) and 1 s while outputs may move (the owner's). A thread that takes more than 1 s but less than 10 s of each window at rest is no longer caught, which is acceptable with outputs stopped. Firmware tasks are refused at or above the feeder's priority (`Task::__new`).

### What confirmation proves

A healthy network proves the update path works, not that the build can feed. So a trial build of a firmware with a feeding owner (`feeder` declares one with `wellfed_sdk::claim_feeding` in its start hook) confirms only once all of these hold:

1. **The runtime's own health has held for 30 s:** the startup probes finished, outputs verified stopped, and the link live.
2. **The confirmation has armed the flash hooks.** The owner opens only on armed hooks, and on a trial boot this is the first arming, so the confirmation arms and then waits. Arming and the record's write each take the write turn and set the attempt marker for themselves only, so the marker is clear while it waits.
3. **The owner has proved itself** (`feeding::Proof`, reported each pass with `wellfed_sdk::report_feeding`): it opened on its journal, resolved any feed a reset interrupted as uncertain, and stayed at rest for 2 s, Idle with no owner or supervisor fault and motor STOP and the emitters off read back. That is about a hundred of its 20 ms polls, each reading the sensor inputs, and about twenty battery conversions. So a trial reads the food beam at rest only after it has confirmed ([optical inputs](optical-inputs.md#reading-the-food-beam-at-rest)): a window's lit emitters would restart this and step 1's 30 s.
4. **The link is still live at that moment.**

Then the floor rises and the record is written. A firmware with no feeding owner (`blink`, `qa`, the bring-up areas) confirms on step 1 alone.

**A build whose owner can't open never confirms.** If the owner fails determinedly (its journal can't be opened, or it faults in setup, recovery or a check), a trial boot stops feeding the watchdog at once, as for a failed startup diagnostic, with `feeding=3` on the `PLSUP1 stop` line; KM0 counts the reset and gives way after three. If the owner never gets there (arming skipped, a hang), the 10-minute deadline resets the boot. A confirmed build, or bank 0, whose owner fails holds STOP instead.

**Feeds wait for confirmation.** An unconfirmed trial refuses feeds from clients and the Dispense button (readiness `Blocked::Starting`), so the confirmation never waits on a feed towards the deadline.

- The owner's proof takes about 2 s after arming, so a healthy trial confirms a few seconds later than on health alone. The window from the activation's reset to confirmation is typically 2–3 minutes (the boot, the Wi-Fi join, 30 s of health).
- With the network down a trial can't confirm: each trial boot runs to its 10-minute deadline, and after the third (about 30 minutes) KM0 gives way to bank 0, which feeds.
- **For schedules:** a meal due in this window is refused like any feed, so the schedule must not drop it silently. It should retry once readiness turns `Ready`, if still within a grace period after the meal's time, and otherwise record it as missed. It must never feed it twice; the feed's write id stays the same across the retry.

**A valid bank 0, to KM0,** is its marker and the five core headers behind it, read with the same check KM4 makes before its own give-way (`trial::fallback`, over `wellfed_signed::image::extent`). The loader refuses a bank whose KM4 header fails even with its marker intact, so a trial or a storm never clears bank 1's marker over such a bank 0: it runs on as one with no bank 0 (below), and the feeder keeps something to boot.

**A journal that stays unreadable** (corrupt in flash, not just a bad build) makes every trial fall back, since the owner fails to open on every trial boot. Bank 0 then runs with `Blocked::Journal`; it never confirms, so its failed owner holds STOP rather than resetting. No update can confirm until the journal is repaired or recreated. The `owner-fail` variant of `feeder` exercises this: its owner never opens (the journal reads as failed and is left untouched), so each trial boot stops at once and the third gives way to bank 0.

### Reset storms

Confirmation proves one boot, but a confirmed build can still fault into a reset soon after every boot. So KM0 also counts every boot that runs no trial (a confirmed bank 1, bank 0, or a trial that ran out with no bank 0) in bits 20–23 of backup register 4, and KM4 gives the count back once a boot that isn't held has run 10 minutes (`trial::STABLE_MS`, the trial's deadline; `PLSUP1 storm settled=1`) or before a restart it asks for (a reboot, an activation, a give-way, setup). The count KM0 finds is the number of boots in a row that ended, within 10 minutes, in a reset nobody asked for. Every boot reports it: `PLTRL1 confirmed=1 resets=N` or `PLTRL1 bank=0 resets=N`, and KM4's `PLSUP1 storm resets=N level=L mode=…` when either isn't zero. The rules are in the module docs of `src/sdk/wellfed-hal-common/src/trial.rs`; in short:

- **Brownouts are forgiven, within a bound.** On a boot whose `BOOT_Reason` carries the brownout bit (bit 18), KM0 removes the previous boot's pending count (`trial::decide_after`; `PLTRL1 brownout=1 reg=…`), so a weak pack can't give a good bank 1 away. A trial's own count is never touched. The bit may be stale, so a boot that forgave sets bit 31 (`trial::FORGIVEN`) and the next forgives nothing; a halt's own reset sets it too. So a fault still reaches the storm limit within 10 early resets and the halt within 16. Whether a real brownout on this board sets the bit is unverified ([power](power.md)).
- **At 5 resets** (`trial::STORM_LIMIT`) a confirmed bank 1 with a valid bank 0 gives way, as a failed trial does (`PLTRL1 storm=5 limit=5`, then `PLTRL1 give_way=1 …`), leaving no count for bank 0. It was confirmed, so the floor stays at its version, and a fixed build must carry a higher one.
- **Otherwise** (bank 0 itself, or no valid bank 0) KM4 boots **held** (`PLTRL1 storm=N held=1`, `PLSUP1 storm resets=N level=L mode=held`): the feeding owner stays closed and the outputs stopped, readiness reports `Blocked::Fault`, and the device API, settings writer and update stage run so the owner can read why, install a fix or reboot. A held boot that runs 10 minutes × 2^level (10, 20, 40, 80, at most 160; `trial::hold_ms`) opens the owner and feeds in place (`PLSUP1 storm released=1 level=L`), the count cleared and the level one up. Every stable boot takes a level back: a normal boot after 10 minutes, a released boot once it has fed for its level's hold.
- **At 8 resets** (`HALT_LIMIT`), when held boots keep resetting too, KM4 starts nothing but a timer for an hour (`trial::HALT_MS`; `PLSUP1 storm resets=8 … mode=halted`): no network, no owner, outputs as `stopped::initialize` left them. It then leaves a held count a level up and lets the watchdog reset the board (`PLSUP1 storm halt_over=1 reset=1`), so the next boot is held, not halted. The `held-stall` variant reaches this on the bench: a priority-3 task starves the service task about a minute into every boot, held boots included, which `owner-stall` can't do (a held boot owes no owner beat).
- **A trial that runs out with no bank 0** (`PLTRL1 error fallback=invalid boot=3`) runs on with no trial deadline (`trial::past_trial`), held at once and then released, its resets counting as a storm's; it can still confirm later. That holds only while bank 0 stays invalid: once bank 0 is valid it is back on trial from 0. The same rule stops a count another build left under the same tag (a reinstall, or one identity byte in 128) from letting a new build skip its trial. An activation or give-way restart clears the counts outright.

**Nothing is permanent.** A build that faults at every boot resets at most 14 times in its first burst (6 on bank 1, then 8 on bank 0), then a handful an hour around each halt. The counts live in backup registers, so there is no flash record to tear. They survive a watchdog reset and a brownout, and clear only on **power loss, which means the adapter and the D cells both out.** One case the count misses: a build that crashes just after 10 minutes on every boot settles first, so it resets about every 11 minutes indefinitely, feeding between. The simulator's banks run the firmware's own `trial::decide`; on hardware the `owner-stall` variant storms, resetting about 3 minutes into every boot once confirmed, so KM0 gives way at the sixth boot after the one that confirmed.

### Recovering a held or halted feeder

- **Held.** The app and server reach the feeder, but every feed is refused with a fault (`Blocked::Fault`), and the log tail shows `PLSUP1 storm … mode=held`. It feeds again by itself once the hold ends: 10 minutes the first time, doubling for each storm in a row, at most 160 minutes. To retry at once, reboot it from the app, since a requested reboot clears the count. If it keeps coming back, install a fixed build; updates install as usual while held.
- **Halted.** The feeder doesn't answer at all for up to an hour, then comes back held. Waiting is enough. To start over at once, cut power fully: unplug the adapter **and** take out the D cells for a few seconds.
- **Bank 1 gave way.** The feeder answers as its bank-0 build (the update view shows bank 0) and feeds. Install a build with a higher version than the one that stormed.

### The attempt marker and decided resets

Every store write sets the flash attempt marker for its duration. A boot that finds it set skips arming once per power cycle, so a write that hangs can't reset the board over and over.

A reset the runtime decides never lands in a write it let start. Otherwise a supervision reset could land in a store write begun seconds before, leaving the marker set, and the next boot would skip arming and the feeding owner wouldn't open until a power cycle.

- From the first feeder pass that doesn't feed the watchdog, transient or certain, `supervision::writes_held` holds every new write: no write turn is granted, no arming starts, and an install (which holds its turn across sectors) goes no further. A transient stop holds writes for under a second, a certain one until the reset.
- A write already running when the feeding stops has the rest of the watchdog's expiry, at least 2.4 s less a feed period, and takes tens of milliseconds.

So the marker stays set only after a reset the runtime didn't decide (a hang, a fault, a lease that halted), which is what it is for. A withheld reset doesn't clear it, and the storm breaker bounds the rest.

## Resets that tell you why

Each reset that would otherwise leave no line on the UART leaves a breadcrumb for the next boot.

- **A failed flash lease.** The hook halts in RAM with IRQs masked, so nothing reaches the UART before the watchdog resets the board. The provider writes the failure into the attempt register's top bits (bits 31–10; the attempt and update bits keep 9–0), which survive a watchdog reset. The next boot reports it before anything arms, then clears those bits:

  ```
  PLSUP1 lease_fault stage=<LeaseStage> phase=<FlashPhase> fault=<FlashFault> reply=<FlashReply> elapsed_ms=<…>
  ```

  `elapsed_ms` counts from this lease's request in 16 ms steps; 1008 means 1008 or more, and 0 a request-stage failure. The skip rule still reads only the attempt bits.
- **A refused watchdog checkpoint.** A gap over the 1 s progress limit, an altered register or a clock fault latches, and the hardware is never refreshed again: `PLSUP1 stop watchdog_fault=<WatchdogFault> gap_ms=… fed_ms=… ms=…`. It shares one latch with `PLSUP1 stop startup=…`, so a boot logs whichever comes first, once.
- **A hang that stops the feeder task itself** (a spin above its priority, or interrupts masked). The tick hook leaves the interrupted task and the time since the feeder last ran in backup register 2, and the next boot after a watchdog reset prints `PLSUP1 hang kind=spin|masked|stopped task=… since_ms=… suspended=…` ([silent watchdog resets](startup-diagnostics.md#silent-watchdog-resets)).

**A restart leaves the network first.** Every intentional restart (activation, give-way, a requested reboot) asks the Wi-Fi task to disconnect while the watchdog runs out, so the access point drops the association instead of rejecting the trial boot's join with status 30. After such a rejection the vendor's `wifi_connect` can hang the board into a silent reset and spend a trial boot. Feeding stops at once regardless ([leaving before a restart](startup-diagnostics.md#leaving-before-a-restart)).

## Bank-0 download faults

Bank 0 used to fault now and then while downloading: hard faults at random places, imprecise bus errors, and once a single-bit read error caught by readback. Lowering the flash clock from 80 to 50 MHz stopped them. The working explanation, not yet established, is that the board's flash read margin at 80 MHz was too thin under download load. The builder sets KM0's `Flash_Speed` to 50 MHz; the read mode stays Dual IO, since the Quad check fails at the slow boot clock.

## On the bench

`wellfed-bench ota` and `itest` push updates over the device API ([bench](../devices/plaf103/bench.md)). No motor moves.

- In the push model a truncated download is a stall: the bench stops sending and the install times out (`stage=timeout`).
- Between failure drills, `ota NAME --cancel` forgets the candidate a stop left recorded.
- The hands-off round trip offers bank 0's build back from bank 1, so the build in bank 1 must have another build ID: R1 is the feeder's `twin` variant (`--variant twin`), R0 the plain feeder.
- A bank-0 build is first installed by a reviewed UART install (`flash-prepare/flash-execute --bank 0`); bank 1 keeps running until a give-way.
- Bring-up firmwares (`qa`, `optics` and the rest) serve no device API, so a unit left on one in bank 1 takes no offer. Leaving it needs a UART install or a failed trial, and after installing one the bench has only the UART confirmation (`wellfed-bench ota` says so).
- If a new build's own KM0 code is broken, nothing counts its boots, and UART recovery is needed. That stays the backstop, and the relay bench can do it unattended.

## Evidence

- **Observed** (October 27, 2026): both banks run the same `feeder` build at a 50 MHz flash clock. Bank 0 was written over UART and booted by a give-way that recorded its image. Then 8 of 8 installs, alternating a `twin` and the baseline through bank 0's download, passed with no fault, each a full download and a confirmed trial (`otaloop-20261027-n01`…`n08`). That is one evening's runs, not yet the sustained run.
- **Observed** (October 27), the faults before the change: at 80 MHz, bank 0's downloads faulted in 8 of 10 installs in one series (`otaloop-20261027-d01`…`d10`) and 6 of 8 after a full power-off (`g01`…`g08`). The dumps differed each time: a SecureFault on an instruction fetch in `FlashCoord::request`; a SecureFault in the SysTick handler through a corrupted ready-list pointer; an imprecise BusFault on `wellfed-loop`; a fault in `__aeabi_memclr4`. One install failed `stage=write` on a transient single-bit read error: `sector=237 step=readback offset=604 expected=bee707f1 read=bee707f5 reread=bee707f1`. At 50 MHz the boot's flash calibration window reads 29–31 (`calibration_ok:[2:31:17]`, 15 boots), against 11–17 at 80 MHz. Flash wear is ruled out by the feeder's own erase counts.
- **Observed** (October 24), earlier bank-0 faults: 3 crashes and 1 silent hang in 157 bank-0 boots that started Wi-Fi, none in 199 bank-1 boots of the same build: in `cmd_thread` (`fw47-itest-05`), twice in `rtw_recv` (a backpower run, `warm-20261024-05`), the last symbolized to `report_survey_event` under `OnProbeRsp`, 11 s after boot, before any download. Two faulting PCs hold valid instructions, so the CPU fetched wrong bytes from flash (static analysis). Ruled out then: a power cut, UART back power (relays 3 and 4 now isolate it), the old `qa` image, and our leases, which masked IRQs for 41–44 ms at most, none past 100 ms. The Wi-Fi driver takes no leases (17 per written sector, exactly), and both cores hold `PMU_OS`, so nothing sleeps.
- **Observed** (October 26–27), what confirmation proves: an `owner-fail` build stopped each of three trial boots with `owner=0`, and KM0 gave way to bank 0. An `owner-stall` build confirmed, then reset about 190 s into every boot (`hang kind=stopped`); after `storm resets=1…4`, KM0 logged `storm=5 limit=5 give_way=1` and bank 0 started.
- **Observed** (October 27), a held boot: the `owner-stall` build, written to bank 0 and given way to, reset five boots in a row (`PLTRL1 bank=0 resets=1…4`). The sixth booted `PLTRL1 storm=5 held=1`, readiness `blocked reason=fault`, and the device API answered from bank 0. The held boot ran its stalls without a reset, and 10 minutes in logged `PLSUP1 storm released=1 level=0`, opened the owner on its journal and went `ready`.
- **Not yet run on hardware:** the halted state, which needs a variant that resets while held; a real brownout setting the forgiveness bit; the standalone `wellfed update-key enroll` command, though it shares its enrollment with the installer, which has run.
- **Observed** (October 30), enrolling an owner's key: a consumer image on the research unit enrolled the installer's update key after a Dispense press, then installed an update that key signed and confirmed it on bank 1.
- **Simulated** (November 13, 2026), what a trial and a storm show clients (`wellfed-feeder-sim/tests/trial.rs`): on the simulator's loop, a trial build reads `blocked reason=starting` until it confirms, half a second into its boot (the simulator's scale of the firmware's healthy window); one that never confirms reads `starting` until its deadline's reset; one whose confirmation record cannot be written stays in trial, resets at each deadline and gives way to bank 0 after three boots; and a boot a storm holds reads `fault`, its feeding owner closed, until its level's hold has run, then is released a level up and feeds. The update stage leaves a request waiting while a feed holds the outputs (`the_update_stage_waits_for_a_feed_holding_the_outputs`).
- **Simulated** (October 29, 2026), enrolling an owner's key (`wellfed-feeder-sim`'s update tests): a press enrolled the key and a feeder embedding no key then installed an update it signed; an unenrolled key was refused `Key`, no press `Unconfirmed`, a member `Forbidden`; a session cut after the press left the key enrolled and the outcome reported uncertain.
- **Modeled:** `wellfed-signed` (`no_std`, `ed25519-dalek` and `sha2` without default features) and `wellfed-core::ota` refuse tampering, the wrong key, a bank other than 1, a size outside one sector to 0x150000, downgrades, out-of-order chunks, a missing marker, failed readback, corruption and truncation, and the writer withholds the marker until every sector reads back and the whole-image hash matches; host tests include RFC 8032 test 1. Every stop runs against the fake SDK (`wellfed-hal/tests/update.rs`), and the client end to end over the fake and over TCP against the simulator's two banks, loader and trial (`wellfed-client/tests/update.rs`, `wellfed-feeder-sim/tests/update.rs`). A test checks a `wellfed-build sign` manifest against the firmware's offer check. In emulation, `every_ota_step_and_marker_cut_keeps_a_bootable_bank` (`src/tools/wellfed-emu/tests/factory.rs`) hands every update step, a bank-1 marker cut after each of its 8 bytes, and erased or staged bodies to the factory loader's own selection code: all 14 states pass for stock and the replacement, and only the forbidden step (giving way with no valid bank 0) leaves nothing to boot. Any single cleared bit in bank 1's marker gives way to bank 0. `a_trial_or_storm_never_gives_way_onto_a_bank_0_the_loader_refuses` runs KM0's trial decision over stock in both banks with bank 0's KM4 header damaged (marker intact): through a trial to its limit and a storm, the loader still selects bank 1 after every boot, and with bank 0 intact both give way to it. The breaker and the decided-reset rule are host-tested and simulated.
- **Observed** (October 22), the trial: the `stall` variant (`wellfed-build build --profile firstboot --firmware bringup --variant stall`) was counted 1, 2, 3, then gave way to bank 0. Over the air, so were a failed-start variant (`--variant fail`, stopped 253 ms into each boot), the hang variant, and a normal build whose server was stopped after activation (reset at `ms=600192` each boot). A fixed build then confirmed after 30 s healthy (`PLCNF1 confirmed=1 healthy_ms=30000`), the next cold boot reported `PLTRL1 confirmed=1`, and reinstalling it skipped its trial.
- **Observed** (October 22): the `FLASH_Write_Lock` hang after a watchdog reset, then its fix.
- **Observed** (October 29), the feeder's priority: at priority 3, SAE starved the feeder to within 40–50 ms of the 1 s progress gap, and in one image's layout the same work ran 13–15 % slower and every trial boot crossed the gap. At priority 7 the worst gap was 250 ms, the feed period itself, through a join that met status 126 and ran SAE twice (v368).
- **Observed** (October 24), stops on hardware: a flipped bit, a short range, a stalled server, a restarted server and a power cut mid-download each stopped before activation, or resumed and activated, with bank 1's marker never written early. A power cut mid-download made bank 0 start over, and a cut early in a trial counted that boot again before the build confirmed (`qa-20261024-03`, `-04`). The earlier transport these ran over is retired.
