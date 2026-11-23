#!/bin/sh
# Publishes the hub image main's check built and tested, so what installs pull is what was tested: `sh
# scripts/ci/image-publish.sh` (`mise run image:publish`). Run by main.yml once `check` is green, after it downloads the
# `image` artifact (what scripts/image-save.sh wrote) into $RUNNER_TEMP. The commit's tag first (`:sha-<short>`), then
# `:main`, which is what an install that follows main pulls.
# Reads GH_TOKEN (the job's token, packages: write), GITHUB_ACTOR, GITHUB_REPOSITORY_OWNER and GITHUB_SHA.
set -eu

die() { echo "image-publish: $*" >&2; exit 1; }

tar=${RUNNER_TEMP:-${TMPDIR:-/tmp}}/lull-image.tar
[ -f "$tar" ] || die "no image at $tar: the artifact wasn't downloaded"
[ -n "${GH_TOKEN:-}" ] || die "GH_TOKEN is not set"
sha=${GITHUB_SHA:?GITHUB_SHA is not set}
image="ghcr.io/$(printf '%s' "${GITHUB_REPOSITORY_OWNER:?}" | tr '[:upper:]' '[:lower:]')/lull"

printf '%s' "$GH_TOKEN" | docker login ghcr.io --username "${GITHUB_ACTOR:?}" --password-stdin >/dev/null
docker load -i "$tar"
# The image says which commit it was built from (scripts/image-save.sh's label): it has to be this one.
built=$(docker image inspect --format '{{index .Config.Labels "org.opencontainers.image.revision"}}' lull:candidate 2>/dev/null || true)
[ "$built" = "$sha" ] || die "the image was built from $built, not $sha"

short=$(printf '%s' "$sha" | cut -c1-7)
for tag in "sha-$short" main; do
  docker tag lull:candidate "$image:$tag"
  docker push -q "$image:$tag"
done
echo "Pushed $(docker inspect --format '{{index .RepoDigests 0}}' "$image:main") as :main and :sha-$short" >> "${GITHUB_STEP_SUMMARY:-/dev/stdout}"
