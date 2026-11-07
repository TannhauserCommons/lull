#!/bin/sh
# lull with a listener's existing Jellyfin (ADR-0090, ADR-0003), as server-package.yml runs it: a real Jellyfin set up
# through its own startup API with a music library on a folder, then the hub pointed at that Jellyfin by `lull setup`'s
# questions, answered as a listener would. Checks the hub's library is Jellyfin's, connected, with every song, and
# that the hub saves into the folder Jellyfin reads.
#
#   sh tests/hub-contract/jellyfin.sh <lull binary>
# Needs docker, node and ffmpeg (the fixture library).
set -eu
lull=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
here=$(cd "$(dirname "$0")" && pwd)
w=$(mktemp -d)
music=$w/music
jf=http://127.0.0.1:8096
# Jellyfin 12.4, pinned by digest.
image=jellyfin/jellyfin:12.4@sha256:fea863d0cd9ec445985d9c75fc6ed64e38a1d2766cf0c250a33d553f4fc1b771
fail() { echo "FAIL: $*" >&2; tail -30 "$w/hub.log" >&2 || true; exit 1; }
trap 'docker rm -f lull-jellyfin >/dev/null 2>&1 || true; pkill -f "$lull serve" || true' EXIT

echo "== the fixture library, and a Jellyfin reading it"
node "$here/../ui/fixtures/make-library.mjs" "$music" >/dev/null
songs=$(find "$music" -type f \( -name '*.flac' -o -name '*.mp3' -o -name '*.ogg' -o -name '*.opus' -o -name '*.m4a' \) | wc -l)
# The same path inside and out, so the folder lull saves into is the one Jellyfin's library names.
docker run -d --name lull-jellyfin --network host -v "$music:$music:ro" "$image" >/dev/null
auth='MediaBrowser Client="lull-ci", Device="ci", DeviceId="lull-ci", Version="1"'
jfcall() { # <method> <path> [json]
  curl -fsS -X "$1" "$jf$2" -H "Authorization: $auth${token:+, Token=\"$token\"}" -H 'Content-Type: application/json' ${3:+-d "$3"}
}
token=""
# Jellyfin answers 503 while it starts; its startup wizard opens once it is up.
for _ in $(seq 1 120); do curl -fsS "$jf/Startup/Configuration" -H "Authorization: $auth" >/dev/null 2>&1 && break; sleep 2; done
jfcall POST /Startup/Configuration '{"UICulture":"en-US","MetadataCountryCode":"US","PreferredMetadataLanguage":"en"}' >/dev/null
jfcall GET /Startup/User >/dev/null
jfcall POST /Startup/User '{"Name":"admin","Password":"lull-ci-admin"}' >/dev/null
jfcall POST /Startup/Complete >/dev/null
token=$(jfcall POST /Users/AuthenticateByName '{"Username":"admin","Pw":"lull-ci-admin"}' | node -p 'JSON.parse(require("fs").readFileSync(0)).AccessToken')
jfcall POST "/Library/VirtualFolders?name=Music&collectionType=music&refreshLibrary=true" \
  "{\"LibraryOptions\":{\"PathInfos\":[{\"Path\":\"$music\"}]}}" >/dev/null
jfcall POST "/Auth/Keys?app=lull" >/dev/null
key=$(jfcall GET /Auth/Keys | node -p 'JSON.parse(require("fs").readFileSync(0)).Items.find((k) => k.AppName === "lull").AccessToken')
for _ in $(seq 1 90); do
  n=$(jfcall GET "/Items?IncludeItemTypes=Audio&Recursive=true&Limit=0" | node -p 'JSON.parse(require("fs").readFileSync(0)).TotalRecordCount')
  [ "$n" -ge "$songs" ] && break
  sleep 2
done
[ "$n" -ge "$songs" ] || fail "Jellyfin found $n of $songs songs"
echo "Jellyfin knows the $songs songs"

echo "== a hub, set up with lull setup's questions"
export LULL_DATA_DIR="$w/data" LULL_LISTEN=127.0.0.1:8484 LULL_SLSKD_URL=http://127.0.0.1:9 LULL_SLSKD_API_KEY=k
"$lull" serve >"$w/hub.log" 2>&1 &
for _ in $(seq 1 60); do [ -S "$w/data/admin.sock" ] && break; sleep 0.5; done
# 2: an existing Jellyfin; its address; the key; its first admin (none); the folder it reads; no outside address.
printf '2\n%s\n%s\nnone\n%s\nnone\n' "$jf" "$key" "$music" | "$lull" setup | tee "$w/setup.log"
code=$(sed -n 's/.*enter the code \([A-Z0-9]*\).*/\1/p' "$w/setup.log")
[ -n "$code" ] || fail "lull setup printed no pair code"

# shellcheck disable=SC2016 # JavaScript template strings, not shell
hub=$(node -e '
  const [code] = process.argv.slice(1), base = "http://127.0.0.1:8484";
  const go = async () => {
    const pair = await (await fetch(`${base}/api/pair`, { method: "POST", headers: { "content-type": "application/json" },
      body: JSON.stringify({ code, name: "jellyfin check", kind: "app" }) })).json();
    const get = async (p) => (await fetch(base + p, { headers: { authorization: `Bearer ${pair.token}` } })).json();
    for (let i = 0; i < 90; i++) {
      const source = await get("/api/library/source");
      const count = (await get("/api/library/songs")).total ?? 0;
      if (source.mode === "jellyfin" && source.jellyfin?.connected && count > 0) {
        console.log(JSON.stringify({ mode: source.mode, musicDir: source.musicDir, songs: count }));
        return;
      }
      await new Promise((r) => setTimeout(r, 2000));
    }
    console.log(JSON.stringify({ stuck: await get("/api/library/source") }));
  };
  go();' "$code")
echo "the hub: $hub"
echo "$hub" | grep -q '"mode":"jellyfin"' || fail "the hub's library isn't Jellyfin's: $hub"
echo "$hub" | grep -q "\"musicDir\":\"$music\"" || fail "the hub saves somewhere Jellyfin doesn't read: $hub"
echo "$hub" | grep -q "\"songs\":${songs}[,}]" || fail "the hub lists another number of songs than Jellyfin's $songs: $hub"
echo "== lull uses the listener's Jellyfin: its songs, and saves into the folder it reads"
