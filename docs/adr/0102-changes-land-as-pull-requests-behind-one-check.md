# ADR-0102: Changes land as pull requests behind one check

- Status: accepted (the owner's call in the CI review, 2026-11-23: remove the queue and its custom code). Supersedes ADR-0095; amends ADR-0097. Amended 2026-11-24: main's check is path-gated against its newest green run, not full.
- Date: 2026-11-23

## Context

ADR-0095's queue tested every change on top of main before landing it, and it held main green. Running it cost more than it saved:
- **It was lull's own code to keep up:** `queue.yml`, `queue-kick.yml`, `scripts/queue.sh` (planning, batching, halving on red, landing, retrying GitHub's API), `queue-first.sh`, and `main-full.yml` for the checks it skipped. A fault in any of them stopped every landing, and only lull had them to fix.
- **Its checks ran three times:** each `queue/**` push started the queue run, the shadow `check.yml` and the per-area workflows' own pull request runs, against 20 jobs at once. Waits came from full runners, not from tests.
- **Its last day showed the fault:** two runs tested a topic green, then had the land push refused ("Required status check 'queue' is expected") 2 s after writing that status, and each retested from scratch about every 20 minutes without converging.

The shared CI skeleton (the `ci-skeleton` project) gives every project the same path: `check.yml` with one `gate` status, `main.yml` after every merge and `after.yml` for what goes red there. lull moved onto it in two steps: main.yml and after.yml first, then `mise run ship` and `mise run merge`.

## Decision

- **Every change is a pull request, and main's ruleset requires `gate` from GitHub Actions,** with linear history and no force push. `mise run ship <topic>` opens it with the App's token and waits for `check / gate`; `mise run merge <pr>` rebases it onto main, pinned to the head that passed. Who may merge without the owner's yes is merge-policy's.
- **`check.yml` is the one check,** path-gated by `.github/ci/paths.yml`: it holds what `ci`, `privacy`, `ui`, `ui-gates`, `app`, `aur`, `slskd-contract`, `server-package` and `share` checked. Those workflows lose their pull request runs and the queue's calls; what they still do is make releases (`app`, `server-package`, `aur`, `slskd-windows`), deploy (`share`, `pages`), or take new goldens by hand (`ui`, `ui-gates`).
- **`main.yml` runs on main after each merge the checks whose paths changed since its newest green run** (every check when there is none, and the `image` job always), publishes the image only from a green run, and starts the jobs the queue ran after a landing. `graphics` and Lighthouse run there, after the merge (the owner's call): timing belongs to main's quiet runners.
- **`after.yml` keeps one `main is red: <job>` issue per failed check on main** and files flaky tests from `check`'s artifacts. It reruns a failure that is all infrastructure once (the owner's call, 2026-11-14), the one retry ADR-0095 forbade.
- **Caches are saved only by main's runs** and restored by pull requests, as before; `check` now saves the pinned downloads the deleted workflows used to save.
- **ADR-0097's "newest green"** is the newest commit on main whose `main.yml` run passed (`scripts/release.sh`), falling back to the queue's `queue` and `full` statuses for older commits. The version commit goes to the branch `release-X.Y.Z` and lands by its pull request.

## Rejected

- **Keeping the queue for its stacking:** it caught breaks between changes, but none that only the stack caught was found in 176 topics (not checked one by one), and it was the part that failed.
- **GitHub's merge queue:** it needs an organisation, which would break the share links (ADR-0032), as ADR-0095 found.
- **Moving `graphics` and Lighthouse into `check`:** two more jobs on every pull request for checks that caught one real bug in a month.

## Consequences

- **Two green pull requests can break main together,** since each was checked on its own head. `main.yml` shows it within one run, `after` files it, and the fix is a fix or a revert, revert by default. `:main` publishes only from a green run, so no install pulls a red build.
- **Privacy is checked on the pull request's head,** and the merged commit is made at merge time; main's own `privacy` job catches a leak there only after it is public. The rebase merge keeps the commits' own UTC dates.
- **No `full` status any more.** Main's run is path-gated like a pull request's, against `main.yml`'s newest green run (`base`), so a red job stays selected until a run is green. Main's own full history (`main-full`) found 0 real breaks behind a path skip in 48 working runs; a full pull request run is about 122 job-minutes and a full main run about 145, on 20 runner slots.
- **Pull requests are opened by the App, never a person's account,** whose time zone GitHub writes into the test-merge commit of an open pull request.
