# lull

A music app for one listener who hosts it themselves (PROJECT.md). It streams from Soulseek while the transfer is still running, saves songs into a music folder or Jellyfin, recommends music, scrobbles to Last.fm and has a Winamp Classic mode. Devices pair with the listener's hub; there are no accounts.
- **Hub:** the Rust server (`crates/server`, a library plus the `lull` binary) wiring domain crates.
- **Soulseek:** reached through an [slskd](https://github.com/slskd/slskd) sidecar.
- **Web UI:** buildless (`web/`), served by the binary, installable as a PWA.
- **Apps:** one Tauri v2 project (`app/`) for Windows, Linux and Android (ADR-0006).
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
| After changing `web/` (CI runs it too) | `mise run check:ui`: performance budgets, key screens, accessibility ([quality.md](docs/context/quality.md#ui-gates)) |
| Run locally (settings in `.env`, see `deploy/.env.example`) | `mise run dev` |
| Container image | `mise run image` |
| Land your commits on main | `mise run ship <topic>` |
| Cut a release (on the owner's word only) | `gh workflow run release.yml -f version=X.Y.Z` (quality.md) |

**On this Windows workstation:** set `CARGO_TARGET_DIR` to a folder outside `Desktop` (e.g. `%LOCALAPPDATA%\cargo-target\<your-worktree>`). C dependencies (bundled SQLite, TLS) fail to compile under `Desktop` with `D8050`. Use one target dir per worktree so parallel builds don't block each other; they still share compiled dependencies through the workstation's sccache (`~/.cargo/config.toml` `rustc-wrapper`), so only your own crates compile.

## Engineering rules

- **Quality over development cost.** When making technical decisions, do not give much weight to development cost. Instead, prefer quality, simplicity, robustness, scalability, and long term maintainability.
- **Surgical.** Touch only what the task needs: no speculative refactors, no unrelated cleanups. Visual defects, lint, and failing or flaky tests you come across are the exceptions (next two rules).
- **Pixel-perfect when testing end to end.** When end-to-end testing, be picky about the UI you see and be obsessed with pixel perfection. If something clearly looks off, even if it is not directly related to what you are doing, try to get it fixed along the way.
- **Same standard for engineering.** Apply that same high standard to engineering excellence: lint, test failures, and test flakiness. If you see one, even if it is not caused by what you are working on right now, still get it fixed.
- **One source of truth.** A fact lives in one place, and everything else reads it.
- **Fix the cause, where it starts.** Don't use special cases, arbitrary size or threshold changes, retries, sleeps, weakened checks, or error suppression to hide symptoms. Trace the failure to its origin and correct it there.
- **Then fix its siblings.** When the underlying cause is shared or likely to exist elsewhere, search for other occurrences and fix every confirmed instance. Keep the search proportional to the likelihood and impact of recurrence. Don't perform repository-wide searches for clearly isolated defects. `docs/context/INDEX.md` names where each kind of code lives: callers of the fixed code, and its parallel copies (the five designs, the crates, Windows, Linux and Android). Search those places for the cause's pattern with `rg` instead of reading files.
- **Validate at the boundary, then trust.** Fail loudly on anything unknown.
- **Readable top-down.** High-level behaviour first, details in well-named helpers. Comments say *why*, briefly.
- **No dead code.** No stubs, "coming soon" or mocks in shipped code. `grep -rn SKELETON crates` lists what the scaffold left for owners to replace.
- **Few dependencies, pinned.** `Cargo.lock` and `web/package-lock.json` are committed; builds use `--locked`. Justify each new crate in its `Cargo.toml` comment.
- **No panics on input.** `unwrap`, `expect` and `panic!` are denied by clippy outside tests.
- **Tests are stingy and real-path.** Write them where a regression would hurt (ranking, streaming and cache bounds, scrobble rules, save and tagging, taste weights). Skip trivial glue. Name each test after the behaviour it pins.

## Changes

- **Git history is the changelog.** Commit as `type(scope): summary`, where scope is the crate or area (`soulseek`, `web`, `app`, `deploy`, `docs`). The body says why and how it was verified. No change logs, session notes or handoff files.
- **Small commits, landed by the queue.** `mise run ship <topic>` pushes to `queue/<topic>`, which tests it on top of main and lands it only if green ([quality.md](docs/context/quality.md#the-queue-main-is-always-green)). Never push to `main`; it refuses. Red: fix it on your branch and ship again.
- **Name what it holds.** Topics and branches say what they hold (`loops-in-css`), never the worktree's generated name: the repository is public. A branch's checks run by hand (docs/context/quality.md).
- **Big work lands switched off** (`lull_core::flags`, ADR-0096; workspace.md), so a release never waits for it. Finishing it removes the switch.
- **A flaky check is a bug:** its area's owner fixes the cause that day. CI gives a failing UI test one retry, and one that passes on it gets a `flaky` issue instead of holding the landing; nothing else is retried, and no budget is loosened (quality.md, Flaky checks).
- **Stage only the files you changed.** Other agents may have uncommitted work here.
- **Keep docs current.** A change that makes a `docs/context/` page wrong updates that page in the same commit.
- **Record decisions as ADRs.** A decision with alternatives worth remembering gets an ADR in `docs/adr/`, numbered from your range in `workspace.md`.

## Boundaries

- **Never commit** secrets, `.env*` (except `deploy/.env.example`), datasets or media. Local data lives in `Desktop\files\data\lull\`.
- **Nothing that points to the owner,** in files or commit messages: no name, country or region, time zone, network or host name, or personal path. Say "from the homelab" or "on the hub", never where that is. Commit with `TZ=UTC`. CI's `privacy` job (`scripts/privacy-check.sh`) checks every file and commit against a list kept in a secret.
- **Secrets come from the user's Bitwarden** via keygate. Never ask the user to paste one.
- **Don't scrape** Rate Your Music or any site whose terms forbid it.
- **Be polite to every outside service:** it gets timeouts, backoff and caching (`lull_core::http`).
