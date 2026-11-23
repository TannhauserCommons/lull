#!/bin/sh
# A file pinned by its hash, from the folder CI keeps between runs when it is there, else from its URL:
#
#   sh scripts/ci/pinned-download.sh <url> <sha256|sha512> <hash> <dest> [curl options]
#
# $DOWNLOADS_CACHE, when set, holds the files by hash. A kept file is used only when it hashes to the pin; a download is
# kept only when it does too. The caller checks <dest> against the pin as it always did, so a wrong download still
# fails there, with its own message. With no $DOWNLOADS_CACHE this is `curl`.
set -eu
url=$1 algo=$2 hash=$3 dest=$4
shift 4
sum() { "${algo}sum" "$1" | cut -d' ' -f1; }
kept=${DOWNLOADS_CACHE:+$DOWNLOADS_CACHE/$hash}
if [ -n "$kept" ] && [ -f "$kept" ] && [ "$(sum "$kept")" = "$hash" ]; then
  cp "$kept" "$dest"
  exit 0
fi
curl -fsSL -o "$dest" "$@" "$url"
if [ -n "$kept" ] && [ "$(sum "$dest")" = "$hash" ]; then
  mkdir -p "$DOWNLOADS_CACHE"
  cp "$dest" "$kept"
fi
