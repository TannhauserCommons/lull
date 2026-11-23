#!/bin/sh
# The hub's image, built as CI builds it and saved as a tar: `sh scripts/image-save.sh [tar]` (`mise run image:save`).
# The tar defaults to $RUNNER_TEMP/lull-image.tar, else $TMPDIR; `docker load` brings it back as lull:candidate
# (tests/ui/hub.sh up does).
#
# In CI the layers come from main's build cache (type=gha) and are written back only when LULL_IMAGE_CACHE_SAVE is set,
# which check.yml does for a run on main: only main saves caches. A buildx builder (docker/setup-buildx-action) and the
# Actions runtime variables in the environment (crazy-max/ghaction-github-runtime) let it reach the cache. Elsewhere the
# build is cold.
#
# Last.fm's app key and secret are compiled in only when LULL_LASTFM_API_KEY and LULL_LASTFM_API_SECRET are set: main.yml
# passes its repository secrets down, a pull request's check has none, so its image has no built-in key.
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
# What the registry shows and links the package to the repository by (the queue's image job set the same labels).
set -- "$@" --label "org.opencontainers.image.revision=$commit" --label org.opencontainers.image.title=lull \
  --label org.opencontainers.image.licenses=AGPL-3.0-only
[ -z "${GITHUB_REPOSITORY:-}" ] || set -- "$@" --label "org.opencontainers.image.source=${GITHUB_SERVER_URL:-https://github.com}/$GITHUB_REPOSITORY"
if [ -n "${LULL_LASTFM_API_KEY:-}" ] && [ -n "${LULL_LASTFM_API_SECRET:-}" ]; then
  set -- "$@" --secret id=lastfm_key,env=LULL_LASTFM_API_KEY --secret id=lastfm_secret,env=LULL_LASTFM_API_SECRET
fi
if [ -n "${ACTIONS_RUNTIME_TOKEN:-}" ]; then
  set -- "$@" --cache-from type=gha
  [ -z "${LULL_IMAGE_CACHE_SAVE:-}" ] || set -- "$@" --cache-to type=gha,mode=max
elif [ "${CI:-}" = true ]; then
  echo "image-save: no ACTIONS_RUNTIME_TOKEN, so main's build cache can't be read: expose the runner's variables first" >&2
  exit 1
fi

mkdir -p "$(dirname "$tar")"
docker buildx build "$@" --tag lull:candidate --output "type=docker,dest=$tar" .
