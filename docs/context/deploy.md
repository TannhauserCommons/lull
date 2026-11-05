# Deploying lull

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

`IMAGE_TAG=latest` follows lull's releases (from 0.2.4, the first made by release.yml), and `X.Y.Z` pins one; `main` follows every change lull's queue lands, and `sha-<commit>` pins one of those. The slskd and gluetun images are pinned in `compose.yaml` and move with lull's releases, because lull reads slskd's file layout (`crates/soulseek/src/layout.rs`).
