# ADR-0002: One hub per listener; devices pair, nobody logs in

- Status: accepted
- Date: 2026-11-01

## Context

lull 2 should feel like Spotify: download an app, and the same history, likes and playlists appear on every device, without an account (PROJECT.md). Two facts shape this:
- **Soulseek allows one login per account,** and slskd can't run on a phone. So one place has to own the Soulseek session and the music.
- **Normal users don't run Docker or edit `.env` files.**

## Decision

**One hub per listener.**
- The hub is `crates/server`: a library with `lull_server::run(env, shutdown)`, plus the `lull` binary.
- The Windows and Linux apps run it in-process with a bundled slskd (ADR-0006). Servers run the binary or the Docker image.
- The hub owns Soulseek, the library, the database, themes and the Last.fm connection. Every device is a client of it, so nothing is synced between devices.

**Pairing instead of logins.**
- Each device holds a revocable token from `core::devices`: the hub stores its sha256, and the device sends it as the HttpOnly cookie `lull_device` or `Authorization: Bearer`.
- **Kinds:**
  - `browser`: the browser or installed web app;
  - `app`: the lull app;
  - `subsonic`: an app password. It is stored as given, because Subsonic's `md5(password + salt)` scheme needs it.
- **Adding a device:** a paired device asks for a pair code (6 characters, single use, 10 minutes), and the hub shows it as a QR code of `<hub url>/#/pair/<code>`.
- **The first device:**
  - The desktop app creates its own token in-process when it starts the hub.
  - A headless hub with no devices prints a one-time setup code in its log; `LULL_SETUP_CODE` fixes it instead.
- **Without a token,** the gate allows only `/health`, `GET /api/setup`, `POST /api/setup/claim`, `POST /api/pair` and the static UI.
- **Loopback is never trusted,** because `tailscale serve` proxies to the hub from 127.0.0.1.
- `LULL_ACCESS_TOKEN` and `POST /api/session` are retired.

**Settings live in the app.**
- `core::settings` stores `LULL_*` keys in `core_config`.
- The hub starts from the environment merged over the stored keys: **env wins and shows as locked** in the UI, stored comes next, then the defaults in `config`.
- `LULL_DATA_DIR`, `LULL_LISTEN` and `LULL_LOG` are environment-only, because the hub needs them before the database opens.
- A change is validated with `Config::from_vars` before it's stored.
- **Applying changes:**
  - A change to a key a crate reads at `start` restarts the hub's crates in-process. The listener stays bound, and the UI reconnects over SSE, as it already does after a deploy.
  - Any crate can ask for this by publishing `hub.restart` on the bus.
- **Soulseek credentials** are settings too. The hub applies them through slskd's options API: verify it first, and fall back to writing the bundled sidecar's `slskd.yml`.

**Remote access** is the listener's Tailscale. The app offers `tailscale serve`, which also gives the HTTPS browsers need to install the web app.

## Alternatives

- **Accounts with a hosted sync service.** It contradicts self-hosting and needs a service someone pays for.
- **A peer-to-peer sync between devices** (CRDTs). It's heavy machinery, and it still needs one place to run Soulseek.
- **Trusting the local network.** Any device on a café or shared network could use the hub, and `tailscale serve` makes everything look local.

## Consequences

- **No phone works alone.** A phone needs its hub reachable, or offline downloads (ADR-0005).
- **Settings come from two places.** The Settings screen shows where each value comes from.
- **Subsonic app passwords sit in the database in full.** They're random, per client and revocable, and the database lives on the listener's own machine.
