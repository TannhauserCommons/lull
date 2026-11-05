#!/bin/sh
# lull's slskd for Windows (ADR-0062): slskd's own release tag, lull's one-line patch, published the way slskd's
# release is (its bin/publish flags), so the build is the same bytes each time it runs.
#
#   sh app/slskd/build.sh <out dir>
#
# Writes <out dir>/slskd-<build>-win-x64.zip and -win-arm64.zip, each holding slskd.exe, slskd's LICENSE,
# NOTICE.txt (what lull changed and where the source is) and the patch, and prints their sha256. Needs git, the
# .NET SDK in global.json and python3. CI runs it (.github/workflows/slskd-windows.yml); app/build.rs pins the
# zips it published.
set -eu

UPSTREAM_TAG=0.26.2
UPSTREAM_COMMIT=e472ecd706050332c8615dc3b908c92afaed592c
# lull's builds of one slskd version, counted from 1. A new patch or SDK is a new build.
LULL_BUILD=1
BUILD="$UPSTREAM_TAG-lull.$LULL_BUILD"
RUNTIMES="win-x64 win-arm64"

out=${1:?usage: build.sh <out dir>}
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$out"
out=$(cd "$out" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

git -c advice.detachedHead=false clone -q --depth 1 --branch "$UPSTREAM_TAG" https://github.com/slskd/slskd.git "$work/src"
got=$(git -C "$work/src" rev-parse HEAD)
[ "$got" = "$UPSTREAM_COMMIT" ] || { echo "slskd's tag $UPSTREAM_TAG is $got now, not $UPSTREAM_COMMIT" >&2; exit 1; }
git -C "$work/src" apply "$here/share-for-reading.patch"
cp "$here/global.json" "$work/src/global.json"
# What slskd reports as its version: its release's own form (tag.0+commit), and lull's build.
version="$UPSTREAM_TAG.0+$(git -C "$work/src" rev-parse --short=7 HEAD).lull.$LULL_BUILD"

for rid in $RUNTIMES; do
  # bin/publish's flags, plus ContinuousIntegrationBuild: paths in the binaries are /_/, not this folder.
  (cd "$work/src/src/slskd" && dotnet publish \
    --configuration Release \
    -p:PublishSingleFile=true \
    -p:ReadyToRun=true \
    -p:IncludeNativeLibrariesForSelfExtract=true \
    -p:CopyOutputSymbolsToPublishDirectory=false \
    -p:ContinuousIntegrationBuild=true \
    -p:Version="$version" \
    --self-contained \
    --runtime "$rid" \
    --output "$work/publish/$rid" \
    -nologo -v minimal)
  cat > "$work/publish/$rid/NOTICE.txt" <<EOF
This is slskd $UPSTREAM_TAG, modified by lull (https://github.com/TannhauserCommons/lull) for its Windows app.

What changed: slskd opens a file it is downloading so that other programs may read it while it arrives
(FileShare.Read instead of FileShare.None), so lull can play a song from Soulseek while slskd is still downloading
it. The change is share-for-reading.patch, next to this file. Nothing else differs.

Source: slskd's tag $UPSTREAM_TAG (https://github.com/slskd/slskd/tree/$UPSTREAM_COMMIT) with that patch applied.
lull's build recipe: https://github.com/TannhauserCommons/lull/tree/main/app/slskd

slskd is free software under the GNU Affero General Public License v3.0, in LICENSE next to this file.
Copyright (c) JP Dillingham and the slskd contributors.
EOF
  cp "$here/share-for-reading.patch" "$work/publish/$rid/"
  python3 - "$work/publish/$rid" "$out/slskd-$BUILD-$rid.zip" <<'EOF'
# The same bytes for the same files: fixed order, dates and permissions.
import sys, zipfile
src, dest = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(dest, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
    for name in ["slskd.exe", "LICENSE", "NOTICE.txt", "share-for-reading.patch"]:
        info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
        info.external_attr = 0o644 << 16
        info.compress_type = zipfile.ZIP_DEFLATED
        with open(f"{src}/{name}", "rb") as f:
            z.writestr(info, f.read())
EOF
done

(cd "$out" && sha256sum slskd-"$BUILD"-*.zip)
