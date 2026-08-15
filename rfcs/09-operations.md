# 09 — Operations Cookbook

**Status: v1.18 (ratified)** · informative chapter, but §5.1 is normative for tools · *amended in v1.2, v1.9, v1.13 and v1.18 — see [00-index.md](00-index.md)*

Worked recipes for the infrastructure concerns the grammar was shaped
around: session setup, subscriptions, storage, ACL, and constrained links,
plus the obligations of tools that only read the bus (§5.1).
Base = `zensight` throughout; substitute your deployment's base.

---

## 0. Session configuration

Every participant sets the base as its session **namespace** (stable
config; [03-grammar.md §1.1](03-grammar.md)) and enables timestamping
(required by state LWW, storages, and publisher caches):

```json5
{
  namespace: "zensight",                 // = <base>; app code never spells it
  timestamping: { enabled: true },       // peers/clients default to false — turn it on
  // mode/connect/listen as the deployment requires
}
```

A base-less deployment (*v1.6*, [03-grammar.md §1.1](03-grammar.md)) simply
omits `namespace` — Zenoh's default — and its full wire keys start at
`v1/…`; the timestamping requirement stands either way.

With the namespace set, everything the application declares is
base-relative (`v1/…`), and ingress from outside the namespace is
filtered — the base is an isolation boundary, not a string convention.
Remember the two scope rules: router-side config (storages, ACL,
interceptors — everything below) is written with the **full** key, and a
namespaced session cannot reach the router admin space (§5).

### 0.1 Discovery and scouting

*Added in v1.2. The words `scout`, `gossip` and `multicast` appeared zero
times across the ratified chapters; §0's entire treatment of connectivity
was the comment `// mode/connect/listen as the deployment requires`. The
gap cost a debugging session and produced a green test over a broken
system, which is the expensive kind.*

Zenoh discovers peers by two **independent** mechanisms, with independent
switches:

| | `scouting/multicast/enabled` | `scouting/gossip/enabled` |
|---|---|---|
| **what** | UDP multicast beacons on the LAN | peers tell their peers about their peers |
| **reaches** | anything on the segment, including things you did not mean | only within the **already-connected** graph |
| **needs** | a multicast-capable network | at least one explicit `connect`/`listen` edge |

They are not a single "discovery" knob, and **collapsing them into one is
a bug**:

- **Turning both off** to isolate a test does more than isolate it. Gossip
  is what carries reachability *through* a peer, so a hub-and-spoke
  deployment with gossip off has spokes that can each reach the hub and
  **cannot reach each other** — silently. Data whose consumer sits on
  another spoke simply never arrives, and the failure looks like a
  publisher fault. (This is exactly how an isolated correlator run came
  back with an empty entity seed: the evidence was published, and could not
  traverse the gossip-less hub.)
- **Turning multicast on** in a shared environment does more than discover.
  A default-config session joins *any* reachable Zenoh mesh, including a
  colleague's, including production. A test that scouts is not isolated,
  and the contamination flows both ways.

**Normative recipe for isolated verification** — the one every
acceptance run in this chapter assumes:

```json5
{
  scouting: {
    multicast: { enabled: false },   // never join a mesh we did not name
    gossip:    { enabled: true  },   // but let reachability cross the hub
  },
  connect: { endpoints: ["tcp/127.0.0.1:<port>"] },   // an explicit graph
}
```

Multicast **off**, gossip **on**, explicit endpoints. That is isolation
*and* a connected graph — and a deployment knob that disables "scouting"
wholesale MUST be understood to mean the multicast half only, or it will
break spoke-to-spoke discovery the first time it is used in anger.

## 1. Selector cookbook

| Consumer | Declares | Notes |
|---|---|---|
| UI, full fleet | `zensight/v1/*/telemetry/**` + `zensight/v1/*/state/**` + `zensight/v1/*/events/**` + `zensight/v1/@catalog/state/entity/*` + `zensight/v1/@catalog/state/alias/*` | three class subs replace firehose-plus-filtering; catalog (and its alias records — the origin→entity re-pointing on merges) named explicitly (D4) |
| UI, one host drill-down | `zensight/v1/h-xxx/**` | complete data plane of one host; cannot pull media/blob/rpc (D2) |
| UI, presence | liveliness subs `zensight/v1/*/state/*/alive` + `zensight/v1/*/state/*/device/*/alive` + `zensight/v1/@catalog/state/alive` | token keys are the identity; zero payload; the catalog token named explicitly (D4 — `*` never matches it), else "catalog dead" is indistinguishable from "no entities" |
| Exporter (metrics) | `zensight/v1/*/telemetry/**` | nothing to discard client-side |
| Exporter (alerts) | `zensight/v1/*/state/*/alert/*` | |
| Protocol-specialist view | `zensight/v1/*/telemetry/netring/**` | one `*`, protocol-first ergonomics preserved |
| Catalog (evidence intake) | `zensight/v1/*/state/*/evidence/**` | |
| Late-joiner seeds | `GET` the same state selectors | state is its own seed ([05-control-rpc.md §4](05-control-rpc.md)); discipline in [04-planes.md §3.2](04-planes.md) |
| Media viewer | exact `…/@media/<producer>/<stream>/preview/jpeg`, or exact `…/video/<codec>/<tier>` | single-stream, single-tier — no `@media` wildcard (07 §1) |

Anti-patterns:

- `zensight/v1/**` — legal, but you almost never mean it; it spans every
  host's every class. Subscribe per class or per origin.
- Any selector containing `$*` — forbidden ([02-principles.md P6](02-principles.md)).
- Subscribing a data selector to "watch" RPC or media — structurally
  impossible; if you think you need it, the data you want is `state`.

## 2. Router storage configuration

Class-driven storages, all with a literal `strip_prefix` (Zenoh
requirement; a verbatim chunk inside it is fine — validation is
string-prefix + no-wildcards). Sketch (storage-manager JSON5):

```json5
plugins: {
  storage_manager: {
    volumes: {                                // REQUIRED for non-memory volumes;
      fs: {},                                 // each needs its backend plugin installed:
      influxdb: { url: "http://localhost:8086" },  // zenoh-backend-{filesystem,influxdb},
    },                                        // out-of-tree, version-matched to the router
    storages: {
      latest: {                                   // current truth of the fleet
        key_expr: "zensight/v1/*/state/**",
        strip_prefix: "zensight/v1",
        volume: { id: "fs", dir: "latest" },      // any LWW-honouring backend
      },
      timeseries: {                               // charts and history
        key_expr: "zensight/v1/*/telemetry/**",
        strip_prefix: "zensight/v1",
        volume: { id: "influxdb", db: "telemetry" },
      },
      events: {                                   // immutable record; retention is the
        key_expr: "zensight/v1/*/events/**",     // DATABASE's policy (InfluxDB RP) —
        strip_prefix: "zensight/v1",             // zenoh's garbage_collection GCs metadata
        volume: { id: "influxdb", db: "events" },
      },
      catalog: {                                  // explicit: '*' never matches @catalog (D4)
        key_expr: "zensight/v1/@catalog/state/**",
        strip_prefix: "zensight/v1/@catalog",
        volume: { id: "fs", dir: "catalog" },
      },
      pdns_history: {                             // history of LWW state = a storage choice (04 §4)
        key_expr: "zensight/v1/@catalog/state/pdns/**",
        strip_prefix: "zensight/v1/@catalog/state/pdns",
        volume: { id: "influxdb", db: "pdns" },
      },
    },
  },
}
```

Notes:

- The `latest` storage doubles as the fleet-wide late-joiner seed: a GET on
  any state selector is answered by the router even when producers sleep.
  (It is what makes plain-GET seeds work at all —
  seed discipline in [04-planes.md §3.2](04-planes.md).) Timestamping must be enabled
  on the publishing side for LWW to be meaningful
  ([04-planes.md §4](04-planes.md)).
- `catalog` and `pdns_history` **overlap**: a GET under `…/state/pdns/**`
  is answered by both (duplicate, possibly divergent replies). Either
  accept it (subscribers are unaffected; GET consumers consolidate) or
  carve `pdns/**` out of the `catalog` storage's selector.
- Media is never stored (recording is a deliberate consumer, not a storage
  rule); blob chunks MAY be stored to make the router a content cache
  ([07-bulk-planes.md §2](07-bulk-planes.md)).

### 2.1 Choosing volumes

A backend advertises a capability pair — *persistence* (volatile/durable)
× *history* (latest/all) — and the class semantics pick it:

| Volume | Capability | Use for | Caveats |
|---|---|---|---|
| `memory` (bundled) | volatile · latest | seed-only deployments, testing | gone on router restart — late joiners lose their seed until state refreshes |
| `fs` / `rocksdb` | durable · latest | `latest`, `catalog` — the LWW truth stores | out-of-tree plugins, version-matched to the router |
| `influxdb` | durable · all | `timeseries`, `events`, `pdns_history` — anything whose value is the *sequence* | retention lives in the database (RP), not zenoh config |

Storages are read-write by construction (there is no `read_only` field);
restrict who can write *into* a storage's selector with ACL, not storage
config. Out-of-order and wildcard writes are safe: the storage applies
updates by timestamp (an outdated sample is discarded, a wildcard delete
still masks a slower concrete put).

### 2.2 Replication (HA for the seed store)

The `latest` storage is a single point of seed failure; Zenoh storage
**replication** (anti-entropy digest alignment between storages on
different routers) fixes that for latest-value storages:

```json5
latest: {
  key_expr: "zensight/v1/*/state/**",
  strip_prefix: "zensight/v1",
  volume: { id: "fs", dir: "latest" },
  replication: {
    interval: 10.0,          // digest period, seconds
    sub_intervals: 5,
    hot: 6, warm: 30,        // eras of decreasing digest resolution
    propagation_delay: 250,  // ms; MUST be < interval/2
  },
},
```

Rules: every replica MUST use the **identical** `key_expr`,
`strip_prefix`, and replication parameters (divergent parameters cause
digest storms, not errors); replication requires timestamps and works
**only on latest-value backends** — the influx history storages do not
replicate at the Zenoh layer, their availability is the database's
concern (cluster the database, or accept a history gap on router loss).

A **replicated, fully-covering** `latest` storage is also the one place
`complete: true` is *right*: it lets the router answer any state GET from
the nearest replica without fanning out. Keep the round-1 caveat in mind —
never mark a storage complete whose selector intersects `@rpc` fan-in
paths ([05-control-rpc.md §2.1](05-control-rpc.md)); the class-scoped
selectors above cannot (verbatim `@rpc` is unreachable from `state/**`).

### 2.3 Garbage collection = tombstone lifetime

`garbage_collection: { period, lifespan }` on a storage prunes *metadata*
— including deletion tombstones — older than `lifespan` (default 24 h).
This is the knob that enforces the tombstone-visibility row of the
staleness contract ([04-planes.md §1.2](04-planes.md)): `lifespan` ≥ the
longest `ttl_s` in the registry, else a slow replica may resurrect a
retired key.

## 3. ACL recipes

Four facts of Zenoh ACL shape every recipe, and the first two follow from
the convention's own algebra:

1. **Matching is keyexpr *inclusion*** (rule ⊇ message key), and `**` never
   crosses a verbatim chunk in inclusion either. So a host's
   `…/h-xxx/**` rule does **not** cover its `@rpc` replies, `@media`
   frames, or `@blob` keys — the hermeticity that protects selectors cuts
   ACL prefixes identically. Per-principal ACL is therefore a **fixed set
   of literal-prefix rules, one per plane** (~4 per host), not one rule.
2. A rule with `*` in the origin position never covers `@catalog` (D4
   applies to ACL keyexprs too); catalog access is always its own rule.
3. The config requires **three lists** — `rules`, `subjects`, and
   `policies` binding them; rules alone are rejected at router startup. The
   cert-CN ↔ origin enrollment ([03-grammar.md §4 D6](03-grammar.md))
   lives in `subjects`.
4. Under `default_permission: "deny"`, *declarations* need allowing too:
   a consumer that may not `declare_subscriber` receives nothing, a
   producer that may not `declare_queryable` serves nothing, and queries
   must be allowed **egress** toward the responder's face as well as
   ingress from the caller's.

The grant matrix at a glance — one row per rule id in the sketch below, so
a wrong or missing grant is visible before reading any JSON5 (`(adv)` =
only for principals on the advanced tier):

| Principal | Rule | Covers | Flow | Messages |
|---|---|---|---|---|
| host | `host-data` | `h-xxx/**` (data classes + `alive` tokens) | in | put, delete, liveliness_token |
| host | `host-media` | `h-xxx/@media/**` | in | put |
| host | `host-serve` | `h-xxx/@rpc/**`, `h-xxx/@blob/**` | both | declare_queryable, reply, query |
| host | `host-blob-seed` | `h-xxx/@blob/{store,tree}/**` | in | put |
| host | `host-adv` (adv) | `h-xxx/**/@adv/**` | both | put, liveliness_token, declare_queryable, reply, query |
| catalog | `catalog-own` | `@catalog/**` (+ its `@rpc`) | both | put, delete, liveliness_token, declare_queryable, reply, query |
| catalog | `catalog-intake-declare` | `v1/**` | **in only** | declare_subscriber, declare_liveliness_subscriber, liveliness_query, query |
| catalog | `catalog-intake-recv` | `v1/**` | **out only** | put, delete, liveliness_token, reply |
| console | `ops-sub` | all planes (each named) | in | declares, liveliness_query, query |
| console | `ops-own-token` (adv) | `**/@adv/**` | in | liveliness_token |
| console | `ops-recv` | all planes (each named) | out | put, delete, reply, liveliness_token |
| console | `no-remote-actions` | `*/@rpc/systemd/action` | both | **deny** query |
| svc-origin | `desired-author` | `@desired/state/**` (its own subtree only) | in | put, delete |

Sketch (structure verified against the Zenoh 1.9 schema; validate against
a live `zenohd` before deploying):

```json5
access_control: {
  enabled: true, default_permission: "deny",

  rules: [
    // ---- sensor host (template: one set per enrolled host) ----
    { id: "host-data",  permission: "allow", flows: ["ingress"],
      messages: ["put", "delete", "liveliness_token"],
      key_exprs: ["zensight/v1/h-3fa9c2d41b7e/**"] },          // data classes + alive tokens
    { id: "host-media", permission: "allow", flows: ["ingress"],
      messages: ["put"],
      key_exprs: ["zensight/v1/h-3fa9c2d41b7e/@media/**"] },   // plane needs its own rule (fact 1)
    { id: "host-serve", permission: "allow",
      messages: ["declare_queryable", "reply", "query"],        // query egress = router forwards calls to it
      key_exprs: ["zensight/v1/h-3fa9c2d41b7e/@rpc/**",
                  "zensight/v1/h-3fa9c2d41b7e/@blob/**"] },
    // hosts that seed the router @blob content store use the sanctioned
    // one-shot PUT path (04-planes §3) — grant it explicitly, or omit this
    // rule in deployments without a router content store:
    { id: "host-blob-seed", permission: "allow", flows: ["ingress"],
      messages: ["put"],
      key_exprs: ["zensight/v1/h-3fa9c2d41b7e/@blob/store/**",
                  "zensight/v1/h-3fa9c2d41b7e/@blob/tree/**"] },
    // ONLY for hosts on the advanced tier (04-planes §3.3): the sidecars
    // (cache queryable, liveliness token, heartbeat publisher at
    // <key>/@adv/pub/<zid>/…) live under a verbatim @adv suffix the
    // host-data '**' rule cannot reach. Omitting this rule when the tier
    // is in use fails SILENTLY — empty seeds, dead recovery:
    { id: "host-adv", permission: "allow",
      messages: ["put", "liveliness_token",
                 "declare_queryable", "reply", "query"],
      key_exprs: ["zensight/v1/h-3fa9c2d41b7e/**/@adv/**"] },

    // ---- catalog service ----
    { id: "catalog-own", permission: "allow",
      messages: ["put", "delete", "liveliness_token",
                 "declare_queryable", "reply", "query"],
      key_exprs: ["zensight/v1/@catalog/**",
                  "zensight/v1/@catalog/@rpc/**"] },
    // intake is split by flow: the catalog DECLARES interest (ingress) and
    // RECEIVES data/tokens (egress) — a flowless rule here would let the
    // catalog principal ingress-publish and tombstone ANY host's keys,
    // defeating the per-host enrollment story:
    { id: "catalog-intake-declare", permission: "allow", flows: ["ingress"],
      messages: ["declare_subscriber", "declare_liveliness_subscriber",
                 "liveliness_query", "query"],
      key_exprs: ["zensight/v1/**"] },
    { id: "catalog-intake-recv", permission: "allow", flows: ["egress"],
      messages: ["put", "delete", "liveliness_token", "reply"],
      key_exprs: ["zensight/v1/**"] },

    // ---- operator console: read everything, write nothing but RPC ----
    // (the **/@adv/** entries carry AdvancedSubscriber traffic: history/
    //  recovery GETs to publisher caches, late-publisher liveliness
    //  detection, and the console's own subscriber-detection token)
    { id: "ops-sub", permission: "allow", flows: ["ingress"],
      messages: ["declare_subscriber", "declare_liveliness_subscriber",
                 "liveliness_query", "query"],
      key_exprs: ["zensight/v1/**", "zensight/v1/@catalog/**",
                  "zensight/v1/*/@rpc/**", "zensight/v1/@catalog/@rpc/**",
                  "zensight/v1/*/@blob/**", "zensight/v1/*/@media/**",
                  "zensight/v1/**/@adv/**"] },
    // the console's OWN token (advanced-tier subscriber detection) is
    // confined to @adv — a broad ingress liveliness_token allow would let
    // the console forge any host's `state/*/alive` roster entry:
    { id: "ops-own-token", permission: "allow", flows: ["ingress"],
      messages: ["liveliness_token"],
      key_exprs: ["zensight/v1/**/@adv/**"] },
    { id: "ops-recv", permission: "allow", flows: ["egress"],
      messages: ["put", "delete", "reply", "liveliness_token"],
      key_exprs: ["zensight/v1/**", "zensight/v1/@catalog/**",
                  "zensight/v1/*/@rpc/**", "zensight/v1/@catalog/@rpc/**",
                  "zensight/v1/*/@blob/**", "zensight/v1/*/@media/**",
                  "zensight/v1/**/@adv/**"] },

    // dangerous procedures deniable per-key, because the key IS the target
    // (deny wins; sound under default-deny — an origin-`**` query that would
    // sidestep this literal also matches no allow rule):
    { id: "no-remote-actions", permission: "deny",
      messages: ["query"],
      key_exprs: ["zensight/v1/*/@rpc/systemd/action"] },

    // ---- desired-state service origin (07 §3 carve-out) ----
    // a controller that authors desired-state on behalf of a target
    // (07-bulk-planes §3) needs EXACTLY ONE ingress put/delete grant, on
    // ITS OWN service-origin subtree — never on any host origin. Absent
    // this rule it is refused ingress-put by default-deny; present, it can
    // still only write under @desired, so it cannot forge a host's own
    // state (the target host id is the first SUBJECT chunk, not the origin):
    { id: "desired-author", permission: "allow", flows: ["ingress"],
      messages: ["put", "delete"],
      key_exprs: ["zensight/v1/@desired/state/**"] },
  ],

  subjects: [
    // enrollment: transport identity ↔ origin (03-grammar §4 D6)
    { id: "host-3fa9", cert_common_names: ["h-3fa9c2d41b7e"] },
    { id: "catalog",   cert_common_names: ["zensight-catalog"] },
    { id: "console",   cert_common_names: ["zensight-console"] },
    // service origin @desired enrolls like any principal (03-grammar §4 D6)
    { id: "desired", cert_common_names: ["zensight-desired"] },
  ],

  policies: [
    { rules: ["host-data", "host-media", "host-serve",
              "host-blob-seed", "host-adv"],                 subjects: ["host-3fa9"] },
    { rules: ["catalog-own", "catalog-intake-declare",
              "catalog-intake-recv"],                        subjects: ["catalog"] },
    { rules: ["ops-sub", "ops-recv", "ops-own-token"],       subjects: ["console"] },
    { rules: ["no-remote-actions"],                          subjects: ["console"] },
    { rules: ["desired-author"],                             subjects: ["desired"] },
  ],
}
```

The property to notice: *"host X may act only as itself"* and *"nobody but
the console may invoke actions"* are **static literal-prefix rules pinned
to enrolled identities** — inexpressible in a keyspace where the host
discriminator is a mutable name at varying positions. One more rule of
thumb: a rule's `key_exprs`
must **include** (⊇) the consumer's declared selector, not merely
intersect it — allow `zensight/v1/**` does not admit a `zensight/**`
subscriber.

**Sub-host authority needs the resource in the path (v1.4).** Because ACL
matching is keyexpr *inclusion* (fact 1) and a rule **cannot** discriminate
on *selector parameters* — a `?if=eth1` on a query is invisible to the
allow/deny decision — any authority *below* the host granularity ("this
principal may shape `eth1` but not the management NIC") requires the
actuated resource to be a **path chunk**, never a selector. The write
procedure MUST therefore be keyed
`@rpc/<producer>/config/{ns}/{if}/set`, so a rule can allow
`…/@rpc/actuator/config/*/eth1/set` and deny `…/config/*/mgmt0/set`; a design
that instead spelled it `@rpc/actuator/config/set?if=eth1` collapses to a single
`…/config/set` keyexpr the ACL can only allow or deny *wholesale*, and the
sub-host distinction is unenforceable. This is grammar-legal because the
actuated population — network interfaces — is **bounded**: the
per-message-data ban ([03-grammar.md §2](03-grammar.md)) does not bite
(an interface is an observed-population chunk, not a request id), and
[08 §2](08-registry.md)'s cardinality budget covers it. Put the discriminator
where the ACL can see it: in the key.

## 4. Constrained links

A bandwidth-limited leaf (radio, cell, tactical link) is provisioned by
prefix policy on its router — the keyspace *is* the bandwidth policy (the
Indy-Autonomous-Challenge pattern, [10-prior-art.md §1](10-prior-art.md)).
Zenoh has no dedicated "bandwidth allowlist"; the policy is expressed with
two real mechanisms, both selecting on the same class prefixes:

- **`access_control` scoped to the link's interface**
  (`subjects: [{ id: "radio", interfaces: ["wlan0"] }]`), with the §3
  pattern: allow `…/h-xxx/state/**` (truth, small, must flow),
  `…/h-xxx/telemetry/sysinfo/**` (chosen rollups), `…/h-xxx/events/**`
  (rare by budget); the un-allowed `telemetry/netring/**` firehose then
  stays local by default-deny. The §3 caveats apply verbatim — in
  particular, "@rpc/@blob on demand" is not free under default-deny: the
  query/reply/declare legs must be explicitly allowed per plane, which is
  five more literal-prefix rules, not zero.
- **`downsampling`** on the link's interface for rate-limits softer than
  allow/deny: `[{ interfaces: ["wlan0"], rules: [{ key_expr:
  "zensight/v1/*/telemetry/**", freq: 0.1 }] }]` — telemetry crosses at
  ≤ 0.1 Hz, state and events untouched.
- **`qos` overwrite interceptor** to *enforce* the class QoS profiles
  ([04-planes.md §3](04-planes.md)) at the router regardless of what
  publishers set: one rule per class prefix, e.g. force
  `zensight/v1/*/telemetry/**` to `priority: "data_low"` and
  `zensight/v1/*/state/*/alert/*` to
  `{ priority: "interactive_high", congestion_control: "block" }`. The
  interceptor ignores API-level QoS — deployment policy wins.

Advanced-tier traffic deserves a thought on constrained links: per-key
miss-detection heartbeats and declare-time history bursts are real bytes
(the cost box in [04-planes.md §3.3](04-planes.md)). The baseline
([04 §3.2](04-planes.md)) creates none of it — no per-key entities, no
heartbeats — which is why it is the default; the tier is opt-in per
subject, and a constrained leaf simply doesn't opt in: a leaf consumer
runs a plain subscriber + local store instead of `history()` (the
reference GUI's constrained profile does exactly that).

Complementary conventions already assumed by the classes: superseded
streams drop under congestion, must-arrive state blocks
([04-planes.md §3](04-planes.md)); payloads are compact self-describing
binary (CBOR in the reference application); high-cardinality detail stays
pull-only; `@media` crosses only for an explicitly subscribed stream (and
can simply not be allowed on the link at all).

**Base handling is the session's job (v1.5).** The session `namespace`
prefixes the base onto *every* egress — publications, subscriptions,
queries, replies, declarations, liveliness tokens — and strips it on
ingress. An application running a namespaced session therefore has **no
legitimate use** for manual base composition: the enforcement crate's
`with_base`/`strip_base`/`parse_full` are **observer-side tools** —
un-namespaced explorers (`zenctl`, `zengui`), router artifacts (storage
selectors, ACL rules — §2/§3), and tests. Application code that reaches
for them has re-implemented what the session already does, in the one way
that can drift from it.

## 5. Debugging etiquette

- `z_sub 'zensight/v1/*/state/**'` shows fleet truth; add
  `…/@catalog/state/entity/*` to see conclusions. (Debug tools run
  *without* the namespace and spell full keys — which is also the honest
  view of what is on the wire.)
- Router administration (`GET @/<zid>/**`, admin space) needs an
  **un-namespaced** session: a namespaced session's selector is rewritten
  to `zensight/@/<zid>/**` and matches nothing
  ([03-grammar.md §1.1](03-grammar.md)).
- Reading a raw key aloud is the parse: *base, version, origin, class,
  producer, subject* — the chunk after `v1` is always who, the next is
  always what kind (positions are base-relative: multi-chunk bases are
  legal, [03-grammar.md §1.1](03-grammar.md)). No lookup table required;
  that property is worth defending in review.
- If a needed selector is awkward to write, that is registry feedback —
  file it against the subject layout before inventing a client-side filter
  ([08-registry.md §5](08-registry.md)).
- **Base discovery is an observer-side sweep (v1.5).** An observer that
  does not know a deployment's base recovers the bases in use from the
  wire: a liveliness query with the base wildcarded
  (`**/v1/*/state/*/alive` — `**` spans any base depth, *including zero*;
  `@catalog` asked by name because `*` never matches a verbatim chunk,
  D4), joined with the router storage configs' `key_expr`/`strip_prefix`
  (§2), which name a base even while the fleet is down. Attribution is
  fixed-arity from the right of the alive tail, never a "first `v1`"
  scan — a base containing a literal `v1` chunk attributes correctly.
  Observer tools MUST treat the **empty base** as legal *input*
  (`with_base`/`strip_base` are identities for it). Since *v1.6* a wire
  whose keys start at `v1/` is not merely observable but a **legal
  base-less deployment** ([03-grammar.md §1.1](03-grammar.md)) — the
  default one, in fact — so an explorer that cannot see and name it is
  blind to the common case. `zenctl base list` implements this sweep and
  reports the empty base as `(empty)`, selected with `--base ""`.

### 5.1 Observer obligations (v1.9; reviewed and amended at ratification, v1.18)

*Added in v1.9. The convention told tools how to read a conformant key and
said nothing about the other three cases an explorer actually meets: a key
under a different base, a key that is not this convention at all, and a
question the tool has not yet asked. Each was left to be invented per tool,
and the obvious inventions are the dishonest ones.*

An **observer** is any tool that reads the bus without publishing on it —
`zenctl`, `zengui`, a probe, a dashboard. Observers run un-namespaced (§4),
so they see everything on the wire, including traffic this convention does
not govern. The rules below are what keeps that honest.

**O1 — A key that does not parse is a fact, not an error.** An observer
**MUST NOT** discard, hide, or refuse a key merely because it is not
conformant. Non-conformant traffic is not prohibited — nothing in this
convention binds a foreign publisher, and [03 §1.2](03-grammar.md) is a rule
about where the *convention's* keyspace lives, not a claim on the bus. An
observer **SHOULD** present such a key with whatever it can still establish
(its chunks, its traffic, its payload rendered structurally per
[08 §7](08-registry.md)) and state what it could not.

**O2 — Classify by degrading, in this order.** Each rung that fails weakens
the claim rather than discarding the key:

| Rung | Question | On failure |
|---|---|---|
| 1 | Does it sit under the configured base (`strip_base`)? | **Not under this base.** Terminal. |
| 2 | Does the remainder parse as `v1/…` ([03 §1](03-grammar.md))? | **Unparsed**, carrying the reason. |
| 3 | Does a registry slice refine the subject ([08 §2](08-registry.md) precedence)? | **Unregistered**, or **no slice for this producer**. |

**O3 — Do not guess another deployment's base.** An observer that finds a key
outside its configured base **MUST NOT** attribute it to a base by scanning
left-to-right for a `v1` chunk: a subject tail has no fixed arity, and a base
may itself contain a literal `v1`. Naming other bases is the §5 sweep's job,
which attributes fixed-arity *from the right*. "Not under this base" is the
honest terminal answer.

**O4 — "Not asked" is not "answered no".** An observer **MUST** distinguish a
question it has not put to the bus from one that was answered negatively. A
tool with no registry loaded has learned nothing about whether a subject is
registered, and rendering that identically to "this subject is not registered"
reports a verdict it never obtained — [05 §3.1](05-control-rpc.md)'s rule
applied to a badge rather than to a reply set. The same holds for a roster not
yet seeded and a schema not yet fetched.

**O5 — A wildcard scope is not total coverage, and must not be presented as
such.** `*` and `**` never match a chunk beginning with `@` (this is what
makes §4 D2 and D4 true). A `**` subscription therefore cannot reach `@rpc`,
`@media`, `@blob`, `@adv` sidecars, the admin space, **or any service origin**.
The first half is a gift — a firehose subscriber cannot accidentally pull
video frames or bulk objects. The second is a trap: an observer scoped `**`
sees no `@catalog` traffic *by construction*, and if it labels that scope
"everything" then a healthy catalog and a dead one look identical. An observer
offering a wildcard scope **MUST** either name the verbatim planes it wants
alongside it, or state that they are excluded.

**O6 — A bounded observer reports what it dropped.** Any long-running observer
bounds something — a buffer, a scrollback, a key table — and an unbounded one
is merely a leak with better manners. Whatever the bound, the tool **MUST**
report what it cost, and MUST NOT fold the kinds into one number, because
they are different facts about the same window: samples missed while it was
behind ("we could not keep up"), keys or lines retired to stay within the
bound ("we chose to forget"), and — where the tool folds a burst into one
rendered update — samples **coalesced** ("we kept the newest and summarized
the rest"; amended at ratification, v1.18). Coalescing is neither of the
first two: nothing the view promised to show was lost, but a consumer that
assumes it saw every sample individually is wrong. A view that silently
shrinks is indistinguishable from a bus that went quiet.

*Informative — frugality (added at ratification, v1.18).* The obligations
above are about honesty, not thrift, but one habit keeps both cheap: an
observer SHOULD retrieve only what its user asked to see. Rendering what is
already in hand — registry slices, a seeded roster — is ambient and free;
anything that costs the data plane (a probe, a fetch, a subscription) is
asked for explicitly, once per ask. Guidance rather than obligation, because
cost is a design budget rather than a truth condition — recorded because the
reference explorers hold to it (data movement costs exactly one deliberate
action; zenkey #84/#85), and because a tool that ignores it tends to violate
O5 by accident: a pane that quietly fans out to keep itself fresh is
claiming coverage nobody asked it to have.

> **Where this came from.** Every rule above is a mistake that was made and
> caught while building `zengui` against this convention, not a hypothetical.
> O2 and O4 replaced a boolean "registered" flag that rendered "no registry
> loaded" as "unregistered". O3 replaced a base guess. O5 replaced a design
> that had defended against `@media` frames arriving through a `**` scope —
> which cannot happen — while missing that the same scope silently hid
> `@catalog`. O6's third kind arrived the same way, at ratification: the
> reference GUI's link layer batches bursts under a cap and counts the
> overflow (`coalesced`) beside its broadcast lag (`lagged`) — an honest
> number the two-counter wording could only misfile.

### 5.2 Capture and replay — `.zrec` etiquette (v1.13)

*Added in v1.13. The v1.5 amendment slate's H7 promised cookbook material
on "record/replay etiquette" and never delivered it — nothing existed to
document. With the `record` module in `zenkey-fleet` (zenkey #39) and the
`zenctl record` / `zenctl replay` commands (zenkey #53), the etiquette half
matters, because replay is **publishing**, and publishing a capture onto a
live fleet base is exactly the accident this convention exists to prevent.
This section is informative; the code is normative for the format. The
obligations of §5.1 apply throughout — a capture file is an observer whose
window happens to be on disk.*

**The format, in one screen.** A `.zrec` file is newline-delimited JSON —
deliberately the *same row dialect* the explorers already emit
(`zenctl topic echo --format ndjson`) and read back
(`zenctl topic pub --from ndjson`), not a second format:

- **Line 1 — the header**: `{"zrec": 1, "selectors": […], "base": "…",
  "captured_at": "<RFC 3339>"}`. The header names what was asked (O4) and
  under which base; a wildcard selector cannot cross an `@`-chunk, so a
  `**` capture states that `@rpc`/`@blob`/`@media`/`@catalog` and service
  origins are excluded rather than claiming "everything" (O5).
- **Sample rows**: the echo row shape plus `"bytes"` (base64, the exact
  wire payload — `"value"` is a decoded *rendering* and is not
  round-trippable), `"t"` (microseconds since capture start, the
  **observer's arrival clock** — this is what replay paces by), and the
  publisher's HLC `"timestamp"` carried **informatively** (see below).
  Tombstones are rows with `"delete": true`. Non-conformant keys are
  recorded verbatim (O1) — a capture curates nothing.
- **Drop records, interleaved where they happened**: `{"dropped": n}`.
  O6 applied to a file: a capture taken while the observer was behind is a
  partial view, and the file itself says so, at the position where the gap
  is. A reader surfaces the sum; a replay repeats it.

**Timestamps are re-stamped on replay, deliberately.** Replayed samples go
through declared publishers and receive the *replaying* session's HLC; the
capture's `"timestamp"` field is provenance, not a value to reproduce.
Reconciliation is by HLC — newer value wins, newer delete wins, and an
untimestamped sample cannot be reconciled at all
([04-planes.md §3.2](04-planes.md)) — so carrying a foreign, hours-old HLC
back onto the wire would make every replayed sample silently lose LWW
against anything live, which turns "replay onto a quiet base" into a no-op
that *looks* like a replay. Re-stamping keeps replay legible: what you
published now is newest now. The cost is the inverse hazard, and it is the
whole reason this section exists: **re-stamped old data wins LWW against a
live fleet** ([04-planes.md §1.2](04-planes.md)) — a replayed capture can
overwrite current state with last Tuesday.

**Hence the etiquette:**

- **Dry-run first.** A replay whose puts you have not previewed is a write
  you have not reviewed. `zenctl replay --dry-run` lists every would-be
  put and performs none.
- **The header base is a contract.** Replaying under a base other than the
  one in the capture header is refused unless explicitly forced
  (`--force-base`) — the tool never re-derives a base from the recorded
  keys (O3), and "same keys, different deployment" is presumed to be a
  mistake until the operator says otherwise.
- **Tombstone rows are operator deletes.** A recorded delete replays
  through the same class-conscious retire gate as a live one
  ([04-planes.md §1.2](04-planes.md), the v1.12 bullet): confirmation off
  the `state` class, wildcards impossible by construction (rows carry
  concrete keys).
- **Two replays exist; do not confuse them.** *Re-publishing* replay (the
  CLI) makes real puts through declared publishers and is governed by
  everything above. *Pane* replay (the GUI's scrubber) feeds a tool's own
  views from the file and never touches a session — it is reading, not
  publishing, and needs no etiquette beyond honesty about its mode.
- **A time scrubber states its clock.** A `.zrec` carries two clocks — the
  observer's arrival offsets (`"t"`) and the publishers' HLCs — and a
  consumer plotting a time axis says which one it plotted. The reference
  scrubber plots `"t"`.

## 6. Cutover acceptance

*Added in v1.2. Nothing in the convention said how you **prove** a
migration finished. [11](11-zensight-profile.md) scopes verification out
explicitly, so it belongs here.*

A cutover to (or between majors of) this convention is **not done** until
an isolated run demonstrates **both** of the following. One without the
other is not evidence.

**1. The retired key family is silent.**

Stand up the deployment, subscribe to the *whole* old root, and assert an
empty result set while the new planes carry traffic. A migration you can
assert the *absence* of is a migration you can finish; one you cannot is a
migration you merely believe in.

Note what this costs you if the version chunk is plain (`v1`, not `@v1`):
`<base>/**` reaches the new keys too, so the subscriber sees your own
traffic and the leak check must state its meaning explicitly — *anything
outside `<base>/v1/`* — rather than riding on key algebra. Same guarantee,
stated rather than inferred. (A verbatim version chunk gives the algebraic
version for free, and costs you zenoh-ext's `@adv` sidecars, which is a bad
trade — [03-grammar.md §1.2](03-grammar.md).)

**2. A consumer-shaped, concrete-key probe passes.**

> **A probe MUST build its keys the way the product builds them.**

A fleet-selector probe (`<base>/v1/*/@rpc/…`) **cannot** catch a broken
origin path: the `*` matches *any* origin, so a caller whose origin concept
is complete garbage still gets replies. The reference implementation's
smoke was green — subscribing on wildcards, asserting replies arrived —
while **every drill-down in the product was broken**, because the product
used concrete origins and the probe did not
([06-identity.md §6.3](06-identity.md)).

> **A test that uses a wildcard where the product uses a concrete value is
> testing a different program.**

So the probe MUST resolve an origin through the same bridge the consumer
uses ([06-identity.md §6](06-identity.md)) and then issue **origin-scoped**
calls — and it MUST fail if the bridge yields nothing. Absence of replies
and absence of *callers* must not look alike.

Recommended shape (both halves, one run): multicast off / gossip on
(§0.1), an explicit endpoint, the full producer set, one un-namespaced
observer for the honest wire view (§5), and a consumer-shaped phase that
resolves origins and drills down exactly as the UI does.
