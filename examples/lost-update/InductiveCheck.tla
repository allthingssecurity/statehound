--------------------------- MODULE InductiveCheck --------------------------
(* Checks whether an invariant of LockedUpdate is INDUCTIVE: true in every *)
(* state where it holds, it stays true after any one step. The initial     *)
(* states are ALL type-correct states satisfying the candidate invariant,  *)
(* reachable or not, so a pass covers every state a proof must consider    *)
(* (for these constants).                                                   *)
EXTENDS LockedUpdate

TypeStates == /\ counter \in 0..Cardinality(Procs)
              /\ tmp  \in [Procs -> 0..Cardinality(Procs)]
              /\ pc   \in [Procs -> {"read", "write", "done"}]
              /\ lock \in Procs \cup {Free}

IndInit(I) == TypeStates /\ I

IndSpecStrong == IndInit(Inv) /\ [][Next]_vars
IndSpecWeak   == IndInit(TypeOK /\ NoLostUpdate) /\ [][Next]_vars

WeakInv == TypeOK /\ NoLostUpdate
=============================================================================
