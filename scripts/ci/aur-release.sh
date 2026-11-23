#!/bin/sh
# The newest release of $GITHUB_REPOSITORY, as 1.2.3 (the aur check tests that one, so it is looked up every run;
# $GH_TOKEN, when set, signs the lookup). Prints it, and in a workflow step also writes `version=` to $GITHUB_OUTPUT, for
# the cache that keeps that release's files by its tag. Needs curl and sed only: it runs before the Arch image's packages.
set -eu
: "${GITHUB_REPOSITORY:?set GITHUB_REPOSITORY (owner/name) to find the release}"
if [ -n "${GH_TOKEN:-}" ]; then
  body=$(curl -fsSL -H "Authorization: Bearer $GH_TOKEN" "https://api.github.com/repos/$GITHUB_REPOSITORY/releases/latest")
else
  body=$(curl -fsSL "https://api.github.com/repos/$GITHUB_REPOSITORY/releases/latest")
fi
version=$(printf '%s\n' "$body" | sed -n 's/.*"tag_name": *"v\{0,1\}\([^"]*\)".*/\1/p' | head -n 1)
[ -n "$version" ] || { echo "aur-release: no tag_name in the newest release of $GITHUB_REPOSITORY" >&2; exit 1; }
[ -z "${GITHUB_OUTPUT:-}" ] || echo "version=$version" >> "$GITHUB_OUTPUT"
echo "$version"
