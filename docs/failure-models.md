# Failure models from failure studies

A model checker only finds the failures you put into the model. Studies of production failures show which
kinds matter most, so Statehound's hunt protocol carries a catalogue of failure models, and every hunt says
which ones apply to its area and which it modelled. The full catalogue and the wording given to the agents are
in [hunt-protocol.md](../skills/statehound/references/hunt-protocol.md). This page explains the four added in
0.4.0. Metastability, added in 0.3.0, has [its own page](metastability.md).

## `thread-death`: the process lives, a thread does not

A timer, worker or callback thread dies from one exception and the component silently stops doing that part of
its job. In Java, a task given to `scheduleAtFixedRate` or `scheduleWithFixedDelay` that throws is never run
again; an executor keeps a task's exception inside a future that nobody reads.

- Evidence: in 100 partial failures from five systems, 48% left some functionality stuck while the process
  stayed up (Lou, Huang and Smith, "Understanding, Detecting and Localizing Partial Failures in Large System
  Software", NSDI 2020).
- What the spec adds: a throw in each periodic or background task, handled exactly as the code does, and a
  liveness property that the task keeps running.
- Scan: `periodic` counts scheduled tasks and tickers; `periodic_unguarded` marks files that schedule them with no
  broad catch anywhere.
- Example: [`examples/thread-death/`](../examples/thread-death/).

## `retry-semantics`: IF, WHEN and HOW

Every retry (redelivery, reconnect, resubscribe, failover, client retry) is checked on three questions:

- **IF**: only errors that can succeed on retry are retried, and retryable ones are not given up on;
- **WHEN**: attempts have a delay and a cap, so a failing call does not spin;
- **HOW**: the state attempt N reads from attempt N-1 (counters, offsets, partial output, half-open connections)
  is reset or kept as intended, and the retried operation is idempotent or deduplicated.

Evidence: Stoica et al., "If At First You Don't Succeed, Try, Try, Again...? Insights and LLM-informed Tooling for
Detecting Retry Bugs in Software Systems", SOSP 2024, classify retry bugs into these three kinds; their tool found
over a hundred retry bugs in eight open-source systems.

## `gray-failure`: the health signal says healthy, the work is stuck

A health check, heartbeat, "started" status, leader flag or success counter vouches for work it does not observe.
The spec models the signal and the data path separately, with the property that "healthy" or "done" implies
real progress, and a broken data path is eventually reported.

Evidence: Huang et al., "Gray Failure: The Achilles' Heel of Cloud-Scale Systems", HotOS 2017, name differential
observability (the application suffers while failure detectors stay silent) as the defining trait and report
gray failures behind many incidents in Azure. The `HonestMetric` check of the metastability model is the same idea
for controllers.

## `fail-slow`: slow, not dead

A dependency, disk or peer answers slowly instead of failing. Timeouts tuned for a dead peer may never fire, and
threads, permits, queues and connections pile up behind the slow step. The spec adds a step that succeeds after a
long delay, and checks that held resources stay bounded and unrelated work still progresses.

Evidence: Gunawi et al., "Fail-Slow at Scale: Evidence of Hardware Performance Faults in Large Production
Systems", FAST 2018, collected 101 fail-slow incidents from 12 institutions across every hardware type; Do et al.,
"Limplock", SoCC 2013, showed one slow node can slow a whole cluster.
