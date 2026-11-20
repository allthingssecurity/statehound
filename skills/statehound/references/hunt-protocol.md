# Hunt protocol: formally verify one area of a repository

You are verifying one hunt area of a real codebase with TLA+ and Statehound. Work only inside the
worktree and the area directory named in the task. The worktree is a private copy of the repository:
you may build it, add tests and add logging there; never push, commit to a remote, or open issues.

The goal is the truth about this area, with evidence: real bugs reproduced against the code, or a clear
statement of what was verified and up to which bounds. A fabricated or exaggerated finding is worse than
none.

## 1. Understand the mechanism

Read the area's files and whatever they call that matters for ordering. Write `model.md` in the area
directory:

- the actors and how many of each can run at once;
- the shared state (in memory, database, files, messages) and who reads and writes it;
- the atomic steps: what happens between two points where another actor can interleave (a lock region,
  one SQL statement or transaction, one message handler, one `await`);
- the failures the code must survive (crash, restart, timeout with late success, retry, duplicate or
  reordered delivery, lost lease, clock skew), and which ones the code claims to handle;
- each property from the area description, stated precisely, with the code that is supposed to ensure it.

Cite file:line for every claim.

## 2. Write the spec

In `spec/` in the area directory, write `<Name>.tla` and `<Name>.cfg`:

- one variable per piece of shared state, plus per-actor local state (program counter, registers);
- one action per atomic step, named after the code (`RenewLease(n)`, `CommitOffset(c)`), with a
  comment citing file:line;
- failure actions for the failures in scope (`Crash(n)`, `Timeout(r)`, `Duplicate(m)`), each bounded by
  a constant (`MaxCrashes`) so the state space stays finite;
- every property as an invariant (or an action property / temporal property where it is about steps or
  progress), plus `TypeOK`;
- constants for the number of actors and the failure bounds, set in the `.cfg`.

Model what the code does, not what it should do. When the code is ambiguous, model the most permissive
behaviour it allows and note the choice in `model.md`.

Check the spec is not vacuous: temporarily add an invariant that must fail (for example that some
action never happens) and make sure `statehound` reports it; then remove it.

## 3. Check it, climbing the scope ladder

Run, from `spec/`:

```sh
statehound -config <Name>.cfg <Name>.tla -workers 4 -deadlock
```

(drop `-deadlock` when the system must never get stuck; then a deadlock is a finding too.)

Start with the smallest interesting scope: 2 actors, each failure bound 1. If the check passes, grow the
scope one step at a time (3 actors; failure bounds 2; more messages or keys) and check again, keeping a
copy of each config (`<Name>_s1.cfg`, `<Name>_s2.cfg`, ...). Stop climbing when a check exceeds the
per-check budget given in the task, or at a scope beyond which the code has no new behaviour (explain
why). For a check that does not finish within the budget, run a guided hunt on it:

```sh
everystate -config <Name>_sN.cfg <Name>.tla -hunt budget=<budget seconds> -deadlock
```

A pass is exhaustive for its scope; a hunt without a violation is not a pass. Record every run (scope,
result, distinct states, time) in `checks.md`.

## 4. Triage every violation

For each counterexample:

1. Read the trace as a story: which actor did what, in which order, with which failure.
2. Decide whether it is a **spec mistake** (the model allows something the code prevents; cite the code
   that prevents it, fix the spec, re-check) or a **candidate bug** (the code allows it).
3. For a candidate bug, reproduce it against the real code in the worktree: a test or a small program
   that forces the same order of events (threads with latches or barriers, a controllable clock, fault
   injection at the exact step, two processes against one database). Run it. Keep the test file and its
   output in `repro/` in the area directory.
4. Classify: **confirmed** (the reproduction shows the property violated in the real code),
   **model-only** (the trace looks real but you could not reproduce it; say what blocked you), or
   **spec mistake** (fixed).

Before calling anything confirmed, check that it is not documented or intended behaviour and not already
fixed upstream (search the repository history for the mechanism).

## 5. Report

Write `report.md` (for people) and `report.json` (for the aggregator) in the area directory.

`report.json`:

```json
{
  "area": "lease-renewal",
  "status": "bugs-found",
  "verified": [
    {"property": "AtMostOneHolder", "scope": "3 nodes, 2 crashes, 2 timeouts", "result": "pass", "distinct_states": 1840221}
  ],
  "findings": [
    {
      "id": "lease-renewal-1",
      "title": "Two nodes hold the lease after a renewal that times out but commits",
      "property": "AtMostOneHolder",
      "classification": "confirmed",
      "severity": "high",
      "trace": "spec/trace-1.txt",
      "reproduction": "repro/LeaseDoubleHolderTest.java",
      "code": ["src/cluster/LeaseManager.java:88", "src/cluster/LeaseManager.java:104"],
      "summary": "Renewal retries after a timeout whose write committed; the retry sees its own row and ..."
    }
  ],
  "notes": "what was out of scope, what blocked reproduction"
}
```

`status` is one of `bugs-found` (at least one confirmed finding), `leads` (model-only findings, none
confirmed), `verified` (no violation; `verified` lists what passed and at which scope), or `blocked`
(the area could not be modelled or built; say why). Severity: `critical` (data loss or corruption,
safety broken under normal operation), `high` (under plausible failures), `medium`, `low`.

Finish with a short final message: the status and one line per finding.
