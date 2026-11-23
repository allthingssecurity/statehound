# Metastability: retry storms and controller feedback loops

A metastable failure is a degraded state that the system keeps going by itself after its trigger has
ended. A burst of traffic slows a server, clients time out and retry, and the retries keep the server
overloaded after the burst is over. Huang et al. (OSDI 2022) collected 22 such incidents from 11
organizations. At least 4 of the 15 major AWS outages in the preceding decade were metastable, they lasted
1.5 to 73 hours, and retries sustained more than half of them.

Controllers can do the same. Demirbas et al. (ICCCN 2026) describe a database scheduler in which two
queues each tuned their own ticket count by watching the ticket-acquisition rate. A ticket bounds
wall-clock time, not CPU time, so issuing more tickets raises the acquisition rate without any more work
getting done: the metric measures churn. Under overload, the long-query controller added tickets to pump
its metric, the short queries slowed down, and the short-query controller added tickets too. Each
correction made the other's situation worse.

## Why ordinary checks miss it

These loops are invisible from a system's normal initial state: an empty queue with no retries in flight
recovers trivially. The loop needs the shock to be in place already. Statehound therefore checks
recovery **from every state** the type allows, with the trigger over and no new arrivals:

```tla
Init     == TypeOK /\ trigger = "off"
Recovers == <>[]Healthy        \* under weak fairness of the code's own steps only
```

A counterexample ends in a cycle ("Back to state N") that never becomes healthy: the self-sustaining
loop, found by exhaustive search rather than by a lucky simulation run.

## The four checks

| Property | Question | Counterexample |
|---|---|---|
| `Recovers` | After the trigger ends, from any state, does the system become healthy and stay healthy? | a loop the system sustains by itself |
| `HonestMetric` | Does every controller step that keeps a change because its metric improved also keep or raise the true progress? | a metric that churn inflates |
| `Settles` | With two controllers on one resource and constant load, do the limits stop moving? | oscillation or ratcheting |
| `BoundedAmplification` | Do attempts per original request stay within what the code intends across every retry layer? | stacked retries multiplying load |

Two rules keep the counterexamples meaningful. Fairness goes on the code's own steps (service,
retries, controllers), never on arrivals or the trigger. And `Recovers` is also checked with the
feedback switched off: it must pass then, which shows the loop comes from the feedback, not from the
abstraction.

## The example

[`examples/retry-storm/`](../examples/retry-storm/) holds both mechanisms above. Statehound and Java
TLC agree on all seven configurations, with identical verdicts and state counts:

| Config | Result |
|---|---|
| `RetryStorm_NoRetry` | pass: recovers from all 45 states |
| `RetryStorm_Retry` | `Recovers` violated: from a queue of 4 with one waiting client, retries hold the queue at 7 to 8 for ever |
| `Tickets_rate_Metric` | `HonestMetric` violated: the long queue adds a ticket because its acquisition rate rose, and short completions drop from 1 to 0 |
| `Tickets_rate_Settles` | pass: the escalation stops at the ticket cap... |
| `Tickets_rate_Recovers` | ...but `Recovers` is violated: short tasks never finish again. A cap bounds the damage; it does not remove the loop |
| `Tickets_fixed_Metric`, `Tickets_fixed_Recovers` | pass: the paper's fix (long tickets fixed, one controller on an honest signal) |

## What model checking does not tell you

It finds whether a loop exists and shows its mechanism. It does not say how likely the loop is under
real load or how long recovery takes. For that, a discrete-event simulation of the same actors and
queues with timing distributions (Demirbas et al. used their simulator, MESSI) answers "after a 10 s
burst, how long until useful throughput recovers?". The hunt protocol's reproduction step for a
metastable finding is a load test of that kind against the real code: drive a burst or a slowdown,
remove it, and measure useful throughput per interval afterwards.

## In an audit

- `statehound scan` counts limiters, breakers, throttles and adaptive limits (category `feedback`) and
  marks files where they meet retries or timeouts (`feedback_loop`).
- The survey asks, for areas with retries or a controller, what keeps the extra work going after a burst
  ends, and notes what each controller measures, what it changes, and which resource it shares.
- The hunt protocol's `metastable` failure model gives the abstraction (small counters, a tick, a trigger
  that ends, useful vs. wasted work as a ghost variable), the four checks, and the reproduction.
