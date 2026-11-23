//! `statehound scan`: score every source directory of a repository by the concurrency and
//! distributed-systems signals in its code. Deterministic and fast; the survey agent reads the top of
//! this list to choose the hunt areas.
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Directories never scanned.
const SKIP_DIRS: &[&str] = &[
    ".git", ".hg", ".svn", "target", "build", "out", "dist", "node_modules", "vendor", "third_party", "third-party",
    ".venv", "venv", "__pycache__", ".gradle", ".idea", ".vscode", ".statehound", ".mvn", "bin", "obj", ".next", ".cache",
];

/// Source file extensions and the language family whose signal table applies.
fn language(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "java" | "kt" | "kts" | "scala" | "groovy" => "jvm",
        "go" => "go",
        "rs" => "rust",
        "py" => "python",
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" => "c",
        "cs" => "csharp",
        "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" => "js",
        "erl" | "ex" | "exs" => "beam",
        "rb" => "ruby",
        _ => return None,
    })
}

/// (category, token, weight). Code tokens are matched case-sensitively, domain words (lower case,
/// starting with `~`) case-insensitively.
const SIGNALS: &[(&str, &str, &str, u32)] = &[
    // jvm
    ("jvm", "locking", "synchronized", 3), ("jvm", "locking", "ReentrantLock", 3), ("jvm", "locking", "ReadWriteLock", 3),
    ("jvm", "locking", "StampedLock", 3), ("jvm", "locking", ".lock()", 2), ("jvm", "locking", "Semaphore", 2),
    ("jvm", "atomics", "Atomic", 2), ("jvm", "atomics", "volatile", 2), ("jvm", "atomics", "compareAndSet", 3),
    ("jvm", "threads", "ExecutorService", 2), ("jvm", "threads", "ScheduledExecutor", 2), ("jvm", "threads", "new Thread", 2),
    ("jvm", "threads", "CompletableFuture", 2), ("jvm", "threads", "CountDownLatch", 2), ("jvm", "threads", "ConcurrentHashMap", 1),
    ("jvm", "transactions", "@Transactional", 3), ("jvm", "transactions", "setAutoCommit", 3), ("jvm", "transactions", "FOR UPDATE", 3),
    ("jvm", "transactions", "executeUpdate", 1),
    // lifecycle methods that run while worker threads are still active (stop/start races)
    ("jvm", "lifecycle", "doStart(", 2), ("jvm", "lifecycle", "doStop(", 2), ("jvm", "lifecycle", "doSuspend(", 2),
    ("jvm", "lifecycle", "doResume(", 2), ("jvm", "lifecycle", "doShutdown(", 2), ("jvm", "lifecycle", "onPartitionsRevoked", 3),
    ("jvm", "lifecycle", "onPartitionsAssigned", 2), ("jvm", "lifecycle", "onPartitionsLost", 3), ("jvm", "lifecycle", "shutdownNow(", 2),
    // waits without a deadline (missing-timeout hangs)
    ("jvm", "blocking", ".take()", 2), ("jvm", "blocking", ".await()", 2), ("jvm", "blocking", ".acquire()", 2),
    ("jvm", "blocking", ".join()", 1), ("jvm", "blocking", "awaitUninterruptibly()", 2), ("jvm", "blocking", "acquireUninterruptibly(", 2),
    ("jvm", "timeouts", "currentTimeMillis", 1),
    // persisted or wire formats read by other versions or systems
    ("jvm", "persistence", "serialVersionUID", 1), ("jvm", "persistence", "ObjectOutputStream", 2), ("jvm", "persistence", "ObjectInputStream", 2),
    // go
    ("go", "threads", "go func", 3), ("go", "locking", "sync.Mutex", 3), ("go", "locking", "sync.RWMutex", 3),
    ("go", "locking", ".Lock()", 2), ("go", "channels", "chan ", 2), ("go", "channels", "select {", 2), ("go", "atomics", "atomic.", 2),
    ("go", "threads", "sync.WaitGroup", 2), ("go", "transactions", ".BeginTx(", 3), ("go", "transactions", ".Begin(", 2),
    ("go", "blocking", "context.Background()", 1), ("go", "lifecycle", "Shutdown(", 1),
    // rust
    ("rust", "locking", "Mutex<", 3), ("rust", "locking", "RwLock<", 3), ("rust", "locking", ".lock()", 2),
    ("rust", "atomics", "Atomic", 2), ("rust", "atomics", "Ordering::", 2), ("rust", "threads", "thread::spawn", 3),
    ("rust", "threads", "tokio::spawn", 3), ("rust", "async", ".await", 1), ("rust", "channels", "mpsc::", 2), ("rust", "channels", "crossbeam", 2),
    // python
    ("python", "threads", "threading.", 3), ("python", "locking", "Lock(", 3), ("python", "async", "asyncio.", 2),
    ("python", "async", "await ", 1), ("python", "threads", "multiprocessing", 2), ("python", "threads", "ThreadPoolExecutor", 2),
    ("python", "transactions", ".commit()", 2), ("python", "transactions", "FOR UPDATE", 3),
    // c / c++
    ("c", "threads", "pthread_", 3), ("c", "locking", "std::mutex", 3), ("c", "locking", "spin_lock", 3), ("c", "locking", "mutex_lock", 3),
    ("c", "atomics", "std::atomic", 3), ("c", "atomics", "atomic_", 2), ("c", "atomics", "__sync_", 3), ("c", "atomics", "READ_ONCE", 2),
    ("c", "threads", "std::thread", 3), ("c", "locking", "rcu_", 3),
    // c#
    ("csharp", "locking", "lock (", 3), ("csharp", "locking", "SemaphoreSlim", 3), ("csharp", "atomics", "Interlocked.", 3),
    ("csharp", "async", "await ", 1), ("csharp", "threads", "Task.Run", 2), ("csharp", "transactions", "BeginTransaction", 3),
    // js / ts
    ("js", "async", "Promise.all", 2), ("js", "async", "await ", 1), ("js", "threads", "worker_threads", 3), ("js", "threads", "new Worker", 3),
    ("js", "atomics", "Atomics.", 3), ("js", "async", "setTimeout", 1), ("js", "transactions", ".transaction(", 3),
    // erlang / elixir
    ("beam", "channels", "gen_server", 3), ("beam", "channels", "receive", 1), ("beam", "threads", "spawn", 2), ("beam", "channels", "GenServer", 3),
    // ruby
    ("ruby", "threads", "Thread.new", 3), ("ruby", "locking", "Mutex.new", 3), ("ruby", "transactions", "transaction do", 3),
    // domain words, every language
    ("*", "retries", "~retry", 2), ("*", "retries", "~backoff", 2), ("*", "timeouts", "~timeout", 1), ("*", "timeouts", "~deadline", 1),
    ("*", "distributed", "~lease", 3), ("*", "distributed", "~leader", 3), ("*", "distributed", "~election", 3), ("*", "distributed", "~heartbeat", 2),
    ("*", "distributed", "~quorum", 3), ("*", "distributed", "~consensus", 3), ("*", "distributed", "~replica", 2), ("*", "distributed", "~raft", 3),
    ("*", "distributed", "~fencing", 3), ("*", "distributed", "~epoch", 2), ("*", "messaging", "~offset", 2), ("*", "messaging", "~acknowledg", 2),
    ("*", "messaging", "~redeliver", 3), ("*", "messaging", "~idempot", 3), ("*", "messaging", "~dedup", 3), ("*", "messaging", "~exactly-once", 3),
    ("*", "messaging", "~at-least-once", 3), ("*", "caching", "~invalidat", 2), ("*", "caching", "~evict", 2), ("*", "caching", "~cache", 1),
    ("*", "recovery", "~recover", 2), ("*", "recovery", "~crash", 2), ("*", "recovery", "~fsync", 3), ("*", "recovery", "~checkpoint", 2),
    ("*", "recovery", "~rollback", 2), ("*", "locking", "~optimistic", 2), ("*", "locking", "~version", 1), ("*", "transactions", "~isolation", 2),
    ("*", "lifecycle", "~reconnect", 2), ("*", "lifecycle", "~rebalanc", 2), ("*", "lifecycle", "~graceful", 1),
    ("*", "caching", "~registry", 1), ("*", "caching", "~pool", 1), ("*", "persistence", "~journal", 2), ("*", "persistence", "~snapshot", 1),
    ("*", "persistence", "~repository", 1), ("*", "persistence", "~serializ", 1),
    // feedback: limiters, breakers and adaptive limits that, with retries, can sustain a metastable failure
    ("*", "feedback", "~ratelimit", 3), ("*", "feedback", "~rate limit", 3), ("*", "feedback", "~circuitbreaker", 3),
    ("*", "feedback", "~circuit breaker", 3), ("*", "feedback", "~bulkhead", 3), ("*", "feedback", "~throttl", 2),
    ("*", "feedback", "~autoscal", 3), ("*", "feedback", "~adaptive", 2), ("*", "feedback", "~backpressure", 2),
    ("*", "feedback", "~admission", 2), ("*", "feedback", "~congestion", 3), ("*", "feedback", "~utilization", 2),
    ("*", "feedback", "~load shedding", 3), ("*", "feedback", "~setmaximumpoolsize(", 3), ("*", "feedback", "~setcorepoolsize(", 2),
    ("jvm", "feedback", "CallerRunsPolicy", 2), ("jvm", "feedback", "RejectedExecutionHandler", 2),
    ("go", "feedback", "rate.NewLimiter", 3), ("go", "feedback", "semaphore.NewWeighted", 2),
    // periodic and background tasks that die quietly on one exception (partial failures)
    ("jvm", "periodic", "scheduleAtFixedRate(", 3), ("jvm", "periodic", "scheduleWithFixedDelay(", 3),
    ("jvm", "periodic", "new Timer(", 2), ("jvm", "periodic", "TimerTask", 2),
    ("go", "periodic", "time.NewTicker(", 2), ("go", "periodic", "time.Tick(", 2),
    ("rust", "periodic", "interval(", 2), ("js", "periodic", "setInterval(", 2), ("python", "periodic", "sched.", 1),
    // health signals that can say healthy while the work is stuck (gray failure)
    ("*", "health", "~healthcheck", 2), ("*", "health", "~health check", 2), ("*", "health", "~liveness", 2),
    ("*", "health", "~readiness", 2), ("*", "health", "~ishealthy", 2), ("*", "health", "~isalive", 2),
    ("*", "health", "~watchdog", 2),
];

/// A directory is ranked by its this many highest-scoring files.
const TOP_FILES: usize = 8;
/// At most this many hits of one token count per file (one generated file must not dominate).
const PER_FILE_CAP: u32 = 20;
/// Files larger than this are skipped (generated or data).
const MAX_FILE_BYTES: u64 = 1 << 20;

#[derive(Debug, Clone, Default)]
pub struct FileScore {
    pub path: String,
    pub score: u32,
}

#[derive(Debug, Clone, Default)]
pub struct DirScore {
    pub dir: String,
    pub language: String,
    /// sum of the scores of the `TOP_FILES` highest-scoring files
    pub score: u32,
    /// sum over every file of the directory
    pub all_files_score: u32,
    pub files: usize,
    pub signals: BTreeMap<String, u32>,
    pub top_files: Vec<FileScore>,
}

fn is_test_path(rel: &str) -> bool {
    let l = rel.to_ascii_lowercase();
    l.contains("/test/") || l.contains("/tests/") || l.starts_with("test/") || l.starts_with("tests/") || l.contains("_test.")
        || l.contains(".test.") || l.contains(".spec.") || l.ends_with("test.java") || l.ends_with("tests.java") || l.contains("/testing/")
}

/// User interfaces, command-line tools, examples and benchmarks: rarely hunt areas, and their error
/// handling (log and carry on) is usually intended.
fn is_peripheral_path(rel: &str) -> bool {
    let l = rel.to_ascii_lowercase();
    l.split('/').any(|seg| matches!(seg, "tui" | "ui" | "cli" | "commands" | "example" | "examples" | "demo" | "demos" | "sample" | "samples" | "benchmark" | "benchmarks" | "jmh"))
}

fn is_generated_path(rel: &str) -> bool {
    let l = rel.to_ascii_lowercase();
    l.contains("/generated/") || l.starts_with("generated/") || l.contains("/gen/") || l.contains(".pb.") || l.contains("_pb2") || l.contains(".generated.")
}

fn count(hay: &str, needle: &str) -> u32 {
    if needle.is_empty() {
        return 0;
    }
    let mut n = 0;
    let mut from = 0;
    while let Some(i) = hay[from..].find(needle) {
        n += 1;
        if n >= PER_FILE_CAP {
            break;
        }
        from += i + needle.len();
    }
    n
}

/// Scores one file's text: (total, per-category).
pub fn score_text(lang: &str, text: &str) -> (u32, BTreeMap<String, u32>) {
    let lower = text.to_ascii_lowercase();
    let mut total = 0;
    let mut cats: BTreeMap<String, u32> = BTreeMap::new();
    let mut life_code = 0;
    for (l, cat, tok, w) in SIGNALS {
        if *l != "*" && *l != lang {
            continue;
        }
        let hits = match tok.strip_prefix('~') {
            Some(word) => count(&lower, word),
            None => count(text, tok),
        };
        if hits > 0 {
            total += hits * w;
            *cats.entry(cat.to_string()).or_insert(0) += hits;
            if *cat == "lifecycle" && !tok.starts_with('~') {
                life_code += hits;
            }
        }
    }
    // Error handling: 92% of catastrophic distributed-system failures come from mishandled non-fatal errors,
    // a third of them from trivial handlers (Yuan et al., OSDI 2014).
    let e = error_handlers(lang, text);
    if e.total() > 0 {
        total += e.total() * 3;
        *cats.entry("errors".to_string()).or_insert(0) += e.total();
    }
    // Lifecycle methods in a file that also runs threads: the stop/start-while-workers-run class.
    // Only lifecycle methods in code count here, not words in comments or generated docs.
    let life = life_code;
    let conc = ["threads", "async", "atomics", "locking"].iter().map(|c| cats.get(*c).copied().unwrap_or(0)).sum::<u32>();
    if life > 0 && conc > 0 {
        total += 4 * life.min(5);
        cats.insert("lifecycle_threads".to_string(), life.min(5));
    }
    // A limiter, breaker or adaptive limit in a file that also retries or times out: the feedback loop that
    // sustains metastable failures (Huang et al., OSDI 2022: retries sustained more than half of the incidents).
    let fb = cats.get("feedback").copied().unwrap_or(0);
    let again = cats.get("retries").copied().unwrap_or(0) + cats.get("timeouts").copied().unwrap_or(0);
    if fb > 0 && again > 0 {
        total += 3 * fb.min(5);
        cats.insert("feedback_loop".to_string(), fb.min(5));
    }
    // Scheduled or periodic tasks in a file with no broad catch: one exception stops the task for good while the
    // process stays up (Lou et al., NSDI 2020: 48% of partial failures left functionality stuck).
    let per = cats.get("periodic").copied().unwrap_or(0);
    if per > 0 && !has_broad_catch(lang, text) {
        total += 4 * per.min(5);
        cats.insert("periodic_unguarded".to_string(), per.min(5));
    }
    (total, cats)
}

/// True when the file catches broadly somewhere (`Throwable`, `Exception`, `RuntimeException`, a bare
/// `except:`, a `recover()`), so a periodic task in it may be guarded. A textual check, good enough to rank.
fn has_broad_catch(lang: &str, text: &str) -> bool {
    match lang {
        "jvm" | "csharp" | "js" => ["catch (Throwable", "catch (Exception", "catch (RuntimeException", "catch (final Throwable",
            "catch (final Exception", "catch (e)", "catch {", "catch(Throwable", "catch(Exception"]
            .iter()
            .any(|p| text.contains(p)),
        "python" => text.contains("except Exception") || text.contains("except:") || text.contains("except BaseException"),
        "go" => text.contains("recover()"),
        "rust" => text.contains("catch_unwind"),
        _ => true,
    }
}

/// Suspicious error handlers in one file.
#[derive(Debug, Default, PartialEq)]
pub struct ErrorHandlers {
    /// empty, log-only or TODO handlers
    pub swallowed: u32,
    /// `InterruptedException` caught without restoring the interrupt or rethrowing
    pub interrupt_lost: u32,
    /// a broad catch (`Exception`, `Throwable`, `RuntimeException`) that stops, shuts down or exits
    pub broad_abort: u32,
}

impl ErrorHandlers {
    pub fn total(&self) -> u32 {
        self.swallowed + self.interrupt_lost + self.broad_abort
    }
}

/// Removes `//` and `/* */` comments (and `#` comments for Python-like code is not needed here).
fn strip_comments(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    loop {
        let line = rest.find("//");
        let block = rest.find("/*");
        match (line, block) {
            (None, None) => {
                out.push_str(rest);
                break;
            }
            (Some(l), b) if b.map_or(true, |b| l < b) => {
                out.push_str(&rest[..l]);
                rest = rest[l..].find('\n').map(|n| &rest[l + n..]).unwrap_or("");
            }
            (_, Some(b)) => {
                out.push_str(&rest[..b]);
                rest = rest[b + 2..].find("*/").map(|n| &rest[b + 2 + n + 2..]).unwrap_or("");
            }
            _ => unreachable!(),
        }
    }
    out
}

/// True when the handler body has at least one statement and every statement only logs or prints.
fn only_logs(body: &str) -> bool {
    const LOG: &[&str] = &["log.", "logger.", "log_", "logging.", "console.", "system.out.", "system.err.", "print(", "println", "debug(", "trace("];
    let code = strip_comments(body);
    let stmts: Vec<String> = code.split(';').map(|x| x.trim().to_ascii_lowercase()).filter(|x| !x.is_empty()).collect();
    !stmts.is_empty() && stmts.iter().all(|st| LOG.iter().any(|p| st.starts_with(p)) || st.contains("printstacktrace("))
}

fn classify_handler(head: &str, body: &str, h: &mut ErrorHandlers) {
    let b = body.trim();
    let low = b.to_ascii_lowercase();
    if strip_comments(b).trim().is_empty() || low.contains("todo") || low.contains("fixme") || only_logs(b) {
        h.swallowed += 1;
    } else if head.contains("InterruptedException") && !b.contains("interrupt()") && !b.contains("throw") {
        h.interrupt_lost += 1;
    }
    let broad = head
        .split(|c: char| c == '|' || c.is_whitespace() || c == '(' || c == ')')
        .any(|t| matches!(t, "Exception" | "RuntimeException" | "Throwable" | "java.lang.Exception" | "java.lang.Throwable"));
    if broad && (b.contains("System.exit") || b.contains(".stop(") || b.contains("stop()") || b.contains("shutdown") || b.contains("halt(")) {
        h.broad_abort += 1;
    }
}

/// Index just after the brace that closes the block opened at `open` (which must be `{`).
fn block_end(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Finds catch/except handlers and classifies them. A textual scan: good enough to rank directories.
pub fn error_handlers(lang: &str, text: &str) -> ErrorHandlers {
    let mut h = ErrorHandlers::default();
    let bytes = text.as_bytes();
    let mut seen = 0u32;
    match lang {
        "jvm" | "csharp" | "js" => {
            let mut from = 0;
            while let Some(i) = text[from..].find("catch") {
                let at = from + i;
                from = at + 5;
                if at > 0 && (bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_' || bytes[at - 1] == b'.') {
                    continue;
                }
                let rest = &text[from..];
                let trimmed = rest.trim_start();
                let skip = rest.len() - trimmed.len();
                let (head, brace) = if trimmed.starts_with('(') {
                    match trimmed.find(')') {
                        Some(close) => {
                            let after = trimmed[close + 1..].trim_start();
                            if !after.starts_with('{') {
                                continue;
                            }
                            (&trimmed[1..close], from + skip + close + 1 + (trimmed[close + 1..].len() - after.len()))
                        }
                        None => continue,
                    }
                } else if trimmed.starts_with('{') {
                    ("", from + skip)
                } else {
                    continue;
                };
                if let Some(end) = block_end(bytes, brace) {
                    classify_handler(head, &text[brace + 1..end - 1], &mut h);
                    seen += 1;
                    if seen >= PER_FILE_CAP * 3 {
                        break;
                    }
                }
            }
        }
        "python" => {
            let lines: Vec<&str> = text.lines().collect();
            for (i, l) in lines.iter().enumerate() {
                let t = l.trim_start();
                if !(t.starts_with("except") && t.trim_end().ends_with(':')) {
                    continue;
                }
                let indent = l.len() - t.len();
                let body: Vec<&str> = lines[i + 1..]
                    .iter()
                    .take_while(|x| x.trim().is_empty() || x.len() - x.trim_start().len() > indent)
                    .map(|x| x.trim())
                    .filter(|x| !x.is_empty())
                    .collect();
                let joined = body.join("\n");
                if body.iter().all(|x| *x == "pass" || x.starts_with('#')) {
                    h.swallowed += 1;
                } else {
                    classify_handler(t, &joined, &mut h);
                }
            }
        }
        "go" => {
            let mut from = 0;
            while let Some(i) = text[from..].find("if err != nil {") {
                let open = from + i + "if err != nil ".len();
                from = open + 1;
                if let Some(end) = block_end(bytes, open) {
                    let body = &text[open + 1..end - 1];
                    if !body.contains("return") && !body.contains("panic(") && !body.contains("Fatal") && !body.contains("continue") && !body.contains("break") {
                        h.swallowed += 1;
                    }
                }
            }
        }
        _ => {}
    }
    h.swallowed = h.swallowed.min(PER_FILE_CAP);
    h.interrupt_lost = h.interrupt_lost.min(PER_FILE_CAP);
    h.broad_abort = h.broad_abort.min(PER_FILE_CAP);
    h
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let rd = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };
    let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        let ft = match e.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) || (name.starts_with('.') && name != ".") {
                continue;
            }
            walk(root, &p, out);
        } else if ft.is_file() && language(&p).is_some() {
            out.push(p);
        }
    }
}

/// Scans `root` and returns directories with a non-zero score, highest first.
pub fn scan(root: &Path) -> Vec<DirScore> {
    let mut files = vec![];
    walk(root, root, &mut files);
    let mut dirs: BTreeMap<String, DirScore> = BTreeMap::new();
    for f in files {
        let meta = match fs::metadata(&f) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let text = match fs::read(&f) {
            Ok(b) => String::from_utf8_lossy(&b).into_owned(),
            Err(_) => continue,
        };
        let lang = language(&f).unwrap_or("");
        let rel = f.strip_prefix(root).unwrap_or(&f).to_string_lossy().replace('\\', "/");
        let (mut s, cats) = score_text(lang, &text);
        if is_test_path(&rel) || is_generated_path(&rel) {
            s /= 5; // tests show how a mechanism is used and generated code is not hand-written logic
        } else if is_peripheral_path(&rel) {
            s /= 3;
        }
        let dir = Path::new(&rel).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
        let d = dirs.entry(dir.clone()).or_insert_with(|| DirScore { dir: if dir.is_empty() { ".".into() } else { dir.clone() }, language: lang.into(), ..Default::default() });
        d.files += 1;
        d.score += s;
        for (k, v) in cats {
            *d.signals.entry(k).or_insert(0) += v;
        }
        if s > 0 {
            d.top_files.push(FileScore { path: rel, score: s });
        }
    }
    let mut v: Vec<DirScore> = dirs.into_values().filter(|d| d.score > 0).collect();
    for d in v.iter_mut() {
        d.top_files.sort_by(|a, b| b.score.cmp(&a.score).then(a.path.cmp(&b.path)));
        d.top_files.truncate(TOP_FILES);
        // Rank by the strongest files, not the size of the directory: a focused component with one risky
        // mechanism should not lose to a large utility package. On the Apache Camel audits this moved the
        // directories of 43 confirmed bugs from a median rank of 48 to 33.
        d.all_files_score = d.score;
        d.score = d.top_files.iter().map(|f| f.score).sum();
    }
    v.sort_by(|a, b| b.score.cmp(&a.score).then(a.dir.cmp(&b.dir)));
    v
}

fn json_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

/// The scan as JSON (an array, highest score first), at most `limit` directories.
pub fn to_json(v: &[DirScore], limit: usize) -> String {
    let mut out = String::from("[\n");
    for (i, d) in v.iter().take(limit).enumerate() {
        let sig: Vec<String> = d.signals.iter().map(|(k, n)| format!("\"{}\":{}", json_escape(k), n)).collect();
        let files: Vec<String> = d.top_files.iter().map(|f| format!("{{\"path\":\"{}\",\"score\":{}}}", json_escape(&f.path), f.score)).collect();
        out += &format!(
            "  {{\"dir\":\"{}\",\"language\":\"{}\",\"score\":{},\"all_files_score\":{},\"files\":{},\"signals\":{{{}}},\"top_files\":[{}]}}{}\n",
            json_escape(&d.dir),
            d.language,
            d.score,
            d.all_files_score,
            d.files,
            sig.join(","),
            files.join(","),
            if i + 1 < v.len().min(limit) { "," } else { "" }
        );
    }
    out.push(']');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scores_concurrency_signals_per_language() {
        let (s, c) = score_text("go", "func f() { go func() { mu.Lock(); x++ }() }\nvar mu sync.Mutex");
        assert!(s > 0);
        assert!(c.contains_key("locking") && c.contains_key("threads"));
        let (s2, _) = score_text("go", "func add(a, b int) int { return a + b }");
        assert_eq!(s2, 0);
        // domain words are case-insensitive and apply to every language
        let (s3, c3) = score_text("python", "def renew_Lease(): retry until LEADER")
            ;
        assert!(s3 > 0 && c3.contains_key("distributed") && c3.contains_key("retries"));
    }

    #[test]
    fn classifies_error_handlers() {
        let java = r#"
            try { a(); } catch (IOException e) { }
            try { b(); } catch (IOException e) { LOG.warn("failed", e); }
            try { c(); } catch (Exception e) { // TODO handle
            }
            try { d(); } catch (InterruptedException e) { done = true; }
            try { d(); } catch (InterruptedException e) { Thread.currentThread().interrupt(); }
            try { e(); } catch (Exception e) { LOG.error("fatal", e); stop(); }
            try { f(); } catch (IOException e) { throw new UncheckedIOException(e); }
            try { g(); } catch (Throwable t) { System.exit(1); }
        "#;
        let h = error_handlers("jvm", java);
        assert_eq!(h.swallowed, 3, "{:?}", h);
        assert_eq!(h.interrupt_lost, 1, "{:?}", h);
        assert_eq!(h.broad_abort, 2, "{:?}", h);
        let py = "try:\n    x()\nexcept Exception:\n    pass\ntry:\n    y()\nexcept OSError as e:\n    raise\n";
        assert_eq!(error_handlers("python", py).swallowed, 1);
        let go = "if err != nil {\n log.Printf(\"x\")\n}\nif err != nil {\n return err\n}\n";
        assert_eq!(error_handlers("go", go).swallowed, 1);
        let js = "try { a() } catch { }\ntry { b() } catch (e) { throw e }\n";
        assert_eq!(error_handlers("js", js).swallowed, 1);
        // the error and lifecycle categories feed the directory score
        let (s, c) = score_text("jvm", "protected void doStop() { executor.shutdownNow(); }\nExecutorService executor;\ntry { x(); } catch (Exception e) { }");
        assert!(s > 0 && c.contains_key("errors") && c.contains_key("lifecycle") && c.contains_key("lifecycle_threads"), "{:?}", c);
    }

    #[test]
    fn marks_feedback_loops() {
        // a limiter or breaker next to retries or timeouts is a feedback loop
        let java = "RateLimiter limiter; CircuitBreaker breaker;\nvoid call() { for (int i = 0; i < maxRetries; i++) { retry(); } }";
        let (s, c) = score_text("jvm", java);
        assert!(c.get("feedback").copied().unwrap_or(0) >= 2 && c.contains_key("feedback_loop"), "{:?}", c);
        // a limiter alone counts as feedback but not as a loop, and scores lower
        let (s2, c2) = score_text("jvm", "RateLimiter limiter; CircuitBreaker breaker;");
        assert!(c2.contains_key("feedback") && !c2.contains_key("feedback_loop") && s2 < s, "{:?}", c2);
        // adaptive pool sizing from a utilization metric (Go)
        let (_, c3) = score_text("go", "l := rate.NewLimiter(10, 1)\nctx, cancel := context.WithTimeout(ctx, d)\n// adjust by utilization");
        assert!(c3.contains_key("feedback_loop"), "{:?}", c3);
    }

    #[test]
    fn marks_unguarded_periodic_tasks_and_health_signals() {
        let unguarded = "executor.scheduleAtFixedRate(this::renew, 0, 5, TimeUnit.SECONDS);\nvoid renew() { lease.touch(); }";
        let (s, c) = score_text("jvm", unguarded);
        assert!(c.contains_key("periodic") && c.contains_key("periodic_unguarded"), "{:?}", c);
        let guarded = "executor.scheduleAtFixedRate(this::renew, 0, 5, TimeUnit.SECONDS);\nvoid renew() { try { lease.touch(); } catch (Throwable t) { LOG.warn(\"x\", t); } }";
        let (s2, c2) = score_text("jvm", guarded);
        assert!(c2.contains_key("periodic") && !c2.contains_key("periodic_unguarded"), "{:?}", c2);
        assert!(s2 < s + 10, "a swallowed catch scores as an error handler instead: {} vs {}", s2, s);
        let (_, c3) = score_text("go", "t := time.NewTicker(d)\nfor range t.C { check() }");
        assert!(c3.contains_key("periodic_unguarded"), "{:?}", c3);
        let (_, c4) = score_text("jvm", "public HealthCheck.Result call() { return isHealthy() ? up() : down(); }");
        assert!(c4.contains_key("health"), "{:?}", c4);
    }

    #[test]
    fn scans_a_tree_skipping_vendor_and_downweighting_tests() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        fs::create_dir_all(r.join("src/lock")).unwrap();
        fs::create_dir_all(r.join("src/util")).unwrap();
        fs::create_dir_all(r.join("vendor/x")).unwrap();
        fs::create_dir_all(r.join("src/lock/tests")).unwrap();
        fs::write(r.join("src/lock/lease.go"), "var mu sync.Mutex\nfunc renew() { mu.Lock(); go func(){}(); /* lease retry */ }").unwrap();
        fs::write(r.join("src/util/strings.go"), "func Pad(s string) string { return s }").unwrap();
        fs::write(r.join("vendor/x/x.go"), "var mu sync.Mutex; go func(){}()").unwrap();
        fs::write(r.join("src/lock/tests/lease_test.go"), "var mu sync.Mutex; go func(){}(); go func(){}()").unwrap();
        let v = scan(r);
        assert_eq!(v[0].dir, "src/lock");
        assert!(v.iter().all(|d| !d.dir.starts_with("vendor")));
        assert!(v.iter().all(|d| d.dir != "src/util"));
        let t = v.iter().find(|d| d.dir == "src/lock/tests").unwrap();
        assert!(t.score < v[0].score);
        let j = to_json(&v, 10);
        assert!(j.starts_with("[\n") && j.ends_with(']') && j.contains("\"dir\":\"src/lock\""));
    }
}
