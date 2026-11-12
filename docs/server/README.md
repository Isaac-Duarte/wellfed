# The server

The Wellfed server is an optional program you run at home, on a small Linux machine or in a container. It talks to your feeders for you and gives the household one place to see and control them. This page covers what it adds, how it fits beside the feeder, and where to read more.

## What it adds

The feeder doesn't need it: the feeder keeps its own journal, history and schedules, the app alone is the complete product, and the app and `wellfed` CLI talk to the feeder directly on your network. If the server is down, the feeder keeps feeding. The server has no web page; you use it through the app or the CLI.

It adds what a small device can't hold:

- **Remote access**, through a reverse proxy or a VPN.
- **Accounts and audit.** Each person logs in with a role (owner, member or viewer), and the audit log names who fed, changed a setting or adopted a feeder.
- **Long history.** Every event and sample, kept as long as you like, with hourly rollups for charts.
- **Owner data.** Pets, foods, weights and refills. From these and the mirrored feeds it counts portions since each refill and per pet, and lists missed, uncertain and short feeds. Once portions are calibrated (weighed in the app) it estimates the grams left and when the hopper runs out, and each pet's grams and calories against its daily target.
- **Alerts away from home.** It reads each feeder's alert table over its own session and serves it, so the app hears a feeder's alerts away from home, and it can POST each alert's opening and clearing to a webhook of yours ([alerts](#alerts)). No push service is involved.
- **Releases and installs.** It fetches or takes signed releases, verifies them, and installs one on a feeder already running Wellfed when an owner asks. Installing from stock is the app's job. It also keeps each feeder's stock backup, for going back.

One server serves one home and all its devices. Each device is stored with the kind and model its first session names; today every device is a feeder.

**This is a proof of concept.** It adopts a real feeder and keeps its history whole across a restart, a power cut and a Wi-Fi drop. Feeds, accounts, releases and installs are tested in simulation only. Status is on the [roadmap](../roadmap.md).

## Two paths, one feed

With a server configured, the app reaches a feeder two ways, and one rule keeps that safe: **an uncertain write never switches path.**

**The paths.** The app's Rust half routes through the server whenever it can reach it, so remote access works and the audit log names the person. Each owner's or member's phone also holds its own key on every feeder; with the server down it uses that key on the home network, for reads and new requests only. A viewer's phone holds no key, so only the server enforces a viewer's limits.

**Why a retry can't cross.** The server and the phone send different request ids, so a feed timed out on one path and retried on the other could feed twice. An uncertain write is resolved on its own path: from the server's journal under its key, or from the feeder's history (a feed by its `Fed` event). The CLI follows the same rule: each command takes one path, the device API or `wellfed server`, with no fallback.

**The server's half.** A write whose answer was lost gets `504 outcome_unknown` and stays unknown under its `Idempotency-Key`. A retry under that key reads the command journal and, for a feed, sends nothing. The feed settles later from its `Fed` event, readable at `GET /api/v1/commands/{key}` and pushed on the stream as a `command` event.

**The app's half.** The app sends each write once and its retry only reads. The server dedups a key only while its journal holds it, and it holds none for a write it never recorded: a `feeder_offline` answer the phone never heard, a failure before the insert, or a request that never arrived. Resending such a key after another feed would feed again. So the app reads `GET /api/v1/commands/{key}`; a `404` settles the write as never sent only from the same server and account, ten minutes after the attempt. The server keeps its side of that answer: a read that finds no write under the key retires it for a week, so a write under it that reaches the server later, held up somewhere in front of the journal, is refused `409 key_retired` and never runs behind the app's back. Those ten minutes have to pass on the phone's wall clock and on its monotonic clock, counted from when the app's current run first knew of the attempt, so a clock step or a restart never cuts them short. Anything else stays uncertain.

One answer is certain the other way: `503 feeder_offline` means the server's feeder task never took the write and dropped its journal entry. The write ran nowhere, so the app sends it on the phone's own session. A retry under a key the journal holds unknown never gets it: that retry is `outcome_unknown`.

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
| Alerts | `/feeders/{id}/alerts` | The feeder's open alerts as the server last read them, the cursor they were read under, and whether the server reaches the feeder |
| Owner data | `/pets`, `/foods`, `/pets/{id}/weights`, `/pets/{id}/feeders/{feeder}`, `/feeders/{id}/food`, `/refills`, `/pets/{id}/intake` | Pets, foods, weights, which feeder serves which pet, refills; the food left since the last refill and a pet's intake against its target |
| Firmware | `/releases`, `/releases/check`, `/feeders/{id}/install`, `/feeders/{id}/stock…` | Releases, installs, the stock backup and restore |
| Server data | `/backups`, `/export`, `/import`, `/status` | Backups, a full export, import onto a fresh server, readiness |
| Live | `/stream` | Server-Sent Events for every feeder you can see |

`GET /health` sits outside `/api/v1`. It needs no login and answers the version and the API's revision. `GET /api/v1/identity` needs no login either, and answers the server's identity key ([server identity](#server-identity)).

Some conventions hold across the API:

- **Errors** are RFC 9457 `application/problem+json`, with a typed `code`. There is no free-text error. A client reads a code it doesn't know as `unknown`, still a refusal.
- **Writes** carry an `Idempotency-Key` header (a UUID). A feed requires one.
- **Edits** carry `If-Match` with the generation of the settings, plans or calibration they change. A stale one gets `412` with the current settings, plans or calibration. A feeder whose two settings records disagree holds its settings read-only until it repairs them: a read then carries no `ETag` (`generation` reads `4294967295`, `writable` false), and an edit gets `409` with the feeder's `not_writable` refusal, whatever it names.
- **The meal call** goes up as the feeder's own stream (`PUT /feeders/{id}/call`, `application/octet-stream`), converted on the client. The server converts nothing and checks only its length, as the feeder does. It sends the clip a chunk at a time beside feeds and status reads, and answers once the feeder stores it.
  - A retry under the same key after `outcome_unknown` sends only what the feeder lacks.
  - A retry never undoes a later change. It is `409 superseded`, and sends nothing, if the feeder holds another clip than its first send found. The same goes for an upload the feeder had begun, once the feeder no longer holds its clip.
  - A delete erases only the clip it read: the feeder checks it, so an upload that lands between the read and the erase stays, and the delete is `409 superseded`.
- **The stream** resumes from `Last-Event-ID`. When that id is too old, the server sends `resync` and the client fetches again.

## Server identity

The server holds one Ed25519 key for its life, its identity key. The app pins it when you add the server, so a password or a token never goes to a machine that only claims to be your server.

- **Derived, not stored.** The key comes from the sealing key in `keys/server.key` (HMAC-SHA256 under a fixed label), so it needs no file of its own. A server restored with its sealing key keeps its identity. A server set up again with a new sealing key has a new identity, and every phone has to add it again.
- **Logged at each start.** The log names its fingerprint, the key's first 16 bytes as eight groups of four hex digits, such as `the server's identity key: 3f2a 91c0 …`. The app shows the same fingerprint under the server's address, so you can compare the two.
- **Its own HTTPS.** With `tls_listen` set, the server serves the same API over TLS 1.3 on a second port, with a certificate it signs itself with the identity key ([find it from the app](running.md#find-it-from-the-app)). The certificate names no host: no authority vouches for it, and a client checks only that the handshake is signed by the pinned key.
- **Announced.** The mDNS record names that HTTPS port and the key in its TXT `key`. Anyone on the network can announce a record, so the key in it proves nothing by itself: the app checks the server's handshake against it before the password goes.
- **Public.** `GET /api/v1/identity` answers the key on either listener, with no login. The app reads it from a server it reaches by a public name through a reverse proxy, which signs with the proxy's certificate instead.

The app pins the key in three ways:

| Server reached at | The app checks |
| --- | --- |
| An `https://` address found on the home network | That the handshake is signed by the key mDNS announced |
| An `https://` address typed for the home network | The key the first handshake proves, which it then pins |
| An `https://` public name | The system's certificate authorities, and the key `GET /identity` answers at each login |

The app speaks no plain `http://` on the network: only to the phone or computer it runs on. Every later connection to a home-network address must prove the pinned key. One that proves another fails in the handshake, before anything is sent, the token included, and the app says the key changed. It never takes the new key by itself. The only thing it offers is removing the server; you add it again once you know why the key changed. Meanwhile the phone reaches its own feeders directly, as with the server down.

The pinned key is part of the server link the app keeps in its own file. A file kept by an app from before the pin has no key, so it no longer reads: the app sets it aside with a notice, and you add the server and your feeders again.

The CLI and Home Assistant don't pin the key yet. They still reach a server on the home network by its plain `http://` address (`listen`), and a public name by `https://` through the system's authorities.

## Alerts

The feeder decides every alert ([alerts](../firmware/alerts.md)). The server reads each feeder's table over its own session whenever the status shows the table moved: a moment after each event, and every minute while the feeder is online. It keeps the latest table and what it made of it, and judges one thing itself: a feeder it has failed to reach three times in a row, over at least ten minutes, is unreachable. That is the rule the phone uses at home. A feeder that answers counts as reached, even when it turns the session away because every session is taken. One that no longer knows the server's key (an owner revoked it, or a factory reset) is unreachable at once: the server can't read it again until it is adopted anew, and the table it serves stays as old as its `read.at`. A phone that has reached the feeder itself since then opens nothing from it.

**`GET /api/v1/feeders/{id}/alerts`** answers, for any role:

```json
{
  "read": {
    "epoch": 7, "boot": 42, "generation": 3,
    "more": false, "settling": false, "unread": 0,
    "clock_valid": true,
    "at": "2026-11-09T08:00:03Z",
    "checked": "2026-11-09T08:42:03Z"
  },
  "alerts": [
    { "at": "2026-11-09T08:00:00Z",
      "alert": { "kind": "meal_failed", "feed": 12, "plan": 0, "requested": 2,
                 "completed": 0, "outcome": "failed", "reason": "jammed" } }
  ],
  "unreachable": null
}
```

`read` is `null` before the server's first read. `epoch`, `boot` and `generation` are the cursor a phone's own read carries, so the app applies the server's copy and the feeder's to one tracker and never shows an alert twice. `read.at` is when the server read this table; `read.checked` is when it last found the table unchanged, or read it, so a table that stays the same keeps its `at` while its `checked` moves on with each poll. The server keeps `checked` in memory, since an unchanged poll writes nothing; after a restart it reads as `at` until the first poll.

The app takes a copy of a higher generation in the same epoch and boot as its own last read whatever the clocks say, and never one of a lower generation. Across boots or epochs, where cursors can't say which table is newer (the boot count starts again after a power loss), it takes a copy only when `read.checked` is at least 5 minutes after its own last contact with the feeder. It takes none while `unreachable` is set. The margin assumes both clocks are kept by NTP, and covers them disagreeing by up to 5 minutes:

- **A server clock more than 5 minutes ahead** of the phone's can let a stale copy count.
- **A server clock behind, or a phone clock ahead,** delays a fresh copy by as much, until the server's later polls pass the margin. More than 5 minutes the same way can also hide a "can't reach" the server judged after the phone last reached the feeder, since the phone then seems to have reached it after.

An entry a client can't read, a kind a newer feeder added, is left out and counted in `unread`. `unreachable` is since when the server hasn't reached the feeder. The stream's `alerts` update says the table or the reach changed.

### Alert webhook

Set `webhook.url` ([settings](running.md#settings)) and the server POSTs a small JSON body to it each time an alert opens or clears:

```json
{
  "version": 1,
  "id": "3f0c9a1e7b2d4c5f8a6e1d2c3b4a5f60",
  "event": "opened",
  "time": "2026-11-09T08:00:05Z",
  "feeder": { "id": "01920000-0000-7000-8000-000000000001", "name": "Kitchen" },
  "alert": {
    "key": "7:feed:12",
    "kind": "meal_failed",
    "level": "urgent",
    "words": "A meal failed: 0 of 2 portions dispensed.",
    "at": "2026-11-09T08:00:00Z"
  }
}
```

- **`event`** is `opened` or `cleared`. **`key`** is the same in both, so a receiver pairs a clear with its open.
- **`kind`** is one of the feeder's alert kinds, or `unreachable`, the server's own. **`level`** is `urgent`, `soon` or `security`.
- **`words`** is one plain English sentence. **`at`** is the feeder's time for the alert, `null` without a valid clock; **`time`** is when the server saw the change: for an opening, when it judged the alert open; for a clear, when it first tried to send it, which an earlier change still being retried can delay.
- **The body only grows.** A newer server adds fields and never changes these; `version` moves only if a field's meaning must.

Delivery is at least once, under one `id` per change:

- **Retries.** A delivery that fails (no answer, `408`, `429` or a `5xx`) is tried again after 10 s, 30 s, 90 s, 4.5 min and 13.5 min, then given up; any other answer is final.
- **Duplicates.** Every try of one change carries the same `id`. A try whose answer was lost, or a server stopped between a delivery and noting it, sends the change again, so a receiver that must not act twice keeps the ids it has seen.
- **Pairing.** Pair events by `key`: for each key, the latest event is the alert's state. An alert whose kind changes while it is open (an uncertain meal found failed) comes as `opened` again under the same key, with a new `id`.
- **Order.** A feeder's next change waits behind one that is being retried, so a clear never comes before its open. Delivery runs on its own, never in a feeder's session, and a stopped server resumes the deliveries it hadn't finished at its next start.
- **Clears.** A clear goes only for an alert whose opening the receiver took with a `2xx`, or one already open when the server first read the feeder. An opening refused or given up is never followed by its clear, though the clear of an earlier opening the receiver took under the same key still goes (an alert whose new kind was refused). A change made while a delivery is on its way (the alert cleared and opened again, or its kind changed) is sent after it, never taken for it.
- **What isn't sent.** The alerts open when the server first reads a feeder are taken quietly: just adopted, after an import (kept alerts aren't exported), or after the server's copy was reset (a row of another layout). So are alerts open while no webhook is set: an alert already open when you set the webhook isn't sent until it opens again. A feeder you remove is sent nothing more, not even the clears of what was open: its kept alerts are dropped as it is archived, and again when it is adopted anew, so it starts afresh.

The URL may be `https://` anywhere, checked by the server's own TLS stack. Plain `http://` goes only to this computer or the home network (a loopback, private or link-local address, `localhost`, or a `.local` name), unless you set `webhook.allow_plain_http`. The server never logs the URL, since a webhook's path is often its secret.

**Home Assistant** needs nothing of its own. Make an automation with a webhook trigger, and put its URL in `webhook.url`:

```yaml
alias: Wellfed alert
triggers:
  - trigger: webhook
    webhook_id: wellfed-alerts
    allowed_methods: [POST]
    local_only: true
actions:
  - condition: template
    value_template: "{{ trigger.json.event == 'opened' }}"
  - action: notify.notify
    data:
      title: "{{ trigger.json.feeder.name }}"
      message: "{{ trigger.json.alert.words }}"
```

With the server and Home Assistant on one network, the URL is `http://homeassistant.local:8123/api/webhook/wellfed-alerts`. The Wellfed integration also shows each feeder's open alerts as a sensor, with an event as each opens or clears ([Home Assistant](../../src/integrations/homeassistant/README.md)).

### API evolution

The app, the CLI and Home Assistant often run another build than the server they reach. So from the beta, `/api/v1` only grows:

- No path, operation, field or enum variant is removed, renamed or retyped. No field becomes required, or stops being required.
- A body gains only optional fields, and an operation only optional query parameters.
- An enum gains variants only if its readers can cope: it has a catch-all that an older client reads a newer value as (`unknown`, or `newer` for the rotor's phase), or it lives only in entries an older client skips.
- Each growth raises the API's revision by one. `GET /health` answers it as `revision`, so a client can say which side to update: an older server lacks what a newer client asks for, and a newer server sends what an older client reads as unknown.

What an older client does with what it doesn't know:

| It meets | It reads |
| --- | --- |
| An error code it doesn't know | `unknown`: a refusal for a reason it can't name |
| A command outcome it doesn't know | `unknown`: the write may have run, so it's never sent again |
| A feeder connection state it doesn't know | `unknown`, shown as not online |
| A device's kind or model it doesn't know, or that device's status or settings | `unknown`: the device still lists, and the client asks to be updated to use it |
| A role it doesn't know | `unknown`, which grants nothing: the client offers no more than a viewer's |
| A journal state it doesn't know | `unknown`: uncertain, never sent again |
| Any other value it doesn't know, in an enum with a catch-all | `unknown` (`newer` for the rotor's phase), shown as unknown to that client |
| An event, journal entry, audit entry or statistics section it can't read | Left out of the page; the page's `next` still moves past it |
| An alert it can't read | Left out of the feeder's alerts and counted in `unread` |
| A stream update it can't read | Skipped; its id still moves the position to resume from |

A server reads a request the same way, and refuses a value it can't name (`400 invalid`) rather than acting on `unknown`.

A few enums are closed, since nothing can be added to them without changing what they mean:

| Enum | Why it's closed |
| --- | --- |
| `Weekday` | Seven days |
| `TimeFormat` | A 24- or 12-hour clock |
| `Species` | `other` already holds every species it doesn't name |
| `CallClip` | The one clip slot is empty, receiving or stored |
| `PortionWeight` | A portion is calibrated or not |
| `Readiness` | Ready or blocked; a new reason is a new `Blocked` value, which has a catch-all |
| `TraceRecord` | A feed's trace is stored or missing |
| `TraceMotor` | The motor driver stops or drives one of two ways |
| `SamplePage` | The client names the resolution it reads |
| `TuneName`, `NewRefill` | Only in requests: a server refuses what it doesn't know |
| `CoreEvent`, `FeederEvent`, `EventBody` | Events go only in pages and stream updates, which skip what they can't read. A catch-all on either half of the untagged body would swallow the other half's events |
| `StreamEvent`, `Stats`, `StatsSection`, `SampleValue` | Read only in entries a client skips, or by a metric it names |
| `Alert` | Read only in a feeder's alerts, which leave out one a client can't read and count it as unread |

The `api_growth` test in `wellfed-server` checks each build's document against the published one in `tests/fixtures/api-v1.json`. A change in place fails it, and no blessing stores one. A growth fails it until the revision is raised and the fixture blessed (`WELLFED_BLESS=1`).

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
- **Simulation**, November 9, 2026 (later): against a test receiver, an opening answered `400`, or given up after six tries, was never followed by its clear, while one taken was cleared once; a change gone before its retry was forgotten; a removed feeder was sent nothing more. The server's first table was taken quietly, a server copy older than the phone's own read ended and revived nothing, the kept row's format 1 read back as its committed sample, and no error quoted the webhook URL.
- **Simulation**, November 9, 2026: against `wellfed-feeder-sim`, the server served a feeder's alert table, POSTed a new client's joining to a webhook once, and judged a stopped feeder unreachable after ten minutes on a moved clock and reachable again once it answered, each sent once. The app read the same alert through the server away and directly at home, and posted it once. None of this has run on a feeder or a phone.
- **Simulation**, November 8, 2026: the growth check refused a new required field even when blessed, and asked for revision 2 for a new optional one. An older client's reads of a newer value, outcome, error code, page entry and stream update ran as unit tests.
