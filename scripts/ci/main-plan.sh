#!/bin/sh
# What a push to main brought, for main.yml's after-landing jobs: `sh scripts/ci/main-plan.sh` (`mise run main:plan`).
# Reads BEFORE (main's tip before the push) and SHA (its tip now) and writes one `name=true|false` line each to
# $GITHUB_OUTPUT, or to stdout when that isn't set.
set -eu

die() { echo "main-plan: $*" >&2; exit 1; }

before=${BEFORE:-}
sha=${SHA:-}
[ -n "$sha" ] || die "SHA is not set"
# A push that made main (no tip before) or lost its old tip can't say what it brought; the run fails instead of
# guessing, and shows it.
[ -n "$before" ] && ! printf '%s' "$before" | grep -Eq '^0+$' || die "the push has no tip before it"
git cat-file -e "$before^{commit}" 2>/dev/null || die "$before isn't in the clone: check out with fetch-depth 0"

files=$(git diff --name-only "$before" "$sha")
out=${GITHUB_OUTPUT:-/dev/stdout}
changed() { printf '%s\n' "$files" | grep -Eq "$1"; }
flag() { # <name> <true|false>
  echo "$1=$2" >> "$out"
  echo "main-plan: $1=$2"
}
want() { if changed "$2"; then flag "$1" true; else flag "$1" false; fi; }

# lull's open page (site/, ADR-0032) on GitHub Pages
want pages '^(site/|web/assets/app-icon\.svg$|web/fonts/nunito-latin\.woff2$)'
# The share Worker (ADR-0034): the files it serves and checks
want share '^(share/|site/|web/assets/app-icon\.svg$|web/assets/fallback-art-512\.png$|web/fonts/nunito-latin\.woff2$|packaging/android/assetlinks\.json$|app/gen/android/app/src/main/AndroidManifest\.xml$)'
# YouTube Music's live answer, after a change to the reader (YouTube being down must not hold anything back)
want youtube_contract '^crates/discovery/src/search/youtube_music\.rs$'
# What every release build compiles: main builds the bundles then, to warm their caches (ADR-0097)
want warm '^(Cargo\.lock|app/Cargo\.lock|app/Cargo\.toml|rust-toolchain\.toml)$'
# A feat or fix reached main: the next release's draft is redrawn (ADR-0098)
if git log --format=%s "$before..$sha" | grep -Eq '^(feat|fix)\('; then flag notes true; else flag notes false; fi
