# No reference: how alike are a song's uploads to each other? Each pair fingerprinted whole and aligned.
import json, os, subprocess, sys, itertools
F = os.environ.get("FPTRIAL", "fptrial")
for D in sys.argv[1].split(","):
    plan = json.load(open(os.path.join(D, "plan.json"), encoding="utf-8"))
    lb = json.load(open(os.path.join(D, "labels.json"), encoding="utf-8")) if os.path.exists(os.path.join(D, "labels.json")) else {}
    for key, p in plan.items():
        if p["refs"].get("apple"): continue
        cs = [c for c in p["cands"] if c.get("src") == "youtube" and os.path.exists(os.path.join(D, c.get("file", "-")))]
        if len(cs) < 2: continue
        print(f"## {key}")
        for c in cs: print(f"   {c['file'][:11]} {lb.get(c['file'], {}).get('kind', c['kind']):12} {c['label'][:70]}")
        for a, b in itertools.combinations(cs, 2):
            out = subprocess.run([F, a["file"], b["file"]], capture_output=True, text=True, cwd=D, encoding="utf-8").stdout
            r = json.loads(out.splitlines()[-1])
            print(f"     {a['file'][:11]} ~ {b['file'][:11]}  ber {r.get('ber')}")
