#!/bin/sh
# What main brought since its after-landing jobs last ran, for main.yml's: `sh scripts/ci/main-plan.sh`
# (`mise run main:plan`). Reads BEFORE (main's tip before the push) and SHA (its tip now) and writes one
# `name=true|false` line each to $GITHUB_OUTPUT, or to stdout when that isn't set.
#
# In main.yml (GITHUB_REPOSITORY and GH_TOKEN set) it plans from main.yml's newest green run on main instead of BEFORE:
# a waiting run that a newer push replaced never ran, and a red run skipped its after-landing jobs, so what those
# pushes brought is planned here. BEFORE stays the base when no green run is an ancestor of SHA (the first run, or a
# re-run of an older one) and for a run by hand.
#
# It also writes `base=<sha>`, that green run's commit, which main.yml hands to check.yml: main's check runs the jobs
# whose paths changed since it. `base=` is empty when no green ancestor was found or the lookup failed (BEFORE is never
# used for that), and check then runs every job.
set -eu

die() { echo "main-plan: $*" >&2; exit 1; }

before=${BEFORE:-}
base=
sha=${SHA:-}
[ -n "$sha" ] || die "SHA is not set"
# A push that made main (no tip before) or lost its old tip can't say what it brought; the run fails instead of
# guessing, and shows it.
[ -n "$before" ] && ! printf '%s' "$before" | grep -Eq '^0+$' || die "the push has no tip before it"
git cat-file -e "$before^{commit}" 2>/dev/null || die "$before isn't in the clone: check out with fetch-depth 0"
if [ -n "${GITHUB_REPOSITORY:-}" ]; then
  # A lookup that fails finds no green run: main then checks every job, which is never less than it needs.
  green=$(gh api "repos/$GITHUB_REPOSITORY/actions/workflows/main.yml/runs?branch=main&event=push&status=success&per_page=1" \
    --jq '.workflow_runs[0].head_sha // empty') || { echo "main-plan: main.yml's runs couldn't be read" >&2; green=; }
  if [ -n "$green" ] && git merge-base --is-ancestor "$green" "$sha" 2>/dev/null; then
    before=$green
    base=$green
  fi
fi
echo "main-plan: from $before to $sha"

files=$(git diff --name-only "$before" "$sha")
out=${GITHUB_OUTPUT:-/dev/stdout}
changed() { printf '%s\n' "$files" | grep -Eq "$1"; }
flag() { # <name> <true|false>
  echo "$1=$2" >> "$out"
  echo "main-plan: $1=$2"
}
want() { if changed "$2"; then flag "$1" true; else flag "$1" false; fi; }

flag base "$base"

# lull's open page (site/, ADR-0032) on GitHub Pages
want pages '^(site/|web/assets/app-icon\.svg$|web/fonts/nunito-latin\.woff2$)'
# The share Worker (ADR-0034): the files it serves and checks
want share '^(share/|site/|web/assets/app-icon\.svg$|web/assets/fallback-art-512\.png$|web/fonts/nunito-latin\.woff2$|packaging/android/assetlinks\.json$|app/gen/android/app/src/main/AndroidManifest\.xml$)'
# YouTube Music's live answer, after a change to the reader (YouTube being down must not hold anything back)
want youtube_contract '^crates/discovery/src/search/youtube_music\.rs$'
# The Linux app on a computer with no graphics card (graphics.yml): what it builds (app/, the crates it links and the
# files they compile in, the web pages it bundles and the fixture hub serves), the tests it runs and the stand-in
# servers they start, and its own workflow. main.yml also runs it whenever `base` is empty, as check runs every job then; the nightly always does.
want graphics '^(app/|crates/|web/|tests/desktop/|tests/ui/fixture-hub/|tests/slskd-contract/|share/hosts\.json$|deploy/\.env\.example$|scripts/release-areas\.tsv$|Cargo\.toml$|rust-toolchain\.toml$|\.github/workflows/graphics\.yml$)'
# What every release build compiles: main builds the bundles then, to warm their caches (ADR-0097)
want warm '^(Cargo\.lock|app/Cargo\.lock|app/Cargo\.toml|rust-toolchain\.toml)$'
# A feat or fix reached main: the next release's draft is redrawn (ADR-0098)
if git log --format=%s "$before..$sha" | grep -Eq '^(feat|fix)\('; then flag notes true; else flag notes false; fi
