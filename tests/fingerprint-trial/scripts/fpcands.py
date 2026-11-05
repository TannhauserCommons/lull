# Builds the fingerprint test set: references (Apple + Deezer previews) and labelled YouTube candidates.
# Polite: one request at a time, a few seconds apart; everything cached on disk.
import json, os, re, subprocess, sys, time, unicodedata, urllib.request, urllib.parse
D = sys.argv[1]; os.makedirs(D, exist_ok=True)
Y = os.environ.get("YTDLP", "yt-dlp")
SONGS = [l.split(" | ") for l in open(sys.argv[2], encoding="utf-8").read().splitlines() if l.strip()]
def norm(s):
    s = unicodedata.normalize("NFKD", s.lower()); s = "".join(c for c in s if not unicodedata.combining(c))
    return re.sub(r"[^a-z0-9]+", " ", s).strip()
def get(url):
    return json.load(urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "lull-fingerprint-trial/0.1"}), timeout=20))
def save(url, path):
    if not os.path.exists(path):
        open(path, "wb").write(urllib.request.urlopen(url, timeout=30).read())
def flat(q, n=10):
    out = subprocess.run([Y, "-J", "--flat-playlist", "-I", f"1:{n}", q], capture_output=True, text=True, encoding="utf-8")
    time.sleep(3)
    try: return json.loads(out.stdout)["entries"]
    except Exception: return []
VERSIONS = {"live": ["live", "concert"], "cover": ["cover"], "sped-up": ["sped up", "speed up", "nightcore"], "slowed": ["slowed"],
            "karaoke": ["karaoke"], "instrumental": ["instrumental"], "remaster": ["remaster"], "radio-edit": ["radio edit"], "acoustic": ["acoustic"]}
EXPECT = {"right": "pass", "fan": "pass", "remaster": "pass", "radio-edit": "pass", "live": "refuse", "cover": "refuse", "sped-up": "refuse",
          "slowed": "refuse", "karaoke": "refuse", "instrumental": "refuse", "acoustic": "refuse", "namesake": "refuse"}
plan = json.load(open(os.path.join(D, "plan.json"), encoding="utf-8")) if os.path.exists(os.path.join(D, "plan.json")) else {}
for artist, title, length in SONGS:
    key = f"{artist} - {title}"
    if key in plan: continue
    p = {"artist": artist, "title": title, "length": int(length), "refs": {}, "cands": []}
    slug = norm(key).replace(" ", "-")[:40]
    # Apple
    r = get("https://itunes.apple.com/search?" + urllib.parse.urlencode({"term": f"{artist} {title}", "entity": "song", "limit": 15, "country": "US"}))
    hit = [x for x in r["results"] if norm(x["artistName"]).startswith(norm(artist)[:6]) and norm(x["trackName"]) == norm(title)]
    p["apple_found"] = bool(hit); p["apple_preview"] = bool(hit and hit[0].get("previewUrl"))
    if p["apple_preview"]:
        save(hit[0]["previewUrl"], os.path.join(D, slug + ".apple.m4a")); p["refs"]["apple"] = slug + ".apple.m4a"
    for x in r["results"]:
        if norm(x["trackName"]) == norm(title) and norm(artist)[:6] not in norm(x["artistName"]) and x.get("previewUrl"):
            n = slug + ".namesake-apple.m4a"; save(x["previewUrl"], os.path.join(D, n))
            p["cands"].append({"file": n, "kind": "namesake", "src": "apple", "label": f"{x['artistName']} - {x['trackName']}"}); break
    time.sleep(4)
    # Deezer
    r = get("https://api.deezer.com/search?" + urllib.parse.urlencode({"q": f'artist:"{artist}" track:"{title}"'}))
    hit = [x for x in r.get("data", []) if norm(x["title"]) == norm(title) and x.get("preview")]
    p["deezer_preview"] = bool(hit)
    if hit:
        save(hit[0]["preview"], os.path.join(D, slug + ".deezer.mp3")); p["refs"]["deezer"] = slug + ".deezer.mp3"
    time.sleep(1)
    # YouTube: official/Topic + fan uploads from a plain search, versions from keyword searches
    seen = set()
    def add(e, kind):
        if e.get("id") in seen or len(p["cands"]) > 12: return
        seen.add(e["id"]); p["cands"].append({"id": e["id"], "kind": kind, "src": "youtube", "label": f"{e.get('title')} [{e.get('channel') or e.get('uploader')}] {e.get('duration')}"})
    light = os.environ.get("LIGHT") == "1"
    for e in flat(f"ytsearch10:{artist} {title}"):
        t, ch = norm(e.get("title") or ""), norm(e.get("channel") or e.get("uploader") or "")
        if any(w in t for ws in VERSIONS.values() for w in ws) or norm(title) not in t: continue
        kind = "right" if ch.endswith("topic") or ch.startswith(norm(artist)) or "vevo" in ch else "fan"
        if sum(c["kind"] == kind for c in p["cands"]) < (1 if light else 2): add(e, kind)
    items = list(VERSIONS.items())
    if light: items = [items[len(plan) % len(items)]]
    for kind, words in items:
        for e in flat(f"ytsearch5:{artist} {title} {words[0]}", 5):
            t = norm(e.get("title") or "")
            if norm(title) in t and any(w in t for w in words): add(e, kind); break
    for e in ([] if light else flat(f"ytsearch10:{title}")):
        t = norm(e.get("title") or ""); ch = norm(e.get("channel") or "")
        if norm(title) in t and norm(artist) not in t and norm(artist) not in ch and not any(w in t for ws in VERSIONS.values() for w in ws):
            add(e, "namesake"); break
    for c in p["cands"]: c["expect"] = EXPECT[c["kind"]]
    plan[key] = p
    json.dump(plan, open(os.path.join(D, "plan.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    print(key, "apple" if p["apple_preview"] else "-", "deezer" if p["deezer_preview"] else "-", [c["kind"] for c in p["cands"]], flush=True)
