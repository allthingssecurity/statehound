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
    // go
    ("go", "threads", "go func", 3), ("go", "locking", "sync.Mutex", 3), ("go", "locking", "sync.RWMutex", 3),
    ("go", "locking", ".Lock()", 2), ("go", "channels", "chan ", 2), ("go", "channels", "select {", 2), ("go", "atomics", "atomic.", 2),
    ("go", "threads", "sync.WaitGroup", 2), ("go", "transactions", ".BeginTx(", 3), ("go", "transactions", ".Begin(", 2),
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
];

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
    pub score: u32,
    pub files: usize,
    pub signals: BTreeMap<String, u32>,
    pub top_files: Vec<FileScore>,
}

fn is_test_path(rel: &str) -> bool {
    let l = rel.to_ascii_lowercase();
    l.contains("/test/") || l.contains("/tests/") || l.starts_with("test/") || l.starts_with("tests/") || l.contains("_test.")
        || l.contains(".test.") || l.contains(".spec.") || l.ends_with("test.java") || l.ends_with("tests.java") || l.contains("/testing/")
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
        }
    }
    (total, cats)
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
        d.top_files.truncate(8);
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
            "  {{\"dir\":\"{}\",\"language\":\"{}\",\"score\":{},\"files\":{},\"signals\":{{{}}},\"top_files\":[{}]}}{}\n",
            json_escape(&d.dir),
            d.language,
            d.score,
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
