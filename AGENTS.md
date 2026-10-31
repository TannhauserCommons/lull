# lull

A self-hosted music player for one listener. It streams from Soulseek while the transfer is still running, saves songs into a Jellyfin library, recommends music, scrobbles to Last.fm and has a Winamp Classic mode.
- **Server:** one Rust binary (`crates/server`) wiring domain crates.
- **Soulseek:** reached through an [slskd](https://github.com/slskd/slskd) sidecar.
- **Web UI:** buildless (`web/`), served by the binary, installable as a PWA.
- **Desktop wrapper:** Tauri (`desktop/`).
- **Shipping:** a container image, `ghcr.io/tannhausercommons/lull`, plus a compose kit in `deploy/`.

## Start here

- **Find your page.** `docs/context/INDEX.md`: find the page for your task, then open the files it lists. Don't scan the repo.
- **Read `PROJECT.md`** before changing anything (goal, non-goals, current priority). Work that doesn't serve it needs the user's OK first. A pure lookup question doesn't.
- **Read `docs/context/workspace.md`** before writing code. It covers who owns which folder, the seams between crates, and the rules that let several agents work at once without conflicts.
- **The UI design** is `design/`. Read `design/DESIGN.md` for the web UI and `design/desktop/DESKTOP.md` for the wrapper. It's a frozen reference; `web/` is what ships.

## Commands

| Task | Command |
|---|---|
| First time in a clone | `mise run setup` (Rust toolchain, Webamp bundle) |
| Everything CI runs | `mise run check` (fmt, clippy `-D warnings`, tests) |
| Tests only | `mise run test` |
| Run locally (settings in `.env`, see `deploy/.env.example`) | `mise run dev` |
| Container image | `mise run image` |

**On this Windows workstation:** set `CARGO_TARGET_DIR` to a folder outside `Desktop` (e.g. `%LOCALAPPDATA%\cargo-target\<your-worktree>`). C dependencies (bundled SQLite, TLS) fail to compile under `Desktop` with `D8050`. Use one target dir per worktree so parallel builds don't block each other; they still share compiled dependencies through the workstation's sccache (`~/.cargo/config.toml` `rustc-wrapper`), so only your own crates compile.

## Engineering rules

- **Surgical.** Touch only what the task needs: no speculative refactors, no unrelated cleanups.
- **One source of truth.** A fact lives in one place, and everything else reads it.
- **Fix the cause.** No retries, sleeps, waivers or skipped checks added just to pass.
- **Validate at the boundary, then trust.** Fail loudly on anything unknown.
- **Readable top-down.** High-level behaviour first, details in well-named helpers. Comments say *why*, briefly.
- **No dead code.** No stubs, "coming soon" or mocks in shipped code. `grep -rn SKELETON crates` lists what the scaffold left for owners to replace.
- **Few dependencies, pinned.** `Cargo.lock` and `web/package-lock.json` are committed; builds use `--locked`. Justify each new crate in its `Cargo.toml` comment.
- **No panics on input.** `unwrap`, `expect` and `panic!` are denied by clippy outside tests.
- **Tests are stingy and real-path.** Write them where a regression would hurt (ranking, streaming and cache bounds, scrobble rules, save and tagging, taste weights). Skip trivial glue. Name each test after the behaviour it pins.

## Changes

- **Git history is the changelog.** Commit as `type(scope): summary`, where scope is the crate or area (`soulseek`, `web`, `desktop`, `deploy`, `docs`). The body says why and how it was verified. No change logs, session notes or handoff files.
- **Small commits, straight to `main`.** Rebase on `origin/main`, run `mise run check`, then push. Never push red; if you break `main`, fix or revert at once.
- **Stage only the files you changed.** Other agents may have uncommitted work here.
- **Keep docs current.** A change that makes a `docs/context/` page wrong updates that page in the same commit.
- **Record decisions as ADRs.** A decision with alternatives worth remembering gets an ADR in `docs/adr/`, numbered from your range in `workspace.md`.

## Boundaries

- **Never commit** secrets, `.env*` (except `deploy/.env.example`), datasets or media. Local data lives in `Desktop\files\data\lull\`.
- **Secrets come from the user's Bitwarden** via keygate. Never ask the user to paste one.
- **Don't scrape** Rate Your Music or any site whose terms forbid it.
- **Be polite to every outside service:** it gets timeouts, backoff and caching (`lull_core::http`).
