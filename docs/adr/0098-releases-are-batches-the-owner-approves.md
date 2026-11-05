# ADR-0098: Releases are batches of related updates, which the owner approves

- Status: accepted
- Date: 2026-11-05

## Context

lull is for every listener, not only the owner's hub. Changes reach the others only through releases: the desktop app and its built-in hub, the Android app (directly and through F-Droid), and self-hosters' images. Since the queue (ADR-0095) and releases from the newest green commit (ADR-0097), a release costs one command and about 20 minutes of CI, so how often to release is now a choice, not a cost.

The owner's choice, in their words: "batch releases grouping new updates". A release is a batch of updates that belong together, such as the new designs, or search and discovery, not a fixed weekly clock. 0.2.3 went out on 2026-11-05 with what was on main; the designs are 0.2.4.

## Decision

### A standing draft of the next release, grouped by area

`release-draft.yml` keeps one GitHub draft Release, titled "Next release", up to date. It runs after every landing that brought a `feat` or `fix` (the queue starts it), and by hand. It holds:
- **The proposed version:** the next patch, as lull's releases have gone so far (0.2.1 to 0.2.4, features and all). Whoever runs the release may give another.
- **The landed updates since the last release, grouped by area.** The area comes from the commit's scope, through one table in `scripts/release-notes.sh`:

  | Area | Scopes |
  |---|---|
  | Search and discovery | discovery, search, youtube, lastfm, lyrics |
  | Playing and sources | soulseek, streaming, library, collection, player |
  | Designs and themes | web, designs, themes, skins |
  | The apps | app, desktop, android |
  | The hub | hub, server, core, subsonic, deploy |

  Scopes that never reach a listener (`queue`, `ship`, `release`, `ci`, `docs`, `test`, `build`) are left out. A scope missing from the table makes the job fail, so the table can't go stale quietly.
- **Whether each area's batch is complete.** The planned pieces still unfinished are exactly the switches in `lull_core::flags` (ADR-0096); each names its area. An area with no dark switch is complete and can ship; one with a switch says which piece it waits for.
- **The batch's name, when a person gives one** ("the new designs"): set on the draft's first line, kept from run to run, and it leads the notes.
- **The notes, drafted:** the 500-character "what's new" the stores show: the batch's name, then the complete areas' subjects. A person may edit them in the draft.

### The owner approves each release in chat

The owner is not expected to run workflows. They say "release it" in chat, for that release; the integrator session then runs `release.yml` on that word (CI dispatch is delegated to it). Agents prepare releases and never cut one on their own initiative. Running `release.yml` from the Actions tab or the GitHub phone app is the second way, documented for when no session is around.

`release.yml` (run with the version) does what `mise run release` does (ADR-0097) on the newest green commit: the version commit with the notes, the tag, and the version commit queued for main. A tag pushed with the job's token starts no workflow, so it then starts `app.yml` on the tag: installers, the signed updater files, the APK and the GitHub Release.

### Self-hosters follow releases

`release.yml` also builds the hub's image from the tag and publishes it as `:X.Y.Z` and `:latest`. `:main` and `:sha-<short>` stay what the queue publishes, for the homelab and anyone who wants every landing. `deploy/` and docs/context/deploy.md point self-hosters at `:latest`.

## Rejected

- **A release on a fixed clock (weekly):** it splits a feature's batch across releases, or ships half of one; the owner asked for batches.
- **Releasing on every landing:** installed apps can't be rolled back, and every release asks each listener to update.
- **Agents releasing on their own:** a release reaches every listener's device, so it stays the owner's decision.
- **The owner running the workflow:** they are not expected to; a run on their word is the same release.

## Consequences

- **The owner sees what's ready at a glance** and releases with one word in chat. The draft is never empty for long, so nothing waits for someone to remember.
- **Every switch names its area,** so a batch is complete when its area's switches are gone. Work that doesn't use a switch can't say it is unfinished; small changes don't need to.
- **Self-hosters' `:latest`** moves only with releases, at the pace of the apps.
- **The scope table is one more thing to keep current;** the job fails on an unknown scope, which says so the day it appears.
