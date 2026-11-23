---- MODULE RetryStorm ----
(* A server with a bounded queue serves one request per tick. When the queue is longer than the clients'
   patience T, a waiting client times out and retries: its old copy stays queued as wasted work and a new
   copy joins. After the trigger (a burst) has ended there are no new arrivals, yet with retries on the
   queue can stay long for ever: a metastable failure. Checked from every state, not only an empty queue. *)
EXTENDS Naturals
CONSTANTS Max,    \* queue capacity
          T,      \* queue length above which waiting clients time out
          Retry   \* TRUE: clients retry on timeout (the feedback); FALSE: they give up
VARIABLES q,      \* queue length
          useful  \* requests in the queue whose client is still waiting (ghost)
vars == <<q, useful>>

TypeOK == q \in 0..Max /\ useful \in 0..q

\* Recovery from any state: every shocked state, trigger already over (no arrivals in Next).
Init == TypeOK

\* The server takes the oldest request; wasted copies are older than the useful ones.
Serve == q > 0 /\ q' = q - 1 /\ useful' = IF q > useful THEN useful ELSE useful - 1

\* A waiting client times out and sends again: its old copy is now wasted, the new copy is useful.
TimeoutRetry == Retry /\ q > T /\ useful > 0 /\ q < Max /\ q' = q + 1 /\ useful' = useful

\* With Retry = FALSE the client gives up instead: its copy becomes wasted.
TimeoutGiveUp == ~Retry /\ q > T /\ useful > 0 /\ q' = q /\ useful' = useful - 1

Next == Serve \/ TimeoutRetry \/ TimeoutGiveUp
Spec == Init /\ [][Next]_vars /\ WF_vars(Serve) /\ WF_vars(TimeoutRetry) /\ WF_vars(TimeoutGiveUp)

Healthy == q <= T
Recovers == <>[]Healthy
====
