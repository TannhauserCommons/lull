# Does Apple have the song with a 30 s preview, per storefront? (the fingerprint check's first reference)
import json, re, sys, time, unicodedata, urllib.request, urllib.parse
def norm(s):
    s = unicodedata.normalize("NFKD", s.lower()); s = "".join(c for c in s if not unicodedata.combining(c))
    return re.sub(r"[\W_]+", " ", re.sub(r"\s*[\(\[].*?[\)\]]", "", s)).strip()
songs = [l.split(" | ")[:2] for l in open(sys.argv[1], encoding="utf-8").read().splitlines() if l.strip()]
C = sys.argv[2].split(","); tot = {c: 0 for c in C}; out = {}
for a, t in songs:
    line = []
    for c in C:
        try:
            r = json.load(urllib.request.urlopen("https://itunes.apple.com/search?" + urllib.parse.urlencode({"term": f"{a} {t}", "entity": "song", "limit": 15, "country": c}), timeout=20))
            ok = any(norm(x["trackName"]) == norm(t) and x.get("previewUrl") for x in r["results"])
        except Exception as e: ok = None
        tot[c] += bool(ok); line.append(f"{c}={'Y' if ok else ('?' if ok is None else '-')}"); out[f"{a} - {t}|{c}"] = ok
        time.sleep(4)
    print(f"{a} - {t}: " + " ".join(line), flush=True)
json.dump(out, open(sys.argv[3], "w", encoding="utf-8"), ensure_ascii=False, indent=1)
print({c: f"{v}/{len(songs)}" for c, v in tot.items()})
