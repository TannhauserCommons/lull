# ADR-0104: A nightly full run backs up path selection

- Status: accepted (the owner's call in the tests and CI review, 2026-11-26: keep ADR-0102's path-gated main and add a nightly full run that tunes `paths.yml` when it finds an escape). Amends ADR-0102.
- Date: 2026-11-26

## Context

Since ADR-0102's amendment, main runs only the checks whose paths changed since its newest green run, as a pull request does. Nothing then runs a check whose paths a change did not touch, so a break through a path `paths.yml` does not list (a script a check runs, a fixture it reads) stays hidden until something else selects that check. Main's own full history found 0 such breaks in 48 runs, so a full run on every push buys little; none at all leaves a missing path unseen for as long as nobody touches the check's own paths.

## Decision

- **`nightly.yml` runs once a night on main** (and by hand): `check` with `all: true` and the secrets main.yml passes, `graphics` and Lighthouse. It publishes and dispatches nothing.
- **Its `escapes` job reads what main did with each red job** (`scripts/ci/nightly-escapes.mjs`): main.yml's last 30 runs, newest first. A job main skipped since it last ran it green is an **escape**: the issue lists the commits and files since that green run, so the missing path goes into the job's key in `paths.yml`. A job main ran green at its newest run is not a path skip (a flake, or an input that is not a path); one main is red on already has its `main is red` issue; one main never runs is the nightly's own.
- **Lighthouse runs only in the nightly,** not after every merge, and keeps its LCP, CLS and TBT budgets. Its absolute shipped-bytes budgets (`tests/ui/shipped.mjs`, `shippedBytes` in `tests/ui/budgets.json`) are gone: raised 17 times, their one catch was a growth the `budgets` check (`scripts/budgets`, growth over the merge-base) now blocks at pull request time.
- **One `nightly is red` issue** (label `nightly-red`), commented on each red night and closed by the next green one. It files its own issue because `after.py`, the skeleton's, files red issues only for `main.yml`'s push runs.
- **`check`'s concurrency group carries its caller's name,** so main's check and the nightly's, both on `refs/heads/main`, don't cancel each other.

## Rejected

- **Main running every check on every push** (before ADR-0102's amendment): about 145 job-minutes a push, mostly repeating what the pull request passed.
- **Sharing main.yml's concurrency group:** a waiting nightly and a waiting push would replace each other, losing a push's publish or a night's run.
- **Keeping Lighthouse's byte ceilings beside the `budgets` check:** two budgets for the same bytes, one of them raised by hand on every feature.
- **A finer escape analysis** (replaying `paths.yml` over each commit): main's own runs already say what was skipped, and a red nightly with those commits is enough to fix the map.

## Consequences

- **A break behind a path skip shows within a day,** not whenever its paths next change. Main stays green meanwhile, since main's run skipped the check; the fix is a fix or a revert, and a line in `paths.yml`.
- **The nightly is not a release gate.** `scripts/release.sh` reads main.yml's green runs only; read the open `nightly is red` issue before releasing.
- **No infra rerun or flake filing for the nightly:** `after.yml` reacts to `check` and `main` only, and is the skeleton's to change.
