# Review request: the Zenkey v2 analysis

**To:** the author of the brief (`brief.md`)

**From:** Claude, the model that analysed the brief inside the zenkey
repository.

**This file is self-contained.** Part 1 is the review request. Part 2 is my
full analysis.

**References.** Bare section numbers such as "§8.4" refer to Part 2.
"Brief §N" refers to your brief. Its sections, so the references resolve
even if you no longer have the brief in context:

1. Strategic goal
2. Mental model
3. Specification first
4. Core vs optional
5. Keyspace from first principles
6. Semantic resource model
7. Registry redesign
8. Type and schema model
9. Runtime introspection
10. Discovery and liveliness
11. Compatibility model
12. Generated Rust API
13. Do not centralize
14. Security boundary
15. Things to remove from the core
16. Repository shape
17. Conformance tests
18. Performance and scaling tests
19. Validation domain (navigation, health, hardware driver)
20. ROS 2
21. Raw Zenoh
22. Prior art
23. Documentation structure
24. Governance
25. Implementation plan (phases 1–8)
26. Permission to break things
27. Things to avoid
28. Success criteria
29. Deliverables
30. Final instruction

---

# Part 1: Review request

## What I am asking

You wrote the brief. I analysed it against the actual repository (the RFC
set at v1.50, the workspace at 0.14.0, zenoh 1.10) and against prior art,
and I disagree with parts of it. Please review my analysis as an
**adversarial reviewer**. Your job is not to defend the brief, and not to
agree with me. The maintainer makes the decision; your review should make
that decision better informed.

Please do five things:

1. **Find the errors in my analysis.** Look for:
   - factual mistakes;
   - internal contradictions;
   - designs that would not work on Zenoh;
   - claims that go further than their evidence.
2. **Rule on each of my challenges to your brief** (list A) and each gap I
   claim it has (list B). For each one: accept, reject or amend, with the
   reason.
3. **Stress-test the proposed design (§8)** against:
   - your own success criteria (brief §28);
   - your validation domains (brief §19): navigation, health, and a
     hardware driver with multiple devices.
4. **Attack the points where I am least confident first** (list C).
5. **Recommend an answer** for each of the seven maintainer decisions
   (§11).

**Ground rules:**

- **Wrong versus weighed differently.** Say whether a point "is wrong" or
  whether you "would weigh the trade-off differently". Both are useful,
  but they are not the same.
- **Concrete alternatives.** When you disagree, propose the alternative as
  a key shape, a rule or an API, not just the objection.
- **No re-litigating the agreed points.** Leave what we already agree on
  (§3) alone unless you have found a new reason.
- **Unverified facts stay unverified.** If you cannot verify a fact, say
  so; do not assume it either way.
- **Sourced facts need contrary evidence.** The facts below were verified
  in the source I name. Challenge any of them if you have evidence against
  it, and cite that evidence.

---

## Context you probably don't have

### The repository (read directly)

**The RFC set**

- 13 chapters, about 8,000 lines, plus a 1,900-line changelog.
- v1.0 was 2026-07-12 and v1.50 was 2026-10-05: 50 amendments in 85 days.
- It uses RFC 2119 roles, theorem/precondition design properties pinned by
  guard tests, and byte-precise derivations with test vectors.

**The key grammar**

- The shape is `<base>/v1/<origin>/<class>/<producer>/<subject...>`.
- `<origin>` is either `h-` plus 12 hex characters of sha256(machine-id +
  salt), which is the *physical machine*, or a verbatim service origin
  such as `@catalog`.
- `<class>` is one of `telemetry`, `state` or `events`, or one of the
  verbatim planes `@rpc`, `@media` or `@blob`.
- `<base>` is the optional zenoh session namespace.
- RFC 02 P11 justifies this chunk order "for an observability fleet".

**The registry**

- There is one TOML (or KDL) file per *producer*, not per API. It declares
  subjects with `{var}`/`{var...}` templates, procedures, media, blob tiers
  and errors.
- Two locks back it: `registry.lock` (backward-compatibility pins) and an
  append-only `deprecated.lock`.
- `when` predicates mark conditional surfaces.

**Introspection**

- Each producer serves `@rpc/<producer>/introspect`, which is its registry
  file compiled in with `include_str!` and served **verbatim**.
- `describe` serves a SchemaSet. Its JSON Schemas are generated **at run
  time from Rust types** through schemars. For protobuf, the hash covers
  the raw descriptor bytes.
- Presence is the liveliness token `v1/<origin>/state/<producer>/alive`.
- The roster and the introspect sweep are two independent wildcard
  fan-outs.

**The runtime**

- The runtime crate builds *keys*. Applications declare publishers and
  queryables with raw zenoh, so zenkey never sees a declaration.
- Truthfulness ("the registry MUST NOT lie", RFC 08 §6.1) is enforced by
  build-time coverage checks plus a startup comparison for procedures.

**Overfit**

- The "application-neutral" runtime crate ships ZenSight's alert-key
  recipe, ZenSight's `errors` token and the catalog subjects.
- The neutral chapters specify a catalog, evidence fusion, incidents,
  acknowledgements, silences and impact edges.
- A CLI's exit codes are normative text (RFC 13 §1.2).

**Size and adopters**

- The codebase is about 180k lines of Rust. The convention itself is about
  22k; tooling is about 150k (the fleet engine, a CLI, an Iced GUI and a
  notifier). Much of the tooling is generic, meaning it works on any
  Zenoh bus.
- The adopters are ZenSight and tcgui, and the CLI runs in production.
- ZenSight's 12 registry files total 105 KB.

**Lessons v1 paid for**

- In v1.0 the version chunk was verbatim (`@v1`). That silently broke
  zenoh-ext's advanced-publisher detection, because its token parser
  cannot capture across `@`, and v1.1 reverted it.
- The fan-in discipline: target `All`, reply on your own concrete key,
  consolidation `None`, never declare a fan-in queryable `complete`.
- ACL deny works by inclusion, so a deny rule never stops a query broader
  than itself, and servers must refuse broadcast writes. Interest is
  evaluated on egress.

### Zenoh, read in the 1.10.1 source

Several of my design choices depend on these facts:

- **Liveliness tokens carry no payload.** Everything is in the key.
- **Every router holds every token.** Clients and peers receive a token
  only if they registered a matching interest (declarations on demand).
- **Verbatim chunks.** A verbatim chunk is one that *starts* with `@`, and
  `*`/`**` never match it. `nav@2` is an ordinary chunk.
- **`BestMatching`** sends the query to the **nearest `complete`
  queryable whose key includes the query's key**. If there is none, it
  falls back to `All`. Consolidation happens only in the querying session.
- **Encoding schema suffix.** It is capped at 255 bytes and sent with
  *every* non-default put. There is no declare-once optimization.
- **ACL limits.**
  - Matching is by inclusion.
  - **Multicast transports bypass ACL.**
  - `zids` subjects are not authenticated.
  - Since 1.3, a query refused by ACL gets an *empty* reply.
- **Per-sample attribution.** `SourceInfo` is unstable, unvalidated, and
  set only by advanced publishers. `replier_id` is unstable. Matching
  status is a boolean only.
- **Advanced pub/sub** is unstable, and its wire format is defined only by
  the Rust source.
- **An open deadlock.** zenoh#2678 (open): `liveliness_query` deadlocks
  past about 256 matching tokens on the default bounded handler.
- **rmw_zenoh** makes every process liveliness-get and history-subscribe
  the whole domain.

---

## A. My challenges to your brief: rule on each

| # | Challenge | Section |
|---|---|---|
| A1 | v1 is already spec-first. The obstacle is the spec's size and churn, so v2 needs a page budget and a fixture per normative rule. | §1.4, §4.1 |
| A2 | Presence belongs to an instance, not to an API as a resource kind. | §4.2 |
| A3 | The API major goes in the key (`nav.v2`, your Model C). v1's global convention-version chunk is dropped. | §4.3 |
| A4 | Exposure means declared capability (handles obtained before `start()`), not observed traffic. | §4.4 |
| A5 | The kind stays in the key (`stream`/`state`/`@op`) even though telemetry semantics leave the core. | §4.5 |
| A6 | Bless protobuf and JSON Schema. Schemas are committed artifacts. Hashes are over normalized descriptions, serialized with JCS. | §4.6 |
| A7 | "All resources of type Y" is answered by introspection, never by the key. | §4.7 |
| A8 | Validate on a real adopter (tcgui, then ZenSight) as well as on neutral examples. | §4.8 |
| A9 | A measured spike comes before the nine documents, and the deliverables fold into four. | §4.9 |
| A10 | Three core crates, not seven. | §4.10 |
| A11 | Write down the long-running and configuration patterns, as standard contracts. | §4.11 |

## B. Gaps I claim the brief has: confirm or refute

| # | Gap | Section |
|---|---|---|
| B1 | Who owns a key once identity is logical. Proposed core rule: one active writer per service. | §5.1 |
| B2 | Parametric resources (collections, devices). | §5.2 |
| B3 | Wildcard GETs that reach side-effecting operations. | §5.3 |
| B4 | Compatibility direction and transitivity (FULL_TRANSITIVE within a major). | §5.4 |
| B5 | The ACL facts that constrain any keyspace. | §5.5 |
| B6 | Infrastructure selectability (storage, QoS overwrite, link budgets). | §5.6 |
| B7 | A measured cost model of what propagates where. | §5.7 |
| B8 | A migration posture for the existing adopters. | §5.8 |

---

## C. Where I am least confident: attack these first

- **C1. A mandatory single-chunk `<system>`** (§8.2).
  - Does one chunk, plus the namespace, cover deeper hierarchies
    (fleet / site / vehicle / subsystem)?
  - Does it cover services that no system owns?
- **C2. The kind chunk plus a verbatim `@op`** (§4.5, §8.4).
  - Is it worth the extra key depth and the extra ACL rules?
  - The alternative is storage and ACL configs generated from contracts.
- **C3. "One active writer per service" as a core rule** (§5.1, §8.13).
  - It excludes active-active, stateless serving of operations.
  - Is that acceptable for the first core version?
- **C4. One token per (instance × API), with a 64-bit contract
  fingerprint in the key** (§8.7).
  - How much router state does that cost at scale?
  - How much churn when contracts change at run time?
  - How does it interact with zenoh#2678?
  - The alternative is one token per instance, with the APIs in the
    descriptor.
- **C5. Location-free contract keys** (`@zk/contract/<api>.v<major>/<sha256>`,
  with `complete` queryables, fetched with `BestMatching`) (§8.7).
  - Look for failure modes: namespaces, ACL per API, a holder serving a
    corrupt bundle (the client must verify the hash), a nearest holder
    behind a slow link.
- **C6. Dropping the global convention-version chunk** (§4.3). This
  reverses v1's principle P4. Is the deployment namespace a sufficient
  escape hatch for a future grammar break?
- **C7. FULL_TRANSITIVE inside a major** (§8.9).
  - Is it too strict for real evolution?
  - Is "enum addition = review" right?
- **C8. Annotations inside the fingerprint** (§8.5). An annotation-only
  change then changes contract identity. Is that what we want?
- **C9. "Profiles are mostly standard contracts"** (§8.12).
  - Does `config.v1`'s commit-confirm protocol (tokens, confirm windows,
    hot versus reach groups) really reduce to a contract?
  - If it needs normative *behavioural* text, then my claim that "only
    media and blob need spec chapters" is false. Which other profiles
    break it?
- **C10. State served by the producer's own queryable** (§8.3). It is
  non-`complete`, and replies are consolidated by timestamp.
  - Is that the right default compared with zenoh-ext's advanced-publisher
    cache?
  - What happens when HLC timestamping is off?
- **C11. Protobuf-first versus Rust ergonomics** (§4.6). Is "JSON Schema
  generated by schemars, then committed" a genuine equal, or a trap?
- **C12. Exposure from declared handles** (§4.4, §8.8). Does "declared but
  idle" leave a hole, such as a handle that is declared but can never
  publish? Do dynamic modules (descriptor `generation`) stay truthful?

## D. Specific checks

- **Keyspace scenarios.** Does K2 (§8.4) really satisfy each scenario of
  brief §25 Phase 3 as the table claims? Is K3 (API-first) dismissed too
  quickly?
- **Internal consistency hotspots I know of.**
  - Position 4 is "exactly `stream`, `state` or `@op`", yet profiles
    register `@media`/`@blob` at the same position.
  - `@zk` appears at both position 3 and position 1.
  - State queryables are "never `complete`" while contract queryables are
    "always `complete`". Does the reasoning hold for both?
  - The ACL table (§8.11) must match the key placement (§8.7).
- **Cost estimate.** Is §8.7's figure plausible: about 10,000 tokens of
  about 75 bytes, versus about 36 MB per v1 inventory query? What is
  missing from it?
- **Compatibility table.** Is the table in §8.9 complete and correctly
  classified? I am thinking of operation request versus response
  directions, parameter types, and `delivery` changes.
- **Rust sketch.** Is the sketch (§8.10) sound with respect to lifetimes,
  async and borrowing? The server handles borrow the service that
  `start()` consumes.
- **Inventory.** Is anything in the inventory (§9) misclassified?

---

## Response format

1. **Summary verdict**, in 10 lines or fewer. Would you adopt §8 as the
   basis for the core spec? With what changes?
2. **Rulings on A1–A11 and B1–B8.** One line each: accept, reject or
   amend, plus the reason.
3. **Findings, most severe first.** For each one, give:
   - an id, the section, a severity (blocker, major or minor) and a type
     (error, risk, trade-off or gap);
   - the claim under review, quoted or paraphrased;
   - the problem;
   - a concrete alternative;
   - your confidence (high, medium or low), and what would change your
     mind.
4. **The seven decisions** (§11). Your recommendation and a one-line
   reason for each.
5. **The spike** (§10, phase 1). What would you measure that I have not
   listed?

Keep it under about 2,000 words. Depth on the top findings beats coverage
of minor ones.

---

# Part 2: The analysis

## Zenkey v2: an independent analysis of the redesign brief

*2026-10-07. Inputs: the brief (`brief.md`), the RFC set at v1.50, the
workspace at 0.14.0 (zenoh 1.10), and external research on Zenoh and prior
art (sources at the end). This is an analysis, not a spec. It says where the
brief is right, where I would change it, what it misses, and what I would
build. Section numbers like "brief §9.2" refer to the brief; "RFC 08 §6.1"
refers to the current RFC set.*

---

### 0. The verdict on one page

**The direction is right. The diagnosis is half right.**

The brief asks for a small, implementation-independent API-contract and
reflection layer over Zenoh. For general company software (navigation,
control, mission, payloads, health), that is the right product, and the
current convention is not that product. v1 is the keyspace of a **fleet
observability product**, generalized outward:

- its identity is the physical machine (`h-<12hex>`, salted machine-id);
- its semantics are telemetry, state and events;
- its "application-neutral" chapters carry an entity catalog, evidence
  fusion, alerts, incidents, acknowledgements, silences, impact edges and a
  CLI's exit codes.

RFC 02 P11 says so in plain words: *"For an observability fleet that is the
origin."*

The brief misreads *why* v1 has the wrong shape, though. v1 is not "a Rust
crate first". It is unusually spec-first: 13 chapters, about 8,000 lines,
RFC 2119 roles, theorem/precondition splits pinned by guard tests,
byte-precise derivations with test vectors, and a decision record. The real
problem runs the other way: **the spec is too large and changes too fast to
be a foundation.** It went through 50 amendments in the 85 days from v1.0
(2026-07-12) to v1.50 (2026-10-05), one every 1.7 days, because every
implementation lesson was promoted to normative text. "Specification first"
is necessary but not enough. v2 needs a *small spec that rarely changes*.

**The six changes to the brief that matter most** (§4 has all eleven):

1. **Presence is not an API resource kind.** It belongs to an *instance*.
2. **The API major goes in the key** (Model C). Zenoh pub/sub has no
   negotiation, and a fleet whose members can be offline cannot do a flag
   day. What should leave the key is v1's global convention-version chunk.
3. **"Introspection derived from runtime registration" needs a sharper
   definition.** v1 already learned (RFC 08 §6.1) that observed
   publication cannot tell an idle host from a registry that lies.
   Exposure must mean *declared capability* (a handle exists), not
   *observed traffic*.
4. **Keep the resource kind in the key.** Taking telemetry/state/events
   out of the core must not take kind out of the key. Storage selection,
   router QoS pinning, read-versus-act ACLs and wildcard-GET safety all
   select on it by prefix.
5. **Bless two schema kinds.** A schema-provider plugin model with no
   default leaves "generic decode" and "type compatibility" with nothing
   behind them. Bless protobuf and JSON Schema; everything else is an
   extension.
6. **Spike before documents.** Nine documents before any code invites a
   paper architecture. Measure first.

**What the brief misses**, chiefly (§5 has all eight):

- who owns a key once identity is logical (single writer);
- parametric resources, which the hardware-driver test needs;
- wildcard GETs that reach side-effecting operations;
- the *direction* and *transitivity* of compatibility;
- the hard-won ACL facts (deny-by-inclusion, egress interest evaluation)
  that constrain any keyspace.

**What I would build** (§8) starts with one grammar:

```
<system>/<service>/<api>.v<major>/stream/<resource…>     pushed values
<system>/<service>/<api>.v<major>/state/<resource…>      latest value, answerable on GET
<system>/<service>/<api>.v<major>/@op/<operation…>       request/reply (verbatim: no wildcard reaches it)
<system>/<service>/@zk/{alive,instance}/…                 presence tokens, instance descriptors
@zk/contract/<api>.v<major>/<sha256>                      contract bundles: content-addressed, location-free
```

On that grammar:

- three resource kinds: stream, state, operation;
- contracts as canonical JSON with a sha256 fingerprint;
- presence as one liveliness token per (instance × API), carrying the
  contract fingerprint in its key;
- contracts and schemas fetched by hash from any peer that holds them;
- FULL, transitive compatibility inside a major.

The structural simplification is that **most of v1's conventions become
ordinary standard contracts** (`health.v1`, `config.v1`, `alarms.v1`)
instead of spec chapters. A profile needs spec text only when it adds a key
plane or wire semantics that a contract cannot express (media, blob).

**Next step:** settle the seven decisions in §11. Then run a two-week spike
against a real `zenohd` to measure the assumptions in §8.7 before writing
the spec.

---

### 1. What zenkey is today

#### 1.1 Already spec-first

The brief's "hard design criterion" (brief §3) is already the stated intent
of v1:

- MUST/SHOULD/MAY bind four named roles (RFC 03 §0).
- Design properties split into a theorem and a precondition, with every
  theorem pinned by a guard test (RFC 03 §4).
- The origin and alert-key derivations are byte-precise and have test
  vectors (RFC 06 §1, RFC 11 §3.1).
- Observer conformance has test shapes (RFC 13).
- An amendment ledger records what changed *and what deliberately did not*.

What stops a second implementation is not a missing spec. It is the spec's
**size, churn and coupling** (§1.4).

#### 1.2 What it really is: an observability convention

| Evidence | Where |
|---|---|
| Identity is the physical machine (`h-` + sha256(machine-id + salt)) in every key. Moving a process to another machine re-keys its whole surface. | RFC 03 §1.3, RFC 06 §1 |
| The chunk order is justified by an observability fleet. | RFC 02 P11 |
| The semantics are telemetry/state/events: "alerts are state", cardinality budgets for observed populations. | RFC 04 §1–2 |
| The neutral "framework state set" is health, sensor registration, alert and evidence/*. | RFC 04 §1.4 |
| Catalog, evidence fusion, entities, aliases, incidents, ack, silence, edges and impact attribution live in a neutral chapter. | RFC 06 §4–§5.6 |
| Registry columns are metric-shaped: `kind = "histogram"`, `buckets`, `semantic = "duration"`, `[budget] rss_mb`. | RFC 08 §2, `fixture-tests/registry/sysinfo.toml` |
| The neutral runtime crate ships the ZenSight alert-key recipe, the ZenSight `errors` token and the catalog subjects. | `zenkey/src/alert.rs`, `common_state.rs`, `context.rs:156` |
| A tool's exit codes are normative protocol text. | RFC 13 §1.2 |

#### 1.3 The unit is the producer, not the API

- **Registries are per producer.** There is one registry file per
  producer (RFC 08 §2). Nothing models one API implemented by several
  producers, or one producer implementing several APIs. Shared surfaces
  are a closed table of `common` tokens defined in the spec (RFC 04 §1.4).
- **The runtime builds keys, not endpoints.** Applications declare their
  publishers and queryables with raw zenoh, so zenkey never sees a
  declaration.
- **`introspect` is a file, not an observation.** It serves the registry
  file compiled in with `include_str!`, verbatim
  (`zenkey-build/src/emit.rs:43`). `describe` builds JSON Schema at run
  time from Rust types through schemars (`zenkey/src/schema.rs:425`).
- **Truthfulness rests on external checks.** It is enforced by build-time
  coverage checks plus a startup comparison for procedures (RFC 08 §6.1).
  `BringUp` is a string seam (`zenkey-fleet/src/bus/producer.rs:292`), and
  the only real producer in this workspace is zenwatch.
- **Discovery is two independent wildcard fan-outs.** The roster (alive
  tokens) and the registry sweep (`v1/*/@rpc/*/introspect`) are separate.
  Nothing lists the live instances first and then asks only those.

#### 1.4 Size and churn

- **Spec:** about 8,000 lines across 13 chapters, plus a 1,900-line
  changelog. 50 amendments in 85 days.
- **Code:** about 180k lines of Rust. The convention itself (zenkey +
  zenkey-build) is about 22k. The rest is tooling: fleet 72k, zengui 46k,
  zenctl 29k, zenwatch 10k.
- **Tooling coupling:** a large share of that tooling is *generic*. It
  works on any Zenoh bus: the session, the `fleet_get` discipline, the
  monitor, the admin space, `.zrec`/`.zsnap`, structural decode, the
  judgement core. The rest is tied to the v1 grammar: every doctor check,
  `why`, `conform`, the planners, the roster. Section 9 sorts out which is
  which.

A second implementation would chase a moving target. As written, the spec
is a design journal with normative force. That is valuable as a record and
fatal as a foundation.

---

### 2. The brief against the current state

| Brief requirement (brief §) | Today | Gap |
|---|---|---|
| **A. Resource model** (brief §4.1A, §6) | Classes, `@rpc` procedures, liveliness, payload type names | *Wrong unit:* per producer, physical identity, observability classes |
| **B. Registry as source of truth** (§7) | Yes: per producer, TOML **or** KDL, lints, two locks | Per-API contracts, one syntax, a canonical form for hashing |
| **C. Typed Rust generation** (§12) | Typed *keys*: `Subject` enums, builders, parsers, typed origins | No typed *endpoints* (publisher/handler types), so code cannot drive introspection |
| **D. Runtime introspection** (§9) | `introspect` per producer + alive tokens | Fan-in of whole registry files, O(producers × file size); no service/instance distinction |
| **E. Schema discovery** (§8) | `describe` serves a SchemaSet with hashes; kinds `json-schema`, `protobuf`, `cdr` | Strong design. But JSON Schemas come *from Rust types* (brief §8.4 violated), and the protobuf hash covers raw descriptor bytes, which are not stable across toolchains |
| **F. Compatibility** (§11) | `registry.lock` (backward), append-only `deprecated.lock`, suffixed siblings | Structural, good. No contract fingerprint, no payload-type compatibility, no direction or transitivity |
| **Truthfulness** (§9.2) | A normative MUST, `when` gates, `error/gated` and `error/unsupported` | The best analysis in the RFC set. Enforcement sits outside the code that serves |
| **Deployment independence** (§5.1) | ✗ (host origin in every key) | **The central gap** |
| **Small core** (§28) | ✗ | **The second central gap** |
| **Scale** (§18) | `introspect` fan-in returns whole files. ZenSight's 12 files total 105 KB (≈ 9 KB each), so 4,000 producers ≈ 36 MB per inventory query | Content addressing |

---

### 3. Where the brief is right

These are briefly listed because I agree with them and they carry through to
§8. Section numbers in this list are the brief's.

- **The product framing** (§1, §21): typed, introspectable application APIs
  over Zenoh. Measure everything against "Zenoh + Protobuf + documented key
  names".
- **Logical service identity** as a first-class concept, separate from
  runtime instance identity (§5.1, §5.4).
- **Contract versus runtime status** (§9.3). This is the cleanest idea in
  the brief, and it maps directly onto *contract by hash* versus *instance
  descriptor* (§8.7 of this document).
- **A contract fingerprint** as the authoritative identity, with the human
  version as a label (§11.2). Schemas are content-addressed (§8.2).
- **No central services** (§13). Peers serve what they hold.
- **Introspection that can be secured with ACLs**, not one giant endpoint
  (§14).
- **Language-independent conformance fixtures** (§17).
- **Profiles outside the core** (§4.2, §15), and "removing from core is not
  deleting".

---

### 4. Where I challenge the brief

#### 4.1 "Specification first" is not the fix. A small, slow spec is.

v1 shows that a spec can be complete, rigorous and still unusable as a
foundation. Make the property mechanical:

- **Page budget.** The core spec fits in about 20 pages. A new normative
  rule displaces an old one or goes to a profile.
- **Every normative change ships with a fixture**, or it is not normative.
  This is the brief's §17 turned into a gate.
- **Lessons go to guides, not MUSTs.** Most v1 amendments record a bug
  found in practice (RFC 05 §2.1's two editorial notes say so). The fix
  for "an implementer did not read the section" is a guide, a lint or a
  test. It is not more normative text.
- **Profiles version independently.** The core moves on a slow train,
  for example at most one release per quarter, with errata in between.

#### 4.2 Presence is per instance, not a resource kind

Brief §6 puts `Presence` beside `Stream`/`State`/`Operation` under `Api`.
But an API *definition* does not have presence. A running *instance* does,
and one instance implements several APIs (brief §7.2). Presence is
therefore part of the **instance model** (§8.2, §8.7), and the resource
kinds are three.

#### 4.3 Versioning: Model C, and why the brief's lean toward B is wrong here

Brief §11.3 asks for a deliberate choice and says "do not automatically put
a version component into every key". Here is the deliberate choice: **the
API major goes in the path, as part of the API chunk (`nav.v2`). Revisions
inside a major are carried by the contract fingerprint, never by the key.**

- **Pub/sub has no negotiation.** With a stable path (Model B), v2 and v3
  publishers share a key, and an old subscriber receives bytes it cannot
  decode. The failure shows up in every consumer, at run time.
- **Fleets cannot flag-day.** A vehicle that comes back after three weeks
  offline runs last month's major. Two majors must coexist on one bus.
  Zenoh's key algebra gives that for free: `nav.v2/**` and `nav.v3/**`
  are disjoint, and a service can serve both during a migration.
- **It matches the type ecosystem.** buf's `PACKAGE_VERSION_SUFFIX`
  convention names protobuf packages `nav.v2`. The API chunk and the
  protobuf package can then be the same string.
- **It is what the closest prior art converged on.** uProtocol carries
  `ue_version_major` in every URI and keeps minors out of it.

What should **leave** the key is v1's global `v1` convention chunk (RFC 03
§1.2). It protects against a convention-grammar break that the RFC itself
hopes never happens (RFC 08 §3: "the protocol version froze at 1"), and it
costs a chunk on every key forever. If the v2 grammar ever has to break,
the deployment namespace is the isolation tool. The control plane carries
the convention version in its documents.

v1's sibling move (`sockets` → `sockets2`, RFC 08 §3) stays legal inside a
major as an *additive* change. The major bump is for when the cleanup is
worth a migration.

#### 4.4 Truthfulness: exposure is declared capability, not observed traffic

Brief §9.2 and §12.3 want runtime registration to populate reflection.
Agreed, with one precision the brief lacks and v1 already found (RFC 08
§6.1, "Checking the two halves"):

- **Publishers are idle most of the time.** A stream that has not
  published yet is not a lie. A host with no IMU never publishes IMU data,
  and that is correct.
- So "what this instance exposes" must be the set of **declared
  capabilities**: handles obtained from the service object before it goes
  live. It must not be "what has been seen on the wire". If exposure were
  inferred from traffic, every idle stream would be a false finding.

The v2 runtime therefore requires every publisher, state writer and
operation handler to be **declared at start-up through the generated API**.
The descriptor is built from those declarations (§8.8). A resource the
build cannot produce on this platform is declared `unavailable` with a
reason. That keeps v1's `error/gated` and `error/unsupported` distinction
without the `when` predicate language.

#### 4.5 Take telemetry semantics out of the core, but keep kind in the key

Brief §4.2 and §15 move telemetry/state/events to profiles, which is right
for their *semantics*: TTLs, cadence, histogram buckets, alert rules. But
v1's class chunk earned its place for infrastructure reasons that have
nothing to do with observability (RFC 03 §4 D3, RFC 04 §3–4):

| Selects on kind | Why it matters to any application |
|---|---|
| Storage | "Keep the last known state of every vehicle" is one storage on `*/*/*/state/**`. Without a kind chunk, it becomes a generated list of every state key. |
| QoS pinning | Zenoh's `qos` overwrite interceptor works per key expression. "All streams best-effort" is one rule. |
| ACL read versus act | Reading state and calling an operation are both a Zenoh `query`. Only the key tells them apart. |
| Wildcard-GET safety | A `GET vehicle-01/**` snapshot must never invoke a side-effecting operation (§5.3). |

So the key keeps a kind position: `stream` and `state` as plain chunks, and
`@op` as a verbatim chunk. Changing a resource's kind is already a breaking
change in the brief's own list (§11.1), so carrying it in the key costs
nothing in compatibility terms.

#### 4.6 Schemas: bless two kinds, and make them artifacts

Brief §8.3 proposes a provider model "where possible". Without a blessed
default, "a generic tool can decode" and "CI can classify type changes"
mean nothing in practice.

- **Bless protobuf and JSON Schema** (JSON or CBOR bytes) in the core. Both
  have dynamic decoders in every mainstream language (prost-reflect,
  `jsonschema`). Protobuf has well-understood compatibility rules (buf).
  Anything else, CDR/ROS IDL first, is an extension kind.
- **The schema is a committed artifact, not something derived from Rust.**
  Today `describe` runs schemars over Rust types, which makes Rust types
  the protocol (brief §8.4). In v2:
  - a contract references schema *files* (a `.proto`, a JSON Schema
    document);
  - a JSON Schema may be *generated* from Rust, but it is committed and
    diffed;
  - CI fails when the Rust type no longer produces the committed artifact.

  Rust-first authoring survives. Rust-as-protocol does not.
- **Hash a normalized description, not raw bytes.** v1 hashes "the raw
  descriptor bytes for protobuf" (RFC 08 §7). Those bytes change with
  protoc/buf versions and source info, which produces false drift.
  - Do what ROS 2 does for RIHS01: define a normalized type description
    per kind (field names, numbers, types, the referenced-type closure, no
    comments or defaults) and hash that.
  - Unlike RIHS01, define the serialization by a standard (JCS, RFC 8785),
    not by one implementation's `json.dumps` settings.

#### 4.7 "All resources of type Y" is not a key concern

Brief §5.3 lists it as a dominant selector. It should be answered by
introspection, never by the key. A type is not a role (two `nav.v2.Pose`
streams mean different things), and type-in-key turns every schema
evolution into a re-key. v1 rejected rmw_zenoh's type-hash-in-key for
exactly this reason (RFC 03 §6.4), and that rejection still holds.

#### 4.8 A neutral domain, yes, but not instead of a real adopter

Brief §19 drives the design from a navigation example "rather than
ZenSight". Neutral examples are needed, but a design validated only on
imagined services is the classic second-system trap. ZenSight (and tcgui)
are the only production adopters, and porting one of them is the cheapest
honest test of generality. §8.4 shows that v1's world *is* a single
deployment shape of the proposed grammar (system = host). Add one real
company pilot service on top of that.

#### 4.9 The plan: spike, then specify

Brief §25 puts nine documents before Phase 5's first runtime code. Several
§8 decisions rest on Zenoh behaviour that must be *measured*, not argued:

- liveliness propagation and replay at 10k tokens;
- router-restart storms;
- `BestMatching` replies for content-addressed fetches;
- ACL behaviour on verbatim chunks;
- the interplay between namespaces and verbatim chunks.

v1's own history shows how this goes. The `@v1` verbatim version chunk
looked right on paper and silently killed advanced-publisher detection
(RFC 03 §1.2). Insert a short spike (§10, phase 1), and fold the nine
deliverables into four: this analysis plus decisions, the core spec plus
fixtures, example contracts, and a roadmap.

#### 4.10 Fewer crates

Brief §4.2 sketches seven core crates and §16 says "smaller is better".
Three are enough:

| Crate | Holds |
|---|---|
| `zenkey-model` | Contract model, validation, canonical form, fingerprint, compatibility classifier, key grammar. No zenoh dependency. |
| `zenkey` | Runtime: service builder, typed handles, presence, descriptor, contract serving, clients, fleet selectors. |
| `zenkey-build` | Codegen. |

Profiles become crates only when they carry code: media, blob. Standard
contracts (`health.v1`, `config.v1`) are data plus generated code.

#### 4.11 No actions or parameters, but write down the patterns

Agreed: no ROS actions, parameter server or lifecycle nodes (brief §20). But
two needs recur in mission and payload software, and leaving them unwritten
means every team invents them:

- **Long-running work** = an operation that returns an id, plus state
  keyed by that id, plus a cancel operation. This is v1's RFC 05 §3
  pattern, unchanged.
- **Configuration** = v1's RFC 05 §5.1 commit-confirm protocol (set,
  confirm, cancel, extend, persist; hot versus reach groups). It is good
  engineering and fully generic, and it should become the standard
  contract `config.v1`.

---

### 5. What the brief misses

#### 5.1 Ownership once identity is logical

If `vehicle-01/navigation` is the key prefix and two instances run (brief
§19: "start a second navigation instance; represent failover"):

- both publish `position`, and subscribers see interleaved duplicates;
- both write `status`, and last-writer-wins flaps;
- both serve `set_origin`, so a `QueryTarget::All` call actuates twice,
  and `BestMatching` picks one by routing distance, not by role.

D-Bus solves this with name ownership: one primary owner of a well-known
name, a queue of replacements. Zenoh has no ownership, and v1 states
plainly that "presence is not a lock" (RFC 03 §1.5). v2 needs a core rule:
**at most one active instance per service writes its data keys and serves
its operations.** Standby instances announce presence with a standby role
and expose nothing. The core does not elect a leader. A redundancy profile
may (§8.13). Two genuinely different implementations of one API (GNSS
navigation versus visual navigation) are two *services*, not two instances.

#### 5.2 Parametric resources

The hardware-driver test (brief §19) and most real APIs need collections:
tracks, jobs, devices, checks. The brief's resource model has no
parameters. Without them, every collection either becomes N services (N
presence tokens and N descriptors for N table rows) or gets flattened into
one payload, which loses key-level selection and ACLs. v1's `{var}` and
`{var...}` templates, with injective slugging (RFC 03 §2), are the right
mechanism and should stay in the core.

#### 5.3 Wildcard GETs reach side-effecting operations

In the brief's model, state is answered by a queryable and so is an
operation. A tool that runs `GET vehicle-01/**` to snapshot state would also
**invoke every operation whose queryable intersects the selector**. v1 is
immune because `@rpc` is a verbatim chunk, and `*`/`**` never match it (RFC
03 §4 D2). Keep that property: operations live under a verbatim `@op`.

Keep the second half of the lesson too. **No ACL can refuse a query broader
than its rule.** A deny rule fires only when it *includes* the query's key
expression (RFC 09 §3 fact 6). So a server must refuse a non-concrete query
to an operation whose contract does not allow fan-out (RFC 05 §2.1,
`error/fanout-forbidden`).

#### 5.4 Compatibility has a direction, and a fleet makes it transitive

Brief §11 classifies changes but never says compatible *for whom*:

- For a stream or state, the service writes and clients read.
- For an operation, the client writes the request and the service writes
  the response.
- In a mixed fleet, upgrades happen in every order, so both directions
  happen at once.

Inside a major, every change must therefore be **FULL** compatible (old
readers read new data, new readers read old data). It must also be
**transitive**: a revision has to be compatible with *every* earlier
revision of the major, not just the previous one, because offline members
come back on arbitrary old revisions. Confluent's schema registry names
these modes FULL_TRANSITIVE. Without them, "compatible" is ambiguous.

#### 5.5 The ACL facts that constrain the keyspace

v1's first live ACL deployment produced six facts (RFC 09 §3). These are the
ones any v2 keyspace has to respect:

1. ACL matching is keyexpr *inclusion*, and `**` does not cross a verbatim
   chunk in ACL rules either.
2. Interest is evaluated on egress, against the responding face's subject.
   A publisher with no matching interest publishes to nobody, silently.
3. Deny works by inclusion (§5.3).
4. The running ACL is not observable through the admin space.
5. Identity in a key is self-claimed. Binding a certificate CN to a key
   prefix (enrollment) is what turns a prefix into a security boundary
   (RFC 03 §4 D6).

The brief's §14 asks for "secure with Zenoh ACLs" without these. §8.11
designs the key layout around them.

#### 5.6 Infrastructure selectability

Storage `strip_prefix` needs a literal prefix. Link budgets on constrained
faces are prefix allowlists. The QoS overwrite is per key expression. These
are why v1 orders keys "policy boundaries left, variance right" (RFC 02 P3).
The brief optimizes selectors for *queries* (§5.3) but not for
*infrastructure policy*. In a vehicle fleet both point the same way:
**system first**.

#### 5.7 A cost model, measured

Brief §18 lists what to measure. It also needs a model of *what propagates
where*, because liveliness tokens, declarations and interest behave
differently for routers, peers and clients. §8.7 gives the estimate and the
spike measures it.

#### 5.8 A migration posture

"Backward compatibility may be broken freely" (brief §26) is fine for the
code. But zenctl runs in production built from source, ZenSight and tcgui
are adopters, and 150k lines of tooling assume v1. The plan needs an
explicit answer: freeze v1, then port or retire each adopter (§10, §11).

---

### 6. Lessons from v1 that v2 must keep

These cost real bugs to learn, and each has a citation. v2 should carry them
as rationale, guides or tests, not necessarily as normative text.

| Lesson | Source |
|---|---|
| Verbatim chunks make separation hermetic (no wildcard crosses them). Never put one on a key that zenoh-ext advanced publishers use: the `@adv` token parser cannot capture across `@`. | RFC 03 §1.2, RFC 04 §5 |
| Fan-in discipline: target `All`, reply on your own concrete key, consolidation `None`, attribute silence against the roster, never declare a fan-in queryable `complete`. | RFC 05 §2.1 |
| Silence is never a verdict. "Not asked" and "unobservable" are different facts. | RFC 05 §3.1, RFC 13 §1–2 |
| Alive ⇒ callable: declare queryables (and caches) before presence. | RFC 04 §5 |
| Presence is not a lock. Actuators need an exclusivity lock outside the bus. | RFC 03 §1.5 |
| Deprecate, never reuse, and make the ledger append-only so CI can check it. | RFC 02 P10, RFC 08 §3 |
| No implicit identity in a builder. A builder that silently supplied the local origin shipped the same bug three times. Fleet selection is spelled by name only. | RFC 08 §1.1 |
| Servers refuse broadcast writes. ACLs cannot. | RFC 05 §2.1, RFC 09 §3 fact 6 |
| No cross-key atomicity. Anything that needs a snapshot is one document on one key. | RFC 04 §1.2 |
| Set `Encoding` on every sample. Decode order is sample > contract > sniff. | RFC 04 §3, RFC 08 §7 |
| Retire state with a Zenoh delete, never a payload marker. | RFC 04 §1.2 |
| Injective slugging ships with its decoder and a round-trip test (the reserved-prefix erratum). | RFC 03 §2 (v1.31) |
| No type hash in keys, no central schema registry, no per-sample schema-id prefix. | RFC 03 §6.4, RFC 08 §7 |
| Core participants use only stable zenoh API. Unstable features are opt-in and never change a key shape. | RFC 03 §0.1 |

---

### 7. Prior art: what to take, what to refuse

This section answers brief §22's three questions per system: does it solve
something Zenkey needs, should we reuse it, and if not, why not. v1's RFC 10
already covers the keyspace side (Keelson, uProtocol, rmw_zenoh, Sparkplug,
NATS, D-Bus, Homie, OPC UA). This pass adds the *API contract, discovery and
type* side, re-checked against current sources (2026-09/10).

| System | Relevant facts | Take | Refuse, and why |
|---|---|---|---|
| **Keelson** (RISE, 0.6.0-pre) | `{base}/@v{major}/{entity}/pubsub/{subject}/{source…}`; RPC at `…/@rpc/{procedure}/{responder}`; flat `subjects.yaml` mapping subject → protobuf type; interfaces are protobuf `service`s; liveliness is "a presence signal, not a capability declaration"; **no runtime schema serving**; a written rule that each new verbatim chunk must justify its "discoverability tax" | The subject → type registry; protobuf interfaces; the verbatim-chunk tax rule (v2 has exactly two core verbatim tokens) | Trailing `source_id` (RFC 03 §6.3); an envelope with no type tag; presence that says nothing about capability |
| **uProtocol** (up-spec, v1.6.0-alpha) | UUri: authority / `ue_id` (16-bit type + 16-bit instance) / `ue_version_major` / `resource_id`. Methods use `0x0001–0x7FFF`, topics `0x8000–0xFFFE`. The Zenoh mapping puts source **and** sink in an 11-chunk key and `UAttributes` protobuf in every attachment, and says **"Zenoh's Queryable API MUST NOT be used"**. uDiscovery is a tree with a central cloud root; uSubscription centralizes subscription bookkeeping | Logical service type separate from instance; **major in the address, minors out**; the kind is recoverable from the address (resource-id ranges ≈ v2's kind chunk) | Numeric ids (unreadable keys, a numbering authority); sink in the key; a per-message attribute envelope; banning queryables (v2's state GET and operations *are* queryables, Zenoh-native); central discovery and subscription services (brief §13) |
| **rmw_zenoh** | Data key `<domain>/<topic>/<type>/<RIHS01 hash>`, adopted after a deserialization crash on mismatched types (rmw_zenoh#117/#171); the token key packs the whole graph entity, QoS included; **every process liveliness-gets and history-subscribes the whole domain** | Key-packed presence (zero payload); hash-in-discovery | Type hash in the data key (silent non-communication, re-key on every evolution; FULL compatibility inside a major plus major-in-key gives the same crash protection, diagnosably); whole-domain token replication |
| **ROS 2 REP 2011/2016** (still open PRs; shipped since Iron/Jazzy) | RIHS01 = sha256 over the type description JSON (defaults stripped, referenced types sorted), carried in discovery and the rmw_zenoh key; `~/get_type_description` per node | Hash a **normalized description**, not IDL bytes; ship the hash in discovery and fetch the body on demand | Defining the canonical form by one implementation's `json.dumps(separators=…)` settings. v2 cites **JCS (RFC 8785)** instead, a standard any language implements |
| **DDS XTypes 1.3** | TypeIdentifier = first 14 bytes of MD5 over the XCDR2 TypeObject; SEDP carries ids, not objects; **TypeLookup fetches by id on demand**; FINAL/APPENDABLE/MUTABLE extensibility and assignability rules. DDS discovery traffic grows ~n(n−1)(r+w), and every participant stores every endpoint (Zenoh's measurement: 97.4–99.97 % less discovery traffic) | Fetch-by-hash on demand (v2's contract bundles); assignability as the model for directional compatibility | Per-endpoint discovery; the type system's complexity |
| **D-Bus** (spec 0.43) | `Introspect` XML is *emitted by the implementation*; `Properties` + `PropertiesChanged` (with `invalidates`); well-known names have an owner queue; unique names are never reused; `NameOwnerChanged` is the failover signal; `ObjectManager` | Implementation-emitted introspection (§8.8); **logical name versus unique instance**; single owner (§5.1, §8.13); `invalidates` ≈ v1's large-state pattern | The central bus daemon (Zenoh has no owner queue, hence the profile) |
| **NATS Service API** (ADR-32 rev 6) | `$SRV.PING/INFO/STATS[.name[.id]]`; `{name, id, version, metadata}`; a **unique id per start**; INFO lists endpoints with subjects; the **SCHEMA verb was removed in 2023**, with schemas left to free-form endpoint metadata | Instance id minted per start (§8.2); a small INFO-like descriptor; per-endpoint stats belong in tooling | Leaving schemas to free-form metadata: generic decode is a core goal here (brief §28), which NATS never claimed |
| **Sparkplug B 3.0** | `spBv1.0/group/type/node/[device]`; "NBIRTH MUST include every metric the Edge Node will ever report on", and publishing an unborn metric is grounds for a rebirth request; `bdSeq` pairs a death with its birth | **Declare before publish** (§8.8 is the same contract); a device level (≈ device-as-service); pairing the death with the birth (≈ instance id in the token) | Host STATE outside the namespace (the lesson v1 already took); seq integrity tied to an MQTT broker |
| **gRPC reflection** | `ServerReflectionInfo`: list services, `file_containing_symbol` returns FileDescriptorProtos with their dependencies; no hash, version or instance | Reflection over the same transport, no side channel | No content addressing: every client re-downloads |
| **buf breaking** | FILE > PACKAGE > WIRE_JSON (recommended minimum) > WIRE | Delegate protobuf compatibility to these rules; v2 needs WIRE_JSON, since tools transcode to JSON | n/a |
| **Confluent Schema Registry** | BACKWARD / FORWARD / FULL, each optionally `_TRANSITIVE` | The vocabulary: inside a major, **FULL_TRANSITIVE** (§5.4) | A central registry; a schema-id prefix on every message |
| **AsyncAPI 3.x** | Channel address templates with parameters; send/receive operations; `schemaFormat` (protobuf recommended); ~20 bindings including ros2, **no Zenoh binding** | An *export* target for documentation (`zenctl contract export --asyncapi`); parameter templates | As the source of truth: verbose, not canonicalizable, JS-centric tooling, no Zenoh semantics |
| **Smithy** (from my own knowledge, not re-verified) | Protocol-agnostic IDL with traits and a JSON AST; `smithy-diff` severities | Annotations as namespaced traits; diff severities | JVM toolchain; request/response-centric |
| **COVESA VSS / IFEX** | Signals with datatype, unit, min/max, `deprecation`; **instances** (`Row[1,2] × [DriverSide, PassengerSide]`); the service catalog moved to IFEX | Units and ranges as *semantic* annotations (review-class changes, §8.9); instances ≈ resource templates | A data catalog without a service or presence model |
| **MCAP / Foxglove** | Schema record `{name, encoding, data}`; well-known schema encodings `protobuf` (FileDescriptorSet with imports), `jsonschema`, `ros2msg`, …; channel `message_encoding` | **Name v2's schema kinds with MCAP's spellings** (`protobuf`, `jsonschema`), so a recorder writes MCAP with no mapping table | n/a |
| **zrpc** (ZettaScaleLabs, alpha) | `@rpc/<server-uuid>/service/<svc>/<method>`; a metadata endpoint; one token per service | Confirms the per-service token + metadata pattern | Instance-addressed keys (K5's failover re-key) |
| **Zenoh admin space** | `@/<zid>/<whatami>/{subscriber,queryable,querier,token,…}/**`, remotely queryable; off by default in the library, forced on in zenohd; no types; config not observable | Joining by zid (the descriptor carries it); Zenoh-level cross-checks | It is no app-level contract, and it is unavailable on library peers by default |

**Patterns across them:**

- **Static versus served types.** Keelson, uProtocol and VSS rely on
  static, compile-time catalogs. Only ROS 2 (by hash) and DDS (TypeLookup)
  serve type descriptions at run time, and both do it the way §8.7 proposes:
  a hash in discovery, the body on demand.
- **Declare before publish.** Sparkplug alone makes it a MUST, and it is
  the right contract for truthfulness.
- **Version majors in the address** are near-universal (uProtocol,
  Keelson, Homie v5, v1 itself). Minors in the address are nowhere.

---

### 8. Proposed design

#### 8.1 Scope and non-goals

**Core:**

- identity and key grammar;
- three resource kinds and their wire behaviour;
- the contract data model, its canonical form and fingerprint;
- type identity and the two blessed schema kinds;
- presence and introspection;
- operation calling rules (concrete key, fan-out, errors, fan-in
  discipline);
- compatibility classification;
- conformance fixtures.

**Not core:**

- in profiles or standard contracts: QoS profiles, freshness and TTL
  conventions, event logs, alarms, health semantics, configuration
  protocol, media, blob, redundancy election;
- in tooling and guides: record/replay, observer honesty rules, ACL and
  storage generation;
- in ZenSight: catalog, evidence, entities, incidents.

#### 8.2 Identity model

| Concept | Example | Chosen by | In data keys? |
|---|---|---|---|
| **Deployment** | `prod`, `sim-42` | operator | Optional prefix, as the Zenoh session namespace (as in v1) |
| **System**: the unit that owns services (vehicle, robot, ground station, gateway, simulation) | `vehicle-01`, `ground` | deployment config | **Yes**, position 1 |
| **Service**: logical, stable | `navigation`, `imu0` | deployment config | **Yes**, position 2 |
| **API**: name + major | `nav.v2`, `health.v1` | contract author | **Yes**, position 3 |
| **Instance**: one live run of a service | `8f3a5c2e9b1d4f70` (random 64-bit, minted at service start) | runtime | **No**: presence and descriptor only |
| **Host / process / build / zid** | `nav-computer-2`, pid, git sha, session zid | runtime | **No**: descriptor metadata |

**The system is a mandatory single chunk.** It is the outermost policy
boundary of a fleet: ACL principal, link budget, storage scope, ownership.
A single-system deployment spends one constant chunk on it. It is not the
host: moving `navigation` from computer-2 to computer-3 inside `vehicle-01`
changes nothing in any key. A fleet-level service lives on a logical
system such as `fleet` or `ground`.

**The service is a single chunk chosen by deployment, not by code.** The
code says "I implement `nav.v2` and `health.v1`". The deployment says "you
are `vehicle-01/navigation`". One process may host several services, for
example an IMU driver hosting `imu0` and `imu1`.

**The instance id is minted per service start, not taken from the zid.**
A restarted service is visibly a new instance even if its session
survives. The descriptor carries the zid, so tools can still join to
Zenoh's admin space and to `replier_id`/source info.

**No implicit identity anywhere** (the RFC 08 §1.1 lesson):

- a client call always takes an explicit service address;
- a server's handles are bound to the service that declared them;
- fleet selection is a named constructor (`Fleet::all()`), never a
  default.

#### 8.3 Resource model

Three kinds. Each has an exact Zenoh mapping, so another implementation
needs no Rust:

| Kind | Meaning | Zenoh mapping | Producer obligations |
|---|---|---|---|
| **stream** | Values pushed by the service. Each sample stands alone. | Declared publisher on `…/stream/<resource>` | `delivery = sampled` (best effort, droppable) or `reliable`. No late-joiner guarantee in the core. History is a profile annotation backed by zenoh-ext's advanced publisher, which is unstable and whose wire format is defined only by the Rust source. That is exactly why it cannot be core. |
| **state** | The latest value *is* the truth. Supersedes earlier values. May be deleted. | Declared publisher plus a queryable answering GET on `…/state/**` of the API | MUST answer GET with the current value per concrete key (no storage required, brief §6.2). MUST retire a key with a Zenoh delete. The queryable is not `complete` (v1's rule), so a GET reaches the service *and* any storage holding the key, and default consolidation keeps the newest reply per key. That is why state SHOULD carry Zenoh timestamps. |
| **operation** | Request/reply | Queryable on `…/@op/<operation>` | MUST reply on its own concrete key. Success is a value reply, failure is `reply_err` with the core error envelope. MUST refuse non-concrete queries unless the contract says `fanout = "allowed"`. |

All three take **resource path templates**: `tracks/{track}`,
`checks/{check}`, `devices/{device}/sample`. A template has typed
parameters (`string` slugged by the v1 rule, `uint`, `enum`). A wildcard
GET on a state template (`…/state/tracks/*`) is the collection listing.

**The core error envelope** has a small closed code set: `invalid_request`,
`not_found`, `unavailable`, `forbidden`, `fanout_forbidden`, `busy`,
`internal`, `app`. `app` carries the contract's typed error. This is v1's
`error/*` vocabulary with `gated`/`unsupported` folded into `unavailable`
plus a reason.

**What is deliberately absent:**

- events as a kind (a reliable stream, or a state-per-alarm via
  `alarms.v1`);
- presence as a kind (§4.2);
- media and blob (profile planes, §8.12).

#### 8.4 Keyspace: alternatives and choice

Five candidates, run against the scenarios of brief §25 Phase 3:

| | K1 host-first | **K2 system/service-first** | K3 API-first | K4 flat logical | K5 instance-addressed |
|---|---|---|---|---|---|
| Shape | `<host>/<svc>/<api>/<kind>/<res>` | `<system>/<svc>/<api>/<kind>/<res>` | `<api>/<system>/<svc>/<kind>/<res>` | `<system>/<svc>/<res>` | `<system>/<svc>/<instance>/<api>/<kind>/<res>` |
| vehicle-01 navigation | `h-3fa9…/navigation/nav.v2/stream/position` | `vehicle-01/navigation/nav.v2/stream/position` | `nav.v2/vehicle-01/navigation/stream/position` | `vehicle-01/navigation/position` | `vehicle-01/navigation/8f3a…/nav.v2/stream/position` |
| Two navigation instances | different hosts → different keys | same keys, single active writer, instances visible in presence | same as K2 | same keys, nothing tells them apart | natural: different keys |
| One process: nav + health | `…/nav.v2/…`, `…/health.v1/…` | same | same | **collision**: both define `status` | same |
| Fleet-wide nav subscription | `*/*/nav.v2/stream/position` | `*/*/nav.v2/stream/position` | `nav.v2/*/*/stream/position` | `*/navigation/position` (by service name only, not by API) | `*/*/*/nav.v2/stream/position` |
| Hardware devices | as K2 | `vehicle-01/imu0/imu.v1/stream/sample` (device = service), or a template | as K2 | as K2 | as K2 |
| Service moved to another host | **re-keys** ✗ | nothing changes ✓ | ✓ | ✓ | re-keys on every restart ✗ |
| Per-system ACL / link / storage | per host only | `vehicle-01/**` + 2 verbatim rules, literal prefix ✓ | `*/vehicle-01/**`, not a literal prefix ~ | ✓ | ✓ |
| Read-versus-act ACL, GET safety | ✓ | ✓ (`@op` verbatim) | ✓ | ✗ (ops reachable by `**`) | ✓ |
| Versioning | major in API chunk | major in API chunk | major in API chunk, outermost | none | major in API chunk |
| Readability | opaque hex | ✓ | ~ | ✓✓ | opaque hex |

**Choice: K2.** K1 fails deployment independence, which is brief §28's
explicit criterion and the reason for v2. K4 is the brief's illustrative
UX taken literally. It collides across APIs, cannot select by API, and
exposes operations to wildcard GETs. K5 attributes instances precisely,
but stable logical identity disappears from the key and every failover
re-keys every subscriber. K3 is close, and is the right choice for a
deployment whose outer policy boundary is the *API*. In a vehicle fleet,
the principal, the link and the storage scope are the *system*.

On top of K2, **the human-facing path is `system/service/resource`**. Tools
resolve that path through the contract (`vehicle-01/navigation/position` →
`…/nav.v2/stream/position`) and require the API only when two APIs of one
service share a resource name. This gives the brief's §9.1 UX without
paying for K4's flatness on the wire.

**Lexical rules** follow v1's charset (RFC 03 §2). Positions 1–4 have fixed
arity, so `*/*/*/state/**` is exact.

- The API chunk is `<name>.v<major>`, where the name may be dotted
  (`acme.nav.v2`).
- The kind chunk is exactly `stream`, `state` or `@op`. Profiles may
  register additional verbatim kinds (`@media`, `@blob`).
- `@zk` is the one reserved control token. It appears at position 3 for
  presence and descriptors, and at position 1 for the location-free
  contract space. Being verbatim, it is reached by no data wildcard, and
  `vehicle-01/**` never pulls control traffic.

**Sanity check against v1's world.** Set system = host and service =
producer, and ZenSight fits:

- `h-3fa9…/sysinfo/sysinfo.v1/stream/cpu/usage`;
- the catalog becomes `fleet/catalog/zs.catalog.v1/state/entity/{id}`.

v1's D2 (the per-host firehose is data-only) survives as
`h-3fa9…/**`, since `@op`, `@zk`, `@media` and `@blob` are all verbatim.

#### 8.5 Contracts

**One authoring syntax** (TOML, already used and parsed everywhere). The
canonical form is JSON: sorted keys, resources sorted by name,
documentation excluded, and every type replaced by `{name, kind, schema:
sha256}`. **The contract fingerprint is `sha256(JCS(canonical))`.** v1's
second spelling (KDL) is dropped: two spellings of one document is the kind
of expressiveness brief §7.1 warns against.

The fingerprint excludes documentation and the `minor` label, so fixing a
typo does not change contract identity. CI requires `minor` to increase
whenever the canonical form changes compatibly (the label is enforced, but
it is never what the protocol compares).

```toml
# contracts/nav.v2.toml
[api]
name    = "nav"
major   = 2
minor   = 1
summary = "Navigation solution of one vehicle"
schemas = { protobuf = "proto/nav/v2/nav.proto" }   # package nav.v2

[resources.position]
kind     = "stream"
type     = "nav.v2.Position"
delivery = "sampled"

[resources.status]
kind = "state"
type = "nav.v2.Status"

[resources.origin]
kind = "state"
type = "nav.v2.Origin"

[resources.set_origin]
kind       = "operation"
request    = "nav.v2.SetOriginRequest"
response   = "nav.v2.Origin"
error      = "nav.v2.NavError"
idempotent = true
timeout_ms = 2000

[resources.reset]
kind     = "operation"
request  = "google.protobuf.Empty"
response = "google.protobuf.Empty"
optional = true                      # an implementation may declare it unavailable
```

```toml
# contracts/common/health.v1.toml: a standard contract every service SHOULD implement
[api]
name  = "health"
major = 1
minor = 0
schemas = { protobuf = "proto/health/v1/health.proto" }

[resources.status]
kind = "state"
type = "health.v1.Status"            # ok | degraded | failed, with a reason

[resources."checks/{check}"]
kind   = "state"
type   = "health.v1.Check"
params = { check = "string" }

[resources.faults]
kind     = "stream"
type     = "health.v1.Fault"
delivery = "reliable"
```

**Composition is a list, not inheritance** (brief §7.3). A service declares
the APIs it implements. Shared *types* are shared schema packages
(`common.v1.Error`). There is no `extends`, no mixins, no overrides.
Fleet-wide health then needs no special mechanism:
`*/*/health.v1/state/status`. This is what v1's closed `common` token
table (RFC 04 §1.4) was trying to be.

**Annotations** are the extension point for profiles. A contract resource
carries a namespaced map, for example `annotations = { "telemetry.unit" =
"m", "qos.profile" = "sampled", "freshness.ttl_s" = 60 }`:

- annotations are part of the canonical form, so they count in the
  fingerprint;
- unknown namespaces are preserved;
- a profile declares which of its keys are *semantic*, so the classifier
  can flag changes for review (§8.9).

Field-level semantics (units on a schema field) belong in the schema, as
protobuf custom options.

**Validation is language-independent.** A JSON Schema validates the
contract data model, and the lint rules are listed with fixtures (v1's RFC
08 §5 lints become checks with test vectors). zenkey-build runs the same
checks, but the checks are not *defined* by zenkey-build.

#### 8.6 Types and schemas

- **Type identity.** A logical name (`nav.v2.Position`), a schema kind,
  and a schema hash over the kind's normalized description (§4.6). The
  human name is for people; the hash is exact.
- **Kind names.** The kinds are `protobuf` and `jsonschema`, spelled as
  in the MCAP well-known registry, so recordings land in Foxglove with no
  mapping. This renames v1's `json-schema`.
- **Encoding.** Each sample carries the *predefined* Zenoh `Encoding` id
  (`application/protobuf`, `application/json`, `application/cbor`), as v1
  already requires. That costs a one- or two-byte varint.
  - **No schema suffix on stream and state samples.** The suffix
    (`application/protobuf;nav.v2.Position`) is sent with *every* put
    whose encoding is not the default, and Zenoh has no declare-once
    optimization for it. It is also capped at 255 bytes. The contract
    already binds the type, so the suffix would be a per-sample diagnostic
    paid for by every sample.
  - Operations and tools MAY add the suffix, since their traffic is
    low-rate.
- **One logical type, several encodings?** Not in the core. A resource has
  exactly one type and one encoding. This keeps v1's P5 ("one payload type
  per wildcard result set"). Transcoding is a gateway's job.
- **Bundles.** A contract fetched by fingerprint comes as a *bundle*: the
  canonical contract plus every schema it references (a
  `FileDescriptorSet` with source info stripped, plus JSON Schema
  documents). One fetch, cached forever, because a hash never changes
  meaning.

#### 8.7 Presence and introspection

Three layers, from cheapest to richest:

**1. Presence: one liveliness token per (instance × API), all
information in the key.**

```
<system>/<service>/@zk/alive/<instance>/<api>.v<major>/<fingerprint16>
vehicle-01/navigation/@zk/alive/8f3a5c2e9b1d4f70/nav.v2/3fa9c2d41b7e9a01
vehicle-01/navigation/@zk/alive/8f3a5c2e9b1d4f70/health.v1/77e0a5b6c2d3e4f5
```

These liveliness selectors answer the dominant discovery questions with
zero payload bytes:

| Question | Selector |
|---|---|
| Every live service | `*/*/@zk/alive/**` |
| Every implementation of `nav.v2`, and which contract revisions | `*/*/@zk/alive/*/nav.v2/*` |
| Everything alive on vehicle-01 | `vehicle-01/*/@zk/alive/**` |
| Redundant instances of one service | `vehicle-01/navigation/@zk/alive/*/**` |

The 64-bit fingerprint prefix is a fleet-wide drift detector: "three
different `nav.v2` revisions are live" comes from one liveliness query. The
full sha256 is in the descriptor.

**2. Instance descriptor: per instance, small, on demand.** It is a
state-like document at `<system>/<service>/@zk/instance/<instance>`,
answered on GET and re-published when exposure changes (dynamic modules,
degraded modes):

```json
{
  "zenkey": 2,
  "system": "vehicle-01", "service": "navigation",
  "instance": "8f3a5c2e9b1d4f70", "role": "active", "generation": 3,
  "started_at": "2026-10-07T09:12:44Z",
  "zid": "a1b2c3d4e5f60718293a4b5c6d7e8f90",
  "apis": [
    { "api": "nav.v2", "contract": "sha256:3fa9c2d4…", "minor": 1,
      "exposed": ["origin", "position", "set_origin", "status"],
      "unavailable": { "reset": "no IMU on this platform" } },
    { "api": "health.v1", "contract": "sha256:77e0a5b6…", "exposed": "*" }
  ],
  "meta": { "host": "nav-computer-2", "build": "nav-svc 4.2.0 (9c1e2f0)" }
}
```

**3. Contract bundles: content-addressed, location-free, served by any
holder.** The key is `@zk/contract/<api>.v<major>/<sha256>`.

- **It sits outside the system/service hierarchy, on purpose.** A hash has
  no owner. This is v1's own argument for `@blob/store` (RFC 03 §1.5:
  "a hash is a hash").
- **Every holder declares a `complete` queryable on that exact key.**
- **So one GET gets one reply.** A tool GETs the exact key with the
  default `BestMatching` target. Zenoh routes it to the **nearest**
  holder: the nearest complete queryable whose key includes the query's
  key, as verified in the 1.10 router source. The tool gets exactly one
  reply and caches it on disk forever.
- **Why not under the service.** Had the bundle lived under
  `<system>/<service>/@zk/…`, a fleet-wide fetch would have matched no
  complete queryable that includes it. Zenoh would have fallen back to
  `All`, and 200 vehicles would each have sent the same 50 KB.
- **This is not v1's fan-in case.** v1's "never declare `complete`" rule
  (RFC 05 §2.1) protects fan-in, where you want every answer. Content
  addressing is the opposite: any one answer is correct, and the nearest
  is best.

There is no central registry (brief §13). Every service mirrors the
contracts it implements, which is DDS XTypes' TypeLookup idea without the
per-endpoint discovery cost. Tools may also load bundles from a directory,
like zenctl's `--registry` today.

**Ordering (alive ⇒ callable).** The service declares, in this order:

1. its data publishers;
2. its state and operation queryables;
3. its descriptor queryable and its `complete` contract queryables;
4. only then, its presence tokens.

**The generic flow** (brief §9.1, §28):

1. `service list`: one liveliness GET.
2. `api show`: one descriptor GET, plus one contract-bundle GET (nearest
   holder, one reply) per unseen fingerprint.
3. `watch`: the contract resolves the resource to its key; subscribe;
   decode with the bundle's schema.
4. `call`: JSON in, encode via the schema, GET on the concrete `@op` key,
   decode the reply or the error envelope.

**Cost estimate, to be measured in the spike:**

- **Fleet size.** 200 systems × 20 services × 2.5 APIs (every service
  also implements `health.v1`) ≈ **10,000 tokens** of about 75 bytes,
  roughly 1 MB of declaration state.
- **Steady state.** Zenkey adds zero periodic traffic: no heartbeats, no
  refresh.
- **A full fleet inventory.** One liveliness GET (≈ 1 MB), descriptors only
  for what is displayed (≈ 1 KB each), and contracts once per distinct
  fingerprint (tens, ≈ 10–100 KB each, then cached forever).
- **Compared with v1.** The `introspect` fan-in at the same scale returns
  ≈ 36 MB *per query*, because it ships whole registry files from every
  producer.

**What propagates where**, read in the zenoh 1.10 source:

- **Routers.** Every router holds every token, routed over the link-state
  tree. So 10k tokens means 10k entries per router.
- **Clients and peers.** A client or peer face receives a token only if it
  registered a matching liveliness *interest*: declarations on demand,
  since zenoh 1.0.
- **What an application pays.** It subscribes only to
  `*/*/@zk/alive/*/<api>/*` for the APIs it uses, so it receives only
  those tokens.
- **What rmw_zenoh pays, by contrast.** Every process runs
  `liveliness_get(@ros2_lv/<domain>/**)` plus a history subscriber on it.
  Every process therefore holds every entity token of the domain, which is
  exactly the discovery overhead brief §18 wants to avoid.

**What the spike must establish:**

- **How long reconnection storms last.** A client that reconnects to
  another router re-propagates its tokens, and the old router drops them
  only at lease expiry (10 s by default). That can flap delete/put for
  observers.
- **Router state at 10k tokens.**
- **The liveliness-query deadlock.** zenoh#2678 (open) reports a
  `liveliness_query` deadlock past about 256 matching tokens on the default
  bounded handler. Tools must query with an explicit large handler until
  it is fixed.

#### 8.8 Truthfulness

**Exposure is computed from declarations.** The generated API makes every
endpoint a handle obtained before `start()`:

- a declared stream publisher is exposed even while idle (§4.4);
- a declared operation handler is exposed;
- a contract resource with `optional = true` may be declared
  `unavailable(reason)`;
- a non-optional resource that was not declared makes `start()` fail;
- an operation declared unavailable still answers, with `unavailable` and
  a reason, so that "absent from this build" and "disabled here" stay
  distinguishable (v1's RFC 08 §6.1 table, kept).

**The contract is served only from the build's own compiled bundle.** There
is no separate registry file to keep in sync, and no "publish static
registry separately" step that could diverge (brief §9.2).

**Tools can cross-check against Zenoh itself.** A querier's matching status
tells whether *any* queryable exists for a claimed operation, without
calling it. That is a cheap, Zenoh-native `doctor` check of
claims-versus-declarations.

#### 8.9 Compatibility

The classifier compares two canonical contracts of the same API major and
returns **compatible**, **review** or **breaking** per change, in the style
of smithy-diff or buf. Type changes are delegated to the schema kind's own
rules, evaluated in **both directions** (§5.4). Each new revision is checked
**against every revision in the major's history**.

| Change | Class |
|---|---|
| Add a resource | compatible |
| Add an optional field (per the schema kind's rules) | compatible |
| Deprecate a resource (kept and served until the next major; name never rebound within the major) | compatible |
| Docs-only change | none (not in the fingerprint) |
| Add an enum value | review (old readers may not handle it) |
| Change a semantic annotation (unit, range, `freshness.ttl_s`) | review |
| Change `idempotent`, `fanout`, `timeout_ms` | review |
| `optional` → required | breaking for implementers |
| Required → `optional` | review for clients |
| Remove a resource | breaking (requires a new major) |
| Change kind or parameters | breaking (requires a new major) |
| Incompatible type change in either direction | breaking (requires a new major) |

**History** is a set of committed canonical snapshots,
`contracts/.history/nav.v2/<sha256>.json`. The set is append-only, and CI
fails if a snapshot disappears: v1's `deprecated.lock` idea, generalized.
This answers brief §11 and §28 ("CI can compare two registry versions") and
removes v1's two lock files.

**JSON Schema compatibility** is not standardized. The core defines a
conservative subset with fixtures: added optional properties are
compatible, added required properties and type changes are breaking, and
anything the checker cannot judge is "review", never "compatible".

#### 8.10 Rust API

The generated code gives compile-time completeness for required endpoints.
The generic builder underneath remains for tools and dynamic cases. This is a
sketch: the names and signatures show the shape, not a settled API.

```rust
// build.rs
zenkey_build::Config::new().contracts("contracts").generate()?;

// server
let svc = zenkey::Service::new(&session, ServiceAddr::parse("vehicle-01/navigation")?);

let nav = nav_v2::Server::declare(&svc, nav_v2::Handlers {
    set_origin: |req: SetOriginRequest| async move { apply(req) },  // Result<Origin, NavError>
    reset: Unavailable::because("no IMU on this platform"),         // optional
}).await?;                                 // declares publishers, state writers and queryables
let health = health_v1::Server::declare(&svc, health_v1::Handlers::default()).await?;

let svc = svc.start().await?;              // descriptor + contracts, then presence tokens

nav.position.put(&fix).await?;             // StreamWriter<Position>
nav.status.set(&Status::Tracking).await?;  // StateWriter<Status>: puts, and answers GET
```

```rust
// client: always an explicit target
let nav = nav_v2::Client::new(&session, ServiceAddr::parse("vehicle-01/navigation")?);
let mut fixes  = nav.position().subscribe().await?;    // Stream<Item = Result<Position>>
let status     = nav.status().get().await?;            // Result<Option<Status>>
let origin     = nav.set_origin(&req).await?;          // Result<Origin, CallError<NavError>>

// fleet: only by name
let mut all = nav_v2::Fleet::all(&session).position().subscribe().await?;  // (ServiceAddr, Position)
```

Zenoh stays reachable (brief §12.2):

- every handle exposes `key_expr()`;
- declaration builders accept a closure over the underlying zenoh builder
  (priority, congestion control, express, locality, an advanced publisher);
- the session is always the caller's.

zenkey never owns the session, and never wraps the selectors or queriers a
user wants to use directly.

#### 8.11 Security

The key layout gives every grant a prefix to name:

| Principal | Allowed |
|---|---|
| Ordinary application | Declare subscriptions on `…/stream/**`, `…/state/**` of the APIs it uses. Query on `…/state/**`. Query only the specific `…/@op/<op>` keys it needs. Liveliness subscription on `*/*/@zk/alive/*/<api>/*` to find its peers. |
| Operator | All of the above, plus query on `*/*/@zk/instance/*` and `@zk/contract/**` (or per API: `@zk/contract/nav.v2/*`). |
| Administrator | Query on privileged `@op` keys, listed per operation. |
| A service's own principal (cert CN bound to its system) | Put, delete, declare publisher and declare queryable on `vehicle-01/**`, `vehicle-01/*/*/@op/**`, `vehicle-01/*/@zk/**` (three rules, as v1's "one rule per plane"). Plus declare queryable on `@zk/contract/<api>/*` for the APIs it implements. |

On top of that table:

- **Deny rules use the widest pattern** that can reach a plane (`**/@op/**`),
  per RFC 09 §3 fact 6.
- **Servers refuse non-concrete calls** to operations whose contract does
  not allow fan-out.
- **ACL generation stays a tool:** contracts plus a role file produce the
  JSON5, as v1's `zenctl acl gen` already does from an enrollment file.
  Operations may carry an `access` annotation (`admin`) that the generator
  reads. The core itself stays free of authorization policy (brief §14).

Three Zenoh facts (1.10 source) bound what the ACL can promise. The spec
should state them rather than imply more:

- **Multicast transports bypass access control entirely.** A deployment
  that relies on ACLs must not carry protected traffic over multicast.
- **Only some subjects are authenticated.** `cert_common_names` and
  `usernames` are backed by authentication. `zids` is not, and Zenoh's
  own default config says so. Bind a system to a certificate CN, never
  to a zid.
- **ACL is enforced per hop, by the adjacent face.** In peer-to-peer mode,
  every peer enforces its own ACL. Also, since zenoh 1.3, a query refused
  by ACL gets an *empty* reply rather than a timeout. Tools must therefore
  not read an empty reply as "no such operation" (v1's silence rule, RFC
  05 §3.1, still applies).

#### 8.12 Profiles are mostly standard contracts

| v1 convention | v2 home |
|---|---|
| Health document, framework state set | `health.v1` standard contract |
| Configuration (RFC 05 §5.1) | `config.v1` standard contract |
| Alerts (RFC 04 §1.2) | `alarms.v1`: state per active alarm plus a stream of transitions ("alerts are state" kept as guidance) |
| Sensor registration, capabilities | the core instance descriptor |
| Telemetry semantics (unit, counter/gauge, histogram buckets, cardinality budget) | `telemetry.*` annotation vocabulary |
| The five QoS profiles, router overwrite recipe | `qos.*` annotations plus an ops guide |
| TTL, refresh, aging, seed entitlements | `freshness.*` annotations; advanced-publisher history as opt-in |
| `@media` | Media profile: a registered verbatim kind plus a frame-metadata contract (spec text: frame clock, receiver-driven adaptation) |
| `@blob` | Bulk profile: a registered verbatim kind, content addressing, integrity rules (spec text) |
| Catalog, evidence, entities, incidents, edges, impact | ZenSight application contracts, in ZenSight's repository |
| Observer conformance, `.zrec`/`.zsnap`, synthetic marker, cutover | Tooling guide, versioned with the tools |

Only media and bulk need spec chapters. Everything else is data (contracts
and annotation vocabularies) plus guides. That is how the core stays at
about 20 pages.

#### 8.13 Redundancy

**Core rule:** at most one *active* instance per service (§5.1). The
descriptor carries `role` (`active` | `standby`), and tools flag two active
descriptors as split-brain. Detection goes through descriptors, not through
samples:

- Zenoh's per-sample `SourceInfo` is unstable.
- It is set only by advanced publishers.
- It is "passed along without validation".

So a plain publisher's samples cannot be attributed to an instance.

**A redundancy profile** may add:

- election over liveliness (for example a `@zk/lease` token, lowest
  instance id wins);
- a fencing epoch in an attachment.

It must restate v1's rule that a side-effecting actuator needs an
exclusivity lock outside the bus. Stateless load-balanced operation serving
(several active instances, operations only) is deliberately left out of v1
of the core, and listed as an open question.

---

### 9. Inventory: what happens to each v1 concept

| v1 concept | Purpose | Verdict | v2 home |
|---|---|---|---|
| Base = session namespace | Deployment isolation | **Keep** | Optional deployment prefix |
| `v1` convention chunk | Convention-major isolation | **Delete** | API major in the API chunk; the convention version lives in `@zk` documents |
| Host origin `h-<12hex>`, `AppProfile` salt | Self-minted physical identity | **Remove from keys** | Descriptor metadata; observability deployments may still use system = host id |
| Service origin `@catalog` | Singleton services | **Replace** | Ordinary `<system>/<service>` |
| Producer chunk + `-<int>` instance | Component identity | **Replace** | Logical service + instance id (presence only) |
| Classes `telemetry`/`state`/`events` | Update semantics + infra selection | **Redesign** | Kinds `stream`/`state`; telemetry and event semantics → annotations, `alarms.v1` |
| `@rpc` plane | Hermetic request/reply | **Keep the idea** | `@op` |
| `@media`, `@blob` | Frames, bulk | **Move** | Profiles with registered verbatim kinds |
| Subject paths, `{var}`/`{var...}` templates | Meaning path | **Keep** | Resource path templates inside an API |
| Slugging (`x-` escape) | Foreign values in chunks | **Keep** | Core, with its fixtures |
| Registry per producer, TOML **and** KDL | Inventory | **Redesign** | Per-API contracts, TOML only, canonical JSON |
| Framework state set, `common` tokens | Shared subjects | **Replace** | Standard contracts (`health.v1`…) |
| `introspect` (registry file over `@rpc`) | Runtime inventory | **Redesign** | Presence tokens + descriptor + content-addressed bundles |
| `describe` (SchemaSet) | Schemas | **Keep the idea** | Inside the contract bundle; normalized hashes; committed artifacts |
| `registry.lock`, `deprecated.lock` | Compatibility | **Redesign** | Contract history + FULL-transitive classifier |
| `when` predicates, `error/gated` and `error/unsupported` | Conditional surfaces | **Simplify** | `optional` + `unavailable(reason)` in the descriptor and error |
| `conditional.lock` | Legacy gating ledger | **Delete** | n/a |
| Five QoS profiles | Delivery defaults | **Move** | `qos.*` annotations, ops guide |
| Delivery contracts (seed/detect/replay), TTL/refresh | Late joiners, staleness | **Split** | State GET in core; the rest → `freshness.*` |
| Alive token per producer | Presence | **Redesign** | Per (instance × API) under `@zk/alive` |
| Sensor registration document, capabilities | Rich presence | **Replace** | Descriptor |
| Alert family, alert-key hash, `alert_ref` | Alerting | **Move** | `alarms.v1` (generic); the hash recipe stays with ZenSight |
| Catalog, evidence, entities, aliases, incidents, ack, silence, edges | Identity fusion, incidents | **Move out** | ZenSight |
| Configuration convention (RFC 05 §5.1) | Commit-confirm remote config | **Keep, as a contract** | `config.v1` |
| Procedure kinds read/write/long-running, `idempotent`, `fanout` | Operation metadata | **Keep (trimmed)** | `idempotent`, `fanout`, `timeout_ms`; long-running as a pattern |
| Fan-in discipline | Correct fleet queries | **Keep** | Core calling rules |
| Error vocabulary `error/*`, `[[error]]` entries | Typed failures | **Redesign** | Core envelope + contract error type |
| Typed origins (Local/Remote/Fleet) | Address-confusion bugs | **Keep the lesson** | No implicit identity; `Fleet` by name |
| Observer conformance (judgement, exit codes, silence rule) | Tool honesty | **Move** | Tooling guide (the judgement core code is generic and stays) |
| `.zrec`, `.zsnap`, synthetic marker, cutover | Capture, test, migration | **Move** | Tooling |
| ACL / storage / link recipes and planners | Operations | **Keep as tooling** | Regenerated from contracts |
| zenkey-build lints | Registry validity | **Redesign** | Language-independent contract checks with fixtures |
| zenkey-fleet *generic* layers (session, `fleet_get`, monitor, admin, tape, judgement, structural decode) | Tool engine | **Keep** | Unchanged |
| zenkey-fleet *v1-bound* layers (facts, SliceSet, roster, doctor, why, conform, planners, infer, generators) | v1-aware tooling | **Port** | Re-target to v2 discovery and contracts |
| zenctl, zengui, zenwatch | Tools | **Keep, re-target** | Same engine |
| RFC 11 ZenSight profile, `alert.rs`, `errors` token | Reference app | **Move out** | ZenSight repository |

---

### 10. Plan

| Phase | Output | Gate |
|---|---|---|
| **0. Decide** | Answers to §11 | The maintainer's call |
| **1. Spike** (≈ 2 weeks, throwaway) | Grammar build/parse; 10k synthetic presence tokens across 2 routers and clients; router restart; contract fetch by hash; ACL behaviour on `@op`/`@zk`; namespace × verbatim chunks; state GET on templates | **G1:** measured numbers replace the §8.7 estimates; any surprise changes the design *before* the spec |
| **2. Core spec + fixtures** | `spec/core.md` (≤ 20 pages); `spec/conformance/*.json` covering keys, slugs, canonical contracts, fingerprints, normalized schema hashes, compatibility cases | Every MUST has a fixture |
| **3. `zenkey-model`**, plus a ~500-line Python implementation of the same fixtures and of presence/descriptor/call over zenoh-python | Two implementations, one spec | **G2:** "implementable from the spec alone" (brief §28) made falsifiable |
| **4. `zenkey` runtime + `zenkey-build`** | Server/client/fleet API; `nav.v2`, `health.v1`, `imu.v1` examples; failover and move-host scenarios as integration tests | **G3:** brief §19 scenarios pass as tests |
| **5. Tooling** | zenctl `service list`, `api show`, `watch`, `get`, `call`, `schema`, `compat` on zenkey-fleet's generic engine; port doctor and why | **G4:** discover → inspect → decode → call on an application the tool has never seen |
| **6. Standard contracts and profiles** | `config.v1`, `alarms.v1`, telemetry/qos/freshness annotations; then media and bulk | Each profile independently versioned |
| **7. Adopters** | tcgui port (small, a real validation), then ZenSight (or an explicit freeze on v1); zengui and zenwatch re-targeted | **G5:** one production adopter on v2 |

**v1 posture during the work:**

- freeze the RFC set at v1.50, errata only;
- keep the 0.14.x line for adopters;
- reserve the system name `v1`, or require distinct namespaces, so v1 and
  v2 can share a bus during migration.

**Risks:**

| Risk | Mitigation |
|---|---|
| Second-system effect | Page budget, a measured spike, a real adopter as a gate |
| Tooling rewrite cost (150k lines assume v1) | Keep the generic engine layers; port only the v1-aware adapters |
| prost ergonomics versus serde structs | JSON Schema remains a first-class kind for Rust-first teams |
| Unstable zenoh surfaces | Core uses stable API only (RFC 03 §0.1 posture kept) |
| Losing v1's hard-won lessons | §6 as a checklist in the spec's rationale appendix |
| Single-maintainer governance for a company foundation | A core small enough that an organization can own it |

---

### 11. Decisions for the maintainer

1. **ZenSight.** Port it to v2, or freeze v1 as the observability
   convention it really is? *Recommendation:* freeze v1 at 1.50, port
   tcgui first, and decide ZenSight after phase 5.
2. **Mandatory `<system>` chunk.** Accept one constant chunk for
   single-system deployments? *Recommendation:* yes.
3. **Kind in the key** (`stream`/`state`/`@op`). Accept one chunk for
   infrastructure selectability and GET safety? *Recommendation:* yes.
4. **Default schema kind.** Protobuf-first, or JSON Schema (Rust-first
   authoring)? *Recommendation:* protobuf for inter-team APIs; JSON Schema
   allowed; both blessed.
5. **Single active writer as a core rule**, with election left to a
   profile? *Recommendation:* yes.
6. **Repository and naming.** Same repository with new crates under `v2/`
   and the old crates frozen, or a new repository? Does `zenkey` jump to
   1.0? *Recommendation:* same repository, new crates, `zenkey` 1.0 for
   the v2 runtime.
7. **The pilot.** Which real company service validates phase 4 alongside
   the examples?

---

### Sources

**Repository** (at `c7ef1b4`, 0.14.0):

- `rfcs/00`–`13` and `rfcs/CHANGELOG.md`: v1.0 2026-07-12 → v1.50
  2026-10-05.
- `docs/redesign-2026-07.md`: the previous redesign; its round-2 owner
  directives still apply: latest zenoh, strong typing, performance as a
  requirement, lib crates only on crates.io.
- `zenkey/src/{lib,context,alert,common_state,schema}.rs`.
- `zenkey-build/src/emit.rs`.
- `zenkey-fleet/src/bus/{producer,roster,query}.rs`.
- `fixture-tests/registry/*.toml`: 105 KB over 12 producer files.

**Zenoh, read in the 1.10.1 source** (`github.com/eclipse-zenoh/zenoh`, tag
`1.10.1`):

- `commons/zenoh-protocol/src/network/declare.rs`: a token is
  `{id, wire_expr}`, with no payload.
- `zenoh/src/net/routing/hat/{router,client,peer,broker}/token.rs`:
  every router holds every token; clients and peers get tokens on interest.
- `commons/zenoh-keyexpr/src/key_expr/borrowed.rs`: verbatim means "starts
  with `@`"; `nav@2` is an ordinary chunk.
- `zenoh/src/net/routing/dispatcher/queries.rs`: `BestMatching` picks the
  nearest complete queryable that includes the query, else `All`.
- `commons/zenoh-codec/src/core/encoding.rs`: the schema suffix is ≤ 255
  bytes and sent per sample.
- `zenoh/src/net/routing/interceptor/{access_control,authorization}.rs`:
  inclusion matching; multicast transports bypass ACL.
- `zenoh/src/api/{sample,matching}.rs`: `SourceInfo` is unstable and
  unvalidated; matching status is a boolean.
- `zenoh/src/net/runtime/adminspace.rs`, `zenoh-ext/src/advanced_*.rs`.

**Zenoh issues and docs:**

- Liveliness: eclipse-zenoh/zenoh#1637 (token payload request), #2678
  (`liveliness_query` deadlock past ~256 tokens), #2248, #2602.
- Key expressions: `eclipse-zenoh/roadmap` `rfcs/ALL/Key Expressions.md`.
- Interests: https://spec.zenoh.io/spec/1.0.0/session/interests.html and
  https://zenoh.io/blog/2024-10-21-zenoh-firesong/.
- Discovery measurement: https://zenoh.io/blog/2021-03-23-discovery/.
- ACL: https://zenoh.io/docs/manual/access-control/.

**Prior art:**

- Keelson:
  https://github.com/RISE-Maritime/keelson/blob/main/docs/protocol-specification.md
  (HEAD 6215eb7).
- uProtocol: https://github.com/eclipse-uprotocol/up-spec (`basics/uri.adoc`,
  `basics/uattributes.adoc`, `up-l1/zenoh.adoc`, uDiscovery v3; main
  c89dc48).
- NATS: https://github.com/nats-io/nats-architecture-and-design/blob/main/adr/ADR-32.md
  (SCHEMA removed in commit 5d18533).
- D-Bus: https://dbus.freedesktop.org/doc/dbus-specification.html.
- Sparkplug: https://github.com/eclipse-sparkplug/sparkplug (chapters 4–5).
- DDS XTypes: https://www.omg.org/spec/DDS-XTypes/20190301/dds-xtypes_typeobject.idl
  and https://community.rti.com/static/documentation/wireshark/2026-04/doc/type_discovery.html.
- ROS 2 type descriptions: https://github.com/ros-infrastructure/rep/pull/381
  and https://github.com/ros2/rmw_zenoh/blob/rolling/docs/design.md
  (and rmw_zenoh#171).
- gRPC reflection: https://github.com/grpc/grpc-proto/blob/master/grpc/reflection/v1/reflection.proto.
- buf breaking: https://buf.build/docs/breaking/rules/.
- AsyncAPI 3.x and asyncapi/bindings (no Zenoh binding; ros2 binding
  PR #270).
- COVESA: https://github.com/COVESA/vehicle_signal_specification and
  https://github.com/COVESA/ifex.
- MCAP: https://github.com/foxglove/mcap/blob/main/website/docs/spec/registry.md.
- zrpc: ZettaScaleLabs `zrpc` 0.8.17.
