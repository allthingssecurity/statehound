# Benchmarks

`results/` holds the raw per-job results behind [docs/results.md](../docs/results.md). Job identifiers
are `<run>/<target>/<file>`; the specs themselves come from private pipeline runs and are not included.

| File | What |
|---|---|
| `engine_vs_tlc.jsonl` | everystate vs Java TLC, 4 workers, same flags (99 jobs) |
| `hunt_frontier.jsonl` | `everystate -hunt` (frontier strategy), 70 violating jobs x 3 seeds, 1 thread |
| `hunt_guided.jsonl` | the same with `strategy=guided` (MCTS + Thompson + Boltzmann) |
| `statehound_vs_tlc.jsonl` | `statehound` end to end vs Java TLC (99 jobs) |
| `hunt_on_tlc_timeouts.jsonl` | 5-minute hunts on the 5 jobs where TLC hit its 30-minute limit |

`scripts/` reproduces them on your own corpus:

```sh
python3 scripts/collect.py RUNS_DIR corpus.json                    # jobs recorded by past pipeline runs
TLC_JAVA=java TLC_CP=/path/tla2tools.jar:/path/CommunityModules-deps.jar \
  python3 scripts/run_bench.py corpus.json engine.jsonl $(which everystate)
python3 scripts/hunt_bench.py engine.jsonl hunts.jsonl $(which everystate)
python3 scripts/wrapper_bench.py engine.jsonl statehound.jsonl $(which statehound)
```

Java TLC appears here only as the reference the benchmarks compare against.
