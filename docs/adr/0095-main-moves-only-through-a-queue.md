# ADR-0095: main moves only through a queue that tests what it lands

- Status: accepted; in force since 2026-11-05 (the cutover, 9705ae7d, and main's ruleset)
- Date: 2026-11-04

## Context

About seven agent sessions push straight to `main`. On 2026-11-04 that cost hours:
- every push started `ci`, `ui`, `ui-gates` and `app` again, and on main a running run finishes while the newest push waits, so a green main was always a few pushes behind;
- a check that crept or flaked (the chrome fitting a frame late, Lighthouse's bytes, Starlight's frames at rest) turned main red for everyone, and nobody could tell whose push did it;
- the integrator chased the newest green commit by hand to release 0.2.2 and to pin the homelab.

Measured on main that day (GitHub-hosted runners): `ci` about 8 minutes (checks 3, image 5), `ui-gates` about 8, `ui` about 20 (image build 6, visual matrix 9, Lighthouse 3), `app` about 15 (checks 4, Android 15).

GitHub's own merge queue needs an organisation. lull lives under the user account TannhauserCommons, and moving it would break the `tannhausercommons.github.io` share links (ADR-0032). So the queue has to be built from Actions.

## Decision

### Sessions push to `queue/<topic>`, never to main

`mise run ship` pushes `HEAD` to `queue/<topic>` and waits for the verdict: landed (main's new commit), or red (the failing check's link). The topic says what the change holds, as topic branches do now.

### One run at a time tests a candidate and fast-forwards main

`.github/workflows/queue.yml` is the only thing that moves main.
- **Started by a push to `queue/**`** through a one-step workflow that dispatches `queue.yml` on main. Running on main matters: Actions caches are scoped to the ref, and a run on `queue/<topic>` could neither use nor warm main's Rust and Docker caches.
- **Serialized** by one concurrency group, never cancelling. Actions keeps at most one run waiting, but nothing is lost: every run takes every branch waiting in `queue/`, not only the one that started it.
- **Batched.** The run rebases each waiting branch, oldest first, onto main and onto the ones before it. That stack is the candidate. A branch that doesn't rebase cleanly is set aside with "conflicts with main" and the rest go on. When the queue backs up, one run tests many branches for the price of one.
- **Tests only what changed.** The candidate's diff against main picks the checks, with the filters the workflows have today:

  | Check | When the diff touches |
  |---|---|
  | `ci` (fmt, clippy, tests, the web's tests) | anything but docs, `*.md`, `design/`, `fastlane/` |
  | `ui-gates` (4 machines) | `web/`, `tests/ui/` |
  | `ui` (visual matrix, Lighthouse) | `web/`, `crates/server/src/web.rs`, `tests/ui/`, `scripts/web-dist/`, `Dockerfile` |
  | `app` checks (Linux, Windows) | `app/`, `crates/`, `web/desktop/`, `web/assets/icons/`, `rust-toolchain.toml` |
  | `app` Android | `app/gen/android/`, `app/src/mobile/`, `scripts/android-*.sh`, `app/tauri.android.conf.json`, `rust-toolchain.toml` |
  | `slskd-contract` | its paths today |

  A queue change (`.github/`, `scripts/queue*`) runs them all. The check workflows become reusable (`workflow_call` with the candidate's commit); they keep `workflow_dispatch` for a branch's checks by hand and `pull_request` for outside contributors.
- **One image, tested and shipped.** When the diff reaches the hub, one job builds the image once. `ui` tests that image, and after landing the same image is pushed as `:main` and `:sha-<short>`. The image a listener runs is the one the UI checks passed.
- **Green: main fast-forwards.** The run sets the commit status `queue` to success on the candidate, pushes it to main as a fast-forward, deletes each landed branch (only if its tip is still what was tested), publishes the image, and starts Pages when `site/` changed. Pushes made by the run's own token start no workflows, so the run starts what main's pushes used to start.
- **Only the queue publishes.** No workflow publishes the image or starts Pages on a push to main any more (`ci`'s image job goes, and Pages runs only when the queue starts it), and the ruleset below refuses a direct push anyway. So the image the homelab pins is always one the queue tested.
- **Red: main stays.** A single branch's red sets `queue` to failure on its tip, with the failing run's link; `mise run ship` prints it. The branch waits until its owner pushes a fix to it. A red batch of several is split: each branch is retried alone, oldest first, so one bad change never holds the others back.

### Main is protected as far as a user's public repository allows

A ruleset on `main`:
- requires the status `queue`, and only from GitHub Actions (so a status set with a person's token doesn't count);
- blocks force pushes and deletion, and requires linear history;
- has no bypass list, so the owner's token is held to it too.

A direct `git push origin HEAD:main` is refused unless the queue already tested that exact commit. Turning the ruleset off in the repository's settings is the owner's escape hatch.

Why a required status and not a list of who may push: on a repository owned by a person, GitHub refuses GitHub Actions as a ruleset's bypass actor ("must be part of the ruleset source or owner organization"), so main can't be limited to the queue's identity. A required status from GitHub Actions does the same job. Probed on 2026-11-04 with a ruleset on test branches only: a commit without the status was refused, a commit the queue had landed was accepted, and the sessions' token can't set a commit status at all.

### Flaky checks get an owner and a fix

A check that fails without a change to blame gets an owner and a fix at its cause, the same day. It never gets a retry, a re-run until green or a loosened budget. While it is being fixed it stays required; a check that can't be trusted can't guard main. The policy lives in `docs/context/quality.md`.

## Rejected

- **Moving lull to an organisation** for GitHub's queue: it breaks the share links.
- **Pull requests with required checks and auto-merge:** they need a review or an up-to-date branch, so each PR re-runs everything after every merge: the same chase, slower.
- **Keep pushing to main, revert on red:** main is still red between the push and the revert, and every other session's runs are wasted meanwhile.
- **A run per branch (no batching):** with seven sessions and a 20-minute `ui`, the queue would back up for hours.
- **Running the queue on `queue/<topic>` refs:** every run would start with cold Rust and Docker caches.

## Consequences

- **Main is always green,** so anything that wants "the newest good lull" takes main's tip: the homelab's pin (homelab ADR-0028) and releases (ADR-0097).
- **A change lands in about the time its slowest check takes,** as now, plus a minute to rebase and publish. Batching keeps that true when sessions push at once. The image is built once instead of twice (`ci` and `ui` each built it).
- **Sessions no longer rebase and push main themselves.** AGENTS.md and workspace.md say `mise run ship`. A branch that conflicts is the session's to rebase and push again.
- **Branch checks by hand still work** (`gh workflow run ci --ref <branch>`), for a change not ready to queue.
- **Open to what may come next** (researched, not yet decided): a flake list where a gate that fails then passes is quarantined with an owner and a removal date; flagging a change that edits goldens or gate thresholds together with the code they judge; actions pinned by commit hash. Each would be a step in `queue.sh` or a check in its own workflow, without changing how the queue lands.
- **A change to the checks is tested with main's checks.** Actions reads a called workflow at the commit that calls it, so the queue runs main's `.github/workflows/`; a changed workflow is run by hand on its branch before it queues (quality.md).
- **A red check ends the run at once.** A watch job beside the checks gives the verdict on the first failure and cancels the rest, so a red batch is split minutes sooner.
- **The queue is a single point of failure.** If it is broken, the owner turns the ruleset off and pushes the fix directly; the queue's next runs then run the fixed copy.
