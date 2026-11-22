---
name: statehound
description: Formal verification of a codebase with TLA+ and Statehound (pure Rust, no JVM). Use when asked to formally verify a repository or component, hunt for concurrency, race, distributed-systems or crash-recovery bugs, find what can go wrong when events interleave, or to model check / explain a TLA+ spec or TLC counterexample. Finds the risky areas of a repo itself, hunts them in parallel, and reports reproduced bugs and what was verified up to which bounds.
---

# Statehound: formal verification of a repository

When the user asks for formal verification (of a repository, a directory, or "this code"), run the
whole process below without asking them to write specs or choose processes. Ask only when the target
itself is unclear.

## The process

```text
scan  ->  survey  ->  parallel hunts (one per area)  ->  AUDIT.md
```

### 1. Ask two questions first

1. **Which installation.** Run `statehound audit --list-profiles`. If it lists more than one profile for
   your agent (for example `claude default`, `claude account2`, `claude sap`), ask the user which one the
   audit should use and pass it as `--profile NAME`. With one profile, use it without asking.
2. **How deep.** Run a dry run to get the estimate, then ask the user to choose the depth:

   ```sh
   statehound audit <repo> --depth short  --dry-run
   statehound audit <repo> --depth medium --dry-run
   statehound audit <repo> --depth full   --dry-run
   ```

   | Depth | Hunt areas | Scope ladder | Reproduced against the code | Per area |
   |---|---|---|---|---|
   | `short` | up to 4 | rung R1 only | the top finding of each area | 15–35 min |
   | `medium` (default) | up to 8 | R1, R2, and R4 for paired areas | high and critical findings | 35–75 min |
   | `full` | up to 20 | R1 to R4, then climb until checks exceed the budget | every finding | 1–3 h |

   Quote the estimate line of each dry run (wall clock and approximate cost). Per-area times are
   assumptions until the first areas finish; during the run `audit` prints a re-estimate after every
   finished area, based on the measured durations of this run.

### 2. Run the audit

```sh
statehound audit <repo> --agent claude --profile <name> --depth <depth> --parallel 3 --yes
```

Use `--agent codex` when you are Codex. Run it in the background and report progress lines to the user as
areas finish. `audit`:

1. **scans** every source file (Java, Go, Rust, Python, C/C++, C#, JS/TS, Kotlin, Scala, Erlang/Elixir)
   and scores each directory on concurrency and distributed-systems signals;
2. runs one **survey** agent that reads the top candidates and writes the queue of hunt areas
   (`hunt-areas.json`: mechanism, files, actors, shared state, failure scenarios, properties), following
   [references/survey.md](references/survey.md);
3. runs one **hunt** agent per area, `--parallel` at a time, each in its own git worktree of the repository
   (the user's checkout is never modified), following
   [references/hunt-protocol.md](references/hunt-protocol.md): model the mechanism, write the TLA+ spec,
   check it with `statehound` on the scope ladder (R1: 2 actors with every applicable failure model on;
   R2: 3 actors or nodes, crash and restart, a partial partition; R3: more crashes, two versions; R4: two
   paired areas together), triage every counterexample, reproduce candidate bugs against the real code, and
   write `report.json`. Every spec covers the failure models of the protocol's catalogue that apply (a call
   that throws, a timeout while the other side is alive, stop and start of the same instance, crash and
   restart, a stale cache, delayed markers, ...), and `report.json` says which were modelled;
4. writes **`AUDIT.md`** and `audit.json` in the output directory (default
   `~/.statehound/audits/<repo name>/`, outside the repository):
   confirmed bugs first, then model-only leads, then the areas verified clean with the scope each was
   verified to, and which failure models each area covered.

Other options: `--out DIR`, `--areas FILE` (skip the survey and use your own queue), `--max-areas N`,
`--check-budget SECONDS` and `--area-timeout MINUTES` (override the depth's defaults), `--model NAME`,
`--dry-run`. Re-running `audit` with the same `--out` resumes: finished areas are kept. Without `--yes`
and in a terminal, `audit` itself asks for the profile and asks before starting the survey and the hunts.

### 3. Read and present the result

Open `AUDIT.md`. For each confirmed bug give the user: what goes wrong, the exact interleaving (from the
trace), the code locations, how the reproduction shows it, and a fix direction. Then the leads, then
what was verified and to which bounds. Do not present model-only leads as bugs.

### Doing it by hand

When `statehound audit` is not available or the user wants one area only, do the same yourself: pick
areas with [references/survey.md](references/survey.md), then follow
[references/hunt-protocol.md](references/hunt-protocol.md) for each (in Claude Code, one subagent per area
runs them in parallel).

## Checking a single spec

```sh
statehound -config Spec.cfg Spec.tla -workers 4            # exhaustive check + guided hunters
statehound -config Spec.cfg Spec.tla -workers 4 -deadlock  # do NOT check for deadlock (TLC's meaning)
everystate -config Spec.cfg Spec.tla -hunt budget=120      # hunt only, for state spaces too big to finish
```

Exit status: 0 no error, 10 assumption failed, 11 deadlock, 12 safety violation, 13 liveness
violation, 150 parse error, 151 config error, 255 other error.

- A **pass** ("Model checking completed. No error has been found.") always comes from the exhaustive
  search: every reachable state was checked, for the constants in the config.
- A **violation** may come from the exhaustive search or from a hunter ("violation found by guided
  hunter"); either way it is a real behaviour of the spec. Use `STATEHOUND_HUNTERS=0` for the shortest
  trace.
- A hunt that finds nothing proves nothing; it prints a Good-Turing estimate of how saturated it was.

## Reading a counterexample

Each `State N:` block is one state; the line in angle brackets names the action that produced it and its
location in the spec. Read it as a story (who did what, in which order, which variable ended up wrong),
then map each action to the code it models.

## Inductive invariants (for proofs)

To test whether `Inv` is inductive, check a spec whose initial states are all type-correct states
satisfying `Inv` (`Init == TypeStates /\ Inv`) with the original `Next`. A violation is a counterexample
to induction: strengthen `Inv` to exclude it and repeat.

## Install

`scripts/install.sh --claude` (or `--codex`) in the statehound repository builds `statehound` and
`everystate` into `~/.local/bin` and installs this skill.
