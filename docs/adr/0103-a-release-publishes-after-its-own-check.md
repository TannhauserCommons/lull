# ADR-0103: A release publishes only after its own check passed

- Status: accepted (the owner's tests and CI plan, 2026-11-26, item A1). Amends ADR-0097 and ADR-0098.
- Date: 2026-11-26

## Context

A release is a version commit on top of main's newest green commit (ADR-0097, ADR-0102). That commit is new: it changes both manifests, both lock files, Android's versionCode and the notes, and no check has seen it yet. Its pull request (`release-X.Y.Z`) runs `check / gate` on it.

Until now the tag and the branch were pushed together, and the tag publishes at once:
- `app.yml` builds the installers, the updater files, `latest.json` and the APK, and puts them on the GitHub Release;
- `release-image.yml` publishes the hub's image as `:X.Y.Z` and `:latest`;
- F-Droid and IzzyOnDroid follow the tags themselves (`packaging/fdroid/`, `UpdateCheckMode: Tags`).

For v0.2.16 and v0.2.17 the tag's runs started 11 and 19 seconds before the pull request's `check` run did, and `release-image.yml` had pushed `:latest` about 6 and 4 minutes before `gate` went green (v0.2.17: tag runs at 13:30:27, image published by 13:36:28, `gate` green at 13:40:05). Both gates passed, but nothing would have stopped a red one.

## Decision

- **The tag goes up last.** `scripts/release.sh` still makes the version commit and its tag in the clone. It prints the steps in order: push the commit to `release-X.Y.Z`, open its pull request with the App's token, wait for its checks, run `scripts/release-gate.sh` on the commit, and only then push the tag. `release.yml` pushes only the branch, starts nothing, and writes the same steps in its summary. The tag it made stays in the runner, because its job token can neither open a pull request that runs `check` nor push a tag that starts a workflow.
- **The tag's workflows refuse an early tag.** `app.yml`'s first job, `release-gate`, and `release-image.yml`'s first step run `scripts/release-gate.sh`. It passes only when the newest `gate` check run from GitHub Actions on the tag's commit completed with success; otherwise the run fails at once and names what it found. It never waits: once the gate is green, re-run the failed run. Runs of `app.yml` on main (warming caches, Gradle's checksums) pass straight through.
- **The guard job is not named `gate`.** Its own check run lands on the tag's commit, and the script looks the gate up by name.

## Rejected

- **The tag's workflows wait for the gate:** a runner polling for ten minutes, and F-Droid reads the tag whatever the workflows do.
- **Build on the tag, gate only the publishing jobs:** a tag that runs before its gate would still be public for F-Droid.

## Consequences

- **A release takes about ten minutes longer** from tag to installers: the version's pull request runs its check first.
- **The guard can't hold back F-Droid** from a tag pushed by hand too early; only the order of the steps can, so it is printed by both ways of releasing.
- **A dispatch of `release-image.yml` for a tag from before pull requests** (v0.2.15 and older, cut under the queue) is refused: those commits have no `gate`.
