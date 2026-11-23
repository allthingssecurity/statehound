# Survey: choose the hunt areas of a repository

You are choosing where formal verification with TLA+ can find real bugs in this repository. You get the
output of `statehound scan`: every source directory scored by concurrency and distributed-systems
signals (locks, atomics, threads, async, channels, transactions, retries, timeouts, leases, leader
election, offsets and acknowledgements, idempotency, caches). The scan is a starting point, not the
answer: read the code.

## What makes a good hunt area

A hunt area is one mechanism whose correctness depends on the order of events, small enough to model in
one spec (typically 1 to 10 source files):

- several actors touch shared state: threads, goroutines, async tasks, processes, nodes, or a database
  shared by several instances;
- the code assumes something about ordering or atomicity that failures can break: crash between two
  writes, timeout then late reply, retry, duplicate or reordered message, lost lease, clock skew,
  concurrent schema or config change;
- a violation would matter: lost or duplicated data, two owners or leaders, a stuck consumer, a leaked
  lock, a resurrected deleted item, an expired value served.

Not hunt areas: pure functions, parsing, formatting, UI, configuration plumbing, code whose concurrency
is entirely delegated to a well-tested library and used in the documented way.

Prefer mechanisms over directories: "the lock-renewal loop and its interaction with shutdown" is a
better area than "package lock". One directory can hold several areas; one area can span directories.

## Where the trickiest bugs are

Studies of real production failures point to the same places again and again. Rank areas by how many of these
they touch, not only by how much locking they contain:

- **Error handling.** 92% of catastrophic failures in distributed systems come from mishandled non-fatal errors,
  and a third of those come from trivial handlers: empty or log-only `catch` blocks, a broad `catch` that stops a
  component or exits, a TODO (Yuan et al., OSDI 2014). The scan category `errors` counts them.
- **Lifecycle vs. workers.** `doStart`/`doStop`/`close`/`reconnect`/rebalance callbacks that share fields with
  worker, callback or timer threads. A stop that does not wait for the threads, then a start of the same
  instance, lets old threads see the new state. The scan category `lifecycle` counts these methods, and
  `lifecycle_threads` marks files that have both lifecycle methods and threads.
- **Timeouts.** In timeout bugs, a timeout that is missing (31%) or wrongly sized (47%) is the most common cause,
  and most give no error message. A timeout firing while the other side is still alive leads to a late
  completion. The scan category `blocking` counts calls that wait with no deadline.
- **Crash and restart.** Recovery that reads persisted state written non-atomically, or races with new work.
  74% of crash-recovery bugs need only one crash.
- **Stale views.** Caches, registries and pools read by control logic that can be stale or skip a transition, and
  eviction of something still in use.
- **Control markers.** Commit/abort/fence/epoch markers delayed or reordered against data (transactions,
  offsets, acknowledgements).
- **Formats across versions.** Persisted or wire formats (serialization, repositories, offset stores) that another
  version or another system writes. Most upgrade failures come from such incompatibilities. The scan category
  `persistence` counts them.
- **Connectors.** Code at a system boundary, where markers, offsets, acknowledgements and headers cross into another
  system. In the systems studied, connectors were under 5% of the code but received over half of the fixes.
- **Threads that die quietly.** A periodic or background task that throws stops for good while the process stays up:
  `scheduleAtFixedRate` cancels the task after one exception, an executor keeps the exception in a future nobody
  reads. In 100 partial failures, 48% left some functionality stuck (Lou et al., NSDI 2020). The scan category
  `periodic` counts scheduled tasks and tickers, and `periodic_unguarded` marks files that schedule them without a
  broad catch.
- **Retries.** Every retry has three questions: IF (is this error worth retrying, and is a retryable one given up
  on?), WHEN (is there a delay and a cap?), HOW (what state does attempt N see from attempt N-1, and is the retried
  operation idempotent?). These classes cover the retry bugs found in eight open-source systems (Stoica et al.,
  SOSP 2024).
- **Health signals that lie.** Health checks, heartbeats, "started" flags and success counters that can say healthy
  while the work they vouch for is stuck (gray failure, Huang et al., HotOS 2017). The scan category `health`
  counts them.
- **Slow, not dead.** A dependency that answers slowly holds threads, permits and connections; timeouts sized for a
  dead peer may never fire (Gunawi et al., FAST 2018).
- **Feedback loops (metastability).** Retries, re-enqueues on timeout, and controllers that change a limit from a
  metric (adaptive concurrency, autoscaling executors, rate limiters, circuit breakers, admission control). After a
  burst or a slowdown ends, they can keep the system degraded by themselves. At least 4 of 15 major AWS outages in a
  decade were metastable, and retries sustained more than half of the 22 incidents studied (Huang et al., OSDI 2022).
  Two sensible controllers on one shared resource can feed each other, and a metric that churn inflates (operations
  per second rather than useful work) rewards the wrong correction. For each controller, note what it measures,
  what it changes, and which resource it shares; two controllers on one resource, or retry layers stacked on one
  path, make a strong area. The scan category `feedback` counts limiters, breakers, throttles and adaptive limits,
  and `feedback_loop` marks files where they meet retries or timeouts.
- **Two mechanisms together.** 80% of distributed concurrency bugs span more than one protocol. Note which areas
  share state or exception handlers (`pairs_with`), so a hunt can combine them.

For every candidate area, answer two questions before you choose it:

1. What happens if each external call in it throws, or hangs?
2. What persisted or wire state does it read that another thread, version or system writes?

If you cannot answer both from the code, it is a weaker area. For an area with retries or a controller, also
answer a third: once a burst or a slowdown ends, what in this code keeps the extra work going?

## Output

Write the file named in the task as a JSON array, most promising area first, at most the number of areas
the task allows. Each element:

```json
{
  "id": "lease-renewal",
  "title": "Lease renewal vs. shutdown in the cluster lock",
  "paths": ["src/cluster/LeaseManager.java", "src/cluster/LockService.java"],
  "actors": "renewal thread, request threads, shutdown hook; 2-3 nodes sharing one DB row",
  "shared_state": "lease row (owner, expiry), in-memory 'held' flag, renewal timer",
  "scenarios": ["node pauses past expiry then renews", "shutdown during renewal", "DB write times out but commits"],
  "properties": ["at most one node believes it holds the lease", "a released lease is never renewed"],
  "failure_models": ["throws", "timeout-alive", "crash-restart", "stop-start"],
  "pairs_with": ["lock-release-on-shutdown"],
  "why": "renewal reads expiry and writes owner in two statements without a version check (LeaseManager.java:88-104); the renewal catch block only logs (LeaseManager.java:121)",
  "priority": 1
}
```

| Field | Meaning |
|---|---|
| `id` | short unique name, lowercase letters, digits and `-` |
| `title` | one line naming the mechanism |
| `paths` | the source files the spec models (relative to the repository root) |
| `actors` | who acts concurrently, and how many of each are worth modeling |
| `shared_state` | the state they share |
| `scenarios` | failures and orderings to model |
| `properties` | what must always (or eventually) hold |
| `failure_models` | which failure models from the hunt protocol's catalogue apply here: `throws`, `timeout-alive`, `no-timeout`, `crash-restart`, `stop-start`, `stale-view`, `delayed-marker`, `dup-reorder`, `partial-partition`, `two-versions`, `config-sweep`, `thread-death`, `retry-semantics`, `gray-failure`, `fail-slow`, `metastable` |
| `pairs_with` | ids of other areas in this list that share state or exception handlers with this one (may be empty) |
| `why` | the code evidence that ordering matters here |
| `priority` | 1 = most promising |

The file must be plain JSON (no comments). Cite file and line numbers in `why`. Be concrete in `properties`: each should become a TLA+ invariant.
If the repository has no plausible hunt area, write an empty array and explain why in your final
message.
