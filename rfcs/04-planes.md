# 04 — Data Classes and Planes

**Status: v1.0 (ratified)** · normative chapter · *amended in v1.4, v1.5, v1.12 and v1.25 — see [CHANGELOG.md](CHANGELOG.md)*

The `<class>` position ([03-grammar.md §1.4](03-grammar.md)) splits the
keyspace into three **data classes** — `telemetry`, `state`, `events` —
and three **verbatim planes** — `@rpc`, `@media`, `@blob`. This chapter
defines what each class *means*: its update semantics, its cardinality
budget, its QoS defaults, and which storage shape captures it. The verbatim
planes get their own chapters ([05](05-control-rpc.md),
[07](07-bulk-planes.md)); this one covers the three data classes and the
rules for deciding where a given piece of information belongs.

---

## 1. The three data classes

| | `telemetry` | `state` | `events` |
|---|---|---|---|
| A sample is… | a measurement at an instant | the current truth of one key | an occurrence that happened |
| Update model | superseded — the next sample replaces the last | last-writer-wins on one stable key | immutable — a key is written once, never updated |
| Delete (`SampleKind::Delete`) | meaningless; MUST NOT be sent | **meaningful** — a tombstone retiring the key | meaningless; MUST NOT be sent |
| Missing a sample costs… | one point on a chart | *the truth* — until the next refresh | the occurrence, forever |
| Key cardinality | bounded, enumerable per producer | bounded and enumerable, **or** population-keyed under an explicit registry budget (§1.2) | unbounded over time (unique id per event), bounded **per rate budget** |
| Subject example | `cpu/usage` | `health`, `alert/<key>` | `capture/<ulid>` |

### 1.1 `telemetry` — superseded time-series

- Numeric or low-cardinality measurements published on a cadence. The key
  set of a producer is stable and enumerable; only values change.
- A publisher MUST publish each metric on its own stable key; it MUST NOT
  encode values, timestamps, or sequence numbers in the key.
- A late joiner on a cadence-published metric (position, temperature,
  a counter) needs no seeding: **the next publication is the seed** —
  `seed = none` is the class default (§3.1), and on constrained links it
  is also the right answer (zero standing machinery, zero seed traffic).
  The rare subject that genuinely needs a tail on arrival declares
  `seed = tail(n)` in the registry and gets it from a seed source
  (§3.1) — never from re-publishing on the data key, which corrupts the
  time-series.

### 1.2 `state` — last-writer-wins with tombstones

- Anything whose *latest value is the truth*: health, liveness documents,
  configuration echoes, identity evidence, alerts, catalog entities.
- The key MUST be stable for the lifetime of the stateful thing, so that
  firing→resolved→gone is a sequence of writes and one delete on a *single*
  key — not a family of keys to garbage-collect.
- **TTL & tombstones — the staleness contract.** Stated once, here;
  every other chapter references this table. The TTL value is the
  registry's `ttl_s` ([08-registry.md §2](08-registry.md)), authoritative
  for all parties (never a locally configured one; the reference
  application uses 60 s re-emission against a 900 s evidence TTL):

  | Rule | Binds | Requirement | Enforced by |
  |---|---|---|---|
  | refresh | publisher | re-emit live state at ≤ `ttl_s`/2 — the self-heal | registry review |
  | aging | consumer | state older than `ttl_s` is stale; producer's `alive` token absent ([§5](#5-liveliness-presence)) ⇒ suspect immediately, stale at TTL — this retires a firing alert whose publisher crashed without tombstoning | liveliness roster |
  | retirement | publisher | retire a key with a Zenoh delete (`SampleKind::Delete` — never a payload marker); consumers treat it as authoritative | class semantics (§1) |
  | tombstone visibility | deployment | a delete stays observable ≥ `ttl_s`; seed replies (§3.2) never present a deleted key as live | storage `gc.lifespan` ≥ max `ttl_s` ([09-operations.md §2.3](09-operations.md)) |
- **Explorer-initiated tombstones (v1.12).** The retirement rule above binds
  the key's *publisher*; an explorer or operator tool MAY nevertheless issue
  a tombstone on any **concrete** key as a deliberate operator act — retiring
  a test key, purging a stray key from storage. On `telemetry`/`events` keys
  this is keyspace cleanup, not a data-class semantic (§1's MUST NOT still
  binds conforming publishers of those classes), and tooling MUST require an
  explicit operator confirmation before sending one there. A wildcard delete
  is not an operator act and MUST be refused outright.
- **Cardinality budget.** A state subject keyed by an *observed population*
  (one key per seen IP, device, unit — e.g. `evidence/names/<ip-slug>`,
  `@catalog/state/pdns/<ip-slug>`) is permitted only with an explicit
  registry budget: the entry MUST declare the expected population bound and
  an aging rule (entries past TTL are tombstoned by their publisher).
  Unbudgeted population-keyed state is a registry-review reject — it is the
  loophole through which the "bus is low-cardinality" corollary (§2) and
  the affordable-firehose story (D2) would otherwise leak.
- **No cross-key atomicity.** The bus cannot deliver multi-key transactions
  and the convention does not simulate them: every state key MUST be
  independently coherent — a consumer acting on one key in isolation may be
  briefly *incomplete*, never *wrong*. Where a transition spans keys (the
  catalog merge writes entity + alias + tombstone,
  [06-identity.md §5](06-identity.md)), the writer MUST order writes so
  every intermediate is safe, and consumers MUST tolerate the torn window.
  Anything needing true snapshot semantics must be one document on one key.
- **Oversized values.** State values SHOULD stay small (they ride
  firehoses and seeds). A subject whose value can grow large MAY register
  `delivery = "invalidate"` ([08-registry.md §2](08-registry.md)): the
  published document carries identity + version/content-hash + summary
  only, and consumers pull the body on demand (`@rpc` read or `@blob` by
  hash) — D-Bus's `invalidated_properties` pattern
  ([10-prior-art.md](10-prior-art.md)).

**Alerts are state.** An alert has a stable identity key
`state/<producer>/alert/<alert_key>`, transitions firing → resolved on that
one key, and is retired by tombstone. `<alert_key>` is normatively: **a
stable, origin-excluded hash of rule identity + discriminating labels,
byte-precise per application profile** — stable, so that
firing → resolved → retired is one key's history; origin-excluded (the
producing source is *not* hashed, and host-scoped labels are excluded with
it), because origin and producer are already in the key, which is what
makes the key origin-scoped. The hash function, its exact input framing,
and the rendered width are **profile constants**: an application's profile
chapter fixes them with the same byte precision
[06-identity.md §1](06-identity.md) gives the origin derivation, so two
producers of one application mint the same key for the same alert (the
reference profile's recipe and test vector:
[11-zensight-profile.md §3.1](11-zensight-profile.md)). Modelling
alerts as events would force every consumer to re-derive "what is firing
now" from an unbounded log — the exact query the class system should answer
with one selector: `<base>/v1/*/state/*/alert/*`.

### 1.3 `events` — immutable occurrences

- Discrete, low-rate happenings that are *records*, not measurements:
  capture triggered, artifact generated, unit entered failed state, config
  applied.
- Each event key MUST end in a unique, time-sortable id (ULID recommended,
  key-encoded lowercase per [03-grammar.md §2](03-grammar.md)) and MUST be
  written exactly once (a publisher attestation — no checker can observe
  it).
- `events` is a **budgeted** class: the registry entry for an event subject
  MUST declare its rate class in the `rate` field
  ([08-registry.md §2](08-registry.md): `rare` ≤ 1/h · `low` ≤ 1/min ·
  `burst(n/h)` a declared cap), and per-record streams that can burst
  unboundedly (log lines, flow records, packets) MUST NOT be events — they
  are served on demand via `@rpc` (rule R3 below). Events exist so that
  *rare, meaningful* occurrences survive verbatim; they are not a log
  transport.
- Retention/replay of events is a storage-deployment concern — the events
  storage is the normative contract, decided in
  [12-open-questions.md §4](12-open-questions.md); the bus contract is
  only immutability + unique keys.

### 1.4 The framework state set (normative, v1.25)

A handful of `state` subjects recur on **every** producer — this chapter
and [06](06-identity.md) have named each of them since v1.0, but the set
itself was never written down in one place, and the registry's `common`
field ([08-registry.md §2](08-registry.md)) needs a definition to point
at. This is that place. The **framework state set** is:

| Subject (under `state/<producer>/`) | `common` token | Defined |
|---|---|---|
| `health` | `health` | the producer health document (§1.2; the identity bridge rides it, [06 §6.2](06-identity.md)) |
| `sensor` | `sensor` | the registration document (§5) |
| `alert/{alert_key}` | `alert` | the alert family (§1.2) |
| `evidence/self` | `evidence_self` | the producer's own identity claim ([06 §4](06-identity.md)) |
| `evidence/device/{device}` | `evidence_device` | an observed device's identity claim ([06 §4](06-identity.md)) |
| `evidence/names/{ip_slug}` | `evidence_names` | a passive-DNS name observation ([06 §4](06-identity.md)) |

plus the `@catalog` **service** subjects — `entity/{entity_id}`,
`alias/{old_id}`, `pdns/{ip_slug}` (tokens `entity`, `alias`, `pdns`;
[06 §5](06-identity.md)) — which are one service's state rather than a
family across producers. `alive` is deliberately **not** in the set: it
is presence, not a state subject (§5, [03-grammar.md §3](03-grammar.md)),
and has no `common` token.

Three rules make the set usable rather than decorative:

- **The spelling is the table's.** A registry entry declaring
  `common = "<token>"` MUST use the token's subject pattern exactly as
  written above — the token is a claim that this entry *is* that
  framework subject, and the generated framework grouping
  ([08-registry.md §2](08-registry.md)) dispatches on it.
- **The neutral set is closed here.** New neutral tokens arrive by
  amendment to this table (or to 06's, for catalog subjects), like any
  other convention vocabulary.
- **Profile tokens extend it, in the profile chapter.** A framework
  subject an application adds beyond this set (ZenSight's `errors`,
  [11-zensight-profile.md §2](11-zensight-profile.md)) is declared in
  that application's profile chapter with the same table discipline; it
  is real vocabulary for that application and invisible to every other.

---

## 2. Placement rules

The class of a subject is fixed by the registry, decided with these rules:

- **R1 — Is the latest value the whole truth?** → `state`.
  (Health, liveness, evidence, alerts, stream status, entity documents.)
- **R2 — Is it a measurement where the next sample makes the last one
  obsolete?** → `telemetry`. (Counters, gauges, rates, rollups.)
- **R3 — Is it an occurrence that must not be lost or rewritten, at a rate
  a human could review?** → `events`. If the rate is machine-scale
  (per-line, per-flow, per-packet), it is **not** publishable: hold it in a
  bounded ring at the producer and serve it via `@rpc` on demand.
- **R4 — Is it opaque bytes at frame rate?** → `@media`.
- **R5 — Is it bulk bytes with an identity (file, tree, chunk)?** → `@blob`.
- **R6 — Is it a question or an instruction?** → `@rpc`. Nothing under the
  data classes is ever a command; the data planes are strictly
  producer→consumer. (The one sanctioned exception is durable **desired
  state** authored by a registered service origin on behalf of a target,
  which the target then reconciles — still a `state` document, not a
  command; [07-bulk-planes.md §3](07-bulk-planes.md),
  [12-open-questions.md §3](12-open-questions.md).)

A corollary worth stating: **the bus is low-cardinality by budget.**
Everything high-cardinality is pull-only (`@rpc`), content-addressed
(`@blob`), rate-budgeted (`events`), or population-budgeted state
(§1.2) — and the last two budgets live in the registry, where review can
refuse them. "By construction" would overclaim: the grammar cannot stop an
unbudgeted key family; the registry can.

---

## 3. QoS defaults per class

QoS is declared per subject as a **named profile** in the registry
(`qos = "<profile>"`, [08-registry.md §2](08-registry.md)); each class has
a default profile. The profile vocabulary is closed — these five, mapping
to Zenoh reliability × congestion control × priority:

| Profile | Reliability | Congestion | Priority | Express | Default for |
|---|---|---|---|---|---|
| `sampled` | best-effort | drop | data-low | no | `telemetry` |
| `refreshed` | best-effort | drop | data | no | `state` that self-heals (see rule below) |
| `transition` | reliable | block | data | no | `state` written on transition; `events` |
| `alert` | reliable | block | interactive-high | **yes** | `state/*/alert/*` |
| `frame` | best-effort | drop | interactive-high | **yes** | `@media` (a stale frame is worthless; the encoder must never block) |

**The `express` axis (v1.5).** Zenoh's per-message `express` flag bypasses
transport batching for lower latency at the cost of batching efficiency. It
is a fourth axis of the profile table, not a per-key knob: `alert` and
`frame` — the two profiles whose whole point is latency — set it; the three
throughput-shaped profiles do not. The vocabulary stays closed at five
profiles; the rejected alternative (a per-key `express` override in the
registry) is recorded in the v1.5 changelog — it would reopen the exact
per-key QoS bikeshed the closed profile set exists to prevent.

Note that the `alert` row's default binds a *family*, not a class, and the
registry's per-class default mechanism cannot see a family: an alert-family
subject therefore **declares** `qos = "alert"` in its registry entry rather
than relying on any default, and the registry lint enforces it
([08-registry.md §2/§5](08-registry.md), v1.23) — a silent fall to the
`state` class default would make the firing flank drop-eligible on the one
family the express axis exists for.

The `refreshed`/`transition` split inside `state` is about the **cost of
waiting out a missed write**, not about self-healing: *all* live state
refreshes at ≤ TTL/2 (§1.2), so every state subject eventually self-heals.
State whose value changes on rare transitions that consumers cannot afford
to learn a refresh-period late (an alert flank, an entity tombstone, a
config echo) MUST use `transition` or stronger — the reliable profile buys
the *latency* of truth; the refresh mandate already guarantees its
eventuality.

The query planes have no publisher-side profile — **Zenoh replies inherit
the QoS of the query** (the server-side reply-QoS setters are documented
no-ops). The obligations are therefore client-side: `@rpc` callers SHOULD
issue GETs at interactive-high priority; `@blob` callers MUST issue GETs at
data-low — that, not anything the responder does, is what makes bulk
transfer yield ([07-bulk-planes.md §2](07-bulk-planes.md)).

All publishers on `telemetry`, `state`, and `@media` MUST be *declared*
publishers — never one-shot ad-hoc puts — so keys are interned, routing is
primed, and QoS is attached once ([02-principles.md P7](02-principles.md)).
Two scoped exemptions, both for write-once keys where interning buys
nothing: seeding a router-hosted `@blob` content store
([07-bulk-planes.md §2](07-bulk-planes.md)), and **`events` publication** —
every events key is written exactly once (§1.3), so there is no routing to
prime and no key to reuse; events are one-shot puts carrying their QoS
profile explicitly, kept cheap by the class's rate budget. (A declared
publisher per event key would be a declaration plus a put plus an
undeclare — the same wire cost wearing a compliance costume.)

**Profiles can be deployment-enforced.** Zenoh's `qos` overwrite
interceptor (stable config) rewrites priority/congestion/express per
keyexpr at the router, ignoring whatever the API set. Because the class is
a fixed key position, a deployment MAY pin the profile table above as
router policy — one overwrite rule per class prefix
([09-operations.md §4](09-operations.md)) — turning publisher discipline
into infrastructure guarantee. This is the QoS counterpart of storage
selection: one more infrastructure concern the grammar made a config file.

**Encoding declaration (v1.5).** Publishers SHOULD set the middleware
`Encoding` on every sample, using the predefined MIME-ish constants
(`application/cbor`, `application/json`, `application/protobuf`) — pure
metadata, free on the wire, and the first thing a generic consumer
consults; the registry MAY declare a per-entry `encoding`
([08-registry.md §2](08-registry.md)). Resolution order is
**sample > registry > sniff** ([08-registry.md §7](08-registry.md)).

### 3.1 Delivery contracts per class

QoS says how samples travel; this section says what consumers are
**entitled to** — and deliberately not which library delivers it. The
entitlements live in the registry beside QoS
([08-registry.md §2](08-registry.md)); *mechanisms* are a deployment/build
choice, exactly as QoS profiles are enforceable by router config:

| Entitlement | Registry field | Meaning | Class defaults |
|---|---|---|---|
| **Seed** | `seed = none \| latest \| tail(n)` | what a late joiner may obtain per key without waiting out a cadence | `state`: `latest` (mandatory) · `telemetry`: `none` (a blank chart until the next cadence conforms) · `events`: n/a (see replay) |
| **Miss detection** | `detect_s` (`state` only; default = `ttl_s`) | the maximum time within which a consumer can *detect* a missed transition | baseline meets `detect_s = ttl_s` for free (refresh + aging + liveliness); smaller values need the advanced tier (§3.3) |
| **Replay** | `replay = none \| window(t)` (`events` only) | how far back events are queryable | satisfied at deployment level by the events storage |
| **Tombstone visibility** | (from `ttl_s`) | a delete is observable ≥ TTL (§1.2) | enforced by storage GC sizing ([09-operations.md §2.3](09-operations.md)) |

Two universal rules, mechanism-independent:

- Late-joiner support MUST NOT be implemented by re-publishing on the data
  key — it corrupts the time-series and the LWW record; seeds come from a
  seed source, never from fake samples.
- **A deployment MUST provide at least one seed source for every `state`
  subject** — a latest-value storage covering `state/**`
  ([09-operations.md §2](09-operations.md)), publisher-side caches (§3.3),
  or both. A deployment with neither has no late-joiner story at all,
  which does not conform. The burden this MUST creates is allocated in two
  halves (v1.25): *which* mechanism discharges it is the **deployment's**
  choice — the deployment is the party that knows whether a storage runs
  (§3.5 states the default) — while the registry's per-subject `seed`
  declaration is how a **producer** knows what is asked of it: in a
  deployment that runs no latest-value storage, the producer of a
  seed-entitled subject is the only possible seed source, and MUST cache
  that subject (§3.3).

Telemetry loss needs no detection machinery: a dropped sample is priced
into the `sampled` profile and superseded by the next cadence — spending
bytes to detect drops the QoS invited is waste (consumers wanting a loss
*metric* can count, §3.3).

### 3.2 Baseline mechanics (stable API — the default)

The baseline is what every conforming participant runs unless a subject's
entitlements say otherwise. It uses nothing unstable:

- **Publishers**: one plain declared publisher per key with the class QoS
  profile (§3), plus one-shot puts for `events` (§3 exemption).
- **Presence & staleness**: one liveliness token per producer
  (`state/<producer>/alive`, §5) — *not* per key. A consumer marks a
  producer's state suspect on token retraction and stale at TTL; the
  mandated refresh (≤ TTL/2, §1.2) is the self-heal. Together these meet
  `detect_s = ttl_s` with zero per-key machinery.
- **Seeding**: the seed discipline below.
- **Events**: reliable one-shot puts; replay and post-crash visibility are
  the events storage's job.

**Seed discipline** (normative, both tiers — this is the single home;
[05-control-rpc.md §4](05-control-rpc.md) explains only why dedicated
seed *procedures* no longer exist):

- *Subscribe first, reconcile by timestamp.* A consumer MUST declare its
  subscriber before issuing the seed GET, and MUST merge seed replies with
  live samples per key by Zenoh (HLC) timestamp — newer value wins, newer
  tombstone wins. GET-then-subscribe is forbidden: a transition published
  in the gap is silently dropped, and a dropped delete is a resurrected
  key. Corollary: state publishers and storages MUST run timestamped — an
  untimestamped sample cannot be reconciled.
- *Two seed paths, answering from different places.* (1) An
  AdvancedSubscriber with `history()` (§3.3) seeds from live publishers'
  `@adv` caches — no storage needed, reconcile internal. (2) A plain GET
  on the state selector is answered only by a router storage — publisher
  caches live under the verbatim `@adv` sidecar a plain GET cannot reach.
  Which path is the deployment's default is §3.5's one rule (storage
  where one runs; caches where none does). They differ in *coverage*: a
  cache dies with its publisher, a storage does not. A consumer whose correctness depends on state from **crashed**
  producers (a UI rendering the firing alert of a dead host — the case
  §1.2's TTL retirement exists for) MUST include the storage seed where
  one is deployed; cache seeding alone suffices only where dead producers'
  state may lapse until TTL.
- *Composition.* A consumer running both paths issues the storage GET
  first (or concurrently), declares the AdvancedSubscriber last on the
  session (§3.3's ordering note; its declare-time history query is
  internally race-free), merges by the same timestamp rule — and, in a
  fleet that also contains baseline (cache-less) publishers, re-issues the
  seed GET once after the declare, since no history query can replay a
  cache-less publisher's gap-window transition. What no consumer may do:
  assume a plain GET reaches publisher caches, or that `history()`
  reaches storages.

The honest limits of the baseline, so the trade is explicit: detection
latency for a missed transition is bounded by TTL (aging), not by a
heartbeat period; a telemetry late joiner sees nothing until the next
cadence unless the deployment adds a telemetry storage; per-sample gap
*recovery* does not exist — a lost reliable sample surfaces as staleness,
not retransmission.

### 3.3 The advanced tier (opt-in, unstable)

zenoh-ext's `AdvancedPublisher`/`AdvancedSubscriber` add per-key
publisher-side history, per-source sequence numbers, and gap recovery.
The tier is **opt-in per subject** — it is never a class default —
because its costs are per *key*, and the fleet's key population multiplies
them:

> **Cost box** (from the zenoh 1.9 source; there are no published
> benchmarks). A fully-optioned AdvancedPublisher creates **4 entities per
> key**, two of which — the cache queryable and the liveliness token at
> `<key>/@adv/pub/<zid>/<eid>/…` — are **network-wide routed declarations
> that no router can aggregate** (the key embeds zid+eid). A periodic
> `heartbeat(p)` publishes unconditionally forever (a 4-byte seqnum every
> `p`); the moment one subscriber enables `recovery(heartbeat)`, **every
> matching publisher's heartbeat crosses the network to it** — K
> matching keys ⇒ K/p msg/s per such subscriber, paid when nothing is
> lost. `history()` cold-start costs O(publishers × depth) reply samples;
> adding `detect_late_publishers()` can double it and turns every
> publisher restart into one token replay + one GET *per subscriber*.
> At a 10 000-key fleet that is ~40 000 entities, ~20 000 router-table
> entries per router, and (at `heartbeat(1 s)`) ~10 000 msg/s per
> recovering subscriber. Field evidence from the closest comparable
> workload (rmw_zenoh's token-per-entity model): seconds of router CPU
> saturation per node restart; its maintainers ship miss-detection off by
> default for traffic reasons.

Where the tier earns its cost:

- **Low-count, high-value transition state** needing `detect_s ≪ ttl_s` —
  alerts, config echoes: O(1–10) keys per producer. Configuration:
  `cache(1)` + `sample_miss_detection(sporadic_heartbeat(detect_s))` —
  sporadic, because these keys are idle almost always and the sporadic
  mode publishes only after a change. Consumers pair it with
  `recovery(heartbeat())`.
- **Router-less / storage-less meshes**, where publisher caches are the
  *only* possible seed source: `cache(1)` on state subjects (no miss
  detection unless `detect_s` demands it), consumers seed with
  `history()`. This is where §3.1's seeding burden lands on the producer,
  and the one deployment shape in which the tier is the *normative* seed
  mechanism rather than an opt-in (§3.5).
- **Chart-tail seeding without a telemetry storage**: `cache(n)` on the
  handful of subjects whose registry says `seed = tail(n)` — not across a
  wide telemetry fan.

What the tier is NOT for: wide telemetry fans (hundreds of keys per
producer — the entity and heartbeat arithmetic above), telemetry gap
*recovery* (fighting the `sampled` profile: re-querying shed samples
presses on exactly the congested link), `events` replay (**unbuildable**:
a publisher owns one key, every events key is unique — an events "cache
ring" cannot exist; replay is the storage's job), and `@media` (recovering
a superseded frame is anti-useful).

Implementation notes for the tier (production-learned): timestamping must
already be on ([09-operations.md §0](09-operations.md)) — and do not rely
on the builder to catch its absence: only the cache-*only* configuration
self-checks; cache + miss-detection builds happily untimestamped. Always
set `HistoryConfig::max_samples` — **the default is unbounded buffering**;
`history()` MUST drain through an unbounded (or provably burst-sized)
handler channel or the declare deadlocks the session; declare
AdvancedSubscribers after one-shot seed GETs on the same session (§3.2's
subscribe-first rule is satisfied by the AdvancedSubscriber itself — its
declare-time history query is internally race-free; the ordering rule is
about *other* GETs sharing the session); prefer `periodic_queries(p)` over
`recovery(heartbeat)` on wide wildcard subscriptions — it bounds load by
the subscriber's choice instead of the publishers' key count. A consumer
that wants a loss *metric* without recovery uses
`sample_miss_listener()` (reports `{source, count}`).

The sidecar keys the tier creates are verbatim-isolated under the data key
(`<key>/@adv/pub/<zid>/…`, `<key>/@adv/sub/<zid>/…`): they ride no
firehose and no data selector, but principals that opt in DO need the
`@adv` ACL rules — and a missing rule fails *silently* (empty seeds,
denied recovery, indistinguishable from "nothing to recover";
[09-operations.md §3](09-operations.md)).

### 3.4 Choosing a tier

| Deployment shape | State | Telemetry | Events |
|---|---|---|---|
| Router + storages (the normal fleet) | baseline; advanced only on subjects with `detect_s ≪ ttl_s` | baseline (storage supplies any tail) | baseline + events storage |
| Router-less mesh (no storage anywhere) | advanced `cache(1)` — publisher caches are the only seed | baseline; `cache(n)` only where a tail entitlement exists | accept producer-lifetime visibility, or add a storage node |
| Constrained leaf | baseline + plain subscriber + local store (no history bursts, no heartbeats) | baseline | baseline |

The split mirrors §3's QoS design: **entitlements in the registry,
mechanisms in deployment/build config.** A subject's row in the registry
says what consumers may rely on; whether a cache or a storage delivers it
is invisible to the keys and to the wire contract. The table's first
column is also the seeding decision (§3.5): the row a deployment sits in
names its seed source.

### 3.5 Seeding, one story: storage first, caches where no storage runs (v1.5, narrowed in v1.25)

The seed entitlement (§3.1) has two conforming mechanisms, and the choice
between them is a fact about the *deployment*, not a preference:

- **Where a latest-value storage covers `state/**` ([09-operations.md
  §2](09-operations.md)), the storage seed is the default.** It answers
  §3.2's plain GET, it outlives every publisher — the coverage the
  crashed-producer case in §3.2 turns on, since a cache dies with its
  publisher — and it costs the fleet nothing per key. A deployment that
  runs the storage anyway, for durable at-rest reasons (§4), has already
  paid for its seed source.
- **Where no such storage exists — the router-less mesh of §3.4's second
  row — publisher-side caches are the normative mechanism**: the advanced
  tier's cache + history/recovery (§3.3), which is the mechanism the
  middleware ecosystem has consolidated on (its older
  cache/querying-subscriber APIs are deprecated upstream). In a
  storage-less deployment the producer *is* the seed source for every
  subject whose registry entry carries a seed entitlement — that is
  §3.1's burden allocation, read from the producer's side.

The v1.5 form of this section declared the advanced tier "the normative
seeding mechanism for volatile state" outright. That sentence is
**narrowed** here (v1.25), not repudiated: read as a universal default it
contradicted §3.3's own rule — the tier is opt-in per subject, never a
class default — and §3.4's first row, which sends the normal fleet to the
baseline; and it would have re-imported, as a default, exactly the
per-key cost the §3.3 box prices. What v1.5 correctly decided survives in
the second bullet: *when a cache is the answer, the middleware's advanced
tier is the cache* — the convention defines no seeding mechanism of its
own, and §3.3's opt-in and cost box stand as the no-storage answer's
price list.

**What does not change:** the storage-manager remains authoritative for
durable at-rest data (§4 — event logs, state history, the catalog);
`seed`/`detect_s` registry semantics are untouched (entitlements in the
registry, mechanisms in deployment — §3.4's split holds); §3.2's seed
discipline binds both paths; and local durability layers (a constrained
leaf's on-disk backfill store) are a different concern entirely. The hard
dependency this rests on is the plain version chunk: the advanced tier's
`@adv` liveliness tokens must remain structurally parseable
([03-grammar.md §1.2](03-grammar.md)) — the enforcement crate pins it
with executable tests.

---

## 4. Storage mapping

The class chunk is the storage selector. A deployment configures storages
per class, with `strip_prefix` = `<base>/v1` (a literal leftmost run, as
Zenoh requires):

| Storage | Selector | Backend shape |
|---|---|---|
| latest-value | `<base>/v1/*/state/**` | LWW store honouring tombstones (fs/redb/rocksdb-class) |
| time-series | `<base>/v1/*/telemetry/**` | append-per-key (influx-class) |
| event log | `<base>/v1/*/events/**` | append-only; retention is the **backend database's** policy (e.g. an InfluxDB retention policy) — Zenoh's storage `garbage_collection` GCs metadata, not data |
| catalog | `<base>/v1/@catalog/state/**` | LWW store — **explicit**, because `*` never matches `@catalog` (design property D4) |
| catalog history | `<base>/v1/@catalog/state/pdns/**` | time-series capture of an LWW stream — see below |

Storages require timestamped samples for LWW to be meaningful: deployments
MUST enable Zenoh timestamping on the publishing side (routers default to
on, peers/clients to off). Two facts that shape deployment choices: Zenoh
**storage replication** (anti-entropy alignment between replicas) works
only on latest-value storages — history capture does not replicate at the
Zenoh layer, its availability is the backend database's concern; and the
storage `garbage_collection.lifespan` is the knob that bounds tombstone
visibility — it MUST be set ≥ the longest state TTL, or §1.2's
tombstone-retention rule is silently violated (the default is 24 h).
Concrete config, volume guidance, replication, and the overlap/`complete`
caveats: [09-operations.md §2](09-operations.md).

Two consequences the incumbent keyspace could not offer:

- **No key ever needs client-side filtering to classify.** An exporter that
  wants only telemetry subscribes `…/*/telemetry/**` and receives only
  telemetry — the discard-after-the-wire filter (`is_telemetry_key`)
  disappears, and so does the bandwidth it wasted.
- **Storage policy is a deployment file, not application knowledge.** The
  storage manager needs no list of "which prefixes are state-like"; the
  grammar already said it.

**History of state is a storage choice, not a class change.** A time-series
storage pointed at a `state` selector records every LWW transition — that
*is* the history of that state, **provided the backend records deletes**: a
history capture MUST persist tombstones as explicit retirement markers (a
backend that silently drops `SampleKind::Delete` cannot distinguish
"retired at T" from "still current" and MUST NOT be claimed as state
history). The reference application's passive-DNS record
(`@catalog/state/pdns/<ip-slug>`: the full accumulated name-set for an IP,
superseded on each update) is ordinary LWW state on the bus whose influx
capture yields the historical IP↔name record. No dedicated "historical
plane" is needed in the grammar.

---

## 5. Liveliness (presence)

Presence is not a data class — it is the middleware's liveliness primitive,
which auto-retracts tokens on crash/disconnect. The convention places
tokens on keys that **mirror the state grammar**, so presence selectors look
like data selectors:

| Token key | Declared by |
|---|---|
| `<base>/v1/<origin>/state/<producer>/alive` | every producer instance |
| `<base>/v1/<origin>/state/<producer>/device/<device>/alive` | producers tracking downstream devices |
| `<base>/v1/@catalog/state/alive` | the elected catalog owner ([06-identity.md §5.3](06-identity.md)) |

- Liveliness tokens live in the middleware's separate liveliness space;
  they do not collide with data subscribers even though the key shapes
  align. The alignment is for humans and for selector reuse:
  `<base>/v1/*/state/*/alive` (liveliness subscriber) is the entire
  fleet-presence protocol, zero payload bytes. To keep that shape
  unambiguous, `alive` is a reserved subject leaf
  ([03-grammar.md §3](03-grammar.md)) — never a data subject.
- Zenoh does **not** enforce token uniqueness (two sessions can hold the
  same token key); a token is presence, not a lock. Where exclusivity
  matters (`@catalog`), the convention builds an explicit claim protocol on
  top ([06-identity.md §5.3](06-identity.md)).
- Consumers SHOULD treat retraction of a producer's `alive` token the way
  GDBusProxy treats a vanished bus-name owner
  ([10-prior-art.md](10-prior-art.md)): mark that producer's cached state
  suspect immediately rather than waiting out the TTL, and re-seed when the
  token reappears.
- Producers MUST declare their `@rpc` queryables **and** (if on the
  advanced tier) all their AdvancedPublishers *before* declaring their
  `alive` token — "alive ⇒ callable **and seedable**": callers can
  attribute RPC silence ([05-control-rpc.md §3](05-control-rpc.md)), and a
  §5-triggered re-seed can never race an undeclared cache. The invariant
  stands as written; a conformance checker sampling the declared set allows
  a bounded grace for declarations spawned concurrently at startup
  ([08-registry.md §6.1](08-registry.md), "Checking the two halves",
  v1.20) — it tolerates the spawn race, not a producer that reports `alive`
  before it is callable.
- **One roster, not two.** The advanced tier's `publisher_detection`
  tokens (`<key>/@adv/pub/…`, §3.3) are per-*publisher-entity* machinery,
  consumed only by AdvancedSubscriber internals
  (`detect_late_publishers()`); the per-producer `alive` token is the
  only presence signal consumers, dashboards, and RPC attribution may
  read. An AdvancedSubscriber already re-seeds natively on `@adv` token
  events and MUST NOT double-trigger from `alive`.
- The token *key* is the identity record (origin + producer + device) —
  the pattern proven by rmw_zenoh's `@ros2_lv` discovery space and
  Keelson's presence tokens ([10-prior-art.md](10-prior-art.md)).
- Richer "who am I" registration (versions, capabilities, config hash)
  is ordinary state: `state/<producer>/sensor` (a registration document),
  refreshed on the state cadence.
