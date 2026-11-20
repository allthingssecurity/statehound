//! Reading a TLC command line (`tlc2.TLC` arguments) well enough to route it.

/// The TLC option that everystate does not implement (the run is refused).
pub type Unsupported = &'static str;

#[derive(Debug, Default, Clone)]
pub struct Invocation {
    /// The arguments as given (TLC arguments; wrapper options removed).
    pub tlc_args: Vec<String>,
    pub spec: Option<String>,
    pub config: Option<String>,
    pub workers: Option<String>,
    /// `-simulate` was given.
    pub simulate: bool,
    /// `-seed` was given (a seeded simulation is reproducible, so it may be cached).
    pub seeded: bool,
    /// `-deadlock` was given: deadlock is NOT checked (TLC's meaning of the flag).
    pub no_deadlock: bool,
    /// `-tool` message framing.
    pub tool: bool,
    /// The run writes files besides its output (`-dump`, `simulate file=`, `-userFile`, ...): not cached.
    pub writes_files: bool,
    /// Set when some argument is a TLC option that everystate does not implement.
    pub unsupported: Option<Unsupported>,
    /// The run must be decided by the exhaustive search alone (`-dumpTrace` and `-loadTrace` write or read
    /// files that a hunter would not produce or use the same way).
    pub no_hunters: bool,
    /// `--cdot`: action composition (everystate's `-cdot`; TLC's `-Dtlc2.tool.impl.Tool.cdot=true`).
    pub cdot: bool,
    /// Library directories given with `-I`, hashed into the cache key.
    pub include_dirs: Vec<String>,
}

/// TLC flags that take one value.
const ONE_VALUE: &[&str] = &[
    "-config", "-workers", "-metadir", "-depth", "-seed", "-aril", "-fp", "-fpbits", "-fpmem", "-checkpoint",
    "-maxSetSize", "-lncheck", "-postCondition", "-teSpecOutDir", "-I", "-invlevel", "-inv",
];
/// TLC flags without a value that everystate handles (or accepts as no-ops) with TLC's meaning.
const NO_VALUE_OK: &[&str] = &["-deadlock", "-tool", "-cleanup", "-nowarning", "-noTE", "-terse", "-gzip", "-messagesAsErrors", "-suppressMessages", "-cdot"];

/// Splits wrapper options (`--name`) from the TLC arguments and classifies the TLC arguments.
pub fn parse(argv: &[String]) -> Invocation {
    let mut inv = Invocation::default();
    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        match a {
            "--cdot" => {
                inv.cdot = true;
                i += 1;
                continue;
            }
            _ if a.starts_with("--") => {
                // unknown wrapper option: keep it from reaching either engine, but be safe
                inv.unsupported.get_or_insert("unknown wrapper option");
                i += 1;
                continue;
            }
            _ => {}
        }
        inv.tlc_args.push(a.to_string());
        let lower = a.to_ascii_lowercase();
        if ONE_VALUE.contains(&a) {
            let v = argv.get(i + 1).cloned();
            if let Some(v) = &v {
                inv.tlc_args.push(v.clone());
            }
            match a {
                "-config" => inv.config = v,
                "-workers" => inv.workers = v,
                "-seed" => inv.seeded = true,
                "-I" => inv.include_dirs.extend(v),
                "-inv" | "-invlevel" => {
                    inv.unsupported.get_or_insert("-inv/-invlevel");
                }
                _ => {}
            }
            i += 2;
            continue;
        }
        if NO_VALUE_OK.contains(&a) {
            match a {
                "-deadlock" => inv.no_deadlock = true,
                "-tool" => inv.tool = true,
                "-cdot" => inv.cdot = true,
                _ => {}
            }
            i += 1;
            continue;
        }
        match lower.as_str() {
            "-simulate" => {
                inv.simulate = true;
                if let Some(v) = argv.get(i + 1) {
                    if v.contains('=') && !v.starts_with('-') {
                        if v.contains("file=") {
                            inv.writes_files = true;
                        }
                        inv.tlc_args.push(v.clone());
                        i += 1;
                    }
                }
                i += 1;
                continue;
            }
            "-dump" => {
                inv.writes_files = true;
                // -dump [dot[,actionlabels,...]] FILE
                if let Some(v) = argv.get(i + 1) {
                    inv.tlc_args.push(v.clone());
                    i += 1;
                    if v.starts_with("dot") {
                        if let Some(f) = argv.get(i + 1) {
                            inv.tlc_args.push(f.clone());
                            i += 1;
                        }
                    }
                }
                i += 1;
                continue;
            }
            // FORMAT FILE: everystate implements json and tla (dump) and json (load); it rejects the others itself
            "-dumptrace" | "-loadtrace" => {
                inv.writes_files = true;
                inv.no_hunters = true;
                for k in 1..=2 {
                    if let Some(v) = argv.get(i + k) {
                        inv.tlc_args.push(v.clone());
                    }
                }
                i += 3;
                continue;
            }
            "-continue" => inv.unsupported = inv.unsupported.or(Some("-continue")),
            "-debugger" => {
                // interactive: decided by the exhaustive search alone, never cached
                inv.no_hunters = true;
                inv.writes_files = true;
                if let Some(v) = argv.get(i + 1) {
                    if !v.starts_with('-') && !v.ends_with(".tla") && v.contains('=') {
                        inv.tlc_args.push(v.clone());
                        i += 1;
                    }
                }
            }
            "-generate" | "-difftrace" | "-recover" | "-coverage" | "-userfile" | "-view" => {
                inv.unsupported.get_or_insert("this TLC-specific option");
                if matches!(lower.as_str(), "-recover" | "-coverage" | "-userfile") {
                    if let Some(v) = argv.get(i + 1) {
                        inv.tlc_args.push(v.clone());
                        i += 1;
                    }
                }
            }
            _ if a.starts_with('-') => {
                inv.unsupported.get_or_insert("unrecognised option");
            }
            _ => inv.spec = Some(a.to_string()),
        }
        i += 1;
    }
    inv
}

#[cfg(test)]
mod tests {
    use super::*;
    fn v(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn plain_model_check() {
        let i = parse(&v("-config MC.cfg MC.tla -workers 4 -deadlock -metadir /tmp/x"));
        assert_eq!(i.spec.as_deref(), Some("MC.tla"));
        assert_eq!(i.config.as_deref(), Some("MC.cfg"));
        assert_eq!(i.workers.as_deref(), Some("4"));
        assert!(i.no_deadlock && !i.simulate && i.unsupported.is_none() && !i.writes_files);
        assert_eq!(i.tlc_args, v("-config MC.cfg MC.tla -workers 4 -deadlock -metadir /tmp/x"));
    }

    #[test]
    fn tlc_only_features_are_routed() {
        let d = parse(&v("-config a.cfg a.tla -dumpTrace json t.json"));
        assert!(d.unsupported.is_none() && d.no_hunters && d.writes_files);
        assert_eq!(d.tlc_args, v("-config a.cfg a.tla -dumpTrace json t.json"));
        assert!(parse(&v("-config a.cfg a.tla -dumptrace json t.json")).unsupported.is_none());
        assert!(parse(&v("-config a.cfg a.tla -continue")).unsupported.is_some());
        let g = parse(&v("-config a.cfg a.tla -debugger nosuspend,port=4712"));
        assert!(g.unsupported.is_none() && g.no_hunters && g.writes_files);
        assert_eq!(g.tlc_args, v("-config a.cfg a.tla -debugger nosuspend,port=4712"));
        assert!(parse(&v("-config a.cfg a.tla -loadTrace json t.json")).no_hunters);
        assert!(parse(&v("-config a.cfg a.tla -frobnicate")).unsupported.is_some());
    }

    #[test]
    fn simulation_and_wrapper_options() {
        let i = parse(&v("--cdot -config a.cfg a.tla -simulate num=5,file=traces/t -depth 30 -seed 7"));
        assert!(i.cdot && i.simulate && i.seeded && i.writes_files);
        assert_eq!(i.spec.as_deref(), Some("a.tla"));
        assert_eq!(i.tlc_args, v("-config a.cfg a.tla -simulate num=5,file=traces/t -depth 30 -seed 7"));
    }
}
