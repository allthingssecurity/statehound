"""Run TLC and everystate on each corpus job; record verdict, counts and wall time (JSONL)."""
import json, os, re, subprocess, sys, tempfile, time, shutil
corpus, out, es_bin = sys.argv[1], sys.argv[2], sys.argv[3]
TIMEOUT = int(os.environ.get("BENCH_TIMEOUT", "300")); W = os.environ.get("BENCH_WORKERS", "4")
java = os.environ.get("TLC_JAVA", "java")  # TLC is the reference being compared against
done = set()
if os.path.exists(out):
    for l in open(out): done.add(tuple(json.loads(l)["key"]))
VERD = {0: "ok", 10: "assumption", 11: "deadlock", 12: "safety", 13: "liveness", 150: "parse-error", 151: "config-error"}
def parse(txt):
    m = re.findall(r"([\d,]+) states generated, ([\d,]+) distinct states found", txt)
    inv = re.search(r"(Invariant|Action property|Temporal properties?) (\S+)? ?(is|were) violated", txt)
    tl = re.findall(r"^State (\d+):", txt, re.M)
    return (max(map(int, tl)) if tl else None,) + (int(m[-1][0].replace(",", "")) if m else None, int(m[-1][1].replace(",", "")) if m else None,
            inv.group(0) if inv else ("Deadlock" if "Deadlock reached" in txt else None))
def run(cmd, cwd):
    t = time.time()
    try:
        p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=TIMEOUT)
        txt, code = p.stdout + p.stderr, p.returncode
    except subprocess.TimeoutExpired as e:
        txt, code = (e.stdout or b"").decode(errors="ignore") if isinstance(e.stdout, bytes) else (e.stdout or ""), "timeout"
    dt = time.time() - t
    tlen, g, d, viol = parse(txt)
    return dict(code=code, trace_len=tlen, verdict=VERD.get(code, str(code)), secs=round(dt, 2), generated=g, distinct=d, violation=viol,
                tail=txt[-600:] if code not in (0, 12, 11, 13) else "")
for job in json.load(open(corpus)):
    key = (job["spec"], job["cfg"])
    if key in done or job["recorded_s"] > float(os.environ.get("BENCH_MAX_RECORDED", "1e9")): continue
    cwd = os.path.dirname(job["spec"]); spec = os.path.basename(job["spec"])
    meta = tempfile.mkdtemp(prefix="tlcmeta-")
    extra = ([] if "-D" in job.get("options", []) else ["-deadlock"]) + (["-continue"] if "-C" in job.get("options", []) else [])
    tlc = run([java, "-XX:+UseParallelGC", "-Xmx8g", "-cp", os.environ["TLC_CP"],
               "tlc2.TLC", "-config", job["cfg"], spec, "-workers", W, "-metadir", meta, "-cleanup"] + extra, cwd)
    shutil.rmtree(meta, ignore_errors=True)
    es = run([es_bin, "-config", job["cfg"], spec, "-workers", W] + extra, cwd)
    rec = dict(key=list(key), flags=extra, run=job["run"], cfg=os.path.basename(job["cfg"]), tlc=tlc, es=es,
               agree=(tlc["verdict"] == es["verdict"] and (
                   (tlc["distinct"] == es["distinct"]) if tlc["verdict"] == "ok" else
                   (tlc["violation"] == es["violation"] and tlc["trace_len"] == es["trace_len"]))))
    with open(out, "a") as f: f.write(json.dumps(rec) + "\n")
    print(f'{rec["cfg"][:40]:40} TLC {tlc["verdict"]:>8} {tlc["secs"]:7.1f}s d={tlc["distinct"]}  ES {es["verdict"]:>8} {es["secs"]:7.1f}s d={es["distinct"]} len={tlc["trace_len"]}/{es["trace_len"]} agree={rec["agree"]}', flush=True)
