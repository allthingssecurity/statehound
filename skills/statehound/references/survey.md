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
  "why": "renewal reads expiry and writes owner in two statements without a version check (LeaseManager.java:88-104)",
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
| `why` | the code evidence that ordering matters here |
| `priority` | 1 = most promising |

The file must be plain JSON (no comments). Cite file and line numbers in `why`. Be concrete in `properties`: each should become a TLA+ invariant.
If the repository has no plausible hunt area, write an empty array and explain why in your final
message.
