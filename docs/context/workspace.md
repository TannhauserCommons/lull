# Workspace: ownership, seams and working in parallel

lull is built by several agents at once. This page is what keeps them from colliding. **Read it before writing code.**

## Layout and owners

Each area has one owner at a time. Edit only what your brief says you own.

| Area | What it is | Owner |
|---|---|---|
| `crates/core` | Shared contracts: config, settings store, devices, db + migrations, `catalog`, HTTP client, event bus, service status, ids, models, seams | integrator; Hub session owns `settings.rs` and `devices.rs`. Others: **additive** changes only, as their own `feat(core): …` commit |
| `crates/server` | The hub: `lib.rs` (`run`, wiring), gate and pairing, `/health`, `/api/events`, devices and Connect, `/api/config`, setup | Hub session; `src/web.rs` is the Quality session's; the Subsonic session adds its own wiring lines in `lib.rs` |
| `crates/soulseek` | slskd client, search + ranking, transfers, progressive streaming, stream cache, sharing | Soulseek session (idle) |
| `crates/youtube` | YouTube through yt-dlp: matching, native audio downloads, keeping yt-dlp current (the `Youtube` seam; soulseek decides when it plays) | Soulseek session (idle) |
| `crates/library` | Library sources (folder, Jellyfin), switching and reconciliation, Quick Connect, the Jellyfin mirror, saved-track streaming, save pipeline | Library session |
| `crates/discovery` | Feed, new releases, radio, album/artist/tag pages, taste model | Discovery session (idle) |
| `share/`, `.github/workflows/share.yml` | Share links on Cloudflare: the card pages, covers, App Links statement (share.md, ADR-0034) | Discovery session |
| `crates/lastfm` | Last.fm auth, now playing, scrobbles + retry queue, the built-in app key | Collection session |
| `crates/lyrics` | LRCLIB lyrics + cache | Discovery session (idle) |
| `crates/collection` | Likes, follows, plays/history, playlists + M3U, stats, settings, the `Collection` seam, `/api/sync`, imports | Collection session |
| `crates/skins` | Winamp Skin Museum proxy, `.wsz` cache, favourites | Collection session |
| `crates/subsonic` | OpenSubsonic API over the seams | Subsonic session |
| `crates/themes` | Stored theme packages, import/export, live reload | Themes session |
| `web/designs/`, `web/tokens.css`, `web/base.css`, `web/screens/appearance.js`, `docs/themes.md` | Theme packages, cascade layers, the five designs, Theme Studio | Themes session |
| `web/` (everything else, including `web/desktop/`) | The web UI: setup, pairing, devices and Connect, offline downloads + outbox, library source, imports, the player's output interface | Web session |
| `app/src/desktop/`, the desktop entries of `app/` config, desktop jobs in `.github/workflows/app.yml` | Windows and Linux app: hub in-process, bundled slskd, tray, mini player, updater, Tailscale serve | Desktop session |
| `app/src/mobile/`, `app/plugins/player/`, `app/gen/android/`, `app/tauri.android.conf.json`, Android jobs in `app.yml` | Android app: native player plugin, downloads, pairing | Mobile session |
| `app/src/` shared files (`lib.rs`, `main.rs`, pairing, discovery), `app/Cargo.toml`, `app/tauri.conf.json` | Shared app code and manifests | Desktop session; Mobile adds only under `[target.'cfg(target_os = "android")'.dependencies]` and its own modules' `mod` lines |
| `tests/ui/`, `.github/workflows/ui.yml`, the Dockerfile web stage | Minify + compression, fixture library, visual matrix, Lighthouse budgets | Quality session |
| `deploy/` | Compose kit (plain, VPN, existing slskd). `deploy/.env.example` keys mirror `core::config` | Soulseek session (env keys: whoever adds the key in core) |
| `Cargo.toml` (root), `Dockerfile` (except its web stage), `.github/workflows/ci.yml`, `mise.toml`, `AGENTS.md`, `PROJECT.md`, `README.md`, `docs/context/{INDEX,workspace,api}.md` (except your own api.md section) | Shared files | integrator |
| `design/` | Frozen design reference | nobody (read only) |

`docs/context/<page>.md` belongs to the area it describes (see INDEX.md). Each backend owner edits only its own section of `api.md`.

## How crates meet

- **Domain crates depend on `lull-core` only, never on each other.** `server` builds them in order and hands each the seams it needs:
  - `Transfers` (implemented by `soulseek`) goes to `library`, for completing a transfer before a save;
  - `Library` (implemented by `library`) goes to `soulseek`, `discovery`, `collection` and `subsonic`, for saved badges, duplicates, the library half of search, M3U matching and Subsonic's browsing, streaming and covers;
  - `Collection` (implemented by `collection`) and `Lyrics` (implemented by `lyrics`) go to `subsonic`, so Subsonic apps like, scrobble, edit playlists and read lyrics as the web UI does;
  - `Youtube` (implemented by `youtube`) goes to `soulseek`, whose fetches write YouTube copies into the stream cache.
- **The event bus** (`ctx.bus`) carries everything else between crates. Every event also reaches the browser over SSE. Topics are listed in `api.md#events`. Example: `collection` publishes `play.started` / `play.ended`, and `lastfm` and `discovery` subscribe.
- **The catalog** (`lull_core::catalog`) resolves any track id. Whoever hands a track to the UI records it there.
- **Each crate exposes the same three things:**
  - `MIGRATIONS` (its own tables, prefixed with the crate name, e.g. `soulseek_peers`);
  - `async fn start(ctx, seams…) -> Result<Arc<Service>>`;
  - `fn router(svc…) -> axum::Router` with full paths (`/api/...`).

  `server/src/lib.rs` calls all of them.
- **A crate reports its outside dependency** with `ctx.status.set(name, state, detail, extra)`. That drives `/health`, the `service` event and the UI's status tray.

## Working in parallel

1. **Work in your own worktree.** Commit small, then `git fetch && git rebase origin/main`, `mise run check`, and `mise run ship <topic>`: the queue lands it on main once its checks pass (quality.md). No long-lived branches, and nobody waits for anyone's merge.
2. **Cargo.lock conflicts** are expected and trivial: take upstream's (`git checkout --theirs Cargo.lock` during the rebase), run `cargo check` to put your dependencies back, `git add`, continue.
3. **Changes to `crates/core` are additive only.** That means new items, new optional fields with defaults, and new config keys with defaults. Never rename or remove; ask the integrator instead. Push them in their own commit straight away so others pick them up on their next rebase.
4. **Dependencies.** Shared versions live in the root `[workspace.dependencies]` (integrator). A dependency only your crate uses goes in your crate's `Cargo.toml` with an exact version and a one-line reason.
5. **Migrations.** Only your crate's `MIGRATIONS`. A shipped step is never edited; add a new one.
6. **ADR numbers come from your range,** so files never collide:

   | Range | Owner |
   |---|---|
   | 0001-0009 | integrator |
   | 0010-0019 | soulseek |
   | 0020-0029 | library |
   | 0030-0039 | discovery, lastfm, lyrics |
   | 0040-0049 | web |
   | 0050-0059 | collection, skins |
   | 0060-0069 | app (desktop, mobile) |
   | 0070-0079 | subsonic |
   | 0080-0089 | themes |
   | 0090-0094 | hub |
   | 0095-0099 | quality |

7. **Big work lands switched off** (ADR-0096). Add a `Flag` to `lull_core::flags` in the work's first commit; the hub asks `ctx.config.flags.on(&X)`, the web UI `App.flag('x')`. With it off, lull must be exactly what it was: no new request, element, setting or cost, so the goldens and budgets don't move. You try it with `LULL_FLAGS=x` (the hub's environment, or `PATCH /api/config`; the fixture hub reads the same variable). Finishing the work removes the flag and the code paths for off; `grep -rn "flags.on(\|App.flag(" crates web` lists what is still dark.
8. **Main can't go red;** the queue tests every change before it lands. If yours comes back red, fix it on your branch. If a check fails for no change of yours, it's flaky: tell its area's owner (quality.md, Flaky checks).
8. **Something you need is in another owner's area?** Use what exists (the seams, events, api.md). If it truly needs changing, stop and ask; don't edit it.

## Local development

- **Live testing happens on the homelab instance** (`https://lull.<tailnet>.ts.net`), the user's choice: push, let CI publish the image, and the integrator bumps the homelab pin. On this workstation the only local builds are `mise run check` and the Windows desktop wrapper; don't run a local server or slskd.
- **Windows:** set `CARGO_TARGET_DIR` outside `Desktop` (see AGENTS.md), one per worktree, and reuse it.
- **`mise run dev`** (for contributors elsewhere) reads `.env` (copy `deploy/.env.example`). In debug builds the UI is served from `web/` on disk, so a page reload picks up edits.
- **Soulseek allows one login per account.** A local slskd with the homelab's account logs the homelab out.
