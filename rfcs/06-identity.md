# 06 — Identity, Origins, and the Catalog

**Status: v1.2 (ratified)** · normative chapter · *amended in v1.2, v1.25, v1.29 and v1.30 — see [CHANGELOG.md](CHANGELOG.md)*

The grammar puts a stable identity in every key (the origin chunk,
[03-grammar.md §1.3](03-grammar.md)). This chapter defines how that identity
is minted, how observed (proxied) devices are named, and the contract of the
`@catalog` service that turns per-origin claims into merged entities.

The design resolves the tension every identity-in-key scheme faces:

> A publisher must know its key at first startup, alone. But the *correct*
> entity identity (this modem and that hostname are the same box) is only
> knowable later, centrally, from accumulated evidence.

The resolution: **origins are self-minted and never re-keyed; the catalog
maps, it does not rename.**

---

## 1. Host origins — self-minted, stable, opaque

`h-<12hex>`. Reference derivation, byte-precise (two independent
implementations MUST mint the same id for the same machine):

```
input   = machine_id_hex ++ salt
machine_id_hex = the 32 lowercase-hex chars of /etc/machine-id,
                 whitespace/newline trimmed
salt    = the application salt, as UTF-8, no separator
origin  = "h-" ++ lowercase_hex(sha256(input))[0..12]
```

Test vector: machine-id `b642b4217b34b1e8d3bd915fc65c4452`, salt
`example-salt-v1` → `h-` + first 12 hex of
`sha256("b642b4217b34b1e8d3bd915fc65c4452example-salt-v1")` =
`h-20609002f7b6` (implementations MUST reproduce this).

- **Self-minted**: derivable at first startup with no coordinator. Every
  publisher on one machine derives the same value — so all producers of a
  host agree on their origin without talking to each other.
- **Stable**: survives restarts, upgrades, renames, re-addressing. A
  hostname change is a catalog event, not a re-key. (A **machine-id**
  change is a new host — see §5.4.)
- **Opaque**: reveals nothing (the salt keeps the machine-id private) and
  promises nothing — it is an *address*, not a description. What the origin
  *is* (its names, addresses, roles, kind) lives in catalog documents and
  can be corrected freely ([02-principles.md P8](02-principles.md)).
- **Salt scope**: the salt is an **application constant** — fixed at
  application level, identical across every deployment of that application,
  compiled in, not operator-configurable (the reference application ships
  `"zensight-host-id-v1"` as a non-configurable constant). This is what
  makes origin id ≡ entity id hold across a fleet without coordination.
  Changing the constant is an application-breaking change that re-keys
  every fleet; it is part of the identity function and MUST be treated as
  such. (A deployment-chosen salt would also work but breaks id
  portability between deployments of one application; an application MUST
  pick one model and document it.)
- **Collisions**: 48 bits of id means truncation collisions are possible
  in principle (birthday: ≈ 1.8 × 10⁻⁹ at 1 k hosts, 1.8 × 10⁻⁵ at 100 k,
  0.18 % at 1 M). The convention *detects* rather than prevents: the
  catalog MUST raise an operator-visible conflict when disjoint
  `evidence/self` claims (different machine-id hashes, different
  hostnames) persist under one origin — that signature has exactly two
  causes, id collision or origin spoofing, and both demand an operator.

### 1.1 Hosts without a machine-id

Constrained or ephemeral hosts (containers, RTOS nodes) MUST still mint a
stable id, in order of preference:

1. a persisted random id: 6 random bytes rendered as 12 hex, generated
   once, written **atomically** to an application-defined well-known local
   path (write-temp + rename), and read by every producer thereafter.
   Because this is a shared file, not a derivation, the
   all-producers-agree invariant needs the file: producers racing at first
   boot MUST create it with an atomic create-exclusive (loser re-reads);
2. a hash of the most stable hardware identity available (primary MAC,
   serial), derived as in §1 with the hardware id in place of the
   machine-id.

The requirement is stability + uniqueness, not provenance; the catalog's
evidence model (below) absorbs the difference in confidence. Note the
"same value on one machine" invariant of §1 holds *by derivation* only for
machine-id (and option-2 hardware) hosts; option-1 hosts get it from the
shared file.

## 2. Origins vs entities — the two-layer contract

- The **origin** is the *publisher's* identity claim: "these keys come from
  the same place." It is in the key because routing, ACL, and grouping need
  it before any correlation exists.
- The **entity** is the *deployment's* identity conclusion: "these origins
  and these observed devices are one host." It is computed by the catalog
  from evidence and can change as evidence improves — which is exactly why
  it MUST NOT be in data keys.

When the machine-id is known, the reference derivation makes the origin id
and the entity id **the same value** — the common case needs no mapping at
all. When identities merge or upgrade (a weakly-identified origin later
proves to be an already-known machine), the catalog publishes an alias, and
consumers re-group; no publisher re-keys, no history is orphaned.

## 3. Observed devices are subjects, not origins

A proxy producer (SNMP poller, Modbus master, gNMI collector, NetFlow
receiver) speaks *about* other devices. Those devices go in the **first
subject chunk**, never in the origin:

```
zensight/v1/h-3fa9c2d41b7e/telemetry/snmp/router01/system/sys_uptime
                            ^ the poller's host      ^ the observed device
```

- The origin answers "who do I trust / throttle / ACL" — that is the
  machine running the producer, not the router it polls.
- The device chunk is the producer's local name for the device (config
  name, slugged). Cross-producer device identity (the same router polled
  from two collectors) is — like all identity — a catalog conclusion, fed
  by observed-evidence claims (§4).
- A device MAY be promoted to a first-class origin only by *running a
  publisher itself*; a deployment that wants per-device ACL on proxied
  devices is asking for a different trust model — decided against, with
  the revisit trigger recorded in
  [12-open-questions.md §2](12-open-questions.md).

## 4. Evidence — identity claims as ordinary state

Identity evidence is not a separate metadata plane (the incumbent
`_meta/evidence/**`); it is ordinary per-origin `state`, because that is
what it is — a producer's current claim, refreshed on a cadence, stale when
unrefreshed:

| Key | Claim |
|---|---|
| `<base>/v1/<origin>/state/<producer>/evidence/self` | "my host is: hostname H, machine-id-hash M, addresses A…" (self-report) |
| `<base>/v1/<origin>/state/<producer>/evidence/device/<device>` | "device `<device>` I observe has: sysName, MACs, addresses…" (third-party claim, weighted lower) |
| `<base>/v1/<origin>/state/<producer>/evidence/names/<ip-slug>` | "IP X currently resolves to name N" (passive DNS observation) |
| `<base>/v1/<origin>/state/<producer>/evidence/relation/<relation-id>` | "these two things are connected, this way" (structural claim, v1.30 — §5.6) |

- Claims carry `last_updated`; every consumer of evidence (the catalog
  first among them) MUST ignore claims older than the subject's registry
  TTL, and publishers MUST refresh live claims at ≤ TTL/2 — the TTL value
  is the registry's, authoritative for both sides
  ([04-planes.md §1.2](04-planes.md); the reference registry sets 900 s).
- `evidence/names/<ip-slug>` is **population-keyed state** and carries the
  mandatory cardinality budget of [04-planes.md §1.2](04-planes.md) —
  profile-sourced material, kept here as the worked example of that
  budget: the passive-DNS family is the reference application's
  ([11-zensight-profile.md](11-zensight-profile.md)), not an obligation on
  every adopter. The
  registry entry declares the expected population bound, and the publisher
  tombstones entries whose observation has aged past TTL. An
  internet-facing sensor MUST aggregate or sample before publishing — the
  per-IP key family is for the *actively observed* set, not for every
  address ever seen (that history belongs to the catalog's storage tier,
  §5.2).
- `evidence/relation/<relation-id>` (v1.30) is a claim about a
  **relationship** rather than about an identity, and it is deliberately
  inside this family rather than beside it: the catalog's input contract
  stays *"evidence only"*, the one selector below already matches it, and
  merge remains a pure function of evidence. `relation-id` is derived from
  the claim's `(kind, from, to)` and nothing else — never from the
  timestamp or the publishing sensor — so a sensor re-observing the same
  relationship overwrites its own previous claim on one key instead of
  accumulating one key per refresh. That is the difference between a
  bounded family and a leak. §5.6 defines what the catalog concludes from
  these.
- The catalog subscribes to one selector:
  `<base>/v1/*/state/*/evidence/**`.

## 5. The `@catalog` service

`@catalog` is the reserved service origin ([03-grammar.md §3](03-grammar.md))
for the deployment's identity/ontology service (the reference
implementation: `zensight-correlator`).

```
<base>/v1/@catalog/state/entity/<entity-id>      merged entity document (LWW, tombstoned on retire/merge)
<base>/v1/@catalog/state/alias/<old-id>          alias record: old-id → entity-id (id upgrades, merges)
<base>/v1/@catalog/state/pdns/<ip-slug>          accumulated IP↔name record (historical tier via storage)
<base>/v1/@catalog/state/edge/<edge-id>          resolved relationship between two endpoints (LWW, tombstoned when unclaimed, §5.6)
<base>/v1/@catalog/state/incident/<incident-id>   firing alerts grouped by entity (LWW, tombstoned when none is firing, §5.5)
<base>/v1/@catalog/state/ack/<alert-ref>         an operator's acknowledgement of one firing alert (§5.5)
<base>/v1/@catalog/state/silence/<id>            a suppression window (§5.5)
<base>/v1/@catalog/state/alive                   liveliness token (declared by the elected owner, §5.3)
<base>/v1/@catalog/state/claim/<zid>             liveliness claim tokens (ownership protocol, §5.3)
<base>/v1/@catalog/@rpc/names                    on-demand name resolution (?ip=…)
<base>/v1/@catalog/@rpc/ack                      acknowledge a firing alert (write, gated, §5.5)
<base>/v1/@catalog/@rpc/unack                    retire an acknowledgement (write, gated, §5.5)
<base>/v1/@catalog/@rpc/silence                  open a suppression window (write, gated, §5.5)
<base>/v1/@catalog/@rpc/unsilence                close one early (write, gated, §5.5)
```

Contract:

- **Single writer.** Exactly one catalog instance publishes under
  `@catalog`, arbitrated by the ownership protocol of §5.3. Everything
  under `@catalog` is a *conclusion*; conclusions have one author.
- **Pure function of live evidence.** The entity set is recomputed from the
  current evidence state (union-find over ranked identity rules — strong
  ids join, weak ids like bare IP/MAC never join alone, conflicting strong
  ids block a merge). A restarted catalog reseeds from evidence and reaches
  the same conclusions: no private database, no migration state.
- **Stable entity ids, aliases on upgrade.** `entity-id` = the machine-id
  hash form when known (== the host origin id, §2), else derived from the
  best available evidence. When an entity's id upgrades or two entities
  merge, the losing id gets an `alias/<old-id>` record and its entity
  document a tombstone; consumers re-point. Ids never round-robin.
- **Consumers join, producers don't wait.** Nothing a producer publishes
  depends on the catalog; if it is down, entities go stale and consumers
  degrade to grouping by raw origin — the same data, one join weaker.
- **Kinds, names, roles, relationships live here** — in entity documents
  and, since v1.30, in the `edge` family beside them (§5.6) — and nowhere
  in any key ([03-grammar.md §6.2](03-grammar.md), the ontology-in-key
  rejection). An edge id is an opaque hash of the relationship it names,
  not a parseable encoding of its endpoints: it is a key, and reading
  structure out of a key is the thing that rejection forbids.

### 5.1 How a UI joins

1. Subscribe `<base>/v1/@catalog/state/entity/*` **and**
   `<base>/v1/@catalog/state/alias/*` (+ GET the same selectors as the
   late-joiner seed, [04-planes.md §3.2](04-planes.md)) — alias
   records are their own key family, and without them step 3's
   origin→entity re-pointing on merges never arrives.
2. Group data keys by their origin chunk — a plain string read at
   position 3, no parsing heuristics.
3. `entity.origins[]` (and `alias` records) map origin → entity;
   unmatched origins render as bare hosts until evidence catches up.
4. Names for arbitrary external IPs (every CDN the flow sensor ever saw)
   are pulled on demand: `GET …/@catalog/@rpc/names?ip=…` — never
   broadcast.

**`origins[]` is normative, and v1.30 says what it contains.** The entity
document MUST carry the set of origin chunks the catalog resolved to that
entity — [§6.4](#64-consequence-for-the-entity-document) has required the
field since v1.2, and step 3 has named it since v1.0 — and it MUST contain
only **self-reports**: origins whose evidence claimed *its own* host, never
a third-party claim about someone else's. That last rule is the part that
was missing, and it is not a detail. A hypervisor publishing
`evidence/device/<guest>` would otherwise bind the *hypervisor's* origin to
the guest's entity, and every consumer would inherit the error.

Two chapters requiring a field is not the same as a field existing, and
this one did not. Consumers reconstructed the join instead, each
differently, by walking the **evidence** subtree and matching
`(sensor, source)` against `members[]` — which is wrong in exactly the two
ways this section is careful about elsewhere:

- **It is a heuristic**, not the lookup step 2 promises. Which member
  matched, and in what order, decides the answer.
- **It needs a subscription step 1 exists to avoid.** The evidence subtree
  is far larger than the entity family, so a consumer holding the entity
  documents alone — an exporter, a notifier, a second console — could not
  perform the join at all. Step 1 says keep the subscription small; step 3
  required the opposite.

Publishing the set is not new authority. The catalog is the party that ran
the union-find, and the origins are simply the keys the evidence arrived
on: this is the §5 promise that everything under `@catalog` is a
*conclusion*, applied to the one conclusion consumers needed most.

The field is additive and MAY be absent (an older catalog); a consumer
reading an entity without it falls back to grouping by bare origin, the
same "one join weaker" degradation as a catalog that is down.

### 5.2 The historical tier is a storage choice

*(Profile-sourced: the passive-DNS family is the reference application's
([11-zensight-profile.md](11-zensight-profile.md)); it stands here as the
worked example of the storage-as-history pattern, not as a contract every
catalog must offer.)*

`state/pdns/<ip-slug>` is LWW state (the latest accumulated name-set per
IP). Pointing a time-series storage at
`<base>/v1/@catalog/state/pdns/**` captures every transition — the
IP↔name history — with no dedicated plane and no consumer on the live bus
([04-planes.md §4](04-planes.md)). This is the catalog's *budgeted*
population-keyed state family ([04-planes.md §1.2](04-planes.md)) — the
one place per-IP keys are the design, aged and tombstoned by the catalog —
and the verbatim `@catalog` origin keeps it structurally out of every
fleet selector (design property D4).

### 5.3 Ownership protocol

Zenoh has no name-ownership primitive (a liveliness token is presence, not
a lock — two sessions can hold the same key). Ownership of a service
origin is therefore arbitrated by an explicit claim protocol, modelled on
D-Bus well-known-name ownership ([10-prior-art.md](10-prior-art.md)):

1. **Claim.** Each candidate declares a liveliness token at
   `…/@catalog/state/claim/<zid>` (its own Zenoh session id, lowercased),
   then queries liveliness on `…/state/claim/*`.
2. **Election.** The owner is the candidate whose claim chunk sorts
   lexically lowest — deterministic and coordinator-free; every candidate
   computes the same winner from the same token set, so simultaneous
   starts converge without messages.
3. **Standby.** Non-owners MAY keep their claim declared and idle
   (D-Bus `IN_QUEUE`), watching a liveliness subscriber on
   `…/state/claim/*` (the `NameOwnerChanged` analog). When the owner's
   claim retracts — crash, shutdown, disconnect — each standby re-runs
   step 2. A candidate that would rather exit than queue undeclares its
   claim and leaves.
4. Only the owner declares `…/state/alive`, the `@catalog` publishers, and
   the `@rpc/names` queryable; a standby declares nothing but its claim.

**What this does and does not guarantee.** On a connected network:
exactly one owner, automatic failover. It is *not* mutual exclusion:
during a **partition**, each side elects its own owner and both write; a
**deposed or paused ex-owner** learns of its loss asynchronously (no
fencing) and its buffered writes can land after the new owner's.
Mitigations, in order of force: every `@catalog` document carries the
writer's claim id and an `elected_at` incarnation timestamp, so dual
authorship is *detectable* (consumers and storages alert on incarnation
regress); and the pure-function contract above makes split-brain
*convergent* — after heal, the surviving owner's next full recompute over
the merged evidence overwrites interleaved conclusions. The convention
accepts **eventual** single-writer, not linearizable single-writer, and
says so.

### 5.4 Reinstall — machine-id change

A reinstall (new machine-id, same hardware) is neither an id upgrade nor a
merge: the host mints a *new* origin while evidence for the old one may
still be live, and the "conflicting strong ids block a merge" rule then
correctly *prevents* automatic linking — the catalog cannot know a
reinstall from two machines. The convention is honest about the default:
**a machine-id change is a new host**; after the old origin's evidence
ages out, its entity document is tombstoned and its history remains under
the old origin, reachable only via storage.

Deployments that want continuity assert it explicitly:
`GET …/@catalog/@rpc/link?old=<id>;new=<id>` (operator-invoked, gated) —
the catalog records the assertion as operator evidence (strong, does not
age out), publishes `alias/<old-id>`, and merges. Retention differs by
record kind: an **alias** is an ordinary put and persists in the
latest-value storage until an operator retires it (`@rpc/unlink`), with
catalog-side GC once unresolved for a deployment-configured horizon; an
entity **tombstone** is storage *metadata* and lives exactly as long as
the storage's `garbage_collection.lifespan`
([09-operations.md §2.3](09-operations.md)) — deployments running
replicated catalog storages SHOULD size that lifespan to their
partition-heal horizon, because a pruned tombstone is what lets a slow
replica resurrect a merged-away entity.

### 5.5 Incidents, acknowledgement and silence (v1.29)

The catalog answers *"what is on fire, whose problem is it, and is anyone
on it"* for the same reason it answers "who is this host": it is the only
participant that has run the union-find, and therefore the only one that
can say **this alert and that one are about the same machine**.

Three families, all `state`, all conclusions with one author (§5):

- **`incident/<incident-id>`** — the currently-firing alerts for one
  entity. `incident-id` is `inc-<entity-id>`, or `inc-<origin>` for an
  alert whose origin resolves to no entity. Tombstoned when no member is
  firing. A **timeline is not carried here**: a document that accumulated
  every transition would grow without bound on a TTL'd LWW key, and
  history is a storage concern (§5.2).
- **`ack/<alert-ref>`** — one operator's acknowledgement of one firing
  alert, `ttl_s = 0` (operator intent does not age out; the catalog
  tombstones it).
- **`silence/<id>`** — a suppression window with matchers, bounds and an
  author.

**`alert-ref`** is the alert's identity as **one key chunk**:
`<origin>.<producer>.<alert_key>`, defined byte-precisely in
[11-zensight-profile.md §3.2](11-zensight-profile.md). It has to be a
single chunk because it is the *last* chunk of `ack/<alert-ref>`, and a
key cannot nest inside a key.

#### The lifecycle rules are normative

They are normative because a **key-agnostic consumer** — an exporter, a
notifier, a second UI — must reach the same conclusion as the catalog
from the documents alone. A rule that lived only in the catalog's code
would make every other consumer guess.

1. **An ack applies only while a firing alert with
   `timestamp <= ack.fired_at` exists.** Two consequences, both
   intended:
   - an **orphan is inert**. An ack that outlives its alert — a catalog
     died holding it — reads as nothing, rather than as a silent
     suppression of the next occurrence. A stale document MUST NOT be
     able to hide a live problem.
   - a **re-fire is not acknowledged**. `fired_at` pins the ack to the
     occurrence someone looked at; when the condition clears and returns,
     the new alert's `timestamp` is later and the ack stops applying.
     That is the distinction between an ack and a silence, expressed as a
     field rather than as prose.
2. **`ack` MUST be refused when no alert is firing for the ref**
   (`error/catalog/not-firing`). An acknowledgement names an occurrence
   someone looked at; one for a problem nobody has would sit on the key,
   inert by rule 1, and then apply the moment that exact alert next fired
   within its `fired_at`.
3. **The catalog tombstones an ack** when its alert resolves or is
   tombstoned, and when a re-fire carries `timestamp > fired_at`. Rule 1
   already makes such an ack inert; the tombstone is the difference
   between *inert* and *gone*, which is what an operator sees when they
   list what is acknowledged.
4. **A silence applies while `starts_at <= now < ends_at`** and all its
   matchers match. It holds **across re-fires** — that is what a
   maintenance window means. The catalog tombstones it at `ends_at`, and
   a consumer MUST stop applying it at that instant whether or not the
   tombstone has arrived, so a partitioned reader cannot keep an expired
   suppression alive.
5. **An empty matcher set matches nothing.** The vacuous reading — "all
   zero conditions hold" — is how one mistake mutes a fleet, and the harm
   is asymmetric: refusing to suppress costs a page; suppressing
   everything costs an outage nobody hears about.

#### The four procedures

`ack`, `unack`, `silence`, `unsilence` are `kind = "write"`
([08-registry.md §4](08-registry.md)) and **gated by the same switch as
`link`/`unlink`** (§5.4): all six change what the deployment believes
about itself on an operator's say-so. A gated procedure MUST still be
*served*, replying `error/gated`, so an operator learns the feature
exists and is switched off rather than learning nothing from a timeout
([05-control-rpc.md §3](05-control-rpc.md)).

A silence MUST be validated before it applies — at least one matcher,
every matcher naming a matchable field, every regular expression
compiling, `ends_at` after `starts_at` — and its author MUST come from
the call's actor, never from the body: a silence whose author is
self-reported is a silence nobody can be asked about, and "who muted
this" is the first question of any review.

#### What this deliberately is not

Notification routing, escalation, on-call rotations, repeat intervals,
`for`-grouping. Those are a notifier's concern (the reference
deployment's is `zenwatch`, which scoped them out deliberately and reaches
an on-call product by webhook). These families are the **documents such a
tool reads**; the convention does not compete with one.

---

### 5.6 Relationships — evidence in, edges out (v1.30)

The catalog already answers *"who is this host"* and *"whose problem is
this"*. This section is the third question of the same kind: **what
depends on what**. It is one question because it has one answer-giver —
resolving "the guest named `db01`" or "the MAC behind that gateway" to an
entity requires the union-find, and nothing else on the bus has run it.

Relationship-shaped facts are already published everywhere, as plain
fields on unrelated documents: a guest's hypervisor node, a container's
host and owning unit, a probe's vantage and its target, an ARP neighbour,
a default gateway. Each is a string on somebody's state doc, legible only
to a consumer that knows that sensor's payload type. The split below turns
them into one vocabulary without making the catalog learn any of those
payload types.

#### The split: sensors claim, the catalog concludes

**Sensors publish `evidence/relation/<relation-id>`** (§4) carrying
`{ sensor, source, kind, from, to, attrs, last_updated }`, where `from` and
`to` are **claims**, not identities: observable attributes only — a host
id when the end *is* the publisher's own host, an observed-device slug in
the same vocabulary as `evidence/device/<device>`, IPs, MACs, a display
name. A sensor does not know entity ids and MUST NOT invent them, exactly
as it does not for host identity.

**The catalog publishes `edge/<edge-id>`** carrying
`{ edge_id, kind, from, to, attrs, observers, last_updated }`, where each
end is **resolved**: an entity id, or an honest `External` for something
the fleet observed but runs no sensor on (an upstream router, a probe
target on the public internet).

The two ends being *different types* is deliberate. Collapsing them into
one would force every consumer to ask "is this resolved yet?" on every
read, and would let an unresolved claim reach a UI looking like a
conclusion.

#### Normative rules

1. **`edge-id` is derived from `(kind, from, to)` after resolution, and
   from nothing else** — never from the observer set, the attrs or a
   timestamp. Two sensors confirming the same relationship MUST land on
   one key: otherwise the catalog publishes the same fact twice and the
   cardinality budget counts it twice. A restart with the same evidence
   MUST produce identical ids and identical documents, which is the same
   determinism requirement §5 already makes of entity ids, for the same
   reason — anything non-deterministic reaching that hash becomes a churn
   of tombstones and upserts on every restart.
2. **`edge-id` is opaque.** A consumer MUST read the endpoints from the
   payload, never parse them out of the key ([03-grammar.md
   §6.2](03-grammar.md)).
3. **The kind vocabulary is closed** and extended by amendment, like every
   other vocabulary here. It names *facts about the deployment*, not
   observations that happen to hold this minute.
4. **Direction is meaningful for containment kinds**: `from` contains,
   `to` is contained. A kind that is symmetric MUST say so, and MUST NOT
   propagate failure (below).
5. **The catalog tombstones an edge when its last observer stops claiming
   it**, on the same TTL discipline as the evidence it was built from
   (§4). An edge nobody claims is not a historical record; history is a
   storage concern (§5.2).
6. **An end that resolves to nothing identifiable is dropped, not
   published as an anonymous node.** A graph of unnamed circles is worse
   than a smaller true graph.

#### Traffic is not a relationship (normative)

Observed **flow** between two addresses MUST NOT be published as an edge.
It is per-observed-peer — every public address the sensor ever saw — it
changes by the second, and it would breach any declared cardinality budget
([04-planes.md §1.2](04-planes.md)), which is the same reason §4 already
requires an internet-facing sensor to aggregate before publishing per-IP
keys. A traffic matrix belongs on an `@rpc` overlay, pulled by the
consumer that wants it, at the resolution that consumer asked for.

This is the line that keeps the family bounded, and it is worth stating as
a rule rather than as advice: `edge` is sized by entities × (one gateway +
guests + containers + probe targets + observed neighbours), refreshed at
evidence cadence. Flow is sized by the internet.

#### Why this is catalog state and not a plane

A `@graph` plane was the obvious alternative and is rejected for the
reason §5 exists: relationships are *conclusions from evidence*, and
conclusions have one author. A plane would be a second identity service
with a second claim protocol, a second ownership election and a second
storage stanza, arriving at the same answers from the same inputs. The
`@catalog` origin is already the reserved, verbatim, D4-excluded place for
exactly this.

#### Impact attribution is a pure function of the graph

The point of publishing edges is that a consumer can answer *"is this
alert a cause or a symptom"* without asking anyone. That answer MUST be
derivable from the edge set, the firing set and the down set alone, and it
MUST be a pure function of them — no clock, no private state, no
subscription the deriving consumer does not already hold.

- **Only containment kinds propagate.** A symmetric kind (two machines
  sharing a link-layer segment) carries no causal direction, and treating
  it as one turns "these two are neighbours" into "this one broke that
  one".
- **A root is a down entity with no down containment ancestor.** Every
  other affected site is a symptom and carries what explains it — the root
  entity, or the root's own firing alert when it has one, which is the
  more useful of the two because it sends the operator to a page that
  describes the failure rather than to a machine that is merely quiet.
- **The walk MUST be bounded** — a depth cap and a visited set. The graph
  is built from claims made by independent sensors, so a cycle is a
  reachable input, not a hypothetical, and an unbounded walk over one is a
  hang in whatever consumer was unlucky.
- **The output MUST be ordered**, so two consumers with the same inputs
  render the same thing and a diff between two runs means something.

#### What this deliberately is not

A general graph database, a query language over relationships, or
service-level dependencies *inside* a host (which are intra-host detail,
not fleet structure). Nor a geographic map. The family answers one
question — what depends on what, across the fleet — and it answers it in
documents any consumer can read.

---

## 6. Targeting a known host — the consumer identity bridge

*Added in v1.2. §5.1 runs origin → entity: "I have a key, whose box is
this?" This section runs the other way — "I have a box, what key do I
build?" — and it is the direction that actually breaks products.*

Every key in the convention is origin-scoped. Every *payload* carries a
human identity (`source` = hostname, a label an operator recognises). A
consumer therefore holds the human identity and needs the origin, because
an origin-scoped key is the only thing it can address. **That step is not
optional and it is not free**, and a convention that leaves it unsaid
invites the consumer to substitute one for the other.

### 6.1 The bridge

> **The payload `host_id` IS the origin id.** They are the same value,
> minted by the same function ([§1](#1-host-origins--self-minted-stable-opaque)).
> Nothing else in a payload is.

A hostname is **not** an identity: it is unstable (renames), non-unique
(`localhost`, cloned images, two containers named alike), and operator-
assigned. It is a *display label*.

### 6.2 Normative

- A consumer that holds a human identity (hostname, `source`, a row the
  user clicked) and needs an origin-scoped key **MUST** resolve it to an
  origin first. It **MUST NOT** interpolate the human identity into the
  origin position.
- A consumer **SHOULD** obtain the origin from the **key it already
  received**. Every data key it consumed carries the origin in chunk 3; a
  consumer that re-derives identity from the payload when the key already
  states it has built a second, weaker identity path for no reason.
- Where the origin is not on a key the consumer holds, the two sanctioned
  bridges are:
  1. **the health / registration documents** — `state/<producer>/health`
     and `state/<producer>/sensor` both carry `host_id` alongside
     `source`, and both are origin-scoped, so the pair is self-certifying;
  2. **the `@catalog` entity document** — which **MUST** therefore
     enumerate the entity's origins (see §6.4).
- A consumer **MUST NOT** key its own device/host tables on the human
  identity. Two hosts reporting one hostname collide, and a collision in a
  table that *builds keys* does not merely muddle a display — it
  **misroutes queries to the wrong host**.
- The `*`-origin fleet selector is a legal fallback *only* while the
  bridge is genuinely unresolved, and only where the amplification is
  acceptable ([07-bulk-planes.md §3](07-bulk-planes.md) — on `@media` and
  `@blob` it is not). It is a fallback, not a default.

### 6.3 Why this is stated so loudly

The reference implementation shipped a UI whose device table was keyed on
the payload hostname. When drill-down fetches became origin-scoped, they
were built from that table — so the UI issued
`GET <base>/v1/toolbx/@rpc/sysinfo/processes`, addressing an origin that
has never existed, while streamed telemetry kept working perfectly.
**Every drill-down in the product died at once**, and it looked like a
sensor fault.

The bridge was on the wire the whole time. The convention never said to
build it.

### 6.4 Consequence for the entity document

An entity document **MUST** carry the origins it merges — the
`entity.origins[]` that [§5.1](#51-how-a-ui-joins) step 3 already assumes.
An entity that records only its members' *human* labels cannot serve as a
bridge, because reading it leaves the consumer holding exactly what it
started with. §5.1 (v1.30) states what the set contains — self-reported
origins only — and what a consumer does with an entity published before
the field existed.

