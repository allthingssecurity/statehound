# Thread death and gray failure

`PeriodicRenewal.tla` models a component that renews a lease with `executor.scheduleAtFixedRate(renew, ...)`.
The renewal call can throw once. In Java, a periodic task that throws is never run again, while the component
and its executor stay up, and a health check that looks at "started" keeps reporting UP.

```sh
statehound -config Unguarded_Live.cfg   PeriodicRenewal.tla -deadlock  # KeepsRenewing violated: one throw, renewal stops for good
statehound -config Unguarded_Health.cfg PeriodicRenewal.tla -deadlock  # HonestHealth violated: UP while nothing renews
statehound -config Guarded.cfg          PeriodicRenewal.tla -deadlock  # pass: the task body catches everything
```

Verdicts and state counts are identical to Java TLC on all three. `-deadlock` turns off deadlock checking: a
cancelled task leaves no step enabled, but the process is alive, so the bug is the liveness violation, not a
deadlock.

These are the `thread-death` and `gray-failure` failure models of the hunt protocol. See
[docs/failure-models.md](../../docs/failure-models.md).
