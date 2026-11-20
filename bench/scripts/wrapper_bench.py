"""statehound (portfolio, cache off) vs the recorded TLC run, same flags, 4 workers, every corpus job."""
import json, os, re, subprocess, sys, time, tempfile
res, out, sc = sys.argv[1], sys.argv[2], sys.argv[3]
env = dict(os.environ, STATEHOUND_CACHE="off")
VERD = {0: "ok", 10: "assumption", 11: "deadlock", 12: "safety", 13: "liveness", 150: "parse-error", 151: "config-error"}
done = set()
if os.path.exists(out):
    for l in open(out): done.add(json.loads(l)["key"][1])
for l in open(res):
    r = json.loads(l)
    spec, cfg = r["key"]
    if cfg in done: continue
    t = time.time()
    try:
        p = subprocess.run([sc, "-config", cfg, os.path.basename(spec), "-workers", "4", "-metadir", tempfile.mkdtemp()] + r["flags"],
                           cwd=os.path.dirname(spec), capture_output=True, text=True, env=env, timeout=600)
        code, txt = p.returncode, p.stdout
    except subprocess.TimeoutExpired:
        code, txt = "timeout", ""
    dt = time.time() - t
    m = re.findall(r"([\d,]+) distinct states found", txt)
    by = "hunter" if "guided hunter" in txt else ("cache" if "identical check found in the cache" in txt else "exhaustive")
    rec = dict(key=r["key"], cfg=r["cfg"], code=code, verdict=VERD.get(code, str(code)), secs=round(dt, 2), decided_by=by,
               distinct=int(m[-1].replace(",", "")) if m else None, tlc_verdict=r["tlc"]["verdict"], tlc_secs=r["tlc"]["secs"],
               tlc_distinct=r["tlc"]["distinct"])
    rec["verdict_agrees"] = rec["verdict"] == rec["tlc_verdict"]
    with open(out, "a") as f: f.write(json.dumps(rec) + "\n")
    print(f'{r["cfg"][:40]:40} {rec["verdict"]:>9} by {by:10} {dt:7.2f}s   TLC {r["tlc"]["verdict"]:>9} {r["tlc"]["secs"]:7.2f}s  agree={rec["verdict_agrees"]}', flush=True)
