# zk2 scenarios

Network rules of [`../core.md`](../core.md) that a static fixture cannot
check. Each scenario has a **setup**, **steps** and **expected**
observations, in the shape the spike ran them (`docs/zk2/spike-report.md`).
The spike is cited where one measured the behaviour. The core was accepted
with these scenarios written, and the reference runtime's tests run them.

| File | Rules |
|---|---|
| [`grammar.md`](grammar.md) | §1.3 the guard, §1.6 namespaces |
| [`state.md`](state.md) | §4: S1–S7 over runs and seqs, archives, events, a late joiner's seed and its merge's open cases, S4's check reads beside a router that is declaring, and a stream key's seq wrapping; §8.2's session posture |
| [`operations.md`](operations.md) | §5: O1–O7; §6 split-brain |
| [`presence.md`](presence.md) | §1.5 epochs, §3.2 an unbound required role, §3.3 the descriptor, §8.1–§8.2 |
| [`bindings.md`](bindings.md) | §3.2: R1–R6, a binding's order kept |
| [`retrieval.md`](retrieval.md) | §8.4 |
| [`types.md`](types.md) | §2.4 QoS, §7.1–§7.2 |
| [`security.md`](security.md) | §11, a tool's inspection; §4.2's admin space; the admin read admitting no GET of a node's declaration records |
| [`constrained.md`](constrained.md) | §1.6, R7, §8.5, §12; §8.1, §8.2 and §8.4 for an owner on a first hop |
| [`budget.md`](budget.md) | §2.7: what a path drops and a reader sees, a stalled reader; an owner keeping its bound and its rate |
| [`frames.md`](frames.md) | §4.3, §7.7: an owner's frames, their seqs, keyframes, delta records, deadbands and times; a reader's field states at a gap, a new run and a late join |

The reader's rules among them are
[`../observer.md`](../observer.md)'s: a scenario's core section leads there
by its pointer, and observer.md's Appendix B indexes the evidence.

Conventions: R1 and R2 are routers, linked R2 → R1 unless a scenario says
otherwise. "Owner", "consumer" and "caller" are client sessions. Timeouts
are 1 s unless stated.
