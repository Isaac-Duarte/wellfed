# Device API

This page explains how the feeder serves its encrypted API, for contributors working on the firmware or a client, and for owners who want to know what happens when they tap Feed. Each operation's answers and refusals are in [operations](device-api-ops.md), along with the parts of the firmware behind them: the settings writer and its record, Wi-Fi join and fallback, the lock's box and the hopper beam's filter. Statistics, traces, the log and the stock backup are in [diagnostics](device-api-diagnostics.md). The types are in [`wellfed-proto`](../../src/product/wellfed-proto/src/lib.rs).

## The one idea

The feeder serves the API itself, from its one loop task, with Noise encryption on every session. There is no cloud and no plaintext mode. The CLI, the app and Home Assistant all speak the same protocol, and every request lands in the same loop as the Dispense button. That loop also steps the [feeding owner](../glossary.md), so a feed from anywhere reaches the one piece of code that moves the motor.

The server inside the firmware is sans-IO: `wellfed_device_api::Server` never touches a socket. The firmware's driver copies received bytes in and sends what a session slot holds, so host tests drive the real server with `wellfed-client` directly.

No build opens an MQTT session; the broker in `wellfed-stock` serves only stock firmware. Requests, update offers and health all ride the device API.

## The protocol

### Sessions

- **Transport.** TCP port 4747, advertised over mDNS as `_wellfed._tcp` with TXT `v=1` and `id=` the identity key in hex, so a client finds the feeder it paired with. Discovery carries no trust; the handshake does.
- **Frames.** A 2-byte little-endian length and its bytes, at most 1 KiB. After the handshake, a frame is one Noise transport message sealing one postcard-encoded message.
- **Opening.** A connection's first frame is a clear `Open`: `Session`, `Invite { id }` or `Recovery`. It only picks the handshake, and tampering with it makes the handshake fail.
- **Sessions are Noise IK.** An enrolled client already knows the feeder's key and sends its own encrypted in the first message, so the feeder learns who is connecting without trying each enrolled key. KK would need up to 8 DH operations per first message, over the loop's 80 ms budget. IK is one round trip, mutually authenticated, with no identity in the clear.
- **Other handshakes.** An invite is redeemed with **XXpsk3** and the invite's secret. The recovery window runs the setup exchange (XX and the code proof) over the stream. Both sides run `wellfed-session::noise`, checked byte for byte against `snow`.
- **Hello and welcome.** IK's first payload is a `Hello { major }`. The second is a `Welcome` or a typed `OpenRefusal`: an unknown key, an unsupported version or no free slot, so the app can say "update the app" or "update the feeder". The welcome names the major, the device's model, its build, the client's role, the event history's epoch and the build's API revision ([below](#evolution)).
- **Model and kind.** The model (`Model`: PLAF103, PLWF106) names the board. Its kind (`Model::kind`, a `DeviceKind`: feeder, fountain, litter box) names the module of ops the device serves beside the shared core, so the client derives the kind rather than reading it twice. There are no feature bits: what a session serves follows from the model and the major.
- **Keys.** A client's identity is its static X25519 key. There are no separate request keys: the session is the security.

### Messages

All messages are serde enums in `wellfed-proto`.

```text
Client → device                       Device → client
Request { id, op }                    Response { id, result }
Subscribe { after, samples }          Events [{ seq, at, body }, …]   // as many as fit a frame, each body behind its length
                                      Gap { from, to }   // events lost to the ring
                                      Sample { at_ms, metric }
                                      Log { seq, dropped, text }   // after TailLog
```

- **Keepalive.** A client that has sent nothing for 45 s sends `Alive`, which any build answers `Done`. A session idle for 90 s is closed.
- **`id`** is a client-generated 128-bit request id. The feeder remembers the last 32 write ids per client key, with their compact outcomes, so a retried write returns the first answer instead of running again. It keeps that memory for four keys at a time, one per session slot. A key's memory outlasts a reconnect until four other keys have opened sessions since it was last used, the least recently used going first, and a memory holding a write still under way goes last. A retry after that runs again. Reads aren't remembered; they run again. An id that has left the ring is no longer recognized, which is safe for everything but feeds ([below](#feeds)).
- **Results** are typed enums of outcomes and refusals, with no free-text error. The ops and their refusals are in [device API operations](device-api-ops.md).
- **`Subscribe`** asks for every event after a cursor (`None`: the oldest kept), then each new one, plus every live sample if `samples` is `On`. Every subscriber gets every event.
- **Events** are durable records ([data](product-architecture.md#data)). `seq` rises across reboots within the ring's epoch. A ring re-created by a storage wipe or the [factory reset](pairing.md#factory-reset) takes a new epoch and counts from 1 again. The event cursor travels in `Subscribe`, and the client sends it only when it counts in the welcome's epoch.
- **A client's write is in the history whether or not its session stays.** The settings writer records the write's event (`Settings`, `Wifi`, `Plans`, `Skipped`, `Unskipped`, `Invited` and `Enrolled`, `Revoked`) in the same turn as the store write, naming the client and the time it asked. The store write goes first, so a power cut between the two loses the event but never records one for a write that didn't land. A settings write is two store writes, the device's record and then the feeder's, with the event after both. If either fails, the client's write is refused `NotWritable` and no event is recorded, even when the device's part ran; the view is then read-only until the records agree again, at once if both writes in fact landed, otherwise at the writer's repair 10 s later or the next boot's. A cut between them leaves the device's part changed, no event, and the client's write unanswered, so uncertain; the next boot repairs the records at a generation no save published ([settings](device-api-ops.md#settings-wi-fi-and-the-lock)). A write that changes nothing, such as settings already as asked or a key already enrolled, records no event. The schedule's own plan changes and BLE setup's Wi-Fi save record none either. Such an event carries the time the client asked, not the time it was saved, and it can land ahead of events the loop queued earlier, so event times in the ring are not always in `seq` order.
- **`Events`** carries the next events after the session's cursor, oldest first, each an `EncodedEvent` in turn: its number, its time, and its body as the ring keeps it, trailing zeros trimmed, behind a one-byte length. As many go as fit `EVENTS_BYTES_MAX` (1,005 bytes), or as the feeder has to hand. A `Gap` always goes alone. The feeder never decodes a body to send it, so a bank 0 sends a newer build's bodies as they are. `wellfed-client` pads each body back to 16 bytes and hands it on as a `Pushed::Event`, or as a `Pushed::Unknown` with its number, time and raw bytes when its build doesn't know the body. The cursor moves past either.
- **Sizes.** Every event body fits the ring's 16 bytes (`every_event_body_fits_sixteen_bytes`). So events and refusals stay one flat enum each, whatever kind they belong to: a feeder's `Fed` already takes 15 bytes, and a kind's tag in front wouldn't fit. The largest messages, the widest statistics section and the fullest `Events` batch are checked against `MESSAGE_MAX` (`the_largest_messages_fit_a_frame`, and the device API's property test through the server).

## In the firmware

### The loop and its sessions

- **Two tasks.** The feeder runs the button service and the loop ([task slots](../../src/firmware/wellfed/README.md#what-the-sdk-runtime-guarantees)). `wellfed-fw-feeder`'s `api` module steps the API inside the loop, after the panel's owner.
- **Nothing blocks.** Every socket call is non-blocking (`wellfed_api_*` in `example_entry.c`), so the loop takes what is ready and moves on.
- **Sockets:** one listener, four session slots, the mDNS socket and the NTP socket. With every slot taken, a fifth connection takes the slot of the open session that has gone longest without sending a frame, whose connection closes (recorded as `evicted`). So a phone that walked off without closing never shuts the next client out until its 90 s idle limit. A session is passed over while it's busy: a request read and not yet answered (a write waiting for the store, a retry waiting for one under way, a Dispense press), a frame not yet read, a feed it asked for while the outputs are held, or an install its last update answer showed under way. So is a connection still opening, a join and the recovery window. With every session busy, the fifth connection is closed at accept, as before.
- **One frame per pass.** A slot reads one frame per pass, and writes again only once its last frame is out, so a flooding client costs only its own turn. That is why events go in batches: a subscriber catching up gets a frame of them a pass, not one event.

### Sessions that stop reading

A connection whose queued bytes haven't moved for 30 s (`STALL_MS`) is dropped and its slot freed, recorded as `stalled`. A client that reads at all keeps its slot, however slowly. So does one with nothing queued, which is the 90 s idle limit's business. A slot already ending gets the same 30 s to send its last bytes, then closes with them unsent.

To the client this is a lost connection, and it loses nothing more. A session sends nothing while a write is pending, so a feed waiting for its reservation, a settings write or a minute's wait for a Dispense press never closes it. An answered write is remembered under its id, so a retry on a new session gets the first answer. A client that can't tell whether its feed ran doesn't resend it.

### Handshakes, and handshakes during a feed

The loop runs one handshake operation per pass, across all slots. A Noise operation costs one or two X25519s, 25–28 ms each on the KM4 ([resource budget](resource-budget.md#handshakes-and-loop-latency)).

The feeder checks a session's key against the enrolled clients the settings writer publishes (`wellfed_device_runtime::keys::publish_clients`). When the table changes, every session whose key is gone closes.

**During a feed, only IK session handshakes go on,** from the loop's claim on the outputs to its release. So an enrolled client can open a session during a feed, read the status and cancel the feed. A client that may want to cancel keeps its session open across the feed, as `wellfed feed` does. Strangers are bounded:

- A key is known only once the first message is read, so any LAN host can start a handshake: one X25519 to build it, two to read that message.
- A session refused during a feed (unknown key, wrong major, full) closes without its reply, saving that reply's two X25519s.
- After one refused or failed session handshake, every other waits for the feed's end. An enrolled client opening then may time out and retry.
- So a stranger costs at most 3 X25519s per connection. Four strangers filling every slot also get their set-up steps: up to 6 X25519s over 5 operations a feed, at most 2 an operation.

**Why that is safe for the feed.** While it drives, the owner's observations must stay within 100 ms of each other (`max_observation_gap_ms`, and the feedback edges' 100 ms age). The loop steps the owner just before each pass. When the pass ran a handshake operation during a feed, it steps the owner again right after (`Server::handshook`), so the gap is the pass itself, not the pass plus the loop's delay. Overrunning costs a failed feed (`ObservationGap`, stopped and durable), never a missed stop: stops stay with the 200 ms motor lease and the supervision timer. `PLOWN1 steps id=… max_gap_ms=… handshakes=…` reports each feed's longest gap on hardware.

**What waits for the feed.** An invite's join and a LAN recovery both end in an enrollment, a store write that would wait for the feed. So the invite is redeemed, and the recovery code taken, only once the key can be written. Such a handshake waits rather than being refused, for at most the claim's `FEED_BOUND_MS`, as a join's wait does. The invite or recovery window can still expire meanwhile.

### Feeds

**A `Feed` goes to the feeding owner** in the same loop (`feeding::Feeding`), under the next journal id, and is answered once the owner has reserved it. With the result, the loop logs `PLOWN1 fed …` and `PLOWN1 current …`, records a `Fed` event ([what a feed records](device-api-ops.md#what-a-feed-records)) and acknowledges it, so the owner takes the next.

**Refusals** are final for that write id, like every refusal. Nothing moved, so trying again is a new feed.

- A feed from a client, a plan or Dispense asks for 1 to 48 portions (`PORTIONS_MAX`, the vendor app's cap on this model). Any other number is refused `Portions { asked }` before anything else is checked, so the refusal says what to change. Stock takes any byte unchecked.
- One feed at a time: a second is refused `Busy`. A faulted owner refuses `Fault`.
- A feed is refused `Busy` while a Wi-Fi join runs (from its scan through association to DHCP) or waits for the outputs. An association holds every task for up to 0.72 s ([resource budget](resource-budget.md#scheduling-gaps-across-wi-fi-rejoins)), so feeds and joins take turns (`wellfed_hal::interlock`).
- The loop logs `PLOWN1 feed portions=… held=…`: 1 a join runs, 2 one waits, 3 a storage write ran past the claim's wait.

**A join during a feed** waits in the Wi-Fi task until the owner is at rest with its outputs stopped, for at most `FEED_BOUND_MS`: the owner's cap on a feed's emitters-on time plus 2 s, about 9 min 40 s for 48 portions. `PLOWN1 joins …` counts waits, and joins forced past that bound.

**A storage write running** when a feed comes makes it wait, for at most `WRITE_WAIT_MS`: 5 s, one write turn's records (a settings change writes two) at an erase and two programs each, every lease at its whole budget, after a waiting writer's slowest poll (`WRITER_POLL_MS`, 500 ms; the settings writer itself polls every 200 ms). A write still running then, or a client's change still queued, refuses the feed `Busy` (`held=3`). The loop tries the claim once a pass without sleeping, so it keeps serving meanwhile:

- The request stays unanswered. `Feeding` still means reserved, and a refusal comes as that request's answer. A retry of the id, on any session, waits for the same feed or hears the same refusal.
- Others see a feed in flight: readiness says `feeding`, and another feed, a restart, an update offer or a change that would queue a write is refused `Busy`.
- Setup mode, an install or the battery hold starting meanwhile ends it, refused, with nothing moved.
- A cancel naming its number is refused `NotFound` and changes nothing: the feed isn't running, and its number hasn't been announced.
- A plan's occurrence due meanwhile waits its turn. A press or a plan refused this way ends as an immediate refusal does: `PLFEED1 dispense refused=…` and its event, or the occurrence given back. So a press can log `dispense feed=N` and then be refused.

**A feed a reset interrupted** (power cut, watchdog, fault) is resolved when the owner next opens:

- The journal records it as uncertain under its own id.
- The loop records its `Fed` event: outcome `Uncertain`, `completed` 0, reason `Interrupted`, and source `Feeder` or the plan that had claimed it. The journal keeps neither requester nor reason, so its record stays the one bank 0 and the flash tools read. If the power cut lost the reservation itself, `requested` is the cap on one feed. The event is dated when the boot resolved it, or not at all if the clock is invalid then (always after a cold boot).
- It is never retried, and its id is never handed out again. A `CancelFeed` for it answers `NotFound`. The owner then takes feeds from the next id.
- Until that event is recorded, a new feed is refused `Busy` (readiness `Resolving`).
- A power cut during the resolution leaves it to resolve at the next boot, once. Only flash corruption of a committed result beside its own reservation (a bit flip that fails the result's checksum) makes the next boot resolve that feed again, so its id can carry a second, uncertain `Fed` event after its first.

**A feed's write id is remembered from the moment it is queued,** not when its answer is sent.

- A client whose session ended before the reservation and resends the id on a new session waits for that same feed and gets its `Feeding { feed }`, during or after it. Nothing is queued twice.
- The window is the boot. Ids live in the server's RAM (32 per client key, 4 keys). The journal keeps the feed's number but not the request id, so after a restart a resent id is a new request. The journal's record keeps room for the id (bytes 48–80: the request id, the sending client's key id and the id of the feed it named), so a later build can make it durable as an additive minor. Bank 0 never reads that room; it only carries the bytes into every record it writes, later feeds' included, so a reader compares that feed id with the record's high-water ID (bytes 32–40, the feed the record is about) and ignores a request id whose feed id differs ([record versions](pairing.md#record-versions)).
- So clients never resend a feed whose answer they didn't get: an unanswered feed is uncertain, and `Connection` reports it `Unanswered`.

**One key, one write.** Each client holds its own key and sends its own write ids, so the same change sent by two clients runs twice. An uncertain write is never resent; it is resolved from the history.

**Readiness** is one decision that says whether a feed can start now (`wellfed_feeder::feeder::feeding::readiness`). `FeederStatus { blocked, supply, … }` carries it as the reason feeding is blocked, `None` when it is ready (the model's `FeederStatus.readiness`), beside `supply`, the judged supply with the pack's charge. A reason a newer build added reads as unknown in an older client.

| Reason | Means | Feed refused as |
| --- | --- | --- |
| `Starting` | The owner hasn't opened, or a trial build hasn't confirmed | `Busy` |
| `Journal` | The journal failed | `Fault` |
| `Fault` | The owner faulted | `Fault` |
| `Resolving` | An interrupted feed is being recorded | `Busy` |
| `Feeding` | A feed is in flight | `Busy` |
| `Joining` | A Wi-Fi join runs or waits | `Busy` |
| `Battery` | On the pack alone below its floor, or after a brownout until the adapter returns or the pack has read charged for 3 minutes ([power](power.md)) | `Busy` |
| `Setup` | BLE setup mode, which ends in a restart that would leave a feed uncertain | `Busy` |
| `Updating` | An install | `Busy` |
| `Unserved` | No owner at all (the simulator, the `gap-probe` image) | `Unavailable` |
| `Restarting` | A restart was asked for (a client's reboot, setup mode's end, the [factory reset](pairing.md#factory-reset)) and comes within seconds | `Busy` |

- It starts from the owner's own reason (`Feeding::blocked`), then adds `Restarting`, `Setup` and `Updating`.
- A `Feed` goes to the owner only while `Ready`. The owner's own checks stand behind that, for a feed that lost a race.
- A change of a lasting reason (`Journal`, `Fault`, `Updating`, `Setup`) is recorded as a `FeedingBlocked { reason }` event, and `FeedingBlocked { reason: None }` once it clears. Passing reasons only go to the log (`PLFEED1 readiness …`), so they never fill the history. Each boot starts with none recorded.
- A running feed whose pack falls under the floor ends `Failed` with reason `Power`.
- The panel lock is not a reason: it holds only the Dispense button.

**The Dispense button** asks for one portion, as stock's does, through the same readiness and owner as a client's `Feed`, and its `Fed` event says `by: Panel`. A locked panel ignores it, and a pending pairing confirmation takes the press first. The loop logs `PLFEED1 dispense feed=N` or `dispense refused=…`. A press while a feed can't start (a running feed included) starts nothing and isn't queued, as stock's (its record code 2022). It becomes a `DispenseRefused { reason }` event only when its reason differs from the last recorded since the feeder was last ready, so presses during a long feed can't fill the event queue.

**Feeds are silent.** A feed asks nothing of the speaker. The loop steps the speaker for a client's `play` outside setup mode, after the owner and the device API, and refuses one while a feed is in flight. No feed decision reads the speaker, so a tune that plays, won't open or stalls leaves a feed's start, cancel and `Fed` event alone. A tune isn't cut when a feed follows it, such as a press, a client's feed, or a meal with no call or whose call has ended. Its steps (the codec's close and `done` line included) run between the owner's steps through the drive, so they count against `max_observation_gap_ms`, as meal calls do. An owner-chosen call before a scheduled meal belongs to the schedule feature ([schedules](schedules.md)).

**No restart into a feed.**

So no restart leaves a feed uncertain or a claimed meal missed:

- `Reboot`, an install offer and a memory read are refused `Busy` while a feed is queued or runs, or while a plan's claim is being written (its meal is the next feed).
- Once a reboot is accepted, feeds are refused (readiness `Restarting`). The restart still waits for a feed in flight at its deadline (a Dispense press in that second).
- BLE setup's closing restart, the factory reset's and one a stalled reset asks for wait for a feed in flight, and for a claim being written for at most 30 s (`CLAIM_WAIT_MS`). A stored claim the journal can't yet be compared with holds none of them.

### No flash write while the motor may move

Every flash lease checks that the outputs are stopped, and fails closed when they aren't: the lease hook halts and the watchdog resets the board mid-feed. So every store write outside the feed journal first takes the write turn in `wellfed_hal::interlock`, granted only while the loop holds no claim on the outputs. That covers the event ring, settings, the client and key stores, Wi-Fi, the seed, the build confirmation, and the update stage's records and bank-1 writes.

- **A write starts when it is queued.** Queuing notifies the service task, so a writer at rest skips its 200 ms poll, and one that just wrote keeps its delay. During an upload it polls every 5 ms for about 1 s after each chunk. A lost or spurious notification leaves the poll to take the change. Static analysis and host tests; not yet measured on hardware.
- **A write that comes during a feed waits.** The settings writer leaves it queued, asks again every 200 ms and still answers event reads. It counts itself among the waiting writers, so the next claim lets it go first. It runs once the owner is at rest with its outputs stopped.
- **The forced path.** Only a claim that outlives `FEED_BOUND_MS` lets a write go ahead with the outputs claimed, and only while they read stopped. The bound is timed from when the writer first saw that same claim, so a writer that misses the gap between two feeds starts again on the second. Forced writes are counted (`writes_forced`). A write never starts into moving outputs.
- **The update stage never forces.** An update request waits in its mailbox, unanswered, until the outputs are free. An install holds the turn from its first write until it stops, so every feed meanwhile is refused (`held=3`). A give-way or an activation keeps the turn until the restart.
- **Marker order.** The confirmation takes the turn before arming the hooks, and the update stage before setting the attempt marker, so the marker is never left set through a wait.
- **Order is kept.** Queued writes complete after the feed in their usual order: events as queued, and each mailbox's single change once.
- **The journal takes no turn.** The owner reserves and finishes a feed only after `quiet()` has stopped the outputs, and opens (creating the journal or resolving an interrupted feed) before any feed is claimed.
- **Trial boots.** No feed is claimed before the confirmation is written. The owner opens only once the hooks are armed, and the confirmation is the boot's first arming (the settings writer starts after it), holding the write turn from arming to its write. So a feed can't push the confirmation towards the trial deadline.
- **Counters.** `PLOWN1 interlock …` counts `writes_deferred`, `writes_forced`, `write_waited_max_ms` and `feeds_delayed` (claims that waited for a write).

**A client's store write during a feed is refused `Busy`.** Waiting for the feed's end would outlast the client's 10 s wait for an answer, and a slot waiting on an answer reads no frames, so the client would end the session unsure of the write.

- While the outputs are claimed, the loop refuses a settings change, a Wi-Fi change and a revocation `Busy` before queuing anything. Nothing changed, and the client may send it again after the feed. Reads still answer.
- An enrollment asks again each pass until the feed ends, as it does when the keys mailbox is taken.
- A change queued before the claim goes first. The claim waits, within `WRITE_WAIT_MS`, while a settings, Wi-Fi or keys change waits in its mailbox, so it is written and answered before the feed starts. A change still in its mailbox after that wait (a stalled writer) refuses the feed `Busy` (`held=3`) instead, so a change is never written after a feed it was queued before.
- BLE setup's Wi-Fi save, which waits for its own answer, waits out a feed.

### Writes, roles and generations

**Writes are remembered per client key**, as [above](#messages). A settings, Wi-Fi, time, plans or calibration write names the generation it was based on, and is refused `Stale { current }` when the store has moved. The loop is the only requester, so the check and the queuing can't interleave with another writer.

**A write the device finishes later is remembered from the moment it is queued,** as a feed is. A settings, Wi-Fi, time, plans, calibration, skip, meal call or revocation write, or an update offer, cancel or key statement, whose session ended before its answer still runs. A retry of its id, on any session, waits for that write and hears its answer, never `Stale` against the generation the write itself moved nor `Busy` behind its own change. Once the write's session has gone, the server settles the write on its next pass, so a retry hears what the write did, not what the store holds when the retry comes. Within the boot, as for feeds: after a restart a resent id is a new request.

**Roles.**

- An owner does everything, and may invite any role but an update signer.
- A member or the server may feed, change settings, the plans and the lock, play, sync the clock, read the time view and the statistics but `Memory`, and list clients. Both may skip a meal, play the meal call and read it. Neither may change Wi-Fi or the time settings, revoke, reboot, update, tail the log, read bank 1 or upload or delete the meal call. The server's key may not set the portion calibration.
- The device classes every op by who may send it (`wellfed_proto::access`), matched with no catch-all, so an op added to the API doesn't build until it says who may send it. A role its class doesn't name is refused `Forbidden` before the op is checked further, so a malformed op is `Forbidden` to that role and `Invalid` to one that may send it. An invite that passes is then checked by pairing for the role it grants (`wellfed_proto::access::may_invite`), and refused `NotAllowed` if its asker may not hand that role out. Clients gate on both checks in that order: `wellfed_model::Op::allowed` in Rust, and in the app a table generated from it (`lib/src/roles.g.dart`), whose invite rows are for inviting a member.
- A member may invite members. The server may invite no one (`pairing::Pairing::invite`).
- An update-signer key never opens a session.
- An owner's clock sync may step the clock; a member's or the server's may only narrow it ([clock](clock.md#the-clock)).

### Events and samples

**The queue.** The loop's event recorder (`wellfed_feeder::feeder::events`) queues records in `wellfed_device_runtime::events`. The queue holds 16 in RAM, the last kept for a feed's `Fed` event, since appends wait out a feed and events pile up meanwhile. Overflow is counted and recorded as a `Lost { count }` event.

**The ring.** The settings writer appends records to the ring in storage sectors 13–24 (`wellfed_device::store::events`: 32-byte records, at least 1,408 kept) and answers the API's reads. It polls every 5 ms while either waits, and for about 1 s after its last read. While a feed holds the outputs, an append waits at the head of the queue.

**Epochs.** A ring created empty takes a new epoch, and every record repeats it in bytes 4–7, the high half of the 64-bit sequence field. The seed record, written every boot and kept through a factory reset, carries the epoch the boot's ring would take: each boot moves it past the last by a random stride, so no ring takes an epoch an earlier one had, generator or not, and a client never meets old numbers under a known epoch. Only a flash restore, which puts an older seed record back, can bring an epoch back. The welcome carries the epoch, and the feeder welcomes no session until the writer has opened the ring and published it.

**Reads.**

- A read answers up to 32 records from one pass over their unit, about a frame of events. It starts at the cursor's slot in its unit, since a unit's records rise one number a slot, and stops once the batch is full, so it reads about as many slots as it returns.
- A subscriber catching up takes the whole batch in one pass and sends it as one `Events` message. Other subscribers whose cursor falls inside the batch read from it too.
- A batch stops before an event that might not fit (`EVENT_BYTES_MAX`, 32 bytes, the largest any event encodes to), at the newest event, when the next read hasn't come back, and before a gap. The session's cursor moves past each event the batch takes. A gap is left for the next pass, which sends it alone.
- Subscribers take turns by session slot. A batch is fetched for one subscriber and stays until it has taken from it. One the batch can't answer waits, and the next read goes to the next waiting slot, round robin. So a subscriber catching up goes without a record for at most one pass more than there are subscribers, in whatever order the loop visits them (simulation, up to four subscribers).
- A subscriber whose session ends gives up its turn and batch at once. One that stops asking without ending (its last frame still going out) keeps its batch for at most 50 asks from the others: about a second with one other subscriber waiting, and about 17 with three.
- Why batches: one record a read cost a writer poll each, 200 ms whenever the reader missed the 5 ms window, about 3 events a second (physical, October 29). One event a frame still cost a loop pass each: 1,140 events took 16.5 s (physical, v359). A frame of events per pass removes that (static analysis, not yet measured).

**What is recorded:** boot, lock changes (panel, idle relock, or which client), settings, plans and Wi-Fi writes with their client, clock syncs, invites redeemed, recovery and revocations, feeds, refused Dispense presses, meal reminders, missed meals, skipped meals and their undos with their client, food-level changes, supply changes and blocked feeding. Four more are there for the feeder's alert rules ([alerts](alerts.md)): `UpdateStop { stop }`, an install that stopped before activation, a cancel among them, since the update's state forgets it at the next offer or restart; `UpdateAtBoot { candidate }`, what the update store held as a boot came up, once a boot; `Restarting`, a restart someone asked for on its way; and `UpdateConfirmed`, the build a boot activated confirming itself with no restart. A key that joins through an invite also records `Invited { client, by }` just before its `Enrolled`: the first 7 bytes of its key and of the key that asked for the invite, too many to grind a matching key for, so a client can tell which key invited which. The bodies are `EventBody` in `wellfed-proto`.

**The feeding history** is the ring's `Fed` events; the feeder keeps no other log of feeds. Stock keeps none on the device: its two 50-record rings only queue records until the cloud acknowledges them.

- Each `Fed` is dated when the feed ended and survives reboots with the ring ([what it records](device-api-ops.md#what-a-feed-records)).
- **Coverage.** The ring keeps at least 1,408 records: 11 of its 12 sectors at 128 records each, the twelfth being the next to erase. Every event shares them, so it keeps at least the last 50 feeds unless the feeder records more than about 1,350 other events between them.
- **How many days.** A normal day of 2 to 4 meals records about 8 to 12 events, so the ring holds about 4 to 6 months (estimated from static analysis; not yet measured on a feeder). The count assumes each meal records its `Fed` and a `MealSoon` (with the meal reminder on), and that each day adds one food-level change, one panel unlock with its idle relock and one change from the app. Clock syncs are few: the feeder records its first fix after a boot and a jump, not each step, and the app syncs only a clock that has none or a feeder it just paired. At 20 events a day the ring still holds about 70 days, and at 200 a day, the design bound in the [resource budget](resource-budget.md#event-history), a week.
- **Gaps.** A reader resuming from a cursor (`--after`, or a client following the history) whose events the ring has since dropped gets a `Gap` for them, never a silent hole. A plain `wellfed feeds` starts at the oldest event kept, so its first line is simply the oldest feed left.
- **Time.** `at` is the feeder's clock when the event was recorded, and none while the clock is invalid (a cold boot until a source syncs it).
- **Not recorded:** a feed whose `Fed` event was still in the RAM queue when the power went. The window runs from the feed's end until the writer appends it: its 5 ms poll and one record's program when no other write waits (static analysis, not measured). The journal holds the result, but no event is written for it at the next boot.
- **An uncertain feed reads as a range**, never a count: from the portions confirmed up to the portions asked (`Delivered::Uncertain`), since anything in that range may have dispensed.
- **On the host.** `wellfed feeds` prints the history oldest first, a line a feed in the computer's time zone with its offset (`--json` for JSON, `--after` to resume after an event number of the current epoch). `wellfed_client::history` makes each entry the CLI and the app show. `wellfed events` prints every event the same way (`--json` for each message as the wire spells it), and `wellfed status` the status, readiness, food level, last failed feed, supply and desiccant included.

**Samples** go only to subscribers that asked for them, each on its own cadence ([samples](device-api-ops.md#samples)). A session holds one sample of each kind and sends the oldest first. A sample not yet sent when the next of its kind comes is replaced by it.

### mDNS

`wellfed_core::mdns` answers `_wellfed._tcp` on one UDP socket in the loop: PTR, SRV (port 4747), TXT (`v=1`, `id=` the identity key) and A, for the host `wellfed-<first 4 key bytes>.local`.

- It waits for a DHCP address before opening its socket.
- It announces twice on every address change, and answers legacy unicast queries.
- The vendor responder (33.5 KB and its own task) stays unlinked, and `wifi_register_multicast_address` isn't needed.

> [!NOTE]
> A host firewall that drops UDP 5353 hides the feeder from discovery. Pass `--address` instead, or allow UDP 5353 on the interface the feeder uses.

## Evolution

The device API changes in two ways. The update path, invite redemption and recovery are frozen for good. Everything else only grows, and an ordered API revision tells each side which of them is the older build.

### The frozen part

Every update passes through bank 0, which keeps the build it was written with ([one update, step by step](ota-design.md#one-update-step-by-step)). So a client of any later build must still open a session with a bank 0 of any earlier build, offer, push chunks and read the answers. If those messages changed, a deployed feeder could never update again without a UART cable. For the same reason a phone must still join a feeder of any earlier build by invite, and an owner who lost every key must still recover it. These are frozen as major 1:

- the TCP port, the mDNS service and its TXT fields (`v=1`, `id=`);
- the frames: a 2-byte little-endian length, at most 1 KiB;
- `Open` with every variant;
- `Open::Session`, the Noise IK handshake under the prologue `wellfed-api-v1`, `Hello`, and `Welcome` with everything it carries (`Model`, `Role`, `OpenRefusal`, the API revision in its `schema` field);
- `Open::Invite`, the Noise XXpsk3 handshake under the same prologue with the invite's secret as the PSK, the client's `Join` and the device's `Joined`;
- `Open::Recovery` and the setup exchange it runs, which BLE setup runs too: the Noise XX handshake under the prologue `wellfed-setup-v2`, the setup `Request`, `Answer` and `Reply` with their refusals, the code proof, and BLE's service, characteristic and 448-byte messages;
- `ClientMessage::Request { id, op }` and `DeviceMessage::Response { id, result }`;
- the ops `Op::Update` (offer, chunk, state, cancel, enrolling an update key, a key statement), `Op::Alive` and `Op::CreateInvite`;
- their answers: `Outcome::Done`, `Outcome::Update` with `UpdateView`, `Outcome::Invite` with `Invite`, and `Outcome::Refused` with the refusals they get (`Forbidden`, `Busy`, `Unavailable`, `Invalid`, `NotWritable`, `Full`, `NotFound`, and every `UpdateRefusal`);
- the signed formats an offer and a key statement carry, which bank 0 checks: the version 2 manifest (`PLOTA2`, 160 bytes, every field at its offset), the downgrade (`PLDWN1`), key-add (`PLKEY1`) and revoke (`PLREV1`) statements, Ed25519 signatures checked strictly, and key ids (the first 8 bytes of SHA-256 over the public key);
- the limits an offer's image is checked against: bank 1's offset and size (`BANK1_BYTES`), the 4 KiB sector, and the loader's marker and five-part layout that give an image its extent.

The rules for the frozen part:

1. **Nothing changes in place.** No field is added, removed, reordered or retyped. Postcard is positional and encodes a variant by its index, so any of these would make an old peer misread every byte after it.
2. **Enums only grow at the end.** An old device never sends a new variant, and a new client decodes every old one.
3. **Frozen variants come first.** In `ClientMessage`, `DeviceMessage`, `Op`, `Outcome`, `Refusal` and `Open`, the frozen variants hold the first indices. The rest can then change behind them without moving a frozen one.
4. **Only a new major changes it, and major 1 stays.** `Hello.major` negotiates it, and a device that doesn't serve the client's major answers `OpenRefusal::Version { supported }`. A device keeps serving major 1's frozen part for good. So does every client, even once a new major exists: a feeder never updated since speaks only major 1, and its update and invites must still go through.
5. **A request this build can't decode is refused, not dropped.** The request's id always sits right after its tag, so a device answers an op it doesn't know `Refused(Unavailable)` and keeps the session. The client can then tell the owner the feeder needs updating first. Bytes with no readable id still end the session as `malformed`.
6. **The device's frozen answers are typed.** The `Device` hooks behind the update and key ops return `FrozenRefusal`, a mirror of `Refusal`'s frozen prefix. So the compiler, not a test, keeps them from answering outside the frozen set.

`tests/fixtures/update-v1.txt` pins the bytes of every frozen message, among them a whole IK session, an invite's XXpsk3 join, and a recovery from its XX opening through every round of the code proof to `Reply::Done`, each from fixed keys. Its test refuses any change to a line already there; blessing only appends lines for new variants. `the_advertised_service_and_txt_fields_never_change` (`wellfed-core/tests/mdns.rs`) pins discovery, and `the_setup_transport_never_changes` (`wellfed-core`'s `ble_setup`) pins BLE. `wellfed-signed/tests/fixtures/signed-v1.txt` pins the signed formats the same way, in a crate that holds nothing else: a manifest of each kind and one statement of each kind from a fixed key, each admitted as bank 0 admits it, with the limits and a loader layout. `every_answer_to_a_frozen_op_is_frozen` (`wellfed-device-api/tests/frozen.rs`) checks the device never answers a frozen op outside the frozen set. `an_update_goes_through_a_bank_0_of_another_build` (`wellfed-client/tests/frozen.rs`) runs a whole update against a device that decodes only the frozen types.

### Everything else

From the public beta, feeders, phones, members' phones and Home Assistant run whatever build each last took, so the rest of the API only grows. Before the beta nothing is released, and the API was reshaped freely; the stored trace starts at revision 1.

1. **Nothing changes in place.** No field is added, removed, reordered or retyped, and no variant moves or changes. A struct that will grow carries room for it at its end.
2. **Enums grow only at their end.**
3. **A changed op is a new op.** A retired op keeps its index, and the device refuses it `Unavailable`.

A client of an older build still reads what a newer feeder says, through these pieces of `wellfed-proto` ([`grow`](../../src/product/wellfed-proto/src/grow.rs)):

- **Unit enums inside answers are lenient.** A food level, a feed's outcome and reason, what blocks feeding, the Wi-Fi source, a reset cause, a clock source, an NTP result, a current unit, the bowl mode and the rotor's phase travel as `Lenient<T>`, and so does every unit enum inside an alert or an event body (a feed's outcome, reason and hopper reading, what blocks feeding, a client's role and how it joined, why an install stopped and what a boot found of one), so a failed meal with a reason a newer build added still reads as a failed meal. An index the client's build doesn't know reads as unknown, and the rest of the answer reads on. The model names it `Unknown`, or `Phase::Newer` beside the phase that is itself unknown; Home Assistant names it `newer_build`. A bowl mode the client can't name, written back in `PutCalibration`, is refused `Unverified`, as any mode but the build's own. A feed's unknown outcome counts as uncertain, never as done. `every_lenient_enum_is_unit_only_in_its_encoded_order` checks that each one is unit-only, so it can only grow by a variant at its end.
- **Structs that will grow end in a tail.** `Status`, `FeederStatus`, `Settings` and `Plan` end in `ext`, a length-prefixed run of bytes. This build writes it empty. A later build puts its new fields there as a struct of their own, and an older reader skips them by their length.
- **An answer the client can't read is not the end of the session.** The response's id sits right after its tag, so `wellfed-client` answers an undecodable response `Unreadable`, or `Refused(Unknown)` when only the refusal is new, and keeps the session. The two mean different things. A refusal comes before the device acts, so `Refused(Unknown)` means the request did not run; the client shows it as a refusal it can't name. `Unreadable` may follow a write that ran, so that write is uncertain: the client reports it as unanswered and never sends it again, as for a write whose session dropped ([feeds](#feeds)).
- **A new kind of message is skipped.** Each device message is a sealed frame of its own, so a client skips one whose tag is at or past the `DeviceMessage` variants its build knows (`DEVICE_MESSAGES`) and reads the next, with the session kept (`a_newer_kind_of_message_is_skipped_and_the_session_kept`).
- **A sample it can't read is skipped.** A live sample of a metric a newer build added, or with a value inside one it can't read (a new battery state, say), is skipped whole by its tag, with the session kept. Samples are never stored and the next of a kind replaces the last, so nothing is lost, while ending the session would leave every write in flight uncertain (`a_newer_metric_is_skipped_and_the_session_kept`).
- **Events carry their length.** The device sends each event body behind its 1-byte length, so an older client reads a body it doesn't know as `Pushed::Unknown { seq, at, raw }`, and its cursor moves past it.
- **Alert entries carry their length.** `AlertsView` sends each open alert behind its own length, so an older client skips an alert kind it doesn't know and counts it (`unread`) rather than failing the table ([alerts](alerts.md#reading-the-table)). A newer build that holds more alerts than the client's `ALERTS_MAX` has its extra entries counted the same way.
- **A trace's later entries carry their length.** A feed trace's body has its own byte format ([`trace`](../../src/product/wellfed-proto/src/trace.rs)), and traces outlive the build that stored them. Its four spare entry kinds are a later build's, each a time, a length and that many bytes, so an older reader skips an entry it doesn't know and reads the rest. `every_kind_keeps_its_bytes` pins how each entry is coded.
- **What an older reader can't skip never grows.** An enum an answer, a live sample or a subscription reaches plainly, not through `Lenient` or a length, would fail the whole message in an older reader: a new battery state would make a `Status` unreadable, and a new `Samples` value would end an older feeder's session. So `the_api_only_grows` refuses a new variant in any of them (`Metric`, `Battery`, `Window` and the rest), and such a value grows by a new op, a new kind of message, a field in a struct's tail or a lenient field instead. `ClientMessage` takes no new kind either, since an older feeder ends the session of a client message it can't read as malformed: a client's new request comes as an op. `DeviceStatus` stays closed too: a new kind of device, such as the fountain, reports its own status through its own op, never as a new `DeviceStatus` variant. The enums that do grow are those an older reader survives: `DeviceMessage` (a new kind is skipped), `Outcome`, `FeederOutcome` and `Stats` (a new variant answers only a new op or section), `Refusal` (read as `Refused(Unknown)`), the frozen update enums by the frozen rules, and every enum only a request's op reaches, since a feeder refuses an op it can't decode `Unavailable`.

**Whole-replace writes.** `PutPlans` replaces all ten slots, and `PutCalibration` the calibration. A write that replaced what a newer build stores beside them would drop fields the older client never saw. So neither op ever gains a field: a later field arrives in a plan's tail or through a new op. A device keeps a slot's stored extension when a plan's tail comes empty, and refuses `Invalid` a plan whose tail it can't read, rather than store less than was asked (`a_plan_with_a_tail_this_build_cannot_read_is_refused_invalid`). `PutCalibration` sets only the fields it carries, so a calibration field a later op adds survives an older client's write.

The first build that adds a plan field must keep this rule, which no build exercises yet: an older client's `PutPlans` carries an empty tail for every slot, including slots whose stored plan it changed. A stored extension is kept only when the slot's other fields come back unchanged. When they change, it is dropped: an extension set for one meal does not carry over to the different meal an older client put in its place.

`tests/fixtures/api.json` holds the traced structure of every message at the newest revision: `tests/api.rs` traces every type of the device API and the setup exchange with `serde-reflection`, down to each enum's variants and each field's name and type, and `the_api_only_grows` refuses a trace that does not hold every stored type unchanged, a new variant at an enum's end being the only change it takes. A trace that grew needs `REVISION` raised by one and a blessing (`WELLFED_BLESS=1`), which stores it; no blessing takes a change in place. `tests/fixtures/v1.txt` pins the bytes of each message beside it, and its test only appends.

**The API revision.** `REVISION` in `wellfed-proto` counts the API's growths from 1, and the welcome carries the build's in its frozen `schema` field. Since every revision's ops mean the same to every build that knows them, a client sends any op it knows to a feeder of any revision:

- A feeder behind the client refuses an op its build lacks `Unavailable` and keeps the session. `wellfed-client`'s `Opened::skew` says `FeederBehind`, so a client can say "update the feeder".
- A feeder ahead of the client answers every op the client knows. `Opened::skew` says `FeederAhead`, so a client can say "update this app". Its events that the client's build doesn't know arrive as `Pushed::Unknown`.
- **A pre-revision build** sent a 32-bit hash of its schema in the same field, which no revision reaches. A value of `0x1_0000` or more (`LEGACY_SCHEMA_FROM`) is such a build, `Skew::Legacy`: the bench unit's bank 0 and every feeder updated before revisions. Every message outside the frozen part moved since. So the session refuses it any op outside the frozen part, and any subscription, with `Error::Legacy` before a byte is sent. The update path, the keepalive and invites still go, and the feeder is the side to update (`a_pre_revision_build_is_sent_the_frozen_part_alone` and `an_update_goes_through_a_pre_revision_bank_0`, in `wellfed-client/tests/frozen.rs`).

Every client serves a feeder of any revision the same way, and `Skew` (`wellfed-model`) only says which side to update. A pre-revision build is the exception, served the frozen part alone:

- **The app** shows "Update this app" for a feeder ahead of it, and "Update the feeder" for one behind it (to a member, that its owner can update it). It never offers its image to a feeder ahead of it, or to one whose confirmed version is at or past the image's: the app sends no downgrade statement, so an install there could only be a downgrade. Its session with a pre-revision build reads no status, history or clock, and carries only the update's own reads and cancel. So the Firmware screen offers that build the update, with no status card, and the app says to update the feeder.
- **Home Assistant** serves it and raises a warning repair, `update_feeder` or `update_integration`, that clears once the revisions match. A pre-revision build is sent nothing: the `legacy_feeder` repair asks for it to be updated, and Home Assistant retries until it is.
- **The update key's enrollment and the installer** read the client list and status as from any feeder; a status they can't read leaves readiness unknown. From a pre-revision build they read neither: the enrollment goes straight to the press, and readiness stays unknown.
- **The bench server** opens a pre-revision build's session with no subscription, status read or clock sync, says `skew` `legacy` in its session info, and lends the session to the CLI for the frozen ops alone. Any other request is refused unsent (`a_pre_revision_feeder_s_session_serves_the_update_path_alone`, in `wellfed-bench`). So `wellfed update-key enroll` runs through it.
- **A return to stock** needs the stock backup, and a pre-revision build can't be asked for its bank 1 (`ReadBank1` is outside the frozen part). So such a feeder returns to stock only from a backup kept earlier, by the app or the CLI, until it is updated.

These rules hold for both parts:

1. **What a session serves follows from the welcome alone.** There are no feature bits. The model names the board, its images and its kind; the kind names the module of ops beside the shared core; the major and the API revision name the messages. The server is generic over the shared `Device`, and reaches a kind's module only through it (`Device::feeder`). A device of another kind refuses the module's ops `Unavailable` after the same retry and role checks as any write, as does a build that doesn't serve an op. A client doesn't send another kind's op at all (`Op::kind`, checked in `wellfed-connect`). Every subscriber gets every event. Enforced by `every_subscriber_receives_every_body` (`wellfed-client/tests/api/revisions_and_leniency.rs`), whose exhaustive match keeps the list of bodies whole, and `a_device_without_a_feeder_module_refuses_its_ops_and_remembers_the_refusal` (`wellfed-device-api/tests/kinds.rs`).
2. **The event history carries an epoch.** `Welcome::Open.epoch` is new for each ring created and kept with it, and event numbers count within it. So a cursor from a re-created ring is recognized, instead of freezing a mirror that waits for numbers it already has. `wellfed-client`'s `Connection::subscribe` resumes after a cursor only in the feeder's current epoch, and otherwise starts from the oldest event kept (`Resume::Restarted`). Home Assistant holds its cursor with the epoch, and a cursor from another epoch is a gap, so it drops what it held of the old history and reads the new one from its start. The app's alert catch-up reads the alert table under the epoch it was read in. Enforced by `a_cursor_resumes_in_its_epoch_and_restarts_from_another` and the event ring's epoch tests.
3. **The prologue never changes.** `wellfed-api-v1` is bound into every handshake. A changed prologue would fail the handshake with no reason, where a new major gets the typed refusal. Enforced by `the_prologue_never_changes` (`wellfed-proto/tests/fixtures.rs`), the frozen fixtures, and `a_major_2_hello_gets_the_typed_version_refusal` (`wellfed-client/tests/api/revisions_and_leniency.rs`).

## Without MQTT

The firmware's MQTT transport is gone, with its request parser, reply writers, request signing and health nonce. The normal role starts no network task of its own.

- The request and reply types the API fills live in `wellfed_device_api::mailbox`. Those the SDK's own services carry are in `wellfed_core::settings` and `wellfed_core::tone`.
- The builder reads no broker credentials or request key (`private/fw-mqtt.json`), and the startup audit requires no transport policy or `PLMQT1` start record.
- A trial build confirms on a live link: associated, leased, and its gateway answering, or never having answered this join. No internet is involved ([link liveness](startup-diagnostics.md#link-liveness-without-internet-dns)).
- A stock-to-replacement install closes on the bench's device-API check of the new build on bank 0 (`POST /api/ota/trial/answered`).

## Resources

From the `feeder` builds' manifest audits and `PLAPI1` records:

| | Before the device API | With the device API | Latest build |
| --- | ---: | ---: | ---: |
| Payload | 868,352 | 942,080 | 1,015,808 |
| Loop stack bound | 3,912 of 6,144 | 4,552 of 8,192 | 5,412 of 8,192 |
| Heap free, steady | 146,464 | 125,088 | — |

- **Stack.** The loop's stack is 8 KiB. At 6 KiB the bound sat 56 bytes under the audit's 25 % margin. The recovery responder's 1.2 KB frame runs out of line (`rescue_op`), so it never nests under the handshake step's 1.1 KB. The [resource budget](resource-budget.md#tasks) names the deepest path in the latest build.
- **Heap.** Flat at 125,088 free over minutes, and through a garbage flood. The difference from the first product budget isn't yet attributed.
- **Other tasks' bounds:** `wellfed` 2,808 of 4,096, `wellfed-key` 1,248 of 2,048, `wellfed-svc` 11,744 of 16,384, `wellfed-wdg` 1,420 of 2,048.
- **Records:** `PLAPI1 session … state=open` ends `handshake_ms=`.

## Evidence

- **Static analysis** (November 13, 2026), the history's depth: the ring's size from `wellfed_device::store::events` (12 sectors of 4 KiB, 32-byte records), and which events a normal day writes from where the feeder records them. A full ring of 1,408 records was read on the feeder, but how many days one holds has not been measured.
- **Modeled** (November 3, 2026), subscribers at once. The simulator reads its event ring through the firmware's own turns (`wellfed_device::store::reads`, which the runtime's shared state holds too), answering one read after each loop pass as the settings writer does.
  - Three sessions (the owner, a member and a second owner session) subscribed together and each got every event of one feed, the same `Fed` among them (`wellfed-feeder-sim/tests/subscribers.rs`).
  - Two sessions at the newest event and a third replaying 160 lock changes and a feed (more than four batches), with new events arriving throughout: the replaying session got the whole ring in order with no gap, and all three the same feed.
  - A session that left after its first replayed event, its batch and turn still held, stalled neither a session replaying the ring nor one following it.
  - These show delivery, not the interleaving the turns exist for: they also pass on the read logic from before the turns, since the simulator answers a read only between whole passes. The runtime's host tests (`wellfed-feeder-runtime/tests/writer/events.rs`) cover the interleaving. Not yet run on the feeder.
- **Observed** (October 24, 2026; no owner action; each install used a bounded `serve`):
  - `fw47-api-02` (v114): `PLAPI1 listen socket=api open=1`. `wellfed status` opened an IK session as Owner. Settings writes moved the generation 3031 → 3032 → 3033. Lock, unlock and a clock sync were recorded, each with the client's fingerprint, and `events` read all six records.
  - **Invite and revoke:** an owner minted a member invite, and `wellfed join` redeemed it (XXpsk3) with a scratch key, which then opened a session as Member. With that member's `events --follow` session open, `revoke` closed the session at once, and the key was refused afterwards (`OpenRefusal::Unknown`). The history showed `Enrolled { path: Invite }`.
  - **Samples:** a subscriber received power samples every 5 s.
  - **Client limit:** with four sessions held open, a fifth connection was closed at accept.
  - **Flood** (`fw47-flood-01`, about 35 s): 112 connections of 4,000 random bytes each were closed as `malformed` one by one. An owner's `status` kept answering in about 0.1 s. There was no supervisor stop, and the heap was unchanged. The Wi-Fi driver logged `Cannot allocate pbuf` four times during the burst.
  - **History across a cold boot** (`fw47-mdns-01`): `PLCFG1 events open=1 last=10 oldest=1`.
  - **mDNS:** the first socket opened on the SDK's static pre-DHCP address (192.168.1.80) and never saw a multicast query, so the responder now waits for DHCP. `wifi_register_multicast_address` returned -95. The feeder then answered every query (`mdns_in=15 mdns_out=13 mdns_failed=0`), and a legacy unicast query got a correct A record. Discovery from the bench host failed until the owner allowed UDP 5353 on the hotspot interface; then `wellfed status` found the feeder without `--address`.
  - **The bench on the API** (`itest`, the owner key, the address from the neighbour table): `fw47-itest-01` (smoke tier, v119) passed `ota-update`, `requests` (the clock 3 ms from the host's after a sync) and `request-auth`. `fw47-itest-02` (standard tier, v120) passed `ota-update`, `requests`, `request-auth`, `settings-persist` and `wifi-fallback`; `network-bounce` failed because the outage showed as a DNS loss `Status` did not report, so `dns_losses` was added. `fw47-itest-03` (v121) then passed `ota-update` and `network-bounce`. `qa-20261024-01` passed all six; `qa-20261024-03` and `-04` passed `ota-recovery` and `ota-power-cuts`.
  - `Status.boot_count` once reported KM0's whole register 4, trial bits included (`2533425155` = `0x97010003`). It now reports bits 0–15.
- **Observed** (October 24), recovery with the owner at the panel:
  - `fw47-recover-05`, passed: the owner unlocked the panel and held Lock and Dispense (6.3 s): `pair recovery state=opened`. `wellfed recover` connected, the window closed at its offer (`pair confirming path=recovery`), and the owner keyed in the six presses: `pair recovery state=entered`, `PLCFG1 keys change=enroll result=done enrolled=2`, and the session ended `joined`. The new key opened an Owner session and listed both clients; the bench key then revoked it.
  - `fw47-recover-04`, refused as it should be: only five presses registered (one of two Dispense presses in a row never reached the button service, `dropped=0`), and 60 s after the offer the feeder refused it as `unconfirmed`, enrolling nobody.
  - A 4.9 s hold, under the 5 s threshold, opened no window, and the connection was turned away (`ended reason=recovery`), as for a closed window.
  - Fixed after these runs: presses taken by a code count as panel activity, so the panel no longer relocks 30 s after the unlock while the code is keyed in. Code presses are logged under their path (`pair recovery keyed=N`).
- **Observed** (October 27), writes and a cancel during a feed: the sustained run's first cycle passed on hardware with writes during a feed and a cancel during the next. Power cuts with a feed reserved, and 4 s into a drive, each resolved as uncertain once at the next boot and never re-ran.
- **Modeled** (October 26), feeds during handshakes: the virtual feeder completes a three-portion feed with a 70 ms hold every half second (`handshake_sized_holds_during_a_drive_leave_the_feed_to_complete`) and with 80 ms holds back to back, the owner stepped once between each (`back_to_back_handshake_holds_leave_the_feed_to_complete`). A 150 ms hold ends the feed `ObservationGap`, stopped and durable. The timing argument is static analysis plus simulation.
- **Modeled:** feeds stay silent while a tune plays or stalls, on the board's platform against the fake codec (`wellfed-fw-feeder/tests/manual_feed.rs`).
- **Observed** (October 24), the resources table's first two columns: `feeder` v109 (before the device API, with the heap figure from the first product budget) and v119 (with it).
- **Observed** (October 24): `qa-20261024-01` ran from commit `ccf3e2d`. `ota-power-cuts` installs the `speaker` firmware, which serves no API, so the bench takes its UART confirmation.
- **Modeled:** the bench's `itest` scenario `feed-writes` (`--feeds`) writes from a session opened before the feed, against the scripted feeder. Its hardware run is the sustained-run entry above.
- **Modeled**, the rest of the host suite: recovery codes, stale writes, retried ids, role limits, handshakes during a feed, feed ids across sessions, readiness, timeouts and the client's `Connection` (`wellfed-client/tests/api/`, `wellfed-device-api/tests/`); the event ring's wrap, gaps, epoch and a power cut at every mutation (`wellfed-device/tests/event_log.rs`); mDNS against `simple-dns`; IK against `snow`; the byte fixtures (`wellfed-proto/tests/fixtures`).
