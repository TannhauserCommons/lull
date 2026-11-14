#!/bin/sh
# The hub's image, built as the queue builds it and saved as a tar: `sh scripts/image-save.sh [tar]` (`mise run image:save`).
# The tar defaults to $RUNNER_TEMP/lull-image.tar, else $TMPDIR; `docker load` brings it back as lull:candidate
# (tests/ui/hub.sh up does).
#
# In CI the layers come from main's build cache (type=gha, the cache the queue's image job keeps on main) and none are
# written back: only main saves caches. A buildx builder (docker/setup-buildx-action) and the Actions runtime
# variables in the environment (crazy-max/ghaction-github-runtime) let it read the cache. Elsewhere the build is cold.
#
# Last.fm's app key and secret are compiled in only when LULL_LASTFM_API_KEY and LULL_LASTFM_API_SECRET are set, as
# the queue's build does with its repository secrets. `check` has no secrets, so its image has no built-in key.
set -eu
cd "$(dirname "$0")/.."
tar=${1:-${RUNNER_TEMP:-${TMPDIR:-/tmp}}/lull-image.tar}
commit=$(git rev-parse HEAD)

# What the image says it is (lull_core::build): the newest release it holds everything of, and the changes since. A
# release's tag is its version commit on top of the green commit it was cut from, and that version commit reaches main
# later, rebased, so the tag itself is seldom in main's history: the commit it was cut from is. Release version commits
# aren't counted as changes. The same as queue.sh's build_version; one copy goes when the queue does.
release= changes=
git fetch -q --tags origin || true
for tag in $(git tag -l 'v[0-9]*.[0-9]*.[0-9]*' --sort=-v:refname); do
  git merge-base --is-ancestor "$tag^" "$commit" 2>/dev/null || continue
  if git merge-base --is-ancestor "$tag" "$commit"; then from=$tag; else from="$tag^"; fi
  release=${tag#v}
  changes=$(git rev-list --count --invert-grep --grep='^build(release): lull ' "$from..$commit")
  break
done

set -- --build-arg "LULL_BUILD_RELEASE=$release" --build-arg "LULL_BUILD_CHANGES=$changes" --build-arg "LULL_BUILD_COMMIT=$commit"
if [ -n "${LULL_LASTFM_API_KEY:-}" ] && [ -n "${LULL_LASTFM_API_SECRET:-}" ]; then
  set -- "$@" --secret id=lastfm_key,env=LULL_LASTFM_API_KEY --secret id=lastfm_secret,env=LULL_LASTFM_API_SECRET
fi
if [ -n "${ACTIONS_RUNTIME_TOKEN:-}" ]; then
  set -- "$@" --cache-from type=gha
elif [ -n "${CI:-}" ]; then
  echo "image-save: no ACTIONS_RUNTIME_TOKEN, so main's build cache can't be read: expose the runner's variables first" >&2
  exit 1
fi

mkdir -p "$(dirname "$tar")"
docker buildx build "$@" --tag lull:candidate --output "type=docker,dest=$tar" .
