---- MODULE PeriodicRenewal ----
(* A component renews a lease with executor.scheduleAtFixedRate(renew, ...). The renewal call can throw (the lease
   store is briefly unreachable). In Java, a periodic task that throws is never run again, while the component and
   its executor stay up. The health check reports UP because the component is started.
   Guarded = TRUE models the fix: the task body catches everything, so the schedule survives.
   thread-death: does renewal keep happening?  gray-failure: is "UP" only reported while renewal really runs? *)
EXTENDS Naturals
CONSTANTS Guarded, MaxThrows
VARIABLES scheduled,  \* the periodic task is still scheduled
          throws,     \* store failures so far (bounded)
          renewals    \* successful renewals, capped so the state space stays finite
vars == <<scheduled, throws, renewals>>
Cap == 3

TypeOK == scheduled \in BOOLEAN /\ throws \in 0..MaxThrows /\ renewals \in 0..Cap
Init == scheduled = TRUE /\ throws = 0 /\ renewals = 0

\* one run of the periodic task: the store answers
RenewOk == scheduled /\ renewals' = (IF renewals < Cap THEN renewals + 1 ELSE 0) /\ UNCHANGED <<scheduled, throws>>
\* one run of the periodic task: the store throws. Unguarded, the executor cancels all later runs.
RenewThrows == scheduled /\ throws < MaxThrows /\ throws' = throws + 1
               /\ scheduled' = Guarded /\ UNCHANGED renewals
Next == RenewOk \/ RenewThrows
Spec == Init /\ [][Next]_vars /\ WF_vars(RenewOk)

\* thread-death: the lease keeps being renewed
KeepsRenewing == []<><<renewals' # renewals>>_renewals
\* gray-failure: the health check (UP because the component is started) is honest only if renewal still runs
Health == "UP"
HonestHealth == Health = "UP" => scheduled
====
