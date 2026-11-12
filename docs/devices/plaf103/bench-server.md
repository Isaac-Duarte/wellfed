# Bench server

`wellfed-bench serve --device plaf103` is the [relay bench](bench.md)'s one server: it holds the bench while it runs, and every other `wellfed-bench` command is its client. This reference, for contributors running or scripting the bench, covers what it holds and keeps, its feeder session, its console, the agent tools, and the lease that keeps two tools off the bench.

## What it holds

For its whole life, `serve` holds:

- the [bench lease](#bench-lease), which the jobs' children run on;
- the relay board, read at the start, after every job and every 30 s while no job runs (never during one), so its front buttons show;
- the control socket `private/bench.sock`, mode 0600 whatever the umask, so only the checkout's owner reaches it;
- the owner's one device API session with the feeder ([below](#the-feeder-session));
- a receive-only UART capture while no job runs and relay 1 is on ([below](#the-uart-capture));
- its [console](#the-console): every job's output as it comes, and a line as each job starts and ends.

**One job at a time.** A job is the command line a client posts (`POST /jobs` with its words), parsed with the command line's checks, so a bad flag or name is refused before anything runs. The client sends its checkout too, and the server refuses another checkout's: a worktree whose `private/` links here reaches this server, and must not run its sources under this checkout's session names. The job runs the command's usual sequence in the server's process. A second job is refused, naming the first:

```
Stopped: The bench is busy with job 3 (dump fw-01); wait for it, or cancel it first
```

A client that disconnects (a closed terminal, say) only stops reading: the job runs on, and `attach ID` prints its output again from the start. The server keeps its last 8 jobs' output in memory, and every job's in `private/bench-status/jobs/ID-NAME.log`, buffered and never synced so a UART write never waits on the disk, and complete once the job ends. Job numbers carry on from the highest file there, so a new server never reuses one. `ota`, and `itest` without `--power`, get a board that refuses every switch.

## Stopping the server

Ctrl+C, SIGTERM or a hangup of its terminal stops the server: it refuses new jobs, cancels the running one as a client's Ctrl+C does, waits for its cleanup, removes the socket and releases the lease. Each job's child runs in a process group of its own, so terminal signals reach only the server, never a writing `wellfed-flash`; the server forwards the cancel itself.

A cancelled write stops terminally and leaves its bank half-written until another UART write, so while the flash client holds the UART nothing stops the server by accident:

- **Ctrl+C or `q`** during a write or loader session only says "A UART write is under way; press again within 5 s to cancel it". A second press within 5 s stops the server, cancelling the write.
- **SIGTERM or a hangup** during a write is deferred, however many arrive: the server says it will stop once the write ends, shows `server.stopping` (on the console's header and in `bench_status`), refuses new jobs, lets the job finish, then stops. Outside a write it stops at once.
- **To stop a deferred server sooner,** send SIGINT twice within 5 s (`kill -INT PID` twice, no terminal needed), which cancels the write for good. SIGKILL is the last resort: the [lease](#bench-lease) and the orphan checks handle a killed server.
- **The full-screen console ending on its own** (a terminal error or hangup, a panic) stops nothing. The server prints plain lines again, says the console ended, and Ctrl+C or SIGTERM stops it from then on.

## The socket's routes

| Route | Answer |
| --- | --- |
| `GET /state` | The live state: the job and its next steps, every child it runs, the UART (idle, capturing, a loader session, or writing with sectors verified), the relays, the feeder session, the capture's tail and records, and the bank ledger |
| `GET /events` | The live state again on every change, as server-sent events |
| `POST /jobs` | `{"root": ..., "argv": [...]}`, the client's checkout and the command's words: 201 with the job's number, 400 for words that are no job, 409 while a job runs |
| `GET /jobs/{id}/stream` | The job's output lines, then its end, as newline-delimited JSON |
| `GET /jobs/{id}/output?after=N` | At most 500 kept lines after line `N`, the next line number, and the job's end once that is all |
| `POST /jobs/{id}/cancel` | `{"during_write": "refuse"\|"cancel"}`: 202; 409 for a job that already ended, or one writing flash over UART unless `"cancel"`; 503 while the server stops |
| `POST /jobs/{id}/answer` | `{"seq": N, "answer": "done"\|"yes"\|"no"}`, the answer to the running job's question (the state's `prompt`): 202; 409 `{"answer": {"error": {"refused": ...}}}` when the job is not running, asks nothing, or asks another question; 400 for an answer of the other kind |
| `POST /device/ask` | `{"id": [...], "op": {...}}`, one device API request on the server's session: 200 with `answered` and the answer, or `uncertain`; 503 `{"reason": ...}` when nothing was sent |
| `GET /device/pushed?stream=events\|logs&from=N&wait=MS` | The history (events and gaps) or log lines pushed to the session, from number `N`, waiting up to `MS` (at most 30 s) for the first, with the event the held history starts after |
| `GET /device/session` | Whom the session speaks as and with: the checkout, the key file and its public key, the feeder, and once open its kind, model, build and role; 503 while paused or unconfigured |
| `GET /banks`, `POST /banks/seed` | The bank ledger, and `banks --seed` |
| `GET /wait?for=...&timeout_ms=MS` | Holds, at most 10 minutes, until `for=job&id=N` ended (at once for a job an earlier server ran, which ended with it), `for=line&text=T` (a UART, device log or job output line containing `T` after the wait began), `for=event&kind=K`, `for=status&build=B&bank=N` (a status read after the wait began, so none from before a reboot counts; the answer names the boot count), `for=timer&label=L` (due, seen or missed), `for=idle` (no job, loader or write) or `for=prompt` (a job asks a question). 200 with `met` and how, or `timed-out`; 400 `{"wait": {"error": {"refused": ..., ...}}}` for a missing field or a condition nothing could meet (a timer nobody set, job 0, a job past the next to be given). It only reads |
| `POST /notes` | One [operator's note](#the-operators-notes), the body of a `notes.jsonl` line: 201 with the note as kept, 400 `{"note": {"error": {"refused": ..., ...}}}` for one refused, such as `{"refused": "outside-private", "path": "src"}` |

## The feeder session

The server opens one session as the bench's owner key (`private/wellfed-owner.json`; a `wellfed` command reaches it through the server only with that key, so run the CLI from the checkout with `WELLFED_HOME=private`, as the server's own jobs do), finding the feeder by its identity over mDNS or its MAC in the hotspot's neighbour table, as the rig does; `serve --feeder-address` names an address instead (the simulator, say). The session:

- subscribes after the newest event it took, kept in `private/bench-status/device-cursor.json`, so a new server does not replay history;
- follows the feeder's debug log into the state and `private/bench-status/device-log/YYYY-MM-DD.log` (one file a UTC day, each line stamped with host time, buffered, never synced). Lines the feeder overwrote before sending (a batch's `dropped`) are marked by a line such as `-- 6 lines not in the feeder's tail --`; no feeder line starts with `--`. The first boot a new server follows reads from the oldest line kept, so lines lost before that read get no marker;
- reads the status every 10 s, and sets the clock as `wellfed time` does (this host's clock and zone) when it is invalid or more than 2 s off, waiting 5 min after any refused sync. A job's clock sync through the session uses this host's zone too. `serve --no-clock-sync` leaves the clock alone, for a check that needs it invalid;
- reads whether the hotspot's connection is up every 30 s (`nmcli`, active connections only), never during a job, a loader session or a write.

It closes while a job holds the UART (`capture`, `boot`, `loader`, `transients`, `dump`, every prepare and execute); during an install push (`ota`'s or `itest`'s), reopening after its cursor once the push ends; for all of `restore-stock`, which pushes stock with sessions of its own and leaves a feeder with no device API; while relay 1 is off; and while the server stops. Otherwise it reopens after a loss, after 1 s and then twice as long each time, up to 30 s.

`ota` and `itest` ask through it: their status checks, cancels and the suite's requests are the server's session's. Only `itest`'s deliberate second and intruding sessions, an install's pushes and `install` open their own.

**Retries.** A read (status, settings, plans, statistics and the like) with no answer, or whose session dropped under it, goes once more under the same id. Nothing else is resent on any path: a write with no answer is reported uncertain, since it may have run. A request while the session is paused or unconfigured is refused with the reason and nothing sent; one while it opens waits up to 20 s. A second request under an id still out is refused (409).

If the feeder counts more sessions than the server's own when it opens, the state warns (`status` prints it): something reached the feeder without the server.

**Finding the server.** Once listening, the server writes a per-user pointer naming its socket, checkout and process to `$XDG_RUNTIME_DIR/wellfed-bench.json` (when XDG is set) and `~/.local/state/wellfed/wellfed-bench.json`, so a client without XDG (cron, ssh) finds it, and removes both when it quits. A pointer whose process is gone, or whose pid now belongs to another process, names no server. One whose process lives but whose socket is missing or refuses fails closed: every device command is refused with that pid. While another checkout's server answers, `serve` refuses to start.

**The `wellfed` CLI** reads that pointer first, whatever its working directory or key file. While the server answers, a command whose key file holds the server's key (a copy counts) goes through it, never through a direct session beside it or as a fallback. Everything else is refused, naming the server: another key, `--address`, and `setup`, `recover`, `join` and `restore-stock`, which open their own sessions. The one exception is the server's own job: a command descended from the pointer's process passes only while the state shows the session paused for that job's sessions (`restore-stock`'s pause); paused for a UART job, power-off or the server's own install push, or not paused, it is refused too. A server that does not answer refuses its jobs like any command.

**History through the server.** `events` and `feeds` read the history the server's session kept, and `logs` its log lines, so none moves the server's subscription or tail; `--samples` is refused. The server keeps the newest 4096 events and gaps and 1024 log batches, from one epoch: after a store reset it starts again empty, and numbers from two epochs are never compared.

- History older than the server holds is refused when the command subscribes, before anything else is sent, so `feed`, which subscribes first, never runs ahead of a refusal of its own events. The refusal names the event the server's copy starts after: pass that number as `--after`, or stop the server to read further back.
- `logs --after` below what the server holds is refused the same way; plain `logs` shows what it holds, with a line saying older lines were dropped.
- A command takes what it wants of the ring at its start and reads on from the end, so what it waits for (a feed's own `Fed`, say) never overruns its place. Only a follower outrun by more than a whole ring stops with an error, rather than skip messages.

## The UART capture

While no job runs and relay 1 is on, the server holds the UART with back-to-back receive-only captures, `private/captures/bench-MILLIS-uart-NN`, through the suite's capture chain. Their lines feed the state's UART tail, `PLFW1 start` checks the bank ledger, and the last `PLOTA1` record shows.

Before every job it kills the capture's observe (which only receives) and waits for the port to free; it starts a fresh one after the job, and when relay 1 reads on again. The decision is made under the lock that accepts jobs, so a capture never starts under a starting job. A capture that stops on its own (a missing port, say) stays stopped, with its reason, until then.

`wellfed-flash` caps one observe at 180 s, so an idle bench writes about 480 segment directories a day, each with `serial-rx.bin`, `serial-tx.bin`, events and a report. Clear old `bench-*-uart-*` directories from `private/captures/` now and then.

## The bank ledger

`private/bench-status/banks.json` records which build the bench believes each bank holds, from the jobs that wrote it:

| Job | Records |
| --- | --- |
| `flash-execute` | The plan's bank: its target review's startup identity, or the stock image, or else the review's OTA image hash, which no report names and so is never checked |
| `give-way-execute` | Bank 1 holds no valid image |
| `flash-execute` or `give-way-execute` stopped or cancelled once the flash client ran | That bank is unknown until a write or a seed settles it. Any non-zero exit counts, 2 included: `wellfed-flash` exits 2 for any error its command returns, not only one before serial access |
| `stock-execute` | Bank 1 holds stock |
| `ota` | Confirmed: bank 1 holds the offered build (or stock). Fell back: no valid image. `--fault give-way-only`: bank 1 gave way |
| `itest` | Each install the suite watched: confirmed, bank 1 holds the build; fell back, no valid image |
| `restore-stock` | Once the tool started, by its exit alone: 0, bank 1 holds stock; any other end, a cancel included, unknown |

Each entry is checked against what the feeder says it runs: the capture's `PLFW1 start build= bank=` banner, and the build and bank in the session's status. A report naming another build marks the entry disputed rather than replacing it, since either may be wrong; the operator decides.

`banks --seed DUMP_SESSION` starts the ledger. Today's flash inspections pin region hashes and name no build, so the seed takes `--bank` and `--build`, and says so. A seed is refused while a job runs. Like every file the server keeps, the ledger is replaced whole, never synced, and not written while a job holds the UART. A `banks.json` that does not read refuses `serve` before the relay board opens, and is left for the operator to repair or remove.

## The console

On a terminal, `serve` draws its console full screen on `/dev/tty`: the UART-write banner with sectors verified; the job, its step and next steps; Activity; the relays and the lease; the feeder (banks, session, status, clock, journal high water, hotspot, capture and the device log's last lines); the operator's step, timers and evidence by card; and a scrolling log of every job's output. PgUp and PgDn scroll the log, Up and Down a line, and End follows it again. A job's question shows in a panel above the log with its number and the `wellfed-bench answer` line that answers it; the console only shows it.

`q` or Ctrl+C stops the server as SIGINT does ([above](#stopping-the-server)). During a UART write or loader session, the first press only shows the confirmation in the footer. The screen redraws at most twice a second while the flash client writes, otherwise four times. It draws only from the live state and a channel, so a slow terminal never holds the output pump: it loses console lines instead, which the job's stream and log file keep. When the server stops, the screen closes and the log's last 30 lines print. Without a terminal (a service, a pipe), or with `--plain`, the console is plain lines on stdout.

### Watching from another terminal

`wellfed-bench console` draws the same screen for a running server, from another terminal or machine. It connects to the socket and adds no listener, so a remote run goes over `ssh -t`:

```
ssh -t HOST wellfed-bench console --root /path/to/checkout
```

- **It only reads.** It posts nothing and touches no hardware. `q` and Ctrl+C leave; the server and its job run on, as the footer says.
- **Its log holds job output only.** It follows the running job, or else the last to finish, from its first kept line, with a separator at each new job and a line at each end. The server's own start-up and stopping lines are not on the socket (a job's log file is still complete). If several jobs start while it reads an earlier one, it skips to the newest.
- **A question shows, read-only.** Answer with the panel's `wellfed-bench answer` line, or `job_answer`.
- **If the server goes away,** the last state stays with a notice, and it retries every 2 s, picking up whichever server answers next and starting its log over for a new one.
- **During a UART write** it redraws and polls at most twice a second. Over ssh it still runs on the bench host, so leave it if you want that host idle for a write.
- **If its terminal goes away** (a dropped `ssh -t`), it exits; the server runs on.

With no terminal or no server at the start, it exits with an error saying which. On leaving, it prints the log's last 30 lines.

## The operator's notes

`wellfed-bench note` posts to the server, which shows each note on the console and appends it to `private/bench-status/notes.jsonl` (buffered, never synced, and not flushed while the flash client holds the UART):

```
wellfed-bench note step "Install the candidate over UART"
wellfed-bench note next "Boot and watch the banner" "Run the smoke tier"     # replaces the list
wellfed-bench note timer fire --in 5m --expect "PLSCH1 fire"                 # or --at 18:00
wellfed-bench note timer reboot --in 2m --expect-event boot --window 5m
wellfed-bench note timer fire --clear
wellfed-bench note evidence --card schedule private/captures/boot-01 --pass --text "banner seen"
```

A card is the operator's free-text name for the work. Evidence must name a file or directory inside `private/`, given from the working directory; the server keeps it root-relative and refuses anything else.

**Timers.** `--at HH:MM` is the next time the bench host's clock reads it; `--in` takes lengths such as `90s`, `5m` or `1h30m`. A timer with `--expect TEXT` is seen by the first UART, device log or job output line containing `TEXT` from its start until its window ends; one with `--expect-event KIND` by a device event of that kind (`boot`, `fed`, `clock_sync`, as the device API names them). The window ends 2 minutes after the due time unless `--window` says otherwise, and a timer unseen by then is missed. Both outcomes are notes, so a restarted server shows them. Lines come from the idle capture, the device log and the running job's output, so a `boot` job's own banner counts.

## MCP

The MCP tools drive real hardware under the bench's [safety rules](bench.md#safety-rules).

`wellfed-bench mcp` serves the bench's tools to an agent over the Model Context Protocol on stdin and stdout. It is a client like every other command, so it needs `serve` running in the same checkout. Register it from the checkout's root, for example in `.mcp.json`:

```json
{
  "mcpServers": {
    "wellfed-bench": {
      "command": "wellfed-bench",
      "args": ["--root", "/path/to/wellfed", "mcp"]
    }
  }
}
```

Every tool's description says what it does physically.

- **Reads:** `bench_status` (the live state, tails cut to 20 lines), `job_output {id, after}`, `logs_tail {source: device|uart|job, lines}`, `banks`, and `wait_for {for, …, timeout}` over `GET /wait`, with the timeout in seconds; `for: prompt` waits for a job's question.
- **Notes:** `note_step`, `note_next`, `note_timer` and `note_evidence`, built straight from the arguments, so text may start with `-`. On the command line such text goes after `--`: `wellfed-bench note step -- "- flash bank 1"`.
- **The feeder:** `device_read {op}` asks the server's session for one read (status, settings, time, plans, clients, update state, calibration, the meal call or a stats section), and `clock_sync` sets the clock from this host. Nothing else reaches the device API from here: never a feed, cancel, plan, setting or update. An uncertain read is reported as such.
- **The panel:** `press` on a `virtual-panel` build has a finger's effects. A press with Dispense needs `confirm_motion: true` (it can feed; the hopper must be empty). A Reset hold over 1000 ms needs `confirm_destructive: true`: 5 s on an unlocked panel erases the enrolled keys, and a stall in the button task can carry a hold just short of 5 s past it. An agent that opens the recovery window with `lock+dispense` still cannot answer an install's `open-recovery-window` question: a person does.
- **Hardware:** `job_cancel`, refused during a UART write unless `during_write: true` (the write then stops for good), and one tool a command: `power {state: on|off}`, `boot`, `capture`, `loader`, `dump`, `flash_prepare`, `flash_execute` (`plan_sha256` required), `give_way_prepare`, `give_way_execute`, `stock_prepare`, `stock_execute`, `ota`, `itest`, `transients`, `restore_stock` and `press`. `restore_stock`'s `virtual_panel` needs `confirm_motion: true`, since the job presses Dispense. Each builds the command's words, the session name after `--`, and posts them as a job, so the server's parse and checks apply as to a person's. It answers with the job's number, to follow with `wait_for {for: job, id}` and `job_output`.
- **Questions:** `job_answer {seq, answer, id}` answers the question `bench_status` shows (`prompt`), once. It refuses the two only the owner may answer at the feeder, which take `wellfed-bench answer`: whether this is their feeder (`recognize-feeder`) and the recovery window at the panel (`open-recovery-window`). An agent may answer `key-setup-code` and `press-dispense`: the feeder's own check of the presses decides, and the answer only lets the installer ask its next question.

**Refused over MCP** before anything is posted, since they need a person at the bench: `attended_motion`, the suite's `feeds`, `break_lease`, a `destructive` suite run without `confirm_destructive: true`, and the two presses above without theirs. So are arguments a tool does not name, and any string a hardware tool gets that starts with `-`, so none reads as a flag (a session name never starts with `-`). Stdout carries the protocol alone.

## Bench lease

One tool drives the bench at a time. These take the lease `private/bench.lease` before touching anything and remove it when they end, on a stop or Ctrl+C as on success:

- `wellfed-bench serve --device plaf103`, for as long as it runs. Its jobs take no lease of their own, and `status` only reads it;
- every `wellfed-flash` command that opens the serial port (`read`, `update`, `give-way`, `stock-recovery`);
- `wellfed-stock setup-send --device plaf103`, for its one frame.

The lease names its holder: pid, start time and boot (so a reused pid is not mistaken for it), host, command line and when it started. A second tool is refused with that holder named, before it opens a port or switches a relay:

```
Stopped: The bench is held by pid 81234 (`wellfed-bench serve --device plaf103`, since 2026-10-26 12:00:00 UTC on bench); wait for it to finish or stop it
```

- **Chained tools.** A process descended from the holder runs on its lease and leaves it in place, so the children of the server's jobs (`wellfed-flash` captures, reads and updates; `wellfed restore-stock`) never wait on their parent. That is checked once, when the child starts, and a job waits for or stops its children before it ends, as the server waits for its job. Run by hand, `wellfed-flash` takes the lease itself, so a hand-run read cannot open the UART under a running server.
- **No lease:** a client only asks the server. The offline `wellfed-flash` commands (`plan-stock-recovery`, `decode-status`, `inspect-journal`) and an emulated helper (`WELLFED_FLASH_EMULATOR`, the host tests) take none.
- **The restore.** The server never stops for one: its `restore-stock` job runs `wellfed restore-stock` on its lease. From another terminal while a server holds the bench, `wellfed restore-stock` is refused with the server named.
- **The stock server.** `wellfed-stock` keeps the lease beside its `--config`, on the bench `private/`. Its `serve` takes none: it only answers the feeder, beside the bench command that drives it, and never asks stock to install (only the Android app does, and a phone holds no lease).

Taking, breaking and releasing run under an advisory lock on `private/bench.lease.lock`, which the kernel drops with the process, so two tools racing for a free or stale lease cannot both win. The lease is replaced atomically, so `status` never reads it torn. The lease and its rules are `wellfed_bench_formats::lease`.

While a server runs, `status` shows the relays as the server last read them, since it holds the board's port. While another holder keeps the port open (a hand-run tool), `status` shows the lease, then stops with "Serial port: Device or resource busy" instead of the relays.

### After a killed run

A server killed outright (SIGKILL, a closed terminal) cannot remove its lease or run its job's cleanup, and may have left relays on, a capture running or an update half done, so the next server refuses rather than take over:

```
Stopped: The bench lease of pid 81234 (…) outlived its process: that run was killed and may have left relays on or an update half done. Check `wellfed-bench status`, then free the bench with `wellfed-bench status --break-lease`
```

With no server only `status` runs, and that is the way back:

1. Run `target/release/wellfed-bench status`. It shows the lease first (free, held, or left by a process that is gone), finds no server, and reads the relays itself without taking the lease.
2. Stop what the killed job left running. Under a lease whose holder is gone, `status` lists every `wellfed-flash`, `wellfed-stock`, `wellfed-build` and `sigrok-cli` running on this bench, with pid and command line. They no longer count as the bench's. Let a `wellfed-flash` finish, or stop it with SIGINT (`kill -INT PID`) so it records its stop. Stop the others by hand.
3. Run `target/release/wellfed-bench status --break-lease`. It refuses while a `wellfed-flash` still runs on this bench, naming it, since it may be writing. Otherwise it removes the lease, only if its holder is gone, then shows the relays.
4. Start the server again. It replaces the stale `private/bench.sock` once it holds the lease; until then a client finds nothing answering and says to start one.

The tools are found by process name and by the `private/` of their working directory, since every job's child runs from its server's checkout, where the lease lives. So a worktree whose `private/` links to the main checkout's shares its lease and finds the main server's tools, a checkout with its own `private/` never counts them, and the killed server needed to record nothing.

`--break-lease` never breaks a live holder's lease (whatever its children), one written on another host, or a file that is not a lease; remove the last two by hand once the bench is idle. Against a file that is not a lease, `--break-lease` stops before the relays; plain `status` still shows them.

## Job records

Each job records its steps, the flash client's progress lines (bytes read, sectors verified), relay changes and the active capture in the live state (`GET /state`), which `wellfed-bench status`, the [console](#the-console) and `bench_status` read. Two files in `private/bench-status/` outlive the server: `jobs/ID-NAME.log`, each job's output, complete once it ends; and `history.jsonl`, one line per job, including the reviewed next command after a prepare, which no tool reads back.

## Evidence

- **Observed, October 26, 2026:** the bench lease refused a second command and a hand-run read while a capture held it; after the holder was killed, `status` reported the stale lease until `--break-lease`. While a capture held the relay port, `status` showed the lease but not the relays.
- **Modeled, October 28, 2026:** the server over a socket in a temporary checkout, with a fake relay board and a stand-in flash client, and no hardware. Jobs stream and end with their code; a second job is refused; a cancel sends the flash client SIGINT and the loader's cleanup cuts power and isolates the UART; a detached client's job is followed by another from its first line; quitting cancels, waits for cleanup and frees the lease; with no server every hardware command refuses. A cancel mid-write (`update execute`) sends only SIGINT and the job ends after the writer exits, the UART shown as writing; a cancelled build takes its grandchildren; the server refuses what the command line refuses (`itest --list`, a missing name, out-of-range values); a 100 KiB output burst never blocks; a panicking job leaves the server serving.
- **Modeled:** the real binary against synthetic status files.
