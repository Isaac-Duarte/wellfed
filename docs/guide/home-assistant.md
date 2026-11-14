# Home Assistant

This page shows you how to put your Wellfed feeder in Home Assistant: install the integration, set it up, and use what it adds. It is for owners who already run Home Assistant and have a feeder on Wellfed.

Home Assistant joins the feeder as its own client, as a second phone does, with a member invite. It talks to the feeder directly over your network, encrypted, and needs no server and no cloud. You still use the app to set the feeder up.

## Before you start

- A feeder running Wellfed, and the app paired with it as an owner, on a phone or a computer.
- Home Assistant 2026.10 or later, with [HACS](https://hacs.xyz) installed.
- The feeder on the same network as Home Assistant, and ten minutes, since an invite runs out after that.

## Install the integration

1. In HACS, open the menu, choose **Custom repositories**, and add `https://github.com/Isaac-Duarte/wellfed-homeassistant` as an **Integration**. You should see Wellfed in the list.
2. Open **Wellfed** and choose **Download**. HACS installs the integration, and Home Assistant installs its bridge, the `wellfed-homeassistant` package, at the next start.
3. Restart Home Assistant.

## Pair it directly

1. Make a **member** invite. Home Assistant joins only as a member: it can feed, change the settings, the lock, the plans and the portion weight, and read everything. It can't change the Wi-Fi, remove people, restart the feeder or install firmware. If you paste an owner invite, Home Assistant removes the key it was given and asks for a member invite.

   In the app, open **Settings**, then **People**, and tap **Invite someone**. Choose **Member**, then share the link and copy its text.

2. In Home Assistant, open **Settings**, **Devices & services**. If the feeder shows as discovered, choose **Add** on it; otherwise choose **Add integration**, and search for Wellfed.
3. Paste the invite. If Home Assistant didn't discover the feeder, also give its IP address, or leave it empty to find it on the network. You should see the feeder's device with its entities.

The invite works once. If pairing fails, make a new one rather than trying the old one again.

## What you get

Every state comes from the feeder itself, never from the last button you pressed.

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
| **Portion weight** | What one portion weighs, 1 to 50 g, for portions counted in grams. |
| **Alerts** sensor | How many of the feeder's alerts are open, each listed in its attributes with its kind and level. |
| **Alert** event | Fires as each alert opens or clears, with the same key both times. |

Home Assistant also keeps the feeder's clock from staying lost. When the feeder says it needs the time, after a power cut or with no time server, Home Assistant gives it its own time and the time zone set in Home Assistant's settings, trying again every 5 minutes until it takes. If the feeder asks your router or a time server, Home Assistant first gives that 2 minutes to answer. A meal held by a lost clock still runs if the time comes back within 15 minutes of it. Home Assistant can't keep a clock from running out in the first place, since only an owner's time does that ([when something is down](everyday-use.md#when-something-is-down)).

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

### Example: tell me when the feeder needs help

The feeder decides what counts as an alert: a feeder that can't feed, food running low, a meal that failed, a battery below its floor. This automation sends each one to your phone through Home Assistant's own app.

```yaml
automation:
  - alias: Feeder alert
    trigger:
      - platform: state
        entity_id: event.kitchen_feeder_alert
    condition:
      - condition: template
        value_template: "{{ trigger.to_state.attributes.event_type == 'opened' }}"
    action:
      - action: notify.mobile_app_your_phone
        data:
          title: Feeder
          message: "{{ trigger.to_state.attributes.kind | replace('_', ' ') }}"
```

Alerts that were already open when Home Assistant started show in the Alerts sensor but don't fire the event, and an alert that opened and cleared while Home Assistant was down fires nothing.

## How it stays current

The feeder pushes each event as it happens, and Home Assistant reads the feeder's status again every 30 seconds. When the connection drops, it reconnects every 10 seconds and catches up on the events it missed, so a feed that ended meanwhile still shows.

Catching up brings the entities up to date, not your automations: the events it reads back don't fire `wellfed_event` again. That is on purpose, so an automation never runs on something that happened hours ago. A button pressed while Home Assistant is still connecting, or one that waits more than a few seconds behind another change, isn't sent: you get an error, and you can press it again.

It supports the PETLIBRO Granary PLAF103 running Wellfed, the one feeder Wellfed runs on ([supported devices](../devices/README.md)). It does not work with the vendor's firmware.

Known limits:

- Home Assistant holds one of the feeder's four client connections while it runs.
- Meal plans can be switched on and off here, but you add and edit them in the app.
- Cancel feed stops only a feed Home Assistant started; `wellfed.cancel_feed` with a feed number stops any.

## When a feed's answer is lost

Home Assistant sends a feed once and never sends it again on its own. If the answer doesn't arrive, you get an error that says the feed may have run, and a repair appears under **Settings**, **Repairs**. It clears itself once the feeder's history shows the feed, even if Home Assistant restarts in between. Until then a new feed is not sent. If the history holds no record of it after 15 minutes, or once the feeder is idle, the repair says so: check the bowl before you feed again. Home Assistant feeds again from then, and keeps watching for that feed's record for a week from when it was sent, so a record that turns up late still clears the repair. (The app keeps watching for a week from when you put the feed away.)

## Firmware updates

Home Assistant doesn't install firmware: that takes an owner, and Home Assistant is a member. Update the feeder from the app.

## Remove it

1. In **Devices & services**, open Wellfed, and **Delete** the feeder's entry.
2. In the app, open **Settings**, then **People**, and remove Home Assistant's key (listed as `home-assistant`). Deleting the entry doesn't remove the key from the feeder.
3. Optionally remove the integration in HACS.

## Common mistakes

- **"Migration failed" on an entry.** The entry was made by an early test build. It can't be carried over: delete the entry and pair the feeder again.
- **"This entry reached the feeder through a Wellfed server".** An earlier version of the integration could reach a feeder through the Wellfed server, which Wellfed no longer has. Delete the entry and pair the feeder directly with a member invite. The entities come back under the same names, but the feeder's device is a new one, so pick it again in any automation that names the device, such as one that calls `wellfed.feed`.
- **The invite ran out.** An invite lasts 10 minutes, and works once. Make a new one.
- **Home Assistant doesn't find the feeder.** Discovery uses mDNS, which doesn't cross a Docker bridge network. Run Home Assistant with host networking, or give the feeder's IP address when you pair. If the feeder's address changes, choose **Reconfigure** on its entry.
- **"The feeder no longer knows this Home Assistant".** Its key was revoked. Home Assistant asks you to pair again: make a new invite and paste it.
- **"Feeder runs older firmware" or "Update the Wellfed integration".** The feeder and the integration are different releases. Everything both know keeps working: what an older feeder lacks is refused, and what a newer one added shows as "Unknown to this integration". Update the side the repair names: the feeder from the Wellfed app, or the integration in HACS. The repair clears once they match.
- **"Feeder needs a firmware update".** The feeder runs firmware from before this integration's device API, so Home Assistant can't read or change anything on it. Update the feeder from the Wellfed app. Home Assistant tries again on its own and picks the feeder up once it answers.
- **Desiccant replaced is refused.** The feeder needs its clock set first. The app sets it when it connects.

## Next steps

- [Everyday use](everyday-use.md): meals, buttons and lights on the feeder.
- [Who can talk to your feeder](how-it-works/access.md): what members and owners may do.
- [How feeding stays safe](how-it-works/feeding-safety.md): why a feed never runs twice.

## Evidence

- **Observed, November 3, 2026.** On one research feeder, Home Assistant paired again after its key was revoked, and fed once from its Feed button.
- **Modeled, November 2026.** Pairing, discovery, every entity and action, repairs, re-pairing and refusing an owner invite ran against the simulated feeder, which runs the feeder's own pairing, stores and feeding code. The [roadmap](../roadmap.md) tracks what has run on a real feeder.
- **Modeled, November 2026.** Against the simulated feeder, the Alerts sensor listed what was open at start without firing the Alert event, and the event fired once for a client that joined after. Clearing was tested in the bridge's own tests and against a scripted bridge. Simulation only.
- **Modeled, November 2026.** A lost feed kept across a Home Assistant restart was settled afterwards, and a key a join may have left behind was named in a repair. Simulation only.
