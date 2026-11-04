# The server

The Wellfed server is an optional program you run at home, on a small Linux machine or in a container. It talks to your feeders on their behalf and gives everyone in the household one place to see and control them. This page says what it adds, how it fits beside the feeder, and where to read more.

## What it is, and what it isn't

The feeder doesn't need the server, and the app alone is the complete product. The feeder keeps its own feed journal, event history and schedules, and the app and the `wellfed` CLI can talk to it directly on your home network. If the server is down, the feeder keeps feeding. The server has no web page of its own: you use it through the `wellfed` CLI today, and through the app once the app connects to it.

The server adds what a small device can't hold:

- **Remote access.** Reach your feeders from outside the home, through a reverse proxy or a VPN.
- **Accounts and audit.** Each person logs in as themselves, with a role (owner, member or viewer). The audit log names who fed, changed a setting or adopted a feeder.
- **Long history.** Every event and sample the feeder reports, kept for as long as you like, with hourly rollups for charts.
- **Owner data.** Pets, foods, weights and refills, which the feeder doesn't track. From them and the feeds it mirrors, the server counts the portions since each refill and each pet's portions, and lists the feeds that were missed, uncertain or short. Once a feeder's portions are calibrated (weighed in the app), it also estimates the grams its hopper has left and when it runs out, and what each pet ate in grams and calories against its daily target.
- **Releases and installs.** It fetches or takes signed firmware releases, verifies them, and installs one on a feeder running Wellfed when an owner asks. It does not install from stock: the app does that. It also keeps each feeder's stock firmware backup, so you can go back to stock.

One server serves one household, with as many feeders as you own.

**This is heavy work in progress, a proof of concept.** The server adopts a real feeder and keeps its history whole across a restart, a power cut and a Wi-Fi drop. Feeds, accounts, releases and release installs are tested in simulation only. The [roadmap](../roadmap.md) lists it as Partial.

## Two paths, one feed

The app's screens don't connect to a server yet. Its Rust half has the router that will carry them, which follows this design and is tested in simulation only. When a server is configured, the app talks to it whenever it can reach it. That way remote access works and the audit log names the person. Each owner's or member's phone also holds its own key on every feeder. When the server is down, the phone uses that key on the home network, for reads and new requests only. A viewer's phone holds no key, so the server alone enforces a viewer's limits.

The danger is a retry that crosses paths. The server and the phone send different request ids, so a feed that timed out on one path and was retried on the other could feed twice. So **an uncertain write never switches path.** It is resolved on its own path only, from the server's journal under its key or from the feeder's history (a feed by its `Fed` event). It is never resent on the other path. The CLI follows the same rule: each command takes one path, the device API or `wellfed server`, with no fallback between them.

The server enforces its half:

1. A write whose answer was lost is answered `504 outcome_unknown` and kept unknown under its `Idempotency-Key`.
2. A retry under that key reads the server's command journal. For a feed it sends nothing new.
3. The feed is settled later from its `Fed` event. You can read the result at `GET /api/v1/commands/{key}`, and it arrives on the live stream as a `command` event.

So a client can resolve a lost feed on either path without resending it. Holding an uncertain write to its own path is the app's half.

The app goes one step further: it sends each write once, and its retry only reads. The server dedups a key only while its journal holds it. It holds none for a write it never recorded: a `feeder_offline` answer the phone never heard, a failure before the insert, or a request that never reached it. Resending such a key after you fed again would be a new feed. So the app reads `GET /api/v1/commands/{key}`. A `404` leaves the write uncertain, however old it is. Restoring a server backup can lose a completed write while keeping the same account and phone token. A new feed stays on hold until you check the bowl and put the uncertain write away.

One answer is certain the other way. An attempt answered `503 feeder_offline` never reached the feeder: the server's feeder task never took it, and the server drops its journal entry. The write has run nowhere, so the app sends it on the phone's own session.

This costs the feeder a client slot per phone. A feeder has 8 slots: the server, the CLI and up to six phones.

A phone that pairs with a feeder before there is a server keeps that key. It claims the key with `POST /feeders/{id}/phones`, or names it when it adopts the feeder (`phone_key` in `POST /feeders`). The server checks that the feeder lists the key, then records it as that account's phone key. So deleting, demoting or putting an expiry on the account revokes it, as it revokes a key the server invited.

- **A refused claim leaves an adoption standing.** If the feeder doesn't list the key (`409 phone_key_not_listed`) or couldn't be read (`409 phone_key_unconfirmed`), the feeder is still adopted. Claim the key again once it is online.
- **A claim asked again is answered its record,** so a lost answer is safe to retry.
- **Only an owner claims a new key.** It takes a phone's token, with no expiry on the account or the token. The feeder can't prove which phone holds a key, so a claim trusts the account to name its own. A member who claimed someone else's key could then revoke it, so a member's claim of a key the server hasn't recorded is refused `403 phone_claim_owners_only`.
- **A member's phone that paired before the server** asks for an invite (`POST /feeders/{id}/invites`) and joins with a new key. Its old key shows as unowned, for an owner to revoke.
- **What a claim can't take:** the server's own key, a client an owner kept, a key the feeder lists as a server's or an update signer's, and another phone's key.

Every device operation the app can make has a server route, or a stated reason it has none, in `wellfed_service::parity`. It is an exhaustive match over the device API's operations, so a new operation doesn't build until it gets one. Some routes need the feeder's owner role: Wi-Fi, reboot, revoking a client, a phone's invite, the time zone rule, changing the portion calibration, uploading or erasing the meal call, installs and the stock backup. They work only when the server was adopted with an owner's invite. With the `server` role, the feeder refuses them `forbidden`.

**Adopt with an owner's invite.** That is the supported setup, since it is the only one where every route works. It gives the server the feeder's full authority, as your own CLI has: it can change Wi-Fi, revoke clients and offer updates, including downgrades and update key changes. The feeder still checks the signature on every image. So protect the server's data directory as you would your owner key.

## The HTTP API

The API lives under `/api/v1`, as JSON. The complete reference is [`openapi.json`](../../src/server/wellfed-server/openapi.json), which is generated from the handlers and checked for staleness in `make check`.

| Group | Routes | What they do |
| --- | --- | --- |
| Setup and accounts | `/setup`, `/session`, `/users`, `/tokens`, `/household` | The first owner, logins, people and their roles, phone tokens, the household's name, units and zone |
| Feeders | `/feeders`, `/feeders/{id}`, `/feeders/{id}/settings`, `/plans`, `/calibration`, `/call`, `/call/play`, `/lock`, `/desiccant/reset`, `/wifi`, `/reboot`, `/play`, `/clients`, `/invites`, `/phones` | Adopt and remove feeders, live status and the next meal, settings, feeding plans, the portion calibration, the meal call and administration |
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
- **The meal call** goes up as the feeder's own stream (`PUT /feeders/{id}/call`, `application/octet-stream`), converted on the client. The server converts nothing and checks only its length, as the feeder does. It sends the clip a chunk at a time beside feeds and status reads, and answers once the feeder stores it. A retry under the same key after `outcome_unknown` sends only what the feeder lacks. A retry never undoes a later change. It is `409 superseded`, and sends nothing, when it finds the feeder holding another clip than its first send found, or when it is an upload the feeder had begun and the feeder no longer holds its clip.
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
