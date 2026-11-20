# Measured results

All numbers were measured on one machine (Apple silicon, 12 cores, 48 GB) against a corpus of real
model-checking jobs: the specs, configurations and TLC flags that an agent-driven TLA+ bug-finding
pipeline produced during three runs against Apache Camel components (cluster leadership, SQL state
stores, Kafka repositories). The corpus has 111 distinct (spec, config) jobs; the 97–99 that TLC
finishes within 5 minutes are compared below. The specs themselves are not included in this repository.

Raw per-job results are in [`bench/`](../bench) (`results.jsonl`, `hunt_results.jsonl`,
`wrapper_results.jsonl`, `timeout_hunt.jsonl`), with the scripts that produced them.

## Where a pipeline run spends its time

One run (SQL state stores, 22.8 h wall clock):

| Phase | Wall time | What it waits on |
|---|---|---|
| Code analysis, spec generation, harness, validation | ~7 h | TLC runs fill 2.9 h of spec generation + validation |
| Bug confirmation (62 findings) | 15.2 h | one agent per finding, ~26 min each, 2 at a time |
| Classification | 0.7 h | an agent |

Model checking is the largest wait inside spec generation and validation; confirmation is the largest
phase overall and is limited by agent parallelism, not by the checker.

## 1. everystate vs TLC (engine only)

99 jobs, 4 workers each, same flags (`bench/results/engine_vs_tlc.jsonl`).

| | |
|---|---|
| Same verdict (pass / safety / deadlock / liveness) | 98 / 99 (see below) |
| Passing jobs with identical distinct-state counts | 22 / 22 |
| Safety counterexamples of identical length | 63 / 70 (see below) |
| Total time | TLC 780 s, everystate 383 s (2.04x) |
| The 16 jobs where TLC took over 5 s | median 2.3x, range 0.6x – 21.6x |

- The one verdict difference is a trace-validation spec whose `ASSUME` reads a file that does not exist:
  TLC exits 10 (assumption failure), unpatched everystate exits 255. Both refuse the spec.
- On the 7 safety jobs with counterexamples of different lengths, re-running TLC with one worker gave
  exactly everystate's trace: 4-worker TLC returned traces 1 to 2 steps longer than the shortest.
- everystate is slower than TLC on 4 jobs, which use `SYMMETRY` or a state constraint; its symmetry
  canonicalization is the hot spot.

## 2. Guided hunting (`everystate -hunt`)

70 violating jobs x 3 seeds, one thread, 60 s budget, the replay self-check on.

| Strategy | Found | Replay failures | Time to violation (median / p90 / max) | Trace vs shortest (median) |
|---|---:|---:|---|---:|
| `frontier` (default) | 207 / 210 | 0 | 0.02 s / 0.13 s / 0.78 s | 1.36x |
| `guided` (MCTS + Thompson + Boltzmann) | 178 / 210 | 0 | 0.02 s / 0.62 s / 8.77 s | 1.54x |

The guided strategy found nothing the frontier strategy missed (0 runs), while the frontier strategy
found 29 that the guided one missed; this is why both of Statehound's hunters use the frontier strategy
(with different seeds).

On the jobs where TLC needed more than 3 s: hunter 0.13 s (one thread) vs everystate exhaustive 1.47 s
(4 threads) vs TLC 4.22 s (4 threads), medians.

On the 5 jobs where TLC ran into its 30-minute limit without a result, a 5-minute hunt each (2.2 to 5.0
million distinct states) found no violation; those specs most likely have none within their bounds.

## 3. Statehound end to end

All 99 jobs through `statehound` vs TLC, 4 workers each (exhaustive search with all workers, 2 hunters
at low priority, cache off):

| Jobs | n | TLC | Statehound | Speed-up |
|---|---:|---:|---:|---:|
| all | 99 | 780 s | 334 s | 2.34x |
| violating | 70 | 128 s | 13 s | 9.8x |
| passing | 22 | 570 s | 315 s | 1.81x |
| liveness | 3 | 81 s | 4 s | 22x |

99 / 99 verdicts agree with TLC, and all 22 passing jobs have TLC's exact state count. A repeated
identical check is answered from the cache in about 0.02 s.

## 4. Correctness of the everystate patch

- everystate's own test suites pass: 42/42 node tests, 21/21 community-module fixtures, 27/27
  multi-worker checks.
- Outside `-hunt`, output is byte-identical to upstream everystate 0.4.1 on 60 corpus specs, for both
  `-simulate` (fixed seed) and exhaustive checking.

## 5. Confirmation phase

Per finding: about 22 min wall clock, 75 % of it model generation, 80–90 agent turns, about $7.
Durations are uniform (median 26–30 min), so reordering findings gains little; parallelism does:
62 findings take about 15 h at 2 in parallel, 7.5 h at 4 and 5 h at 6, at the same total cost.

A small A/B test re-ran the original confirmation prompt of two findings, each with and without a
shared "confirmation kit" (exact skill location, a prebuilt pristine module and its classpath). On the
one usable pair, the kit cut wall time 26 % (7.3 → 5.4 min) and tool calls 35 % (40 → 26) at equal cost
and the same verdict. That is a single pair, so it shows the direction, not the size.
