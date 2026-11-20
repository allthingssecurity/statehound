---------------------------- MODULE LostUpdate ----------------------------
(* Two processes each add 1 to a shared counter by reading it into a local  *)
(* register and writing back register + 1. The read and the write are two   *)
(* separate steps, so the steps of the two processes can interleave.        *)
EXTENDS Naturals, FiniteSets

CONSTANT Procs

VARIABLES counter,  \* the shared counter
          tmp,      \* tmp[p]: the value process p read
          pc        \* pc[p]: where process p is: "read", "write" or "done"

vars == <<counter, tmp, pc>>

Init == /\ counter = 0
        /\ tmp = [p \in Procs |-> 0]
        /\ pc  = [p \in Procs |-> "read"]

Read(p)  == /\ pc[p] = "read"
            /\ tmp' = [tmp EXCEPT ![p] = counter]
            /\ pc'  = [pc  EXCEPT ![p] = "write"]
            /\ UNCHANGED counter

Write(p) == /\ pc[p] = "write"
            /\ counter' = tmp[p] + 1
            /\ pc' = [pc EXCEPT ![p] = "done"]
            /\ UNCHANGED tmp

Next == \E p \in Procs : Read(p) \/ Write(p)

Spec == Init /\ [][Next]_vars

(* Invariants: properties that must hold in every reachable state. *)
TypeOK == /\ counter \in 0..Cardinality(Procs)
          /\ pc \in [Procs -> {"read", "write", "done"}]

AllDone == \A p \in Procs : pc[p] = "done"

\* Once every process has finished, every increment must be in the counter.
NoLostUpdate == AllDone => counter = Cardinality(Procs)
=============================================================================
