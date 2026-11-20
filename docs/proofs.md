# Proofs and model checking

A model checker answers "does this property hold in every reachable state, **for these constants**?"
A proof answers "does it hold **for every** value of the constants?". This page shows the difference on
the lock example and shows how a model checker helps build a proof. All outputs below are real runs of the
files in [`examples/lost-update/`](../examples/lost-update).

## What model checking established

[`LockedUpdate.tla`](../examples/lost-update/LockedUpdate.tla) passes with `Procs = {p1, p2}`:

```text
Model checking completed. No error has been found.
9 states generated, 9 distinct states found, 0 states left on queue.
```

That is a complete check of the two-process system: all 9 reachable states satisfy `NoLostUpdate`. It
says nothing about 50 processes. Checking larger sets of processes gives more confidence, but the state
count explodes (see [how-model-checking-works.md](how-model-checking-works.md#7-state-explosion)), and no
finite number of checks covers every size.

## What a proof needs

To prove that `NoLostUpdate` holds in every reachable state of every instance, the standard method is an
**inductive invariant** `Inv` with three properties:

1. **Initiation**: `Init => Inv`. Every initial state satisfies it.
2. **Consecution**: `Inv /\ Next => Inv'`. From *any* state satisfying it (reachable or not), every step
   leads to a state satisfying it.
3. **Sufficiency**: `Inv => NoLostUpdate`. It implies the property you care about.

1 and 2 together say, by induction on the length of a behaviour, that every reachable state satisfies
`Inv`, and 3 turns that into the property.

The catch is in 2: it quantifies over **all** states satisfying `Inv`, including unreachable ones. The
property you want is almost never inductive by itself.

## Seeing why with a model checker

[`InductiveCheck.tla`](../examples/lost-update/InductiveCheck.tla) turns the consecution check into a model
checking problem. Its initial states are **all** type-correct states satisfying a candidate invariant,
not just the real initial state:

```tla
TypeStates == /\ counter \in 0..Cardinality(Procs)
              /\ tmp  \in [Procs -> 0..Cardinality(Procs)]
              /\ pc   \in [Procs -> {"read", "write", "done"}]
              /\ lock \in Procs \cup {Free}

IndSpecWeak   == (TypeStates /\ TypeOK /\ NoLostUpdate) /\ [][Next]_vars
IndSpecStrong == (TypeStates /\ Inv)                    /\ [][Next]_vars
```

If the candidate is inductive, no step from any of those states can break it.

### The property alone is not inductive

```sh
everystate -config InductiveWeak.cfg InductiveCheck.tla
```

```text
Error: Invariant WeakInv is violated.
Error: The behavior up to this point is:
State 1: <Initial predicate>
/\ counter = 0
/\ lock = p1
/\ pc = (p1 :> "write" @@ p2 :> "done")
/\ tmp = (p1 :> 0 @@ p2 :> 0)

State 2: <Write(p1) line 24, col 13 to line 28, col 28 of module LockedUpdate>
/\ counter = 1
/\ lock = Free
/\ pc = (p1 :> "done" @@ p2 :> "done")
/\ tmp = (p1 :> 0 @@ p2 :> 0)
```

This is a **counterexample to induction** (a CTI). State 1 satisfies `NoLostUpdate` (not everyone is done
yet, so the implication is trivially true), but one step breaks it. State 1 is **unreachable**: `p2` is
done, yet the counter is still 0. The real system can never be there, but the weak invariant does not
rule it out, so it cannot be used in a proof.

The CTI tells you what to add: in every reachable state, the counter equals the number of finished
processes, and a process between read and write holds the lock and has read the current counter.

### The strengthened invariant is inductive

```tla
Inv == /\ TypeOK
       /\ counter = Cardinality({p \in Procs : pc[p] = "done"})
       /\ \A p \in Procs : pc[p] = "write" => (lock = p /\ tmp[p] = counter)
       /\ lock # Free => pc[lock] = "write"
```

```sh
everystate -config InductiveStrong.cfg InductiveCheck.tla
```

```text
Finished computing initial states: 48 distinct states generated.
Model checking completed. No error has been found.
96 states generated, 48 distinct states found, 0 states left on queue.
```

All 48 type-correct states satisfying `Inv` stay inside `Inv` after any step: consecution holds, for two
processes. Initiation and sufficiency are easy to check by hand (initially nobody is done and the counter
is 0; when everyone is done, the counter is the number of processes). Both runs give identical results
in Java TLC, which was used as an oracle.

This loop (propose an invariant, let the checker find a CTI, strengthen, repeat) is how inductive
invariants are found in practice. Tools such as Apalache and TLAPS build on the same idea.

## The proof itself

With `Inv` found, a mechanical proof for **any** finite set of processes is short. A sketch in TLAPS
syntax (the TLA+ Proof System; this sketch was not machine-checked here, because TLAPS is not part of
this toolchain):

```tla
THEOREM Spec => []NoLostUpdate
<1>1. Init => Inv
  BY DEF Init, Inv, TypeOK
<1>2. Inv /\ [Next]_vars => Inv'
  <2> SUFFICES ASSUME Inv, [Next]_vars PROVE Inv'  OBVIOUS
  <2>1. ASSUME NEW p \in Procs, Read(p)   PROVE Inv'
        \* lock was Free, so nobody was writing; p is now the only writer and tmp[p] = counter
  <2>2. ASSUME NEW p \in Procs, Write(p)  PROVE Inv'
        \* p held the lock and tmp[p] = counter, so counter' = counter + 1 = |done'|
  <2>3. CASE UNCHANGED vars  BY DEF vars, Inv
  <2> QED  BY <2>1, <2>2, <2>3 DEF Next
<1>3. Inv => NoLostUpdate
  BY DEF Inv, NoLostUpdate, AllDone
<1> QED  BY <1>1, <1>2, <1>3, PTL DEF Spec
```

The model checker did the hard creative part (finding the invariant and confirming it is inductive on a
small instance), and the proof generalizes it to every instance.

## Which one to use

| | Model checking | Proof |
|---|---|---|
| Covers | every reachable state, for the chosen constants | every instance |
| Effort | write the spec and a config, press a button | find an inductive invariant, write the proof |
| Output on failure | a concrete counterexample | a proof obligation that will not go through |
| Typical use | finding bugs, quickly and automatically | certifying a design once it has stopped changing |

Bug-hunting pipelines, including the one Statehound plugs into, use model checking: they need concrete
counterexamples that agents can turn into failing tests against real code.
