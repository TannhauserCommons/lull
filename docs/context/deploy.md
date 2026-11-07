# Deploying lull

A hub runs three ways (ADR-0090): in the desktop app (desktop.md), as the Linux package below, or in Docker. Docker is one option, never required.

## The Linux package

For a computer that only serves: `lull-server_<version>_<amd64|arm64>.deb`, `lull-server-<version>.<x86_64|aarch64>.rpm`, or `lull-server_<version>_linux-<x64|arm64>.tar.gz` with `sudo ./install.sh`. Not `lull`: that is the desktop app's package, which the server package conflicts with (one hub per computer). Built by `packaging/linux/build.sh` (`server-package` workflow) on Ubuntu 22.04, so glibc 2.35 or newer. The packages depend on ICU, which slskd (.NET) aborts without; `install.sh` warns when it's missing.

**What it holds:** `/usr/lib/lull-server/` has `lull` and the slskd, yt-dlp and Deno that `app/build.rs` pins for the app's Linux build (`packaging/linux/sidecars.mjs` reads them there), plus `/usr/bin/lull` and the licences in `/usr/share/doc/lull-server/`.

**How it runs:** `lull.service`, under the system user `lull` (made by the package), with data in `/var/lib/lull`.
- The unit sets only how lull is run: `LULL_DATA_DIR`, `LULL_LISTEN=0.0.0.0:8484`, `LULL_SLSKD`, `LULL_YTDLP` and `LULL_DENO`.
- With `LULL_SLSKD` the hub runs that slskd itself, as the app does (`lull_server::managed`). Its folders are in the data folder, and systemd stops it with the service.
- The hub advertises `_lull._tcp`, and stores the LAN address as `LULL_PUBLIC_URL` unless an https one is set.
- Installing starts the service and enables it at boot. Removing stops it and keeps `/var/lib/lull` and the music.

**Setting it up:** `sudo lull setup` asks about four things, and stores the answers as the web page does:
- the music: a folder, or an existing Jellyfin (address, API key, user) and the folder it reads. A new folder is made for `lull`; for an existing one lull's user can't use, it offers an access rule (`setfacl`);
- the Soulseek account;
- sharing;
- outside access: `tailscale serve` where Tailscale runs, or the https address of a reverse proxy or the listener's own domain (proxy it to port 8484).

Then it prints the addresses and a pair link.

The other commands:
- `sudo lull status`, `sudo lull pair`;
- `sudo lull config [get|set|unset KEY [VALUE]]`.

They talk to the hub over `/var/lib/lull/admin.sock` (hub.md). A fresh hub's music folder is `/var/lib/lull/music`.

**Firewalls:** where firewalld or ufw is on (Fedora's firewalld is, by default), `lull setup` offers to open the hub's port 8484 and Soulseek's 50300, or prints the commands.

**Proof:** the queue builds the x64 package when its parts change and installs it in a fresh Ubuntu 24.04, Debian 13 and Fedora 43 running systemd (`tests/hub-contract/in-systemd.sh`). The hub contract (`tests/hub-contract/run.mjs`) runs against the service there:
- `/health` and every part;
- the web UI;
- the gate, pairing with `lull pair`, and every part's API;
- a settings round trip and Subsonic;
- its slskd reached, yt-dlp and Deno run;
- the mDNS advert.

The removed package must leave no slskd running.

## Docker

One compose file (`deploy/compose.yaml`) and one env file (`.env`, copied from `deploy/.env.example`). `COMPOSE_PROFILES` in `.env` picks the shape. Why one file for all three: [ADR-0011](../adr/0011-one-compose-file-three-shapes.md).

## Every shape

1. Copy `deploy/compose.yaml` and `deploy/.env.example` (as `.env`) into one folder.
2. In `.env`, set:
   - `MUSIC_DIR` (Jellyfin's music folder on the host);
   - `RUN_AS` (its owner, `id -u`:`id -g`);
   - `LULL_SLSKD_API_KEY` (16 to 255 random characters: `openssl rand -hex 24`);
   - `LULL_SLSKD_WEB_USERNAME` and `LULL_SLSKD_WEB_PASSWORD` (slskd's web login, also a random password): lull restarts a stuck slskd with them ([soulseek.md](soulseek.md#stuck-transitions));
   - `LULL_PUBLIC_URL`, the Jellyfin keys, and Last.fm if wanted.
3. `docker compose up -d`, then open `http://<host>:8080` (`PORT`).

**What runs:**
- **`lull`** is the UI and API. Its data is in the `lull` volume: back it up. It mounts the music folder read-write, because saves are written there, and, optionally, a Winamp skins folder you keep yourself (`SKINS_DIR`, an empty volume when unset) read-only at `/skins`, read when `LULL_SKINS_DIR=/skins`. Skins added in the app need neither: they live in `/data/skins/mine` ([skins.md](skins.md#your-skins)).
- **`init`** runs once per start: it hands fresh docker volumes to `RUN_AS`, so nothing runs as root.
- **slskd's web UI** (port 5030) is never published; lull reaches it inside the compose network.

**Folders shared by lull and slskd:**
- **Transfers** (`TRANSFERS`): slskd writes running downloads to `/transfers/incomplete` and finished ones to `/transfers/complete`. lull streams the first and saves from the second, and deletes only files it fetched once the cache is full.
- **Music** (`MUSIC_DIR`): slskd shares it **read-only** at `/music` unless `LULL_SHARE=none`. Soulseek etiquette expects sharing: many peers refuse users who share nothing.

Check a shape before starting it: `docker compose config --quiet`.

## Plain: `COMPOSE_PROFILES=plain`

slskd uses the host's own connection. Set `SLSK_USERNAME` and `SLSK_PASSWORD`: one Soulseek login per account at a time, so don't share an account with another client.

slskd listens for peers on `SLSK_LISTEN_PORT` (50300). Forwarding that port on your router lets more peers reach you. Without it slskd still works: it connects out to peers, and only peers who can't accept connections either are out of reach. The homelab runs this way, with no router ports.

## VPN: `COMPOSE_PROFILES=vpn`

slskd's traffic all leaves through [gluetun](https://github.com/qdm12/gluetun-wiki); lull's doesn't.
- **Settings:** set `VPN_SERVICE_PROVIDER`, the WireGuard or OpenVPN credentials, optionally `VPN_SERVER_COUNTRIES`, and a random `GLUETUN_API_KEY`.
- **The forwarded port.** gluetun asks the provider for one, and slskd's built-in gluetun integration (slskd 0.24.4 and later) polls gluetun and listens on that port, with no reconnect. So peers reach you through the VPN, with no router changes.
- **Providers.** Only providers with port forwarding work properly: **Private Internet Access, ProtonVPN, Perfect Privacy and PrivateVPN** (gluetun's [port-forwarding list](https://github.com/qdm12/gluetun-wiki/blob/main/setup/advanced/vpn-port-forwarding.md)). Another provider still connects, but like the plain shape without a forwarded port. Provider-specific settings (Proton's WireGuard key with port forwarding enabled, for one) are on gluetun's provider pages.
- **How lull reaches slskd.** slskd shares gluetun's network, so gluetun carries the `slskd` alias and opens port 5030 to the compose network only (`FIREWALL_INPUT_PORTS`); `LULL_SLSKD_URL` stays `http://slskd:5030`.
- **The host** needs `/dev/net/tun`, and gluetun needs `NET_ADMIN`.

## An slskd you already run: `COMPOSE_PROFILES=` (empty)

Only lull and `init` start.
- **`LULL_SLSKD_URL`:** your slskd's address as the lull container reaches it, e.g. `http://192.168.1.10:5030`, or the service name if it's in the same compose project.
- **`LULL_SLSKD_API_KEY`:** an API key you add to slskd's `web.authentication.api_keys`, with role `administrator`.
  - **Why administrator:** lull reads slskd's folder names from `/options`. With a `readwrite` key lull assumes the folders are named `incomplete` and `complete`.
- **`LULL_SLSKD_WEB_USERNAME`, `LULL_SLSKD_WEB_PASSWORD`** (optional): your slskd's `web.authentication` username and password. slskd lets only a web session restart it, so with these lull restarts an slskd stuck mid-connection; without them `/health` says when to restart it (`services.soulseek.extra.restartSlskd`).
- **`TRANSFERS`:** the host folder holding slskd's incomplete and downloads folders. lull maps them by name: slskd's `/data/downloads` and `/data/incomplete` mean `TRANSFERS` is the host's `/data`. lull must be able to read it and delete in it; leave `RUN_AS` as its owner.
- **Sharing:** whatever your slskd shares stays as it is. lull warns in its status tray when that doesn't match `LULL_SHARE`, and with `LULL_SHARE=library` asks slskd to rescan after saves.

## Updating

`IMAGE_TAG=latest`, the default, follows lull's releases (from 0.2.4, the first made by release.yml), and `X.Y.Z` pins one; `main` follows every change lull's queue lands, and `sha-<commit>` pins one of those. The slskd and gluetun images are pinned in `compose.yaml` and move with lull's releases, because lull reads slskd's file layout (`crates/soulseek/src/layout.rs`).

Settings › App shows the server's version and whether a newer release is out. An image from `main` says what it holds, for example **lull 0.2.4 + 12 changes (abc1234)**, and that it follows the latest changes; the queue works this out from git when it builds the image (`build_version` in `scripts/queue.sh`, `lull_core::build`). Updating is `docker compose pull` and `docker compose up -d` where the server runs.

**Updating from any paired device** (ADR-0090, behind the `hub-self-update` switch while it is finished):
- **Update in Settings › App,** or `sudo lull update` on the server. The hub of the Linux package or the image downloads the newest release's hub kit for its machine, checks its signature and files, and copies its database. Then it restarts on the new version.
- **The Linux package** takes the whole kit (lull, slskd, yt-dlp, Deno) into `/var/lib/lull/hub/`. The files the package installed stay as they are, and a newer package wins over older kits.
- **The image** takes only `lull` into `/data/hub/`; the image's yt-dlp and Deno and compose's slskd stay. A newer image wins the same way.
- **The health window:** a new version that doesn't answer healthy within 2 minutes is marked bad, the database copy goes back, and the version before runs again. Settings says so.
- **Following main:** an install that follows main (`IMAGE_TAG=main`) doesn't self-update: it is already ahead of every release.
- **Pinning a version:** `LULL_SELF_UPDATE=off` turns Update off, for a pinned version.
