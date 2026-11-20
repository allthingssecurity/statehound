--------------------------- MODULE LockedUpdate ---------------------------
(* The fix for LostUpdate: a process takes a lock before it reads and gives *)
(* it back after it writes, so no other process can run between the two.   *)
EXTENDS Naturals, FiniteSets

CONSTANTS Procs, Free     \* Free: a model value meaning "nobody holds the lock"

VARIABLES counter, tmp, pc, lock

vars == <<counter, tmp, pc, lock>>

Init == /\ counter = 0
        /\ tmp  = [p \in Procs |-> 0]
        /\ pc   = [p \in Procs |-> "read"]
        /\ lock = Free

Read(p)  == /\ pc[p] = "read"
            /\ lock = Free
            /\ lock' = p
            /\ tmp' = [tmp EXCEPT ![p] = counter]
            /\ pc'  = [pc  EXCEPT ![p] = "write"]
            /\ UNCHANGED counter

Write(p) == /\ pc[p] = "write"
            /\ counter' = tmp[p] + 1
            /\ pc'   = [pc EXCEPT ![p] = "done"]
            /\ lock' = Free
            /\ UNCHANGED tmp

Next == \E p \in Procs : Read(p) \/ Write(p)

Spec == Init /\ [][Next]_vars

TypeOK == /\ counter \in 0..Cardinality(Procs)
          /\ pc \in [Procs -> {"read", "write", "done"}]
          /\ lock \in Procs \cup {Free}

AllDone == \A p \in Procs : pc[p] = "done"

NoLostUpdate == AllDone => counter = Cardinality(Procs)

(* An inductive invariant: strong enough that it holds initially and every  *)
(* step preserves it, which is what a proof needs (see docs/proofs.md).     *)
Inv == /\ TypeOK
       /\ counter = Cardinality({p \in Procs : pc[p] = "done"})
       /\ \A p \in Procs : pc[p] = "write" => (lock = p /\ tmp[p] = counter)
       /\ lock # Free => pc[lock] = "write"
=============================================================================
