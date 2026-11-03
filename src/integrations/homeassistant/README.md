# Home Assistant integration

This integration puts a Wellfed feeder in Home Assistant with no server. It is for owners who run Home Assistant and want feeds, the panel lock and the feeder's state in their automations. Its status is on the [roadmap](../../../docs/roadmap.md).

Home Assistant pairs with the feeder as its own client, by an invite, as a second phone does. It holds one [device API](../../../docs/firmware/device-api.md) session to the feeder over the LAN. Nothing runs on the feeder for it, and it needs no `wellfed-server`.

## What you get

Every state is the feeder's, read from its status and events, never assumed after a command. Entities marked *owner* appear only with an owner invite, and an entity for something the feeder's firmware does not serve is left out.

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
| Portion weight (number), Restart (button) | *Owner*: the weight of one portion, 1 to 50 g, and a restart. |
| Firmware (update) | *Owner*: the installed build, the newest verified release, and an install with its progress. |

The actions:

- `wellfed.feed` takes a feeder device and 1 to 48 portions, and answers the feeder's number for the feed.
- `wellfed.cancel_feed` stops a feed: the one Home Assistant started, or the one whose number you give.
- `wellfed.add_release` verifies a `.plota` file from a folder Home Assistant may read and keeps it for the Firmware entity (*owner*).

Every event the feeder records as it happens also fires `wellfed_event` on the bus: its `kind` (`fed`, `lock`, `meal_soon` and so on), `seq` and `at`, with the feed, the lock, who made the change and the plan where the event names them. Events from the history read at connect are not fired again.

## How a feed stays one feed

- A press or an action sends one request, under a fresh request id. The integration never sends it again.
- If no session is open, the feed is not sent, and you get an error that says so. Pressing again is safe.
- If the request went out and no answer came, the feed may have run. You get an error that says so, and nothing is retried. The bridge keeps the feed and settles it when a `Fed` event names this Home Assistant as its source, even one that arrives after a reconnect, since the event cursor survives it. If the feeder's history holds no such feed and the feeder is not feeding, it is reported as unknown, never as a guessed count.
- A change to the settings, the plans or the portion weight goes on what was last read, one at a time per store. A second change while the first waits is not sent, and one the feeder finds stale is refused and the store read again.

All of this lives in the Rust bridge, so the integration, the app and the CLI share one implementation of the session.

## How it is built

- `bridge/` is the `wellfed-homeassistant` crate. It wraps `wellfed-connect` and `wellfed-client` for Python through PyO3, as the `wellfed_homeassistant` module. Each feeder's session runs on a Rust thread of its own. Status, events and write results cross as typed Python classes built from `wellfed-model`'s values. Their stub, `bridge/stubs/wellfed_homeassistant/__init__.pyi`, is generated from the module and committed, and mypy checks the integration against it.
- `custom_components/wellfed/` is the integration. It holds no wire format, key derivation or feeding logic.
- The client key is X25519, one per config entry. Its secret lives in the config entry, so a restart resumes as the same client.

## Install it

1. Run `make check-homeassistant`. It builds the wheel into `target/homeassistant-wheels/` and runs the tests. It needs [uv](https://docs.astral.sh/uv/). No feeder needed.
2. Install the wheel into Home Assistant's Python environment: `pip install target/homeassistant-wheels/wellfed_homeassistant-0.1.0-*.whl`. The wheel is built for the machine you build on, so build it on (or for) the Home Assistant host.
3. Copy `custom_components/wellfed` into Home Assistant's `config/custom_components/`, and restart Home Assistant.
4. On a paired client, run `wellfed invite --role member`, or make an invite in the app. You should see a `wellfed-invite:` line. It works once, for 10 minutes.
5. In Home Assistant, add the Wellfed integration and paste the invite. Give the feeder's IP address if mDNS does not reach Home Assistant's network. You should see a Feeder device with the entities above.

## Limits

- The session takes one of the feeder's four client slots for as long as Home Assistant runs.
- Last fed starts empty after a restart. The session follows new events only, not the ring's history.
- A history that started over while Home Assistant was away is logged as a gap. Events recorded in between are not replayed.
- The client key's secret is stored in Home Assistant's config entries, in plain text, as Home Assistant stores other integrations' credentials.
- There is no release pipeline for the wheel yet, so Home Assistant cannot install it from its manifest's requirement.

## Evidence

- **Modeled**, November 3, 2026: the bridge's tests pair by invite, feed to the `Fed` event, set the lock, refuse a write with no session, and report a feed whose answer was held back as uncertain, with one feed run. Home Assistant 2026.2.3's tests, under `pytest-homeassistant-custom-component`, pair through the config flow, refuse a spent invite, and drive the button, the action and the switch, all against `wellfed-feeder-sim`.
- Not yet run against a real feeder.
