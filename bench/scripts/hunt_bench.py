"""Guided hunter (1 thread) vs exhaustive everystate / TLC (4 workers) on violating corpus jobs."""
import json, os, re, subprocess, sys, time
res, out, es = sys.argv[1], sys.argv[2], sys.argv[3]
seeds = [int(x) for x in os.environ.get("SEEDS", "1,2,3").split(",")]
budget = os.environ.get("BUDGET", "60")
done = set()
if os.path.exists(out):
    for l in open(out): done.add((json.loads(l)["key"][1], json.loads(l)["seed"]))
env = dict(os.environ, EVERYSTATE_SIM_REPLAY="1")
for l in open(res):
    r = json.loads(l)
    if r["tlc"]["verdict"] not in ("safety", "deadlock") or r["es"]["verdict"] not in ("safety", "deadlock"):
        continue
    spec, cfg = r["key"]
    for seed in seeds:
        if (cfg, seed) in done: continue
        t = time.time()
        p = subprocess.run([es, "-config", cfg, os.path.basename(spec), "-hunt", f"budget={budget}" + (f",strategy={os.environ["STRATEGY"]}" if os.environ.get("STRATEGY") else ""), "-seed", str(seed)] + r["flags"],
                           cwd=os.path.dirname(spec), capture_output=True, text=True, env=env)
        dt = time.time() - t
        txt = p.stdout + p.stderr
        tl = re.findall(r"^State (\d+):", txt, re.M)
        v = re.search(r"(Invariant \S+ is violated|Action property \S+ is violated|Deadlock reached)", txt)
        rec = dict(key=r["key"], cfg=r["cfg"], seed=seed, code=p.returncode, secs=round(dt, 2), found=p.returncode in (11, 12),
                   violation=v.group(0) if v else None, trace_len=max(map(int, tl)) if tl else None,
                   replay_failed="not a behavior" in txt or "Internal error" in txt,
                   es_secs=r["es"]["secs"], tlc_secs=r["tlc"]["secs"], bfs_len=r["es"]["trace_len"], bfs_violation=r["es"]["violation"])
        with open(out, "a") as f: f.write(json.dumps(rec) + "\n")
        print(f'{r["cfg"][:40]:40} seed {seed} hunt {rec["found"]!s:5} {dt:6.2f}s len {rec["trace_len"]}/{rec["bfs_len"]}  ES {r["es"]["secs"]:6.2f}s TLC {r["tlc"]["secs"]:6.2f}s replay_failed={rec["replay_failed"]}', flush=True)
