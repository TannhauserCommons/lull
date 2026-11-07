#!/bin/sh
# A hub updating itself, end to end (ADR-0090), as server-package.yml runs it: two debug builds of lull compiled with a
# throwaway kit key and a stand-in for GitHub's newest release (LULL_DEV_KIT_KEY, LULL_DEV_RELEASES; debug builds
# only), installed as the package. Checks that:
#   1. `lull update` downloads the next version's kit, and after the restart that version answers;
#   2. a kit that doesn't start is marked bad, the database copy goes back, the version before answers again, and
#      `lull update` says so;
#   3. a kit file changed on disk is refused at the next start, and the installed build answers.
#
#   sh tests/hub-contract/self-update.sh <lull v1> <v1> <lull v2> <v2> <v3> <private key file>
# v1, v2 and v3 are releases, oldest first; v3 is a kit whose lull exits at once.
set -eu
lull1=$1 v1=$2 lull2=$3 v2=$4 v3=$5 key=$6
here=$(cd "$(dirname "$0")" && pwd)
cli=$(node -p "require('$here/../../app/package.json').devDependencies['@tauri-apps/cli']")
arch=$(uname -m | sed 's/x86_64/x64/; s/aarch64/arm64/')
w=$(mktemp -d)
mkdir -p "$w/www" "$w/data"
trap 'pkill -f "$lull1 serve" || true; pkill -f "node $w/static.mjs" || true; kill $(jobs -p) 2>/dev/null || true' EXIT
fail() { echo "FAIL: $*" >&2; tail -40 "$w/hub.log" >&2; exit 1; }

# The stand-in release: its files, and latest.json as GitHub's API answers it.
cat >"$w/static.mjs" <<EOF
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
createServer(async (req, res) => {
  try { res.end(await readFile('$w/www' + req.url)); } catch { res.statusCode = 404; res.end(); }
}).listen(9999, '127.0.0.1');
EOF
node "$w/static.mjs" &

release() { # <version> <its lull>
  base="lull-hub-$1-linux-$arch"
  gzip -c "$2" >"$w/www/$base-lull.gz"
  printf '{"version":"%s","format":1,"arch":"%s","files":{"lull":"%s"}}' "$1" "$arch" "$(sha256sum "$2" | cut -d' ' -f1)" \
    >"$w/www/$base.json"
  TAURI_SIGNING_PRIVATE_KEY="$(cat "$key")" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" \
    npx --yes "@tauri-apps/cli@$cli" signer sign "$w/www/$base.json" >/dev/null
  # shellcheck disable=SC2016 # JavaScript template strings, not shell
  node -e '
    const [v, base, dir] = process.argv.slice(1), url = "http://127.0.0.1:9999/";
    const names = [`${base}.json`, `${base}.json.sig`, `${base}-lull.gz`];
    require("fs").writeFileSync(`${dir}/latest.json`, JSON.stringify({ tag_name: "v" + v, html_url: url,
      assets: names.map((name) => ({ name, browser_download_url: url + name })) }));' "$1" "$base" "$w/www"
}
health() { node -e 'fetch("http://127.0.0.1:8484/health").then((r) => r.json()).then((h) => console.log(h.version)).catch(() => console.log(""))'; }
until_health() { # <version>
  for _ in $(seq 1 150); do [ "$(health)" = "$1" ] && return 0; sleep 1; done
  fail "the hub didn't answer as $1 (it says '$(health)')"
}

export LULL_DATA_DIR="$w/data" LULL_LISTEN=127.0.0.1:8484 LULL_SLSKD_URL=http://127.0.0.1:9 LULL_SLSKD_API_KEY=k \
  LULL_FLAGS=hub-self-update LULL_LOG=warn
# systemd's Restart= for the service, as a loop: lull exits to start again on an update. Stopped by stop().
start() { (while [ ! -f "$w/stop" ]; do "$lull1" serve >>"$w/hub.log" 2>&1 || true; sleep 1; done) & }
stop() {
  touch "$w/stop"
  pkill -f "$lull1 serve" || true
  for _ in $(seq 1 60); do pgrep -f "$w/data/hub/" >/dev/null || pgrep -f "$lull1 serve" >/dev/null || return 0; sleep 1; done
  fail "the hub didn't stop"
}
start
until_health "$v1"

echo "== 1. update from $v1 to $v2"
release "$v2" "$lull2"
"$lull1" update || fail "lull update to $v2"
until_health "$v2"
[ -f "$w/data/hub/$v2/good" ] || fail "$v2 wasn't marked good"
[ -f "$w/data/backups/before-$v2.db" ] || fail "no copy of the database from before $v2"

echo "== 2. $v3 doesn't start"
printf '#!/bin/sh\nexit 3\n' >"$w/broken" && chmod 755 "$w/broken"
release "$v3" "$w/broken"
if "$lull1" update; then fail "lull update said $v3 runs"; fi
until_health "$v2"
[ -f "$w/data/hub/$v3/bad" ] || fail "$v3 wasn't marked bad"
grep -q "\"version\":\"$v3\"" "$w/data/hub/failed.json" || fail "failed.json doesn't name $v3"
[ -f "$w/data/backups/before-$v3.db" ] || fail "no copy of the database from before $v3"

echo "== 3. a kit file changed on disk"
stop
printf 'x' >>"$w/data/hub/$v2/lull"
rm "$w/stop" && start
until_health "$v1"
grep -q "a hub kit isn't used" "$w/hub.log" || fail "the changed kit wasn't refused by name"

echo "== a hub updated itself, went back from a kit that didn't start, and refused a changed one"
