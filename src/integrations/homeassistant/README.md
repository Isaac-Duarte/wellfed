# Home Assistant integration

This integration puts a Wellfed feeder in Home Assistant, directly. It is for owners who run Home Assistant and want feeds, the panel lock and the feeder's state in their automations. Its status is on the [roadmap](../../../docs/roadmap.md).

Each feeder is one config entry. Home Assistant pairs with the feeder as its own client, by an invite, as a second phone does, and holds one [device API](../../../docs/firmware/device-api.md) session to it over the LAN. The entities, their unique ids and the device are named by the feeder's identity key. Nothing runs on the feeder for it.

## What you get

Every state is the feeder's, read from its status and events, never assumed after a command. An entity for something the feeder's firmware does not serve is left out.

Home Assistant is a member and nothing more. An invite doesn't say its role until it is redeemed, so the bridge redeems it, and a key enrolled as anything but a member revokes itself at once and asks for a member invite. A key an owner invite enrolled could change the Wi-Fi, remove clients and install firmware whatever Home Assistant shows.

An invite is redeemed once and never sent again. When the join's answer is lost, a session with the new key says whether the feeder enrolled it, as `wellfed join` does. When that session fails too, the key may be enrolled: the flow says so, keeps the key, and asks the feeder about it first at the next try, so an enrolled key is kept rather than a second invite spent on another.

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
| Meal *HH:MM* (switches) | One per feeding plan, on or off. A plan added on the feeder adds its switch; a deleted one removes it. While the plans are not read, the switches are unavailable and none is removed. |
| Volume (number) | The speaker's volume. |
| Desiccant replaced, Play chime (buttons) | Records a fresh pack today; plays the chime. |
| Portion weight (number) | The weight of one portion, 1 to 50 g. |
| Alerts (sensor) | How many of the feeder's [alerts](../../../docs/firmware/alerts.md) are open, each listed in its `alerts` attribute with its key, kind, level and the feeder's time for it. |
| Alert (event) | Each alert as it opens (`opened`) or clears (`cleared`), with the same key both times. |

The actions:

- `wellfed.feed` takes a feeder device and 1 to 48 portions, and answers the feeder's number for the feed.
- `wellfed.cancel_feed` stops a feed: the one Home Assistant started, or the one whose number you give.

Every event the feeder records as it happens also fires `wellfed_event` on the bus: its `kind` (`fed`, `lock`, `meal_soon` and so on), `seq` and `at`, with the feed, the lock, who made the change and the plan where the event names them. Events from the history read at connect are not fired again: catching up brings the entities up to date, not automations, which should never fire on an event hours old.

The feeder decides every alert, and the bridge keeps the same tracker the Android app keeps (`wellfed_client::alerts`), so Home Assistant and the app agree on what is open. The bridge reads the table when the feeder's status shows it moved. The alerts open when Home Assistant starts show in the sensor and fire nothing, as the history does. "Can't reach the feeder" is no alert here, since the Connected sensor already says so.

## How a feed stays one feed

- A press or an action sends one request, under a fresh request id. The integration never sends it again.
- If no session is open, the feed is not sent, and you get an error that says so. Pressing again is safe. The same holds for a write that arrives while a session connects, or that waits more than 5 seconds behind another: a press reaches an idle session within milliseconds, and one that waited longer is not the feed the owner just asked for.
- If the request went out and no answer came, the feed may have run. You get an error that says so, and nothing is retried. The bridge keeps the feed and settles it by the rule the app shares (`wellfed_client::settle`): a `Fed` event that names this Home Assistant, asks for the same portions, comes after the feed went out, is not a feed it has the end of, and is the only one any of its lost feeds could take. It does so even after a reconnect, since the event cursor survives it. If the feeder's history holds no such feed and the feeder reads idle (at the next connect's backlog, or at a status asked after the answer was lost, once every event it names is read), it is reported as unknown (`still_unknown`), never as a guessed count; after 15 minutes at most in any case. While a feed is still waiting on its answer, or a lost one is neither settled nor released by the owner, a new feed is not sent (`not_sent_held`), as the app holds one on every path: the lost feed may have run, and a `Fed` event names the client, not the request. The hold never lapses on its own, and the report does not end it. The owner ends it through the lost feed's repair, which says that feeding goes again though that feed may have run; the bridge confirms it (`released`), and the store keeps when, so the hold does not come back after a restart. Deleting the entry drops the hold, and a repair that outlives the entry (`hold_dropped`) says so. A released feed's end still settles it for a week after the release, as the app counts its week from the owner putting the feed away; then it is `forgotten`. Each feed is kept from just before it goes out (`sending`), as the app writes one down before it sends it: the bridge sends it only once the integration says the store wrote the record to disk, and a write that fails, or no word within 5 seconds, leaves the feed unsent (`not_sent_not_saved`), its record dropped again. Its answer clears it (`cleared`) or makes it lost: so a feed out when Home Assistant stops is lost after the restart, and its `Fed` is never taken for another lost feed's. A store left as it is (a newer integration's, or one that could not be set aside) writes nothing, so it sends no feed until the integration is updated or the entry is added again. A known limit, on a hard kill only: a kill after a feed's answer but before its `cleared` is saved leaves a false "may have run" repair, and keeps a `Fed` of the same portions from settling another lost feed, for up to the week.
- Each feed still unsettled is kept in the entry's store (its id, portions, the time it went out, whether it was reported unknown, so a restart does not report it again, and when the owner released it), with the feed numbers this Home Assistant has the end of (answered, or a lost feed's `Fed` settled), and handed to the next session after a restart or reload, so it is settled then; a `Fed` for an answered feed never settles a lost one. The bridge decides which numbers are kept: every one since the oldest lost feed still open went out, the 64 before it, and the last 64 in any case. A lost feed whose numbers would not all fit is forgotten rather than have one dropped. The bridge announces each lost feed as a `pending` update before any update that settles it (and again after a restart while it still holds feeds back: as `pending`, or as `still_unknown` once reported, so its repair comes back), and the list of numbers as an `answered` update, so the store and the repair follow the updates in order. A reload waits up to 40 seconds for a write still out, so a feed lost just then reaches the store before the next session is handed it, and keeps the updates still waiting in their order. A store file in another shape is copied aside as `<name>.bad-<n>`, never deleted, and the entry starts empty; when the copy fails, or the store is a newer integration's, the entry starts empty and never saves over it. Either way a lost feed the store held went with it, so every feed waits, as the app's set-aside pins do, until the owner says through a repair (`store_unread`) that the bowls are checked; a store that could not be set aside or is a newer integration's asks again at each start, since nothing can be saved, and sends no feed, since none could be kept. A newer minor of the store or the entry is read as it is.
- A change to the settings, the plans or the portion weight goes on what was last read, one at a time per store. A second change while the first waits is not sent, and one the feeder finds stale is refused and the store read again.

All of this lives in the Rust bridge, so the integration, the app and the CLI share one implementation of the session.

## How it is built

- `bridge/` is the `wellfed-homeassistant` crate. It wraps `wellfed-connect` and `wellfed-client` for Python through PyO3, as the `wellfed_homeassistant` module. Status, events and write results cross as typed Python classes built from `wellfed-model`'s values. Their stub, `bridge/wellfed_homeassistant.pyi`, is generated from the module and committed; every wheel ships it, and mypy checks the integration against it.
- Each feeder's session runs on a Rust thread of its own, with a tokio runtime. It reads the status, settings, plans, time and calibration on connect, replays the event backlog into `wellfed_client::today`'s facts, and follows new events. Writes go out once, gated by the role and what the device's kind serves, one at a time per store.
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

- The session takes one of the feeder's four client slots for as long as Home Assistant runs.
- A history that started over while Home Assistant was away is a gap: what happened in between is not known, and the new history is read from its start.
- The client key's secret is stored in Home Assistant's config entries, in plain text, as Home Assistant stores other integrations' credentials.
- Meal plans are switched on and off, not added or edited.
- Mutes and dismissals are the app's alone: Home Assistant shows every alert, and an automation filters by kind or level.
- An alert that opened and cleared while Home Assistant was away fires nothing.
- The config entry is version 2. Version 1 entries, from early test builds, are not migrated: they fail to migrate, and the owner deletes them and pairs again. A version 2 entry an earlier build made for the owner's server, which Wellfed no longer has, holds no client key: it fails setup with an error that says to delete it and add the feeder directly, and its reauth and reconfigure end at once with the same advice.
- A join whose answer was lost, and that no session could check, may have enrolled its key. Setup holds the key, and the bridge checks it before the next invite for that feeder is spent (`redeem`). A setup left there names the key in a repair (`leftover_key_unjoined`), since nothing checks it after.

## Evidence

- **Modeled**, November 13, 2026: the owner's server and the integration's route through it were removed, and with them route switching, sign-in and tokens, the leftover-key checks a route move needed, and hiding write entities from a viewer's account. The bridge's tests and Home Assistant's tests ran against the simulator and a scripted bridge, the unique ids pinned by `tests/fixtures/unique_ids.json` unchanged, and an entry left from the server route was refused at setup with its translated error. Simulation only.
- **Modeled**, November 3, 2026: the bridge's tests pair by invite; replay the backlog into the last feed; feed, cancel, change the lock and settings on the generation read, refusing a second change while one waits and a stale one; settle a feed whose answer was held back, and report one that never reached the feeder as unknown; stop on a revoked key; serve a feeder of a newer API revision and name the side to update; and remove the key an owner invite enrolled. Home Assistant 2026.10.4's tests on Python 3.14, under `pytest-homeassistant-custom-component`, drive the flows (an owner invite refused among them), every entity and action, the repairs and diagnostics, against `wellfed-feeder-sim` and a scripted bridge, at 97% coverage.
- **Modeled**, November 3, 2026: `make homeassistant-wheels` cross-built every release wheel on macOS, and the musllinux aarch64 wheel installed and imported on Python 3.14 in an Alpine container.
- **Observed**, November 3, 2026, on the research feeder running consumer build `5322a455` (version 391): Home Assistant 2026.10.4 found its old key revoked, raised the reauth repair, paired again with a member invite, and ran one feed from its Feed button, which completed. The rest of the bench run (the shared event across the app, the lock and a plan, a lost answer on the feeder) has not run.
