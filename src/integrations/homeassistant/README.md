# Home Assistant integration

This integration puts a Wellfed feeder in Home Assistant, directly or through the owner's server. It is for owners who run Home Assistant and want feeds, the panel lock and the feeder's state in their automations. Its status is on the [roadmap](../../../docs/roadmap.md).

Each feeder is one config entry, on one of two routes chosen at setup:

- **Directly:** Home Assistant pairs with the feeder as its own client, by an invite, as a second phone does, and holds one [device API](../../../docs/firmware/device-api.md) session to it over the LAN. It needs no `wellfed-server`.
- **Through the owner's server:** Home Assistant signs in to `wellfed-server` once, keeps a token of its own, and follows the feeder through the server's [API](../../../docs/server/README.md).

The entities, their unique ids and the device are the same on both routes, named by the feeder's identity key, so an entry can switch routes with its history. Nothing runs on the feeder for either.

## What you get

Every state is the feeder's, read from its status and events, never assumed after a command. An entity for something the feeder's firmware does not serve is left out.

Home Assistant is a member and nothing more. An invite doesn't say its role until it is redeemed, so the bridge redeems it, and a key enrolled as anything but a member revokes itself at once and asks for a member invite. A key an owner invite enrolled could change the Wi-Fi, remove clients and install firmware whatever Home Assistant shows.

| Entity | What it does |
| --- | --- |
| Feed (button) | Sends one feed of the Portions number's value. |
| Portions (number) | What the Feed button asks for, 1 to 48. Home Assistant keeps it across restarts. |
| Cancel feed (button) | Stops the feed this Home Assistant started, while it runs. |
| Feed (event) | Each feed as it ends: fed, cancelled, failed, uncertain or expired, with its portions and who asked. An uncertain feed's portions are a range. |
| Last fed, Portions today, Next meal (sensors) | The feeder's day, reckoned as the app reckons it. Portions today counts confirmed portions, with the most that may have dropped as an attribute. |
| Food, Readiness (sensors) | The food beam's last reading, and whether a feed can start now or why not. |
| Battery, Desiccant days left, Wi-Fi signal (sensors) | From the status. Wi-Fi signal is off by default. |
| Connected, Food low, Feed problem, On battery (binary sensors) | The session, the hopper, the last failed feed this boot and the supply. |
| Child lock, Sound, Lights (switches) | The panel lock and the feeder's settings. |
| Meal *HH:MM* (switches) | One per feeding plan, on or off. A plan added on the feeder adds its switch; a deleted one removes it. |
| Volume (number) | The speaker's volume. |
| Desiccant replaced, Play chime (buttons) | Records a fresh pack today; plays the chime. |
| Portion weight (number) | The weight of one portion, 1 to 50 g. |
| Alerts (sensor) | How many of the feeder's [alerts](../../../docs/firmware/alerts.md) are open, each listed in its `alerts` attribute with its key, kind, level and the feeder's time for it. |
| Alert (event) | Each alert as it opens (`opened`) or clears (`cleared`), with the same key both times. |

The actions:

- `wellfed.feed` takes a feeder device and 1 to 48 portions, and answers the feeder's number for the feed.
- `wellfed.cancel_feed` stops a feed: the one Home Assistant started, or the one whose number you give.

Every event the feeder records as it happens also fires `wellfed_event` on the bus: its `kind` (`fed`, `lock`, `meal_soon` and so on), `seq` and `at`, with the feed, the lock, who made the change and the plan where the event names them. Events from the history read at connect are not fired again: catching up brings the entities up to date, not automations, which should never fire on an event hours old.

The feeder decides every alert, and the bridge keeps the same tracker the Android app keeps (`wellfed_client::alerts`), so Home Assistant, the app and the server agree on what is open. Directly, the bridge reads the table when the feeder's status shows it moved; through the server, it reads the server's copy (`GET /feeders/{id}/alerts`) when the stream says it changed and at each connect. The alerts open when Home Assistant starts show in the sensor and fire nothing, as the history does; through a server that hasn't read the feeder yet, that is the first table it serves. Through the server every table is the server's, so any other cursor is applied, a lower boot count after a power loss included. Through the server, "can't reach the feeder" is the server's judgement and opens like any alert; the direct route has none, since the Connected sensor already says so.

Through the server, a role that writes nothing (an account made a viewer, or a role a newer server added) has every entity that changes the feeder hidden by the integration and unavailable; they are shown again once the role writes, with their unique ids and history, and a hide the user chose stays.

## How a feed stays one feed

- A press or an action sends one request, under a fresh request id (directly) or a fresh idempotency key (through the server). The integration never sends it again.
- If no session is open, the feed is not sent, and you get an error that says so. Pressing again is safe. The same holds for a write that arrives while a session connects, or that waits more than 5 seconds behind another: a press reaches an idle session within milliseconds, and one that waited longer is not the feed the owner just asked for.
- If the request went out and no answer came, the feed may have run. You get an error that says so, and nothing is retried. Directly, the bridge keeps the feed and settles it when a `Fed` event names this Home Assistant as its source, even one that arrives after a reconnect, since the event cursor survives it. If the feeder's history holds no such feed and the feeder is not feeding, it is reported as unknown, never as a guessed count.
- Through the server, the bridge keeps the feed by its key and asks the server's journal, on the stream and at each connect: the journal's `fed` outcome or the feed's `Fed` event settles it, a refusal settles it as never run (its own update kind, `settled_unfed`), and a feeder that restarted with no record of it reports it unknown. One the journal never settles is reported unknown after 15 minutes.
- Each feed still unsettled is kept in the entry's store (its key, portions and the time it went out), with the last 64 feed numbers this Home Assistant was answered for, and handed to the next session after a restart or reload, so it is settled then, within the same 15 minutes; a `Fed` for an answered feed never settles a lost one. The bridge announces each lost feed as a `pending` update before any update that settles it, so the store and the repair follow the updates in order. A store file in another shape is copied aside as `<name>.bad-<n>`, never deleted, and the entry starts empty; when the copy fails, or the store is a newer integration's, the entry starts empty and never saves over it. A newer minor of the store or the entry is read as it is.
- A change to the settings, the plans or the portion weight goes on what was last read, one at a time per store. A second change while the first waits is not sent, and one the feeder finds stale is refused and the store read again.

All of this lives in the Rust bridge, so the integration, the app and the CLI share one implementation of the session.

## How it is built

- `bridge/` is the `wellfed-homeassistant` crate. It wraps `wellfed-connect` and `wellfed-client` for Python through PyO3, as the `wellfed_homeassistant` module. Status, events and write results cross as typed Python classes built from `wellfed-model`'s values. Their stub, `bridge/wellfed_homeassistant.pyi`, is generated from the module and committed; every wheel ships it, and mypy checks the integration against it.
- Each feeder's session runs on a Rust thread of its own, with a tokio runtime. Directly, it reads the status, settings, plans, time and calibration on connect, replays the event backlog into `wellfed_client::today`'s facts, and follows new events. Writes go out once, gated by the role and what the device's kind serves, one at a time per store.
- Through the server, the session uses `wellfed-server-client`, as the app and the CLI do. It checks the server's API revision, the token's account and the feeder, opens the server's event stream, then reads the facts and the history from its cursor (the newest 2,000 events on a first connect). It hands out the same updates as the direct session, so the Python does not know the route. The token goes in the config entry, never the password: setup signs in once, creates the token, and signs out.
- `custom_components/wellfed/` is the integration. It holds no wire format, key derivation or feeding logic. A pump thread hands the bridge's updates to Home Assistant's event loop, and writes run in its executor.
- The client key is X25519, one per config entry. Its secret lives in the config entry, so a restart resumes as the same client.
- The [Python standard](../../../docs/development/python.md) has the rules and the tests; the [owner guide](../../../docs/guide/home-assistant.md) is what an owner reads.

## Run it from a checkout

1. Run `make check-homeassistant`. It builds this machine's wheel into `target/homeassistant-wheels/check/` and runs the tests. It needs [uv](https://docs.astral.sh/uv/). No feeder needed.
2. For another machine, run `make homeassistant-wheels`. It cross-builds every release wheel into `target/homeassistant-wheels/release/`: musllinux for Home Assistant's own images, manylinux for other installs.
3. Install the wheel for the Home Assistant host into its Python environment, copy `custom_components/wellfed` into its `config/custom_components/`, and restart Home Assistant.
4. Set it up as the [owner guide](../../../docs/guide/home-assistant.md#pair-it-directly) says.

## Releasing

The version is the bridge crate's (`bridge/Cargo.toml`). The wheel takes it from there, and `custom_components/wellfed/manifest.json` repeats it as its `version` and as the `wellfed-homeassistant==` requirement. A test in `bridge/tests/version.rs` fails when they disagree.

To cut a release:

1. Bump the version in `bridge/Cargo.toml` and both places in `manifest.json`, then run `make check` and `make check-homeassistant`.
2. Run `make homeassistant-wheels`. It writes every wheel and the sdist to `target/homeassistant-wheels/release/`.
3. The owner uploads them to PyPI (`uv publish target/homeassistant-wheels/release/*`), and publishes `custom_components/wellfed` with the [owner guide](../../../docs/guide/home-assistant.md) as its README to the HACS repository, `Isaac-Duarte/wellfed-homeassistant`, tagged and released `v<version>`. HACS installs from those releases.

Contributors and agents build and check releases. Only the owner publishes, creates repositories or pushes a release tag.


## Limits

- Directly, the session takes one of the feeder's four client slots for as long as Home Assistant runs.
- A history that started over while Home Assistant was away is a gap: what happened in between is not known, and the new history is read from its start.
- The client key's secret, or the server token, is stored in Home Assistant's config entries, in plain text, as Home Assistant stores other integrations' credentials.
- Meal plans are switched on and off, not added or edited.
- Mutes and dismissals are the app's alone: Home Assistant shows every alert, and an automation filters by kind or level.
- An alert that opened and cleared while Home Assistant was away fires nothing.
- Through the server, the token carries its account's whole role: the server has no tokens limited to fewer rights yet. Home Assistant refuses a viewer account at setup.
- A server address on plain `http://` carries the password and the token unencrypted; setup asks the owner to confirm one unless its host is `localhost` or a loopback address.
- A token is revoked whenever a flow ends without an entry keeping it, and when its entry is deleted. One the server could not revoke at deletion is named, by its id, in a persistent repair; it works until the owner revokes it.
- The config entry is version 2. Version 1 entries, from test builds before the two routes, are not migrated: they fail to migrate, and the owner deletes them and pairs again.
- Through the server, there is no route to read the feeder's time view, so the day is counted in the home's zone, which the server keeps every feeder on.
- Through the server, TLS checks the server's certificate against the Home Assistant host's own trusted certificates (`rustls-platform-verifier`, which reads the system store); no roots are bundled. A self-signed certificate needs its authority installed on the host.
- A member may not remove its own key from the feeder, so an entry moved from the direct route to the server asks the server to remove it, which works on an owner's account when the feeder made the server an owner. Otherwise a repair asks the owner to remove it in the app; it clears when the server no longer lists the key, checked at each connect.

## Evidence

- **Modeled**, November 3, 2026: the bridge's tests pair by invite; replay the backlog into the last feed; feed, cancel, change the lock and settings on the generation read, refusing a second change while one waits and a stale one; settle a feed whose answer was held back, and report one that never reached the feeder as unknown; stop on a revoked key; serve a feeder of a newer API revision and name the side to update; and remove the key an owner invite enrolled. Home Assistant 2026.10.4's tests on Python 3.14, under `pytest-homeassistant-custom-component`, drive the flows (an owner invite refused among them), every entity and action, the repairs and diagnostics, against `wellfed-feeder-sim` and a scripted bridge, at 97% coverage.
- **Modeled**, November 3, 2026: `make homeassistant-wheels` cross-built every release wheel on macOS, and the musllinux aarch64 wheel installed and imported on Python 3.14 in an Alpine container.
- **Modeled**, November 9, 2026: the bridge's tests against `wellfed-server` in process, in front of the simulated feeder: sign-in for a token with no session kept, a viewer and a wrong password refused, the feeder announced and a feed answered with its number, a feed whose answer was held back reported uncertain, sent once and settled from the server's journal, and a revoked token stopping the session. Home Assistant's tests run the server route against the `wellfed-server` binary: setup by sign-in, a feed, switching routes both ways with the same unique ids and the old key or token removed, and reauth after a revoked token. `make homeassistant-wheels` cross-built every release wheel on Linux, the musllinux ones with `aws-lc-sys` through zig; they were not installed on a Home Assistant host.
- **Modeled**, November 9, 2026 (later): a feed whose answer was lost, kept across a restart, settled by the next session on both routes (from the backlog directly, from the journal through the server); a write that arrived while a session connected, or waited past 5 seconds, refused unsent (unit tests); write entities hidden after a demotion to viewer and shown again; a token revoked when a flow closed at the feeder picker or was refused there; a token the server could not revoke at deletion named in a repair; a leftover key's repair cleared once the server stopped listing the key and with its entry; no reauth after a move from the direct route; a plain `http://` address confirmed first; and a version 1 entry failing to migrate and deleting cleanly. Simulation and a scripted bridge only.
- **Modeled**, November 9, 2026 (alerts). Directly, Home Assistant's tests against `wellfed-feeder-sim`: its own joining is listed and fires nothing, and a client joining after fires `opened` once and is listed. Through the server, the bridge's test against `wellfed-server` in process: what the server read before the session is listed and fires nothing, and a client joining after fires once. The tracker's unit tests: the first table seeds quietly, a new alert opens and an ended one clears under one key, a server with no table yet leaves the seeding to its first, and through the server a lower boot count after a power loss is applied. Clearing ran against a scripted bridge in Home Assistant's tests, not against the simulator. Simulation only.
- **Observed**, November 3, 2026, on the research feeder running consumer build `5322a455` (version 391): Home Assistant 2026.10.4 found its old key revoked, raised the reauth repair, paired again with a member invite, and ran one feed from its Feed button, which completed. The rest of the bench run (the shared event across the app and the server, the lock and a plan, a lost answer on the feeder) has not run.
