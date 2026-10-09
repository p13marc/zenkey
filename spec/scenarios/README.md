# zk2 scenarios

Network rules of [`../core.md`](../core.md) that a static fixture cannot
check. Each scenario has a **setup**, **steps** and **expected**
observations, in the shape the spike ran them (`docs/zk2/spike-report.md`).
The spike is cited where one measured the behaviour. Version 0.1 of the core
is accepted with these scenarios written; their automated runner comes with
the runtime (#610 onward).

| File | Rules |
|---|---|
| [`grammar.md`](grammar.md) | §1.3 the guard, §1.6 namespaces |
| [`state.md`](state.md) | §4: S1–S7, archives, events |
| [`operations.md`](operations.md) | §5: O1–O7; §6 split-brain |
| [`presence.md`](presence.md) | §1.5 epochs, §3.2 an unbound required role, §3.3 the descriptor, §8.1–§8.2 |
| [`bindings.md`](bindings.md) | §3.2: R1–R6 |
| [`retrieval.md`](retrieval.md) | §8.4 |
| [`types.md`](types.md) | §2.4 QoS, §7.1–§7.2 |
| [`security.md`](security.md) | §11; §4.2's admin space |
| [`constrained.md`](constrained.md) | §1.6, R7, §8.5, §12 |

Conventions: R1 and R2 are routers, linked R2 → R1 unless a scenario says
otherwise. "Owner", "consumer" and "caller" are client sessions. Timeouts
are 1 s unless stated.
