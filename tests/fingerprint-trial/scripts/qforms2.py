# The quoted-artist query across languages, scripts and storefronts: is the right song where playback looks?
import json, re, sys, time, unicodedata, urllib.request
SONGS = [[p.strip() for p in l.split(" | ")] for l in open(sys.argv[1], encoding="utf-8").read().splitlines() if l.strip()]
GLS = sys.argv[3].split(",")
def norm(s):
    s = unicodedata.normalize("NFKD", s.lower()); s = "".join(c for c in s if not unicodedata.combining(c))
    return re.sub(r"[\W_]+", " ", s).strip()
def bare(t): return norm(re.sub(r"\s*[\(\[].*?[\)\]]", "", t))
def search(q, gl):
    body = {"context": {"client": {"clientName": "WEB_REMIX", "clientVersion": "1.20241106.01.00", "hl": "en", "gl": gl}},
            "query": q, "params": "EgWKAQIIAWoKEAkQBRAKEAMQBA%3D%3D"}
    req = urllib.request.Request("https://music.youtube.com/youtubei/v1/search?prettyPrint=false", data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json", "User-Agent": "Mozilla/5.0", "Origin": "https://music.youtube.com"})
    b = json.load(urllib.request.urlopen(req, timeout=20)); rows = []; seen = set()
    def walk(o):
        if isinstance(o, dict):
            if "musicResponsiveListItemRenderer" in o:
                r = o["musicResponsiveListItemRenderer"]
                cols = ["".join(x.get("text", "") for x in c["musicResponsiveListItemFlexColumnRenderer"]["text"].get("runs", [])) for c in r.get("flexColumns", [])]
                vid = (r.get("playlistItemData") or {}).get("videoId")
                if vid and vid not in seen: seen.add(vid); rows.append((vid, cols[0], cols[1].split(" • ")[0] if len(cols) > 1 else ""))
            for v in o.values(): walk(v)
        elif isinstance(o, list):
            for v in o: walk(v)
    walk(b); return rows[:5]
out = {}; tot = {}
for artists, titles, album in SONGS:
    A = artists.split("/"); T = titles.split("/"); a, t = A[0], T[0]
    for gl in GLS:
        line = []
        for name, q in [("now", f"{a} {t}"), ("quoted", f'"{a}" {t}')]:
            try: rows = search(q, gl)
            except Exception as e: rows = []; print("error", e)
            time.sleep(2)
            titled = [r for r in rows if any(bare(r[1]) == bare(x) or norm(r[1]) == norm(x) for x in T)][:2]
            hit = any(any(norm(x) in norm(r[2]) or (norm(r[2]) and norm(r[2]) in norm(x)) for x in A) for r in titled)
            tot[(name, gl)] = tot.get((name, gl), 0) + hit
            out[f"{a} - {t} [{gl}] {name}"] = rows
            line.append(f"{name}={'Y' if hit else '-'}")
        print(f"{a} - {t} [{gl}]: " + "  ".join(line), flush=True)
json.dump(out, open(sys.argv[2], "w", encoding="utf-8"), ensure_ascii=False, indent=1)
for k, v in sorted(tot.items()): print(k, v, "/", len(SONGS))
