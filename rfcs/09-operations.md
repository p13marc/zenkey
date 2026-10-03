# 09 — Operations Cookbook

**Status: v1.24** · informative chapter · *amended in v1.2, v1.4, v1.5, v1.9, v1.13, v1.18, v1.19, v1.21, v1.24, v1.27, v1.28, v1.31, v1.33, v1.38, v1.42, v1.43, v1.47 and v1.49 — see [CHANGELOG.md](CHANGELOG.md)* — the v1.24 amendment is the move: the tool-facing material (§5.1–§5.3, §6, including the former normative carve-outs) went to [13](13-observer-conformance.md), tombstones below

Worked recipes for the infrastructure concerns the grammar was shaped
around: session setup, subscriptions, storage, ACL, and constrained links.
The obligations of tools that only read the bus, and the judgment
procedures built on them, live in [13](13-observer-conformance.md) since
v1.24 — the tombstones below (§5.1–§5.3, §6) keep pre-v1.24 citations
resolvable. Base = `zensight` throughout; substitute your deployment's
base.

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
- **This block is generated, not typed** (v1.33). `zenctl storage gen
  --deployment <file>` takes a small TOML — the base, the volumes with their
  plugin (and, for `redb`, the per-volume history mode of §2.1), and per
  storage a class and a volume — and derives the rest: the selector
  (`@catalog` explicit), the `strip_prefix` as the literal leftmost run, and
  `garbage_collection.lifespan` as ⌈max covered `ttl_s` × margin⌉ with the
  computation shown (§2.3's rule, which nobody computed by hand). It refuses
  what the router would refuse — replication on an all-mode volume (§2.2), an
  undeclared volume, a class the registry declares nothing under — and
  emits the overlap and `complete` caveats of this section as comments
  beside the storage they concern. `--check` compares the plan with the
  storages the admin space reports; `--explain <key>` names the taker.
- The `timeseries` storage is **optional** where a history application
  runs (v1.31): the reference deployment answers charts from
  `@rpc/historian/range` — an ordinary producer that subscribes the
  telemetry class and keeps tiered series ([04 §4](04-planes.md),
  [11 §2](11-zensight-profile.md)) — and keeps the influx capture only where
  raw samples are wanted. The reasons recorded there: the influx backend
  cannot serve wildcard selectors, a `_time=` GET has no aggregation, an
  external database is the wrong shape for a 1 GB VM, and an out-of-tree
  router plugin cannot run in the CI that judges the deployment.

### 2.1 Choosing volumes

A backend advertises a capability pair — *persistence* (volatile/durable)
× *history* (latest/all) — and the class semantics pick it. The pair is not
always a property of the *backend*: a backend may offer both history modes,
in which case the choice is made **per volume** (v1.28, correcting v1.27)
and a deployment declares one volume per mode from the same plugin. The rest
of this section partitions by the volume's mode.

v1.27 said the choice was per *storage*, which was a reasonable reading of a
backend that had not shipped and is not what shipped. Zenoh asks the
**volume** for its capability, and the storage manager makes two decisions
from that answer which no individual storage can override: a storage
declaring `replication` fails to start unless its volume reports
latest-history, and in latest mode the manager discards outdated samples
*before* they reach the backend, while in all mode it forwards every one —
which is what makes the append stream complete. So the mode cannot be a
per-storage field over a shared volume without one of those two decisions
being silently wrong for half the storages on it.

| Volume | Capability | Use for | Caveats |
|---|---|---|---|
| `memory` (bundled) | volatile · latest | seed-only deployments, testing | gone on router restart — late joiners lose their seed until state refreshes |
| `fs` / `rocksdb` | durable · latest | `latest`, `catalog` — the LWW truth stores | out-of-tree plugins, version-matched to the router |
| `influxdb` | durable · all | `timeseries`, `events`, `pdns_history` — anything whose value is the *sequence* | retention lives in the database (RP), not zenoh config. **A sample's payload is stored as one string field, base64-encoded when binary**: a CBOR-serialized deployment gets a durable, time-indexed byte log — useful for `_time`-ranged GETs — and *not* a queryable time-series database. `SELECT mean(…)` over a payload field is not available (v1.27) |
| `redb` | durable · **per volume** (latest or all) | any of them; an all-mode volume for `timeseries`, a latest-mode one for `latest`/`catalog`/`events` | out-of-tree, version-matched. **Retention is in the backend, and mandatory** — the one row here that does not need the "retention is the database's, and Zenoh's `garbage_collection` is not it" caveat the other durable rows carry ([§2.3](#23-garbage-collection--tombstone-lifetime)); an all-mode storage that declares no retention policy **refuses to start**. An all-mode volume cannot host a replicated storage ([§2.2](#22-replication-ha-for-the-seed-store)). Time-ranged reads use the `_time` selector parameter; without it a key returns only its latest sample (v1.28) |

Storages are read-write by construction (there is no `read_only` field);
restrict who can write *into* a storage's selector with ACL, not storage
config. Out-of-order and wildcard writes are safe: the storage applies
updates by timestamp (an outdated sample is discarded, a wildcard delete
still masks a slower concrete put).

> **Deploying the `redb` row (v1.28).** v1.27 listed it ahead of the backend
> with a note saying so; the all-mode and retention work has since shipped
> (`zenoh-backend-redb` #10 and #11), and the note that told a reader to
> treat the row as a plan rather than a configuration is gone with it. Three
> things a deployment has to know, none of them derivable from the
> capability pair:
>
> - **One volume per mode.** `history: "all"` is a volume property, so a
>   deployment declares (say) `redb` and `redb-history` from the same plugin
>   and points each storage at the one it needs. That keeps the replication
>   choice explicit instead of silently stripping replication from every
>   storage sharing a volume.
> - **Retention is mandatory on an all-mode storage**, and its absence is a
>   startup refusal rather than a warning — a loud config error is
>   recoverable in seconds and a full disk is not. Two further shapes are
>   refused for the same reason, because both read as protection that is not
>   there: a `retention` block that sets no limit, and one on a latest-mode
>   volume, which has no history to prune and would report passes while
>   reclaiming nothing.
> - **A deletion is kept, as a tombstone at its timestamp.** Dropping it
>   would make the history claim the previous value was live up to the next
>   sample. Tombstones are never replied to; there is no value to return.
>
> Retention bounds age, total bytes and per-key sample count, and can
> decimate beyond a recent window — full resolution for the last day, one
> sample per bucket before that. This is a policy only a backend can apply:
> the router's downsampling interceptor shapes *traffic*, not storage, so it
> cannot thin data already on disk. Each pass reports what it cost on the
> admin space, which is what makes a bound checkable from outside the
> process — a storage claiming to be bounded is not the same as a storage
> that is ([§5.1](13-observer-conformance.md) O6, the same discipline
> applied to a daemon).

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
**only on latest-value volumes**. Anti-entropy aligns one value per key,
which is the whole of a latest-mode volume's content and a projection of an
all-mode one's — so a storage on an all-mode volume cannot participate.

**Configuring `replication` on one is refused, not discovered** (v1.28).
v1.27 said a deployment "should be told about" the misconfiguration; the
shipped behaviour is stronger and better: the storage manager asks the
volume for its capability and a storage declaring `replication` **fails to
start** unless the answer is latest-history. There is no state in which a
deployment believes it is replicating an all-mode storage.

The influx history storages do not replicate at the Zenoh layer; their
availability is the database's concern (cluster the database, or accept a
history gap on router loss).

This partitions by **volume mode, not by backend** (v1.28, correcting the
v1.27 revision). Until one backend offered both modes the two readings
agreed, and the rule could be stated as "latest-value *backends*". v1.27
separated them the wrong way, as "latest-value *storages*": the mode is the
volume's, so a deployment that wants both runs two volumes on one plugin
([§2.1](#21-choosing-volumes)) rather than mixing modes under one.

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

**It is not a retention policy, and there is no other one in Zenoh** — worth
stating because the names invite the opposite reading. `garbage_collection`
prunes metadata; it never drops a value, so a storage's data grows until
something outside Zenoh prunes it. For the durable rows of
[§2.1](#21-choosing-volumes) that "something" is the database's own policy
(an InfluxDB retention policy) or a schedule of your own against the volume
directory — size the volume against the write rate either way. A backend that
implements retention itself is the exception rather than the rule, and says
so in its row; `redb` is that exception, and on an all-mode volume its
retention policy is not optional (v1.28, [§2.1](#21-choosing-volumes)).

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
5. **Interest is evaluated on egress, against the responding face's
   subject** (v1.33, from the reference deployment's first live ACL,
   2026-08-30). A consumer's declares and queries are checked as they leave
   the router *toward each publisher*, against *that publisher's* policy —
   so a host whose grants are all own-origin (`…/h-xxx/**`) includes no
   wildcard-origin selector, a console's `…/v1/**` interest never reaches
   it, and a peer-mode publisher with no matching interest **publishes to
   nobody**, silently: no error at the console, nothing in the publisher's
   log, the liveliness token still up. The fix is one shared, **egress-only**
   rule allowing declares and queries on the fleet's selectors
   (`interest-prop` below), attached to every publishing principal — hosts,
   the catalog, desired-state authors. It is security-neutral: what is
   *published* (puts, replies, tokens) stays ingress-checked by the rules
   that were already there; this rule only lets interest through. Its
   corollary: `**` cannot cross `@catalog` any more than `@adv` (fact 2 and
   fact 1 together), so a catalog on the advanced tier needs
   `@catalog/**/@adv/**` spelled out.
6. **Deny is by inclusion, so a deny rule stops only a caller who spells a
   key at least as narrow as it** (v1.38). Fact 1 read from the other side:
   the router denies a message when a deny rule's key expression *includes*
   the message's, and a query on `…/v1/*/@rpc/**` is not included by a rule
   on `…/v1/*/@rpc/<producer>/**`, however literal the rule. Under a
   permissive default that query is forwarded to every queryable it
   intersects, and only the concrete-keyed *replies* are denied — after the
   query has crossed the face and been served; under `default_permission:
   "deny"` it crosses whenever an allow rule includes it, which the
   console's fleet grant (`…/v1/*/@rpc/**`, below) does. Two consequences.
   A constrained face (§4) denies a plane by the *widest* pattern that can
   reach it — `**/@rpc/**`: the verbatim chunk must be spelled by any query
   that reaches the plane, and `**` on either side includes every spelling
   around it — never by a producer-scoped literal. And no ACL refuses a
   broadcast *write*: that refusal is the server's
   ([05 §2.1](05-control-rpc.md), `error/fanout-forbidden`), and
   `no-remote-actions` below is the second lock on the door, for the
   callers who spell the key.

Six facts, ~30 rules, 8 subjects and a policy list for a six-host fleet,
every `key_exprs` entry carrying an `h-<12hex>` no human can proofread —
which is why the recipe went undeployed for a year. Since v1.33 the
reference tooling generates it: `zenctl acl gen --enrollment <file>` expands
this matrix from a small TOML binding certificate CNs — or, since v1.49,
zenoh `usrpwd` user names — to roles and origins
(given, or computed from a machine-id by [06 §1](06-identity.md)'s
derivation), narrowed by the registry to the planes each producer declares,
and `--explain <principal> <key> <message>` answers "which rule decided"
by keyexpr inclusion. **The running ACL is not observable**: zenoh 1.10's
admin space serves no GET under `config/**`, so `acl gen --check` compares
the plan against the router's *config file*, read through zenoh's own
loader, and an interest-propagation probe from the consumer side is *not
asked* — a publisher's matching status is the only place it shows.

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
| every publisher (host, catalog, desired-author) | `interest-prop` (v1.33) | the fleet's selectors, each plane named (`v1/*/state/**`, `…/telemetry/**`, `…/events/**`, `v1/*/@rpc/**`, `v1/@catalog/state/**`, `v1/@catalog/@rpc/**`, `@catalog/**/@adv/**` where the catalog runs the advanced tier) | **out only** | declare_subscriber, declare_liveliness_subscriber, liveliness_query, query — fact 5 |

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
    // (deny wins for a caller who spells this key; a broader `…/@rpc/**`
    // query is forwarded — fact 6 — and the server refuses it, 05 §2.1):
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

**Principals by user (v1.49).** A `[[principal]]` is bound by `cn`, by
`user` — the name a transport authenticated with zenoh's `usrpwd`, emitted
as the subject's `usernames` — or by both, which zenoh ANDs: a subject is
the product of its properties, and a transport matches it only when every
property does. `zid` stands in for either only as a prototype, as before.
A `user` binding is as strong as its channel: `usrpwd` authenticates at
establishment and nothing after it, so over a link that is neither TLS nor
otherwise protected, whoever is on the channel can still inject into a
live transport. The subject id defaults to the CN, then the user.

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

**Per-resource grants (v1.43).** The write story above is one switch —
`remote_actions` drops `no-remote-actions`, otherwise every declared write
is denied — and the sub-host paragraph promised a rule that allows
`…/config/*/eth1/set` and denies `…/config/*/mgmt0/set` without saying how
an enrollment asks for one. It asks with `writes`:

```toml
[[principal]]
cn = "radio-ops"
role = "console"
writes = ["modem/config/*/power/set", "modem/config/*/confirm"]
```

Each pattern is `<producer>/<procedure>` relative to `@rpc/`, spelled as the
registry spells the procedure with every `{var}` a `*` — or wider, taking in
several declared writes at once. **Not narrower** (v1.49): a pattern that
stops short of the declared write it falls in — `modem/config/rf0/power/set`
where the registry declares `config/{device}/power/set` — cannot carve it.
Deny is by inclusion, and no finite set of key expressions includes every
key of `config/*/power/set` but `config/rf0/power/set`: a `*` cannot be
subtracted from. The deny keeps the declared write whole, so the grant would
never decide, and the generator does not emit it and says why
(`grant_cannot_carve`) — before v1.49 it emitted the dead allow under a note
claiming the pattern matched nothing. Authority below a variable chunk is
the server's to enforce, or the registry's to make a literal chunk. The
generator emits one allow (`writes-<subject>`: `query`, both flows, every
pattern lifted under `v1/*/@rpc/`) and **carves the deny**: the principal's
`no-remote-actions` lists the declared writes a grant does not include, and
nothing else — because deny wins, a carve-out has to be an absence (fact 6),
which is also why a grant needs the registry: without one the deny is the
unnarrowed `set` leaf, nothing can be subtracted from it, and the generator
keeps the deny whole and says why rather than emit a grant that would never
decide. A `sensitive` procedure ([08 §2](08-registry.md)) is in the deny for
every principal, including one with `remote_actions = true`, unless a
`writes` pattern includes it. A watch refuses `writes` as it refuses
`remote_actions`. `--explain` answers a per-resource question the same way
it answers any other — which rule, which direction.

**A configuration resource, worked (v1.42).** [05 §5.1](05-control-rpc.md)'s
convention as a producer registers it and an ACL then grants it — a radio
driver, one device chunk as the resource, the group as the next chunk, so
that every authority an operator will want to express is a literal prefix.
The registry slice, with the four entry kinds it takes:

```toml
[[procedure]]
path = "config/{device}"
kind = "read"
reply = "ConfigView"
idempotent = true
since = "1.0"
description = "the read-back: the served schema beside every running value, its source, the revision, any pending change (RFC 05 §5.1)"

[[procedure]]
path = "config/{device}/{group}/set"
kind = "write"
fanout = "forbidden"
request = "ConfigChange"
reply = "ConfigView"
since = "1.0"
description = "one group's change; hot: applied and read back — reach: answered {token, apply_at} before it is applied, and only with confirm_s — contract: refused, restart-required"

[[procedure]]
path = "config/{device}/confirm"
kind = "write"
fanout = "forbidden"
request = "ControlRequest"
reply = "ConfigView"
since = "1.0"
description = "make the pending change permanent; cancel and extend are its siblings"

[[procedure]]
path = "config/{device}/persist"
kind = "write"
fanout = "forbidden"
request = "ControlRequest"
reply = "Ack"
since = "1.0"
description = "write the confirmed change into the persisted layer — its own key, so it is its own grant"

[[subject]]
path = "config/{device}"
class = "state"
type = "ConfigView"
qos = "transition"
cardinality = 8
ttl_s = 0
since = "1.0"
description = "the read-back, refreshed on every change (RFC 05 §5.1)"

[[subject]]
path = "config_change/{event_id}"
class = "events"
type = "ConfigChangeEvent"
rate = "low"
replay = "window(24h)"
cardinality = 1000
since = "1.0"
description = "every change, after RFC 6470: the edits with sensitive values redacted, the outcome, the claimed actor"

[[error]]
name = "restart-required"
procedures = ["config/{device}/{group}/set"]
since = "1.0"
description = "the group is contract: set it in the startup configuration and restart"

[[error]]
name = "device-refused"
procedures = ["config/{device}/{group}/set"]
since = "1.0"
description = "the device refused, in its own words"
```

The grants that fall out of the key shape, each one a literal prefix a
person can proofread:

| Who | May | Rule (`…/v1/h-xxx/@rpc/modem/`) |
|---|---|---|
| anyone the console trusts | read the document | `config/**`, `query` in — and `state/modem/config/**` as any state |
| the field operator | change `rf0`'s hot groups, arm and confirm a reach change | `config/rf0/*/set`, `config/rf0/confirm`, `config/rf0/cancel`, `config/rf0/extend` |
| the field operator | **not** make it survive a restart | deny `config/rf0/persist` — the reason `persist` has its own key |
| the same operator | nothing on `sat0` | no rule names it; under `default_permission: "deny"`, absent is denied |
| a script with a stale spelling | nothing by accident | the server's exact-key check ([05 §2.1](05-control-rpc.md)) refuses `config/*/radio/set`, whatever the ACL forwarded |

These rows are rule *shapes*, as a hand-written block under
`default_permission: "deny"` with no broader `@rpc` grant would spell them.
The generator's console carries the fleet's `…/v1/*/@rpc/**` read grant, so
its write authority is a carve of the declared writes, and the per-device
rows would need `rf0` subtracted from the declared `{device}` — which, by
the paragraph on per-resource grants, no deny can do (v1.49).

`zenctl config get h-xxx modem rf0` renders the first row's answer as the
document it is; `zenctl config set h-xxx modem rf0 radio frequency_khz=868100
--confirm 60` is the second row's, typed against the served schema, and it
asks before sending, because `radio` is a reach group and the reply may not
come back the way the request went. The sensitive marker's ACL half —
`acl gen` denying a sensitive procedure by default — is [08 §2](08-registry.md)'s
`sensitive = true`, separate work.

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

**The reference tooling generates the face (v1.43).** `zenctl acl gen
--enrollment <file> --registry <dir> --face constrained --link-protocol
unixsock-stream --link-interval 60` emits the two blocks above for one
constrained face, from the registry's `exposure` markers
([08 §2](08-registry.md)) rather than from a hand-kept list:

- `access_control` with **`default_permission: "allow"`**, because the
  permission is node-global — a face-scoped deny under a `deny` default
  would black-hole every other face, so a carve-out on a constrained link
  has to be a *deny* under a permissive default, and the host bus keeps
  flowing by absence. One subject, selected by the link (`link_protocols`
  or `interfaces` — a unixsock-stream link reports no interface name in
  zenoh 1.10, so a modem lane is selected by protocol) — and one more per
  enrolled operator, below (v1.49).
- A deny per `host`-exposed subject, by its pattern (`{var}` as `*`) — or,
  when every subject of a class under a producer is `host`, one deny on
  `v1/*/<class>/<producer>/**`, which also covers the framework keys under
  it; every message kind the class can carry, both flows spelled.
- The planes denied by the **widest** spelling — `**/@rpc/**` when any
  procedure is `host`, `**/@media/**` and `**/@blob/**` always — and the
  admin space and the sidecars (`@/**`, `**/@adv/**`) always: fact 6, a
  plane deny that names a producer is crossed by a broader query.
- `downsampling`, **egress**, `put`, one rule per `link`-exposed subject at
  `1 / --link-interval` Hz; one rule per key and never two that intersect,
  because zenoh resolves a key to the first intersecting rule and gives
  each rule one timer. `--link-interval none` keeps the `link` subjects
  home too — the link where no rate is affordable, a billed satellite
  channel — and emits no downsampling block.
- A note naming what still **crosses**: every `fleet` entry and every
  unmarked one. The default is `fleet`, so an unmarked registry crosses
  whole, and the profile says so rather than deny what nobody classified —
  the registry is where the classification belongs.

`--check --against` compares a face plan as it compares the principal
plan; `--explain <face-id> <key> <message>` answers over it. The two blocks
merge at the router config's top level. They do **not** merge beside a
principal plan's block (corrected in v1.49): a router has one
`access_control` and one node-global `default_permission`, which the face
needs `allow` and the principal plan needs `deny`; and policies on subjects
that match the same transport are never separate — zenoh evaluates every
matching subject, and any one's allow wins (below).

**Principals on a face (v1.49).** An authenticated face is where remote
configuration becomes a grant rather than a switch: the enrollment's
`[[principal]]`s ride `--face`, and a `user` console or watch — an
operator on the far side, authenticated by the face's `usrpwd` — is planned
onto it. Three facts of zenoh 1.10's matcher
(`zenoh-1.10.0/src/net/routing/interceptor/`) decide the shape:

1. A transport matches **every** subject whose properties all match. The
   face's subject names only the link, so it matches the operator's
   transport too.
2. Within one subject's policy deny wins, and under this block's `allow`
   default a policy evaluates Allow unless a deny includes the key.
3. Across the matching subjects **any Allow wins** (`AclActionMethods::
   action`). And subjects with identical properties are one subject
   (`SubjectMapBuilder::insert_or_get`), so their policies merge and deny
   wins again.

So a principal's grant cannot sit *beside* the face's denies: a subject
holding only an allow evaluates Allow for everything the face denies, and
wins. The generator gives the principal a subject of its own —
`usernames: [<user>]` **and** the face's `link_protocols`/`interfaces`, so it
matches only the operator on that link — whose policy **repeats every face
deny** but `deny-rpc`. That one is split: the legs no call uses (`put`,
`delete`, `declare_subscriber`) stay denied on `**/@rpc/**` whole
(`deny-rpc-legs`), and the three a call does — `query`, `reply`,
`declare_queryable`, the last because between two peers a call is routed
only once the queryable's declaration has crossed — are carved to §3's
console shape: the `writes` grants allowed, `no-remote-actions` denying the
declared writes no grant includes, a `sensitive` write denied unless named.
The operator therefore reads the `@rpc` plane, as a console does, and writes
exactly what it was granted. Its denies are a subset of the face's, so its
own evaluation is its whole answer; everyone else on the link — another
user, or a peer with no user at all — matches only the face's subject and
keeps every deny. A wildcard query broader than the carve
(`…/v1/*/@rpc/**`) is included by none of the operator's denies and
crosses (fact 6); the second lock on a broadcast write is the server's own
refusal ([05 §2.1](05-control-rpc.md), #472), as it is for a console
anywhere.

Refused by name, never dropped: a `cn` or `zid` principal (a certificate
identity belongs to the principal plan, whose router cannot share this
block's default); a principal with no `user`; a host, catalog,
desired-author or link (a publisher's grants are the principal plan's); a
watch with a grant; and every principal on a face that denies no `@rpc` —
there the plane already crosses for every peer, and a principal's subject
can only widen what the face's own allows. `--explain <user> <key>
<message>` answers for the operator.

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
- **An explorer's session posture (v1.47).** A tool that observes a
  deployment it does not belong to opens its session in this posture
  unless its user states otherwise:
  1. **`mode: "client"`.** An observer is not a routing node. A peer
     listens on every interface, gossips its locator, and is offered
     direct links that churn every time the tool exits; pointed at a
     production router, that is a debugging laptop joining the mesh. It is
     a peer only when the user gives a listen endpoint, or when the user's
     Zenoh config **states** `mode`.
  2. **Multicast scouting off** (§0.1), with or without a config file. A
     key the user's file states is the user's choice; a key Zenoh
     *defaulted* while loading the file is not — Zenoh's defaults are
     `peer` and multicast on. A tool that layers this posture over a user
     file has to tell the two apart, or the path a secured deployment uses
     (TLS, credentials — always a file) silently undoes the posture.
  3. **Endpoint errors refuse.** An endpoint that does not parse is
     refused by name before a session is attempted, never dropped. A
     session whose endpoints do not answer is a failure to observe, not an
     observation ([13 §3](13-observer-conformance.md)): a client fails to
     open (Zenoh's client defaults, `connect.timeout_ms` 0 and
     `exit_on_failure` true), and the tool reports no verdict — never an
     empty bus. `zenctl`, `zengui` and `zenwatch` take this posture from
     one session builder (`zenkey_fleet::open_reporting`).

### 5.1 Observer obligations — moved to [13 §3](13-observer-conformance.md) (v1.24)

The observer obligations O1–O7 and the frugality note live in
[13 §3](13-observer-conformance.md), wording and numbering untouched — a
pre-v1.24 citation of "RFC 09 §5.1 O*n*" resolves to 13 §3 O*n*. What the
move added (the per-medium consequences and the conformance-test shape) is
new text in 13, not a change to any rule.

### 5.2 Capture and replay — moved to [13 §4](13-observer-conformance.md) (v1.24)

The `.zrec` etiquette and the re-stamping decision live in
[13 §4](13-observer-conformance.md), carried over verbatim; the format
additionally gained a minimal normative contract there (13 §4.1 — before
v1.24 the reference implementation's code was normative for the format).

### 5.3 Synthetic traffic — moved to [13 §5](13-observer-conformance.md) (v1.24)

The synthetic marker and the generator's etiquette live in
[13 §5](13-observer-conformance.md), unchanged: same attachment shape, same
deliberate non-marking of replayed-real traffic and of `spray`.

## 6. Cutover acceptance — moved to [13 §6](13-observer-conformance.md) (v1.24)

The two-halved acceptance procedure (retired family silent **and** a
consumer-shaped concrete-key probe) lives in
[13 §6](13-observer-conformance.md), unchanged — it is a judgment
procedure, and it moved with the rest of them.
