# ADR-0097: Releases come from the newest green commit and build from warm caches

- Status: accepted
- Date: 2026-11-04

## Context

Releasing 0.2.2 on 2026-11-04 took hours:
- `mise run release` tags `HEAD`, which must equal `origin/main`. Main was red more often than green, so the integrator waited for a green moment, re-checked, and tagged by hand.
- The tag's build took about 26 minutes (measured that day, `app` run 41815456354): Linux x64 11, Linux arm64 15, Windows arm64 22, Windows x64 26 minutes for the bundles, and Android 13 (two builds one after the other). Every bundle compiled every crate from nothing, because Actions caches are scoped to the ref: a tag can read main's caches, but main never builds the release profile, so there was nothing to read.
- `ci`'s and `ui`'s images each compiled the hub's dependencies inside Docker every time (5 to 6 minutes): BuildKit's cache mounts aren't kept between runners.

## Decision

### `mise run release X.Y.Z` tags the newest green commit

- **It finds the commit:** the newest commit on `origin/main` whose `queue` status is success (ADR-0095), so any local or unpushed work is ignored. Under the queue that is main's tip, unless a run is landing it right now. Before the queue, it was the newest commit whose check runs all passed, and the script reads those too.
- **It makes the release commit on that commit, not on `HEAD`:** the version in both manifests, Android's versionCode, and the notes. Then it tags it `vX.Y.Z`.
- **Then the person checks the notes and pushes:** the script prints `git push origin vX.Y.Z vX.Y.Z^{commit}:refs/heads/queue/release-X.Y.Z`. The tag starts the build at once. The queue lands the version commit on main behind whatever landed meanwhile, so main's version always rises; the tag needs no rebase.
- **The notes are drafted from the `feat` and `fix` subjects** since the previous release tag (by version, not `git describe`, because a tag may sit just off main's line). They go into the fastlane changelog (500 characters at most), as now. The script stops if the draft is new, so a person reads it and edits it before the tag exists. A second run, with the file in place, tags.

### Builds read caches that main warmed

- **Rust is cached per platform and profile,** with `Swatinem/rust-cache`. Only main's runs save (the queue runs on main), and pushes elsewhere only read. Workspace crates aren't cached, only dependencies, so a cache stays valid until `Cargo.lock` or the toolchain changes.
- **Main warms the release builds:** when the queue lands a change to `Cargo.lock`, `app/Cargo.lock`, `rust-toolchain.toml` or the app's build profile, and once a week, `app.yml` builds the four bundle targets' release dependencies on main. A tag then compiles only lull's own crates and links.
- **The image's dependencies are their own Docker layer** (`cargo-chef`): a recipe from the manifests, built once into a layer the GitHub Actions cache keeps, so an image build compiles only lull's crates.
- **Android's two builds run side by side:** the signed build and the clean-clone build are separate jobs, and a third compares them with `apksigcopier`, as F-Droid will. The clean build stays clean; that is the point of it.
- **Bundles run only on a tag or by hand,** as now.

Target: the app's checks in a queue run under 10 minutes, and a release's bundles in about 10.

## Rejected

- **sccache on the Actions cache:** it caches every compiler call, but writes thousands of small entries that the 10 GB cache evicts in a day; rust-cache's one archive per platform survives.
- **Saving caches from tags:** another tag can't read them.
- **Building the release profile on every queue run:** 4 platforms for 10 to 25 minutes each, for every change.
- **Tagging `HEAD` after waiting for green:** the wait is what took hours.

## Consequences

- **A release is two commands and one read of the notes,** at any time, whatever is in the queue.
- **The tag may sit one commit off main's line** when main moved while it was made. `git describe` then doesn't find it; the release script reads tags by version instead.
- **Fat LTO stays** (`app/Cargo.toml`): it is the one step a cache can't skip, and measuring it decides whether it is worth its minutes. Changing it is a separate decision with numbers.
- **Cache size:** four bundle targets, two app checks, `ci` and the image fit in the 10 GB the repository gets; the queue's ADR keeps each saved only from main.
