# Home Assistant integration

This integration puts a Wellfed feeder in Home Assistant with no server. It is for owners who run Home Assistant and want feeds, the panel lock and the feeder's state in their automations. Its status is on the [roadmap](../../../docs/roadmap.md).

Home Assistant pairs with the feeder as its own client, by an invite, as a second phone does. It holds one [device API](../../../docs/firmware/device-api.md) session to the feeder over the LAN. Nothing runs on the feeder for it, and it needs no `wellfed-server`.

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

The actions:

- `wellfed.feed` takes a feeder device and 1 to 48 portions, and answers the feeder's number for the feed.
- `wellfed.cancel_feed` stops a feed: the one Home Assistant started, or the one whose number you give.

Every event the feeder records as it happens also fires `wellfed_event` on the bus: its `kind` (`fed`, `lock`, `meal_soon` and so on), `seq` and `at`, with the feed, the lock, who made the change and the plan where the event names them. Events from the history read at connect are not fired again.

## How a feed stays one feed

- A press or an action sends one request, under a fresh request id. The integration never sends it again.
- If no session is open, the feed is not sent, and you get an error that says so. Pressing again is safe.
- If the request went out and no answer came, the feed may have run. You get an error that says so, and nothing is retried. The bridge keeps the feed and settles it when a `Fed` event names this Home Assistant as its source, even one that arrives after a reconnect, since the event cursor survives it. If the feeder's history holds no such feed and the feeder is not feeding, it is reported as unknown, never as a guessed count.
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
4. Pair as the [owner guide](../../../docs/guide/home-assistant.md#pair-it-with-the-feeder) says.

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

## Evidence

- **Modeled**, November 3, 2026: the bridge's tests pair by invite; replay the backlog into the last feed; feed, cancel, change the lock and settings on the generation read, refusing a second change while one waits and a stale one; settle a feed whose answer was held back, and report one that never reached the feeder as unknown; stop on a revoked key, and on a feeder of another build without sending it anything; and remove the key an owner invite enrolled. Home Assistant 2026.10.4's tests on Python 3.14, under `pytest-homeassistant-custom-component`, drive the flows (an owner invite refused among them), every entity and action, the repairs and diagnostics, against `wellfed-feeder-sim` and a scripted bridge, at 97% coverage.
- **Modeled**, November 3, 2026: `make homeassistant-wheels` cross-built every release wheel on macOS, and the musllinux aarch64 wheel installed and imported on Python 3.14 in an Alpine container.
- **Observed**, November 3, 2026, on the research feeder running consumer build `5322a455` (version 391): Home Assistant 2026.10.4 found its old key revoked, raised the reauth repair, paired again with a member invite, and ran one feed from its Feed button, which completed. The rest of the bench run (the shared event across the app and the server, the lock and a plan, a lost answer on the feeder) has not run.
