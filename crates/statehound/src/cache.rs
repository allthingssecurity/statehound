//! Content-addressed result cache. A key covers everything that can change a verdict: the checker
//! version, the arguments (with paths replaced by contents), every `.tla` file in the spec directory and
//! in `-I` library directories, and the configuration file. Any change gives a different key, so a hit
//! can only replay the result of an identical check.
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct Cache {
    dir: PathBuf,
}

pub struct Entry {
    pub code: i32,
    pub output: Vec<u8>,
}

/// Arguments that do not change the result of a check: their values are left out of the key.
const NEUTRAL_WITH_VALUE: &[&str] = &["-metadir", "-workers", "-checkpoint", "-fp", "-fpbits", "-fpmem"];
const NEUTRAL: &[&str] = &["-cleanup", "-nowarning", "-gzip"];

fn hash_dir_tla(h: &mut Sha256, dir: &Path) -> std::io::Result<()> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map_or(false, |x| x == "tla") && p.is_file())
        .collect();
    files.sort();
    for f in files {
        h.update(f.file_name().unwrap().to_string_lossy().as_bytes());
        h.update([0]);
        let data = fs::read(&f)?;
        h.update((data.len() as u64).to_le_bytes());
        h.update(&data);
    }
    Ok(())
}

impl Cache {
    pub fn open(dir: PathBuf) -> Option<Cache> {
        fs::create_dir_all(&dir).ok()?;
        Some(Cache { dir })
    }

    /// The key of a check, or `None` when some input cannot be read (then nothing is cached).
    pub fn key(engine: &str, cwd: &Path, args: &[String], spec_dir: &Path, config: &Path, include_dirs: &[String]) -> Option<String> {
        let mut h = Sha256::new();
        h.update(b"statehound cache v1\0");
        h.update(engine.as_bytes());
        h.update([0]);
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            if NEUTRAL_WITH_VALUE.contains(&a.as_str()) {
                i += 2;
                continue;
            }
            if NEUTRAL.contains(&a.as_str()) {
                i += 1;
                continue;
            }
            if a == "-config" {
                i += 2; // the configuration is hashed by content below
                continue;
            }
            h.update(a.as_bytes());
            h.update([0]);
            i += 1;
        }
        h.update(b"\0config\0");
        h.update(fs::read(config).ok()?);
        h.update(b"\0specdir\0");
        hash_dir_tla(&mut h, spec_dir).ok()?;
        for d in include_dirs {
            h.update(b"\0lib\0");
            hash_dir_tla(&mut h, &cwd.join(d)).ok()?;
        }
        Some(format!("{:x}", h.finalize()))
    }

    pub fn get(&self, key: &str) -> Option<Entry> {
        let p = self.dir.join(key);
        let code: i32 = fs::read_to_string(p.join("code")).ok()?.trim().parse().ok()?;
        let output = fs::read(p.join("output")).ok()?;
        Some(Entry { code, output })
    }

    /// Stores a result atomically (written to a temporary directory, then renamed).
    pub fn put(&self, key: &str, code: i32, output: &[u8]) {
        let tmp = self.dir.join(format!(".tmp-{}-{}", key, std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let ok = (|| -> std::io::Result<()> {
            fs::create_dir_all(&tmp)?;
            fs::File::create(tmp.join("output"))?.write_all(output)?;
            fs::write(tmp.join("code"), code.to_string())?;
            let dest = self.dir.join(key);
            if dest.exists() {
                fs::remove_dir_all(&tmp)?;
                return Ok(());
            }
            fs::rename(&tmp, &dest)
        })();
        if ok.is_err() {
            let _ = fs::remove_dir_all(&tmp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let spec = d.path().join("spec");
        fs::create_dir(&spec).unwrap();
        fs::write(spec.join("MC.tla"), "---- MODULE MC ----\n====\n").unwrap();
        fs::write(spec.join("MC.cfg"), "INIT Init\n").unwrap();
        let cfg = spec.join("MC.cfg");
        (d, spec, cfg)
    }
    fn a(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn key_ignores_neutral_args_but_not_contents() {
        let (_d, spec, cfg) = setup();
        let k1 = Cache::key("es 1", &spec, &a("-config MC.cfg MC.tla -workers 4 -metadir /tmp/a"), &spec, &cfg, &[]).unwrap();
        let k2 = Cache::key("es 1", &spec, &a("-config MC.cfg MC.tla -workers 8 -metadir /tmp/b"), &spec, &cfg, &[]).unwrap();
        assert_eq!(k1, k2);
        let k3 = Cache::key("es 1", &spec, &a("-config MC.cfg MC.tla -deadlock"), &spec, &cfg, &[]).unwrap();
        assert_ne!(k1, k3);
        let k4 = Cache::key("es 2", &spec, &a("-config MC.cfg MC.tla -workers 4"), &spec, &cfg, &[]).unwrap();
        assert_ne!(k1, k4);
        fs::write(spec.join("Other.tla"), "---- MODULE Other ----\n====\n").unwrap();
        let k5 = Cache::key("es 1", &spec, &a("-config MC.cfg MC.tla -workers 4"), &spec, &cfg, &[]).unwrap();
        assert_ne!(k1, k5, "a new module in the spec directory changes the key");
        fs::write(&cfg, "INIT Init\nNEXT Next\n").unwrap();
        let k6 = Cache::key("es 1", &spec, &a("-config MC.cfg MC.tla -workers 4"), &spec, &cfg, &[]).unwrap();
        assert_ne!(k5, k6, "a changed configuration changes the key");
    }

    #[test]
    fn put_then_get() {
        let d = tempfile::tempdir().unwrap();
        let c = Cache::open(d.path().join("c")).unwrap();
        assert!(c.get("k").is_none());
        c.put("k", 12, b"Error: Invariant X is violated.\n");
        let e = c.get("k").unwrap();
        assert_eq!(e.code, 12);
        assert_eq!(e.output, b"Error: Invariant X is violated.\n");
    }
}
