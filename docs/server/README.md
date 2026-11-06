# The server

The Wellfed server is an optional program you run at home, on a small Linux machine or in a container. It talks to your feeders for you and gives the household one place to see and control them. This page covers what it adds, how it fits beside the feeder, and where to read more.

## What it adds

The feeder doesn't need it: the feeder keeps its own journal, history and schedules, the app alone is the complete product, and the app and `wellfed` CLI talk to the feeder directly on your network. If the server is down, the feeder keeps feeding. The server has no web page; you use it through the app or the CLI.

It adds what a small device can't hold:

- **Remote access**, through a reverse proxy or a VPN.
- **Accounts and audit.** Each person logs in with a role (owner, member or viewer), and the audit log names who fed, changed a setting or adopted a feeder.
- **Long history.** Every event and sample, kept as long as you like, with hourly rollups for charts.
- **Owner data.** Pets, foods, weights and refills. From these and the mirrored feeds it counts portions since each refill and per pet, and lists missed, uncertain and short feeds. Once portions are calibrated (weighed in the app) it estimates the grams left and when the hopper runs out, and each pet's grams and calories against its daily target.
- **Releases and installs.** It fetches or takes signed releases, verifies them, and installs one on a feeder already running Wellfed when an owner asks. Installing from stock is the app's job. It also keeps each feeder's stock backup, for going back.

One server serves one home and all its devices. Each device is stored with the kind and model its first session names; today every device is a feeder.

**This is a proof of concept.** It adopts a real feeder and keeps its history whole across a restart, a power cut and a Wi-Fi drop. Feeds, accounts, releases and installs are tested in simulation only. Status is on the [roadmap](../roadmap.md).

## Two paths, one feed

With a server configured, the app reaches a feeder two ways, and one rule keeps that safe: **an uncertain write never switches path.**

**The paths.** The app's Rust half routes through the server whenever it can reach it, so remote access works and the audit log names the person. Each owner's or member's phone also holds its own key on every feeder; with the server down it uses that key on the home network, for reads and new requests only. A viewer's phone holds no key, so only the server enforces a viewer's limits.

**Why a retry can't cross.** The server and the phone send different request ids, so a feed timed out on one path and retried on the other could feed twice. An uncertain write is resolved on its own path: from the server's journal under its key, or from the feeder's history (a feed by its `Fed` event). The CLI follows the same rule: each command takes one path, the device API or `wellfed server`, with no fallback.

**The server's half.** A write whose answer was lost gets `504 outcome_unknown` and stays unknown under its `Idempotency-Key`. A retry under that key reads the command journal and, for a feed, sends nothing. The feed settles later from its `Fed` event, readable at `GET /api/v1/commands/{key}` and pushed on the stream as a `command` event.

**The app's half.** The app sends each write once and its retry only reads. The server dedups a key only while its journal holds it. It holds none for a write it never recorded: a lost `feeder_offline` answer, a failure before the insert, or a request that never arrived. Resending such a key after another feed would feed again. So the app reads `GET /api/v1/commands/{key}`. A `404` leaves the write uncertain, however old it is. Restoring a server backup can lose a completed write while keeping the same account and phone token. A new feed stays on hold until you check the bowl and put the uncertain write away.

One answer is certain the other way: `503 feeder_offline` means the server's feeder task never took the write and dropped its journal entry. The write ran nowhere, so the app sends it on the phone's own session.

**Phone keys.** The second path costs a client slot per phone. A feeder has 8: the server, the CLI and up to six phones. A phone that paired before the server existed claims its key on the server, which records it as that account's key, so deleting, demoting or expiring the account revokes it, like an invited key. Claim rules are in [server design](design.md#phone-keys).

### Adopt with an owner's invite

Every device operation the app can make has a server route or a stated reason it has none. Some need the feeder's owner role: Wi-Fi, reboot, revoking a client, a phone's invite, the time zone rule, the portion calibration, uploading or erasing the meal call, installs and the stock backup. With only the `server` role the feeder refuses them `forbidden`.

So adopt with an owner's invite; it is the only setup where every route works. That gives the server your CLI's full authority, including Wi-Fi changes, revoking clients, and offering updates, downgrades and update-key changes. The feeder still checks every image's signature.

> [!WARNING]
> An owner-adopted server holds your feeder's full authority. Protect its data directory as you would your owner key.

## The HTTP API

The API lives under `/api/v1`, as JSON. The complete reference is [`openapi.json`](../../src/server/wellfed-server/openapi.json), which is generated from the handlers and checked for staleness in `make check`.

| Group | Routes | What they do |
| --- | --- | --- |
| Setup and accounts | `/setup`, `/session`, `/users`, `/tokens`, `/home` | The first owner, logins, people and their roles, phone tokens, and the home: its id, name, units and zone |
| Feeders | `/feeders`, `/feeders/{id}`, `/feeders/{id}/settings`, `/plans`, `/meals/skip`, `/meals/unskip`, `/calibration`, `/call`, `/call/play`, `/lock`, `/desiccant/reset`, `/wifi`, `/reboot`, `/play`, `/clients`, `/invites`, `/phones` | Adopt and remove devices (each listed with its home, kind and model), live status and the next meal, settings, feeding plans and a skip of one of today's meals, the portion calibration, the meal call and administration |
| Feeding | `/feeders/{id}/feed`, `/feeds/{feed}/cancel`, `/commands`, `/commands/{key}` | Feed, cancel, and the journal of every write |
| History | `/feeders/{id}/events`, `/samples`, `/feeds/{feed}/trace`, `/stats`, `/log`, `/audit` | Events, samples, feed traces, statistics, the live debug log and the audit log |
| Owner data | `/pets`, `/foods`, `/pets/{id}/weights`, `/pets/{id}/feeders/{feeder}`, `/feeders/{id}/food`, `/refills`, `/pets/{id}/intake` | Pets, foods, weights, which feeder serves which pet, refills; the food left since the last refill and a pet's intake against its target |
| Firmware | `/releases`, `/releases/check`, `/feeders/{id}/install`, `/feeders/{id}/stock…` | Releases, installs, the stock backup and restore |
| Server data | `/backups`, `/export`, `/import`, `/status` | Backups, a full export, import onto a fresh server, readiness |
| Live | `/stream` | Server-Sent Events for every feeder you can see |

`GET /health` sits outside `/api/v1`. It needs no login and answers the version.

Some conventions hold across the API:

- **Errors** are RFC 9457 `application/problem+json`, with a typed `code` from a closed set. There is no free-text error.
- **Writes** carry an `Idempotency-Key` header (a UUID). A feed requires one.
- **Edits** carry `If-Match` with the generation of the settings, plans or calibration they change. A stale one gets `412` with the current settings, plans or calibration.
- **The meal call** goes up as the feeder's own stream (`PUT /feeders/{id}/call`, `application/octet-stream`), converted on the client. The server converts nothing and checks only its length, as the feeder does. It sends the clip a chunk at a time beside feeds and status reads, and answers once the feeder stores it.
  - A retry under the same key after `outcome_unknown` sends only what the feeder lacks.
  - A retry never undoes a later change. It is `409 superseded`, and sends nothing, if the feeder holds another clip than its first send found. The same goes for an upload the feeder had begun, once the feeder no longer holds its clip.
- **The stream** resumes from `Last-Event-ID`. When that id is too old, the server sends `resync` and the client fetches again.

## Read next

| I want to… | Read |
| --- | --- |
| Install and run the server | [Running the server](running.md) |
| Understand how it's built, or change it | [Server design](design.md) |
| Know what the feeder itself serves | [Device API](../firmware/device-api.md) |
| See the product the server is part of | [Product architecture](../firmware/product-architecture.md) |
| Check what works today | [Roadmap](../roadmap.md) |

## Evidence

- **Simulation**, October 24–27, 2026: the server's crates and their tests run against `wellfed-feeder-sim` over real TCP and Noise. Feeds, uncertain writes, installs, the stock backup and restore, export and import were exercised this way.
- **Observed**, October 28, 2026: the server adopted the research unit by invite, mirrored its history, and changed its settings and lock. The history stayed whole across a server restart, a power cut and a Wi-Fi drop.
- **Static analysis**, October 27, 2026: the route groups above match the committed `openapi.json`, and the role rules match `Permission::can` in `wellfed-service`.
