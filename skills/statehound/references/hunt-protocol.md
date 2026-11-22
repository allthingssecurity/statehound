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
- each property from the area description, stated precisely, with the code that is supposed to ensure it;
- a **failure-model table**: one row per model in the catalogue below. For each row, say whether it applies here
  (cite the code: the call that can throw, the timeout, the persisted field, the lifecycle method), and whether
  you model it. If it applies but you do not model it, say why.

Cite file:line for every claim.

### Failure-model catalogue

Studies of real failures show that the bugs that matter usually need one well-placed fault, not many. They are
missed mostly because a whole kind of fault was never in the model. So enable every model that applies from the
first scope.

| Model | What it adds to the spec | Why (evidence) |
|---|---|---|
| `throws` | Every external or remote call can fail instead of returning. The spec does exactly what the handler in the code does: swallow, log only, retry, rethrow, stop the component, or exit. | 92% of catastrophic failures come from mishandled non-fatal errors; a third of those come from empty, log-only or TODO handlers. |
| `timeout-alive` | A timeout can fire while the other side is still working. The work then completes late, after the timeout's cleanup. | Late completion after a timeout is the shape of most permit, slot and resource leaks. |
| `no-timeout` | A wait with no deadline (`take`, `await`, `join`, `acquire`, a future without a timeout) is modelled as possibly never returning. Check with a liveness property, or a deadlock check, that work is eventually completed or released. | Timeouts that are missing (31%) or wrongly sized (47%) are the most common causes of timeout bugs, and most give no error message. |
| `crash-restart` | Crash at any step. Restart from persisted state only, with in-memory state lost. Allow a crash during recovery. Writes to disk or an external store may be partial or reordered unless the code forces order (fsync, transaction). | 74% of crash-recovery bugs need a single crash, and recovery itself is a common culprit. Applications often assume writes are ordered and atomic. |
| `stop-start` | Stop and start the **same instance** while worker, callback and timer threads still run. A stop that does not wait (`shutdownNow`, a cancel, a flag) leaves old threads running into the new state. Include suspend/resume and reconnect. | This is the most common class in Statehound's own confirmed bugs. |
| `stale-view` | A read of a cache, registry, pool or metadata store returns an older value, or misses an intermediate transition. Entries can be evicted while in use. | Stale and unobserved states cause many controller and cache bugs. |
| `delayed-marker` | Commit, abort, fence, epoch, offset or acknowledgement markers can be delayed or reordered relative to the data they guard. | Jepsen found lost writes and torn transactions from delayed markers in Kafka's transaction protocol. |
| `dup-reorder` | Messages and callbacks can be duplicated, reordered, or delivered after their requester gave up. | A classic distributed-systems failure model. |
| `partial-partition` | Connectivity is per pair and per direction: one node, preferably the leader, is cut off from some peers but not all. Check the invariants again after the partition heals. | 29% of partition failures need a partial partition, and all the ones studied can be triggered by partly isolating one node. |
| `two-versions` | Each node or persisted record carries an old or new version tag, and readers must handle both. Add an upgrade action. | Most upgrade failures come from incompatible persisted or network data, and over 80% need only two versions. |
| `config-sweep` | Key configuration flags become model constants (sync/async, transacted, eager, cache size, retry count, timeout relations). Check a few combinations in separate configs. | Many bugs only appear under one non-default setting. |
| `overload-retry` | A bounded queue, a retry that re-enqueues on timeout, and a check that the overloaded state ends once the trigger is removed. | Retries keep metastable outages going. Treat this one as optional: TLA+ is weak on rates. |

Failures are often silent: 90% of partition failures give no error. So check the data, not only for
exceptions: after recovery or healing, nothing is lost, duplicated, resurrected, stuck or leaked.

## 2. Write the spec

In `spec/` in the area directory, write `<Name>.tla` and `<Name>.cfg`:

- one variable per piece of shared state, plus per-actor local state (program counter, registers);
- one action per atomic step, named after the code (`RenewLease(n)`, `CommitOffset(c)`), with a
  comment citing file:line;
- failure actions for every model in your failure-model table (`Throw(c)`, `TimeoutFires(r)`, `Crash(n)`,
  `Stop`/`Start`, `StaleRead(k)`, `Duplicate(m)`), each bounded by a constant (`MaxThrows`, `MaxCrashes`) so the
  state space stays finite. Bound 1 each is usually enough;
- every property as an invariant (or an action property / temporal property where it is about steps or
  progress), plus `TypeOK`. Include at least one "nothing leaked or stuck" property: permits, slots, locks,
  registrations and in-flight entries are released, and accepted work is completed, rejected or reported. Where
  that needs progress, write it as a temporal property under weak fairness of the code's own steps (not the
  failure actions);
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

Climb this ladder, as far as the run depth in the task allows. Keep a copy of each config (`<Name>_R1.cfg`,
`<Name>_R2.cfg`, ...):

| Rung | Scope | Why this is enough (evidence) |
|---|---|---|
| R1 | 2 actors, a few steps each, **every applicable failure model enabled with bound 1** | 96% of concurrency bugs need 2 threads, and most need an order among at most 4 accesses |
| R2 | 3 actors or nodes; 1 crash + 1 restart; 1 partial partition isolating one node if the mechanism is distributed | 98% of distributed failures show up on 3 or fewer nodes |
| R3 | up to 3 crashes and a crash during recovery; 4–5 nodes; `two-versions` if a persisted or wire format is involved | covers the rest of crash-recovery and partition failures, and upgrades |
| R4 | combine with an area from `pairs_with` (or a neighbouring mechanism sharing state or handlers): add its actions and state at R1 bounds | 80% of distributed concurrency bugs span more than one protocol |

Spend extra budget on failure kinds and rung R4 before adding more actors: larger actor counts find little
that small ones miss. Stop climbing when a check exceeds the per-check budget given in the task, or at a
scope beyond which the code has no new behaviour (explain why). For a check that does not finish within the
budget, run a guided hunt on it:

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
  "failure_models": {
    "modelled": ["throws", "timeout-alive", "crash-restart", "stop-start"],
    "not_applicable": ["two-versions"],
    "skipped": [{"model": "partial-partition", "why": "single node; the store is external"}]
  },
  "rungs": ["R1", "R2"],
  "notes": "what was out of scope, what blocked reproduction"
}
```

`status` is one of `bugs-found` (at least one confirmed finding), `leads` (model-only findings, none
confirmed), `verified` (no violation; `verified` lists what passed and at which scope), or `blocked`
(the area could not be modelled or built; say why). Severity: `critical` (data loss or corruption,
safety broken under normal operation), `high` (under plausible failures), `medium`, `low`.

Finish with a short final message: the status and one line per finding.
