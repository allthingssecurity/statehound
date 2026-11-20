"""Collect the unique (spec, config) model-checking jobs recorded by past pipeline runs (their .tlc-tasks/jobs)."""
import json, glob, os, re, sys
runs = sys.argv[1]
seen = {}
for j in glob.glob(f"{runs}/*/*/.*-output/.tlc-tasks/jobs/*"):
    try:
        rq = json.load(open(f"{j}/request.json")); rs = json.load(open(f"{j}/result.json"))
    except Exception:
        continue
    opts = rq.get("options", [])
    if "-S" in opts:  # simulation jobs are not comparable
        continue
    wd = rq["work_dir"]; cfg = rq["config_file"]; spec = rq["spec_file"]
    cfgp = cfg if os.path.isabs(cfg) else os.path.join(wd, cfg)
    specp = spec if os.path.isabs(spec) else os.path.join(wd, spec)
    if not (os.path.exists(cfgp) and os.path.exists(specp)):
        continue
    log = open(f"{j}/tlc.log", errors="ignore").read() if os.path.exists(f"{j}/tlc.log") else ""
    m = re.findall(r"([\d,]+) states generated, ([\d,]+) distinct states found", log)
    t = rs.get("finished_at", 0) - rq["created_at"]
    key = (os.path.realpath(specp), os.path.realpath(cfgp))
    rec = dict(spec=key[0], cfg=key[1], options=opts, recorded_s=round(t, 1), run=j.split("/")[-6],
               recorded_distinct=int(m[-1][1].replace(",", "")) if m else None)
    # keep the most recent recording for each pair
    if key not in seen or rq["created_at"] > seen[key]["_t"]:
        rec["_t"] = rq["created_at"]; seen[key] = rec
out = sorted(seen.values(), key=lambda r: r["recorded_s"])
for r in out: r.pop("_t")
json.dump(out, open(sys.argv[2], "w"), indent=1)
print(len(out), "unique jobs;", sum(1 for r in out if r["recorded_s"] < 300), "recorded under 5 min")
