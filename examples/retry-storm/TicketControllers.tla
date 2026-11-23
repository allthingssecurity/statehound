---- MODULE TicketControllers ----
(* An execution control system (after Demirbas et al., ICCCN 2026). One CPU does Cpu units of work per
   window. A ticket lets a task occupy a thread; tasks share the CPU equally. Short tasks need 1 unit to
   finish; the long queue is always deep. Mode "rate": each queue's controller adds a ticket when that raises
   its ticket-acquisition rate. A ticket bounds wall-clock time, not CPU, so every ticket of a deep queue is
   acquired once per window whatever the CPU share: the metric measures churn. Mode "fixed": the long queue
   keeps LongFixed tickets and only the short controller acts, on an honest signal (short tasks completed). *)
EXTENDS Naturals
CONSTANTS Cap, Cpu, MaxW, Arr, Mode, LongFixed
VARIABLES tS, tL, waitS, trigger
vars == <<tS, tL, waitS, trigger>>

Min(a, b) == IF a < b THEN a ELSE b
\* short tasks completed per window: each needs 1 unit, the CPU is shared by every ticket holder
Useful(s, l, w) == IF s + l <= Cpu THEN Min(s, w) ELSE 0
\* ticket acquisitions per window (what the "rate" controllers watch)
AcqS(s, w) == Min(s, w)
AcqL(l) == l

TypeOK == tS \in 1..Cap /\ tL \in 1..Cap /\ waitS \in 0..MaxW /\ trigger \in {"on", "off"}

\* every shocked state, trigger already over; in mode "fixed" the long queue holds its fixed tickets
Init == TypeOK /\ trigger = "off" /\ (Mode = "fixed" => tL = LongFixed)

Window == /\ waitS' = Min(MaxW, waitS - Useful(tS, tL, waitS) + IF trigger = "on" THEN Arr ELSE 0)
          /\ UNCHANGED <<tS, tL, trigger>>

\* "rate" controllers: keep a probe that raised their own acquisition rate
LongProbe == Mode = "rate" /\ tL < Cap /\ AcqL(tL + 1) > AcqL(tL) /\ tL' = tL + 1 /\ UNCHANGED <<tS, waitS, trigger>>
ShortProbe == Mode = "rate" /\ tS < Cap /\ AcqS(tS + 1, waitS) > AcqS(tS, waitS)
              /\ tS' = tS + 1 /\ UNCHANGED <<tL, waitS, trigger>>

\* "fixed": the short controller adds a ticket when that completes more short tasks, and sheds one while short
\* tasks wait and none completes
ShortHonest == Mode = "fixed" /\ waitS > 0
               /\ \/ tS < Cap /\ Useful(tS + 1, tL, waitS) > Useful(tS, tL, waitS) /\ tS' = tS + 1
                  \/ tS > 1 /\ Useful(tS, tL, waitS) = 0 /\ tS' = tS - 1
               /\ UNCHANGED <<tL, waitS, trigger>>

Ctrl == LongProbe \/ ShortProbe \/ ShortHonest
Next == Window \/ Ctrl
Spec == Init /\ [][Next]_vars /\ WF_vars(Window) /\ WF_vars(Ctrl)

Healthy == waitS = 0
Recovers == <>[]Healthy
\* a controller step never lowers the true progress (short completions per window)
HonestMetric == [][(tS' # tS \/ tL' # tL) => Useful(tS', tL', waitS) >= Useful(tS, tL, waitS)]_vars
\* the limits eventually stop moving
Settles == <>[][UNCHANGED <<tS, tL>>]_vars
====
