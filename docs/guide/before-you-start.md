# Before you start

This page lists what to check before you install Wellfed, what the install erases, and what could go wrong. It is for owners deciding whether to go ahead. Nothing here touches your feeder.

> [!NOTE]
> There's no public build to install yet: a release needs a signing key that doesn't exist yet. This page describes the install as it runs on our test feeder.

## Check your feeder

Do these checks first, because the installer can only read the feeder after Reset has already erased it.

- **Model:** a PLAF103. It's the only [supported feeder](../devices/README.md), and other PETLIBRO models won't work, even ones with the same chip.
- **Firmware:** vendor firmware 1.1.15. Look it up in the vendor's app. On any other version the installer stops after the Reset, and you set the feeder up again in the vendor's app.
- **Vendor updates:** the one feeder we've tested runs the vendor's firmware from a particular place in its flash. A feeder that has taken a vendor update may not, and we haven't tried one.
- **Power:** on its mains adapter, not batteries, and not in the middle of a feed.

## What you need

| You need | Why |
| --- | --- |
| An Android 12 or newer phone that can leave your Wi-Fi briefly, or a Linux computer with Bluetooth and a Wi-Fi interface | It runs the install and serves the firmware. A wired-only computer can't reach the feeder's setup network. The app's route on a phone that can't hold both networks hasn't been run |
| A 2.4 GHz Wi-Fi network and its password | The feeder has no other way onto your network |
| About 10 minutes | Keep the app or the run open throughout, since your phone or computer is the server |
| A second place to keep a file | You'll want a copy of the vendor's firmware away from your phone or computer |

Your phone or computer must be on the same network as the feeder. Client isolation or a guest network will block it, and a computer's firewall must allow TCP 1883 and 8088 from the feeder.

## What the install erases

Holding Reset erases the feeder's meals, Wi-Fi and settings, its link to the vendor's app, and any feed history it hasn't sent. Your meals don't come across, so **note them down first**, and feed your pet another way until you've set them up in Wellfed.

If you stop after the Reset, the feeder has no meals until you finish the install or set it up again in the vendor's app.

## What can go wrong

| What happens | What it means |
| --- | --- |
| Someone nearby reads your Wi-Fi password | The feeder's setup network is open, and the vendor's setup protocol sends the password unencrypted. The exposure lasts a few seconds. Change the password afterward if that matters to you |
| The feeder never connects | Usually a wrong password or a network without 2.4 GHz. Nothing has been installed, and you can send the details again |
| The app closes or your phone drops out partway | The install can usually continue, and [Install](install.md#if-the-install-stops) says how. Don't unplug the feeder during an install unless the app asks you to restart it |
| Your feeder's firmware doesn't match the known 1.1.15 | The install stops before overwriting it, and the feeder stays on Wellfed. There's then no over-the-air way back to the vendor's firmware. UART is the only route left, and our UART tools are pinned to our one test feeder, so we have no way back to offer you |
| Wellfed has a bug that affects feeding | It's possible. It has run on one feeder and hasn't finished a long unattended run. Don't rely on it alone to feed a pet yet |
| The feeder won't boot at all | The update path is built so that this shouldn't happen. The last resort is [recovery over UART](../devices/plaf103/recovery.md), which means opening the feeder and using a USB serial adapter. Our tools for it are pinned to our one test feeder, so you'd have to adapt them |

[Install and rollback](how-it-works/install-and-rollback.md) explains how the install avoids the worst of these, and where it can't.

## Going back

You can put your feeder's own copy of the vendor's firmware back over the air, then set it up again in the vendor's app. That needs the backup: after the install, the feeder no longer holds a copy. [Going back to stock](going-back.md) has the steps. Don't hold Reset to go back: on a feeder running Wellfed, that wipes it.

## Next steps

- [Install](install.md)
- [Install and rollback](how-it-works/install-and-rollback.md)

## Evidence

- **Observed, October 30 and 31, 2026.** The install and the return to the vendor's firmware ran on one research feeder, from the app and the CLI.
- **Modeled.** The app's minimum Android version comes from its build settings, and it hasn't been tried on every version.
- **Recovered.** What the Reset hold clears on the vendor's firmware comes from analysis of that firmware.
