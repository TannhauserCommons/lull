# Hub: pairing, devices, Connect, settings, restart

`crates/server` (ADR-0002). Endpoints and events: [api.md](api.md#hub-cratesserver-lull_coredevices-settings). Code outranks this page.

## Files

| File | What |
|---|---|
| `lib.rs` | `run` (open the db, resolve settings, bind, serve), the restart loop, the hub's own routes, `wire` (the crates, the only place they meet) |
| `hub.rs` | `Hub`: what outlives a restart (env, db, pairing, who is online) and the current `Wiring` (the crates' `Ctx` and routes); `restart` |
| `gate.rs` | The pairing gate and the `lull_device` cookie |
| `pairing.rs` | Setup code, pair codes, the wrong-code limit; `/api/setup`, `/api/setup/claim`, `/api/pair/codes`, `/api/pair` |
| `devices.rs` | `/api/devices/*` and lull Connect: `Online` presence, `device.state`, `device.command` |
| `events.rs` | `GET /api/events` (SSE) |
| `config.rs` | `GET`/`PATCH /api/config` |
| `web.rs` | The static UI (Quality session's) |

## Invariants

- **Every `/api` call needs a paired browser or app.** `gate::require` runs before every route, the crates' too. The only open `/api` routes are `GET /api/setup`, `POST /api/setup/claim` and `POST /api/pair`. Nothing outside `/api` is gated: `/rest` does Subsonic's own auth. The gate never looks at the peer address: `tailscale serve` proxies from 127.0.0.1.
- **Tokens are stored hashed** (`core::devices`); only a Subsonic app password is kept as given. An app password is refused under `/api`.
- **The setup code exists only while no browser or app is paired.** At start-up and after the last one is unpaired, the hub arms it: `LULL_SETUP_CODE`, or a random 8-character code printed in the log (`setup_code` field, level warn). Claims are serialised, so two devices can't both become the first.
- **Pair codes** are 6 characters from an alphabet without 0/O/1/I, single use, 10 minutes, kept in memory (a process restart drops them). Ten wrong codes in a minute refuse every code until the minute is over, setup code included.
- **Browsers never see their token.** It is set as `lull_device` (HttpOnly, SameSite=Lax so Last.fm's callback carries it, Secure when `LULL_PUBLIC_URL` is https or the proxy says `X-Forwarded-Proto: https`, 400 days). `/api/ping` renews it.

## Settings and restart

- Settings are `LULL_*` keys: environment, then stored (`core_config`), then `Config`'s defaults. An empty environment value counts as unset (the deploy kit passes every key of its env file).
- `settings::KEYS` lists what the app may set and which are secrets; `ENV_ONLY` (`LULL_DATA_DIR`, `LULL_LISTEN`, `LULL_LOG`, `LULL_SETUP_CODE`, `LULL_SLSKD_CONFIG_FILE`) never appear in `/api/config`. A test keeps both lists, `Config` and `deploy/.env.example` in step.
- `PATCH /api/config` validates the whole merged map with `Config::from_vars`, stores, then publishes `hub.restart`. It refuses `LULL_LIBRARY`: `PUT /api/library/source` reconciles the library before it stores the mode.
- **Restart** (`Hub::restart`, on any `hub.restart`):
  1. take the wiring's write lock, so requests wait;
  2. publish `hub.restarting` on the old bus, cancel the old `ctx.shutdown`;
  3. resolve the settings again, build a new `Ctx` (bus, status, http) and call `wire`;
  4. swap it in.

  The listener and everything in `Hub` stay. SSE streams send what was already published, then end, and clients reconnect to the new bus. If the new crates fail to start, `run` returns the error (the container or app restarts the hub).
- **Background tasks** must run through `ctx.spawn` (or select on `ctx.shutdown`). A loop that ignores it outlives the restart, but it hears only the old bus, which nobody publishes on any more.

## lull Connect

- A device is online while it holds at least one `/api/events` stream. `device.online` fires on its first stream and after its last one closes.
- `POST /api/devices/me/state` publishes `device.state` and keeps it for `GET /api/devices` while the device stays online.
- `POST /api/devices/{id}/command` needs the target online and publishes `device.command` to every stream. Only the named device acts on it; `takeover` is played out by the two clients.
- Unpairing cancels the device's streams at once.

## Elsewhere

- **The Soulseek account** lives in the soulseek crate (`account.rs`). slskd 0.26.2 can't take it over its API with an API key: the options overlay covers only the listen address and port, and `PUT /options/yaml` needs a JWT. So with `LULL_SLSKD_CONFIG_FILE` set (the desktop app's sidecar; the app keeps folders, port and API key on slskd's command line), lull owns that `slskd.yml`. At every start it writes only the account into it. When that changed the file, it waits up to 60 s for slskd to reload (logged in: `pendingReconnect`; logged out: the username in `GET /options`), then logs in again. Without the key, the account is configured where slskd runs (`SLSK_USERNAME`/`SLSK_PASSWORD` in the Docker kit), and the hub refuses the account keys.
- **The desktop app's own token** (ADR-0002): the app creates its device in-process with `core::devices::create` (kind `app`) and pairs its webview with that cookie.
