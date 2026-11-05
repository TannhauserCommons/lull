# Re-picks each song's Apple reference strictly (artist and title equal, Unicode-aware), records the track's
# own length and album, and replaces the preview where the pick changed. The check in lull would use the
# catalog track the listener chose, by its id; a search here stands in for that.
import json, os, re, sys, time, unicodedata, urllib.request, urllib.parse
D = sys.argv[1]
def norm(s):
    s = unicodedata.normalize("NFKC", s.lower()); s = unicodedata.normalize("NFKD", s); s = "".join(c for c in s if not unicodedata.combining(c))
    return re.sub(r"[\W_]+", " ", s).strip()
def artists(s): return {norm(p) for p in re.split(r",|&| feat\.? | ft\.? | and | x ", s.lower())} | {norm(s)}
plan = json.load(open(os.path.join(D, "plan.json"), encoding="utf-8"))
for key, p in plan.items():
    if "apple_len" in p: continue
    r = json.load(urllib.request.urlopen("https://itunes.apple.com/search?" + urllib.parse.urlencode({"term": f"{p['artist']} {p['title']}", "entity": "song", "limit": 25, "country": "US"}), timeout=20))
    want = artists(p["artist"]) - {""}
    hits = [x for x in r["results"] if norm(x["trackName"]) == norm(p["title"]) and norm(p["title"]) and (want & artists(x["artistName"]))]
    slug = re.sub(r"[^a-z0-9]+", "-", norm(key).encode("ascii", "ignore").decode())[:40] or str(abs(hash(key)))
    h = next((x for x in hits if x.get("previewUrl")), None)
    if h:
        f = slug + ".apple.m4a" if p["refs"].get("apple") is None else p["refs"]["apple"]
        old = (p.get("apple_pick") or "")
        pick = f"{h['artistName']} - {h['trackName']} ({h['collectionName']})"
        if pick != old:
            open(os.path.join(D, f), "wb").write(urllib.request.urlopen(h["previewUrl"], timeout=30).read())
        p["refs"]["apple"] = f; p["apple_pick"] = pick; p["apple_len"] = round(h["trackTimeMillis"] / 1000); p["apple_preview"] = True
    else:
        p["refs"].pop("apple", None); p["apple_preview"] = False; p["apple_pick"] = None; p["apple_len"] = None
    print(key, "=>", p["apple_pick"], p["apple_len"], flush=True)
    json.dump(plan, open(os.path.join(D, "plan.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    time.sleep(4)
