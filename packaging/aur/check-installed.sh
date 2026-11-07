#!/bin/sh
# lull-bin as pacman installed it, run as a listener runs it (aur.yml, in Arch's container, under Xvfb and a D-Bus
# session): lull starts and its window draws on Arch's WebKitGTK; then, as its own hub ("This computer is my hub"), it
# starts the slskd beside it in /usr/lib/lull and the hub reaches slskd's API. There is no Soulseek login here, so slskd
# answers logged out, which is what the hub must then say.
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

# 2. This computer is the hub: the hub, then its slskd, started by lull.
mkdir -p "$config"
printf '{"hub":{"mode":"local"}}\n' > "$config/settings.json"
lull & app=$!
health=""
for _ in $(seq 1 90); do
  health=$(curl -fsS http://127.0.0.1:8484/health 2>/dev/null || true)
  [ -n "$health" ] && break
  sleep 1
done
[ -n "$health" ] || fail "the hub on this computer never answered on 8484"
pgrep -f '^/usr/lib/lull/slskd' >/dev/null || fail "no slskd running from /usr/lib/lull"
# slskd takes a moment to answer its API; the hub checks it every few seconds.
detail=""
for _ in $(seq 1 60); do
  detail=$(curl -fsS http://127.0.0.1:8484/health | node -e "let s=''; process.stdin.on('data', d => s += d).on('end', () => {
    const x = JSON.parse(s).services.soulseek; console.log(x ? x.state + ': ' + x.detail : 'none'); })")
  case "$detail" in *"not connected to Soulseek"*|*Connected*|*Reconnecting*) break ;; esac
  sleep 2
done
say "soulseek: $detail"
case "$detail" in
  *"not connected to Soulseek"*|*Connected*|*Reconnecting*) ;;
  *) fail "the hub can't use its slskd: $detail" ;;
esac
show_logs
stop $app
say "ok: lull draws, and as its own hub runs its slskd, which answers"
