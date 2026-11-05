# ADR-0062: lull's own slskd build for Windows, so songs stream there

- Status: accepted (owner, 2026-11-05)
- Date: 2026-11-05
- Changes: the desktop sidecar part of ADR-0014's pin (2026-11-04). Linux and the container keep slskd's own release.

## Context

A Windows desktop app that is its own hub couldn't play any Soulseek song: "os error 32". lull reads slskd's incomplete file while slskd writes it (streaming.md), and seeking ahead (ADR-0014) is built on the same read.

slskd opens the file it is downloading with `FileShare.None`, "exclusive access for the duration of the download" (`DownloadService.cs`). Windows enforces that against every other program: the open fails with a sharing violation, error 32. On Linux .NET's lock is advisory and lull's reads ignore it, so the container and the Linux app work. The contract test ran only the Linux image, so it couldn't see this.

Reproduced on Windows with the bundled 0.26.2: the contract test's stand-in server and peer, the peer sending slowly, and a read of the incomplete file mid-transfer.

What doesn't fix it:
- **A newer slskd.** 0.26.2 is the latest release. Every release checked opens the file this way (0.22.5 to 0.26.2), and so does master.
- **slskd's API.** It lists and deletes files, never reads one, and has no way to stream a transfer in progress.
- **A setting.** No option sets the share mode.
- **Anything on lull's side that keeps streaming.** Hard links share the lock. Opening the file before slskd only makes slskd's own open fail.

## Decision

**The Windows app bundles lull's build of slskd 0.26.2, with one line changed** (owner, 2026-11-05). `FileShare.None` becomes `FileShare.Read`: others may read the download as it arrives, and slskd alone writes it. Nothing else differs.
- **Only Windows.** Linux and the container keep slskd's own 0.26.2 release, where nothing needs changing.
- **Nothing goes upstream** (owner, 2026-11-05). slskd's authors may well want the exclusive lock.
- **Still pinned, never updated by itself.** ADR-0014's pin stands: `app/build.rs` names each zip by URL and sha256. A new slskd version means a new lull build of it.

**Built in CI, the same bytes every time.** `app/slskd/build.sh` is the recipe:
- slskd's own tag, with its commit checked;
- `share-for-reading.patch`;
- the flags of slskd's own release build, with paths in the binaries made neutral (`ContinuousIntegrationBuild`);
- a pinned .NET SDK (`app/slskd/global.json`);
- a zip with fixed dates and order.

The `slskd-windows` workflow builds twice, in different folders, and fails unless the bytes are the same. They must also match what `app/build.rs` pins, so the shipped binary is the one the public recipe makes. `slskd-windows-publish`, dispatched by hand on main, runs it and attaches the zips to a pre-release, `slskd-<build>` (for example `slskd-0.26.2-lull.1`). It is a workflow of its own because it writes: the queue calls `slskd-windows` and grants only read, and GitHub refuses to start a run whose called workflow asks for more, even for a job it would skip. The app's updater reads `releases/latest`, which never names a pre-release. `app/build.rs` downloads the zips from there.

**AGPL-3.0, kept.** Each zip holds slskd's `LICENSE`, a `NOTICE.txt` saying what lull changed and where the source is, and the patch itself. The installer ships `licenses/THIRD-PARTY.md`, which says the Windows binary is modified, how, and links the source, and `slskd-LICENSE.txt`. The source is slskd's tag plus the patch, both public, and the recipe is in this repository.

**A check catches the lock coming back.** The slskd contract (`tests/slskd-contract/`) gains a `stream` case and a native mode:
- **The `stream` case:** the stand-in peer sends slowly. While slskd downloads, the test opens the incomplete file the way lull does (read, sharing read, write and delete, as Rust's and Node's opens both do). It checks the bytes so far are the peer's, keeps the file open while slskd finishes and moves it, then reads it to the end.
- **Native mode** (`--native`): runs the binary `app/build.rs` pins for the machine, as the app runs it.
- **Where it runs:** CI runs it on Windows beside the Docker run on Linux. Against stock 0.26.2 on Windows the `stream` case fails with the sharing violation. `resume` and `overwrite` pass there too, so the patched build still seeks ahead.

## Consequences

- **Streaming and seeking ahead work on a Windows desktop hub.** Nothing changes for Linux or the container.
- **A different binary on Windows than slskd ships.** It is built from slskd's tag by a public recipe and checked by its bytes. Its version says so: `0.26.2.0+e472ecd.lull.1`.
- **A new slskd version, or a new SDK for security fixes, is a new lull build.**
  1. Change the recipe (tag, commit, patch, `global.json`, `LULL_BUILD`).
  2. Land it, then dispatch `slskd-windows-publish` on main.
  3. Pin the new zips' URLs and hashes in `app/build.rs`. The contract must pass on Linux and Windows.
- **The contract builds slskd twice** whenever the recipe, the pin or the contract changes: a few minutes of CI, beside the app's checks.

## Rejected

- **Waiting for upstream.** Windows streaming would stay broken until a release nobody has asked for.
- **Playing only finished downloads on Windows.** It works with stock slskd, but a song would start only once the whole file arrived, and there would be no seeking ahead.
- **Building it in the app's own CI each time, or on a developer's machine.** Every app build, local ones included, would need the .NET SDK. A build on one machine can't be checked by anyone else.
