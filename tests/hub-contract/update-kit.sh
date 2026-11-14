# Shared by the hub-contract scripts that update a hub through kits (self-update.sh, secrets-update.sh). Sourced
# with $here (this folder), $w (a fresh work folder), $lull1 (the installed lull) and $key (the throwaway kit key's
# private half) set. Starts a stand-in for GitHub's newest release on 127.0.0.1:9999 and defines:
#   release <version> <its lull>   publishes that version's kit there and makes it the newest release
#   health, until_health <version> what /health answers as, and waiting for a version to answer
#   start, stop                    the installed lull as systemd runs it: started again whenever it exits
cli=$(node -p "require('$here/../../app/package.json').devDependencies['@tauri-apps/cli']")
arch=$(uname -m | sed 's/x86_64/x64/; s/aarch64/arm64/')
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
until_health() { # <version> [seconds, 150 by default]
  for _ in $(seq 1 "${2:-150}"); do [ "$(health)" = "$1" ] && return 0; sleep 1; done
  fail "the hub didn't answer as $1 (it says '$(health)')"
}
# The launcher marks a kit good once its own check of /health passes (launch.rs: every 2 s, each given 3 s), so up to
# a few seconds after until_health already saw the kit answer. Bad is marked before the release starts again.
until_marked() { # <version> <good|bad>
  for _ in $(seq 1 10); do [ -f "$w/data/hub/$1/$2" ] && return 0; sleep 1; done
  fail "$1 wasn't marked $2"
}

# systemd's Restart= for the service, as a loop: lull exits to start again on an update. Stopped by stop().
start() { (while [ ! -f "$w/stop" ]; do "$lull1" serve >>"$w/hub.log" 2>&1 || true; sleep 1; done) & }
stop() {
  touch "$w/stop"
  pkill -f "$lull1 serve" || true
  for _ in $(seq 1 60); do pgrep -f "$w/data/hub/" >/dev/null || pgrep -f "$lull1 serve" >/dev/null || return 0; sleep 1; done
  fail "the hub didn't stop"
}
