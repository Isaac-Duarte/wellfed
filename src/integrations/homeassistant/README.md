# Home Assistant integration

This integration puts a Wellfed feeder in Home Assistant with no server. It is for owners who run Home Assistant and want feeds, the panel lock and the feeder's state in their automations. It is a proof of concept: its status is on the [roadmap](../../../docs/roadmap.md).

Home Assistant pairs with the feeder as its own client, by an invite, as a second phone does. It holds one [device API](../../../docs/firmware/device-api.md) session to the feeder over the LAN. Nothing runs on the feeder for it, and it needs no `wellfed-server`.

## What you get

| Entity | What it does |
| --- | --- |
| Feed one portion (button) | Each press sends one feed of one portion. |
| Child lock (switch) | Locks or unlocks the panel. It shows the feeder's state, not the last command. |
| Last fed (sensor) | When the last feed this session saw ended, with its portions and outcome as attributes. |
| Battery, Desiccant days left, Wi-Fi signal (sensors) | From the feeder's status, read every 30 s and after each event. Wi-Fi signal is off by default. |
| Food low (binary sensor) | The food beam's last reading. |
| Connected (binary sensor) | Whether the session is open. |

The `wellfed.feed` action takes a feeder device and 1 to 48 portions, and answers the feeder's number for the feed. Every event the feeder records also fires `wellfed_event` on the bus, with the event's JSON.

## How a feed stays one feed

- A press or an action sends one request, under a fresh request id. The integration never sends it again.
- If no session is open, the feed is not sent, and you get an error that says so. Pressing again is safe.
- If the request went out and no answer came, the feed may have run. You get an error that says so, and nothing is retried. The feeder's `Fed` event settles it, and it arrives when the session comes back, since the event cursor survives a reconnect.

All of this lives in the Rust bridge, so the integration, the app and the CLI share one implementation of the session.

## How it is built

- `bridge/` is the `wellfed-homeassistant` crate. It wraps `wellfed-connect` and `wellfed-client` for Python through PyO3, as the `wellfed_homeassistant` module. Each feeder's session runs on a Rust thread of its own. Status, events and write results cross as `wellfed-model`'s JSON.
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
