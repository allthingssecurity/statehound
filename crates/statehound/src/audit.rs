//! `statehound audit`: formal verification of a whole repository.
//!
//! scan (deterministic) -> survey (one agent writes the queue of hunt areas) -> hunts (one agent per area,
//! N in parallel, each in its own git worktree) -> AUDIT.md. The agent instructions are the skill's
//! `references/survey.md` and `references/hunt-protocol.md`, compiled in.
use crate::scan;
use serde_json::Value;
use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SURVEY_MD: &str = include_str!("../../../skills/statehound/references/survey.md");
const HUNT_MD: &str = include_str!("../../../skills/statehound/references/hunt-protocol.md");

pub static INTERRUPTED: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------------------------
// agent profiles

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Agent {
    Claude,
    Codex,
}
impl Agent {
    fn name(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub agent: Agent,
    pub name: String,
    pub config_dir: PathBuf,
    /// The program to run: the agent CLI, or a wrapper script (`claude-<name>`, `codex-<name>`) that
    /// sets up this profile itself (proxies, credentials).
    pub program: PathBuf,
    pub wrapper: bool,
}

fn which(prog: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    if let Some(h) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(&h).join("bin"));
        dirs.push(PathBuf::from(h).join(".local/bin"));
    }
    dirs.into_iter().map(|d| d.join(prog)).find(|p| p.is_file())
}

/// Finds the installed profiles: `~/.claude` and `~/.claude-<name>` directories (with settings.json or
/// projects/), `~/.codex` and `~/.codex-<name>` (with config.toml or auth.json).
pub fn find_profiles(home: &Path) -> Vec<Profile> {
    let mut out = vec![];
    let mut entries: Vec<_> = fs::read_dir(home).map(|r| r.filter_map(|e| e.ok()).collect()).unwrap_or_default();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        let dir = e.path();
        if !dir.is_dir() {
            continue;
        }
        for (agent, base, markers) in [
            (Agent::Claude, ".claude", &["settings.json", "projects"][..]),
            (Agent::Codex, ".codex", &["config.toml", "auth.json"][..]),
        ] {
            let suffix = if name == base {
                Some("default".to_string())
            } else {
                name.strip_prefix(&format!("{}-", base)).map(|s| s.to_string())
            };
            let Some(pname) = suffix else { continue };
            if !markers.iter().any(|m| dir.join(m).exists()) {
                continue;
            }
            let cli = agent.name();
            let (program, wrapper) = match (pname.as_str(), which(&format!("{}-{}", cli, pname))) {
                ("default", _) | (_, None) => (which(cli).unwrap_or_else(|| PathBuf::from(cli)), false),
                (_, Some(w)) => (w, true),
            };
            out.push(Profile { agent, name: pname, config_dir: dir.clone(), program, wrapper });
        }
    }
    out
}

fn profile_line(p: &Profile) -> String {
    format!(
        "{:<7} {:<10} {}{}",
        p.agent.name(),
        p.name,
        p.config_dir.display(),
        if p.wrapper { format!("  (via {})", p.program.display()) } else { String::new() }
    )
}

// ---------------------------------------------------------------------------------------------
// run depth and estimates

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Depth {
    Short,
    Medium,
    Full,
}

pub struct DepthPlan {
    pub max_areas: usize,
    pub check_budget_s: u64,
    pub area_timeout_min: u64,
    /// minutes per area, low and high, before any area of this run has finished
    pub area_minutes: (f64, f64),
    pub survey_minutes: (f64, f64),
    pub instructions: &'static str,
}

impl Depth {
    pub fn parse(s: &str) -> Option<Depth> {
        match s {
            "short" | "quick" => Some(Depth::Short),
            "medium" | "standard" => Some(Depth::Medium),
            "full" | "deep" => Some(Depth::Full),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Depth::Short => "short",
            Depth::Medium => "medium",
            Depth::Full => "full",
        }
    }
    pub fn plan(self) -> DepthPlan {
        match self {
            Depth::Short => DepthPlan {
                max_areas: 4,
                check_budget_s: 120,
                area_timeout_min: 45,
                area_minutes: (15.0, 35.0),
                survey_minutes: (5.0, 10.0),
                instructions: "SHORT run. Scope ladder: rung R1 only (2 actors, every applicable failure model with bound 1); \
                               do not climb. Reproduce against the real code only the most severe candidate bug of this area; \
                               list the others as model-only leads. Keep model.md brief.",
            },
            Depth::Medium => DepthPlan {
                max_areas: 8,
                check_budget_s: 600,
                area_timeout_min: 90,
                area_minutes: (35.0, 75.0),
                survey_minutes: (10.0, 15.0),
                instructions: "MEDIUM run. Scope ladder: rungs R1 and R2, then R4 if the area has a `pairs_with` partner or shares \
                               state with a neighbouring mechanism. Reproduce against the real code every candidate bug of severity high or \
                               critical; list lower ones as model-only leads unless they are cheap to reproduce.",
            },
            Depth::Full => DepthPlan {
                max_areas: 20,
                check_budget_s: 1800,
                area_timeout_min: 180,
                area_minutes: (60.0, 180.0),
                survey_minutes: (15.0, 25.0),
                instructions: "FULL run. Scope ladder: rungs R1 to R4, then keep climbing (failure bounds, messages, keys, actors) \
                               until a check exceeds the per-check budget or the code has no new behaviour at a larger scope; \
                               hunt the scope that did not finish. Model every applicable failure model in the catalogue. Reproduce every candidate \
                               bug against the real code.",
            },
        }
    }
}

/// Cost per agent-minute, measured: confirmation sessions of the bug-finding pipeline on Claude Opus
/// averaged about $7 per 25 minutes.
const USD_PER_AGENT_MINUTE: f64 = 0.28;

pub fn estimate(plan: &DepthPlan, areas: usize, parallel: usize, survey_pending: bool) -> String {
    let waves = areas.div_ceil(parallel.max(1)) as f64;
    let (s_lo, s_hi) = if survey_pending { plan.survey_minutes } else { (0.0, 0.0) };
    let lo = s_lo + waves * plan.area_minutes.0;
    let hi = s_hi + waves * plan.area_minutes.1;
    let agent_lo = s_lo + areas as f64 * plan.area_minutes.0;
    let agent_hi = s_hi + areas as f64 * plan.area_minutes.1;
    format!(
        "{} area(s), {} in parallel: about {} to {} wall clock ({:.0} to {:.0} agent-minutes; roughly ${:.0} to ${:.0} at API prices). \
         Per area {:.0}-{:.0} min is an assumption until the first areas finish; the estimate is then recomputed from this run.",
        areas,
        parallel,
        fmt_minutes(lo),
        fmt_minutes(hi),
        agent_lo,
        agent_hi,
        agent_lo * USD_PER_AGENT_MINUTE,
        agent_hi * USD_PER_AGENT_MINUTE,
        plan.area_minutes.0,
        plan.area_minutes.1
    )
}

fn fmt_minutes(m: f64) -> String {
    if m < 90.0 {
        format!("{:.0} min", m)
    } else {
        format!("{:.1} h", m / 60.0)
    }
}

// ---------------------------------------------------------------------------------------------
// options

pub struct Options {
    pub repo: PathBuf,
    pub out: PathBuf,
    pub agent: Option<Agent>,
    pub profile: Option<String>,
    pub parallel: usize,
    pub depth: Depth,
    pub max_areas: Option<usize>,
    pub check_budget_s: Option<u64>,
    pub area_timeout_min: Option<u64>,
    pub model: Option<String>,
    pub areas_file: Option<PathBuf>,
    pub dry_run: bool,
    pub yes: bool,
    pub list_profiles: bool,
}

const USAGE: &str = "statehound audit <repo> [--depth short|medium|full] [--agent claude|codex] [--profile NAME]
                 [--parallel N] [--max-areas N] [--check-budget SECONDS] [--area-timeout MINUTES]
                 [--model NAME] [--areas FILE] [--out DIR] [--dry-run] [--yes]
statehound audit --list-profiles
statehound scan <repo> [--json] [--limit N]";

pub fn parse_options(argv: &[String]) -> Result<Options, String> {
    let mut o = Options {
        repo: PathBuf::new(),
        out: PathBuf::new(),
        agent: None,
        profile: None,
        parallel: 3,
        depth: Depth::Medium,
        max_areas: None,
        check_budget_s: None,
        area_timeout_min: None,
        model: None,
        areas_file: None,
        dry_run: false,
        yes: false,
        list_profiles: false,
    };
    let mut repo: Option<String> = None;
    let mut out: Option<String> = None;
    let mut i = 0;
    let val = |i: &mut usize| -> Result<String, String> {
        *i += 1;
        argv.get(*i).cloned().ok_or_else(|| format!("{} needs a value\n{}", argv[*i - 1], USAGE))
    };
    while i < argv.len() {
        match argv[i].as_str() {
            "--depth" => o.depth = Depth::parse(&val(&mut i)?).ok_or_else(|| format!("--depth is short, medium or full\n{}", USAGE))?,
            "--agent" => {
                o.agent = Some(match val(&mut i)?.as_str() {
                    "claude" => Agent::Claude,
                    "codex" => Agent::Codex,
                    a => return Err(format!("unknown agent '{}' (claude or codex)", a)),
                })
            }
            "--profile" => o.profile = Some(val(&mut i)?),
            "--parallel" => o.parallel = val(&mut i)?.parse::<usize>().map_err(|_| "--parallel needs a number".to_string())?.max(1),
            "--max-areas" => o.max_areas = Some(val(&mut i)?.parse().map_err(|_| "--max-areas needs a number".to_string())?),
            "--check-budget" => o.check_budget_s = Some(val(&mut i)?.parse().map_err(|_| "--check-budget needs seconds".to_string())?),
            "--area-timeout" => o.area_timeout_min = Some(val(&mut i)?.parse().map_err(|_| "--area-timeout needs minutes".to_string())?),
            "--model" => o.model = Some(val(&mut i)?),
            "--areas" => o.areas_file = Some(PathBuf::from(val(&mut i)?)),
            "--out" => out = Some(val(&mut i)?),
            "--dry-run" => o.dry_run = true,
            "--yes" | "-y" => o.yes = true,
            "--list-profiles" => o.list_profiles = true,
            "-h" | "--help" => return Err(USAGE.to_string()),
            a if a.starts_with('-') => return Err(format!("unknown option {}\n{}", a, USAGE)),
            a => {
                if repo.replace(a.to_string()).is_some() {
                    return Err(format!("only one repository\n{}", USAGE));
                }
            }
        }
        i += 1;
    }
    if o.list_profiles {
        return Ok(o);
    }
    let repo = repo.ok_or_else(|| USAGE.to_string())?;
    o.repo = fs::canonicalize(&repo).map_err(|e| format!("{}: {}", repo, e))?;
    let name = o.repo.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "repo".into());
    o.out = match out {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".statehound/audits").join(name),
    };
    Ok(o)
}

// ---------------------------------------------------------------------------------------------
// helpers

fn is_tty() -> bool {
    unsafe { libc::isatty(0) == 1 && libc::isatty(2) == 1 }
}

fn ask(prompt: &str) -> String {
    eprint!("{}", prompt);
    let _ = std::io::stderr().flush();
    let mut s = String::new();
    let _ = std::io::stdin().lock().read_line(&mut s);
    s.trim().to_string()
}

fn git(repo: &Path, args: &[&str]) -> Option<String> {
    let o = Command::new("git").arg("-C").arg(repo).args(args).output().ok()?;
    if o.status.success() {
        Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        None
    }
}

/// A private copy of the repository at `dest`: a detached git worktree with the uncommitted changes
/// applied, or a plain copy when the repository is not a git repository.
fn make_worktree(repo: &Path, dest: &Path) -> Result<(), String> {
    if dest.exists() {
        return Ok(());
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if git(repo, &["rev-parse", "--is-inside-work-tree"]).as_deref() == Some("true") {
        let st = Command::new("git").arg("-C").arg(repo).args(["worktree", "add", "--detach"]).arg(dest).arg("HEAD").stdout(Stdio::null()).stderr(Stdio::piped()).output().map_err(|e| e.to_string())?;
        if !st.status.success() {
            return Err(format!("git worktree add failed: {}", String::from_utf8_lossy(&st.stderr).trim()));
        }
        // carry over uncommitted changes to tracked files
        let diff = Command::new("git").arg("-C").arg(repo).args(["diff", "--binary", "HEAD"]).output().map_err(|e| e.to_string())?;
        if !diff.stdout.is_empty() {
            let mut ap = Command::new("git").arg("-C").arg(dest).args(["apply", "--whitespace=nowarn"]).stdin(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
            ap.stdin.take().unwrap().write_all(&diff.stdout).map_err(|e| e.to_string())?;
            let _ = ap.wait();
        }
        Ok(())
    } else {
        let st = Command::new("cp").arg("-R").arg(repo).arg(dest).status().map_err(|e| e.to_string())?;
        if st.success() {
            Ok(())
        } else {
            Err("copying the repository failed".into())
        }
    }
}

fn binary_path(name: &str) -> String {
    if name == "statehound" {
        if let Ok(p) = std::env::current_exe() {
            return p.to_string_lossy().to_string();
        }
    }
    if name == "everystate" {
        if let Ok(e) = std::env::var("STATEHOUND_EVERYSTATE") {
            return e;
        }
    }
    which(name).map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| name.to_string())
}

/// Runs one agent session headless; returns (exit status, minutes). Output goes to `log_dir`.
fn run_agent(p: &Profile, model: Option<&str>, prompt: &str, cwd: &Path, log_dir: &Path, extra_dir: &Path, timeout: Duration) -> (Option<i32>, f64) {
    use std::os::unix::process::CommandExt;
    let _ = fs::create_dir_all(log_dir);
    let prompt_file = log_dir.join("prompt.md");
    let _ = fs::write(&prompt_file, prompt);
    let mut cmd = Command::new(&p.program);
    match p.agent {
        Agent::Claude => {
            cmd.args(["--print", "--dangerously-skip-permissions", "--output-format", "json", "--add-dir"]).arg(extra_dir);
            if let Some(m) = model {
                cmd.args(["--model", m]);
            }
            if !p.wrapper {
                cmd.env("CLAUDE_CONFIG_DIR", &p.config_dir);
            }
        }
        Agent::Codex => {
            cmd.args(["exec", "--dangerously-bypass-approvals-and-sandbox", "--skip-git-repo-check", "--json", "-C"]).arg(cwd);
            cmd.arg("-o").arg(log_dir.join("last-message.md"));
            if let Some(m) = model {
                cmd.args(["-m", m]);
            }
            cmd.arg("-");
            if !p.wrapper {
                cmd.env("CODEX_HOME", &p.config_dir);
            }
        }
    }
    let stdin = match fs::File::open(&prompt_file) {
        Ok(f) => f,
        Err(_) => return (None, 0.0),
    };
    let out = fs::File::create(log_dir.join("agent.out")).ok();
    let err = fs::File::create(log_dir.join("agent.err")).ok();
    cmd.current_dir(cwd).stdin(stdin).process_group(0);
    if let (Some(o), Some(e)) = (out, err) {
        cmd.stdout(o).stderr(e);
    }
    let t0 = Instant::now();
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = fs::write(log_dir.join("agent.err"), format!("cannot start {}: {}\n", p.program.display(), e));
            return (None, 0.0);
        }
    };
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return (st.code(), t0.elapsed().as_secs_f64() / 60.0),
            Ok(None) => {}
            Err(_) => return (None, t0.elapsed().as_secs_f64() / 60.0),
        }
        if INTERRUPTED.load(Ordering::SeqCst) || t0.elapsed() > timeout {
            unsafe {
                libc::killpg(child.id() as libc::pid_t, libc::SIGTERM);
            }
            std::thread::sleep(Duration::from_secs(5));
            unsafe {
                libc::killpg(child.id() as libc::pid_t, libc::SIGKILL);
            }
            let _ = child.wait();
            let why = if INTERRUPTED.load(Ordering::SeqCst) { "interrupted" } else { "time box reached" };
            let _ = fs::write(log_dir.join("stopped"), format!("{}\n", why));
            return (None, t0.elapsed().as_secs_f64() / 60.0);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn area_id(a: &Value, i: usize) -> String {
    let raw = a.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let id: String = raw.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '-' }).collect();
    let id = id.trim_matches('-').to_string();
    if id.is_empty() {
        format!("area-{}", i + 1)
    } else {
        id
    }
}

fn read_report(dir: &Path) -> Option<Value> {
    let v: Value = serde_json::from_str(&fs::read_to_string(dir.join("report.json")).ok()?).ok()?;
    v.get("status")?.as_str()?;
    Some(v)
}

// ---------------------------------------------------------------------------------------------
// the audit

pub fn run(argv: &[String]) -> i32 {
    let o = match parse_options(argv) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{}", e);
            return 2;
        }
    };
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()));
    let profiles = find_profiles(&home);
    if o.list_profiles {
        if profiles.is_empty() {
            println!("no Claude Code or Codex profile found (looked for ~/.claude*, ~/.codex*)");
        }
        for p in &profiles {
            println!("{}", profile_line(p));
        }
        return 0;
    }

    // ---- agent profile ----
    let agent = o.agent.unwrap_or_else(|| if profiles.iter().any(|p| p.agent == Agent::Claude) { Agent::Claude } else { Agent::Codex });
    let candidates: Vec<&Profile> = profiles.iter().filter(|p| p.agent == agent).collect();
    let profile: Profile = match &o.profile {
        Some(name) => match candidates.iter().find(|p| &p.name == name) {
            Some(p) => (*p).clone(),
            None => {
                eprintln!("no {} profile named '{}'; installed: {}", agent.name(), name, candidates.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", "));
                return 2;
            }
        },
        None if candidates.len() > 1 && is_tty() && !o.yes => {
            eprintln!("Several {} installations were found:", agent.name());
            for (i, p) in candidates.iter().enumerate() {
                eprintln!("  {}) {}", i + 1, profile_line(p));
            }
            let def = candidates.iter().position(|p| p.name == "default").unwrap_or(0);
            let a = ask(&format!("Which one should run the audit? [{}] ", def + 1));
            let k = if a.is_empty() { def } else { a.parse::<usize>().ok().and_then(|n| n.checked_sub(1)).filter(|&n| n < candidates.len()).unwrap_or(def) };
            candidates[k].clone()
        }
        None => match candidates.iter().find(|p| p.name == "default").or(candidates.first()) {
            Some(p) => {
                if candidates.len() > 1 {
                    eprintln!(
                        "note: using the '{}' {} profile; others: {} (choose with --profile)",
                        p.name,
                        agent.name(),
                        candidates.iter().filter(|q| q.name != p.name).map(|q| q.name.as_str()).collect::<Vec<_>>().join(", ")
                    );
                }
                (*p).clone()
            }
            None => {
                eprintln!("no {} installation found (looked for ~/.{}*); install it or pass --agent", agent.name(), agent.name());
                return 2;
            }
        },
    };

    let plan = o.depth.plan();
    let max_areas = o.max_areas.unwrap_or(plan.max_areas);
    let check_budget = o.check_budget_s.unwrap_or(plan.check_budget_s);
    let area_timeout = Duration::from_secs(60 * o.area_timeout_min.unwrap_or(plan.area_timeout_min));
    if let Err(e) = fs::create_dir_all(&o.out) {
        eprintln!("cannot create {}: {}", o.out.display(), e);
        return 2;
    }
    let commit = git(&o.repo, &["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "(not a git repository)".into());
    eprintln!("statehound audit: {} @ {}", o.repo.display(), commit);
    eprintln!("  depth {}, agent {} ({}), {} in parallel, output {}", o.depth.name(), agent.name(), profile.name, o.parallel, o.out.display());

    // ---- scan ----
    let dirs = scan::scan(&o.repo);
    let scan_json = scan::to_json(&dirs, 60);
    let _ = fs::write(o.out.join("scan.json"), &scan_json);
    eprintln!("  scan: {} directories with concurrency signals (top: {})", dirs.len(), dirs.iter().take(3).map(|d| d.dir.as_str()).collect::<Vec<_>>().join(", "));
    if dirs.is_empty() && o.areas_file.is_none() {
        eprintln!("  no concurrency or distributed-systems signals found: nothing to hunt.");
        return 0;
    }

    // ---- survey ----
    let areas_path = o.areas_file.clone().unwrap_or_else(|| o.out.join("hunt-areas.json"));
    let survey_needed = !areas_path.exists();
    let survey_prompt = format!(
        "{}\n\n## Task\n\nRepository: {}\nWrite the hunt areas to: {}\nAt most {} areas.\n\n`statehound scan` output (top directories):\n\n```json\n{}\n```\n",
        SURVEY_MD,
        o.repo.display(),
        areas_path.display(),
        max_areas,
        scan_json
    );
    if survey_needed {
        eprintln!("  estimate: {}", estimate(&plan, max_areas, o.parallel, true));
    }
    if o.dry_run {
        let _ = fs::create_dir_all(o.out.join("survey"));
        let _ = fs::write(o.out.join("survey/prompt.md"), &survey_prompt);
        eprintln!("  dry run: wrote {} and {}; no agent was started.", o.out.join("scan.json").display(), o.out.join("survey/prompt.md").display());
        return 0;
    }
    if survey_needed {
        if is_tty() && !o.yes && ask("Start the survey? [Y/n] ").to_lowercase().starts_with('n') {
            return 1;
        }
        eprintln!("  survey: one {} session reads the code and writes the queue of hunt areas ...", agent.name());
        let wt = o.out.join("survey/worktree");
        if let Err(e) = make_worktree(&o.repo, &wt) {
            eprintln!("  {}", e);
            return 1;
        }
        let (code, mins) = run_agent(&profile, o.model.as_deref(), &survey_prompt, &wt, &o.out.join("survey"), &o.out, Duration::from_secs(45 * 60));
        eprintln!("  survey finished in {:.0} min (status {:?})", mins, code);
        if !areas_path.exists() {
            eprintln!("  the survey did not write {}; see {}/survey/agent.out", areas_path.display(), o.out.display());
            return 1;
        }
    }
    let areas: Vec<Value> = match fs::read_to_string(&areas_path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        Some(Value::Array(a)) => a,
        _ => {
            eprintln!("  {} is not a JSON array of hunt areas", areas_path.display());
            return 1;
        }
    };
    let mut areas: Vec<(String, Value)> = areas.into_iter().enumerate().map(|(i, a)| (area_id(&a, i), a)).collect();
    areas.sort_by_key(|(_, a)| a.get("priority").and_then(|p| p.as_u64()).unwrap_or(99));
    areas.truncate(max_areas);
    if areas.is_empty() {
        eprintln!("  the survey found no hunt area (see {}/survey)", o.out.display());
        return 0;
    }
    let todo: Vec<(String, Value)> = areas.iter().filter(|(id, _)| read_report(&o.out.join("areas").join(id)).is_none()).cloned().collect();
    eprintln!("  {} hunt area(s): {}", areas.len(), areas.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>().join(", "));
    if todo.len() < areas.len() {
        eprintln!("  resuming: {} already reported, {} to go", areas.len() - todo.len(), todo.len());
    }
    eprintln!("  estimate: {}", estimate(&plan, todo.len(), o.parallel, false));
    if !todo.is_empty() && is_tty() && !o.yes && ask("Start the hunts? [Y/n] ").to_lowercase().starts_with('n') {
        return 1;
    }

    // ---- hunts ----
    let statehound_bin = binary_path("statehound");
    let everystate_bin = binary_path("everystate");
    let queue = Arc::new(Mutex::new(todo.into_iter().collect::<VecDeque<_>>()));
    let durations = Arc::new(Mutex::new(Vec::<f64>::new()));
    let total = queue.lock().unwrap().len();
    let started = Instant::now();
    let mut workers = vec![];
    for _ in 0..o.parallel.min(total.max(1)) {
        let (queue, durations) = (queue.clone(), durations.clone());
        let (out, repo, profile, model) = (o.out.clone(), o.repo.clone(), profile.clone(), o.model.clone());
        let (sb, eb) = (statehound_bin.clone(), everystate_bin.clone());
        let instructions = plan.instructions;
        let parallel = o.parallel;
        workers.push(std::thread::spawn(move || loop {
            if INTERRUPTED.load(Ordering::SeqCst) {
                return;
            }
            let next = queue.lock().unwrap().pop_front();
            let Some((id, area)) = next else { return };
            let dir = out.join("areas").join(&id);
            let wt = dir.join("worktree");
            let _ = fs::create_dir_all(&dir);
            if let Err(e) = make_worktree(&repo, &wt) {
                let _ = fs::write(dir.join("report.json"), serde_json::json!({"area": id, "status": "blocked", "findings": [], "verified": [], "notes": e}).to_string());
                continue;
            }
            let prompt = format!(
                "{}\n\n## Task\n\nHunt area (from the survey):\n\n```json\n{}\n```\n\n- Worktree (a private copy of the repository; work and build here): {}\n- Area directory (write model.md, spec/, checks.md, repro/, report.md, report.json here): {}\n- statehound: {}\n- everystate: {}\n- Per-check budget: {} seconds\n- {}\n",
                HUNT_MD,
                serde_json::to_string_pretty(&area).unwrap_or_default(),
                wt.display(),
                dir.display(),
                sb,
                eb,
                check_budget,
                instructions
            );
            eprintln!("  [{}] started", id);
            let (code, mins) = run_agent(&profile, model.as_deref(), &prompt, &wt, &dir.join("agent"), &dir, area_timeout);
            if read_report(&dir).is_none() {
                let why = fs::read_to_string(dir.join("agent/stopped")).unwrap_or_else(|_| format!("the agent ended (status {:?}) without writing report.json", code));
                let _ = fs::write(dir.join("report.json"), serde_json::json!({"area": id, "status": "incomplete", "findings": [], "verified": [], "notes": why.trim()}).to_string());
            }
            let status = read_report(&dir).and_then(|r| r.get("status").and_then(|s| s.as_str()).map(String::from)).unwrap_or_default();
            let mut d = durations.lock().unwrap();
            d.push(mins);
            let done = d.len();
            let mean = d.iter().sum::<f64>() / done as f64;
            let left = total - done;
            let eta = (left as f64 / parallel as f64).ceil() * mean;
            eprintln!(
                "  [{}] {} in {:.0} min  ({}/{} done; measured {:.0} min per area; about {} left)",
                id,
                status,
                mins,
                done,
                total,
                mean,
                if left == 0 { "nothing".to_string() } else { fmt_minutes(eta) }
            );
        }));
    }
    for w in workers {
        let _ = w.join();
    }
    let elapsed = started.elapsed().as_secs_f64() / 60.0;

    // ---- aggregate ----
    let reports: Vec<(String, Value)> = areas.iter().map(|(id, _)| (id.clone(), read_report(&o.out.join("areas").join(id)).unwrap_or_else(|| serde_json::json!({"area": id, "status": "not-run"})))).collect();
    let md = audit_markdown(&o.repo, &commit, o.depth, &profile, &reports, &areas, elapsed);
    let _ = fs::write(o.out.join("AUDIT.md"), &md);
    let _ = fs::write(o.out.join("audit.json"), serde_json::to_string_pretty(&Value::Array(reports.iter().map(|(_, r)| r.clone()).collect())).unwrap_or_default());
    eprintln!("  wrote {}", o.out.join("AUDIT.md").display());
    if INTERRUPTED.load(Ordering::SeqCst) {
        eprintln!("  interrupted: run the same command again to resume.");
        return 130;
    }
    0
}

fn findings(r: &Value) -> Vec<Value> {
    r.get("findings").and_then(|f| f.as_array()).cloned().unwrap_or_default()
}
fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

pub fn audit_markdown(repo: &Path, commit: &str, depth: Depth, p: &Profile, reports: &[(String, Value)], areas: &[(String, Value)], minutes: f64) -> String {
    let title_of = |id: &str| areas.iter().find(|(i, _)| i == id).map(|(_, a)| s(a, "title").to_string()).unwrap_or_default();
    let all: Vec<(String, Value)> = reports.iter().flat_map(|(id, r)| findings(r).into_iter().map(move |f| (id.clone(), f))).collect();
    let rank = |f: &Value| match s(f, "severity") {
        "critical" => 0,
        "high" => 1,
        "medium" => 2,
        "low" => 3,
        _ => 4,
    };
    let mut confirmed: Vec<&(String, Value)> = all.iter().filter(|(_, f)| s(f, "classification") == "confirmed").collect();
    confirmed.sort_by_key(|(_, f)| rank(f));
    let mut leads: Vec<&(String, Value)> = all.iter().filter(|(_, f)| s(f, "classification") == "model-only").collect();
    leads.sort_by_key(|(_, f)| rank(f));
    let mut m = format!(
        "# Formal verification audit\n\n- Repository: `{}` at `{}`\n- Depth: {}; agent: {} ({}); {} hunt area(s); {:.0} min\n- Confirmed bugs: **{}**; model-only leads: {}\n\n",
        repo.display(),
        commit,
        depth.name(),
        p.agent.name(),
        p.name,
        reports.len(),
        minutes,
        confirmed.len(),
        leads.len()
    );
    m += "| Area | Status | Confirmed | Leads | Verified properties |\n|---|---|---:|---:|---:|\n";
    for (id, r) in reports {
        let f = findings(r);
        m += &format!(
            "| [{}](areas/{}/report.md) {} | {} | {} | {} | {} |\n",
            id,
            id,
            title_of(id),
            s(r, "status"),
            f.iter().filter(|x| s(x, "classification") == "confirmed").count(),
            f.iter().filter(|x| s(x, "classification") == "model-only").count(),
            r.get("verified").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0)
        );
    }
    let block = |m: &mut String, head: &str, list: &[&(String, Value)]| {
        if list.is_empty() {
            return;
        }
        *m += &format!("\n## {}\n", head);
        for (id, f) in list {
            *m += &format!(
                "\n### {} ({})\n\n- Area: [{}](areas/{}/report.md); property: `{}`\n- Code: {}\n- Trace: `areas/{}/{}`\n- Reproduction: `areas/{}/{}`\n\n{}\n",
                s(f, "title"),
                if s(f, "severity").is_empty() { "unrated" } else { s(f, "severity") },
                id,
                id,
                s(f, "property"),
                f.get("code").and_then(|c| c.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).map(|x| format!("`{}`", x)).collect::<Vec<_>>().join(", ")).unwrap_or_default(),
                id,
                s(f, "trace"),
                id,
                if s(f, "reproduction").is_empty() { "(none)" } else { s(f, "reproduction") },
                s(f, "summary")
            );
        }
    };
    block(&mut m, "Confirmed bugs", &confirmed);
    block(&mut m, "Model-only leads (not reproduced against the code)", &leads);
    let fm: Vec<&(String, Value)> = reports.iter().filter(|(_, r)| r.get("failure_models").is_some()).collect();
    if !fm.is_empty() {
        let list = |v: Option<&Value>| -> String {
            v.and_then(|x| x.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string()).or_else(|| x.get("model").and_then(|m| m.as_str()).map(|s| s.to_string()))).collect::<Vec<_>>().join(", "))
                .unwrap_or_default()
        };
        m += "\n## Failure models\n\nWhich failure models each area put into its spec. A model that applies but was skipped is a gap in what was verified.\n\n| Area | Rungs | Modelled | Skipped | Not applicable |\n|---|---|---|---|---|\n";
        for (id, r) in fm {
            let f = r.get("failure_models");
            m += &format!(
                "| {} | {} | {} | {} | {} |\n",
                id,
                list(r.get("rungs")),
                list(f.and_then(|f| f.get("modelled"))),
                list(f.and_then(|f| f.get("skipped"))),
                list(f.and_then(|f| f.get("not_applicable")))
            );
        }
    }
    m += "\n## Verified\n\nNo violation within these bounds (an exhaustive check covers every state up to its scope).\n\n| Area | Property | Scope | Result |\n|---|---|---|---|\n";
    for (id, r) in reports {
        for v in r.get("verified").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
            m += &format!("| {} | `{}` | {} | {} |\n", id, s(&v, "property"), s(&v, "scope"), s(&v, "result"));
        }
    }
    let other: Vec<&(String, Value)> = reports.iter().filter(|(_, r)| matches!(s(r, "status"), "blocked" | "incomplete" | "not-run")).collect();
    if !other.is_empty() {
        m += "\n## Not completed\n\n";
        for (id, r) in other {
            m += &format!("- **{}** ({}): {}\n", id, s(r, "status"), s(r, "notes"));
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_profiles_with_wrappers_and_markers() {
        let d = tempfile::tempdir().unwrap();
        let h = d.path();
        for (dir, marker) in [(".claude", "settings.json"), (".claude-account2", "projects"), (".codex", "config.toml"), (".codex-empty", "nothing")] {
            fs::create_dir_all(h.join(dir)).unwrap();
            fs::write(h.join(dir).join(marker), "").unwrap();
        }
        fs::write(h.join(".claude.json"), "{}").unwrap(); // a file, not a profile
        let ps = find_profiles(h);
        let names: Vec<String> = ps.iter().map(|p| format!("{}:{}", p.agent.name(), p.name)).collect();
        assert_eq!(names, vec!["claude:default", "claude:account2", "codex:default"]);
    }

    #[test]
    fn estimates_scale_with_waves() {
        let p = Depth::Medium.plan();
        let one = estimate(&p, 3, 3, false);
        let two = estimate(&p, 6, 3, false);
        assert!(one.contains("35 min to 75 min"), "{}", one);
        assert!(two.contains("70 min to 2.5 h"), "{}", two);
        assert!(estimate(&Depth::Short.plan(), 4, 2, true).contains("4 area(s), 2 in parallel"));
    }

    #[test]
    fn parses_options_and_depths() {
        let a: Vec<String> = ["/", "--depth", "short", "--agent", "codex", "--parallel", "5", "--yes"].iter().map(|s| s.to_string()).collect();
        let o = parse_options(&a).unwrap();
        assert_eq!(o.depth, Depth::Short);
        assert_eq!(o.agent, Some(Agent::Codex));
        assert_eq!(o.parallel, 5);
        assert!(parse_options(&["/".into(), "--depth".into(), "huge".into()]).is_err());
    }

    #[test]
    fn audit_markdown_lists_failure_model_coverage() {
        let p = Profile { agent: Agent::Claude, name: "default".into(), config_dir: "/x".into(), program: "claude".into(), wrapper: false };
        let r = serde_json::json!({"area":"a","status":"verified","findings":[],"verified":[],"rungs":["R1","R2"],
            "failure_models":{"modelled":["throws","stop-start"],"not_applicable":["two-versions"],"skipped":[{"model":"partial-partition","why":"single node"}]}});
        let md = audit_markdown(Path::new("/repo"), "abc", Depth::Medium, &p, &[("a".into(), r)], &[], 1.0);
        assert!(md.contains("## Failure models"));
        assert!(md.contains("| a | R1, R2 | throws, stop-start | partial-partition | two-versions |"));
        let plain = audit_markdown(Path::new("/repo"), "abc", Depth::Medium, &p, &[("b".into(), serde_json::json!({"area":"b","status":"verified"}))], &[], 1.0);
        assert!(!plain.contains("## Failure models"));
    }

    #[test]
    fn audit_markdown_orders_confirmed_before_leads() {
        let p = Profile { agent: Agent::Claude, name: "default".into(), config_dir: "/x".into(), program: "claude".into(), wrapper: false };
        let r = serde_json::json!({"area":"a","status":"bugs-found","verified":[{"property":"P","scope":"2 nodes","result":"pass"}],
            "findings":[{"title":"lead one","classification":"model-only","severity":"low"},{"title":"real one","classification":"confirmed","severity":"high"}]});
        let md = audit_markdown(Path::new("/repo"), "abc", Depth::Medium, &p, &[("a".into(), r)], &[("a".into(), serde_json::json!({"title":"T"}))], 10.0);
        assert!(md.find("real one").unwrap() < md.find("lead one").unwrap());
        assert!(md.contains("Confirmed bugs: **1**"));
        assert!(md.contains("| a | `P` | 2 nodes | pass |"));
    }
}
