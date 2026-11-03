# Home Assistant

This page shows you how to put your Wellfed feeder in Home Assistant: install the integration, pair it with an invite, and use what it adds. It is for owners who already run Home Assistant and have a feeder on Wellfed.

Home Assistant joins the feeder as its own client, as a second phone does. It talks to the feeder directly over your network, encrypted, and needs no server and no cloud. You still use the app or the CLI to set the feeder up and to make the invite.

## Before you start

- A feeder running Wellfed, on the same network as Home Assistant, and the app or the `wellfed` CLI paired with it as an owner.
- Home Assistant 2026.10 or later, with [HACS](https://hacs.xyz) installed.
- Ten minutes: an invite runs out after that.

## Install the integration

1. In HACS, open the menu, choose **Custom repositories**, and add `https://github.com/Isaac-Duarte/wellfed-homeassistant` as an **Integration**. You should see Wellfed in the list.
2. Open **Wellfed** and choose **Download**. HACS installs the integration, and Home Assistant installs its bridge, the `wellfed-homeassistant` package, at the next start.
3. Restart Home Assistant.

## Pair it with the feeder

1. Decide which invite to make. A **member** invite lets Home Assistant feed, change the settings, the lock and the plans, and read everything. An **owner** invite also lets it install firmware, older firmware included, change the feeder's Wi-Fi, set the portion weight, restart the feeder and remove other clients. Make an owner invite only if you want Home Assistant to do those things.

2. Make the invite, as below.

<!-- tabs -->

### App

Open **Settings**, then **People**, and tap **Invite someone**. Choose **Member** or **Owner**, then share the link and copy its text.

### CLI

Run `wellfed invite --role member`, or `--role owner`. It prints the invite.

<!-- /tabs -->

3. In Home Assistant, open **Settings**, **Devices & services**. If the feeder shows as discovered, choose **Add** on it; otherwise choose **Add integration** and search for Wellfed.
4. Paste the invite. If Home Assistant didn't discover the feeder, also give its IP address, or leave it empty to find it on the network. You should see "Paired as member", or "Paired as owner".

The invite works once. If pairing fails, make a new one rather than trying the old one again.

## What you get

Every state comes from the feeder itself, never from the last button you pressed. Entities marked *owner* appear only after an owner invite.

| Entity | What it does |
| --- | --- |
| **Feed** button and **Portions** number | A press feeds the number of portions set, 1 to 48. |
| **Cancel feed** button | Stops a feed Home Assistant started, while it runs. |
| **Feed** event | Fires as each feed ends: fed, cancelled, failed, uncertain or expired, with the portions and who asked. |
| **Last fed**, **Portions today**, **Next meal** | The feeder's day, as the app shows it. |
| **Food**, **Readiness** | The food level, and whether a feed can start now or why not. |
| **Battery**, **Desiccant days left**, **Wi-Fi signal** | From the feeder's status. Wi-Fi signal is off until you enable it. |
| **Connected**, **Food low**, **Feed problem**, **On battery** | The connection, the hopper, the last failed feed and the power supply. |
| **Child lock**, **Sound**, **Lights** switches, **Volume** | The panel lock and the feeder's settings. |
| **Meal *HH:MM*** switches | One for each meal plan, to turn it on or off. New and deleted plans come and go on their own. |
| **Desiccant replaced**, **Play chime** buttons | Record a fresh desiccant pack today; play the chime. |
| **Portion weight**, **Restart** | *Owner*: what one portion weighs, and a restart. |
| **Firmware** update | *Owner*: the installed firmware, the newest verified release, and an install. |

The **wellfed.feed** action feeds a feeder from an automation and answers the feeder's number for the feed. **wellfed.cancel_feed** stops a feed. Every event the feeder records also fires `wellfed_event`, which an automation can trigger on.

### Example: feed at sunrise on weekends

```yaml
automation:
  - alias: Weekend breakfast at sunrise
    trigger:
      - platform: sun
        event: sunrise
    condition:
      - condition: time
        weekday: [sat, sun]
    action:
      - action: wellfed.feed
        data:
          device_id: YOUR_FEEDER_DEVICE_ID
          portions: 2
```

For meals at fixed times, use the feeder's own meal plans instead. They run on the feeder, so they keep working while Home Assistant is down.

## How it stays current

The feeder pushes each event as it happens, and Home Assistant reads the feeder's status again after each one and every 30 seconds. When the connection drops, it reconnects every 10 seconds and catches up on the events it missed, so a feed that ended meanwhile still shows. The Firmware entity looks for releases every 6 hours.

It supports the PETLIBRO Granary PLAF103 running Wellfed, the one feeder Wellfed runs on ([supported devices](../devices/README.md)). It does not work with the vendor's firmware.

Known limits:

- Home Assistant holds one of the feeder's four client connections while it runs.
- Meal plans can be switched on and off here, but you add and edit them in the app or the CLI.
- Cancel feed stops only a feed Home Assistant started; `wellfed.cancel_feed` with a feed number stops any.

## When a feed's answer is lost

Home Assistant sends a feed once and never sends it again on its own. If the feeder's answer doesn't arrive, you get an error that says the feed may have run, and a repair appears under **Settings**, **Repairs**. It clears itself once the feeder's history shows the feed. If the history holds no record of it, the repair says so: check the bowl before you feed again.

## Firmware updates

With an owner invite, the **Firmware** entity offers the newest verified release:

- **From the project's releases.** In the integration's **Configure** options, enter the project's release keys. Home Assistant checks the project's releases a few times a day.
- **From a file.** Put the `.plota` file in a folder Home Assistant may read (listed under `allowlist_external_dirs`), and call **wellfed.add_release** with its path.

Home Assistant offers only a release signed by a project key you entered or by an update key you enrolled on the feeder. Choose **Install** to send it. You see the progress, and the feeder restarts into the new firmware.

## Remove it

1. In **Devices & services**, open Wellfed, and **Delete** the feeder's entry.
2. On the app or with `wellfed clients`, find Home Assistant's key (listed as `home-assistant`) and revoke it. Deleting the entry doesn't remove the key from the feeder.
3. Optionally remove the integration in HACS.

## Common mistakes

- **The invite ran out.** An invite lasts 10 minutes, and works once. Make a new one.
- **Home Assistant doesn't find the feeder.** Discovery uses mDNS, which doesn't cross a Docker bridge network. Run Home Assistant with host networking, or give the feeder's IP address when you pair. If the feeder's address changes, choose **Reconfigure** on its entry.
- **"The feeder no longer knows this Home Assistant".** Its key was revoked. Home Assistant asks you to pair again: make a new invite and paste it.
- **Desiccant replaced is refused.** The feeder needs its clock set first. The app sets it when it connects.

## Next steps

- [Everyday use](everyday-use.md): meals, buttons and lights on the feeder.
- [Who can talk to your feeder](how-it-works/access.md): what members and owners may do.
- [How feeding stays safe](how-it-works/feeding-safety.md): why a feed never runs twice.

## Evidence

- **Modeled, November 2026.** Pairing, discovery, every entity and action, repairs, re-pairing and a firmware install ran against the simulated feeder, which runs the feeder's own pairing, stores and feeding code. None of this has run on a real feeder yet; the [roadmap](../roadmap.md) tracks that.
