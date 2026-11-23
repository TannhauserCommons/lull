#!/bin/sh
# The Linux server package (ADR-0090): lull built as the image builds it (the web UI minified into the binary), with the
# slskd, yt-dlp and Deno that app/build.rs pins, as a .deb, an .rpm and a .tar.gz (packaging/linux/), into
# $PACKAGES (default <temp>/packages). x64, on the oldest glibc (Ubuntu 22.04), so the packages run on older
# distributions too. Unsigned: a release's signing is not part of the check. The `package` job of
# .github/workflows/check.yml runs it; the contracts then install what it made.
#
# While it runs, web/ is the minified build, as the Dockerfile's web stage makes it (the binary embeds it). The
# checkout's own web/ waits beside it as web.ci-source and is put back at the end, also when the build fails or the
# script is interrupted or terminated (SIGKILL can't be caught: the leftover web.ci-source then stops the next run
# instead of being overwritten). It is only ever removed after the minified build's folder, never before it is back.
# Needs the full history with its tags, for the version the hub says it is (lull_core::build).
set -eu
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh

packages=${PACKAGES:-$(tmp_root)/packages}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
commit=$(git rev-parse HEAD)

# What the hub says it is, as main's image says it (scripts/image-save.sh): the newest release this
# commit holds everything of, and the changes since. A release's tag is its version commit on top of the commit it was
# cut from, and that version commit reaches main later, so the tag itself is seldom in the history: the commit it was cut
# from is. Release version commits aren't counted as changes.
release= changes=
for tag in $(git tag -l 'v[0-9]*.[0-9]*.[0-9]*' --sort=-v:refname); do
  git merge-base --is-ancestor "$tag^" HEAD 2>/dev/null || continue
  if git merge-base --is-ancestor "$tag" HEAD; then from=$tag; else from="$tag^"; fi
  release=${tag#v}
  changes=$(git rev-list --count --invert-grep --grep='^build(release): lull ' "$from..HEAD")
  break
done
if [ -n "$release" ] && [ "$changes" != 0 ]; then
  version="$release+$changes.$(printf %s "$commit" | cut -c1-7)"
else
  version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
fi
echo "package version $version"

# The web UI as the binary embeds it: Webamp vendored, every file minified and pre-compressed, in place of web/.
[ ! -e web.ci-source ] || { echo "package-build: web.ci-source is the checkout's web/ from a run that was killed: put it back first (rm -rf web; mv web.ci-source web)" >&2; exit 1; }
sh scripts/vendor-web.sh
npm ci --prefix scripts/web-dist --ignore-scripts --no-audit --no-fund
node scripts/web-dist/build.mjs web "$work/web-dist"

# One trap for every way out. The source is renamed in its own folder (atomic), so web.ci-source exists exactly when
# web/ is not the source; the minified web/ is removed only to put it back, and the work folder only after that.
restore() {
  status=$?
  trap - EXIT INT TERM
  if [ -e web.ci-source ]; then
    rm -rf web
    mv web.ci-source web || { echo "package-build: couldn't put web/ back; it is web.ci-source" >&2; exit 1; }
  fi
  rm -rf "$work"
  exit "$status"
}
trap restore EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
mv web web.ci-source
mv "$work/web-dist" web

# As the image builds it, but installed as the package: Settings > App and the updates say so (a kit's lull run by an
# image's hub runs as the image's, launch.rs).
env LULL_BUILD_RELEASE="$release" LULL_BUILD_CHANGES="$changes" LULL_BUILD_INSTALL=package LULL_BUILD_COMMIT="${release:+$commit}" \
  cargo build --release --locked --bin lull
sh packaging/linux/build.sh "${CARGO_TARGET_DIR:-target}/release/lull" "$version" "$packages"
