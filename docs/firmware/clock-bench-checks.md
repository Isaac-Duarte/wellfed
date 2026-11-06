# Clock bench checks

This guide holds the [clock](clock.md)'s physical checks still to run on the [relay bench](../devices/plaf103/bench.md): live NTP, learning the rate, the spoof check, the remaining warm-reset cases and expiry with NTP on. None of them moves the motor.

## Network for NTP

A live NTP test needs the owner to add one host firewall rule, so check the host first. The bench host's firewall (`/etc/computer-security.nft`, table `inet computer_security`, input policy drop) admits from the hotspot interface `wlan1` only:

- DHCP (UDP 67), DNS (TCP and UDP 53) and mDNS (UDP 5353);
- the stock server's broker and image server (TCP 1883 and 8088 to `10.42.103.1`, [the bench host's firewall](../development/stock-server.md#the-bench-hosts-firewall)).

UDP 123 from the feeder is dropped, as is any other port. No time service runs on the host (`systemd-timesyncd` and `chronyd` inactive), and unprivileged processes cannot bind below 1024 (`ip_unprivileged_port_start` is 1024). So the owner adds one input rule beside the hotspot's others, either:

- **the standard port:** `iifname "wlan1" ip saddr 10.42.103.0/24 ip daddr 10.42.103.1 udp dport 123 accept`, with the responder run with the privilege to bind port 123 (`sudo`, the owner's call); or
- **an unprivileged port**, which the setting's port allows: `iifname "wlan1" ip saddr 10.42.103.0/24 ip daddr 10.42.103.1 udp dport 1123 accept`, with the feeder set to `--ntp gateway:1123`.

Replies need no rule: the output path is unfiltered, and the input chain accepts established flows. The responder serves the host's own clock, which nothing on the host disciplines. That is fine, since the reference is the host.

## 1. Live NTP sync

About 45 minutes.

1. Install the `feeder` image, with the hotspot up and the rule in place.
2. Run the bounded responder in the foreground: `cargo run -p wellfed-rig --example ntp-serve -- 10.42.103.1:1123 --seconds 2700` (or `:123` with the standard rule).
3. Run `wellfed time --ntp gateway:1123 --zone America/Denver --address <feeder>`, then `wellfed time --show`. With no client sync since boot, you should see:
   - `PLAPI1 ntp server=10.42.103.1:1123`, and a `PLTIM1 sync ok=1 source=ntp kind=first` within seconds;
   - the view's `clock_ms` within 1 s of the host, `error_ms` under 100, and `offset_s` −21600 or −25200 as the date says;
   - a `ClockSync` event by the feeder in `wellfed events`.
4. Leave it 45 minutes. Every poll is `used`, and each poll's `delta_ms` over its spacing is the clock's rate. `learned` stays false with NTP alone.
5. Stop the responder. Restore the settings (`--ntp gateway`), the hotspot and the firewall as the owner wants them.

## 2. Learning the rate

1. Make two owner's `wellfed time` syncs about 11 hours apart.
2. `learned` should turn true, with `rate_ppb` near the measured rate and `rate_bound_ppb` 200 000. Each sync carries `SYNC_ERROR_MS` (1 s), and a rate is learned only to within 50 ppm.

Logging the rates over a long run shows how much the rate moves within a boot, which is what the learned floor stands for.

## 3. Spoof check

A member's `TimeSync` an hour off must be refused `Time`. Alternatively, point `--ntp` at a second responder whose clock is offset. That needs the responder to gain an offset option, or a second host clock.

## 4. Warm resets, remaining cases

1. Power-cycle with the relay. The boot logs `result=cold` with the clock invalid, and the next warm reset logs `empty`.
2. Make the clock invalid first (`--ntp off`, then wait out the window or reboot cold). `wellfed reboot` then gives `empty`, since an invalid clock is never stored.
3. To check the carry again, confirm in the link map that `wellfed_time_carry` is at `0x1007BFC0` and `__sram_end__` at `0x1007BC50`, below the call trail.

## 5. Expiry with NTP on

About 7 hours, unattended. The bench hotspot's NTP does not answer, so this needs a network whose NTP does.

The clock should stay valid with a small `error_ms`, apart from up to a minute around each expiry. Then the next poll (within 64 s) re-sets it with `PLTIM1 sync ok=1 source=ntp kind=first`.

## Next steps

Record results in the [clock evidence](clock.md#evidence) with their level and date. Put the network and services back as you found them, and record what ran.
