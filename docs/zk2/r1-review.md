# Review of the r1 analysis (2026-10-07)

*This is the review the brief's author (a second model) wrote of the r1
analysis (`r1-analysis.md`, Part 2). It is recorded here in substance:
every ruling and finding is kept, and the wording is condensed. It matters
because it drove r2: the `zk2/` grammar major, exclusive-by-default ownership,
normative state timestamps, hash-verified contract retrieval and behavioural
profiles. r3 (`architecture.md`) later redesigned the data plane from
use cases.*

---

# Response to the Zenkey v2 architecture review

Thank you for the analysis. I think it materially improves the original redesign brief.

I am **not asking you to defend your proposal or to revert toward the original brief**. I want to use your §8 as the new starting point, with several architectural amendments before implementation.

My overall conclusion is:

> **Adopt §8 as the basis of Zenkey v2, but revise versioning, ownership/redundancy, state semantics, contract retrieval, and the boundary between contracts and behavioral profiles before freezing the design.**

The redesign is converging toward the right product:

> **a small, typed, introspectable API-contract and reflection layer over Zenoh, rather than an observability convention or a ROS-like middleware framework.**

## 1. Summary verdict

I agree with the central diagnosis that v1 is already specification-first; its problem is that the specification grew too large and too quickly because implementation lessons repeatedly became normative protocol.

I accept the main §8 architecture: logical `system/service` identity; API major as part of API identity; `stream`, `state`, and `@op`; per-API contracts; typed generated APIs; declaration-driven runtime exposure; liveliness-based discovery; instance descriptors; content-addressed contract/schema bundles; compatibility classification; optional profiles outside the core.

However, I want five significant changes:

1. **Keep a Zenkey grammar/protocol major in the keyspace**, distinct from API major.
2. **Exclusive ownership is the default, not a universal one-active-instance invariant.**
3. **State timestamp semantics must be normative and explicit.**
4. **Contract-by-hash retrieval must tolerate multiple valid responders; do not specify "exactly one reply".**
5. **Behavioral protocols such as configuration, redundancy and long-running operations need versioned profile specifications, not only ordinary contracts.**

With those changes, I would proceed to the spike.

## 2. Rulings on A1–A11

- **A1 — small spec, page budget, fixture per normative rule: AMEND.** The page budget should be a design pressure, not a normative constraint ("approximately 20–30 pages" is a useful target). Not literally every MUST can have a JSON fixture: *every machine-testable normative rule MUST have a conformance fixture or integration test*; behavioral/network requirements may require scenario tests.
- **A2 — presence belongs to an instance: ACCEPT.** The resource model remains stream, state, operation.
- **A3 — API major in the key; global convention version disappears: AMEND.** API version (`nav.v2`) and Zenkey grammar version (`zk2`) solve different problems; a v3 grammar could change identity ordering, control-plane layout, operation semantics, resource kinds or introspection layout without `nav.v2` becoming `nav.v3`. Proposed `zk2/<system>/<service>/<api>.v<major>/…`, `zk2/<system>/<service>/@zk/…`, `zk2/@zk/contract/…` — `zk2` an ordinary chunk, not verbatim.
- **A4 — exposure means declared capability: ACCEPT.** Declare resources → validate required exposure → declare descriptor/contracts → announce presence.
- **A5 — keep kind in the key: ACCEPT STRONGLY.** `@op` especially: discovery/snapshot selectors must not execute side-effecting operations.
- **A6 — bless Protobuf and JSON Schema: AMEND.** Asymmetric: Protobuf default/recommended for portable inter-service APIs; JSON Schema first-class for JSON/CBOR, configuration/document-shaped data and Rust-first use. Generated schemas are fine but the committed artifact is the protocol. Be careful with normalized Protobuf hashing: the canonical representation must include every compatibility/wire-relevant element; heavily fixture-tested.
- **A7 — "all resources of type Y" belongs to introspection: ACCEPT.**
- **A8 — validate on a real adopter: ACCEPT.**
- **A9 — spike before the full specification: ACCEPT.**
- **A10 — three crates rather than seven: ACCEPT.** `zenkey-model`, `zenkey`, `zenkey-build`; profiles become crates only with reusable runtime behavior.
- **A11 — standard long-running/configuration patterns: ACCEPT, WITH AN IMPORTANT CHANGE.** A configuration protocol (set → temporary application → confirmation deadline → confirm/rollback) is a state machine; a contract alone cannot define it. Introduce versioned behavioral profiles (`profiles/config/v1`, `profiles/jobs/v1`, `profiles/redundancy/v1`).

## 3. Rulings on B1–B8

- **B1 — ownership: ACCEPT THE GAP, AMEND THE SOLUTION.** "At most one active instance per service" is too restrictive. Ownership at the resource level: stream/state exclusive writer, operation exclusive responder by default; a contract may allow `serving = "replicated"` (with idempotency) for stateless operations; side-effecting replicated operations need fencing or external coordination.
- **B2 — parametric resources: ACCEPT STRONGLY.** Keep injective chunk encoding/decoding and fixtures.
- **B3 — wildcard GET reaching operations: ACCEPT STRONGLY.** A non-fanout operation MUST reject non-concrete invocation even if ACL fails to prevent it.
- **B4 — compatibility direction and transitivity: ACCEPT.** FULL_TRANSITIVE inside a major; otherwise `nav.v3`. Metadata compatibility is directional (`idempotent` true→false is worse than false→true; `reliable`→`sampled` differs from the reverse).
- **B5 — ACL constraints: ACCEPT.** Authorization stays outside the core; the grammar must be intentionally securable.
- **B6 — infrastructure selectability: ACCEPT.** Strengthens `system/service/api/kind/resource`.
- **B7 — measured cost model: ACCEPT.** Measure router RSS, declaration structures, wire traffic, reconnection bursts, CPU, convergence time — not raw key byte counts.
- **B8 — migration posture: ACCEPT.** Freeze v1 and maintain it while v2 is developed.

## 4. Major findings

- **F1 — missing Zenkey grammar major (MAJOR).** Use `zk2/<system>/<service>/<api>.vN/…`; do not depend on deployment namespaces as the only escape hatch.
- **F2 — single-active-service rule too restrictive (MAJOR).** Resources have an explicit ownership/serving model; exclusive is the default; minimally `exclusive` and `replicated` (operations only, initially).
- **F3 — state semantics underspecified (BLOCKER).** Make timestamps normative: every state mutation carries a Zenoh timestamp; GET responses preserve the mutation's timestamp; storage preserves it; deletion carries a timestamp; state GET explicitly selects target/consolidation; the spike tests HLC enabled/disabled and across routers. If this cannot be robust on stable APIs, reconsider producer+storage merging rather than weakening the semantics.
- **F4 — contract fetch must tolerate multiple holders/replies (MAJOR).** Keep `zk2/@zk/contract/<api>.v<major>/<sha256>`; 0 valid replies = unavailable; ≥1 = verify SHA-256, accept the first valid, duplicates harmless; retry with a broader target if appropriate. Correctness from content addressing, not routing uniqueness.
- **F5 — behavioral profiles need specifications (MAJOR).** Reject "only media and bulk need profile spec text"; consider `config.v1`, `jobs.v1`, `redundancy.v1`, `freshness.v1`, `media.v1`, `bulk.v1`, possibly alarms. Profiles stay outside the core and version independently.

## 5. Other design decisions

- **Presence layout:** agree with one token per (instance × API); test `…/@zk/alive/<api>.v<major>/<instance>/<fingerprint16>` (API first) against instance first; benchmark both. The 64-bit prefix is a drift hint; the descriptor stays authoritative.
- **Mandatory `system`:** keep it for now; define it as the logical deployment/ownership unit used as the outer application policy boundary, not the organisational hierarchy; test vehicle, ground, fleet-global, simulator and device-driver services; revisit if fake system names appear.
- **Fingerprint and annotations:** keep semantic annotations in the fingerprint (same fingerprint = exactly the same machine-relevant contract); documentation stays outside.

## 6. Recommended answers to the seven maintainer decisions

1. **ZenSight:** freeze v1 at 1.50/0.14.x; porting ZenSight is not a prerequisite; port tcgui or another small adopter first.
2. **Mandatory `<system>`:** yes, provisionally.
3. **Kind in the key:** yes (`stream`, `state`, `@op`).
4. **Default schema kind:** Protobuf-first; JSON Schema first-class; not conceptually identical.
5. **Single active writer:** no as a universal rule — exclusive resource ownership by default plus explicit replicated serving (operations initially).
6. **Repository / naming:** same repository; a `v1` branch/tag as the frozen maintenance line, `main` for the new architecture; 1.0 only once the core protocol/spec is intentionally stable.
7. **Pilot:** a real, non-safety-critical service with at least one stream, state, operation and parametric resource, restart/reconnect behavior, and more than one machine if practical.

## 7. Changes to the spike

Add: liveliness scaling at 100/1,000/10,000/50,000 tokens (router RSS, CPU, propagation bytes, discovery latency, router restart, reconnect convergence, churn, handler backpressure — do not extrapolate memory from key length); constrained networks (RTT, bandwidth, loss, reconnect); contract serving (one/many/slow/unreachable/corrupt holders, topologies); state correctness (producer/storage/both/stale/restart/delete/wildcard GET/HLC on/off); ownership and redundancy (exclusive + standby, two active writers, replicated stateless operation, failover, slow disappearance, partition/rejoin); an API-compatibility fixture matrix with explicit direction.

## 8–9. Revised target and next task

Converge on `<namespace>/zk2/<system>/<service>/<api>.v<major>/{stream,state,@op}/…`, `…/@zk/{alive,instance}/…`, `zk2/@zk/contract/<api>.v<major>/<sha256>` — static contract (identity, templates, schemas, serving, metadata, annotations, fingerprint), runtime descriptor (instance, contracts, exposure, unavailable optional resources, metadata), liveliness for cheap selective discovery, and profiles outside the core. Produce a revised architecture proposal, focused on unresolved design decisions rather than more features; prefer an existing Zenoh primitive plus a small semantic rule over another Zenkey abstraction.
