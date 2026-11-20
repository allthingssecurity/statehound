//! `statehound`: a pure-Rust replacement for `java tlc2.TLC` (no JVM).
//!
//! * Exhaustive checks run on **everystate** (native Rust) with TLC's command line, output and exit
//!   codes.
//! * Next to the exhaustive search, **guided hunters** (`everystate -hunt`, different seeds) look for an
//!   invariant violation; the first violation found by either side is reported. A "no error" verdict
//!   always comes from the exhaustive search: a hunter can only end a run by finding a real violation.
//! * Identical checks are answered from a **content-addressed cache**.
//! * TLC options that everystate does not implement (`-dumpTrace`, `-loadTrace`, `-continue`,
//!   `-debugger`, ...) are rejected with a clear message instead of being silently ignored.
//!
//! Environment:
//!   STATEHOUND_EVERYSTATE      everystate binary (default: `everystate` on PATH)
//!   STATEHOUND_HUNTERS         number of hunters (default: 2 with 2+ workers, else 1)
//!   STATEHOUND_HUNT_OPTS       `-hunt` options (default: budget=60,max=5000000)
//!   STATEHOUND_CACHE           cache directory (default: ~/.cache/statehound; `off` disables)
//!   STATEHOUND_LOG             append one JSON line per run (decisions and timings) to this file
//!
//! The exhaustive search keeps every worker it was given. Hunters are extra processes at low CPU
//! priority with a short time budget: on the 70 violating models of the benchmark corpus every hunt
//! that succeeded did so within 0.8 s, so a passing check loses almost nothing to them.
mod args;
mod audit;
mod cache;
mod scan;

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
    audit::INTERRUPTED.store(true, Ordering::SeqCst);
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

struct Runner {
    everystate: Option<String>,
    started: Instant,
    log: Vec<(String, String)>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Role {
    Exhaustive,
    Hunter(usize),
}

enum Msg {
    Line(usize, String),
}

fn spawn(cmd: &mut Command) -> std::io::Result<Child> {
    use std::os::unix::process::CommandExt;
    cmd.env("EVERYSTATE_EXIT_WITH_PARENT", "1").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0).spawn()
}

fn kill_group(c: &Child) {
    unsafe {
        libc::killpg(c.id() as libc::pid_t, libc::SIGTERM);
    }
}
fn kill_group_hard(c: &Child) {
    unsafe {
        libc::killpg(c.id() as libc::pid_t, libc::SIGKILL);
    }
}

/// Forwards every line of `child`'s stdout and stderr to `tx`, then its exit status.
fn pump(idx: usize, child: &mut Child, tx: &mpsc::Sender<Msg>) -> Vec<std::thread::JoinHandle<()>> {
    let mut hs = vec![];
    if let Some(out) = child.stdout.take() {
        let tx = tx.clone();
        hs.push(std::thread::spawn(move || {
            for l in BufReader::new(out).lines().map_while(Result::ok) {
                let _ = tx.send(Msg::Line(idx, l));
            }
        }));
    }
    if let Some(err) = child.stderr.take() {
        let tx = tx.clone();
        hs.push(std::thread::spawn(move || {
            for l in BufReader::new(err).lines().map_while(Result::ok) {
                let _ = tx.send(Msg::Line(idx, l));
            }
        }));
    }
    hs
}

fn exit_code(st: std::process::ExitStatus) -> Option<i32> {
    st.code()
}

/// Whether the configuration names invariants or properties (what a hunter can find violations of).
/// When it cannot be read, assume it does: the hunters then simply find nothing.
fn has_invariants(cfg: &Path) -> bool {
    std::fs::read_to_string(cfg)
        .map(|t| t.lines().any(|l| {
            let w = l.trim_start();
            w.starts_with("INVARIANT") || w.starts_with("PROPERTY")
        }))
        .unwrap_or(true)
}

fn workers_of(inv: &args::Invocation) -> usize {
    match inv.workers.as_deref() {
        Some("auto") => std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        Some(w) => w.parse().unwrap_or(1).max(1),
        None => 1,
    }
}

/// The everystate arguments: TLC's, with `-workers` replaced and `-cdot` added when asked for.
fn es_args(inv: &args::Invocation, workers: usize) -> Vec<String> {
    let mut out = vec![];
    let mut i = 0;
    let a = &inv.tlc_args;
    while i < a.len() {
        if a[i] == "-workers" {
            i += 2;
            continue;
        }
        out.push(a[i].clone());
        i += 1;
    }
    out.push("-workers".into());
    out.push(workers.to_string());
    if inv.cdot && !out.iter().any(|x| x == "-cdot") {
        out.push("-cdot".into());
    }
    out
}

fn hunter_args(inv: &args::Invocation, k: usize) -> Vec<String> {
    let mut out = vec!["-config".to_string(), inv.config.clone().unwrap_or_default(), inv.spec.clone().unwrap_or_default()];
    let opts = env("STATEHOUND_HUNT_OPTS").unwrap_or_else(|| "budget=60,max=5000000".into());
    out.push("-hunt".into());
    out.push(opts);
    out.push("-seed".into());
    out.push((1_000_003 * (k as i64 + 1)).to_string());
    if inv.no_deadlock {
        out.push("-deadlock".into());
    }
    if inv.tool {
        out.push("-tool".into());
    }
    if inv.cdot {
        out.push("-cdot".into());
    }
    for (i, a) in inv.tlc_args.iter().enumerate() {
        if a == "-I" {
            if let Some(v) = inv.tlc_args.get(i + 1) {
                out.push("-I".into());
                out.push(v.clone());
            }
        }
    }
    out
}

/// The lines of a hunter's output that report its violation: from the error message (with its `-tool`
/// framing line, if any) to the end.
fn violation_report(lines: &[String]) -> &[String] {
    let pos = lines.iter().position(|l| l.contains("is violated") || l.contains("Deadlock reached") || l.starts_with("Error:"));
    match pos {
        Some(p) if p > 0 && lines[p - 1].starts_with("@!@!@STARTMSG") => &lines[p - 1..],
        Some(p) => &lines[p..],
        None => lines,
    }
}

impl Runner {
    fn note(&mut self, k: &str, v: impl ToString) {
        self.log.push((k.to_string(), v.to_string()));
    }

    fn write_log(&self) {
        if let Some(p) = env("STATEHOUND_LOG") {
            let fields: Vec<String> = self
                .log
                .iter()
                .map(|(k, v)| format!("\"{}\":\"{}\"", k, v.replace('\\', "\\\\").replace('"', "\\\"")))
                .collect();
            let line = format!("{{\"millis\":\"{}\",{}}}\n", self.started.elapsed().as_millis(), fields.join(","));
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
                let _ = f.write_all(line.as_bytes());
            }
        }
    }

    /// The everystate portfolio: the exit code and the captured output of the deciding engine.
    fn run_everystate(&mut self, es: &str, inv: &args::Invocation, out: &mut dyn Write) -> (i32, Vec<String>) {
        let workers = workers_of(inv);
        let cfg_path = inv.config.as_ref().map(PathBuf::from).unwrap_or_else(|| PathBuf::from(inv.spec.clone().unwrap_or_default().trim_end_matches(".tla").to_string() + ".cfg"));
        let want_hunt = !inv.simulate && (has_invariants(&cfg_path) || !inv.no_deadlock);
        let default_hunters = if workers >= 2 { 2 } else { 1 };
        let hunters = if want_hunt { env("STATEHOUND_HUNTERS").and_then(|h| h.parse().ok()).unwrap_or(default_hunters) } else { 0 };
        let bfs_workers = workers;
        self.note("hunters", hunters);
        self.note("bfs_workers", bfs_workers);

        let (tx, rx) = mpsc::channel::<Msg>();
        let mut children: Vec<(Role, Child)> = vec![];
        let mut pumps = vec![];
        let mut c0 = Command::new(es);
        c0.args(es_args(inv, bfs_workers));
        match spawn(&mut c0) {
            Ok(mut c) => {
                pumps.extend(pump(0, &mut c, &tx));
                children.push((Role::Exhaustive, c));
            }
            Err(e) => {
                let _ = writeln!(out, "statehound: cannot start everystate ({}): {}", es, e);
                self.note("everystate_spawn_error", e);
                return (255, vec![]);
            }
        }
        for k in 0..hunters {
            let mut ch = Command::new(es);
            ch.args(hunter_args(inv, k));
            {
                use std::os::unix::process::CommandExt;
                // low priority: the exhaustive search keeps the CPU it was budgeted
                unsafe {
                    ch.pre_exec(|| {
                        libc::setpriority(libc::PRIO_PROCESS, 0, 15);
                        Ok(())
                    });
                }
            }
            if let Ok(mut c) = spawn(&mut ch) {
                pumps.extend(pump(k + 1, &mut c, &tx));
                children.push((Role::Hunter(k), c));
            }
        }
        drop(tx);

        // Waiter threads report exits through the same channel.
        let mut exited: Vec<Option<Option<i32>>> = vec![None; children.len()];
        let mut buffers: Vec<Vec<String>> = vec![vec![]; children.len()];
        let mut streaming = false;
        let mut captured: Vec<String> = vec![];
        let mut winner: Option<usize> = None;
        let mut interrupted = false;
        loop {
            // drain output
            loop {
                match rx.try_recv() {
                    Ok(Msg::Line(idx, l)) => {
                        if idx == 0 {
                            if streaming {
                                let _ = writeln!(out, "{}", l);
                                captured.push(l);
                            } else {
                                let start = l.contains("Starting...");
                                buffers[0].push(l);
                                if start {
                                    streaming = true;
                                    for b in buffers[0].drain(..) {
                                        let _ = writeln!(out, "{}", b);
                                        captured.push(b);
                                    }
                                }
                            }
                        } else {
                            buffers[idx].push(l);
                        }
                    }
                    Err(_) => break,
                }
            }
            // poll exits
            let mut any_running = false;
            for (i, (_, c)) in children.iter_mut().enumerate() {
                if exited[i].is_some() {
                    continue;
                }
                match c.try_wait() {
                    Ok(Some(st)) => exited[i] = Some(exit_code(st)),
                    Ok(None) => any_running = true,
                    Err(_) => exited[i] = Some(None),
                }
            }
            if STOP.load(Ordering::SeqCst) {
                interrupted = true;
                break;
            }
            // decide
            if exited[0].is_some() {
                // the exhaustive search has ended: its verdict is final (unless setup failed)
                winner = Some(0);
                break;
            }
            for i in 1..children.len() {
                if let Some(Some(code)) = exited[i] {
                    if code == 12 || code == 11 {
                        winner = Some(i);
                    }
                }
            }
            if winner.is_some() || !any_running {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // stop everything that is still running
        for (i, (_, c)) in children.iter().enumerate() {
            if exited[i].is_none() {
                kill_group(c);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        for (i, (_, c)) in children.iter_mut().enumerate() {
            if exited[i].is_none() {
                loop {
                    match c.try_wait() {
                        Ok(Some(st)) => {
                            exited[i] = Some(exit_code(st));
                            break;
                        }
                        Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
                        _ => {
                            kill_group_hard(c);
                            exited[i] = Some(c.wait().ok().and_then(exit_code));
                            break;
                        }
                    }
                }
            }
        }
        for h in pumps {
            let _ = h.join();
        }
        // the rest of the output (all pumps have finished)
        while let Ok(m) = rx.try_recv() {
            let Msg::Line(idx, l) = m;
            {
                if idx == 0 && streaming {
                    if winner == Some(0) {
                        let _ = writeln!(out, "{}", l);
                        captured.push(l);
                    }
                } else {
                    buffers[idx].push(l);
                }
            }
        }
        if interrupted {
            self.note("outcome", "interrupted");
            return (143, captured);
        }
        match winner {
            Some(0) => {
                // the exhaustive search decides, whatever its status (errors in the spec included)
                if !streaming {
                    for b in buffers[0].drain(..) {
                        let _ = writeln!(out, "{}", b);
                        captured.push(b);
                    }
                }
                let code = match exited[0].flatten() {
                    Some(c) => c,
                    None => {
                        let _ = writeln!(out, "statehound: everystate was stopped by a signal");
                        255
                    }
                };
                self.note("decided_by", "exhaustive");
                (code, captured)
            }
            Some(i) => {
                let code = exited[i].flatten().unwrap_or(12);
                if !streaming {
                    // print the exhaustive search's header so that the log reads like one run
                    for b in buffers[0].drain(..) {
                        let _ = writeln!(out, "{}", b);
                        captured.push(b);
                    }
                }
                let report: Vec<String> = violation_report(&buffers[i]).to_vec();
                let _ = writeln!(out, "statehound: violation found by guided hunter {} (everystate -hunt); exhaustive search stopped", i);
                for l in &report {
                    let _ = writeln!(out, "{}", l);
                    captured.push(l.clone());
                }
                self.note("decided_by", format!("hunter{}", i));
                (code, captured)
            }
            None => {
                let _ = writeln!(out, "statehound: no checker produced a result");
                (255, captured)
            }
        }
    }
}

fn main() {
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGHUP, on_signal as *const () as libc::sighandler_t);
    }
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let code = match argv.first().map(String::as_str) {
        Some("audit") => audit::run(&argv[1..]),
        Some("scan") => run_scan(&argv[1..]),
        _ => run(&argv),
    };
    let _ = std::io::stdout().flush();
    std::process::exit(code);
}

/// `statehound scan <repo> [--json] [--limit N]`
fn run_scan(argv: &[String]) -> i32 {
    let mut repo = None;
    let mut json = false;
    let mut limit = 25usize;
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--json" => json = true,
            "--limit" => {
                i += 1;
                limit = argv.get(i).and_then(|v| v.parse().ok()).unwrap_or(limit);
            }
            a if !a.starts_with('-') => repo = Some(a.to_string()),
            a => {
                eprintln!("unknown option {}\nstatehound scan <repo> [--json] [--limit N]", a);
                return 2;
            }
        }
        i += 1;
    }
    let Some(repo) = repo else {
        eprintln!("statehound scan <repo> [--json] [--limit N]");
        return 2;
    };
    let dirs = scan::scan(Path::new(&repo));
    if json {
        println!("{}", scan::to_json(&dirs, limit));
    } else {
        println!("{:>6}  {:<8} {:>5}  directory  [signals]", "score", "language", "files");
        for d in dirs.iter().take(limit) {
            let sig: Vec<String> = d.signals.iter().map(|(k, v)| format!("{}:{}", k, v)).collect();
            println!("{:>6}  {:<8} {:>5}  {}  [{}]", d.score, d.language, d.files, d.dir, sig.join(" "));
        }
    }
    0
}

fn everystate_version(es: &str) -> Option<String> {
    let o = Command::new(es).arg("-version").output().ok()?;
    let mut s = String::new();
    o.stdout.as_slice().read_to_string(&mut s).ok()?;
    let v = s.lines().next().unwrap_or("").trim().to_string();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

fn run(argv: &[String]) -> i32 {
    let inv = args::parse(argv);
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut r = Runner { everystate: env("STATEHOUND_EVERYSTATE").or_else(|| Some("everystate".into())), started: Instant::now(), log: vec![] };
    r.note("args", inv.tlc_args.join(" "));
    let es = r.everystate.clone().unwrap();
    if let Some(why) = inv.unsupported {
        let _ = writeln!(
            out,
            "statehound: {} is not supported. statehound is pure Rust (the everystate engine) and does not run Java TLC; \
             drop the option (counterexamples are printed in TLC's text format, which TLC output readers parse).",
            why
        );
        r.note("unsupported", why);
        r.write_log();
        return 255;
    }
    let version = match everystate_version(&es) {
        Some(v) => v,
        None => {
            let _ = writeln!(out, "statehound: the everystate binary ({}) was not found or did not run; set STATEHOUND_EVERYSTATE", es);
            return 255;
        }
    };

    // ---- cache ----
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let cache = match env("STATEHOUND_CACHE").as_deref() {
        Some("off") => None,
        Some(d) => cache::Cache::open(PathBuf::from(d)),
        None => env("HOME").and_then(|h| cache::Cache::open(Path::new(&h).join(".cache/statehound"))),
    };
    let cacheable = !inv.writes_files && (!inv.simulate || inv.seeded);
    let key = match (&cache, &inv.spec, cacheable) {
        (Some(_), Some(spec), true) => {
            let spec_path = cwd.join(spec);
            let spec_dir = spec_path.parent().map(Path::to_path_buf).unwrap_or_else(|| cwd.clone());
            let cfg = inv.config.as_ref().map(|c| cwd.join(c)).unwrap_or_else(|| PathBuf::from(spec_path.to_string_lossy().trim_end_matches(".tla").to_string() + ".cfg"));
            cache::Cache::key(&version, &cwd, &inv.tlc_args, &spec_dir, &cfg, &inv.include_dirs)
        }
        _ => None,
    };
    if let (Some(c), Some(k)) = (&cache, &key) {
        if let Some(e) = c.get(k) {
            let _ = writeln!(out, "statehound: identical check found in the cache ({}); replaying its result", &k[..16]);
            let _ = out.write_all(&e.output);
            r.note("cache", "hit");
            r.note("code", e.code);
            r.write_log();
            return e.code;
        }
    }

    // ---- everystate portfolio ----
    let (code, captured) = r.run_everystate(&es, &inv, &mut out);
    if let (Some(c), Some(k)) = (&cache, &key) {
        if matches!(code, 0 | 10 | 11 | 12 | 13) {
            let mut text = captured.join("\n");
            text.push('\n');
            c.put(k, code, text.as_bytes());
        }
    }
    r.note("code", code);
    r.write_log();
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn violation_report_keeps_tool_framing() {
        let l: Vec<String> = ["a", "@!@!@STARTMSG 2110:1 @!@!@", "Invariant X is violated.", "b"].iter().map(|s| s.to_string()).collect();
        assert_eq!(violation_report(&l)[0], "@!@!@STARTMSG 2110:1 @!@!@");
        let l: Vec<String> = ["Starting...", "Error: Invariant X is violated.", "State 1:"].iter().map(|s| s.to_string()).collect();
        assert_eq!(violation_report(&l)[0], "Error: Invariant X is violated.");
    }

    #[test]
    fn es_args_replaces_workers_and_adds_cdot() {
        let inv = args::parse(&"--cdot -config a.cfg a.tla -workers 8 -deadlock".split_whitespace().map(String::from).collect::<Vec<_>>());
        assert_eq!(es_args(&inv, 6), "-config a.cfg a.tla -deadlock -workers 6 -cdot".split_whitespace().map(String::from).collect::<Vec<_>>());
    }
}
