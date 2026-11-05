# Downloads each planned YouTube candidate (AAC 140, the copy symphonia decodes), timed, then fingerprints
# every candidate against each reference. Polite: one download at a time, 4 s apart; files kept.
import json, os, subprocess, sys, time
D = sys.argv[1]
Y = os.environ.get("YTDLP", "yt-dlp")
F = os.environ.get("FPTRIAL", "fptrial")
plan = json.load(open(os.path.join(D, "plan.json"), encoding="utf-8"))
res_path = os.path.join(D, "results.json")
res = json.load(open(res_path, encoding="utf-8")) if os.path.exists(res_path) else {}
for key, p in plan.items():
    for c in p["cands"]:
        if c.get("src") != "youtube": continue
        f = c["id"] + ".m4a"; c["file"] = f
        path = os.path.join(D, f)
        if os.path.exists(path) or c.get("failed"): continue
        t = time.time()
        out = subprocess.run([Y, "-q", "--no-warnings", "-f", "140", "-o", path, "https://www.youtube.com/watch?v=" + c["id"]],
                             capture_output=True, text=True, encoding="utf-8")
        c["fetch_s"] = round(time.time() - t, 2)
        if not os.path.exists(path): c["failed"] = (out.stderr or "")[-200:]
        else: c["bytes"] = os.path.getsize(path)
        print(key, c["kind"], c.get("fetch_s"), c.get("bytes") or c.get("failed"), flush=True)
        time.sleep(4)
    json.dump(plan, open(os.path.join(D, "plan.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    for tier, ref in p["refs"].items():
        files = [c["file"] for c in p["cands"] if os.path.exists(os.path.join(D, c.get("file", "-")))]
        if not files: continue
        out = subprocess.run([F, ref] + files, capture_output=True, text=True, cwd=D, encoding="utf-8")
        for line in out.stdout.splitlines():
            r = json.loads(line)
            if "candidate" in r: res[f"{key}|{tier}|{r['candidate']}"] = r
    json.dump(res, open(res_path, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
print("done", len(res))
