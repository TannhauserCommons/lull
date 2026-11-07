#!/bin/sh
# lull-bin as pacman installed it, run as a listener runs it (aur.yml, in Arch's container, under Xvfb and a D-Bus
# session): lull starts and its window draws on Arch's WebKitGTK; then, as its own hub ("This computer is my hub"), it
# runs Arch's yt-dlp and deno and the AUR's slskd and signs in to Soulseek (tests/desktop/hub-signin.mjs).
set -eu
id=io.github.tannhausercommons.lull
data=$HOME/.local/share/$id
config=$HOME/.config/$id
logs=$data/logs

say() { printf '%s\n' "$*"; }
fail() { say "FAIL: $*"; show_logs; exit 1; }
show_logs() {
  for f in "$logs/webview.log" "$logs/hub.log"; do [ -f "$f" ] && { say "--- $f"; tail -n 60 "$f"; }; done
  find "$data/slskd" -name '*.log' 2>/dev/null | while read -r f; do say "--- $f"; tail -n 40 "$f"; done
}
stop() { kill "$1" 2>/dev/null || true; for _ in $(seq 1 20); do kill -0 "$1" 2>/dev/null || return 0; sleep 0.5; done; kill -9 "$1" 2>/dev/null || true; }

# 1. A first start: the welcome page, drawn.
lull & app=$!
for _ in $(seq 1 60); do grep -q started "$logs/webview.log" 2>/dev/null && break; sleep 1; done
sleep 8
kill -0 $app || fail "lull exited at its first start"
sd=$(magick x:root -colorspace gray -format '%[fx:standard_deviation]' info:)
say "first start: the screen's grey standard deviation is $sd"
awk -v sd="$sd" 'BEGIN { exit !(sd > 0.05) }' || fail "the window is blank"
stop $app

# 2. This computer is the hub, with Arch's yt-dlp and deno and the AUR's slskd (ADR-0064): it signs in to Soulseek,
# the slskd contract's stand-in server, with no real account.
node tests/desktop/hub-signin.mjs /usr/bin/lull "$id" system || { show_logs; exit 1; }
say "ok: lull draws, and as its own hub signs in to Soulseek with this computer's helpers"
