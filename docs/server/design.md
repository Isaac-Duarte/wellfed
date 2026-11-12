# Server design

How the server is built, for contributors changing it: the crates and their boundaries, storage, the per-feeder runtime, accounts, the command journal, and the larger features. For what it does, read [the server](README.md); for running it, [running the server](running.md).

The one idea: **the server is a mirror of each feeder, never a second source of truth.** The feeder owns feeding, its schedule and its journal. The server holds a session to each feeder, copies its history into SQLite, and forwards people's requests. Anything it isn't sure the feeder did, it settles from the feeder's history rather than doing again.

## Why it's built this way

- **REST JSON with Server-Sent Events, OpenAPI generated from code.** The live view needs only server-to-client, so SSE beats a WebSocket's custom reconnect and auth. GraphQL would need per-field authorization, and gRPC a proxy in browsers.
- **SQLite through sqlx, with compile-time-checked queries.** Self-hosters shouldn't have to run Postgres, and supporting both would mean writing every query twice.
- **Household accounts, plus a revocable token per phone.** One admin password gives no per-person audit, and a proxy or OIDC alone is too hard for most owners (it can come later on top).
- **One home per server, many devices in it.** Setup makes the home, and every device the server adopts belongs to it, whatever its kind; pets join it later. Several homes would scope every query and authorization check, so the schema holds a server to one for now.
- **Layered crates, so the compiler enforces the boundaries.** One server crate would keep its layers only by review, and the HTTP types couldn't be shared with the app.
- **A concrete `Store`, tested against a fresh SQLite per test.** Repository traits with mocks are boilerplate, and mocks drift from real SQL.
- **`jiff` for times.** Zoned times, daylight-saving changes and schedules are easy to get wrong with chrono and chrono-tz.

## Crates

```text
src/sdk/wellfed-session      no_std, no alloc. Noise, HMAC, the setup code, length-prefixed frames, hex and base64
src/sdk/wellfed-signed       no_std, no alloc. The frozen signed update formats bank 0 checks: manifest, key statements, key ids
src/product/wellfed-proto    no_std, no alloc. Device-protocol messages, framing, versions, and device kinds and models
src/product/wellfed-device-api no_std, no alloc. The feeder's side: the sans-IO session server, pairing, the setup exchange
src/client/wellfed-model     owned device API values (status, settings, events, requests, answers) and their conversion from the wire
src/client/wellfed-client    sans-IO. Noise sessions, pairing, invites, the Connection: keepalive, answers, event cursor, recovery
src/client/wellfed-connect   tokio loop over wellfed-client's Connection: locate, open, ask, request and receive (several outstanding), follow, join, recover,
                              and the update driver (Installer) the CLI and the server share
src/client/wellfed-install   tokio driver of wellfed-client's install from stock: stock's server in-process (wellfed-stock),
                              pairing, the backup, the promotion; the Android app runs it, the CLI its restore
src/client/wellfed-server-client  a typed async client of the server's HTTP API over wellfed-api (reqwest with rustls,
                              the SSE stream included): the app, the CLI and the server's own tests
src/tools/wellfed-feeder-sim the host-simulated feeder: wellfed-feeder's loop pass and stores over real TCP,
                              on wellfed-device-sim, which every simulated device kind shares
src/server/wellfed-api       HTTP bodies, error codes, OpenAPI schemas; a module per resource. No axum. Builds for Android
src/server/wellfed-store     sqlx, migrations, typed queries; a module per table group, one Store. Raw rows plus
                              projections. The only crate that contains SQL
src/server/wellfed-mirror    the per-feeder runtime: connect, sync, ingest and projections, the command lanes (an
                              install, a stock restore or a read of bank 1 on the long one), and a typed broadcast of
                              each feeder's mirror events
src/server/wellfed-releases  fetch, upload and verify releases, the release cache, and each feeder's verified stock backup
src/server/wellfed-webhook   the alert webhook: each alert's opening and clearing POSTed to the owner's URL, at least once under one id
src/server/wellfed-service   the use cases: accounts and auth, the home, commands and their journal, pets and food,
                              derived views, audit, export, and the hub. No axum, no SQL
src/server/wellfed-server    the binary: config, axum routers per resource, the auth extractors, SSE, the jobs
                              supervisor
src/server/wellfed-server-testkit  a temp store, a fixed clock and random, the simulator harness, signed test releases, the server in process
src/app/                      Flutter app, plus a thin flutter_rust_bridge crate (wellfed-app-bridge) over wellfed-client,
                              wellfed-connect, wellfed-install and wellfed-model, and wellfed-server-client and wellfed-api for the server
```

Every crate on the map exists, and the app talks both to feeders and, through `wellfed-server-client`, to the server. There is no web UI, and none is planned.

The server's crates follow one rule: **a crate per runtime or dependency boundary, a module per domain inside it.** A new domain, such as pets or alerts, is a module and some tables, and gets a crate only if it brings its own runtime (tasks), a heavy dependency (`reqwest`) or a consumer outside the server. So the mirror (the feeder tasks), the releases (`reqwest` and the verifier), the webhook (`reqwest` and its delivery task) and the HTTP client (the CLI) are crates, and accounts and pets are not.

| Crate | What it owns |
| --- | --- |
| `wellfed-api` | Every HTTP body, a module per resource. It builds for every target, the app's Android library included, so it carries no server dependencies. `wellfed-service` returns these bodies directly until a second API version exists. |
| `wellfed-store` | All SQL, a module per table group (`users`, `homes`, `feeders`, `history`, `projections`, `samples`, `owner`, `commands`, `releases`, `jobs`). One crate, so the sqlx offline metadata and the migrations stay in one place. |
| `wellfed-mirror` | Every task that holds a feeder session (connect, sync, clock sync, ingest, reconnect and the command lanes), and the injected `Clock` and `Random` every server crate uses. It never sees a user or a stream id. |
| `wellfed-releases` | Fetching, uploading and verifying releases, the release cache, and each feeder's stock backup. |
| `wellfed-webhook` | The alert webhook's body, its URL rule and the one task that delivers each change. |
| `wellfed-service` | The use cases, a module per domain, each with its own error enum wrapped in the crate's `Error`, plus the auth rules, the hub and each job's body. It never sees a socket or an HTTP type. |
| `wellfed-server` | The binary and the jobs supervisor. A handler parses, authorizes through an axum extractor, calls one service method and maps the result. It never sees SQL. |
| `wellfed-server-client` | The typed client the app, the CLI's `wellfed server` commands and the server's own tests go through, so neither can disagree with the routes. Nothing in it retries. It leaves out a page entry or stream update it can't read, so a newer server's additions cost an older client only those. |
| `wellfed-server-testkit` | The service on a temporary store with a fixed `Clock` and a seeded `Random`, the simulated feeder, and the HTTP server in process. |

A few crates outside `src/server/` matter here:

- **`wellfed-model`** holds the device API's values as a host sees them, owned, with no lifetimes or wire buffers, and they cross into Dart and the server's store and HTTP API unchanged. `wellfed_model::convert` is the one place they meet `wellfed-proto`'s borrowed wire types. `wellfed-api` serves the model's status, settings and event bodies as they are, adding only what the server knows (when it received them, its own ids).
- **`wellfed-client`** holds no sockets, timers or radios. Its `Connection` owns every session rule: an `Alive` keepalive after 45 s quiet, under the feeder's 90 s idle limit; the end of a session heard from not at all for 90 s; one answer or `Unanswered` per request; and the event cursor with its epoch.
- **`wellfed-connect`** is a thin tokio loop over that `Connection`, taking its randomness from the caller so the server can inject its generator and tests can fix it.
- **`wellfed-feeder-sim`** runs `wellfed-feeder`'s own API server, panel and pairing policy on a flash image laid out like the feeder's storage, so service, end-to-end and web tests meet the feeder's real behavior, not a mock. A test can stall a feed in flight, and the harness's relay can hold back the feeder's answers or cut every session.
- **The export archive's** manifest and row types live in `wellfed-formats` (`wellfed_formats::export`), since export writes the archive and import reads it.

### Allowed dependencies

Each arrow means "may depend on":

```text
wellfed-server ─► wellfed-service ─┬─► wellfed-mirror ─┬─► wellfed-store ─► wellfed-api ─► wellfed-model
       │                             │                    │          └─► wellfed-formats (the export archive's rows)
       │                             │                    └─► wellfed-connect ─► wellfed-client
       │                             ├─► wellfed-releases ─► wellfed-store, wellfed-client (the release verifier)
       │                             ├─► wellfed-webhook ─► wellfed-mirror (the kept alerts), wellfed-store, wellfed-client
       │                             └─► wellfed-store (projections), wellfed-api (the bodies it returns)
       └─► wellfed-api, wellfed-store (to open it)

wellfed (the CLI) ─► wellfed-server-client ─► wellfed-api
          └─► wellfed-connect (the device API, as today)

wellfed-app-bridge (the app) ─► wellfed-server-client, wellfed-api
          └─► wellfed-connect (the device API)

wellfed-homeassistant (Home Assistant's bridge) ─► wellfed-server-client
          └─► wellfed-connect (the device API)
```

| Crate | May depend on (workspace crates) |
| --- | --- |
| `wellfed-api` | `wellfed-model` |
| `wellfed-store` | `wellfed-api` (its ids and enums), `wellfed-formats` (the export archive's rows, which it reads and inserts) |
| `wellfed-mirror` | `wellfed-store`, `wellfed-api`, `wellfed-connect`, `wellfed-client`, `wellfed-model`, `wellfed-proto`, `wellfed-session` |
| `wellfed-releases` | `wellfed-store`, `wellfed-api`, `wellfed-client`, `wellfed-model`; the verifier is `wellfed_client::release`, over the firmware's own offer check |
| `wellfed-webhook` | `wellfed-mirror`, `wellfed-store`, `wellfed-api`, `wellfed-client`, `wellfed-model`, `wellfed-formats` (hashing) |
| `wellfed-service` | `wellfed-mirror`, `wellfed-releases`, `wellfed-webhook`, `wellfed-store`, `wellfed-api`, `wellfed-model`, `wellfed-formats`, `wellfed-session` (hex) |
| `wellfed-server` | `wellfed-service`, `wellfed-api`, `wellfed-store` |
| `wellfed-server-testkit` | `wellfed-server`, `wellfed-service`, `wellfed-store`, `wellfed-api`, `wellfed-server-client`, `wellfed-feeder-sim`, `wellfed-client`, `wellfed-connect`, `wellfed-model`, `wellfed-session`, `wellfed-signed` (to sign test releases) |
| `wellfed-server-client` | `wellfed-api` |

`cargo xtask check-repository` enforces these from `cargo metadata` (all features, every platform), each error naming the edge or path that breaks them.

- **SQL stays in the store.** No crate but `wellfed-store` depends on sqlx, in its tests included.
- **HTTP stays in the binary.** Nothing below `wellfed-server` links axum, or any of axum's own crates (`axum-core`, `axum-extra`), by any path.
- **The arrows as drawn.** Each server crate's normal and build dependencies on workspace crates come from its row above. A crate outside the server takes a server crate only where drawn: the CLI and Home Assistant's `wellfed-homeassistant` take `wellfed-server-client`, and the app's `wellfed-app-bridge` takes `wellfed-server-client` and `wellfed-api`. Dev-dependencies are exempt, since a test may build a simulated feeder or a whole server. A row's crate is checked from its first manifest. Changing a row changes this table and the check together.
- **`wellfed-api` on Android.** Its own dependencies come from an allow-list (`jiff`, `wellfed-model`, `serde`, `utoipa`, `uuid`). `make check` builds no Android target; `make check-app`'s APK build does, `wellfed-server-client`'s reqwest and rustls included.
- **Files stay small.** A Rust source under `src/server/` has at most 400 lines, or 600 under a `tests/` directory; a `tests.rs` beside the code it tests keeps 400. Generated files (`openapi.json`, the sqlx metadata) aren't counted.

Beyond the [Rust standard](../development/rust.md): a change that wants SQL in a handler or axum in the service moves the code rather than adding the dependency. Service errors map to `problem+json` in one `IntoResponse`, with no `anyhow` below the binary. IDs are newtypes, roles and metrics are enums, and generations are their own type. Every server dependency's version is pinned in `[workspace.dependencies]`; a crate's manifest only picks features.

## Storage

- **SQLite in WAL mode.** One writer connection, through which every write goes so writes never contend, beside a small read pool. A household's one to five feeders need no writer task in front of it. Every commit is synced before it returns (`synchronous=FULL`), so a command journal row outlasts a power cut before its write goes out.
- **Migrations** are sqlx's, embedded in the binary and applied at startup. Before pending migrations run on an existing database, `VACUUM INTO` copies it beside itself as `server.db.before-<version>-<unix ms>`, written under a hidden partial name, made readable by the server's user alone, synced, and renamed into place, as the backups are; a partial copy a cut start left is removed at the next. Until the first server release they are squashed freely, since no owner's database has run them; from then on they are append-only, and a released migration is never edited: sqlx refuses to open a database whose recorded checksum differs from the migration the build embeds. `wellfed-store/tests/fixtures/migrations.txt` pins each migration's checksum, and its test fails on an edited migration or one not yet listed. A database whose record names a migration the build lacks was migrated by a newer build: `Store::open` opens it read-only, running and copying nothing, and the server refuses to start on it rather than write over what the newer build stored. Codes the feeder's events carry grow with the device API, which needs no migration, so a rolled-back server reads an event kind, feed outcome or reason, blocked reason or source it doesn't know as unknown. Every other stored code is the server's own (models, roles, units, species, jobs, install and command states): a build that stores a new one adds a migration with it, even an empty one, so an older build opens that database read-only rather than fail on the value. The JSON columns (a command's outcome, an audit action, an install's failure, a revoke attempt) hold device API values too, and one an older build can't read, a new weekday in a stored plan for instance, reads as unknown; a cached status, settings or plans reads as not yet read. `wellfed-store/tests/fixtures/codes.txt` pins each such set, the JSON columns' through every type they nest, under the newest migration's version, and its test fails when a set changes with no new migration.
- **Queries** use `sqlx::query!` and `query_as!`, with the offline metadata (`src/server/wellfed-store/.sqlx/`) committed, so builds need no database. `make sqlx-prepare` regenerates it from a scratch database that the `sqlite3` command line builds from the migrations.
- **Time** is stored as integer Unix milliseconds (UTC), with zones as IANA names, converted by `jiff` at the store boundary. The store never reads the wall clock: `Store::open` is given the time.
- **Secrets.** The sealing key lives in `keys/server.key`, outside the database, and seals each feeder key with ChaCha20-Poly1305, bound to the feeder's row ID and identity, so a copied database alone can't impersonate the server.
- **Volume.** About 100k sample rows a day per feeder is 1–2 GB a year, which SQLite handles. Export streams rows, never loading the database into memory.

The tables:

| Group | Tables |
| --- | --- |
| Accounts | `homes` (one row for now: its id, name, units, time format and IANA zone), `users` (with an optional expiry), `sessions` (with their last use), `tokens` |
| Devices | `feeders` (every adopted device whatever its kind: its home, kind, model, identity, sealed key and cursor), `feeder_state` (cached plans, settings and status, with generation), `phone_keys` (the phones' keys the server invited, an adopting phone named, or a phone key invited), `invitations` (joins through an invite the feeders reported, until attributed), `dropped_phones` (the first bytes of phone keys the feeders dropped, whose invitees still go), `kept_clients` |
| History | `events`, raw, keyed by feeder, epoch and `seq`, so the cursor sync is idempotent and a re-created ring's history stays apart; `gaps` |
| Projections | `feeds` (one row per feed, its latest event), `lock_changes`, `boots`, `clock_syncs`, `blocked_changes`: written in the raw insert's transaction, and rebuilt from `events` when `projection_version` changes |
| Samples | `samples` (feeder, metric, time, the raw value and its number), `sample_rollups` (count, min, max and sum per feeder, metric and UTC hour), `sample_marks` (the newest sample rolled up, and the number code's version) |
| Diagnostics | `feed_traces`, `feeder_stats`, `food_readings`: beside the history, never rebuilt from it |
| Alerts | `feeder_alerts`: each feeder's last alert table and what the server made of it, one JSON document with its own `format`, written over the version it read |
| Commands | `commands` (the journal), `installs`, `job_runs` |
| Owner data | `pets`, `foods`, `pet_weights`, `pet_feeders`, `feeder_foods`, `refills` |
| Audit | `audit`: account changes as typed JSON, and a row per journal entry that the journal's own insert trigger adds |

Releases and stock backups are files in the data directory, not tables.

## Runtime

**One task per feeder.** The mirror's task owns the TCP connection and the `wellfed-client` session. It reconnects with jittered backoff and syncs events from the stored cursor before anything else (from the oldest event when the feeder's epoch is new), then sets the feeder's clock, and again every 6 h. A feeder of another API revision gets all of this, since the API only grows: the feeder refuses what its build lacks, and what a newer build adds reads as unknown. The feeder's `skew` says which side to update ([device API evolution](../firmware/device-api.md#evolution)). A pre-revision build (`skew` `legacy`) is the exception: its session carries only an install, and any other command is refused `409 feeder_legacy` unsent. An event the server can't decode is stored raw with no kind, and a later server build decodes it when it rebuilds its projections. It records every event, gap and sample raw, each raw row with its projections and the cursor in one transaction. Samples go in small batches: 32, or `Timing::sample_wait` after the first, and at the session's end.

**Device kinds.** Each adopted device is stored with its home, model and kind. The invite redemption's answer names the model, as a session's welcome does, and the adoption stores it with the key, opening no session of its own. Every op says which kind serves it (`wellfed_model::Op::kind`, `None` for the shared core), and the service refuses another kind's read or write `409 other_kind` by the stored kind, before it is journaled or sent, online or not. The mirror's task ends a session whose welcome names another model than the one adopted before it reads or sends anything, and `wellfed-connect` refuses another kind's op on the wire too. The API and the tables still name every device a feeder.

**Two command lanes.** Commands reach the task through a bounded channel per lane, one for short commands and one for the steps of a long operation such as an install or a meal call's upload, so a short command waits at most one step of a long one. Each lane has one request in flight, matched to its answer by id. A command times out as "unknown, retry with the same key", never "failed", since the feeder may have run it. A feeder unheard for 90 s counts as gone, so one that loses power shows offline within that time.

**The hub**, the service's, merges each feeder's broadcast with command results and install progress, and assigns the stream ids `GET /stream` resumes from. A forwarder that falls behind drops its backlog and publishes `resync`. The SSE `id` is `<epoch>-<seq>`, the epoch fixed at each start, so an id from before a restart resyncs.

**Discovery** browses mDNS (`mdns-sd`) for a paired feeder whose stored address fails, unless `discovery = "off"`.

**Shutdown** on Ctrl-C or SIGTERM goes through one `CancellationToken` tree, which ends the streams, stops the jobs and every feeder task, and closes the database.

**Determinism.** Time, randomness and the network sit behind small injected interfaces, so service tests run against the host-simulated feeder with a fixed clock. No server crate reads the wall clock outside `SystemClock`: `src/server/clippy.toml` refuses jiff's `Timestamp::now` and `Zoned::now`, std's `SystemTime::now`, `SystemTime::elapsed`, `Instant::now` and `Instant::elapsed`, and uuid's `Uuid::now_v7` and `Timestamp::now`, and `SystemClock`'s `#[expect]` fails the build if that rule stops applying. IDs are UUIDv7 built from the `Clock` and `Random`. Timeouts use tokio's `Instant`, which is monotonic and follows a paused test runtime.

**Jobs.** One supervisor task in the binary (`wellfed_server::jobs`) runs the background jobs one at a time on the service's `Clock`, in the home's zone (UTC before there is one). A job missed while the server was down runs once at the next start.

| Job | When | What |
| --- | --- | --- |
| `expiry` | Hourly | Deletes idle sessions and every session of an expired account |
| `rollups` | Hourly | Folds new samples into their hours |
| `retention` | 03:00 | Drops raw samples past `retention.raw_sample_days` |
| `backup` | 03:30 | Copies the database into `backups/` |
| `release_check` | 04:00 | Checks GitHub releases, while `releases.fetch` is on |

**Health and logs.** `GET /health` is liveness: it reads nothing and answers the version. `GET /api/v1/status` is readiness for an owner: whether the store answers on its writer and a reader, each feeder's link, and each job's last run. Every log line from a feeder's task sits under a `feeder{id=…}` span, and every HTTP request under a `request{id=… method=… path=…}` span whose id comes back in `x-request-id`; the query string isn't logged. There is no metrics endpoint, since a home has no scraper to feed.

## Alerts

The feeder decides every alert ([alerts](../firmware/alerts.md)); the server reads, keeps and passes them on.

- **The reader.** A task beside each feeder's session reads the status on the short lane, and the table (`GetAlerts`) only when the status shows its cursor moved: the ring's epoch, the boot count and the table's generation. It looks at each connect, 2 s after each new event (an event opens most reports), and every minute while the feeder is online (`AlertReads`), since a condition such as food running low opens with no event. Neither read changes the feeder.
- **The judgement is the phone's.** The table goes into a `wellfed_client::alerts::Watch`, the tracker the app keeps, with no mutes and the server's own fingerprint, so its own joining stays quiet. It decides what opened, what ended and how long an absent alert is held, by the phone's rules. Each failed connect counts towards unreachable and each session opened resets it, by the phone's rule: three failures in a row over at least ten minutes.
- **Kept in the store, not in memory.** `feeder_alerts` holds the last table, the watch and the alerts the webhook was told opened. A restart serves the last table at once and doesn't announce what was already told, and a delivery the server never finished goes at the next start. The first table after a fresh row (an adoption, an import, a row of another layout) is taken quietly. The row's JSON is format 1, pinned against a committed sample, and a poll that changes nothing writes nothing. The reader and the webhook both write the row, each over the version it read, so neither loses the other's change. The table is the feeder's RAM, so the row is derived and isn't exported; an import reads every table again.
- **The webhook** (`wellfed-webhook`) is one task. It takes each feeder's changes from the watch in order, POSTs them, and confirms each in the row once delivered or given up, at least once under one id; only an opening the receiver took (a `2xx`) is noted for its clear. A removed feeder's row is dropped unsent. With no URL set it confirms them unsent ([alert webhook](README.md#alert-webhook)).
- **The route** serves the row (`GET /feeders/{id}/alerts`), and the stream says when it changed (`alerts`).

## Accounts and roles

- **Passwords** are argon2id-hashed, at least 10 characters, with no other rule. The server hashes at most two at once (about 19 MiB each), and an unknown name costs the same hash as a wrong password.
- **A login** gets a session cookie (`HttpOnly`, `SameSite=Strict`, `Secure` when a trusted proxy reports HTTPS). The server has no web UI, but tokens are made from a login, so the CLI logs in this way, and these rules keep a browser on the owner's machine from being turned against the API. A session lasts 30 days from its last use. Every cookie-authenticated write needs an `Origin` matching the server, and `Sec-Fetch-Site: same-origin` when the browser sends it.
- **Phones and the CLI** get a bearer token, made from a login only: `plt_` and 64 hex digits, shown once and stored as its SHA-256. A token carries its user, not a role, so a role change applies at once. A request with `Authorization` is judged by it alone and skips the `Origin` check. From a token, making another token, setting a password, adding an account, raising a role, clearing or extending an expiry, or keeping a feeder's client is `403 session_required`, so a leaked token can't mint others, leave a login behind it, or vouch for a key it enrolled. A token may still demote, shorten an expiry and delete. **A phone logs out** with `POST /tokens/{id}/logout` from a login session, so the app asks for the password. The body names the phone's own public keys (`keys`, at most 64, or `400 invalid`); the app has one key for every feeder. In one transaction the token is deleted, the named keys under it and the keys they invited, in turn, stay the account's with no token, and every other key under it is revoked with it, so a key someone else got through the token never outlives it. A named key not under the token is passed over, since the phone doesn't know which feeders recorded it. The phone still reaches its feeders directly, the client list shows the kept keys under the account, deleting, demoting or putting an expiry on the account still revokes them, and revoking any other token never does. The audit log records a `logged_out`, not a `token_revoked`. The token can't ask itself, or a leaked one could keep the keys it enrolled past its revocation; `DELETE /tokens/{id}` from anywhere else (the lost-phone case) revokes them all. **Revoking a stolen owner's token is not always enough.** The token can get an owner-role phone key on a feeder, and while that key is live on the LAN its holder can revoke the server's own key or move the feeder to another Wi-Fi network. Either way the server never reaches the feeder again, so no revocation of that key ever lands, and only an owner at the feeder (a revoke from a phone on the LAN, or a reset) removes it. This is a known gap, without a design yet.
- **Expiry.** An account may carry an expiry, for a pet sitter. Past it the account is disabled, not deleted: its login, sessions and tokens are refused `403 account_disabled` until an owner extends it. Owners never expire, and the last owner can be neither demoted nor deleted (`409 last_owner`).
- **Failed logins** are limited to 5 a minute for a name from one address, 30 a minute for a name from every address together, and 20 a minute from one address across names, in token buckets on the injected clock, in one map of at most 10,000 entries. Someone guessing at a name from their own address locks out only that address, not the owner logging in from theirs; the ceiling bounds guessing spread across addresses at 43,200 a day. Six addresses (six IPv4 addresses or six IPv6 /64s, and one /48 holds 65,536) fill that ceiling for as long as they keep at it, so an address the account logged in from within the last 30 days skips it, keeping only its own 5 a minute and its address's 20. Those known addresses are the account's, by its id (names are compared as typed, so two accounts may fold to one bucket), kept in memory, at most 16 for an account and 4,096 in all, and only a login with the right password adds one. Locking an owner out then takes sharing their address, or a flood while they log in from a new network, a rotated /64, or just after a server restart. A limited login is `429 login_limited` with `Retry-After`; no account is ever locked.
- **The server has its own feeder key** per feeder, enrolled by invite like any client, so the feeder's history shows the server as one client and the server's audit log says which person it acted for.

Roles are owner, member and viewer, and every account sees every feeder. The service's `Permission` enum and its one `can(role, permission)` table hold the whole matrix. Each route names the permission it needs as a type (`Allowed<need::Control>`); a refusal is `403 forbidden`.

| Permission | Covers | Owner | Member | Viewer |
| --- | --- | --- | --- | --- |
| `Read` | Status, history, samples and the food views | Yes | Yes | Yes |
| `Feed` | Feed, and cancel a feed | Yes | Yes | No |
| `Control` | Settings, the portion calibration, the lock, a tune, the meal call (upload, play, erase), a clock sync that doesn't step the clock | Yes | Yes | No |
| `InviteOwnPhone` | An invite for one's own phone | Yes | Yes | No |
| `OwnHistory`, `OwnAccount` | One's own commands, audit entries, password and tokens | Yes | Yes | Yes |
| `AllHistory`, `Users` | Everyone's commands and audit entries, and other accounts | Yes | No | No |
| `Home`, `OwnerData` | The home and the feeders' time settings; pets, foods and refills | Yes | No | No |
| `FeederAdmin`, `Adopt` | Wi-Fi, reboot, a feeder's clients and its debug log; adopting and removing feeders | Yes | No | No |
| `ServerData` | Releases, installs, the stock backup, backups, export and import | Yes | No | No |

**The audit log** (`GET /audit`), kept forever, is the command journal interleaved with account changes: setup, logins and failed logins with the client's address, users created, changed and deleted, passwords set, tokens made and revoked, and changes to the home. A journal row joins the log through a trigger in the statement that records it, so no write reaches a feeder unlogged. Owner data and adoptions aren't in it yet.

### Phone keys

Owners' and members' phones hold their own feeder keys beside the server's, for when the server is down ([two paths, one feed](README.md#two-paths-one-feed)). A phone that paired before the server claims its key with `POST /feeders/{id}/phones`, or names it on adoption (`phone_key` in `POST /feeders`). The server checks the feeder lists the key and records it as that account's, so account changes revoke it.

- **A refused claim leaves the adoption standing**: `409 phone_key_not_listed` if the feeder doesn't list it, `409 phone_key_unconfirmed` if it couldn't be read. Claim again once it's online.
- **A repeated claim returns its record**, so retrying a lost answer is safe. A claim of the account's own key that a logout left with no token puts it under the claiming token, so after logging in again, revoking the phone's token revokes it again; the token's other key on that feeder is replaced, as on any claim. **That claim proves the phone holds the key.** Any token of the account may claim, so without the proof another token (a stolen one) could take the logged-out phone's key and then have it revoked with itself. The phone asks for a challenge (`POST /phones/challenge`): the server makes a one-use X25519 key pair and a nonce, held in memory for 60 seconds, at most 8 per token, and only the asking token may answer it, once. The claim's `proof` is an HMAC-SHA256 over the nonce, the token's id and the phone's public key, keyed by HMAC-SHA256 of a fixed label under the X25519 of the phone's secret and the challenge's key. The server derives the same key from its own secret and the phone's public key (`wellfed_session::proof`). Without a proof the claim is refused `phone_key_proof_required`, and with one that answers no challenge of the token's, `phone_key_proof_refused`; a proof given is checked on any claim. The app proves every claim it makes. It claims its own key on every feeder the server lists after each login, once a session per feeder: a claim that fails (the server unreachable, the feeder offline, a challenge that ran out) holds up neither the login nor the other feeders, and is tried again on the next list. Until the phone claims it, a key with no token is left alone by the reconcile, by other tokens' revocations and by another key's replacement.
- **Only owners claim new keys**, with a phone's token and no expiry on account or token. The feeder can't prove which phone holds a key, so a claim trusts the account, and a member could claim and then revoke someone else's: a member's claim of an unrecorded key is `403 phone_claim_owners_only`.
- **A member's phone that paired before the server** asks for an invite (`POST /feeders/{id}/invites`) and joins with a new key; the old one shows unowned for an owner to revoke.
- **An invite is bound to the key it names.** The server records the key, then asks the feeder for an invite only that key can redeem (`CreateBoundInvite`), so a token's holder who joins with any other key is refused. A feeder too old for the op refuses it, and the request fails until the feeder is updated.
- **A phone key's own invitees are its account's.** A joined phone key may ask the feeder for plain invites over its own session. The feeder reports each join through an invite with the first 7 bytes of both keys (an `invited` event, just before the `enrolled`), and the server records the new key under the inviter's account and token, with the inviter's prefix. Deleting, demoting or putting an expiry on the account, revoking the token, or an owner revoking the inviter (through the server, or on the LAN: on the feeder's `revoked` event the server reads its clients and forgets each phone key no longer listed, once that key's invite can no longer be redeemed) then revokes the invitee too, and any key it invited in turn (`inviter_revoked` when the inviter's revoke is why). A token given a new key keeps them: the old key's invitees move to the new one. A join through the old key's invite that the server reads only after the old key was dropped is revoked as it is recorded, not moved: by then the old key is a dropped one. An inviter the feeder already dropped leaves a record, so an invite's join the server reads later is revoked as it is recorded, and so does an invitee the feeder dropped before the server attributed it, so the keys it invited still go. Only a join the server records after it recorded the inviter counts, by the order it recorded them in rather than its clock, so claiming a phone never sweeps in the invites it made before the server knew it. What this can't reach: those earlier joins, a join whose event the feeder's history lost before the server read it (a gap), and a feeder before device API revision 3, which reports none. Such a key shows unowned for an owner to revoke or keep. An owner who keeps an attributed invitee (`POST /feeders/{id}/clients/{key}/keep`, from a login) detaches it: it shows as kept, and nothing of its inviter's revokes it. The keys it had invited stay the inviter's account's and go with it, and a join it makes once kept is no phone key's, so that key shows unowned for an owner to revoke or keep. Keeping one key never vouches for keys the owner hasn't seen.
- **Never claimable:** the server's own key, a client an owner kept, a key the feeder lists as a server's or update signer's, or another phone's key.

**Route parity.** `wellfed_service::parity` gives every device operation the app can make a server route or a stated reason for none, matching exhaustively over the device API's operations, so a new operation doesn't build until it has one.

## Commands and feeds

**The command journal**, kept forever, records every write before it is sent: its key, user, token, feeder, kind, the SHA-256 of the request's canonical JSON, the device request ID, and its state (`sent`, `answered`, `unknown`, `resolved`) with the outcome. Every user shares the server's key on the feeder, so the device request ID hashes the user, the feeder, the kind of write and the `Idempotency-Key`. A key reused for a different request is refused `422 key_reused`.

A retry under the same key reads the journal first:

- An **answered** write returns its recorded outcome without reaching the feeder.
- A **sent** write is still in flight, so a retry is answered `outcome_unknown` and sends nothing. A server that stops first leaves it `sent`, and the next start marks it `unknown`.
- An **unknown** write is sent again under the same device request ID when its kind allows: settings, plans, the calibration, lock and a cancel. A meal call's upload resumes instead: the retry reads where the feeder's upload stands and sends only the rest. An upload or an erase of the meal call keeps the clip the feeder held at its first send, and an upload also keeps that the feeder took its start. A retry is `superseded`, sending nothing, when the feeder holds another clip by then, or, for an upload the feeder took, any clip but its own. A feed never is resent, because the feeder forgets request ids at a reboot. A retry that finds the feeder offline sends nothing, but the first send may have run, so it is answered `outcome_unknown`, never `feeder_offline`.
- An **unknown** write the feeder answered without settling it is never sent again, whatever its kind, and a retry is answered `outcome_unknown`. That is an answer the server can't read (a newer feeder's), and a refusal of a resend: a resend that finds the feeder forgot the first send runs again, so a settings change the first send made comes back stale, and the refusal says nothing of the first.
- A **refused** write is final for its key, `409 feeder_refused` with the feeder's reason; trying later is a new request with a new key.

**Feeds.** `POST /feeders/{id}/feed` (`{ "portions": n }`) requires `Idempotency-Key` and answers `202` once the feeder has reserved the feed in its journal, with the feeder's number for the feed, which keeps counting across restarts. There are no server-side limits: the feeder's refusals (`busy`, `invalid` past its cap) are the policy. The feeder's `Fed` event settles the entry.

A feed whose answer was lost has no number and is never sent again. The one `Fed` event from the server's key after the history the server held when it was sent settles it, when that match is unambiguous; another client's feed or a panel feed never does. Once the feeder has restarted since, and the server holds its whole history with no gap, a feed still unsettled is `resolved` as `unrecorded`: whether food was dispensed is unknown, and it is never resent. A `Fed` event found later replaces `unrecorded`.

`POST /feeders/{id}/feeds/{feed}/cancel` goes through the journal too: `204` while the feed runs, `409 feeder_refused` `not_found` once it has ended, or for a number no feed holds yet.

**Settings** are always read from the feeder, never the cache, since a change must name the current generation; an offline feeder's settings are `503 feeder_offline`.

**The zone.** The home stores its IANA zone by its canonical name, and every feeder keeps it twice: each clock sync carries its offset and next change, and the feeder's time settings hold its POSIX rule, so its clock and plans follow daylight saving with the server down. The rule comes from the tzdb bundled with `wellfed-client`, and is written whenever the feeder holds a different one.

## The CLI on the server

`wellfed server …` speaks this API through `wellfed-server-client` alone, beside the device commands. Each command takes one path ([two paths, one feed](README.md#two-paths-one-feed)).

- **Login.** `wellfed server login --server URL --name NAME` reads the password from standard input, opens a session, makes a token, keeps it in `wellfed-server.json` in the CLI's data directory (`--token-file`, or `WELLFED_HOME`) with mode 0600, beside the device API's key file, and ends the session. Both files are versioned by the beta upgrade rule (`wellfed_formats::kept`): one a newer CLI wrote is refused and never written over. A token file that doesn't read at all is set aside as `wellfed-server.json.damaged-<n>` by the next `login`. `logout` revokes the token and deletes the file.
- **Verbs.** `status`, `settings`, `plans`, `lock`, `desiccant`, `feed`, `cancel-feed`, `clients` and `reboot` take `--feeder` with an id or a name when the server holds more than one. Then there are `events` (`--follow` over the stream), `command <key>` and `commands`, `users`, `tokens`, `releases`, `install`, `stock`, `clients`, `revoke` (a client by at least 8 hex digits of its key), `reboot`, and `export <file>` (`--csv` for spreadsheets).
- **Output.** Text by default; with `--json`, the API's bodies, one per line, a refusal's problem included. The exit status is 0 on success, 1 on an error and 3 when an outcome is unknown.
- **Uncertain writes.** A write goes out once under a fresh `Idempotency-Key`. When its outcome is unknown, the command prints the key and `wellfed server command <key>`, exits 3, and sends nothing more: not under a new key, and never over the device API. A feed waits on the stream for its result, opened before the feed is sent. An `uncertain` or `unrecorded` result exits 3 too.

## Samples

- **A number per sample.** Each stored sample keeps the feeder's bytes and one number to chart it by (`wellfed_api::sample_number`): `power` the battery's millivolts, `rssi` dBm, `heap` free bytes, `loop` the 99th-percentile pass in µs, `gateway` the round trip in ms, `wifi` the access-point changes, `supply` the battery's millivolts. When what a number means changes, `SAMPLE_NUMBER_VERSION` moves, and the next start renumbers every stored sample.
- **Hourly rollups** (count, min, max, mean per UTC hour), kept forever, are folded in by sample id by the hourly `rollups` job, so hours a down server missed are filled later. A read at hour resolution adds samples past the last run from the raw rows.
- **Retention.** Raw samples are kept forever unless `retention.raw_sample_days` is set; then the daily `retention` job deletes older raw samples in whole UTC hours, leaving their rollups.
- **Reads.** `GET /feeders/{id}/samples?metric=&from=&to=&resolution=raw|hour&after=&limit=` answers pages of 1–1000 items (default 100), oldest first.

## Releases

`wellfed-releases` keeps the signed updates an owner may install. Nothing installs from here.

- **A release file** is the version 2 manifest (160 bytes) followed by the image it names. On GitHub it is a release asset whose name ends in `.plota`. Drafts and pre-releases are skipped.
- **Verification** is `wellfed_client::release::verify`: the feeder's own `admit_offer`, with the project's keys as built-in keys and the owner's as enrolled ones, then the image's length and SHA-256 against the manifest, and that it starts with the bank's validity marker. Uploads and fetched assets go through the same call, and an install verifies the release again.
- **Models.** Each manifest names the device model its image is built for, and one naming no known model is no release. `GET /releases?model=plaf103` lists one model's, and an install refuses a release for another model than the device's `409 other_model` before anything is sent.
- **Keys** are settings, `releases.project_keys` and `releases.owner_keys`; nothing is built into the server, and a server with neither keeps no release.
- **The cache** is `releases/` in the data directory: `<id>.plota` and its record `<id>.json`, the id being the manifest's SHA-256 in hex, each written to a `.part`, synced and renamed. The directory is the index.
- **Fetching** is off unless `releases.fetch`. A check reads the newest 30 releases from the GitHub API and refuses an asset larger than a manifest and a full bank without downloading it.

## Installs

An owner installs a cached release on one feeder with `POST /feeders/{id}/install`. Nothing installs without that request.

The driver is `wellfed_connect::update::push`, which the CLI also runs. On the server every step goes out on the feeder's long lane, so feeds and status reads keep working. When the session drops, as when the feeder restarts into its new bank, the driver waits for the link and reads the status from the new session. Chunks are idempotent by offset, so a dropped session resumes where the feeder says, and an offer is never sent again behind the owner's back.

One install runs at a time per server. The stream carries `install` updates (the start, progress every 32 KiB, and the end), and an install a restart cut short reads as `interrupted`.

## Stock backup and restore

The server keeps each feeder's own stock firmware, the copy promotion would otherwise destroy, and offers "Restore stock" from it ([rollback](../firmware/consumer-installation.md#rollback)).

- **The copy** comes from the feeder (`POST /feeders/{id}/stock/read` reads bank 1 over the device API while stock is still there) or from an owner's upload of the copy the installer kept.
- **It's checked** against stock 1.1.15's pinned payload and container (`422 stock_refused` otherwise), then kept under `stock/<identity>/`. A kept copy is never replaced, and every read checks the file again.
- **Promotion waits for it.** An install on a feeder idle on bank 0 is `409 stock_backup_needed` until a verified copy is kept.
- **Restore** is signed by the owner on their computer, since the server holds no signing key (`wellfed server stock restore --version <n>`), and the server runs it as an install.

## Backups, export and import

**Backups** are `VACUUM INTO` copies in `backups/`, taken on a read connection so ingest goes on, named by the second they were taken (`server-20261027T033000Z.db`), and without the sealing key. [Running the server](running.md#back-it-up) covers restoring one.

### Export and import

- **The archive** (`wellfed_formats::export`, format version 4) is a POSIX tar: `manifest.json` first, in canonical JSON with each table's row count, size and SHA-256, then a file per table, `<table>.jsonl` from `GET /export` or `<table>.csv` with `?format=csv`. A quiet server exports the same bytes each time.
- **No secrets.** Accounts go without their password hashes and feeders without the server's sealed key; sessions and tokens aren't exported.
- **Streaming.** The export reads one snapshot twice, once to count and hash each table and once to write it, so memory stays flat however large the tables. One export runs at a time (`409 export_running`).
- **Same build.** Until the first server release the archive changes with the tables, in place: an archive imports only into a server of the build that wrote it, and its version is not raised ([upgrade it](running.md#upgrade-it)).
- **`POST /import`** takes a JSON archive onto a fresh server in one transaction. Like `/setup`, it needs no login and works only while no account exists (`409 import_closed`); anything wrong rolls it all back (`422 archive_invalid`).
- **After an import** no account has a password, so `wellfed-server reset-owner` gives an owner a temporary one, and every feeder is archived until adopted again, which continues its history under its id.

## Traces, statistics and the debug log

- **Traces and statistics.** A collector beside each feeder task reads on the long lane. After each `Fed` event and at each connect it fetches the trace of every feed among the 16 highest-numbered that has none stored. It reads every statistics section the session allows at each connect and then hourly, and the food section after each `Fed` event. Reads change nothing on the feeder, so asking again is always safe.
- **What is kept:** a trace's summary and its body in the feeder's own encoding; the latest read of each statistics section; and every read of the food section, as a `food_readings` row, the only way the food beam reaches the server.
- **Routes.** `GET /feeders/{id}/feeds/{feed}/trace` and `GET /feeders/{id}/stats`, for every role.
- **The debug log.** `GET /feeders/{id}/log` is Server-Sent Events of the feeder's log lines, for owners only. The first follower opens a second session to the feeder, which closes within a second of the last follower leaving. Nothing of it is stored.

## Tests

Tests follow the layers:

| Layer | Tested against |
| --- | --- |
| Store | Real SQLite, fresh per test |
| Service | `wellfed-feeder-sim` over TCP, with a fixed clock |
| HTTP API | The server in process, through `wellfed-server-client` |
| End to end | The server against the host simulator, speaking real Noise over TCP |
| Release | The shipped binary or image as a separate process ([release check](running.md#the-release-check)) |

Every path that can request a feed has a test that retries it and proves one feed. `openapi.json` and the `.sqlx/` metadata are generated, committed and checked for staleness in `make check`, so a reviewer sees every contract change as a diff. From the beta the document only grows, and the `api_growth` test holds it to the published one ([API evolution](README.md#api-evolution)).

The service tests fix the wall clock but run timers in real time with short intervals: tokio's paused clock would jump while sqlx's SQLite worker threads are busy, and fire timeouts that never happen.

## Evidence

- **Simulation**, October 24–27, 2026: every behavior on this page is covered by the server's tests against `wellfed-feeder-sim`. The stock backup and restore ran against the simulator's banks and a synthetic stock.
- **Static analysis**, October 27, 2026: the crate rules match `src/tools/xtask/src/layers.rs`, and the permission table matches `Permission::can` in `src/server/wellfed-service/src/accounts/permissions.rs`.
