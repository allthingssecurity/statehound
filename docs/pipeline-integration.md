# Using Statehound in a TLA+ bug-finding pipeline

Statehound was built for an agent-driven TLA+ bug-finding pipeline
([github.com/specular-org/Specular](https://github.com/specular-org/Specular)): agents read a codebase,
write TLA+ specs, model-check them, and reproduce violations against the real code. Out of the box that
pipeline runs Java TLC. [`patches/pipeline/`](../patches/pipeline) makes it run Statehound instead and
removes Java from its default path.

## Apply

```sh
git clone https://github.com/specular-org/Specular pipeline
git -C pipeline checkout 6079cd135122cd0d9d86b3fca5aa4002ae435844     # the commit the patch was made against
scripts/install.sh --pipeline pipeline        # builds statehound + everystate, applies the patch, installs into pipeline/bin
```

Then set the pipeline up and run it as its README describes. `STATEHOUND=off` switches the model checker
back to Java TLC for a run.

## What the patch changes

| Where | Before | After |
|---|---|---|
| `scripts/tlc/run_model_check.sh` (every model-checking run of the agents) | `java ... tlc2.TLC`, requires `lib/tla2tools.jar` | `bin/statehound` when installed; the jar is required only when Statehound is absent or `STATEHOUND=off` |
| `scripts/tlc/validate.sh` (trace validation) | Java TLC; downloads the jars | `everystate` when installed; no download |
| trace debugger tool: trace validation | Java TLC | `everystate` when installed |
| invariant-checking tool: trace replay (`-loadTrace json`, `-dumpTrace json`) | Java TLC | `everystate` when installed |
| trace debugger tool: debugger launch (`-debugger`, DAP) | Java TLC | `everystate` when installed |
| `scripts/infra/setup.sh` | requires Java, downloads jars, builds the Java CFA tool | skips the jars when `bin/statehound` and `bin/everystate` exist, and the CFA build when `bin/tla-cfa` exists |

The coverage script switches to everystate in the commit that completes `-coverage` (see the status
table below); until then it keeps using Java TLC.
| `tools/inv_checking_tool/src/trace_reader.py` | `eval()` on text from TLC output; a dynamic import | `ast.literal_eval` (same result on all 99,166 labels of a real TLC state-graph dump, never executes code); the user handler plugin moved to its own module |
| `src/specular/native_compaction.py` | `importlib.import_module("copilot")` | a plain lazy import (same behaviour) |

The last two changes remove the dynamic code that prevents a verified port of the pipeline to Rust with
[rust-anything](https://github.com/erkamyaman/rust-anything); they do not change behaviour.

Every site falls back to its original Java command when the Rust binary is not installed, so the patch
is safe on machines without Statehound.

## Verified

- The pipeline's own test suite (2,000+ unit tests, plus the invariant-checking tool's tests) has the
  same results with the patch as without it, in the same environment, both with and without Statehound
  installed in `bin/`. The patch adds `tests/conftest.py`, which sets `STATEHOUND=off` for the suite so
  that the launcher tests (which use a fake `java`) keep testing the Java path when Statehound is
  installed.
- With `lib/` removed entirely, the patched launcher runs a real check through Statehound and reports
  the violation; the pipeline's own `TLCOutputReader` parses Statehound's output.
- Statehound stops its children within 3 s of the supervisor's SIGTERM (the pipeline allows 5 s), and
  every child exits on its own if Statehound is killed outright.

## Status of the remaining Java paths

| Feature | Used by | Status |
|---|---|---|
| model checking, simulation, liveness | every phase | Rust (everystate) |
| `-dumpTrace json\|tla`, `-loadTrace json` | trace replay, `-j` | Rust (everystate patch 0002): 46 of 72 corpus dumps byte-identical to TLC's, the rest differ only in variable order or (liveness) in the lasso chosen; the replay tool's output on a sample trace is byte-identical to TLC's. TLC's binary `tlc` format is not supported. Like TLC, a JSON trace with model values does not replay (TLC writes them as strings) |
| `-continue` | `-C` | in progress in everystate |
| `-coverage` | coverage script | in progress in everystate |
| `-debugger` (DAP) | trace debugger tool | Rust (everystate patch 0003): the tool's test suite passes 9/9 with no Java runtime present; breakpoint hit counts identical to TLC on every recorded scenario (252 and 792 stops on the etcdraft trace spec). Differences: functions with string keys are printed in record syntax (as in everystate's normal output); no Stack scope, stepBack or reverseContinue |
| SANY parse check | spec validation | in progress in everystate |
| CFA `vav` check | spec analyzer tool | Rust port in progress (`crates/tla-cfa`) |

Until a feature lands, Statehound refuses the corresponding option with a clear message instead of
silently ignoring it (in the past runs measured, agents passed `-dumpTrace` in 0 of 393 model-checking
jobs and `-continue` in 1), and the tools that need it keep using Java TLC.
