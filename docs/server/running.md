# Running the server

This guide is for an owner running the Wellfed server at home: installing it, its data and backups, its settings, reaching it from outside, and adding people. For what the server is, start with [the server](README.md).

The server is one static binary, run under systemd or in a container. Either way it keeps everything in one data directory, with no separate database.

## Install it

There is no published release, so you build it yourself ([packaging](#packaging)): one static binary per target, `wellfed-server-x86_64-unknown-linux-musl` and `wellfed-server-aarch64-unknown-linux-musl`, and the same binary in a container image.

### With systemd

[`packaging/wellfed-server.service`](../../src/server/wellfed-server/packaging/wellfed-server.service) is an example unit for the static binary.

1. Install the binary and the unit:

   ```sh
   sudo install -m 0755 wellfed-server-x86_64-unknown-linux-musl /usr/local/bin/wellfed-server
   sudo install -m 0644 wellfed-server.service /etc/systemd/system/
   sudo systemctl daemon-reload
   sudo systemctl enable --now wellfed-server
   ```

2. Run `curl http://127.0.0.1:8080/health`. You should see the server's version.

How the unit runs it:

- **Its own user.** `DynamicUser=yes` makes a user for the unit alone, owning only `StateDirectory=wellfed-server`, which is `/var/lib/wellfed-server` (mode 0700, files 0600 through `UMask=0077`).
- **Locked down.** The rest of the system is read-only or hidden (`ProtectSystem=strict`, `ProtectHome`, `PrivateTmp`, `PrivateDevices`, no capabilities, `@system-service` system calls only); `systemd-analyze security` rates it 1.3.
- **Discovery works.** The unit shares the host's network, so mDNS finds feeders whose address changed. Address families are limited to IPv4, IPv6, Unix and netlink, which mDNS needs to list the interfaces it browses on.
- **Local only at first.** It listens on `127.0.0.1:8080` until `listen` says otherwise.

### In a container

The image (`FROM scratch`) holds the binary (with the zone database built in), the CA certificates its GitHub release checks need, and an empty `/tmp`. It runs as uid 65532, keeps its data in the volume at `/data` (`WELLFED_SERVER_DATA`), and listens on `0.0.0.0:8080` (`WELLFED_SERVER_LISTEN`). Arguments after the image name are the server's flags.

A container's bridge network doesn't pass mDNS multicast, so there are two choices.

**Host networking keeps discovery,** and is simplest:

```sh
docker run -d --name wellfed-server --restart unless-stopped \
    --network host -v wellfed-server:/data wellfed-server
```

It listens on the host's port 8080 directly. If a proxy on the host is the only way in, set `WELLFED_SERVER_LISTEN=127.0.0.1:8080`.

**A bridged network works by address.** Turn discovery off:

```sh
docker run -d --name wellfed-server --restart unless-stopped \
    -p 8080:8080 -v wellfed-server:/data wellfed-server --no-discovery
```

The server then adopts and reconnects at the stored address only, which you give when you adopt each feeder (`address` in `POST /feeders`: an IP address, on the API's port by default, or `IP:port`; a DNS name is not resolved). Adopting with no address is refused (`400 invalid`).

- **Give each feeder a fixed address,** with a DHCP reservation. A feeder whose address changes stays offline until it is back at that address.
- **The feeders must be reachable from the container.** A bridge network routes out to the LAN but not back in, which the feeder API never needs.
- **The volume** is a named volume (as above) or a bind mount of a local directory owned by uid 65532. `--user` picks another uid; with rootless podman, `--userns=keep-id` maps yours.

## The data directory

The server keeps everything in one directory, given by `--data` or `WELLFED_SERVER_DATA`: `/var/lib/wellfed-server` under systemd, the volume at `/data` in the image.

| Path | What |
| --- | --- |
| `server.db` | The database. |
| `server.toml` | The settings. Optional: every key has a default. |
| `server.id` | The server's id, 16 hex digits made on the first start and not a secret, by which the app recognizes the server ([find it from the app](#find-it-from-the-app)). If it's missing, unreadable or damaged, the server makes a new one and warns. |
| `keys/server.key` | The sealing key that seals every feeder key in the database: 32 bytes, mode 0600, in a 0700 directory. A key or directory that others can read is tightened at start, with a warning; if the server can't change them (another user owns them, or a read-only mount), it refuses to start. |
| `backups/` | Daily copies of the database (mode 0700, each copy 0600), with a `README.txt` saying where the key is and how to restore. |
| `releases/` | Verified firmware releases, each with a record. |
| `stock/` | Each feeder's stock firmware backup: `stock/<identity>/stock-1.1.15.bin` (the vendor's container) and its `.json` record. |

The first start makes the key, placed with a hard link, so keep the directory on local disk or a volume whose driver supports hard links; on some network and FUSE mounts the first start fails with a write error. On a `server.db` with no key file the server refuses to start, since a new key couldn't open the feeder keys; that usually means a backup was restored without its key.

## Back it up

The server copies its database into `backups/` every day at 03:30 in the home's zone, and an owner can take one any time with `POST /api/v1/backups`. The newest 7 are kept (`backups.keep`).

> [!WARNING]
> The backups leave out `keys/server.key` and `stock/`. A restored `server.db` opens only with its key, and without it the server can no longer reach your feeders. Once an install has written over a feeder's stock firmware, the server's copy in `stock/` may be the only one left, and neither these backups nor an export holds it. Copy `keys/server.key` somewhere off the machine once, copy `stock/` off the machine after the server keeps a feeder's stock, and copy `backups/` off the machine regularly. Or back up the whole data directory.

To restore from a backup:

1. Stop the server.
2. Run `ls -l backups/` and pick the copy by its date. Then `cp server.db server.db.before-restore`: the next step replaces it.
3. Copy the backup to `server.db`, and delete any `server.db-wal` and `server.db-shm` beside it.
4. Put `keys/server.key` back.
5. Start the server, then run `curl http://127.0.0.1:8080/health` (or your `listen` address). You should see the server's version.

## Upgrade it

Until the first server release, migrations are squashed in place, so a build can change one that an existing `server.db` already ran. sqlx then refuses to start on that database: the applied migration's checksum no longer matches. Such a build says so in its commit. To move to it:

1. Stop the server.
2. Move `server.db` (and any `server.db-wal` and `server.db-shm`) aside. Keep `keys/server.key`.
3. Start the server; it makes an empty database.
4. Set it up and adopt the feeders again.

An export from the old build doesn't carry over either, since the archive's rows changed with the tables. The November 8, 2026 change added `api_revision` to the `projection_version` row (migration `0004_projections.sql`) and to the export's row of that table.

To go back to an older build after an upgrade migrated the database, start the older build only on a copy from before the upgrade. Each migration run leaves one beside the database as `server.db.before-<version>-<unix ms>`. An older build that finds a database a newer one migrated opens it read-only, writes nothing, and refuses to start, naming the migration it lacks. So starting the old build by mistake loses nothing, and the newer build still opens the database.

## Settings

`server.toml` in the data directory holds what is neither a secret nor a setting of the home (the home's time zone lives in the database). [`server.example.toml`](../../src/server/wellfed-server/server.example.toml) lists every key with its default and a comment: copy it and keep only the keys you change.

| Key | Default | What |
| --- | --- | --- |
| `listen` | `127.0.0.1:8080` | Where the API listens over plain HTTP: this computer, a reverse proxy in front, and today's CLI and Home Assistant. `0.0.0.0:8080` serves the household's other devices. |
| `tls_listen` | none | Where the API also listens over HTTPS, with a certificate the server's identity key signs ([server identity](README.md#server-identity)). The app reaches the server on the home network only here, and mDNS announces only this port. `0.0.0.0:8443` serves the household's phones. |
| `hosts` | none | Names the server is reached by, besides IP addresses and `localhost`. Any other name is refused `421 unknown_host`, which stops DNS rebinding. |
| `label` | `wellfed-server` | The label the server enrolls under on each feeder, 1-16 bytes. |
| `discovery` | `mdns` | `off` adopts and reconnects by address only. |
| `advertise` | `mdns` | Announce the server on the LAN so the app finds it ([find it from the app](#find-it-from-the-app)). `off` never does. |
| `trusted_proxies` | none | Proxies (addresses or CIDR ranges) whose forwarded headers are believed. |
| `retention.raw_sample_days` | keep forever | Days raw samples are kept. Hourly rollups are always kept. |
| `releases.fetch` | `false` | Check the project's GitHub releases daily, at 04:00. |
| `releases.project_keys` | `[]` | The project's update keys, 64 hex digits each. |
| `releases.owner_keys` | `[]` | Your own enrolled update keys, 64 hex digits each. |
| `backups.keep` | `7` | Backups kept. |
| `webhook.url` | none | Where each alert's opening and clearing is POSTed as JSON ([alert webhook](README.md#alert-webhook)): `https://` anywhere, plain `http://` only to this computer or the home network. |
| `webhook.allow_plain_http` | `false` | Take a plain `http://` webhook off the home network. Anyone between the server and that host then sees the URL and every alert. |

Every key has a flag, and every flag an environment variable: `--listen` (`WELLFED_SERVER_LISTEN`), `--tls-listen` (`WELLFED_SERVER_TLS_LISTEN`), `--host` (`WELLFED_SERVER_HOSTS`, comma-separated), `--label`, `--discovery` (or `--no-discovery`), `--advertise` (or `--no-advertise`), `--trusted-proxy` (`WELLFED_SERVER_TRUSTED_PROXIES`), `--raw-sample-days`, `--releases-fetch true|false`, `--release-project-key` and `--release-owner-key` (`WELLFED_SERVER_RELEASE_PROJECT_KEYS` and `..._OWNER_KEYS`, comma-separated), `--backups-keep`, `--webhook-url` and `--webhook-allow-plain-http true|false`. Each variable is `WELLFED_SERVER_` and the flag's name.

- **A flag beats its variable,** and either replaces the file's value whole. `--host` given once replaces the file's `hosts`.
- **An override can't clear a list.** No flag or an empty variable means "not given". To drop every host or proxy, edit the file.
- **`--no-discovery` beats** `--discovery` and its variable, and `--no-advertise` beats `--advertise`.
- **A bad file stops the start.** An unknown key, a wrong type or an out-of-range value (a 17-byte label, `raw_sample_days = 0`, a host name with a port) is refused with the key's path (`retention.raw_sample_days`, `hosts[1]`) and its line. So is a webhook URL that is no URL, or plain `http://` off the home network without `webhook.allow_plain_http`; that error names `webhook.url` and never quotes it. A missing file means the defaults.

Under systemd, install `server.toml` as root with `sudo install -m 0644 server.toml /var/lib/wellfed-server/`; the server only reads it, and it holds no secret, so a root-owned file is fine. A webhook URL may be a secret of its own (Home Assistant's is), so keep it out of the world-readable file and out of the unit, whose drop-ins anyone on the machine can read. Put it in an environment file only root reads:

```sh
sudo install -m 0600 /dev/null /etc/wellfed-server.env
sudoedit /etc/wellfed-server.env     # one line: WELLFED_SERVER_WEBHOOK_URL=https://…
sudo systemctl edit wellfed-server   # add under [Service]: EnvironmentFile=/etc/wellfed-server.env
```

The URL goes in through the editor, so it never lands in a shell's history.

The next start reads it, and `--help` never shows its value. Other `WELLFED_SERVER_*` variables, which hold no secret, can go in the same drop-in with `Environment=`.

## Find it from the app

The server announces its own HTTPS on the home network over mDNS, so the app can list it without a typed address. The app reaches the server on the home network only over that HTTPS, so set `tls_listen`, such as `tls_listen = "0.0.0.0:8443"`. Without it, or while it listens on this computer alone, nothing is announced, and the log says so.

- **The record** is `_wellfed-server._tcp`, with the `tls_listen` port. Its instance and host names are `label` (`wellfed-server` by default) and the start of the server's id, such as `wellfed-server-1a2b3c4d` and `wellfed-server-1a2b3c4d.local`, so two servers never announce the same name, and neither clashes with the machine's own.
- **Its TXT record** holds only the server's version (`version`), id (`id`, from `server.id`) and identity key (`key`, 64 hex digits). Anyone on the network can hear it, so it names no household, account, feeder or machine.
- **The app connects by the address it resolves** as `https://`, and checks the server's handshake against the announced key before it sends the password ([server identity](README.md#server-identity)). It recognizes the server by `id` when that address changes. The server always answers to an IP address, so `hosts` needs nothing for it.
- **To add the server by its address** instead, type the `https://` address the log prints at the start (`serving on https://…`), such as `https://192.168.1.20:8443`. The app pins the key that address proves, and shows its fingerprint under the address: compare it with the one the log prints.
- **On `0.0.0.0`** the record carries the machine's IPv4 addresses, following them as they change; on `[::]`, IPv6 addresses, and IPv4 too when the socket also takes IPv4; on one address, only that one.
- **At shutdown** the server withdraws the record, so the app drops it at once.
- **`advertise = "off"`** (or `--no-advertise`) turns it off, separately from `discovery`, which finds feeders. On a container's bridged network the announcement doesn't leave the bridge, so turn it off there too.

## Add the first owner

A fresh server has no accounts, and the first visitor creates the first owner, over the API since the server has no web UI. Until then, anyone who reaches the server can claim it, through setup or by importing an archive, so add the owner while it still listens on `127.0.0.1` alone, before you [reach it from outside](#reach-it-from-outside). To keep that window short, a server with no account takes a setup or an import only for an hour after it starts, and its log says so at the start. Past the hour, `GET /api/v1/setup` answers `"open": false` and setup is refused `409 setup_closed`; restart the server to open the hour again.

1. Run `curl http://127.0.0.1:8080/api/v1/setup`. You should see `"open": true`, and the host's time zone as `zone` if it has one.
2. Create the owner with a name, a password of at least 10 characters, and the home's IANA zone:

   ```sh
   curl -X POST http://127.0.0.1:8080/api/v1/setup \
       -H 'Content-Type: application/json' -H 'Origin: http://127.0.0.1:8080' \
       -d '{"name": "alex", "password": "…", "zone": "Europe/Berlin"}'
   ```

3. Run `wellfed server login --server http://127.0.0.1:8080 --name alex` and type the password. You should see the CLI keep a token, after which `wellfed server feeders` works.

Setup closes as soon as any account exists, or an hour after the start with none. The zone must be one the feeder can follow on its own, since the server writes its daylight-saving rule to each feeder: a zone with irregular changes no rule describes is refused `422 zone_unsupported`, and an unknown name `400 invalid`.

## Reach it from outside

The app speaks plain `http://` only to the device it runs on, and reaches a server on the home network over the server's own HTTPS ([server identity](README.md#server-identity)). An address typed into the app with no scheme is `https://`. The CLI speaks plain `http://` only to this computer or the home network: a loopback, private or link-local address, `localhost`, or a `.local` name, as the [alert webhook](README.md#alert-webhook) does. Any other name takes the server's `https://` address, since a login sends the password and every later call a token that can feed. A name only your router resolves (`nas.lan`) counts as off the home network, so use the server's IP address or its `.local` name. Home Assistant asks you to confirm a plain `http://` address instead.

On your home network, reach the server directly: set `tls_listen = "0.0.0.0:8443"` for the app, `listen = "0.0.0.0:8080"` for the CLI and Home Assistant until they pin the key too, and leave `trusted_proxies` empty. From outside, a VPN works over plain HTTP only when it routes your LAN's own addresses, so the phone still reaches the server at a private address such as `192.168.1.20`. A VPN that gives the server an address of its own instead (Tailscale's `100.x` addresses, which are carrier-grade NAT space, or its `*.ts.net` names) counts as off the home network, and the app and the CLI refuse plain `http://` to it. Use `https://` there: Tailscale's own HTTPS, or a reverse proxy that terminates HTTPS, such as Caddy:

```caddyfile
feeder.example.org {
    reverse_proxy 127.0.0.1:8080 {
        # A request id both logs share (the server keeps one plain id).
        header_up X-Request-ID {http.request.uuid}
    }
}
```

And the matching `server.toml`:

```toml
listen = "127.0.0.1:8080"         # only the proxy reaches the server
hosts = ["feeder.example.org"]    # the public name, besides IPs and localhost
trusted_proxies = ["127.0.0.1"]   # the proxy's address, or its range
```

Caddy obtains and renews the certificate, preserves `Host` and sets `X-Forwarded-For` and `X-Forwarded-Proto`. The server reads those headers only from a peer in `trusted_proxies`:

- **The client's address** comes from `X-Forwarded-For` (or `Forwarded` when that is absent), read right to left, each comma-separated entry on its own: the first hop that isn't a trusted proxy is the client, and anything a client wrote to its left is ignored. If `X-Forwarded-For` is present at all, `Forwarded` is ignored, so your proxy must set or append to `X-Forwarded-For`, as Caddy does, and nginx with `$proxy_add_x_forwarded_for`. The address drives the per-address login limit and is recorded with each login in the audit log. A proxy missing from `trusted_proxies` makes every request come from the proxy's own address, so the per-address limit of 20 failed logins a minute becomes one limit for everyone, and anyone can use it up to stop everyone's logins.
- **The scheme** is `X-Forwarded-Proto`'s last value (or the last `Forwarded` element's `proto`), and `https` makes the session cookie `Secure`. Behind a chain of proxies, the front one sets it from the client's connection and each one behind passes it on.
- **From any other peer** these headers are ignored: the address and the scheme are the connection's (HTTPS on `tls_listen`, plain HTTP on `listen`), so a server reached both directly and through the proxy keeps its direct clients' real addresses.

> [!WARNING]
> Anyone listed in `trusted_proxies` can claim to be any client. List only the proxy's own address, or a container network's range such as `172.16.0.0/12` when the proxy runs in one. Never list your LAN or the internet.

The public name goes in `hosts`, since the server refuses names it wasn't given. The `Origin` check compares the request's `Origin` with `Host`, so the proxy must pass the client's `Host` on, as Caddy does. nginx does not by default: add `proxy_set_header Host $host;`, or every login and setup is refused `403 cross_origin`. A request's own `x-request-id` is kept when it is one plain id (at most 64 bytes of letters, digits, `.`, `_` and `-`), so the proxy's id appears in both logs; anything else gets a fresh UUID.

### A phone on a Wi-Fi with no way out

The Android app keeps its connections on the Wi-Fi, so it reaches feeders there even while that Wi-Fi has no internet: a feeder's own network, a hotspot with no uplink, a captive portal. Android then routes the rest of the phone over mobile data. A server reached by a public name would be out of reach, so for it alone the app goes over mobile data instead, through a relay inside the app.

- **When.** Only while the Wi-Fi the app uses has no validated internet and another network has, and only for a server whose address is off the home network. A server at a home-network address, and every feeder, stay on the Wi-Fi.
- **How.** The relay is a SOCKS5 proxy on the phone's loopback, open only to the app (it takes a user and password the app makes at each start). It opens each connection, and looks up each name, on the network Android routes by default. The request is the same HTTPS as ever, end to end with the server.

The CLI and Home Assistant run on machines that route on their own, so they have no relay.

## Add a member's phone

1. As an owner, from a login, add an account for the person with a temporary password (`POST /api/v1/users`) and the member role. A viewer can only read; an account can carry an expiry, for a pet sitter.
2. The person logs in and sets their own password (`PATCH /api/v1/session`).
3. From that login, the phone gets its own token (`POST /api/v1/tokens`), shown once, which it sends as `Authorization: Bearer`.

Outside the home the phone also needs the server's public URL, such as `https://feeder.example.org`. A token needs no cookie or `Origin`; if the phone is lost, revoke its token alone. A phone that should keep feeding while the server is down also holds its own feeder key, which works only on the home network ([two paths, one feed](README.md#two-paths-one-feed)).

## Reset a locked-out owner

If an owner forgets their password, run `reset-owner` on the host with the server stopped. It prints a new temporary password and ends their sessions; with several owners, add `--name NAME`.

```sh
wellfed-server --data DIR reset-owner
```

Under the systemd unit, run it as the unit's user:

1. `sudo systemctl stop wellfed-server`
2. `sudo systemd-run --pty --wait -p DynamicUser=yes -p User=wellfed-server -p StateDirectory=wellfed-server -p UMask=0077 /usr/local/bin/wellfed-server --data /var/lib/wellfed-server reset-owner`
3. `sudo systemctl start wellfed-server`

After an [import](design.md#export-and-import) no account has a password, so `reset-owner` is also how the first owner gets back in.

## Packaging

This part is for whoever builds the release.

- **`make server-dist`** builds both static binaries into `dist/server/`.
- **`make server-image`** builds the container image. Set `CONTAINER=podman` for podman.
- **Publishing** the binaries to a GitHub release, and pushing the image to a registry, is the owner's step. Nothing in the repository does it.

Both run [`packaging/Containerfile`](../../src/server/wellfed-server/packaging/Containerfile), whose `rust:1.99.1-alpine3.24` stage on the build machine's own platform links both targets with zig (`cargo zigbuild`), so the aarch64 binary and the arm64 image build on an x86_64 host with no emulation. rustls with its aws-lc provider and the bundled SQLite both build for musl this way. `docker buildx build --platform linux/amd64,linux/arm64` builds a multi-arch image.

Without a container, a host with zig and `cargo-zigbuild` runs the same build after `rustup target add` for both targets:

```sh
cargo zigbuild --release --locked -p wellfed-server --target x86_64-unknown-linux-musl --target aarch64-unknown-linux-musl
```

A plain `cargo build --target x86_64-unknown-linux-musl` also needs a musl C compiler (`musl-gcc`, or Alpine's own gcc) for aws-lc and SQLite.

### The release check

`wellfed-server`'s `tests/release.rs` checks the server as it ships. It starts the server as a separate process on a fresh temporary data directory, sets the household up, adopts the simulated feeder by address with `--no-discovery`, reads readiness and the feeder's live status, and takes a backup. It then stops the server with SIGTERM, as systemd and container runtimes do, expects a clean exit, and starts it again to check it reconnects by the stored address.

- **`make check`** runs it against the debug build.
- **`make check-server-release`** runs it against this host's binary in `dist/server/` (`WELLFED_RELEASE_SERVER=<binary>`), then against the image (`WELLFED_RELEASE_IMAGE=<image>`, `WELLFED_RELEASE_RUNTIME=docker|podman`). The image run is an ignored test, so the gate never needs a container runtime.

Neither leaves anything running.

## Next steps

- [The server](README.md): what it adds, and how feeds reach the feeder.
- [Server design](design.md): its crates, API and storage.
- [Owner guide](../guide/README.md): pairing and daily use.

## Evidence

- **Simulation**, October 27, 2026: the release check above runs in `make check` against the debug build. Both musl targets were built through the Containerfile, rustls with aws-lc and the bundled SQLite included.
- **Simulation**, November 3, 2026: unit tests check the announcement's record (type, instance, host name, port, version and id only, addresses for each kind of listen address, none on loopback) and that the id is made once and kept. No announcement was sent on a real network.
- **Static analysis**, October 27, 2026: the settings table matches `server.example.toml`, which a test parses; the unit's hardening matches `packaging/wellfed-server.service`.
