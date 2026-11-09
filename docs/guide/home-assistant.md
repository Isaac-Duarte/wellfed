# Home Assistant

This page shows you how to put your Wellfed feeder in Home Assistant: install the integration, set it up, and use what it adds. It is for owners who already run Home Assistant and have a feeder on Wellfed.

Home Assistant reaches each feeder one of two ways, chosen when you add it:

- **Directly.** Home Assistant joins the feeder as its own client, as a second phone does, with a member invite. It talks to the feeder over your network, encrypted, and needs no server and no cloud.
- **Through your Wellfed server.** If you run the [Wellfed server](../server/README.md), Home Assistant signs in to it with an account's name and password and reaches the feeder there. The server keeps the feeder's history while Home Assistant is down.

The entities are the same either way, and you can switch a feeder to the other route later without losing its history. You still use the app or the CLI to set the feeder up.

## Before you start

- A feeder running Wellfed, and the app or the `wellfed` CLI paired with it as an owner.
- Home Assistant 2026.10 or later, with [HACS](https://hacs.xyz) installed.
- Directly: the feeder on the same network as Home Assistant, and ten minutes, since an invite runs out after that.
- Through your server: the feeder adopted on the server, and an owner or member account there.

## Install the integration

1. In HACS, open the menu, choose **Custom repositories**, and add `https://github.com/Isaac-Duarte/wellfed-homeassistant` as an **Integration**. You should see Wellfed in the list.
2. Open **Wellfed** and choose **Download**. HACS installs the integration, and Home Assistant installs its bridge, the `wellfed-homeassistant` package, at the next start.
3. Restart Home Assistant.

## Pair it directly

1. Make a **member** invite. Home Assistant joins only as a member: it can feed, change the settings, the lock, the plans and the portion weight, and read everything. It can't change the Wi-Fi, remove people, restart the feeder or install firmware. If you paste an owner invite, Home Assistant removes the key it was given and asks for a member invite.

<!-- tabs -->

### App

Open **Settings**, then **People**, and tap **Invite someone**. Choose **Member**, then share the link and copy its text.

### CLI

Run `wellfed invite --role member`. It prints the invite.

<!-- /tabs -->

2. In Home Assistant, open **Settings**, **Devices & services**. If the feeder shows as discovered, choose **Add** on it; otherwise choose **Add integration**, search for Wellfed, and choose **Directly, with a member invite**.
3. Paste the invite. If Home Assistant didn't discover the feeder, also give its IP address, or leave it empty to find it on the network. You should see the feeder's device with its entities.

The invite works once. If pairing fails, make a new one rather than trying the old one again.

## Set it up through your server

1. In Home Assistant, open **Settings**, **Devices & services**, choose **Add integration**, search for Wellfed, and choose **Through my Wellfed server**.
2. Give the server's address, such as `https://wellfed.example.com`, and the name and password of an owner or member account. Home Assistant uses the password once: it gets a token of its own, which the server lists as `home-assistant`, and keeps only that. A viewer account is refused, since it can't change anything.
3. Choose the feeder. You should see the feeder's device with its entities. Add each further feeder the same way: each is its own entry.

> [!NOTE]
> Home Assistant checks the server's certificate against the certificates its own system trusts. A server with a self-signed certificate is refused unless its certificate authority is installed on the Home Assistant host. Plain `http://` works, but the password and the token then cross your network unencrypted, so Home Assistant asks you to confirm such an address unless you give it as `localhost` or a loopback address such as `127.0.0.1`.

## Switch to the other route

Choose **Reconfigure** on the feeder's entry, then the route you want. The entities, their history and your automations stay as they are.

- **To your server:** sign in as in [Set it up through your server](#set-it-up-through-your-server). The feeder must already be adopted there. Home Assistant asks the server to remove the key it held on the feeder directly. A member can't remove its own key, so if the server can't either, a repair asks you to remove `home-assistant` from the feeder's people in the app. The repair clears once the server no longer sees the key on the feeder.
- **Directly:** paste a member invite for the feeder. Home Assistant revokes its token on the server.

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

The **wellfed.feed** action feeds a feeder from an automation and answers the feeder's number for the feed. **wellfed.cancel_feed** stops a feed. Every event the feeder records also fires `wellfed_event`, which an automation can trigger on.

Through the server, Home Assistant can do what its account can. If the account is made a viewer, the buttons, switches and numbers that change the feeder are hidden and unavailable, and they come back, with their history, once the account can change things again.

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

Alerts that were already open when Home Assistant started show in the Alerts sensor but don't fire the event. If you run the Wellfed server, it can also send alerts straight to a Home Assistant webhook ([alert webhook](../server/README.md#alert-webhook)).

## How it stays current

The feeder pushes each event as it happens, directly or through the server's event stream, and Home Assistant reads the feeder's status again every 30 seconds. When the connection drops, it reconnects every 10 seconds and catches up on the events it missed, so a feed that ended meanwhile still shows. Through the server, the feeder shows as disconnected while the server can't reach it.

Catching up brings the entities up to date, not your automations: the events it reads back don't fire `wellfed_event` again. That is on purpose, so an automation never runs on something that happened hours ago. A button pressed while Home Assistant is still connecting, or one that waits more than a few seconds behind another change, isn't sent: you get an error, and you can press it again.

It supports the PETLIBRO Granary PLAF103 running Wellfed, the one feeder Wellfed runs on ([supported devices](../devices/README.md)). It does not work with the vendor's firmware.

Known limits:

- Directly, Home Assistant holds one of the feeder's four client connections while it runs. Through the server it holds none: the server does.
- Meal plans can be switched on and off here, but you add and edit them in the app or the CLI.
- Cancel feed stops only a feed Home Assistant started; `wellfed.cancel_feed` with a feed number stops any.
- Through the server, Home Assistant's token can do whatever its account can. Use a member account if Home Assistant shouldn't act as an owner. Tokens limited to fewer rights come after the beta.
- Through the server, the day (portions today, the next meal) is counted in the home's time zone, which the server keeps every feeder on. Reading the feeder's own clock settings through the server comes after the beta.
- Through the server, a feeder first set up shows its latest 2,000 events as history, not all of them.

## When a feed's answer is lost

Home Assistant sends a feed once and never sends it again on its own, on either route. If the answer doesn't arrive, you get an error that says the feed may have run, and a repair appears under **Settings**, **Repairs**. It clears itself once the feeder's history shows the feed, or, through the server, once the server learns how it ended, even if Home Assistant restarts in between. If the history holds no record of it, the repair says so: check the bowl before you feed again.

## Firmware updates

Home Assistant doesn't install firmware: that takes an owner, and Home Assistant is a member. Update the feeder from the app.

## Remove it

1. In **Devices & services**, open Wellfed, and **Delete** the feeder's entry. Through the server, this tries to revoke Home Assistant's token there. If the server can't be reached, a repair names the token's id: revoke it in the app or with the CLI.
2. Directly: on the app or with `wellfed clients`, find Home Assistant's key (listed as `home-assistant`) and revoke it. Deleting the entry doesn't remove the key from the feeder.
3. Optionally remove the integration in HACS.

## Common mistakes

- **"Migration failed" on an entry.** The entry was made by an early test build, before the two routes. It can't be carried over: delete the entry and pair the feeder again.
- **The invite ran out.** An invite lasts 10 minutes, and works once. Make a new one.
- **Home Assistant doesn't find the feeder.** Discovery uses mDNS, which doesn't cross a Docker bridge network. Run Home Assistant with host networking, or give the feeder's IP address when you pair. If the feeder's address changes, choose **Reconfigure** on its entry.
- **"The feeder no longer knows this Home Assistant".** Its key was revoked. Home Assistant asks you to pair again: make a new invite and paste it.
- **"Feeder runs older firmware" or "Update the Wellfed integration".** The feeder and the integration are different releases. Everything both know keeps working: what an older feeder lacks is refused, and what a newer one added shows as "Unknown to this integration". Update the side the repair names: the feeder from the Wellfed app, or the integration in HACS. The repair clears once they match.
- **"Feeder needs a firmware update".** The feeder runs firmware from before this integration's device API, so Home Assistant can't read or change anything on it. Update the feeder from the Wellfed app. Home Assistant tries again on its own and picks the feeder up once it answers.
- **"Update the Wellfed server".** The server is older than the feeder or the integration. Everything all three know keeps working; update the server, and the repair clears.
- **"Sign in to the Wellfed server again".** Home Assistant's token was revoked or expired, or its account was disabled. Sign in again from the repair; Home Assistant gets a new token.
- **"The Wellfed server no longer reaches the feeder".** The feeder was removed from the server, or it dropped the server's key. Adopt it on the server again. Home Assistant checks every few minutes and picks it up on its own.
- **"That address does not answer as a Wellfed server".** Check the address and port, and that it starts with `http://` or `https://`. If the server sits behind a reverse proxy, give the address the proxy serves it at.
- **Desiccant replaced is refused.** The feeder needs its clock set first. The app sets it when it connects.

## Next steps

- [Everyday use](everyday-use.md): meals, buttons and lights on the feeder.
- [Who can talk to your feeder](how-it-works/access.md): what members and owners may do.
- [How feeding stays safe](how-it-works/feeding-safety.md): why a feed never runs twice.

## Evidence

- **Observed, November 3, 2026.** On one research feeder, Home Assistant paired again after its key was revoked, and fed once from its Feed button.
- **Modeled, November 2026.** Pairing, discovery, every entity and action, repairs, re-pairing and refusing an owner invite ran against the simulated feeder, which runs the feeder's own pairing, stores and feeding code. The [roadmap](../roadmap.md) tracks what has run on a real feeder.
- **Modeled, November 2026.** The server route ran against the Wellfed server in front of the simulated feeder: setup by sign-in, a feed, switching routes both ways with the same entity ids, signing in again after a revoked token, and refusing a viewer. A feed whose answer was lost was reported uncertain, sent once, and settled from the server's journal. Simulation only; it has not run on a real feeder or a Home Assistant host.
- **Modeled, November 2026.** On both routes, the Alerts sensor listed an alert from the simulated feeder and the Alert event fired once as it opened, and not for alerts open at start. Simulation only.
- **Modeled, November 2026.** A lost feed kept across a Home Assistant restart was settled afterwards on both routes, and a token or key left behind was named in a repair that clears itself. Simulation only.
