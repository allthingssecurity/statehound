# Metastability examples

Two small specs that show the `metastable` failure model of the hunt protocol. See
[docs/metastability.md](../../docs/metastability.md) for the background.

- `RetryStorm.tla`: a bounded queue whose clients retry when it is longer than their patience.
- `TicketControllers.tla`: two ticket controllers on one CPU that watch a churn-inflated metric
  (mode `rate`), and the fix with one controller on an honest signal (mode `fixed`).

Both start from every state (`Init == TypeOK`, trigger over), which is what makes the loops visible.

```sh
statehound -config RetryStorm_NoRetry.cfg    RetryStorm.tla        -deadlock   # pass
statehound -config RetryStorm_Retry.cfg      RetryStorm.tla        -deadlock   # Recovers violated
statehound -config Tickets_rate_Metric.cfg   TicketControllers.tla -deadlock   # HonestMetric violated
statehound -config Tickets_rate_Settles.cfg  TicketControllers.tla -deadlock   # pass: stops at the cap
statehound -config Tickets_rate_Recovers.cfg TicketControllers.tla -deadlock   # Recovers violated
statehound -config Tickets_fixed_Metric.cfg  TicketControllers.tla -deadlock   # pass
statehound -config Tickets_fixed_Recovers.cfg TicketControllers.tla -deadlock  # pass
```

`-deadlock` turns off deadlock checking: a drained queue with nothing left to do is a healthy end, not a
deadlock. In the `Tickets_rate_Recovers` trace the last state is followed by "Stuttering": every enabled
step of the code leaves the state unchanged (no short task can finish), so the system stays unhealthy
for ever.
