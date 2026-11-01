# Workspace: ownership, seams and working in parallel

lull is built by several agents at once. This page is what keeps them from colliding. **Read it before writing code.**

## Layout and owners

Each area has one owner at a time. Edit only what your brief says you own.

| Area | What it is | Owner |
|---|---|---|
| `crates/core` | Shared contracts: config, db + migrations, `catalog`, HTTP client, event bus, service status, ids, models, seams | integrator. Others: **additive** changes only, as their own `feat(core): …` commit |
| `crates/server` | `main`, wiring, `/health`, `/api/events`, token gate, static web | integrator |
| `crates/soulseek` | slskd client, search + ranking, transfers, progressive streaming, stream cache, sharing | Soulseek session |
| `crates/library` | Jellyfin reads, saved-track streaming, track lookup, save pipeline (MusicBrainz, Cover Art Archive, tags, rescan) | Library session |
| `crates/discovery` | Feed, new releases, radio, album/artist/tag pages, taste model | Discovery session |
| `crates/lastfm` | Last.fm auth, now playing, scrobbles + retry queue | Discovery session |
| `crates/lyrics` | LRCLIB lyrics + cache | Discovery session |
| `crates/collection` | Likes, follows, plays/history, playlists + M3U, stats, settings | Collection session |
| `crates/skins` | Winamp Skin Museum proxy, `.wsz` cache, favourites | Collection session |
| `web/` (except `web/desktop/`) | The web UI, ported from `design/prototype` | Web session |
| `web/desktop/`, `desktop/`, `.github/workflows/desktop.yml` | Wrapper UI pieces and the Tauri app | Desktop session |
| `deploy/` | Compose kit (plain, VPN, existing slskd). `deploy/.env.example` keys mirror `core::config` | Soulseek session (env keys: whoever adds the key in core) |
| `Cargo.toml` (root), `Dockerfile`, `.github/workflows/ci.yml`, `mise.toml`, `AGENTS.md`, `PROJECT.md`, `README.md`, `docs/context/{INDEX,workspace,api}.md` (except your own api.md section) | Shared files | integrator |
| `design/` | Frozen design reference | nobody (read only) |

`docs/context/<page>.md` belongs to the area it describes (see INDEX.md). Each backend owner edits only its own section of `api.md`.

## How crates meet

- **Domain crates depend on `lull-core` only, never on each other.** `server` builds them in order and hands each the seams it needs:
  - `Transfers` (implemented by `soulseek`) goes to `library`, for completing a transfer before a save;
  - `Library` (implemented by `library`) goes to `soulseek`, `discovery` and `collection`, for saved badges, duplicates, the library half of search and M3U matching.
- **The event bus** (`ctx.bus`) carries everything else between crates. Every event also reaches the browser over SSE. Topics are listed in `api.md#events`. Example: `collection` publishes `play.started` / `play.ended`, and `lastfm` and `discovery` subscribe.
- **The catalog** (`lull_core::catalog`) resolves any track id. Whoever hands a track to the UI records it there.
- **Each crate exposes the same three things:**
  - `MIGRATIONS` (its own tables, prefixed with the crate name, e.g. `soulseek_peers`);
  - `async fn start(ctx, seams…) -> Result<Arc<Service>>`;
  - `fn router(svc…) -> axum::Router` with full paths (`/api/...`).

  `server/src/main.rs` already calls all of them, so you never edit it.
- **A crate reports its outside dependency** with `ctx.status.set(name, state, detail, extra)`. That drives `/health`, the `service` event and the UI's status tray.

## Working in parallel

1. **Work in your own worktree.** Commit small, then `git fetch && git rebase origin/main`, `mise run check`, and `git push origin HEAD:main`. No long-lived branches, and nobody waits for anyone's merge.
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
   | 0060-0069 | desktop |

7. **Never push red.** If `main` is red because of you, fix or revert at once. If it's red because of someone else, rebase again later; don't fix their area.
8. **Something you need is in another owner's area?** Use what exists (the seams, events, api.md). If it truly needs changing, stop and ask; don't edit it.

## Local development

- **Live testing happens on the homelab instance** (`https://lull.<tailnet>.ts.net`), the user's choice: push, let CI publish the image, and the integrator bumps the homelab pin. On this workstation the only local builds are `mise run check` and the Windows desktop wrapper; don't run a local server or slskd.
- **Windows:** set `CARGO_TARGET_DIR` outside `Desktop` (see AGENTS.md), one per worktree, and reuse it.
- **`mise run dev`** (for contributors elsewhere) reads `.env` (copy `deploy/.env.example`). In debug builds the UI is served from `web/` on disk, so a page reload picks up edits.
- **Soulseek allows one login per account.** A local slskd with the homelab's account logs the homelab out.
