# Reads plan.json + results.json (+ labels.json overrides) and prints every candidate's score and the summary.
import json, os, statistics, sys
DS = sys.argv[1].split(","); TH = float(sys.argv[2]) if len(sys.argv) > 2 else 0.20
plan, res, over = {}, {}, {}
for D in DS:
    pl = json.load(open(os.path.join(D, "plan.json"), encoding="utf-8")); plan.update(pl)
    res.update(json.load(open(os.path.join(D, "results.json"), encoding="utf-8")))
    lb = json.load(open(os.path.join(D, "labels.json"), encoding="utf-8")) if os.path.exists(os.path.join(D, "labels.json")) else {}
    for k in pl:
        for f, v in lb.items(): over[(k, f)] = v
rows = []
for key, p in plan.items():
    for c in p["cands"]:
        f = c.get("file") or (c.get("id", "") + ".m4a")
        r = res.get(f"{key}|apple|{f}")
        o = over.get((key, f), {}); expect = o.get("expect", c["expect"]); kind = o.get("kind", c["kind"])
        rows.append(dict(song=key, file=f, kind=kind, expect=expect, label=c["label"], r=r, fetch_s=c.get("fetch_s"), bytes=c.get("bytes")))
if "-v" in sys.argv:
    for x in rows:
        r = x["r"]; s = f"ber={r['ber']:.3f} off={r['offset_s']:>6} ov={r['overlap_s']:>5} cand={r['cand_s']:>6}" if r and "ber" in r else ("no-ref" if not r else r.get("error"))
        print(f"{x['song'][:28]:28} {x['kind']:12} {x['expect']:7} {s:60} {x['file'][:16]:16} {x['label'][:70]}")
scored = [x for x in rows if x["r"] and "ber" in x["r"] and x["expect"] in ("pass", "refuse")]
print(f"\nscored {len(scored)} (threshold ber < {TH})")
for kind in sorted({x["kind"] for x in scored}):
    b = [x["r"]["ber"] for x in scored if x["kind"] == kind]
    print(f"  {kind:12} n={len(b):3} ber min {min(b):.3f} median {statistics.median(b):.3f} max {max(b):.3f}")
wa = [x for x in scored if x["expect"] == "refuse" and x["r"]["ber"] < TH]
rr = [x for x in scored if x["expect"] == "pass" and x["r"]["ber"] >= TH]
print("wrong accepted:", len(wa)); [print("   ", x["song"], x["kind"], x["r"]["ber"], x["label"][:80]) for x in wa]
print("right refused:", len(rr)); [print("   ", x["song"], x["kind"], x["r"]["ber"], x["label"][:80]) for x in rr]
offs = [x["r"]["offset_s"] for x in scored if x["expect"] == "pass" and x["r"]["ber"] < TH]
if offs: print("preview offset in song (s): p50", statistics.median(offs), "p90", sorted(offs)[int(len(offs)*0.9)-1], "max", max(offs))
f = [x["fetch_s"] for x in rows if x["fetch_s"]]; by = [x["bytes"] for x in rows if x["bytes"]]
if f: print("yt-dlp fetch (extract+download AAC 140) s: p50", statistics.median(f), "p90", sorted(f)[int(len(f)*0.9)-1])
if by: print("bytes per candidate MB: p50", round(statistics.median(by)/1e6, 2), "p90", round(sorted(by)[int(len(by)*0.9)-1]/1e6, 2))
cms = [x["r"]["cand_ms"] for x in scored]; rms = [x["r"]["ref_ms"] for x in scored]; mms = [x["r"]["match_ms"] for x in scored]
print("CPU ms: candidate decode+print p50", statistics.median(cms), "p90", sorted(cms)[int(len(cms)*0.9)-1], "| reference p50", statistics.median(rms), "| match p50", statistics.median(mms))

# The proposed rule: sound decides "same recording"; version words still refuse (sound can't hear a
# missing vocal); length refuses only far off (a loop, a compilation), else it is a hint.
import re as _re, unicodedata as _u
OTHER = ["live","cover","covers","covered","nightcore","sped up","speed up","spedup","slowed","reverb","8d","karaoke","instrumental",
         "remix","acoustic","unplugged","demo","reaction","tutorial","lesson","piano","guitar","drum","bass boosted","mashup","parody",
         "loop","hour","hours","concert","acapella","a cappella","extended"]
def _n(s):
    s = _u.normalize("NFKD", (s or "").lower()); s = "".join(c for c in s if not _u.combining(c))
    return " " + _re.sub(r"[\W_]+", " ", s).strip() + " "
LEN = float(sys.argv[3]) if len(sys.argv) > 3 else 0.25
def verdict(x, p):
    r = x["r"]; title = _n(x["label"].rsplit(" [", 1)[0]); own = _n(p["artist"] + " " + p["title"])
    if any(f" {w} " in title and f" {w} " not in own for w in OTHER): return "refuse-words"
    length = p.get("apple_len") or p["length"]
    if r["cand_s"] and abs(r["cand_s"] - length) > LEN * length: return "refuse-length"
    return "pass" if r["ber"] < TH else "refuse-sound"
wa2, rr2, why = [], [], {}
for x in scored:
    p = plan[x["song"]]; v = verdict(x, p); why[v] = why.get(v, 0) + 1
    if x["expect"] == "refuse" and v == "pass": wa2.append((x, v))
    if x["expect"] == "pass" and v != "pass": rr2.append((x, v))
print(f"\ncombined rule (ber < {TH}, version words, length within {LEN:.0%}): verdicts {why}")
print("  wrong accepted:", len(wa2)); [print("    ", x["song"], x["kind"], x["r"]["ber"], x["label"][:80]) for x, v in wa2]
print("  right refused:", len(rr2)); [print("    ", x["song"], x["kind"], v, x["r"]["ber"], round(x["r"]["cand_s"]), "vs", plan[x["song"]].get("apple_len") or plan[x["song"]]["length"], x["label"][:70]) for x, v in rr2]

# Roughly today's rule (youtube.md#matching, close): version words, the length within 5 s, the title read in
# the upload's title, the artist in its title or channel. Python stand-in, not the Rust judge.
def today(x, p):
    r = x["r"]; full = x["label"].rsplit(" [", 1); title = _n(full[0]); chan = _n(full[1] if len(full) > 1 else "")
    own = _n(p["artist"] + " " + p["title"]); length = p.get("apple_len") or p["length"]
    if any(f" {w} " in title and f" {w} " not in own for w in OTHER): return "refuse"
    if r["cand_s"] is None or abs(r["cand_s"] - length) > 5: return "refuse"
    if _n(_re.sub(r"\s*[\(\[].*?[\)\]]", "", p["title"])).strip() not in title: return "refuse"
    names = [a for a in _re.split(r",|&| feat\.? | ft\.? ", p["artist"].lower()) if a.strip()]
    if not any(_n(a).strip() in title + chan.replace(" topic ", " ") or _n(a).replace(" ", "").strip() in chan.replace(" ", "") for a in names): return "refuse"
    return "pass"
t_wa = [x for x in scored if x["expect"] == "refuse" and today(x, plan[x["song"]]) == "pass"]
t_rr = [x for x in scored if x["expect"] == "pass" and today(x, plan[x["song"]]) != "pass"]
print(f"\ntoday's words-and-length rule on the same candidates: wrong accepted {len(t_wa)}, right refused {len(t_rr)}")
for x in t_wa: print("   wrong accepted today:", x["song"], x["kind"], x["r"]["ber"], round(x["r"]["cand_s"]), x["label"][:80])
both = [x for x in t_rr if verdict(x, plan[x["song"]]) == "pass"]
print(f"   right copies today refuses that the sound rule plays: {len(both)}")
for x in both: print("     ", x["song"], x["kind"], x["r"]["ber"], round(x["r"]["cand_s"]), "vs", plan[x["song"]].get("apple_len") or plan[x["song"]]["length"], x["label"][:70])

# Three zones: under LOW the same recording; LOW..HIGH another master or mix of it, taken only when today's
# words-and-length rule also takes it; HIGH and over another recording.
LOW, HIGH = 0.15, 0.25
def zoned(x, p):
    v = verdict(x, p)
    if v in ("refuse-words", "refuse-length"): return v
    b = x["r"]["ber"]
    if b < LOW: return "pass"
    if b < HIGH: return "pass" if today(x, p) == "pass" else "unverified"
    return "refuse-sound"
z = {}
for x in scored: z.setdefault(zoned(x, plan[x["song"]]), []).append(x)
wa3 = [x for k, xs in z.items() if k == "pass" for x in xs if x["expect"] == "refuse"]
rr3 = [(k, x) for k, xs in z.items() if k != "pass" for x in xs if x["expect"] == "pass"]
print(f"\nthree zones (<{LOW} same, <{HIGH} with today's rule, else other): " + ", ".join(f"{k} {len(v)}" for k, v in z.items()))
print("  wrong accepted:", len(wa3)); [print("    ", x["song"], x["kind"], x["r"]["ber"], x["label"][:80]) for x in wa3]
print("  right refused or unverified:", len(rr3)); [print("    ", k, x["song"], x["kind"], x["r"]["ber"], x["label"][:70]) for k, x in rr3]
gain = [x for x in scored if x["expect"] == "pass" and today(x, plan[x["song"]]) != "pass" and zoned(x, plan[x["song"]]) == "pass"]
print("  right copies today refuses that this plays:", len(gain), "| right copies in all:", sum(x["expect"] == "pass" for x in scored), "| wrong in all:", sum(x["expect"] == "refuse" for x in scored))
