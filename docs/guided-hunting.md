# Guided hunting

Exhaustive breadth-first search (see [how-model-checking-works.md](how-model-checking-works.md)) visits
every state at depth 1, then every state at depth 2, and so on. If the bug is at depth 28 in a graph
with half a million states at that depth, it has to visit all of them first. A **hunter** gives up
completeness to get to unusual states quickly. Statehound runs hunters *next to* the exhaustive search,
never instead of it.

## Why a hunter can never make a verdict wrong

- Every successor a hunter computes is checked against the invariants and action properties by the same
  code that checks them in everystate's `-simulate` mode (`Sim::check_action`).
- A hunter keeps, for every state it has seen, the state and action it was first reached from, so a
  violation comes with a complete behaviour from an initial state. With `EVERYSTATE_SIM_REPLAY=1` that
  behaviour is replayed through the exhaustive checker's own successor function before it is reported;
  all 385 benchmark hunts that found a violation (both strategies) passed that replay.
- A hunter can only end a Statehound run by reporting a violation. "No error has been found" always comes
  from the exhaustive search. A hunt that ends without a violation says so, and says that it proves
  nothing.

So the worst a bad hunter can do is waste CPU. It runs at low priority with a 60-second budget, and on the
benchmark corpus every successful hunt succeeded within 0.8 seconds.

## Strategy 1: surprisal-guided best-first search (`strategy=frontier`, the default)

The hunter keeps an **archive** (every distinct state seen, by fingerprint) and a **frontier** (archived
states not expanded yet). Each step:

1. **Pick a frontier state.** With probability `dive` (default 0.5) take the newest one, a depth-first
   *dive* that gets deep quickly. Otherwise draw a random tournament of `pool` (32) frontier states and
   take the most **surprising** one:

   ```text
   surprisal(s) = Σ over variables v of  1 / sqrt( count(v, value of v in s) )
   ```

   `count(v, x)` is how many archived states have value `x` in variable `v`. A state whose variables take
   values the search has rarely seen scores high. This is count-based exploration, the same idea used in
   reinforcement learning (exploration bonuses) and in coverage-guided fuzzing (rare edges first): rare
   values are where untested behaviour is.

2. **Expand it by every action.** Every successor is checked; new ones join the archive and the frontier.

3. If the frontier empties, every reachable state has been expanded and checked: the hunt was in fact
   exhaustive (for safety properties).

Why it works on real specs: bugs in distributed-systems specs typically need a few unusual events
(a crash, a timeout, a retry) in a particular order. Breadth-first search spends its time on the huge
number of ordinary interleavings at each depth; surprisal pulls the search towards states where some
variable has an unusual value, and dives get it deep quickly.

On one real spec where exhaustive search reaches the violation after 480,456 distinct states (depth 28),
the frontier hunter reached it after 5,000 to 30,000 states, in 0.3 to 2.2 seconds on one thread, with
each of 5 seeds.

### Shortening the counterexample

A hunter's path to the bad state is usually longer than the shortest one. Before reporting, the hunter
**shortcuts** it: from each state of the trace, it computes that state's successors and jumps to the
furthest later state of the trace that is among them (compared exactly, and only through states inside
the `CONSTRAINT`s). The result is still a behaviour of the spec ending in the same bad state. On the
corpus the median reported trace is 1.36 times the shortest one. Run with `STATEHOUND_HUNTERS=0` when you
need the shortest.

## Strategy 2: bandits, tree search and annealing (`strategy=guided`)

This strategy runs bounded **rollouts** (32 steps) from archived states and combines four ideas:

| Idea | Field | What it decides | Rule |
|---|---|---|---|
| UCB1 (the tree policy of Monte Carlo tree search) | reinforcement learning | which archived state to start a rollout from | `mean reward + c·sqrt(ln N / (n_s + 1))`, reward = new states found per step, over a random tournament of 32 states |
| Thompson sampling | Bayesian statistics | the order in which to try actions at each step | per action a Beta(α, β) posterior of "this action yields a new state"; sample, sort descending |
| Boltzmann selection with annealing | statistical physics | which successor to move to | weight `exp(-(visits - 1) / T)`, temperature `T = T0 / ln(e + rollouts / 1000)` |
| Good–Turing estimator | information theory | when to stop | stop when `N1 / N < eps`: `N1` = states observed exactly once, `N` = all observations; `N1/N` estimates the probability that the next observed state is new |

Each of these is standard in its field, and together they make a reasonable bug hunter: 178 of 210
benchmark runs found the violation, with a median of 0.02 s. But the frontier strategy found 207, found
everything this strategy found, and found 29 that it did not. Rollouts keep revisiting the same region
once bounded resources in the spec (one crash, one restart) are used up early in each rollout; full
expansion of surprising states does not have that problem. So `guided` is available, but neither hunter
uses it by default.

The Good–Turing estimate is still printed at the end of every hunt as a measure of how saturated the
search was.

## Diagram equivalence: one representative per interleaving class (`por=on`)

A Feynman diagram stands for all the time orderings of its interaction vertices: you compute the topology
once instead of every ordering. Interleavings have the same structure. Two steps of different processes
that touch unrelated variables **commute**: doing `a` then `b` reaches the same state as `b` then `a`.
All the interleavings that differ only by such swaps form one equivalence class (a Mazurkiewicz trace),
and they all reach the same states.

Applied to the hunters, this means: at each state, when two enabled actions commute, explore only one
of their two orders. Because a hunter only ever reports violations it actually reached (and replays them),
such pruning can make a hunt faster or slower at finding bugs but never makes a verdict wrong; the
exhaustive search is not touched. This is being implemented and benchmarked (`-hunt por=on`, sleep sets
with an exact per-state commutativity check); it will be enabled by default only if it measurably helps.

## The portfolio

```text
statehound -workers W
├── everystate exhaustive search, W workers, normal priority     -> pass, violation or error
├── hunter 1: everystate -hunt (frontier, seed 1), nice 15, 60 s -> violation only
└── hunter 2: everystate -hunt (frontier, seed 2), nice 15, 60 s -> violation only
```

The first violation reported by anyone wins and the others are stopped (SIGTERM to their process
groups; every child also exits on its own if Statehound itself dies). If the exhaustive search finishes
first, its verdict is final.

On the benchmark corpus this gives 9.8x on jobs with a bug and 1.8x on jobs without one (the second
number is the faster engine alone; the hunters cost almost nothing because they stop after 60 s at low
priority). See [results.md](results.md).

## Options

```text
everystate -config X.cfg X.tla -hunt [budget=S][,max=N][,strategy=frontier|guided|uniform]
                                     [,dive=X][,pool=N][,rollout=N][,c=X][,temp=X][,eps=X][,minobs=N]
```

| Option | Default | Meaning |
|---|---|---|
| `budget` | none | wall-clock budget in seconds |
| `max` | 20,000,000 | maximal archived states (Statehound uses 5,000,000) |
| `strategy` | `frontier` | `frontier`, `guided`, or `uniform` (random, for comparison) |
| `dive` | 0.5 | frontier: probability of continuing from the newest state |
| `pool` | 32 | tournament size |
| `rollout` | 32 | guided: steps per rollout |
| `c` | 1.4 | guided: UCB exploration constant |
| `temp` | 1.0 | guided: initial Boltzmann temperature |
| `eps`, `minobs` | 1e-4, 200,000 | guided: Good–Turing stopping threshold and minimum observations |

`-seed N` makes a hunt reproducible; `-depth N` bounds the depth of archived states.
