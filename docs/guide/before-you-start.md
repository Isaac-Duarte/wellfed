# Before you start

This page lists what to check before you install Wellfed, what the install erases, and what could go wrong. It is for owners deciding whether to go ahead. Nothing here touches your feeder.

> [!NOTE]
> There's no public build to install yet: no release has been published. This page describes the install as it runs on our test feeder.

## Check your feeder

Do these checks first, because the installer can only read the feeder after Reset has already erased it.

- **Model:** a PLAF103. It's the only [supported feeder](../devices/README.md), and other PETLIBRO models won't work, even ones with the same chip.
- **Firmware:** vendor firmware 1.1.15. Look it up in the vendor's app. On any other version the installer stops after the Reset, and you set the feeder up again in the vendor's app.
- **Vendor updates:** the one feeder we've tested runs the vendor's firmware from a particular place in its flash. A feeder that has taken a vendor update may not, and we haven't tried one.
- **Power:** on its mains adapter, not batteries, and not in the middle of a feed.

## What you need

| You need | Why |
| --- | --- |
| An Android 12 or newer phone that can leave your Wi-Fi briefly, or a Mac, Linux or Windows computer with Wi-Fi | It runs the install and serves the firmware. The app's route on a phone that can't hold both networks hasn't been run, and neither has the computer's, where you switch networks yourself |
| A 2.4 GHz Wi-Fi network and its password | The feeder has no other way onto your network |
| About 10 minutes | Keep the app open throughout, since your phone is the server |
| A second place to keep a file | You'll want a copy of the vendor's firmware away from your phone |

Your phone or computer must be on the same network as the feeder. Client isolation or a guest network will block it.

## Get the app on a computer

The computer app comes as one installer per system. They aren't signed yet, so each system warns you the first time you open the app. The steps below get past that warning once; after that, the app opens like any other.

Download the file for your system from a release, with its `SHA256SUMS`. If you'd like to check the download, compare its SHA-256 with the list: `shasum -a 256 <file>` on a Mac, `sha256sum <file>` on Linux, or `Get-FileHash <file>` in PowerShell on Windows.

### macOS

It needs macOS 12 or later, on Apple silicon or Intel.

1. Open `Wellfed-<version>-macos.dmg`, and drag Wellfed onto the Applications folder beside it.
2. Open Wellfed from Applications. macOS says it can't verify the app or its developer. Close that message; its buttons differ between macOS versions, and none of them opens the app.
3. Open **System Settings › Privacy & Security** (on macOS 12, **System Preferences › Security & Privacy › General**), and choose **Open Anyway** beside the line about Wellfed. Confirm with your password.

Open Anyway works on every version the app supports. On macOS 14 or earlier, Control-clicking Wellfed in Applications and choosing **Open**, then **Open** again, works too.

When Wellfed first looks for feeders, macOS asks whether it can find devices on your local network: allow it. After each update, macOS asks whether Wellfed may use its key in your keychain: choose **Always Allow**. If you deny it, the window stays blank; quit Wellfed, open it again and allow.

### Windows

It needs 64-bit Windows 10 (version 1809) or Windows 11.

1. Open `Wellfed-<version>-windows-setup.exe`.
2. If Windows says it protected your PC, choose **More info**, then **Run anyway**.

   On Windows 11 with Smart App Control on, Windows blocks an unsigned installer outright, with no Run anyway. **Windows Security › App & browser control › Smart App Control settings** shows whether it's on. The only way past it is to turn it off, and that can't be undone without resetting Windows, so weigh that first.
3. Follow the installer. It installs for your account only and asks for no administrator password.

Wellfed is then in the Start menu. To remove it, use **Settings › Apps › Installed apps**, or **Settings › Apps › Apps & features** on Windows 10.

### Linux

It needs a 64-bit PC with glibc 2.39 or later (Ubuntu 24.04, Debian 13 or Fedora 40, or newer), and a keyring such as GNOME Keyring or KWallet, which most desktops run.

- **Debian or Ubuntu:** run `sudo apt install ./wellfed_<version>_amd64.deb` in the folder you saved it to. Wellfed is then in your applications menu. Remove it with `sudo apt remove wellfed`.
- **Any other distribution:** run `chmod +x Wellfed-<version>-x86_64.AppImage`, then open the file. It doesn't add itself to the applications menu. If it says FUSE is missing, install your distribution's `fuse3` package.

### Where the app keeps its data

The app keeps a key that each feeder knows your computer by, and a list of your feeders. Removing the app leaves both, so installing it again finds the same feeders.

| System | Feeders and settings | The key |
| --- | --- | --- |
| macOS | `~/Library/Application Support/io.github.isaacduarte.wellfed` | Your login keychain |
| Windows | `%LOCALAPPDATA%\Wellfed` | `%APPDATA%\Wellfed contributors\Wellfed`, encrypted for your Windows account |
| Linux | `~/.local/share/io.github.isaacduarte.wellfed` | Your keyring |

> [!NOTE]
> If you delete the key, your feeders no longer know this computer. You'd take each one back through **Recover a feeder** at its panel, or join with an invite.

On Windows, the key's folder roams: if your account has a roaming profile, as some work and school PCs do, the key follows you to every PC you sign in to, but the feeder list stays on each. Your feeders then see those PCs as one computer. Use Wellfed from one PC per Windows account.

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

- **Observed, October 30 and 31, 2026.** The install and the return to the vendor's firmware ran on one research feeder, from the app.
- **Modeled.** The app's minimum Android version comes from its build settings, and it hasn't been tried on every version.
- **Recovered.** What the Reset hold clears on the vendor's firmware comes from analysis of that firmware.
- **Observed, November 13, 2026.** The computer installers were built and checked, and the Windows one installed, opened and uninstalled in a Windows 11 virtual machine, leaving its data. The Linux packages started in clean Debian 13 and Ubuntu 24.04 containers. Nobody has yet opened a downloaded installer on a real Mac or PC, so the warnings above are each system's documented behavior for unsigned apps.
- **Modeled.** The macOS data folder is where the app's storage plugin puts it, from its code. The Linux and Windows ones were seen in the containers and the virtual machine.
