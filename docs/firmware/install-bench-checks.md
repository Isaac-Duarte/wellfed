# Bank, install and rollback bench checks

This guide runs the [bank rules](bank-policy.md), an [install from stock](consumer-installation.md) and a [rollback](consumer-installation.md#rollback) on the [relay bench](../devices/plaf103/bench.md). Each check writes flash, so each starts by checking what is about to be written. The bench rules apply: one holder, the hotspot and `wellfed-stock serve` as bounded foreground processes, and the network and services put back as you found them.

## Checking the bank rules

The tools cover each step with the bank-1 updater's checks, and the factory region stays unwritable ([UART development](../development/uart-development.md)). Each step ends with a full-flash read, compared with the one before it. The UART loader path stays available throughout.

1. **Check the plan.** Run `wellfed-bench flash-prepare --bank 0` with a reviewed replacement image. Read the plan it prints: bank 0 must get exactly the frozen `--source` image or, without `--source`, the pinned baseline with an invalidated marker, and bank 1 must stay valid throughout.
2. **Both valid.** Run `wellfed-bench flash-execute --bank 0` with that plan's SHA-256. You should see the boot report bank 1.
3. **Check the give-way.** Run `wellfed-bench give-way-prepare --fallback <bank-0 review>`. The give-way runs only while bank 0 is exactly that review, and its plan clears only bank 1's first marker byte.
4. **Fallback.** Run `give-way-execute` with that plan's SHA-256. You should see the boot report bank 0.
5. **Check the restore.** Run `wellfed-bench flash-prepare --restore-marker`. Its plan must rewrite only bank 1's first sector, marker last.
6. **Restore.** Run `flash-execute` with that plan's SHA-256. You should see the boot report bank 1 again.

## Running an install on the bench

This needs the owner at the panel and a phone. Build the APK with the image to install ([the image it installs](../../src/app/README.md#the-image-it-installs)), and clear the app first if the phone ran an earlier test.

1. **Check the start.** `wellfed-bench status` (or the feeder's status over a session) shows the running build and its bank; its "Bank N" lines are only the bench's record of what it last wrote. Check that the review you will pass is the stock 1.1.15 review. A consumer image starts Bluetooth setup only with no Wi-Fi stored, so while a bench image still runs on bank 1, clear its Wi-Fi with `wellfed wifi --clear`.
2. **Return the unit to stock** over the air: `wellfed-bench ota <name>-stock --review <stock review>`. You should see stock 1.1.15 on bank 1 and the replacement in bank 0.
3. **Check the network.** The phone must be on a network the feeder can join, and the feeder must reach the phone on the broker's and the image server's ports. Resolve any trial left open first ([the install ledger](../development/stock-server.md#the-install-ledger)).
4. **At the panel,** unlock stock (Lock about 5 s if the lock light is on). Hold nothing else yet: the app asks for the Reset hold after its warning.
5. **Run the install in the app** ([the steps](../guide/install.md)). You should see the replacement confirmed on bank 1, and the backup offered for saving.

   > [!WARNING]
   > A phone takes no bench lease, and the bench server doesn't know about its install. Start no job while it runs: nothing refuses one.

6. **Record it.** `wellfed-bench banks` still names what the bench last wrote, since the phone's install is not a job. Read what the feeder runs (`bench_status`, or the feeder's status over a session) and record the run.

## Proving rollback on the bench

This needs the owner. It starts from a confirmed `feeder` on bank 1, an older `feeder` on bank 0, and a phone with an APK that carries a `feeder` image. To watch stock report on `serve`, stock must already hold this host's broker from an earlier setup frame, and the host must admit TCP 1883 and 8088 from the hotspot ([the bench host's firewall](../development/stock-server.md#the-bench-hosts-firewall)). This sequence, with the app installing, has not run in this order.

1. **Check the start,** as in step 1 above, and check the hotspot is up (`nmcli connection show --active`).
2. **Go to stock:** `wellfed-bench ota <name>-stock --review <stock review>` from the confirmed build. You should see stock 1.1.15 on bank 1, and the older build on bank 0.
3. **Install and take the backup:** run the install in the app from a real Reset hold ([above](#running-an-install-on-the-bench)). It reads bank 1 (1,792 pages), keeps it, promotes and confirms. Save the backup through the share sheet.
4. **Check the backup before anything writes:** `sha256sum` of the saved file prints `05147476…` (so its payload is `802ef620…`), and `cmp` finds it identical to the stock review's `image.bin`. Then stop `wellfed-bench serve --device plaf103`, since `wellfed join` is refused beside it, and join this computer with an Owner invite from the app (`wellfed join`). An update needs an owner's session, and the invite sheet starts on Member.
5. **Roll back.** If stock holds this host's broker, run `wellfed-stock serve --device plaf103 --config private/installer.json` (`init-installer` writes that config; `serve` refuses the bench's `server.json`, whose `plans: []` would erase stock's plans as soon as stock comes back). Start a UART capture (`wellfed-flash read observe`), then run `wellfed restore-stock --backup <the saved file>`. It makes a fresh key and asks for a Dispense press to enroll it, because the app's image doesn't trust the bench key. Bank 1 gives way, bank 0 writes stock, and the restore reports activated. You should see stock's `app_main:version:` banner from bank 1 on the UART.
6. **Go back:** run the install in the app again. You should see the replacement confirmed on bank 1.
7. **Put things back.** Stop `wellfed-stock serve`, start `wellfed-bench serve --device plaf103` again, and leave the hotspot as you found it. Record the run: versions, digests and the kept file's path.

## Next steps

- What each install step decides: [the installer](consumer-installation.md#the-installer).
- What a rollback keeps and leaves behind: [rollback](consumer-installation.md#rollback).
